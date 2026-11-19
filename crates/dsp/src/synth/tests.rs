use super::*;

const SR: f32 = 48000.0;

fn render(p: &SynthParams, notes: &[(u8, f32, u64)], secs: f32) -> (Vec<f32>, Vec<f32>) {
    let mut s = Synth::new(SR, 16);
    for &(k, v, d) in notes {
        s.note_on(p, k, v, Some(d));
    }
    let n = (secs * SR) as usize;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    for (cl, cr) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
        s.render(p, cl, cr);
    }
    (l, r)
}

fn assert_clean(name: &str, buf: &[f32]) {
    for (i, v) in buf.iter().enumerate() {
        assert!(v.is_finite(), "{name}: NaN/Inf at {i}");
        assert!(*v == 0.0 || v.abs() >= f32::MIN_POSITIVE, "{name}: denormal {v:e} at {i}");
    }
}

fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(buf: &[f32]) -> f32 {
    (buf.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / buf.len().max(1) as f64).sqrt() as f32
}

/// frequency from rising zero crossings (linear interpolation) inside `a..b`
fn zc_freq(buf: &[f32], a: usize, b: usize) -> f32 {
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0;
    for i in a.max(1)..b {
        if buf[i - 1] <= 0.0 && buf[i] > 0.0 {
            let t = (i - 1) as f32 + buf[i - 1] / (buf[i - 1] - buf[i]);
            if first.is_none() {
                first = Some(t);
            } else {
                count += 1;
            }
            last = t;
        }
    }
    count as f32 * SR / (last - first.unwrap_or(last)).max(1.0)
}

#[test]
fn legacy_projects_sound_identical() {
    // presets of the first TSynth version as stored in old projects (only the old fields),
    // with fingerprints rendered by the old engine before the expansion
    const FP: [(&str, f64, f32, f32, f32); 9] = [
        ("Init", 5.780289019733463e2, 1.0891766e-1, 3.0619842e-1, 3.2901328e-2),
        ("Bass", 3.1293169329343486e3, -1.9840623e-1, 2.8678453e-1, 3.811069e-2),
        ("Lead", 7.932618488797511e2, -1.1047889e-1, -5.5530444e-2, -4.1731935e-2),
        ("Pad", 2.4907774621399852e1, 1.0146212e-2, 3.0306503e-2, -2.3368e-3),
        ("Pluck", 3.5527081581943577e2, -1.087239e-1, 2.0040855e-2, 4.280942e-3),
        ("808 Sub", 3.2366456764032487e3, 7.1639076e-2, 5.6279385e-1, -2.9730982e-1),
        ("Hard Kick", 2.2392286636957874e3, -7.507005e-2, 6.8230546e-1, -8.247431e-3),
        ("Acid", 1.761267255934134e3, -4.1389665e-1, -3.6407086e-1, -4.78434e-2),
        ("Hard Stab", 2.651999018524897e2, -3.2792622e-1, -1.6448808e-1, -2.2993014e-4),
    ];
    let legacy: Vec<(String, SynthParams)> = ron::from_str(include_str!("../testdata/legacy_synth_presets.ron")).expect("old presets load");
    assert_eq!(legacy.len(), FP.len());
    for ((name, p), (fname, e0, a0, b0, c0)) in legacy.iter().zip(FP) {
        assert_eq!(name, fname);
        let mut s = Synth::new(SR, 8);
        s.note_on(p, 60, 1.0, Some(4800));
        s.note_on(p, 64, 0.5, Some(9600));
        let mut l = vec![0.0f32; 48000];
        let mut r = vec![0.0f32; 48000];
        for (cl, cr) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            s.render(p, cl, cr);
        }
        let e: f64 = l.iter().chain(&r).map(|v| (*v as f64) * (*v as f64)).sum();
        assert!(((e - e0) / e0).abs() < 1e-5, "{name}: energy {e} vs {e0}");
        for (got, want) in [(l[1000], a0), (r[2000], b0), (l[10000], c0)] {
            assert!((got - want).abs() < 1e-5, "{name}: sample {got} vs {want}");
        }
    }
}

#[test]
fn all_presets_clean_audible_and_unclipped() {
    for (cat, name, p) in SynthParams::presets_by_category() {
        // chords for the new poly presets; the original ones keep their original levels
        const LEGACY: [&str; 9] = ["Init", "Bass", "Lead", "Pad", "Pluck", "808 Sub", "Hard Kick", "Acid", "Hard Stab"];
        // slow attacks (pads, risers) get a note long enough to open up
        let len = ((p.amp_env.attack * 1.2 * SR) as u64).max(9600);
        let chord: Vec<(u8, f32, u64)> = if cat == "Kick" || cat == "Bass" || p.mono || LEGACY.contains(&name) { vec![(60, 1.0, len)] } else { vec![(60, 1.0, len), (64, 1.0, len), (67, 1.0, len)] };
        let chord = &chord[..];
        // long enough for every release and FX tail
        let (l, r) = render(&p, chord, if p.fx.reverb_on || p.fx.delay_on || p.amp_env.attack > 1.0 { 16.0 + p.amp_env.attack * 1.2 } else { 4.0 });
        assert_clean(name, &l);
        assert_clean(name, &r);
        let pk = peak(&l).max(peak(&r));
        println!("PEAK {name}: {pk:.3} ({:.1} dBFS)", crate::gain_to_db(pk));
        if std::env::var("TSYNTH_PEAKS").is_ok() {
            continue;
        }
        assert!(pk > 0.05, "{name}: too quiet ({pk})");
        assert!(pk < 1.0, "{name}: clips ({pk})");
        // it ends: the last 100 ms are silent
        let tail = &l[l.len() - 4800..];
        assert!(peak(tail) < 1e-3, "{name}: still sounding at the end ({})", peak(tail));
    }
}

#[test]
fn presets_have_categories_and_round_trip() {
    let all = SynthParams::presets_by_category();
    for c in PRESET_CATEGORIES {
        assert!(all.iter().filter(|(cat, _, _)| *cat == c).count() >= 2, "category {c} needs presets");
    }
    for (_, name, p) in all {
        let s = ron::to_string(&p).unwrap();
        let back: SynthParams = ron::from_str(&s).unwrap();
        assert_eq!(back, p, "{name} round trip");
    }
}

#[test]
fn kick_pitch_envelope_lands_on_the_fundamental() {
    // clean engine: zero crossings after the sweep match the tuned note exactly
    for tune in [28.0f32, 29.0, 31.0, 33.0, 36.0] {
        let p = SynthParams {
            osc1: OscParams { wave: Waveform::Sine, level: 1.0, ..OscParams::default() },
            osc2: OscParams::off(),
            cutoff: 18000.0,
            filter_env_amount: 0.0,
            key_track: 0.0,
            amp_env: AdsrParams::adsr(0.0005, 2.0, 0.0, 0.05),
            kick: KickParams { enabled: true, tune, click_level: 0.0, pitch_start: 48.0, pitch_time: 0.08, ..KickParams::default() },
            ..SynthParams::default()
        };
        // any key plays the tuned kick
        for key in [36u8, 60, 72] {
            let (l, _) = render(&p, &[(key, 1.0, 48000)], 0.6);
            let f = zc_freq(&l, (0.1 * SR) as usize, (0.5 * SR) as usize);
            let want = midi_to_freq(tune);
            assert!((f / want - 1.0).abs() < 0.003, "tune {tune} key {key}: {f} Hz vs {want} Hz");
            // the hit starts four octaves higher
            let f0 = zc_freq(&l, 0, (0.004 * SR) as usize);
            assert!(f0 > want * 5.0, "sweep start {f0}");
        }
        assert!((p.kick.freq_at(60, 0.2) - midi_to_freq(tune)).abs() < 1e-3);
    }
    // key follow transposes relative to C4
    let mut p = SynthParams { osc2: OscParams::off(), kick: KickParams { enabled: true, key_follow: true, click_level: 0.0, ..KickParams::default() }, ..SynthParams::default() };
    p.osc1 = OscParams { wave: Waveform::Sine, level: 1.0, ..OscParams::default() };
    p.cutoff = 18000.0;
    p.amp_env = AdsrParams::adsr(0.0005, 2.0, 0.0, 0.05);
    let (l, _) = render(&p, &[(72, 1.0, 48000)], 0.6);
    let f = zc_freq(&l, (0.15 * SR) as usize, (0.5 * SR) as usize);
    assert!((f / midi_to_freq(p.kick.tune + 12.0) - 1.0).abs() < 0.003, "key follow {f}");
}

#[test]
fn kick_presets_hit_their_tuning() {
    for (cat, name, p) in SynthParams::presets_by_category() {
        if cat != "Kick" {
            continue;
        }
        assert!(p.kick.enabled, "{name}");
        let (l, r) = render(&p, &[(60, 1.0, 24000)], 1.5);
        let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        let f = crate::music::fundamental(&mono, SR as u32).expect("fundamental");
        let want = midi_to_freq(p.kick.tune) as f64;
        let cents = 1200.0 * (f.hz / want).log2();
        assert!(cents.abs() < 40.0, "{name}: {} Hz ({}), want {want:.1} Hz ({cents:.0} ct)", f.hz, f.note);
        let pk = peak(&l).max(peak(&r));
        assert!(pk > 0.5 && pk < 1.0, "{name}: peak {pk}");
    }
}

#[test]
fn unison_spreads_stereo_and_blend_works() {
    let base = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, unison: 7, detune: 30.0, spread: 1.0, level: 0.7, ..OscParams::default() },
        osc2: OscParams::off(),
        cutoff: 18000.0,
        filter_env_amount: 0.0,
        amp_env: AdsrParams::adsr(0.001, 0.1, 1.0, 0.05),
        ..SynthParams::default()
    };
    let corr = |l: &[f32], r: &[f32]| {
        let (mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64);
        for (a, b) in l.iter().zip(r) {
            lr += (*a as f64) * (*b as f64);
            ll += (*a as f64) * (*a as f64);
            rr += (*b as f64) * (*b as f64);
        }
        lr / (ll * rr).sqrt().max(1e-12)
    };
    let (l, r) = render(&base, &[(57, 1.0, 48000)], 1.0);
    let wide = corr(&l[4800..], &r[4800..]);
    assert!(wide < 0.8, "spread unison should decorrelate L/R (corr {wide})");
    let narrow = SynthParams { osc1: OscParams { spread: 0.0, ..base.osc1.clone() }, ..base.clone() };
    let (l, r) = render(&narrow, &[(57, 1.0, 48000)], 1.0);
    assert!(l.iter().zip(&r).all(|(a, b)| a == b), "no spread = mono");
    // blend 0: only the centre voice remains → one saw, perfectly periodic
    let single = SynthParams { osc1: OscParams { blend: 0.0, ..base.osc1.clone() }, ..base.clone() };
    let (l, r) = render(&single, &[(57, 1.0, 48000)], 1.0);
    let c = corr(&l[4800..], &r[4800..]);
    assert!(c > 0.999, "blend 0 leaves the centred voice ({c})");
    // 9 voices work and stay finite
    let nine = SynthParams { osc1: OscParams { unison: 9, ..base.osc1.clone() }, ..base };
    let (l, _) = render(&nine, &[(57, 1.0, 4800)], 0.5);
    assert_clean("nine", &l);
}

/// energy of the first difference relative to the signal: a brightness measure
fn brightness(buf: &[f32]) -> f32 {
    let d: Vec<f32> = buf.windows(2).map(|w| w[1] - w[0]).collect();
    rms(&d) / rms(buf).max(1e-9)
}

#[test]
fn mod_matrix_routes_take_effect() {
    let mut p = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, level: 0.7, ..OscParams::default() },
        osc2: OscParams::off(),
        filter_type: FilterType::Svf24,
        cutoff: 300.0,
        resonance: 0.0,
        filter_env_amount: 0.0,
        key_track: 0.0,
        amp_env: AdsrParams::adsr(0.001, 0.1, 1.0, 0.05),
        ..SynthParams::default()
    };
    assert!(p.add_mod(ModSource::Macro1, ModDest::Cutoff1, 0.8));
    let (dark, _) = render(&p, &[(48, 1.0, 24000)], 0.5);
    p.macro1 = 1.0;
    let (bright, _) = render(&p, &[(48, 1.0, 24000)], 0.5);
    let (bd, bb) = (brightness(&dark[2400..]), brightness(&bright[2400..]));
    assert!(bb > bd * 2.0, "macro → cutoff opens the filter ({bd} → {bb})");

    // LFO → amp (matrix): tempo-synced 1/4 note tremolo at 120 BPM = 2 Hz
    let mut q = SynthParams { cutoff: 18000.0, ..p.clone() };
    q.mod_matrix = [ModSlot::default(); NUM_MOD_SLOTS];
    q.lfo1 = LfoParams { wave: Waveform::Sine, sync: true, beats: 1.0, retrig: true, ..LfoParams::default() };
    q.add_mod(ModSource::Lfo1, ModDest::Amp, -1.0);
    let (l, _) = render(&q, &[(48, 1.0, 96000)], 2.0);
    let win = 1200; // 25 ms
    let env: Vec<f32> = l.chunks(win).map(rms).collect();
    let (mx, mn) = env[4..].iter().fold((0.0f32, f32::MAX), |(a, b), v| (a.max(*v), b.min(*v)));
    assert!(mn < mx * 0.2, "deep tremolo ({mn} .. {mx})");
    // minima every 0.5 s
    let mins: Vec<usize> = (5..env.len() - 1).filter(|&i| env[i] < env[i - 1] && env[i] <= env[i + 1] && env[i] < mx * 0.3).collect();
    assert!(mins.len() >= 3, "{mins:?}");
    for w in mins.windows(2) {
        let dt = (w[1] - w[0]) as f32 * win as f32 / SR;
        assert!((dt - 0.5).abs() < 0.06, "tremolo period {dt}");
    }

    // velocity → pitch: a soft note is lower than a hard one
    let mut v = SynthParams { cutoff: 18000.0, ..p.clone() };
    v.mod_matrix = [ModSlot::default(); NUM_MOD_SLOTS];
    v.osc1.wave = Waveform::Sine;
    v.add_mod(ModSource::Velocity, ModDest::Pitch, 0.5);
    let (soft, _) = render(&v, &[(57, 0.0, 24000)], 0.5);
    let (hard, _) = render(&v, &[(57, 1.0, 24000)], 0.5);
    let fs = zc_freq(&soft, 4800, 20000);
    let fh = zc_freq(&hard, 4800, 20000);
    assert!((fs - 220.0).abs() < 2.0 && (fh / fs - 2.0).abs() < 0.02, "velocity pitch {fs} {fh}");

    // macro → global FX destination
    let mut d = p.clone();
    d.mod_matrix = [ModSlot::default(); NUM_MOD_SLOTS];
    d.fx.reverb_on = true;
    d.fx.reverb.mix = 0.0;
    d.add_mod(ModSource::Macro2, ModDest::FxReverbMix, 1.0);
    d.macro2 = 0.0;
    let (dry, _) = render(&d, &[(48, 1.0, 4800)], 2.0);
    d.macro2 = 0.8;
    let (wet, _) = render(&d, &[(48, 1.0, 4800)], 2.0);
    let tail = (0.6 * SR) as usize..(1.2 * SR) as usize;
    assert!(rms(&wet[tail.clone()]) > 10.0 * rms(&dry[tail]).max(1e-7), "macro opens the reverb");
}

#[test]
fn wavetable_position_and_warp_change_the_sound() {
    let p = SynthParams {
        osc1: OscParams { table: WtKind::Morph, position: 0.0, level: 0.7, ..OscParams::default() },
        osc2: OscParams::off(),
        cutoff: 18000.0,
        filter_env_amount: 0.0,
        amp_env: AdsrParams::adsr(0.001, 0.1, 1.0, 0.05),
        ..SynthParams::default()
    };
    let (sine, _) = render(&p, &[(57, 1.0, 24000)], 0.5);
    let saw = SynthParams { osc1: OscParams { position: 0.5, ..p.osc1.clone() }, ..p.clone() };
    let (saw, _) = render(&saw, &[(57, 1.0, 24000)], 0.5);
    assert!(brightness(&saw[2400..]) > brightness(&sine[2400..]) * 3.0);
    for w in WarpMode::ALL {
        for t in WtKind::ALL {
            let q = SynthParams {
                osc1: OscParams { table: t, warp: w, warp_amount: 0.7, position: 0.6, ..p.osc1.clone() },
                osc2: OscParams { enabled: true, wave: Waveform::Sine, level: 0.2, ..OscParams::default() },
                osc3: OscParams { enabled: true, table: WtKind::Fm, level: 0.2, ..OscParams::default() },
                sub_level: 0.2,
                ..p.clone()
            };
            let (l, r) = render(&q, &[(45, 1.0, 4800), (81, 1.0, 4800)], 0.3);
            assert_clean(w.name(), &l);
            assert_clean(t.name(), &r);
            assert!(peak(&l) > 0.01);
        }
    }
}

#[test]
fn filter_types_and_routing_render_cleanly() {
    for ft in FilterType::ALL {
        for routing in [FilterRouting::Serial, FilterRouting::Parallel] {
            let p = SynthParams {
                filter_type: ft,
                resonance: 0.9,
                filter_drive: 12.0,
                filter2: Filter2Params { enabled: true, kind: FilterType::Ladder, mode: FilterMode::LowPass, cutoff: 2000.0, resonance: 0.95, drive_db: 6.0, ..Filter2Params::default() },
                filter_routing: routing,
                noise_level: 0.3,
                noise_type: NoiseType::Crackle,
                ..SynthParams::default()
            };
            let (l, r) = render(&p, &[(36, 1.0, 9600), (84, 1.0, 9600)], 1.5);
            assert_clean(ft.name(), &l);
            assert_clean(ft.name(), &r);
            assert!(peak(&l) < 10.0, "{} resonance stays bounded ({})", ft.name(), peak(&l));
        }
    }
}

#[test]
fn mono_legato_and_retrigger() {
    let p = SynthParams {
        osc1: OscParams { wave: Waveform::Sine, level: 0.7, ..OscParams::default() },
        osc2: OscParams::off(),
        mono: true,
        legato: true,
        cutoff: 18000.0,
        amp_env: AdsrParams::adsr(0.05, 0.1, 1.0, 0.05),
        ..SynthParams::default()
    };
    let mut s = Synth::new(SR, 16);
    s.note_on(&p, 57, 1.0, None);
    let mut l = vec![0.0f32; 9600];
    let mut r = vec![0.0f32; 9600];
    s.render(&p, &mut l, &mut r);
    // legato: the envelope does not restart → no dip in level
    s.note_on(&p, 60, 1.0, None);
    let mut l2 = vec![0.0f32; 480];
    let mut r2 = vec![0.0f32; 480];
    s.render(&p, &mut l2, &mut r2);
    assert!(peak(&l2) > 0.3, "legato keeps the level ({})", peak(&l2));
    assert_eq!(s.active_voices(), 1);
    // retrigger: the attack restarts from the current level and rises again
    let q = SynthParams { legato: false, ..p };
    let mut s = Synth::new(SR, 16);
    s.note_on(&q, 57, 1.0, None);
    s.render(&q, &mut l, &mut r);
    s.note_on(&q, 60, 1.0, None);
    let mut l3 = vec![0.0f32; 480];
    s.render(&q, &mut l3, &mut r2);
    assert!(peak(&l3[..48]) < 0.1, "retrigger restarts the envelope ({})", peak(&l3[..48]));
    assert_eq!(s.active_voices(), 1);
}

#[test]
fn kick_is_a_one_shot() {
    let p = SynthParams::presets().into_iter().find(|(n, _)| *n == "Schranz Kick").unwrap().1;
    let (short, _) = render(&p, &[(60, 1.0, 480)], 1.0);
    let (long, _) = render(&p, &[(60, 1.0, 48000)], 1.0);
    assert_eq!(short, long, "note length must not cut the kick");
}

#[test]
fn polyphony_limits_voices() {
    let p = SynthParams { polyphony: 3, amp_env: AdsrParams::adsr(0.001, 0.1, 1.0, 0.05), ..SynthParams::default() };
    let mut s = Synth::new(SR, 16);
    for k in 60..68 {
        s.note_on(&p, k, 1.0, None);
    }
    assert_eq!(s.active_voices(), 3);
}

#[test]
fn fx_tail_keeps_ringing_then_stops() {
    let p = SynthParams { fx: SynthFxParams { reverb_on: true, reverb: ReverbParams { mix: 0.5, ..ReverbParams::default() }, ..SynthFxParams::default() }, amp_env: AdsrParams::adsr(0.001, 0.05, 0.0, 0.02), ..SynthParams::default() };
    let mut s = Synth::new(SR, 16);
    s.note_on(&p, 60, 1.0, Some(480));
    let mut l = vec![0.0f32; 24000];
    let mut r = vec![0.0f32; 24000];
    s.render(&p, &mut l, &mut r);
    assert_eq!(s.active_voices(), 0);
    assert!(s.is_sounding(), "reverb tail");
    assert!(rms(&l[12000..]) > 1e-4);
    for _ in 0..20 {
        l.fill(0.0);
        r.fill(0.0);
        s.render(&p, &mut l, &mut r);
    }
    assert!(!s.is_sounding());
}

#[test]
fn automatable_fields_keep_their_order() {
    let old = ["cutoff", "resonance", "filter_env_amount", "key_track", "sub_level", "noise_level", "glide", "gain", "velocity_sens", "pitch_env_amount", "pitch_env_decay", "drive_db"];
    assert_eq!(&SynthParams::FIELDS[..old.len()], &old);
    let mut p = SynthParams::default();
    for i in 0..SynthParams::FIELDS.len() {
        *p.field_mut(i).unwrap() = 0.123;
        assert_eq!(p.field(i), Some(0.123));
    }
    assert!(p.field_mut(SynthParams::FIELDS.len()).is_none());
    let mut names = SynthParams::FIELDS.to_vec();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), SynthParams::FIELDS.len());
}

#[test]
fn custom_lfo_and_envelope_curves_are_finite() {
    let mut p = SynthParams::default();
    p.lfo3 = LfoParams { shape: LfoShape::Custom, smooth: true, sync: true, beats: 0.25, ..LfoParams::default() };
    p.lfo4 = LfoParams { shape: LfoShape::SampleHold, rate: 8.0, ..LfoParams::default() };
    p.add_mod(ModSource::Lfo3, ModDest::Cutoff1, 0.5);
    p.add_mod(ModSource::Lfo4, ModDest::Pan, 0.5);
    p.add_mod(ModSource::Random, ModDest::Osc1Pitch, 0.02);
    p.add_mod(ModSource::Note, ModDest::Reso1, 0.3);
    p.amp_env = AdsrParams { attack: 0.02, hold: 0.05, decay: 0.2, sustain: 0.4, release: 0.3, attack_curve: 0.8, decay_curve: -0.6 };
    let (l, r) = render(&p, &[(40, 1.0, 9600), (76, 0.3, 9600)], 1.5);
    assert_clean("lfo", &l);
    assert_clean("lfo", &r);
}

/// RMS in dB of `buf[a_ms..b_ms]`
fn seg_db(buf: &[f32], a_ms: usize, b_ms: usize) -> f32 {
    let s = &buf[a_ms * 48..b_ms * 48];
    20.0 * ((s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt() + 1e-9).log10()
}

/// The amp envelope runs before the kick drive: more drive must hold the body up longer
/// (fat bass) instead of only adding grit to a fading tone.
#[test]
fn kick_drive_holds_the_body() {
    let base = SynthParams::presets().into_iter().find(|(n, _)| *n == "Hard Kick").unwrap().1;
    let mut clean = base.clone();
    clean.kick.drive_db = 0.0;
    let mut hot = base.clone();
    hot.kick.drive_db = 20.0;
    let (lc, _) = render(&clean, &[(60, 1.0, 48000)], 0.5);
    let (lh, _) = render(&hot, &[(60, 1.0, 48000)], 0.5);
    let drop_clean = seg_db(&lc, 0, 50) - seg_db(&lc, 200, 250);
    let drop_hot = seg_db(&lh, 0, 50) - seg_db(&lh, 200, 250);
    assert!(drop_hot + 6.0 < drop_clean, "drive should hold the tail: clean drop {drop_clean:.1} dB, driven {drop_hot:.1} dB");
}

/// Schranz/Hardtekk kicks keep their bass for ~250 ms and are gone before the next beat at 160 BPM.
#[test]
fn fat_kicks_keep_their_bass() {
    for name in ["Hard Kick", "Schranz Kick", "Hardtekk Kick", "Fat Kick", "Tekk Bass Kick"] {
        let p = SynthParams::presets().into_iter().find(|(n, _)| *n == name).unwrap().1;
        let (l, _) = render(&p, &[(60, 1.0, 48000)], 0.6);
        let start = seg_db(&l, 0, 50);
        let body = seg_db(&l, 200, 250);
        let tail = seg_db(&l, 500, 550);
        assert!(start > -8.0, "{name}: body too quiet ({start:.1} dB RMS)");
        assert!(body > start - 6.0, "{name}: bass dies too early ({start:.1} → {body:.1} dB at 200 ms)");
        assert!(tail < start - 9.0, "{name}: tail too long for 150–180 BPM ({tail:.1} dB at 500 ms)");
    }
}

/// Kick presets choke: at 160 BPM the next hit cuts the previous tail instead of stacking bodies.
#[test]
fn kick_presets_choke_their_tail() {
    let beat = (48000.0 * 60.0 / 160.0) as usize;
    for (cat, name, p) in SynthParams::presets_by_category() {
        if cat != "Kick" {
            continue;
        }
        let mut s = Synth::new(SR, 16);
        let mut l = vec![0.0f32; beat * 3];
        let mut r = vec![0.0f32; beat * 3];
        for (cl, cr) in l.chunks_mut(beat).zip(r.chunks_mut(beat)) {
            s.note_on(&p, 60, 1.0, Some(4800));
            for (a, c) in cl.chunks_mut(256).zip(cr.chunks_mut(256)) {
                s.render(&p, a, c);
            }
        }
        assert_clean(name, &l);
        let active = s.voices.iter().filter(|v| v.active).count();
        assert!(active <= 1, "{name}: {active} voices ringing at once");
        assert!(peak(&l) < 1.0, "{name}: peak {}", peak(&l));
    }
}

/// 303 slide: with `slide_only` a detached note jumps, a tied (overlapping) note glides.
#[test]
fn slide_only_glides_tied_notes() {
    let p = SynthParams { mono: true, legato: true, glide: 0.2, slide_only: true, ..SynthParams::default() };
    let mut l = vec![0.0f32; 256];
    let mut r = vec![0.0f32; 256];
    // detached: first note ends before the next starts
    let mut s = Synth::new(SR, 16);
    s.note_on(&p, 48, 1.0, Some(64));
    s.render(&p, &mut l, &mut r);
    s.note_on(&p, 60, 1.0, Some(64));
    assert_eq!(s.voices[0].pitch, 60.0, "detached note must not glide");
    // tied: the next note starts while the first is held
    let mut s = Synth::new(SR, 16);
    s.note_on(&p, 48, 1.0, None);
    s.render(&p, &mut l, &mut r);
    s.note_on(&p, 60, 1.0, None);
    s.render(&p, &mut l[..32], &mut r[..32]);
    assert!(s.voices[0].pitch < 59.0, "tied note must slide, pitch {}", s.voices[0].pitch);
    // without slide_only every mono note glides (old behaviour)
    let q = SynthParams { slide_only: false, ..p.clone() };
    let mut s = Synth::new(SR, 16);
    s.note_on(&q, 48, 1.0, Some(64));
    s.render(&q, &mut l, &mut r);
    s.note_on(&q, 60, 1.0, Some(64));
    assert!(s.voices[0].pitch < 59.0);
}
