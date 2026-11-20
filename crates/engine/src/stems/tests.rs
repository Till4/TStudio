use super::*;
use crate::compile_project;
use std::path::PathBuf;
use tstudio_project::{AudioClip, MixerSend, MixerTrack};

const SR: u32 = 48000;

/// Project without channels, `n` inserts, one audio clip per (insert, file) pair.
fn project(n: usize, clips: &[(usize, &str)]) -> Project {
    let mut p = Project::default();
    p.mixer.truncate(1);
    for i in 1..=n {
        p.mixer.push(MixerTrack { name: format!("Insert {i}"), volume: 1.0, ..Default::default() });
    }
    let bar = p.ticks_per_bar();
    for (k, (track, file)) in clips.iter().enumerate() {
        p.add_clip(k, 0, bar, ClipKind::Audio(AudioClip { path: PathBuf::from(file), mixer_track: *track, ..Default::default() }));
    }
    p
}

/// Deterministic test audio: "a" = 0.2 DC-ish sine, anything else = quieter, different sine.
fn sample_for(path: &std::path::Path) -> Arc<SampleData> {
    let f = if path.to_string_lossy() == "a" { 220.0 } else { 1330.0 };
    let amp = if path.to_string_lossy() == "a" { 0.2 } else { 0.12 };
    let v: Vec<f32> = (0..SR as usize * 3).map(|i| (std::f32::consts::TAU * f * i as f32 / SR as f32).sin() * amp).collect();
    Arc::new(SampleData::new(v.clone(), v, SR))
}

fn compile(p: &Project) -> Snapshot {
    compile_project(p, &mut |path, _, _| Some(sample_for(path)), true)
}

#[test]
fn plans_inserts_and_buses() {
    // 1 → send → 2 (send-only bus); 3 → output → 4 (bus); 5 muted
    let mut p = project(5, &[(1, "a"), (3, "b"), (5, "a")]);
    p.mixer[1].sends.push(MixerSend { target: 2, amount: 0.5 });
    p.mixer[3].output = 4;
    p.mixer[5].muted = true;
    let jobs = stem_jobs(&p);
    let tracks: Vec<(usize, bool, bool)> = jobs.iter().map(|j| (j.track, j.bus, j.without.is_some())).collect();
    assert_eq!(tracks, vec![(1, false, false), (2, true, true), (3, false, false), (4, true, false)]);
    for j in &jobs {
        assert!(j.with.mixer[0].fx.iter().all(|f| !f.enabled), "stems are pre-master");
    }
    // insert 1 alone: other buses muted, the bus its output does not pass through
    assert!(jobs[0].with.mixer[1].solo && jobs[0].with.mixer[2].muted && jobs[0].with.mixer[4].muted);
    // insert 3 keeps its own output bus 4
    assert!(!jobs[2].with.mixer[4].muted);
}

#[test]
fn stems_add_up_to_the_premaster_mix() {
    let mut p = project(3, &[(1, "a"), (2, "b")]);
    p.mixer[2].sends.push(MixerSend { target: 3, amount: 0.5 });
    let jobs = compile_stems(stem_jobs(&p), compile);
    assert_eq!(jobs.len(), 3);
    let mut last = 0.0f32;
    let stems = render_stems(jobs, SR, PlayMode::Song, 0.0, |f| {
        assert!(f >= last - 1e-6 && f <= 1.0 + 1e-6);
        last = f;
        true
    })
    .unwrap();
    assert!((last - 1.0).abs() < 1e-6);
    let mut full = p.clone();
    for f in full.mixer[0].fx.iter_mut() {
        f.enabled = false;
    }
    let mix = render_offline(Arc::new(compile(&full)), SR, PlayMode::Song, 0.0, |_| true).unwrap();
    let n = stems.iter().map(|s| s.data.len()).min().unwrap().min(mix.len());
    assert!(n > SR as usize);
    let mut err = 0.0f32;
    for i in 0..n {
        let sum: f32 = stems.iter().map(|s| s.data.left[i]).sum();
        err = err.max((sum - mix.left[i]).abs());
    }
    assert!(err < 1e-4, "stems differ from the mix by {err}");
    // the send-only bus stem carries only the send (0.5 × insert 2)
    let bus = stems.iter().find(|s| s.track == 3).unwrap();
    let ins2 = stems.iter().find(|s| s.track == 2).unwrap();
    let peak = |d: &SampleData| d.left[..n].iter().fold(0.0f32, |a, b| a.max(b.abs()));
    assert!((peak(&bus.data) / peak(&ins2.data) - 0.5).abs() < 0.01);
}

#[test]
fn cancel_stops_rendering() {
    let p = project(2, &[(1, "a"), (2, "b")]);
    let jobs = compile_stems(stem_jobs(&p), compile);
    assert!(render_stems(jobs, SR, PlayMode::Song, 0.0, |_| false).is_none());
}

#[test]
fn file_names_are_safe() {
    assert_eq!(stem_file_name(3, "Kick", "wav"), "03 Kick.wav");
    assert_eq!(stem_file_name(12, "Bass/Sub: \"Hard\"", "wav"), "12 Bass_Sub_ _Hard_.wav");
    assert_eq!(stem_file_name(1, "  ", "mp3"), "01 Insert.mp3");
}

#[test]
fn pattern_mode_stems() {
    let p = project(1, &[(1, "a")]);
    // pattern mode ignores audio clips: one (silent) stem per insert with content
    let jobs = compile_stems(stem_jobs(&p), compile);
    let stems = render_stems(jobs, SR, PlayMode::Pattern(0), 0.0, |_| true).unwrap();
    assert_eq!(stems.len(), 1);
    assert!(stems[0].data.left.iter().all(|v| v.abs() < 1e-6));
}

#[test]
fn stems_follow_the_tempo_map() {
    // 2 bars of audio; a tempo jump to double speed at bar 2 shortens the song like the master
    let mut p = project(1, &[(1, "a")]);
    let bar = p.ticks_per_bar();
    p.playlist.clips[0].len = bar * 2;
    p.bpm = 120.0;
    p.set_tempo_point(bar, 240.0, false);
    let master = render_offline(Arc::new(compile(&p)), SR, PlayMode::Song, 0.0, |_| true).unwrap();
    let stems = render_stems(compile_stems(stem_jobs(&p), compile), SR, PlayMode::Song, 0.0, |_| true).unwrap();
    assert_eq!(stems.len(), 1);
    assert_eq!(stems[0].data.left.len(), master.left.len());
    // bar 1 at 120 bpm = 2 s, bar 2 at 240 bpm = 1 s
    assert!((master.left.len() as f64 / SR as f64 - 3.0).abs() < 0.05, "{}", master.left.len());
}
