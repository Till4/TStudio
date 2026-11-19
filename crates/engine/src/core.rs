//! The engine core: sequencing, voice rendering and mixing. Real-time safe
//! (no allocation, no locks) once constructed.

use crate::snapshot::{ChannelProc, ClipSnap, FxSnap, InstrumentSnap, Rack, Snapshot, TrackProc};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tstudio_dsp::fx::{Effect, FxContext};
use tstudio_dsp::{pan_gains, SampleData};
use tstudio_project::{AutoTarget, PPQ};

/// Automation lanes evaluated per chunk (further lanes are ignored).
pub const MAX_AUTO: usize = 64;
type AutoVals = [Option<f32>; MAX_AUTO];

pub const MAX_CHUNK: usize = 256;
pub const MAX_TRACKS: usize = 128;
const MAX_EVENTS: usize = 4096;
const MAX_LIVE: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    /// Loop one pattern (index into `Snapshot::patterns`).
    Pattern(usize),
    /// Play the playlist.
    Song,
}

pub enum Command {
    SetSnapshot(Arc<Snapshot>),
    SetRack(Box<Rack>, Arc<Snapshot>),
    Play,
    Pause,
    Stop,
    SetPosition(f64),
    SetMode(PlayMode),
    SetMetronome(bool),
    NoteOn { channel: usize, key: u8, velocity: f32 },
    NoteOff { channel: usize, key: u8 },
    PreviewSample(Arc<SampleData>, f32),
    StopPreview,
    /// Loop the current preview sample (sample cutter)
    PreviewLoop(bool),
    Panic,
    /// Live input for monitoring (replaces the previous one)
    SetMonitor(Option<Box<MonitorIn>>),
    /// Mixer insert the live input is played through (None = not audible)
    SetMonitorTrack(Option<usize>),
}

/// Live input frames (interleaved stereo) from the audio input callback.
pub struct MonitorIn {
    pub rx: rtrb::Consumer<f32>,
    primed: bool,
}

impl MonitorIn {
    pub fn new(rx: rtrb::Consumer<f32>) -> Self {
        Self { rx, primed: false }
    }
}

/// Objects handed back to the UI thread for deallocation.
pub enum Garbage {
    Rack(Box<Rack>),
    Snapshot(Arc<Snapshot>),
    Sample(Arc<SampleData>),
    Monitor(Box<MonitorIn>),
}

/// (time, position) written by the audio thread, read consistently by the UI (seqlock).
struct Stamp {
    seq: AtomicU32,
    ns: AtomicU64,
    pos: AtomicU64,
}

impl Stamp {
    fn new() -> Self {
        Self { seq: AtomicU32::new(0), ns: AtomicU64::new(0), pos: AtomicU64::new(0) }
    }
    fn write(&self, ns: u64, pos: f64) {
        self.seq.fetch_add(1, Ordering::AcqRel);
        self.ns.store(ns, Ordering::Relaxed);
        self.pos.store(pos.to_bits(), Ordering::Relaxed);
        self.seq.fetch_add(1, Ordering::AcqRel);
    }
    fn read(&self) -> Option<(u64, f64)> {
        for _ in 0..100 {
            let a = self.seq.load(Ordering::Acquire);
            if a % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let ns = self.ns.load(Ordering::Relaxed);
            let pos = f64::from_bits(self.pos.load(Ordering::Relaxed));
            if self.seq.load(Ordering::Acquire) == a {
                return if a == 0 { None } else { Some((ns, pos)) };
            }
        }
        None
    }
}

/// Lock-free state the UI reads every frame.
pub struct SharedState {
    position: AtomicU64,
    playing: AtomicBool,
    meters: Vec<[AtomicU32; 2]>,
    cpu: AtomicU32,
    pub sample_rate: AtomicU32,
    pub stream_error: AtomicBool,
    /// time base shared with the audio input (ns since `epoch`)
    pub epoch: std::time::Instant,
    /// when (audible at the speakers) and where the last playback started
    play_start: Stamp,
    /// output latency of the device in ns
    out_latency_ns: AtomicU64,
}

impl SharedState {
    pub fn new(sr: u32) -> Self {
        Self {
            position: AtomicU64::new(0f64.to_bits()),
            playing: AtomicBool::new(false),
            meters: (0..MAX_TRACKS).map(|_| [AtomicU32::new(0), AtomicU32::new(0)]).collect(),
            cpu: AtomicU32::new(0),
            sample_rate: AtomicU32::new(sr),
            stream_error: AtomicBool::new(false),
            epoch: std::time::Instant::now(),
            play_start: Stamp::new(),
            out_latency_ns: AtomicU64::new(0),
        }
    }

    /// (ns since epoch when it reached the speakers, start position in ticks) of the last playback start.
    pub fn play_start(&self) -> Option<(u64, f64)> {
        self.play_start.read()
    }

    pub fn output_latency_ms(&self) -> f32 {
        self.out_latency_ns.load(Ordering::Relaxed) as f32 / 1e6
    }

    /// Called by the output callback at the start of every block.
    pub fn note_block(&self, started_playing: bool, pos: f64, latency: std::time::Duration) {
        self.out_latency_ns.store(latency.as_nanos() as u64, Ordering::Relaxed);
        if started_playing {
            let ns = (self.epoch.elapsed() + latency).as_nanos() as u64;
            self.play_start.write(ns, pos);
        }
    }

    /// Playhead in ticks.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Peak since the last call (linear), per side.
    pub fn take_meter(&self, track: usize) -> (f32, f32) {
        match self.meters.get(track) {
            Some(m) => (f32::from_bits(m[0].swap(0, Ordering::Relaxed)), f32::from_bits(m[1].swap(0, Ordering::Relaxed))),
            None => (0.0, 0.0),
        }
    }

    /// Fraction of the real-time budget used by the last callback.
    pub fn cpu_load(&self) -> f32 {
        f32::from_bits(self.cpu.load(Ordering::Relaxed))
    }

    pub fn set_cpu_load(&self, v: f32) {
        self.cpu.store(v.to_bits(), Ordering::Relaxed);
    }

    fn set_meter(&self, track: usize, l: f32, r: f32) {
        if let Some(m) = self.meters.get(track) {
            // positive floats order like their bit patterns
            m[0].fetch_max(l.abs().to_bits(), Ordering::Relaxed);
            m[1].fetch_max(r.abs().to_bits(), Ordering::Relaxed);
        }
    }
}

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

        // ---- instruments, split at event frames for sample accuracy
        let mut cur = 0usize;
        for i in 0..self.events.len() {
            let ev = self.events[i];
            let f = ev.frame.min(n);
            if f > cur {
                render_channels(snap, &mut self.rack, &auto, cur, f);
                cur = f;
            }
            match ev.kind {
                EvKind::Click(down) => self.click.pending = Some((f, down)),
                EvKind::On | EvKind::Off => apply_event(snap, &mut self.rack, &ev),
            }
        }
        render_channels(snap, &mut self.rack, &auto, cur, n);

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
        let bpm = snap.bpm as f32;
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
    }
}

/// Automated value for a whole-track / channel target, if a lane is active.
fn track_auto(snap: &Snapshot, auto: &AutoVals, target: AutoTarget) -> Option<f32> {
    snap.autos.iter().take(MAX_AUTO).enumerate().find(|(_, l)| l.target == target).and_then(|(i, _)| auto[i])
}

/// Runs a track's effect chain; automated parameters are applied on a stack copy (no allocation).
#[allow(clippy::too_many_arguments)]
fn run_fx(fx: &mut [Effect], slots: &[FxSnap], track: usize, snap: &Snapshot, auto: &AutoVals, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
    for (si, (proc, slot)) in fx.iter_mut().zip(slots).enumerate() {
        if !slot.enabled {
            continue;
        }
        let automated = snap.autos.iter().take(MAX_AUTO).enumerate().any(|(i, lane)| {
            auto[i].is_some() && matches!(lane.target, AutoTarget::Fx { track: t, slot: s, .. } if t == track && s == si)
        });
        if automated {
            let mut p = slot.params.clone();
            for (i, lane) in snap.autos.iter().take(MAX_AUTO).enumerate() {
                if let (AutoTarget::Fx { track: t, slot: s, field }, Some(v)) = (lane.target, auto[i]) {
                    if t == track && s == si {
                        if let Some(f) = p.field_mut(field) {
                            *f = v;
                        }
                    }
                }
            }
            proc.process(&p, ctx, l, r);
        } else {
            proc.process(&slot.params, ctx, l, r);
        }
    }
}

fn apply_event(snap: &Snapshot, rack: &mut Rack, ev: &Event) {
    let (Some(cs), Some(proc)) = (snap.channels.get(ev.channel), rack.channels.get_mut(ev.channel)) else { return };
    match (proc, &cs.instrument, ev.kind) {
        (ChannelProc::Sampler(s), InstrumentSnap::Sampler { params, sample }, EvKind::On) => {
            s.note_gain = ev.gain;
            s.note_on(params, sample.as_ref(), ev.key, ev.vel, ev.dur)
        }
        (ChannelProc::Synth(s), InstrumentSnap::Synth(p), EvKind::On) => {
            s.note_gain = ev.gain;
            s.note_on(p, ev.key, ev.vel, ev.dur)
        }
        // SoundFonts only take a MIDI velocity: approximate the track volume through it
        (ChannelProc::SoundFont(s), InstrumentSnap::SoundFont { bank, program, .. }, EvKind::On) => s.note_on(*bank, *program, ev.key, ev.vel * ev.gain.sqrt(), ev.dur),
        (ChannelProc::Sampler(s), _, EvKind::Off) => s.note_off(ev.key),
        (ChannelProc::Synth(s), _, EvKind::Off) => s.note_off(ev.key),
        (ChannelProc::SoundFont(s), _, EvKind::Off) => s.note_off(ev.key),
        _ => {}
    }
}

fn render_channels(snap: &Snapshot, rack: &mut Rack, auto: &AutoVals, from: usize, to: usize) {
    if to <= from {
        return;
    }
    let Rack { channels, tracks, scratch_l, scratch_r, .. } = rack;
    for (ci, proc) in channels.iter_mut().enumerate() {
        let Some(cs) = snap.channels.get(ci) else { continue };
        if !proc.is_active() {
            continue;
        }
        let sl = &mut scratch_l[from..to];
        let sr = &mut scratch_r[from..to];
        sl.fill(0.0);
        sr.fill(0.0);
        match (proc, &cs.instrument) {
            (ChannelProc::Sampler(s), InstrumentSnap::Sampler { params, .. }) => s.render(params, sl, sr),
            (ChannelProc::Synth(s), InstrumentSnap::Synth(p)) => {
                s.bpm = snap.bpm as f32;
                let automated = snap.autos.iter().take(MAX_AUTO).enumerate().any(|(i, l)| auto[i].is_some() && matches!(l.target, AutoTarget::Synth { channel, .. } if channel == ci));
                if automated {
                    let mut q = p.clone();
                    for (i, lane) in snap.autos.iter().take(MAX_AUTO).enumerate() {
                        if let (AutoTarget::Synth { channel, field }, Some(v)) = (lane.target, auto[i]) {
                            if channel == ci {
                                if let Some(f) = q.field_mut(field) {
                                    *f = v;
                                }
                            }
                        }
                    }
                    s.render(&q, sl, sr);
                } else {
                    s.render(p, sl, sr);
                }
            }
            (ChannelProc::SoundFont(s), InstrumentSnap::SoundFont { gain, .. }) => s.render(*gain, sl, sr),
            _ => continue,
        }
        let vol = track_auto(snap, auto, AutoTarget::ChannelVolume(ci)).unwrap_or(cs.volume);
        let pan = track_auto(snap, auto, AutoTarget::ChannelPan(ci)).unwrap_or(cs.pan);
        let (gl, gr) = pan_gains(pan);
        let v = if cs.muted { 0.0 } else { vol };
        let track_idx = if cs.mixer_track < tracks.len() { cs.mixer_track } else { 0 };
        let t = &mut tracks[track_idx];
        for k in 0..(to - from) {
            t.l[from + k] += sl[k] * gl * v;
            t.r[from + k] += sr[k] * gr * v;
        }
    }
}

#[inline]
fn push_event(events: &mut Vec<Event>, ev: Event) {
    if events.len() < MAX_EVENTS {
        events.push(ev);
    }
}

/// Advances the transport over one chunk: collects note/click events and renders audio clips.
/// Returns the new position and whether playback reached the end (only with `stop_at_end`).
#[allow(clippy::too_many_arguments)]
fn sequence(
    snap: &Snapshot,
    tracks: &mut [TrackProc],
    events: &mut Vec<Event>,
    pos: f64,
    mode: PlayMode,
    sr: f32,
    n: usize,
    metronome: bool,
    stop_at_end: bool,
) -> (f64, bool) {
    let spt = sr as f64 * 60.0 / (snap.bpm.max(1.0) * PPQ as f64);
    let bar = PPQ * snap.beats_per_bar.max(1) as i64;
    let (lo, hi, looping) = match mode {
        PlayMode::Pattern(i) => (0, snap.patterns.get(i).map(|p| p.length).unwrap_or(bar), !stop_at_end),
        PlayMode::Song => match snap.loop_region {
            Some((a, b)) if b > a && !stop_at_end => (a, b, true),
            _ => {
                let end = snap.song_end.max(bar);
                (0, end, !stop_at_end)
            }
        },
    };
    let (lo, hi) = (lo as f64, hi as f64);
    let mut cur = pos;
    if cur >= hi {
        if looping {
            cur = lo;
        } else {
            return (hi, true);
        }
    }
    let total = n as f64;
    let mut frame0 = 0.0f64;
    let mut guard = 0;
    while frame0 < total - 1e-6 && guard < 64 {
        guard += 1;
        let seg_ticks = ((total - frame0) / spt).min(hi - cur);
        if seg_ticks <= 1e-9 {
            if looping {
                cur = lo;
                continue;
            }
            return (cur, true);
        }
        let a = cur;
        let b = cur + seg_ticks;
        let to_frame = |t: f64| -> usize { ((frame0 + (t - a) * spt - 1e-6).ceil().max(0.0) as usize).min(n.saturating_sub(1)) };

        // notes
        let mut emit_pattern = |pat: usize, base: i64, win_lo: f64, win_hi: f64, clip_len: Option<i64>, gain: f32| {
            let Some(p) = snap.patterns.get(pat) else { return };
            let local_lo = win_lo - base as f64;
            let local_hi = win_hi - base as f64;
            let first = p.notes.partition_point(|x| (x.start as f64) < local_lo);
            for note in &p.notes[first..] {
                if note.start as f64 >= local_hi {
                    break;
                }
                if let Some(cl) = clip_len {
                    if note.start >= cl {
                        break;
                    }
                }
                let Some(ch) = snap.channels.get(note.channel) else { continue };
                if ch.muted {
                    continue;
                }
                let t = (base + note.start) as f64;
                push_event(
                    events,
                    Event {
                        frame: to_frame(t),
                        channel: note.channel,
                        key: note.key,
                        vel: note.velocity,
                        gain,
                        dur: Some((note.len as f64 * spt).max(1.0) as u64),
                        kind: EvKind::On,
                    },
                );
            }
        };
        match mode {
            PlayMode::Pattern(i) => emit_pattern(i, 0, a, b, None, 1.0),
            PlayMode::Song => {
                for clip in &snap.clips {
                    if let ClipSnap::Pattern { pattern, start, len, gain } = *clip {
                        // a clip longer than its pattern repeats the pattern (drag the right edge to loop)
                        let plen = snap.patterns.get(pattern).map(|p| p.length).unwrap_or(len).max(1);
                        let end = start + len;
                        if b <= start as f64 || a >= end as f64 {
                            continue;
                        }
                        let mut k = ((a - start as f64) / plen as f64).floor().max(0.0) as i64;
                        loop {
                            let base = start + k * plen;
                            if base >= end || base as f64 >= b {
                                break;
                            }
                            let rep_end = (base + plen).min(end);
                            let wl = a.max(base as f64);
                            let wh = b.min(rep_end as f64);
                            if wl < wh {
                                emit_pattern(pattern, base, wl, wh, Some(rep_end - base), gain);
                            }
                            k += 1;
                        }
                    }
                }
            }
        }

        // metronome
        if metronome {
            let mut k = (a / PPQ as f64).ceil() as i64;
            while ((k * PPQ) as f64) < b {
                let down = k % snap.beats_per_bar.max(1) as i64 == 0;
                push_event(events, Event { frame: to_frame((k * PPQ) as f64), channel: 0, key: 0, vel: 0.0, gain: 1.0, dur: None, kind: EvKind::Click(down) });
                k += 1;
            }
        }

        // audio clips
        if mode == PlayMode::Song {
            for clip in &snap.clips {
                if let ClipSnap::Audio { sample, start, len, offset, gain, mixer_track, fade_in, fade_out } = clip {
                    // latency compensation (e.g. Autotune): start the clip that much earlier
                    let adv = chain_latency(snap, *mixer_track, sr) as f64 / spt;
                    let cs = *start as f64 - adv;
                    let ce = (*start + *len) as f64 - adv;
                    let wl = a.max(cs);
                    let wh = b.min(ce);
                    if wl >= wh {
                        continue;
                    }
                    let Some(track) = tracks.get_mut(*mixer_track) else { continue };
                    let fs = (frame0 + (wl - a) * spt).round().max(0.0) as usize;
                    let fe = ((frame0 + (wh - a) * spt).round() as usize).min(n);
                    for f in fs..fe {
                        let t = a + (f as f64 - frame0) / spt;
                        let local = t - cs;
                        let idx = ((local + *offset as f64) * spt).max(0.0) as usize;
                        if sample.is_empty() {
                            break;
                        }
                        // a clip longer than its sample loops the sample (drag the right edge to repeat)
                        let idx = idx % sample.len();
                        let mut g = *gain;
                        if *fade_in > 0 && local < *fade_in as f64 {
                            g *= (local / *fade_in as f64) as f32;
                        }
                        let to_end = ce - t;
                        if *fade_out > 0 && to_end < *fade_out as f64 {
                            g *= (to_end / *fade_out as f64).max(0.0) as f32;
                        }
                        track.l[f] += sample.left[idx] * g;
                        track.r[f] += sample.right[idx] * g;
                    }
                }
            }
        }

        frame0 += seg_ticks * spt;
        cur = b;
        if cur >= hi - 1e-9 {
            if looping {
                cur = lo;
            } else {
                return (hi, true);
            }
        }
    }
    (cur, false)
}

/// Total latency (samples) of the enabled effects on a mixer track and the buses it feeds.
pub fn chain_latency(snap: &Snapshot, track: usize, sr: f32) -> usize {
    let mut t = track;
    let mut total = 0;
    for _ in 0..snap.mixer.len() {
        let Some(tr) = snap.mixer.get(t) else { break };
        total += tr.fx.iter().filter(|f| f.enabled).map(|f| f.params.latency_frames(sr)).sum::<usize>();
        if t == 0 {
            break;
        }
        t = tr.output;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{ChannelSnap, FxSnap, NoteSnap, PatternSnap, TrackSnap};
    use tstudio_dsp::sampler::SamplerParams;

    const IMPULSE: usize = 100;

    fn click_sample(sr: u32) -> Arc<SampleData> {
        // impulse after the 0.5 ms minimum attack ramp
        let mut v = vec![0.0f32; 2000];
        v[IMPULSE] = 1.0;
        let _ = sr;
        Arc::new(SampleData::new(v.clone(), v, sr))
    }

    fn snapshot_with_notes(starts: &[i64], sr: u32) -> Snapshot {
        let mut s = Snapshot::empty();
        s.bpm = 120.0;
        s.channels.push(ChannelSnap {
            instrument: InstrumentSnap::Sampler {
                params: SamplerParams { amp_env: tstudio_dsp::env::AdsrParams::adsr(0.0, 0.1, 1.0, 0.01), velocity_sens: 0.0, ..Default::default() },
                sample: Some(click_sample(sr)),
            },
            mixer_track: 0,
            volume: 1.0,
            pan: 0.0,
            muted: false,
        });
        s.patterns.push(PatternSnap {
            length: PPQ * 4,
            notes: starts.iter().map(|&t| NoteSnap { channel: 0, key: 60, start: t, len: 24, velocity: 1.0 }).collect(),
        });
        s.mixer[0].fx.push(FxSnap { enabled: false, params: tstudio_dsp::fx::EffectKind::Limiter.default_params() });
        s
    }

    fn run(snap: Snapshot, sr: u32, frames: usize) -> Vec<f32> {
        let snap = Arc::new(snap);
        let rack = Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK));
        let mut core = EngineCore::new(sr as f32, snap, rack, Arc::new(SharedState::new(sr)));
        let mut gc = |_g: Garbage| {};
        core.handle(Command::SetMode(PlayMode::Pattern(0)), &mut gc);
        core.handle(Command::Play, &mut gc);
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        // odd block size to exercise chunk boundaries
        for (cl, cr) in l.chunks_mut(333).zip(r.chunks_mut(333)) {
            core.process(cl, cr);
        }
        l
    }

    #[test]
    fn notes_land_on_exact_samples() {
        let sr = 48000;
        // 120 bpm: one beat = 24000 samples, one tick = 250 samples
        let out = run(snapshot_with_notes(&[0, 96, 192, 300], sr), sr, 48000 * 2);
        let onsets: Vec<usize> = out.iter().enumerate().filter(|(_, v)| **v > 0.5).map(|(i, _)| i - IMPULSE).collect();
        assert_eq!(onsets, vec![0, 24000, 48000, 75000]);
    }

    #[test]
    fn pattern_loops() {
        let sr = 48000;
        // pattern is 4 beats = 96000 samples; note at 0 must retrigger at 96000
        let out = run(snapshot_with_notes(&[0], sr), sr, 96000 * 2 + 200);
        let onsets: Vec<usize> = out.iter().enumerate().filter(|(_, v)| **v > 0.5).map(|(i, _)| i - IMPULSE).collect();
        assert_eq!(onsets, vec![0, 96000, 192000]);
    }

    #[test]
    fn signature_changes_with_structure() {
        let a = snapshot_with_notes(&[0], 48000);
        let mut b = snapshot_with_notes(&[0, 5], 48000);
        assert_eq!(a.signature(), b.signature());
        b.mixer.push(TrackSnap::new(1.0));
        assert_ne!(a.signature(), b.signature());
    }

    // ---- mixer routing / sidechain / automation (song mode, DC audio clips)

    fn dc_clip(track: usize, level: f32) -> ClipSnap {
        let v = vec![level; 48000 * 3];
        ClipSnap::Audio {
            sample: Arc::new(SampleData::new(v.clone(), v, 48000)),
            start: 0,
            len: PPQ * 4,
            offset: 0,
            gain: 1.0,
            mixer_track: track,
            fade_in: 0,
            fade_out: 0,
        }
    }

    fn song(mixer_tracks: usize) -> Snapshot {
        let mut s = Snapshot::empty();
        s.bpm = 120.0;
        for _ in 1..mixer_tracks {
            s.mixer.push(TrackSnap::new(1.0));
        }
        s.song_end = PPQ * 4;
        s
    }

    fn run_song(snap: Snapshot, frames: usize) -> Vec<f32> {
        let sr = 48000;
        let snap = Arc::new(snap);
        let rack = Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK));
        let mut core = EngineCore::new(sr as f32, snap, rack, Arc::new(SharedState::new(sr)));
        let mut gc = |_g: Garbage| {};
        core.handle(Command::SetMode(PlayMode::Song), &mut gc);
        core.handle(Command::Play, &mut gc);
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        core.process(&mut l, &mut r);
        l
    }

    #[test]
    fn bus_output_and_sends() {
        // track 1 -> bus 2 (muted): silent
        let mut s = song(4);
        s.clips.push(dc_clip(1, 0.25));
        s.mixer[1].output = 2;
        s.mixer[2].muted = true;
        assert!(run_song(s, 4800)[2000].abs() < 1e-6);
        // ... plus a send to track 3: audible through the send only
        let mut s = song(4);
        s.clips.push(dc_clip(1, 0.25));
        s.mixer[1].output = 2;
        s.mixer[2].muted = true;
        s.mixer[1].sends.push((3, 0.5));
        let a = run_song(s, 4800)[2000];
        // direct to master for reference
        let mut s = song(4);
        s.clips.push(dc_clip(1, 0.25));
        let b = run_song(s, 4800)[2000];
        assert!(b > 0.05 && (a - b * 0.5).abs() < 1e-4, "send {a} vs direct {b}");
    }

    #[test]
    fn sidechain_ducks_by_key_track() {
        use tstudio_dsp::fx::{EffectParams, SidechainParams};
        let mut s = song(3);
        s.clips.push(dc_clip(1, 0.5)); // key (muted, still keys)
        s.clips.push(dc_clip(2, 0.25));
        s.mixer[1].muted = true;
        let plain = run_song(s, 9600)[9000];
        let mut s = song(3);
        s.clips.push(dc_clip(1, 0.5));
        s.clips.push(dc_clip(2, 0.25));
        s.mixer[1].muted = true;
        s.mixer[2].fx.push(FxSnap { enabled: true, params: EffectParams::Sidechain(SidechainParams { source: 1, depth: 1.0, ..Default::default() }) });
        let ducked = run_song(s, 9600)[9000];
        assert!(plain > 0.1 && ducked.abs() < 0.01, "plain {plain} ducked {ducked}");
    }

    #[test]
    fn automation_ramps_track_volume() {
        use crate::snapshot::{AutoClipSnap, AutoLane};
        let mut s = song(2);
        s.clips.push(dc_clip(1, 0.5));
        s.autos.push(AutoLane {
            target: AutoTarget::TrackVolume(1),
            log: false,
            clips: vec![AutoClipSnap { start: 0, len: PPQ * 4, points: vec![(0, 0.0), (PPQ * 4, 1.0)] }],
        });
        // 4 beats at 120 bpm = 96000 frames
        let out = run_song(s, 96000);
        let (a, b, c) = (out[1000], out[48000], out[95000]);
        assert!(a < 0.02 && b > a && c > b, "{a} {b} {c}");
    }

    #[test]
    fn swing_shifts_offbeat_sixteenths() {
        use tstudio_project::{Instrument, Note, Project};
        let mut p = Project::new_default();
        p.swing = 1.0;
        let pat = p.patterns[0].id;
        let ch = p.channels[0].id;
        p.pattern_mut(pat).unwrap().notes.extend([0, 24, 48, 72].map(|t| Note { channel: ch, key: 60, start: t, len: 24, velocity: 1.0 }));
        let _ = Instrument::Synth(Default::default());
        let snap = crate::compile_project(&p, &mut |_, _, _| None, false);
        let starts: Vec<i64> = snap.patterns[0].notes.iter().map(|n| n.start).collect();
        assert_eq!(starts, vec![0, 36, 48, 84]);
    }

    #[test]
    fn stretched_audio_clip_loops_sample() {
        // 0.25 s ramp sample, clip of 4 beats (2 s at 120 bpm) → plays 8 times
        let n = 12000;
        let v: Vec<f32> = (0..n).map(|i| 0.1 + 0.4 * i as f32 / n as f32).collect();
        let mut s = song(2);
        s.clips.push(ClipSnap::Audio {
            sample: Arc::new(SampleData::new(v.clone(), v, 48000)),
            start: 0,
            len: PPQ * 4,
            offset: 0,
            gain: 1.0,
            mixer_track: 1,
            fade_in: 0,
            fade_out: 0,
        });
        let out = run_song(s, 96000);
        for k in 0..8 {
            let a = out[k * n + 100];
            let b = out[k * n + n - 100];
            assert!(a > 0.05 && b > a * 2.0, "repeat {k}: {a} {b}");
        }
    }

    #[test]
    fn stretched_pattern_clip_repeats() {
        use tstudio_project::{ClipKind, Note, Project};
        let mut p = Project::new_default();
        p.playlist.clips.clear();
        let pat = p.patterns[0].id;
        let ch = p.channels[0].id;
        // one short note at the start of a 1-bar pattern
        p.pattern_mut(pat).unwrap().notes.push(Note { channel: ch, key: 60, start: 0, len: PPQ / 4, velocity: 1.0 });
        let bar = p.ticks_per_bar();
        p.add_clip(0, 0, bar * 3, ClipKind::Pattern { pattern: pat });
        let snap = Arc::new(crate::compile_project(&p, &mut |_, _, _| None, false));
        let sr = 48000;
        let out = crate::render_offline(snap, sr, PlayMode::Song, 0.0, |_| true).unwrap();
        let spt = sr as f64 * 60.0 / (p.bpm * PPQ as f64);
        let energy = |tick: i64| {
            let s = (tick as f64 * spt) as usize;
            out.left[s..s + 2000].iter().map(|x| x * x).sum::<f32>()
        };
        for k in 0..3 {
            assert!(energy(k * bar) > 1.0, "note in repeat {k}");
            assert!(energy(k * bar + bar / 2) < 1e-3, "silence mid-bar {k}");
        }
    }

    #[test]
    fn playlist_track_volume_scales_notes() {
        use tstudio_project::{ClipKind, Note, Project};
        let render = |vol: f32| {
            let mut p = Project::new_default();
            p.playlist.clips.clear();
            for m in p.mixer.iter_mut() {
                m.fx.clear();
            }
            let pat = p.patterns[0].id;
            let ch = p.channels[0].id;
            p.pattern_mut(pat).unwrap().notes.push(Note { channel: ch, key: 60, start: 0, len: PPQ, velocity: 1.0 });
            p.add_clip(0, 0, p.ticks_per_bar(), ClipKind::Pattern { pattern: pat });
            p.playlist.tracks[0].volume = vol;
            let snap = Arc::new(crate::compile_project(&p, &mut |_, _, _| None, false));
            let out = crate::render_offline(snap, 48000, PlayMode::Song, 0.0, |_| true).unwrap();
            out.left[..20000].iter().map(|x| x * x).sum::<f32>()
        };
        let (full, half) = (render(1.0), render(0.5));
        assert!(full > 1.0);
        assert!((half / full - 0.25).abs() < 0.02, "energy ratio {}", half / full);
    }

    #[test]
    fn channel_solo_silences_other_channels() {
        use tstudio_project::{Instrument, Project};
        let mut p = Project::new_default();
        let extra = p.add_channel("Other", Instrument::Synth(Default::default()));
        p.channels[0].solo = true;
        let snap = crate::compile_project(&p, &mut |_, _, _| None, false);
        assert!(!snap.channels[0].muted);
        assert!(snap.channels[extra].muted);
        p.channels[0].solo = false;
        let snap = crate::compile_project(&p, &mut |_, _, _| None, false);
        assert!(snap.channels.iter().all(|c| !c.muted));
    }

    #[test]
    fn autotune_latency_is_compensated() {
        use tstudio_dsp::autotune::AutotuneParams;
        use tstudio_dsp::fx::EffectParams;
        // a click at beat 1 on insert 1 must come out at the same frame with and without Autotune
        let first_hit = |with_fx: bool| {
            let mut s = song(2);
            let mut v = vec![0.0f32; 4000];
            v[0] = 1.0;
            s.clips.push(ClipSnap::Audio { sample: Arc::new(SampleData::new(v.clone(), v, 48000)), start: PPQ, len: PPQ, offset: 0, gain: 1.0, mixer_track: 1, fade_in: 0, fade_out: 0 });
            if with_fx {
                s.mixer[1].fx.push(FxSnap { enabled: true, params: EffectParams::Autotune(AutotuneParams { mix: 0.0, ..Default::default() }) });
            }
            let out = run_song(s, 48000);
            out.iter().position(|x| x.abs() > 0.1).unwrap()
        };
        let plain = first_hit(false);
        assert_eq!(plain, 24000);
        let tuned = first_hit(true);
        assert!((tuned as i64 - plain as i64).abs() <= 1, "{tuned} vs {plain}");
    }

    #[test]
    fn live_input_plays_through_monitor_insert() {
        let sr = 48000;
        let snap = Arc::new(song(3));
        let rack = Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK));
        let mut core = EngineCore::new(sr as f32, snap, rack, Arc::new(SharedState::new(sr)));
        let mut gc = |_g: Garbage| {};
        let (mut tx, rx) = rtrb::RingBuffer::<f32>::new(48000);
        for _ in 0..4096 {
            tx.push(0.5).unwrap();
        }
        core.handle(Command::SetMonitor(Some(Box::new(MonitorIn::new(rx)))), &mut gc);
        let mut l = vec![0.0; 512];
        let mut r = vec![0.0; 512];
        // not routed → silent, but the queue is drained
        core.process(&mut l, &mut r);
        assert!(l.iter().all(|x| *x == 0.0));
        for _ in 0..4096 {
            let _ = tx.push(0.5);
        }
        core.handle(Command::SetMonitorTrack(Some(2)), &mut gc);
        core.process(&mut l, &mut r);
        assert!(l[300] > 0.3, "monitor level {}", l[300]);
    }
}
