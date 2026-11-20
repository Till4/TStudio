use super::*;

fn trim_with(d: SampleData) -> Trim {
    let mut t = Trim::default();
    let len = d.len();
    t.set_loaded(PathBuf::from("song.mp3"), Loaded { peaks: Peaks::of(&d), hits: vec![0, len / 2], data: d });
    t
}

#[test]
fn render_range_trims_tail_normalizes_and_isolates() {
    let sr = 44100;
    // 0.2 s of 100 Hz + 8 kHz, then 0.3 s silence
    let mut l: Vec<f32> = (0..sr / 5).map(|i| {
        let x = i as f32 / sr as f32;
        0.2 * (2.0 * std::f32::consts::PI * 100.0 * x).sin() + 0.2 * (2.0 * std::f32::consts::PI * 8000.0 * x).sin()
    }).collect();
    l.extend(std::iter::repeat(0.0).take(sr * 3 / 10));
    let mut t = trim_with(SampleData::new(l.clone(), l, sr as u32));
    let len = t.data.as_ref().unwrap().len();
    let out = render_range(&t, 0, len).unwrap();
    assert!(out.len() < sr / 5 + sr / 100, "tail removed: {}", out.len());
    assert!((out.peak() - 0.98).abs() < 0.01, "normalized");
    // isolate the bass: the 8 kHz part must be gone
    t.iso = IsolateParams { lp_hz: 300.0, ..Default::default() };
    t.iso_on = true;
    t.normalize = false;
    let lo = render_range(&t, 0, len).unwrap();
    let hf: f32 = lo.left.windows(2).skip(2000).take(4000).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / 4000.0;
    assert!(hf < 0.01, "high frequencies left: {hf}");
}

#[test]
fn hits_and_numbered_paths() {
    let d = SampleData::new(vec![0.1; 1000], vec![0.1; 1000], 44100);
    let t = trim_with(d);
    assert_eq!(t.hit_around(100), (0, 500));
    assert_eq!(t.hit_around(700), (500, 1000));
    let dir = std::env::temp_dir().join(format!("tstudio_cut_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p1 = numbered_path(&dir, "song Kick", "wav");
    assert!(p1.ends_with("song Kick 01.wav"));
    std::fs::write(&p1, b"x").unwrap();
    assert!(numbered_path(&dir, "song Kick", "wav").ends_with("song Kick 02.wav"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn moved_selection_keeps_width_and_stays_in_file() {
    assert_eq!(moved_selection((100, 300), 500, 1000), (500, 700));
    assert_eq!(moved_selection((100, 300), 900, 1000), (800, 1000));
    assert_eq!(moved_selection((100, 300), 0, 1000), (0, 200));
}

#[test]
fn nudge_moves_by_beat_and_fine_step() {
    // 4 beats in a 4000-sample loop: one beat = 1000 samples
    assert_eq!(beat_step((1000, 5000), 4, 48000, 48000.0, 120.0), 1000);
    // whole file selected: one beat of the project tempo (120 BPM @ 48 kHz = 24000)
    assert_eq!(beat_step((0, 48000), 4, 48000, 48000.0, 120.0), 24000);
    assert_eq!(fine_step(48000.0), 48);
    assert_eq!(fine_step(10.0), 1);
    assert_eq!(nudged_selection((1000, 5000), 1000, 48000), (2000, 6000));
    assert_eq!(nudged_selection((1000, 5000), -1500, 48000), (0, 4000));
    assert_eq!(nudged_selection((1000, 5000), 100_000, 48000), (44000, 48000));
}

#[test]
fn markers_add_jump_and_step() {
    let d = SampleData::new(vec![0.1; 1000], vec![0.1; 1000], 44100);
    let mut t = trim_with(d);
    t.sel = (100, 200);
    assert_eq!(t.add_marker(), 0);
    assert_eq!(t.add_marker(), 0, "same selection is not stored twice");
    t.sel = (500, 700);
    assert_eq!(t.add_marker(), 1);
    assert_eq!(t.markers[0].name, "Marker 1");
    assert_eq!(t.markers[1].name, "Marker 2");
    t.markers.remove(0);
    assert_eq!(next_marker_name(&t.markers), "Marker 1");
    t.sel = (0, 1000);
    t.add_marker();
    t.markers.push(CutMarker { name: "Snare".into(), sel: (300, 400) });
    // markers by start: (0,1000)=1, (300,400)=2, (500,700)=0
    assert_eq!(marker_step(&t.markers, 300, true), Some(0));
    assert_eq!(marker_step(&t.markers, 300, false), Some(1));
    assert_eq!(marker_step(&t.markers, 500, true), None);
    t.jump_marker(2);
    assert_eq!(t.sel, (300, 400));
    assert_eq!(marker_label("  "), "Marker");
    // a newly loaded file starts without markers
    t.set_loaded(PathBuf::from("other.wav"), Loaded { peaks: Peaks::of(&SampleData::new(vec![0.0; 10], vec![0.0; 10], 44100)), hits: vec![], data: SampleData::new(vec![0.0; 10], vec![0.0; 10], 44100) });
    assert!(t.markers.is_empty());
}

#[test]
fn save_target_names() {
    let base = Path::new("cuts");
    assert_eq!(save_target(base, "song", Some(0), None), (base.join("Kick"), "song Kick".to_string()));
    assert_eq!(save_target(base, "song", Some(1), Some("Fill")), (base.join("Snare"), "song Fill Snare".to_string()));
    assert_eq!(save_target(base, "song", None, Some("A")), (base.to_path_buf(), "song A".to_string()));
}
