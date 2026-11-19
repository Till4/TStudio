#[test]
fn eq_response_matches_bands() {
    let flat = super::EqParams::default();
    for f in [30.0, 1000.0, 15000.0] {
        assert!(flat.response_db(f, 48000.0).abs() < 0.01);
    }
    let e = super::EqParams { low_gain: 6.0, mid_gain: -9.0, high_gain: 4.0, ..Default::default() };
    assert!((e.response_db(25.0, 48000.0) - 6.0).abs() < 0.5);
    assert!((e.response_db(1000.0, 48000.0) + 9.0).abs() < 0.3);
    assert!((e.response_db(18000.0, 48000.0) - 4.0).abs() < 0.6);
}

use super::*;

#[test]
fn all_effects_stay_finite() {
    let ctx = FxContext { sr: 48000.0, bpm: 140.0, key: &[] };
    for kind in EffectKind::ALL {
        let mut fx = Effect::new(kind, ctx.sr);
        let p = kind.default_params();
        let mut l: Vec<f32> = (0..48000).map(|i| ((i as f32) * 0.03).sin() * 0.9).collect();
        let mut r = l.clone();
        for (cl, cr) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            fx.process(&p, &ctx, cl, cr);
        }
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{} produced NaN/inf", kind.name());
        assert!(l.iter().any(|v| v.abs() > 1e-3), "{} silenced the signal", kind.name());
    }
}

#[test]
fn limiter_respects_ceiling() {
    let ctx = FxContext { sr: 48000.0, bpm: 120.0, key: &[] };
    let mut fx = Effect::new(EffectKind::Limiter, ctx.sr);
    let p = EffectParams::Limiter(LimiterParams { gain_db: 12.0, ceiling_db: -1.0, release_ms: 50.0 });
    let mut l: Vec<f32> = (0..4800).map(|i| ((i as f32) * 0.05).sin()).collect();
    let mut r = l.clone();
    fx.process(&p, &ctx, &mut l, &mut r);
    let c = db_to_gain(-1.0) + 1e-6;
    assert!(l.iter().all(|v| v.abs() <= c));
}

#[test]
fn sidechain_ducks_on_key() {
    let key: Vec<f32> = (0..4800).map(|i| if i < 2400 { 1.0 } else { 0.0 }).collect();
    let ctx = FxContext { sr: 48000.0, bpm: 120.0, key: &key };
    let mut fx = Effect::new(EffectKind::Sidechain, ctx.sr);
    let p = EffectParams::Sidechain(SidechainParams { depth: 1.0, release_ms: 10.0, ..Default::default() });
    let mut l = vec![0.5f32; 4800];
    let mut r = l.clone();
    fx.process(&p, &ctx, &mut l, &mut r);
    assert!(l[1000].abs() < 0.01, "ducked while key plays");
    assert!(l[4700] > 0.45, "recovers after release");
}

#[test]
fn field_accessors_cover_names() {
    for kind in EffectKind::ALL {
        let mut p = kind.default_params();
        let n = p.field_names().len();
        assert!(p.field_mut(n - 1).is_some() && p.field_mut(n).is_none(), "{}", kind.name());
    }
}
