//! Engine-level count-in tests (recording): timing in pattern mode and in song mode with
//! time-signature and tempo changes.

use super::tests::{snapshot_with_notes, IMPULSE};
use super::*;
use tstudio_project::{BarMap, MeterChange, TempoMap, TempoPoint};

/// Runs `core` for `frames` frames in odd blocks; returns the output and where the count-in
/// handed over to playback (absolute frame, position).
fn run_count_in(core: &mut EngineCore, frames: usize) -> (Vec<f32>, Option<(usize, f64)>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    let mut started = None;
    let mut base = 0;
    for (cl, cr) in l.chunks_mut(1000).zip(r.chunks_mut(1000)) {
        core.process(cl, cr);
        if let Some((f, pos)) = core.take_started_at() {
            started = Some((base + f, pos));
        }
        base += cl.len();
    }
    (l, started)
}

#[test]
fn count_in_delays_the_song_by_whole_bars() {
    let sr = 48000;
    let snap = Arc::new(snapshot_with_notes(&[0], sr));
    let rack = Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK));
    let mut core = EngineCore::new(sr as f32, snap, rack, Arc::new(SharedState::new(sr)));
    let mut gc = |_g: Garbage| {};
    core.handle(Command::SetMode(PlayMode::Pattern(0)), &mut gc);
    core.handle(Command::PlayCountIn(1), &mut gc);
    assert!(!core.is_playing() && core.is_counting_in());
    let (l, started) = run_count_in(&mut core, 96000 + 24000);
    // one 4/4 bar at 120 bpm = 96000 frames, then the note at tick 0
    assert_eq!(started, Some((96000, 0.0)));
    assert!(core.is_playing() && !core.is_counting_in());
    let onsets: Vec<usize> = l.iter().enumerate().filter(|(_, v)| **v > 0.9).map(|(i, _)| i - IMPULSE).collect();
    assert_eq!(onsets, vec![96000]);
    // the count-in itself is audible (clicks before the song)
    assert!(l[..96000].iter().any(|v| v.abs() > 0.1));
    // stop cancels a running count-in
    core.handle(Command::Stop, &mut gc);
    core.handle(Command::PlayCountIn(2), &mut gc);
    core.handle(Command::Stop, &mut gc);
    assert!(!core.is_counting_in() && !core.is_playing());
}

#[test]
fn song_count_in_follows_meter_and_tempo_at_the_start() {
    // bar 2 (0-based 1) is 7/8 at 90 bpm: one bar of count-in = 7 eighths = 3.5 beats at 90 bpm
    let sr = 48000;
    let mut s = Snapshot::empty();
    s.bpm = 120.0;
    s.bars = BarMap::new(4, &[MeterChange { bar: 1, beats_per_bar: 7, beat_unit: 8 }]);
    s.tempo = TempoMap::new(120.0, &[TempoPoint { tick: PPQ * 4, bpm: 90.0, ramp: false }]);
    s.song_end = PPQ * 16;
    let snap = Arc::new(s);
    let rack = Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK));
    let mut core = EngineCore::new(sr as f32, snap, rack, Arc::new(SharedState::new(sr)));
    let mut gc = |_g: Garbage| {};
    core.handle(Command::SetMode(PlayMode::Song), &mut gc);
    core.handle(Command::SetPosition((PPQ * 4) as f64), &mut gc);
    core.handle(Command::PlayCountIn(1), &mut gc);
    let (_, started) = run_count_in(&mut core, 150_000);
    assert_eq!(started, Some((112_000, (PPQ * 4) as f64)));
}
