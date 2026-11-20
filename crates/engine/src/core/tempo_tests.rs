//! Tempo track: onsets, note lengths and render length follow tempo changes and ramps.

use super::tests::{click_sample, snapshot_with_notes, IMPULSE};
use super::*;
use tstudio_project::{TempoMap, TempoPoint};

const SR: u32 = 48000;

/// Song with one pattern clip of `beats` beats holding a note on every beat.
fn song_with_beats(beats: i64, tempo: &[TempoPoint]) -> Snapshot {
    let starts: Vec<i64> = (0..beats).map(|b| b * PPQ).collect();
    let mut s = snapshot_with_notes(&starts, SR);
    s.patterns[0].length = beats * PPQ;
    s.clips.push(ClipSnap::Pattern { pattern: 0, start: 0, len: beats * PPQ, gain: 1.0 });
    s.song_end = beats * PPQ;
    s.tempo = TempoMap::new(s.bpm, tempo);
    s
}

fn onsets(out: &[f32]) -> Vec<usize> {
    out.iter().enumerate().filter(|(_, v)| **v > 0.5).map(|(i, _)| i - IMPULSE).collect()
}

fn render(s: Snapshot) -> Vec<f32> {
    crate::render_offline(Arc::new(s), SR, PlayMode::Song, 0.0, |_| true).unwrap().left
}

#[test]
fn tempo_jump_moves_onsets() {
    // bar 1 at 120 bpm (0.5 s per beat), from bar 2 on 60 bpm (1 s per beat)
    let s = song_with_beats(8, &[TempoPoint { tick: 4 * PPQ, bpm: 60.0, ramp: false }]);
    let out = render(s);
    let sec = SR as usize;
    assert_eq!(onsets(&out), vec![0, sec / 2, sec, sec * 3 / 2, 2 * sec, 3 * sec, 4 * sec, 5 * sec]);
    // render length = 2 s + 4 s
    assert!(out.len() >= 6 * sec, "length {}", out.len());
}

#[test]
fn tempo_ramp_accelerates_smoothly() {
    // 120 → 240 bpm linearly over the first bar, then 240
    let b0 = 120.0f64;
    let b1 = 240.0f64;
    let ramp_len = 4.0 * PPQ as f64;
    let s = song_with_beats(8, &[TempoPoint { tick: 4 * PPQ, bpm: b1, ramp: true }]);
    let out = render(s);
    let k = (b1 - b0) / ramp_len;
    let secs = |t: f64| {
        if t <= ramp_len {
            60.0 / (PPQ as f64 * k) * ((b0 + k * t) / b0).ln()
        } else {
            60.0 / (PPQ as f64 * k) * (b1 / b0).ln() + (t - ramp_len) * 60.0 / (PPQ as f64 * b1)
        }
    };
    let got = onsets(&out);
    assert_eq!(got.len(), 8, "{got:?}");
    for (beat, &frame) in got.iter().enumerate() {
        let expect = secs((beat as i64 * PPQ) as f64) * SR as f64;
        // a ramp is linearised per chunk (≤ 256 frames): stays within a couple of samples
        assert!((frame as f64 - expect).abs() <= 2.0, "beat {beat}: {frame} vs {expect}");
    }
    // the gaps shrink while accelerating
    let gaps: Vec<usize> = got.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps[0] > gaps[1] && gaps[1] > gaps[2] && gaps[2] > gaps[3], "{gaps:?}");
    assert_eq!(gaps[4], gaps[5]);
}

#[test]
fn pattern_mode_ignores_tempo_track() {
    let mut s = snapshot_with_notes(&[0, 96, 192, 288], SR);
    s.tempo = TempoMap::new(120.0, &[TempoPoint { tick: 96, bpm: 60.0, ramp: false }]);
    let snap = Arc::new(s);
    let rack = Box::new(Rack::build(&snap, SR as f32, MAX_CHUNK));
    let mut core = EngineCore::new(SR as f32, snap, rack, Arc::new(SharedState::new(SR)));
    let mut gc = |_g: Garbage| {};
    core.handle(Command::SetMode(PlayMode::Pattern(0)), &mut gc);
    core.handle(Command::Play, &mut gc);
    let mut l = vec![0.0; 96000];
    let mut r = vec![0.0; 96000];
    core.process(&mut l, &mut r);
    assert_eq!(onsets(&l), vec![0, 24000, 48000, 72000]);
}

#[test]
fn following_audio_clip_tracks_song_position() {
    // ramp sample 0..1 over 2 s; fitted at 120 bpm, so 1 beat of song = 0.5 s of sample
    let n = SR as usize * 2;
    let v: Vec<f32> = (0..n).map(|i| i as f32 / n as f32 * 0.8).collect();
    let mk = |follow: bool| {
        let mut s = Snapshot::empty();
        s.mixer.push(crate::snapshot::TrackSnap::new(1.0));
        s.song_end = PPQ * 4;
        s.clips.push(ClipSnap::Audio {
            sample: Arc::new(SampleData::new(v.clone(), v.clone(), SR)),
            start: 0,
            len: PPQ * 4,
            offset: 0,
            gain: 1.0,
            mixer_track: 1,
            fade_in: 0,
            fade_out: 0,
            fit_bpm: 120.0,
            follow,
        });
        // from beat 2 on half tempo: beat 3 is reached at 1.5 s instead of 1.0 s
        s.tempo = TempoMap::new(120.0, &[TempoPoint { tick: PPQ, bpm: 60.0, ramp: false }]);
        render(s)
    };
    let at = |out: &[f32], secs: f64| out[(secs * SR as f64) as usize];
    let followed = mk(true);
    let real_time = mk(false);
    // at 1.5 s the song is at beat 2 (tick 192) → following clip reads sample time 1.0 s
    assert!((at(&followed, 1.5) - 0.4).abs() < 0.01, "follow {}", at(&followed, 1.5));
    // real-time clip reads sample time 1.5 s
    assert!((at(&real_time, 1.5) - 0.6).abs() < 0.01, "real time {}", at(&real_time, 1.5));
}

#[test]
fn compile_fits_clips_to_tempo_at_their_start() {
    use tstudio_dsp::stretch::StretchMode;
    use tstudio_project::{AudioClip, ClipKind, Project, StretchSettings};
    let mut p = Project::new_default();
    p.bpm = 140.0;
    p.set_tempo_point(4 * PPQ, 150.0, false);
    let st = StretchSettings { mode: StretchMode::Stretch, source_bpm: 128.0, ..Default::default() };
    p.add_clip(1, 8 * PPQ, 4 * PPQ, ClipKind::Audio(AudioClip { path: "x.wav".into(), stretch: st, ..Default::default() }));
    let mut asked = Vec::new();
    let snap = crate::compile_project(
        &p,
        &mut |_, _, bpm| {
            asked.push(bpm);
            Some(Arc::new(SampleData::new(vec![0.0; 10], vec![0.0; 10], SR)))
        },
        true,
    );
    assert_eq!(asked, vec![150.0]);
    assert!(matches!(snap.clips[0], ClipSnap::Audio { follow: true, fit_bpm, .. } if fit_bpm == 150.0));
    assert!(!snap.tempo.is_constant());
    let _ = click_sample(SR);
}
