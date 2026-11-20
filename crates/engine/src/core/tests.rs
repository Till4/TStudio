use super::*;
use crate::snapshot::{ChannelSnap, FxSnap, NoteSnap, PatternSnap, TrackSnap};
use tstudio_dsp::sampler::SamplerParams;

pub(super) const IMPULSE: usize = 100;

pub(super) fn click_sample(sr: u32) -> Arc<SampleData> {
    // impulse after the 0.5 ms minimum attack ramp
    let mut v = vec![0.0f32; 2000];
    v[IMPULSE] = 1.0;
    let _ = sr;
    Arc::new(SampleData::new(v.clone(), v, sr))
}

pub(super) fn snapshot_with_notes(starts: &[i64], sr: u32) -> Snapshot {
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
        fit_bpm: 120.0,
        follow: false,
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
        fit_bpm: 120.0,
        follow: false,
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
        s.clips.push(ClipSnap::Audio { sample: Arc::new(SampleData::new(v.clone(), v, 48000)), start: PPQ, len: PPQ, offset: 0, gain: 1.0, mixer_track: 1, fade_in: 0, fade_out: 0, fit_bpm: 120.0, follow: false });
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
