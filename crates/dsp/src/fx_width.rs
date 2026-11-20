//! Stereo width tool: mid/side width (0–200 %), mid/side gain and optional bass mono
//! (side signal high-passed below a crossover, so the low end stays mono-compatible).

use crate::db_to_gain;
use crate::filter::{FilterMode, Svf};
use crate::fx::{numeric_fields, FxContext};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WidthParams {
    /// 0 = mono, 1 = unchanged, 2 = 200 % (side doubled)
    pub width: f32,
    /// gain of the mid (L+R) signal in dB
    pub mid_db: f32,
    /// gain of the side (L−R) signal in dB
    pub side_db: f32,
    /// everything below this frequency becomes mono (0 = off)
    pub bass_mono_hz: f32,
}

impl Default for WidthParams {
    fn default() -> Self {
        Self { width: 1.0, mid_db: 0.0, side_db: 0.0, bass_mono_hz: 0.0 }
    }
}

numeric_fields!(WidthParams { width, mid_db, side_db, bass_mono_hz });

/// Lowest crossover the bass-mono filter accepts; below it the option counts as off.
pub const BASS_MONO_MIN_HZ: f32 = 20.0;

#[derive(Default)]
pub struct WidthFx {
    /// 24 dB/oct high-pass on the side signal (two Butterworth stages)
    hp: [Svf; 2],
    last: Option<(f32, f32)>,
}

impl WidthFx {
    pub fn process(&mut self, p: &WidthParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let bass_mono = p.bass_mono_hz >= BASS_MONO_MIN_HZ;
        if bass_mono && self.last != Some((p.bass_mono_hz, ctx.sr)) {
            for f in self.hp.iter_mut() {
                f.set(p.bass_mono_hz, 0.0106, ctx.sr);
            }
            if self.last.is_none() {
                for f in self.hp.iter_mut() {
                    f.reset();
                }
            }
            self.last = Some((p.bass_mono_hz, ctx.sr));
        } else if !bass_mono && self.last.is_some() {
            for f in self.hp.iter_mut() {
                f.reset();
            }
            self.last = None;
        }
        let mid_g = db_to_gain(p.mid_db);
        let side_g = db_to_gain(p.side_db) * p.width.clamp(0.0, 2.0);
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let m = 0.5 * (*a + *b) * mid_g;
            let mut s = 0.5 * (*a - *b);
            if bass_mono {
                let x = self.hp[0].process(s, FilterMode::HighPass);
                s = self.hp[1].process(x, FilterMode::HighPass);
            }
            s *= side_g;
            *a = m + s;
            *b = m - s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> FxContext<'static> {
        FxContext { sr: 48000.0, bpm: 140.0, key: &[] }
    }

    fn tone(freq: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| (std::f32::consts::TAU * freq * i as f32 / 48000.0).sin()).collect()
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |a, b| a.max(b.abs()))
    }

    /// Stereo test signal: shared mid tone + independent side tone.
    fn stereo(len: usize) -> (Vec<f32>, Vec<f32>) {
        let m = tone(440.0, len);
        let s = tone(1300.0, len);
        let l = m.iter().zip(&s).map(|(m, s)| 0.4 * m + 0.3 * s).collect();
        let r = m.iter().zip(&s).map(|(m, s)| 0.4 * m - 0.3 * s).collect();
        (l, r)
    }

    #[test]
    fn default_is_transparent() {
        let (l0, r0) = stereo(4800);
        let (mut l, mut r) = (l0.clone(), r0.clone());
        WidthFx::default().process(&WidthParams::default(), &ctx(), &mut l, &mut r);
        assert!(l.iter().zip(&l0).chain(r.iter().zip(&r0)).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn mono_sum_is_unchanged_at_any_width() {
        let (l0, r0) = stereo(4800);
        for width in [0.0, 0.5, 1.5, 2.0] {
            let (mut l, mut r) = (l0.clone(), r0.clone());
            let p = WidthParams { width, side_db: 3.0, bass_mono_hz: 120.0, ..Default::default() };
            WidthFx::default().process(&p, &ctx(), &mut l, &mut r);
            for i in 0..l.len() {
                assert!(((l[i] + r[i]) - (l0[i] + r0[i])).abs() < 1e-5, "width {width} sample {i}");
            }
        }
    }

    #[test]
    fn zero_width_is_mono_and_200_percent_doubles_side() {
        let (l0, r0) = stereo(4800);
        let (mut l, mut r) = (l0.clone(), r0.clone());
        WidthFx::default().process(&WidthParams { width: 0.0, ..Default::default() }, &ctx(), &mut l, &mut r);
        assert!(l.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1e-6));
        let (mut l, mut r) = (l0.clone(), r0.clone());
        WidthFx::default().process(&WidthParams { width: 2.0, ..Default::default() }, &ctx(), &mut l, &mut r);
        let side: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a - b)).collect();
        let side0: Vec<f32> = l0.iter().zip(&r0).map(|(a, b)| 0.5 * (a - b)).collect();
        assert!((peak(&side) / peak(&side0) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn mid_and_side_gain() {
        let (l0, r0) = stereo(4800);
        let (mut l, mut r) = (l0.clone(), r0.clone());
        let p = WidthParams { mid_db: -6.0, side_db: -120.0, ..Default::default() };
        WidthFx::default().process(&p, &ctx(), &mut l, &mut r);
        let mid: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        assert!((peak(&mid) - 0.4 * db_to_gain(-6.0)).abs() < 0.01);
        assert!(l.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1e-4));
    }

    #[test]
    fn bass_mono_removes_low_side_but_keeps_high_side() {
        // pure side signals (L = -R): a 50 Hz one must vanish, a 3 kHz one must survive
        let run = |freq: f32| {
            let s = tone(freq, 48000);
            let mut l: Vec<f32> = s.iter().map(|v| v * 0.5).collect();
            let mut r: Vec<f32> = s.iter().map(|v| -v * 0.5).collect();
            let mut fx = WidthFx::default();
            let p = WidthParams { bass_mono_hz: 150.0, ..Default::default() };
            for (cl, cr) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
                fx.process(&p, &ctx(), cl, cr);
            }
            let side: Vec<f32> = l[24000..].iter().zip(&r[24000..]).map(|(a, b)| 0.5 * (a - b)).collect();
            peak(&side) / 0.5
        };
        let low = run(50.0);
        let high = run(3000.0);
        assert!(low < 0.15, "50 Hz side kept {low}");
        assert!(high > 0.97 && high < 1.03, "3 kHz side {high}");
        // a mono bass is untouched by bass mono
        let m = tone(50.0, 9600);
        let (mut l, mut r) = (m.clone(), m.clone());
        WidthFx::default().process(&WidthParams { bass_mono_hz: 150.0, ..Default::default() }, &ctx(), &mut l, &mut r);
        assert!(l.iter().zip(&m).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
