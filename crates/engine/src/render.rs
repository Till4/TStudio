//! Offline (faster than real time) rendering for export.

use crate::core::{Command, EngineCore, Garbage, PlayMode, SharedState, MAX_CHUNK};
use crate::snapshot::{Rack, Snapshot};
use std::sync::Arc;
use tstudio_dsp::SampleData;
use tstudio_project::PPQ;

/// Renders the song (or one pattern) plus a release/reverb tail.
/// `progress` receives 0..1 and returns false to cancel.
pub fn render_offline(snap: Arc<Snapshot>, sr: u32, mode: PlayMode, tail_secs: f64, mut progress: impl FnMut(f32) -> bool) -> Option<SampleData> {
    let rack = Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK));
    let shared = Arc::new(SharedState::new(sr));
    let bar = PPQ * snap.beats_per_bar.max(1) as i64;
    let length_ticks = match mode {
        PlayMode::Song => snap.song_end.max(bar),
        PlayMode::Pattern(i) => snap.patterns.get(i).map(|p| p.length).unwrap_or(bar),
    } as f64;
    let spt = sr as f64 * 60.0 / (snap.bpm * PPQ as f64);
    let main_frames = match mode {
        PlayMode::Song if !snap.tempo.is_constant() => (snap.tempo.secs_at(length_ticks) * sr as f64).ceil() as usize,
        _ => (length_ticks * spt).ceil() as usize,
    };
    let total = main_frames + (tail_secs * sr as f64) as usize;

    let mut core = EngineCore::new(sr as f32, snap, rack, shared);
    core.stop_at_end = true;
    let mut sink = |_g: Garbage| {};
    core.handle(Command::SetMode(mode), &mut sink);
    core.handle(Command::SetPosition(0.0), &mut sink);
    core.handle(Command::Play, &mut sink);

    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    let block = 4096;
    let mut done = 0;
    while done < total {
        let n = (total - done).min(block);
        core.process(&mut l[done..done + n], &mut r[done..done + n]);
        done += n;
        if !progress(done as f32 / total as f32) {
            return None;
        }
    }
    // trim silent tail (keep the full musical length)
    let last = l.iter().zip(&r).rposition(|(a, b)| a.abs() > 1e-4 || b.abs() > 1e-4).unwrap_or(0);
    let keep = (last + (0.05 * sr as f64) as usize).max(main_frames).min(total);
    l.truncate(keep);
    r.truncate(keep);
    Some(SampleData::new(l, r, sr))
}
