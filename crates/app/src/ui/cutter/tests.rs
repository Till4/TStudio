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
