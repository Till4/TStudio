//! Loudness / hard-dance effects: oversampled clipper, multiband saturation and an
//! OTT-style multiband upward/downward compressor.

use crate::filter::{FilterMode, Svf};
use crate::fx::{numeric_fields, FxContext};
use crate::{db_to_gain, gain_to_db};
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------ clipper

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClipperParams {
    /// gain into the clipper (dB)
    pub drive_db: f32,
    /// clip level (dBFS)
    pub ceiling_db: f32,
    /// 0 = hard clip, 1 = very soft knee
    pub softness: f32,
    pub output_db: f32,
    pub mix: f32,
}

impl Default for ClipperParams {
    fn default() -> Self {
        Self { drive_db: 3.0, ceiling_db: -0.5, softness: 0.25, output_db: 0.0, mix: 1.0 }
    }
}

numeric_fields!(ClipperParams { drive_db, ceiling_db, softness, output_db, mix });

/// Soft-knee clip curve: linear below `c - k`, smoothly approaching `c`.
#[inline]
pub fn clip_curve(x: f32, c: f32, k: f32) -> f32 {
    let a = x.abs();
    let start = c - k;
    let y = if a <= start {
        a
    } else if k <= 1e-6 {
        c
    } else {
        c - k * (-(a - start) / k).exp()
    };
    y.min(c).copysign(x)
}

#[derive(Default)]
pub struct ClipperFx {
    hist: [[f32; 4]; 2],
    aa: [[Svf; 2]; 2],
    last_sr: f32,
    /// samples above the ceiling in the last block (for metering)
    pub last_clipped: usize,
}

impl ClipperFx {
    pub fn process(&mut self, p: &ClipperParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        const OS: usize = 4;
        if self.last_sr != ctx.sr {
            for ch in self.aa.iter_mut() {
                for f in ch.iter_mut() {
                    f.set(ctx.sr * 0.45, 0.0106, ctx.sr * OS as f32);
                    f.reset();
                }
            }
            self.last_sr = ctx.sr;
        }
        let drive = db_to_gain(p.drive_db);
        let c = db_to_gain(p.ceiling_db.min(0.0));
        let k = c * p.softness.clamp(0.0, 1.0) * 0.5;
        let out = db_to_gain(p.output_db);
        let mut clipped = 0;
        for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
            for v in buf.iter_mut() {
                let h = &mut self.hist[ch];
                h.copy_within(1..4, 0);
                h[3] = *v * drive;
                let dry = h[2] / drive.max(1e-9);
                let mut y = 0.0;
                for s in 0..OS {
                    let t = s as f32 / OS as f32;
                    let up = crate::sample::hermite(h[0], h[1], h[2], h[3], t);
                    if up.abs() > c {
                        clipped += 1;
                    }
                    let z = clip_curve(up, c, k);
                    let z = self.aa[ch][0].process(z, FilterMode::LowPass);
                    y = self.aa[ch][1].process(z, FilterMode::LowPass);
                }
                *v = dry + (y * out - dry) * p.mix;
            }
        }
        self.last_clipped = clipped / OS;
    }
}

// ------------------------------------------------------------------ 3-band split

/// Perfect-reconstruction (subtractive) 3-band splitter with 24 dB/oct lowpasses.
#[derive(Default, Clone, Copy)]
struct Split3 {
    lo: [Svf; 2],
    mid: [Svf; 2],
}

impl Split3 {
    fn set(&mut self, f1: f32, f2: f32, sr: f32) {
        for f in self.lo.iter_mut() {
            f.set(f1, 0.0106, sr);
        }
        for f in self.mid.iter_mut() {
            f.set(f2.max(f1 * 1.5), 0.0106, sr);
        }
    }
    #[inline]
    fn run(&mut self, x: f32) -> [f32; 3] {
        let a = self.lo[0].process(x, FilterMode::LowPass);
        let low = self.lo[1].process(a, FilterMode::LowPass);
        let rest = x - low;
        let b = self.mid[0].process(rest, FilterMode::LowPass);
        let mid = self.mid[1].process(b, FilterMode::LowPass);
        [low, mid, rest - mid]
    }
}

// ------------------------------------------------------------------ multiband saturation

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MultibandParams {
    pub low_freq: f32,
    pub high_freq: f32,
    pub low_drive_db: f32,
    pub mid_drive_db: f32,
    pub high_drive_db: f32,
    pub low_gain_db: f32,
    pub mid_gain_db: f32,
    pub high_gain_db: f32,
    pub mix: f32,
}

impl Default for MultibandParams {
    fn default() -> Self {
        Self { low_freq: 150.0, high_freq: 2500.0, low_drive_db: 6.0, mid_drive_db: 9.0, high_drive_db: 4.0, low_gain_db: 0.0, mid_gain_db: 0.0, high_gain_db: 0.0, mix: 1.0 }
    }
}

numeric_fields!(MultibandParams { low_freq, high_freq, low_drive_db, mid_drive_db, high_drive_db, low_gain_db, mid_gain_db, high_gain_db, mix });

#[derive(Default)]
pub struct MultibandFx {
    split: [Split3; 2],
    last: Option<(f32, f32, f32)>,
}

/// tanh saturation with partial level compensation (louder, but not drive_db louder).
#[inline]
fn sat(x: f32, g: f32) -> f32 {
    (x * g).tanh() / g.sqrt()
}

impl MultibandFx {
    pub fn process(&mut self, p: &MultibandParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let key = (p.low_freq, p.high_freq, ctx.sr);
        if self.last != Some(key) {
            for s in self.split.iter_mut() {
                s.set(p.low_freq, p.high_freq, ctx.sr);
            }
            self.last = Some(key);
        }
        let drive = [db_to_gain(p.low_drive_db), db_to_gain(p.mid_drive_db), db_to_gain(p.high_drive_db)];
        let gain = [db_to_gain(p.low_gain_db), db_to_gain(p.mid_gain_db), db_to_gain(p.high_gain_db)];
        for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
            for v in buf.iter_mut() {
                let b = self.split[ch].run(*v);
                let y: f32 = (0..3).map(|i| sat(b[i], drive[i]) * gain[i]).sum();
                *v += (y - *v) * p.mix;
            }
        }
    }
}

// ------------------------------------------------------------------ OTT

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OttParams {
    /// 0..1 amount (the classic "depth" knob)
    pub depth: f32,
    /// scales attack/release (0.2 = fast … 3 = slow)
    pub time: f32,
    /// 0..1 amount of upward compression (lifts quiet details)
    pub upward: f32,
    /// 0..1 amount of downward compression (tames peaks)
    pub downward: f32,
    /// level where downward compression starts (dBFS)
    pub threshold_db: f32,
    pub in_gain_db: f32,
    pub out_gain_db: f32,
    pub low_gain_db: f32,
    pub mid_gain_db: f32,
    pub high_gain_db: f32,
}

impl Default for OttParams {
    fn default() -> Self {
        Self { depth: 0.4, time: 1.0, upward: 1.0, downward: 1.0, threshold_db: -24.0, in_gain_db: 0.0, out_gain_db: 0.0, low_gain_db: 0.0, mid_gain_db: 0.0, high_gain_db: 0.0 }
    }
}

numeric_fields!(OttParams { depth, time, upward, downward, threshold_db, in_gain_db, out_gain_db, low_gain_db, mid_gain_db, high_gain_db });

#[derive(Default)]
pub struct OttFx {
    split: [Split3; 2],
    env: [f32; 3],
    last_sr: f32,
    /// current gain change per band in dB (for metering)
    pub last_gain_db: [f32; 3],
}

impl OttFx {
    pub fn process(&mut self, p: &OttParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        if self.last_sr != ctx.sr {
            for s in self.split.iter_mut() {
                s.set(120.0, 2500.0, ctx.sr);
            }
            self.last_sr = ctx.sr;
        }
        let t = p.time.clamp(0.05, 10.0);
        // classic OTT band timings (ms)
        let att_ms = [47.8 * t, 22.4 * t, 13.5 * t];
        let rel_ms = [282.0 * t, 282.0 * t, 132.0 * t];
        let coef = |ms: f32| (-1.0 / (ms.max(0.05) * 0.001 * ctx.sr)).exp();
        let att = [coef(att_ms[0]), coef(att_ms[1]), coef(att_ms[2])];
        let rel = [coef(rel_ms[0]), coef(rel_ms[1]), coef(rel_ms[2])];
        let ing = db_to_gain(p.in_gain_db);
        let outg = db_to_gain(p.out_gain_db);
        let band_g = [db_to_gain(p.low_gain_db), db_to_gain(p.mid_gain_db), db_to_gain(p.high_gain_db)];
        let thr_d = p.threshold_db;
        let thr_u = p.threshold_db - 14.0;
        let depth = p.depth.clamp(0.0, 1.0);
        for i in 0..l.len() {
            let bl = self.split[0].run(l[i] * ing);
            let br = self.split[1].run(r[i] * ing);
            let (mut yl, mut yr) = (0.0, 0.0);
            for b in 0..3 {
                let x = bl[b].abs().max(br[b].abs());
                let c = if x > self.env[b] { att[b] } else { rel[b] };
                self.env[b] = x + (self.env[b] - x) * c;
                let lv = gain_to_db(self.env[b]);
                let mut g = 0.0;
                if lv > thr_d {
                    g -= p.downward * (lv - thr_d) * (1.0 - 1.0 / 66.7);
                } else if lv < thr_u && lv > -70.0 {
                    g += (p.upward * (thr_u - lv) * (1.0 - 1.0 / 4.17)).min(24.0);
                }
                let g = g * depth;
                self.last_gain_db[b] = g;
                let gl = db_to_gain(g) * band_g[b];
                yl += bl[b] * gl;
                yr += br[b] * gl;
            }
            l[i] = yl * outg;
            r[i] = yr * outg;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> FxContext<'static> {
        FxContext { sr: 48000.0, bpm: 150.0, key: &[] }
    }

    #[test]
    fn clipper_holds_ceiling() {
        let mut fx = ClipperFx::default();
        let p = ClipperParams { drive_db: 12.0, ceiling_db: -1.0, softness: 0.0, output_db: 0.0, mix: 1.0 };
        let mut l: Vec<f32> = (0..9600).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut r = l.clone();
        fx.process(&p, &ctx(), &mut l, &mut r);
        let c = db_to_gain(-1.0);
        // the anti-alias filter may ring a little above the ceiling
        assert!(l[200..].iter().all(|v| v.abs() < c * 1.12), "max {}", l.iter().fold(0.0f32, |a, b| a.max(b.abs())));
        assert!(fx.last_clipped > 0);
    }

    #[test]
    fn split_reconstructs() {
        let mut s = Split3::default();
        s.set(150.0, 2500.0, 48000.0);
        let x: Vec<f32> = (0..4800).map(|i| ((i as f32) * 0.37).sin() * 0.5 + ((i as f32) * 0.011).sin() * 0.3).collect();
        for v in &x {
            let b = s.run(*v);
            assert!((b.iter().sum::<f32>() - v).abs() < 1e-5);
        }
    }

    #[test]
    fn ott_lifts_quiet_and_tames_loud() {
        let mut fx = OttFx::default();
        let p = OttParams { depth: 1.0, ..Default::default() };
        let quiet = |amp: f32| -> f32 {
            let mut fx2 = OttFx::default();
            let mut l: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.06).sin() * amp).collect();
            let mut r = l.clone();
            fx2.process(&p, &ctx(), &mut l, &mut r);
            l[24000..].iter().fold(0.0f32, |a, b| a.max(b.abs())) / amp
        };
        assert!(quiet(0.001) > 2.0, "upward: {}", quiet(0.001));
        assert!(quiet(0.9) < 0.7, "downward: {}", quiet(0.9));
        let mut l = vec![0.0f32; 512];
        let mut r = l.clone();
        fx.process(&p, &ctx(), &mut l, &mut r);
        assert!(l.iter().all(|v| v.is_finite()));
    }
}
