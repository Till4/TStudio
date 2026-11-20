//! Per-chunk helpers: automation lookup, FX chain run, event sequencing (notes, clips, swing, stretch) and latency calculation.

use super::*;

/// Automated value for a whole-track / channel target, if a lane is active.
pub(super) fn track_auto(snap: &Snapshot, auto: &AutoVals, target: AutoTarget) -> Option<f32> {
    snap.autos.iter().take(MAX_AUTO).enumerate().find(|(_, l)| l.target == target).and_then(|(i, _)| auto[i])
}

/// Runs a track's effect chain; automated parameters are applied on a stack copy (no allocation).
#[allow(clippy::too_many_arguments)]
pub(super) fn run_fx(fx: &mut [Effect], slots: &[FxSnap], track: usize, snap: &Snapshot, auto: &AutoVals, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
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

pub(super) fn apply_event(snap: &Snapshot, rack: &mut Rack, ev: &Event) {
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

/// `bpm` = current song tempo (tempo-synced synth LFOs).
pub(super) fn render_channels(snap: &Snapshot, rack: &mut Rack, auto: &AutoVals, bpm: f32, from: usize, to: usize) {
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
                s.bpm = bpm;
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
pub(super) fn push_event(events: &mut Vec<Event>, ev: Event) {
    if events.len() < MAX_EVENTS {
        events.push(ev);
    }
}

/// Advances the transport over one chunk: collects note/click events and renders audio clips.
/// Returns the new position and whether playback reached the end (only with `stop_at_end`).
#[allow(clippy::too_many_arguments)]
pub(super) fn sequence(
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
    let srf = sr as f64;
    // the tempo track only applies to the song; patterns loop at the start tempo
    let song = mode == PlayMode::Song;
    let tempo = &snap.tempo;
    let const_bpm = |a: f64, b: f64| if song { tempo.constant_between(a, b) } else { Some(snap.bpm) };
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
        // one piece of constant tempo (exact), or a ramp linearised over at most one chunk
        let next_change = if song { tempo.next_change_after(cur) } else { f64::INFINITY };
        let (seg_ticks, spt) = match const_bpm(cur, cur) {
            Some(bpm) => {
                let spt = samples_per_tick(srf, bpm);
                (((total - frame0) / spt).min(hi - cur).min(next_change - cur), spt)
            }
            None => {
                let s0 = tempo.secs_at(cur);
                let end = tempo.tick_at(s0 + (total - frame0) / srf).min(hi).min(next_change);
                let ticks = end - cur;
                let spt = if ticks > 1e-9 { (tempo.secs_at(end) - s0) * srf / ticks } else { samples_per_tick(srf, tempo.bpm_at(cur)) };
                (ticks, spt)
            }
        };
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
                let len = note.len as f64;
                let dur = match const_bpm(t, t + len) {
                    Some(bpm) => len * samples_per_tick(srf, bpm),
                    None => (tempo.secs_at(t + len) - tempo.secs_at(t)) * srf,
                };
                push_event(
                    events,
                    Event {
                        frame: to_frame(t),
                        channel: note.channel,
                        key: note.key,
                        vel: note.velocity,
                        gain,
                        dur: Some(dur.max(1.0) as u64),
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
                if let ClipSnap::Audio { sample, start, len, offset, gain, mixer_track, fade_in, fade_out, fit_bpm, follow } = clip {
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
                    let spt_fit = samples_per_tick(srf, *fit_bpm);
                    // tempo-following clips read by song position (at the tempo they were fitted to),
                    // the others play in real time from the clip start
                    let by_ticks = *follow || tempo.is_constant();
                    let secs_a = if by_ticks { 0.0 } else { tempo.secs_at(a) - tempo.secs_at(cs) };
                    for f in fs..fe {
                        let t = a + (f as f64 - frame0) / spt;
                        let local = t - cs;
                        let pos = if by_ticks { (local + *offset as f64) * spt_fit } else { (secs_a + (f as f64 - frame0) / srf) * srf + *offset as f64 * spt_fit };
                        let idx = pos.max(0.0) as usize;
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
