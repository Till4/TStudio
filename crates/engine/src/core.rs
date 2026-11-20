//! The engine core: sequencing, voice rendering and mixing. Real-time safe
//! (no allocation, no locks) once constructed.

use crate::spectrum::{SpectrumShared, SpectrumTaps};
use crate::snapshot::{ChannelProc, ClipSnap, FxSnap, InstrumentSnap, Rack, Snapshot, TrackProc};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tstudio_dsp::fx::{Effect, FxContext};
use tstudio_dsp::{pan_gains, SampleData};
use tstudio_project::tempo::samples_per_tick;
use tstudio_project::{AutoTarget, PPQ};

mod shared;
mod sequencer;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tempo_tests;

pub use shared::*;
pub use sequencer::*;

/// Automation lanes evaluated per chunk (further lanes are ignored).
pub const MAX_AUTO: usize = 64;
type AutoVals = [Option<f32>; MAX_AUTO];

pub const MAX_CHUNK: usize = 256;
pub const MAX_TRACKS: usize = 128;
const MAX_EVENTS: usize = 4096;
const MAX_LIVE: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq)]
enum EvKind {
    On,
    Off,
    Click(bool),
}

#[derive(Clone, Copy, Debug)]
struct Event {
    frame: usize,
    channel: usize,
    key: u8,
    vel: f32,
    /// playlist-track volume
    gain: f32,
    dur: Option<u64>,
    kind: EvKind,
}

#[derive(Default)]
struct Click {
    pending: Option<(usize, bool)>,
    remaining: u32,
    phase: f32,
    inc: f32,
    amp: f32,
}

pub struct EngineCore {
    sr: f32,
    snap: Arc<Snapshot>,
    rack: Box<Rack>,
    playing: bool,
    mode: PlayMode,
    pos: f64,
    /// Offline rendering: stop at the end instead of looping.
    pub stop_at_end: bool,
    metronome: bool,
    click: Click,
    preview: Option<Arc<SampleData>>,
    preview_pos: f64,
    preview_gain: f32,
    preview_loop: bool,
    live: Vec<Event>,
    events: Vec<Event>,
    auto: AutoVals,
    shared: Arc<SharedState>,
    monitor: Option<Box<MonitorIn>>,
    monitor_track: Option<usize>,
    spectra: SpectrumTaps,
}

impl EngineCore {
    pub fn new(sr: f32, snap: Arc<Snapshot>, rack: Box<Rack>, shared: Arc<SharedState>) -> Self {
        Self {
            sr,
            snap,
            rack,
            playing: false,
            mode: PlayMode::Pattern(0),
            pos: 0.0,
            stop_at_end: false,
            metronome: false,
            click: Click::default(),
            preview: None,
            preview_pos: 0.0,
            preview_gain: 1.0,
            preview_loop: false,
            live: Vec::with_capacity(MAX_LIVE),
            events: Vec::with_capacity(MAX_EVENTS),
            auto: [None; MAX_AUTO],
            shared,
            monitor: None,
            monitor_track: None,
            spectra: SpectrumTaps::new(sr),
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    pub fn position(&self) -> f64 {
        self.pos
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    fn release_all(&mut self) {
        for c in self.rack.channels.iter_mut() {
            c.release_all();
        }
    }

    fn start_position(&self) -> f64 {
        match self.mode {
            PlayMode::Pattern(_) => 0.0,
            PlayMode::Song => self.snap.loop_region.map(|(a, _)| a as f64).unwrap_or(0.0),
        }
    }

    pub fn handle(&mut self, cmd: Command, gc: &mut dyn FnMut(Garbage)) {
        match cmd {
            Command::SetSnapshot(s) => {
                let old = std::mem::replace(&mut self.snap, s);
                gc(Garbage::Snapshot(old));
            }
            Command::SetRack(rack, s) => {
                let old_rack = std::mem::replace(&mut self.rack, rack);
                let old = std::mem::replace(&mut self.snap, s);
                gc(Garbage::Rack(old_rack));
                gc(Garbage::Snapshot(old));
            }
            Command::Play => {
                self.playing = true;
            }
            Command::Pause => {
                self.playing = false;
                self.release_all();
            }
            Command::Stop => {
                self.playing = false;
                self.pos = self.start_position();
                self.release_all();
            }
            Command::SetPosition(t) => {
                self.pos = t.max(0.0);
                self.release_all();
            }
            Command::SetMode(m) => {
                if m != self.mode {
                    self.mode = m;
                    self.pos = self.start_position();
                    self.release_all();
                }
            }
            Command::SetMetronome(on) => self.metronome = on,
            Command::SetMonitor(m) => {
                if let Some(old) = std::mem::replace(&mut self.monitor, m) {
                    gc(Garbage::Monitor(old));
                }
            }
            Command::SetMonitorTrack(t) => self.monitor_track = t,
            Command::NoteOn { channel, key, velocity } => {
                if self.live.len() < MAX_LIVE {
                    self.live.push(Event { frame: 0, channel, key, vel: velocity, gain: 1.0, dur: None, kind: EvKind::On });
                }
            }
            Command::NoteOff { channel, key } => {
                if self.live.len() < MAX_LIVE {
                    self.live.push(Event { frame: 0, channel, key, vel: 0.0, gain: 1.0, dur: None, kind: EvKind::Off });
                }
            }
            Command::PreviewSample(s, gain) => {
                if let Some(old) = self.preview.replace(s) {
                    gc(Garbage::Sample(old));
                }
                self.preview_pos = 0.0;
                self.preview_gain = gain;
            }
            Command::PreviewLoop(on) => self.preview_loop = on,
            Command::StopPreview => {
                if let Some(old) = self.preview.take() {
                    gc(Garbage::Sample(old));
                }
            }
            Command::Panic => {
                for c in self.rack.channels.iter_mut() {
                    c.kill_all();
                }
                if let Some(old) = self.preview.take() {
                    gc(Garbage::Sample(old));
                }
            }
        }
    }

    /// Renders `out_l.len()` frames (overwrites the buffers).
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let total = out_l.len().min(out_r.len());
        let mut off = 0;
        while off < total {
            let n = (total - off).min(MAX_CHUNK);
            self.process_chunk(&mut out_l[off..off + n], &mut out_r[off..off + n]);
            off += n;
        }
        self.shared.position.store(self.pos.to_bits(), Ordering::Relaxed);
        self.shared.playing.store(self.playing, Ordering::Relaxed);
    }

    fn process_chunk(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let n = out_l.len();
        let snap = Arc::clone(&self.snap);
        let snap: &Snapshot = &snap;
        let sr = self.sr;

        self.spectra.sync(&self.shared.spectrum);
        for t in self.rack.tracks.iter_mut() {
            t.l[..n].fill(0.0);
            t.r[..n].fill(0.0);
        }

        // ---- automation (song mode only), evaluated at the chunk start
        for (i, slot) in self.auto.iter_mut().enumerate() {
            *slot = match (self.mode, snap.autos.get(i)) {
                (PlayMode::Song, Some(lane)) => lane.value(self.pos),
                _ => None,
            };
        }
        let auto = self.auto;

        // ---- events
        self.events.clear();
        for ev in self.live.drain(..) {
            if self.events.len() < MAX_EVENTS {
                self.events.push(ev);
            }
        }
        if self.playing {
            let (new_pos, ended) = sequence(
                snap,
                &mut self.rack.tracks,
                &mut self.events,
                self.pos,
                self.mode,
                sr,
                n,
                self.metronome,
                self.stop_at_end,
            );
            self.pos = new_pos;
            if ended {
                self.playing = false;
                for c in self.rack.channels.iter_mut() {
                    c.release_all();
                }
            }
        }
        self.events.sort_unstable_by_key(|e| e.frame);
        // current tempo (tempo track in song mode) for synced LFOs / delays
        let bpm = match self.mode {
            PlayMode::Song => snap.tempo.bpm_at(self.pos) as f32,
            PlayMode::Pattern(_) => snap.bpm as f32,
        };

        // ---- instruments, split at event frames for sample accuracy
        let mut cur = 0usize;
        for i in 0..self.events.len() {
            let ev = self.events[i];
            let f = ev.frame.min(n);
            if f > cur {
                render_channels(snap, &mut self.rack, &auto, bpm, cur, f);
                cur = f;
            }
            match ev.kind {
                EvKind::Click(down) => self.click.pending = Some((f, down)),
                EvKind::On | EvKind::Off => apply_event(snap, &mut self.rack, &ev),
            }
        }
        render_channels(snap, &mut self.rack, &auto, bpm, cur, n);

        // ---- live input (monitoring through a mixer insert)
        if let Some(m) = &mut self.monitor {
            // keep latency low: never let more than ~1024 frames pile up
            while m.rx.slots() > 2 * (n + 1024) {
                let _ = m.rx.pop();
            }
            if !m.primed && m.rx.slots() >= 2 * (n + 128) {
                m.primed = true;
            }
            if m.primed {
                let track = self.monitor_track.and_then(|t| self.rack.tracks.get_mut(t));
                match track {
                    Some(tr) => {
                        for f in 0..n {
                            let (Ok(l), Ok(r)) = (m.rx.pop(), m.rx.pop()) else {
                                m.primed = false;
                                break;
                            };
                            tr.l[f] += l;
                            tr.r[f] += r;
                        }
                    }
                    None => {
                        for _ in 0..2 * n {
                            if m.rx.pop().is_err() {
                                m.primed = false;
                                break;
                            }
                        }
                    }
                }
            }
        }

        // ---- sidechain keys: pre-fx level of the source track
        let any_solo = snap.mixer.iter().skip(1).any(|t| t.solo);
        let Rack { tracks, keys, .. } = &mut *self.rack;
        let n_tracks = tracks.len().min(snap.mixer.len());
        for t in 0..n_tracks {
            if keys[t].is_empty() {
                continue;
            }
            let key = &mut keys[t][..n];
            let src = snap.mixer[t].fx.iter().filter(|f| f.enabled).find_map(|f| f.params.sidechain_source());
            match src {
                Some(s) if s < n_tracks && s != t => {
                    let st = &tracks[s];
                    for k in 0..n {
                        key[k] = 0.5 * (st.l[k].abs() + st.r[k].abs());
                    }
                }
                _ => key.fill(0.0),
            }
        }

        // ---- inserts in index order: fx -> fader -> output (master or bus) + sends
        for t in 1..n_tracks {
            let ts = &snap.mixer[t];
            let (lo, hi) = tracks.split_at_mut(t + 1);
            let (master, inserts) = lo.split_at_mut(1);
            let TrackProc { fx, l, r } = &mut inserts[t - 1];
            let (l, r) = (&mut l[..n], &mut r[..n]);
            if ts.muted || (any_solo && !ts.solo && !ts.is_bus) {
                continue;
            }
            let key: &[f32] = if keys[t].is_empty() { &[] } else { &keys[t][..n] };
            run_fx(fx, &ts.fx, t, snap, &auto, &FxContext { sr, bpm, key }, l, r);
            let vol = track_auto(snap, &auto, AutoTarget::TrackVolume(t)).unwrap_or(ts.volume);
            let pan = track_auto(snap, &auto, AutoTarget::TrackPan(t)).unwrap_or(ts.pan);
            let (gl, gr) = pan_gains(pan);
            let (gl, gr) = (gl * vol, gr * vol);
            let mut pl = 0.0f32;
            let mut pr = 0.0f32;
            for k in 0..n {
                l[k] *= gl;
                r[k] *= gr;
                pl = pl.max(l[k].abs());
                pr = pr.max(r[k].abs());
            }
            self.shared.set_meter(t, pl, pr);
            self.spectra.feed(&self.shared.spectrum, t, l, r);
            let dest: &mut TrackProc = if ts.output > t && ts.output < n_tracks { &mut hi[ts.output - t - 1] } else { &mut master[0] };
            for k in 0..n {
                dest.l[k] += l[k];
                dest.r[k] += r[k];
            }
            for &(target, amount) in &ts.sends {
                if target > t && target < n_tracks {
                    let d = &mut hi[target - t - 1];
                    for k in 0..n {
                        d.l[k] += l[k] * amount;
                        d.r[k] += r[k] * amount;
                    }
                }
            }
        }
        let ms = &snap.mixer[0];
        {
            let TrackProc { fx, l, r } = &mut tracks[0];
            let (l, r) = (&mut l[..n], &mut r[..n]);
            let key: &[f32] = if keys[0].is_empty() { &[] } else { &keys[0][..n] };
            run_fx(fx, &ms.fx, 0, snap, &auto, &FxContext { sr, bpm, key }, l, r);
            let vol = track_auto(snap, &auto, AutoTarget::TrackVolume(0)).unwrap_or(ms.volume);
            let pan = track_auto(snap, &auto, AutoTarget::TrackPan(0)).unwrap_or(ms.pan);
            let (gl, gr) = pan_gains(pan);
            let v = if ms.muted { 0.0 } else { vol };
            for k in 0..n {
                out_l[k] = l[k] * gl * v;
                out_r[k] = r[k] * gr * v;
            }
        }

        // ---- metronome + preview (post master, not affected by project fx)
        for k in 0..n {
            if let Some((f, down)) = self.click.pending {
                if f == k {
                    self.click.pending = None;
                    self.click.remaining = (0.03 * sr) as u32;
                    self.click.phase = 0.0;
                    self.click.inc = if down { 1760.0 } else { 1320.0 } / sr;
                    self.click.amp = if down { 0.5 } else { 0.32 };
                }
            }
            if self.click.remaining > 0 {
                self.click.remaining -= 1;
                let env = self.click.remaining as f32 / (0.03 * sr);
                let v = (self.click.phase * std::f32::consts::TAU).sin() * self.click.amp * env * env;
                self.click.phase = (self.click.phase + self.click.inc) % 1.0;
                out_l[k] += v;
                out_r[k] += v;
            }
        }
        if let Some(p) = self.preview.as_ref() {
            let step = p.sample_rate as f64 / sr as f64;
            for k in 0..n {
                let mut i = self.preview_pos as usize;
                if i >= p.len() {
                    if !self.preview_loop || p.is_empty() {
                        break;
                    }
                    self.preview_pos -= p.len() as f64;
                    i = (self.preview_pos.max(0.0) as usize).min(p.len() - 1);
                }
                out_l[k] += p.left[i] * self.preview_gain;
                out_r[k] += p.right[i] * self.preview_gain;
                self.preview_pos += step;
            }
        }

        let mut pl = 0.0f32;
        let mut pr = 0.0f32;
        for k in 0..n {
            out_l[k] = out_l[k].clamp(-1.0, 1.0);
            out_r[k] = out_r[k].clamp(-1.0, 1.0);
            pl = pl.max(out_l[k].abs());
            pr = pr.max(out_r[k].abs());
        }
        self.shared.set_meter(0, pl, pr);
        self.spectra.feed(&self.shared.spectrum, 0, out_l, out_r);
    }
}
