//! Procedurally generated, band-limited wavetables for TSynth. No files: every table is
//! computed once from formulas (first use, off the audio thread) and shared read-only.

use crate::fft::fft;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};
use std::sync::OnceLock;

/// samples per single-cycle frame
pub const TABLE_LEN: usize = 2048;
/// morph frames per table
pub const FRAMES: usize = 16;
/// band-limited copies per frame (one per octave)
pub const MIPS: usize = 11;
/// oversampled length used to generate the frames before band-limiting
const GEN_LEN: usize = 8192;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
pub enum WtKind {
    /// Classic analog shapes chosen by `wave` (old TSynth oscillators, PolyBLEP)
    #[default]
    Basic,
    /// Sine → Triangle → Saw → Square → thin Pulse
    Morph,
    /// Pulse width 50 % → 3 %
    Pulse,
    /// Additive: 1 → 64 harmonics
    Harmonics,
    /// Saw through vowel formants A-E-I-O-U
    Vowel,
    /// Hard-sync sweep (slave 1× → 8×)
    SyncSweep,
    /// Sine wavefolder, gentle → brutal
    Fold,
    /// Bit-reduced / stepped sine
    Digital,
    /// 2-op FM (ratio 3), index 0 → 8
    Fm,
    /// Saturated sine: clean body → square-ish (kick bodies)
    DriveSine,
    /// Detuned/hollow "Hoover" style cycle
    Hollow,
}

impl WtKind {
    pub const ALL: [WtKind; 11] = [
        WtKind::Basic,
        WtKind::Morph,
        WtKind::Pulse,
        WtKind::Harmonics,
        WtKind::Vowel,
        WtKind::SyncSweep,
        WtKind::Fold,
        WtKind::Digital,
        WtKind::Fm,
        WtKind::DriveSine,
        WtKind::Hollow,
    ];
    pub fn name(self) -> &'static str {
        match self {
            WtKind::Basic => "Analog",
            WtKind::Morph => "Morph",
            WtKind::Pulse => "Pulse",
            WtKind::Harmonics => "Harmonics",
            WtKind::Vowel => "Vowel",
            WtKind::SyncSweep => "Sync Sweep",
            WtKind::Fold => "Fold",
            WtKind::Digital => "Digital",
            WtKind::Fm => "FM",
            WtKind::DriveSine => "Drive Sine",
            WtKind::Hollow => "Hollow",
        }
    }
    fn index(self) -> usize {
        WtKind::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
    fn frames(self) -> usize {
        if self == WtKind::Basic {
            4
        } else {
            FRAMES
        }
    }
}

pub struct Wavetable {
    pub frames: usize,
    /// [(frame * MIPS + mip) * TABLE_LEN + i]
    data: Vec<f32>,
}

impl Wavetable {
    #[inline]
    fn slice(&self, frame: usize, mip: usize) -> &[f32] {
        let o = (frame * MIPS + mip) * TABLE_LEN;
        &self.data[o..o + TABLE_LEN]
    }

    /// One frame at full bandwidth (for drawing).
    pub fn frame(&self, frame: usize) -> &[f32] {
        self.slice(frame.min(self.frames - 1), 0)
    }

    /// Mip level that keeps the playback alias-free (`dt` = freq / sample rate).
    #[inline]
    pub fn mip_for(dt: f32) -> usize {
        let x = dt.abs() * TABLE_LEN as f32 * 0.85;
        if x <= 1.0 {
            0
        } else {
            (x.log2().ceil() as usize).min(MIPS - 1)
        }
    }

    #[inline]
    fn read(t: &[f32], phase: f32) -> f32 {
        let p = phase * TABLE_LEN as f32;
        let i = p as usize;
        let f = p - i as f32;
        let a = t[i & (TABLE_LEN - 1)];
        let b = t[(i + 1) & (TABLE_LEN - 1)];
        a + (b - a) * f
    }

    /// `pos` 0..1 morphs through the frames, `phase` 0..1.
    #[inline]
    pub fn sample(&self, pos: f32, phase: f32, mip: usize) -> f32 {
        let fp = pos.clamp(0.0, 1.0) * (self.frames - 1) as f32;
        let f0 = fp as usize;
        let fr = fp - f0 as f32;
        let a = Self::read(self.slice(f0, mip), phase);
        if fr <= 1e-6 || f0 + 1 >= self.frames {
            a
        } else {
            let b = Self::read(self.slice(f0 + 1, mip), phase);
            a + (b - a) * fr
        }
    }

    /// Exact frame (no morph), used for the classic shapes when warped.
    #[inline]
    pub fn sample_frame(&self, frame: usize, phase: f32, mip: usize) -> f32 {
        Self::read(self.slice(frame.min(self.frames - 1), mip), phase)
    }
}

static BANK: OnceLock<Vec<Wavetable>> = OnceLock::new();

/// All tables, generated on first call (call once off the audio thread, e.g. in `Synth::new`).
pub fn bank() -> &'static [Wavetable] {
    BANK.get_or_init(|| WtKind::ALL.iter().map(|k| build(*k)).collect())
}

#[inline]
pub fn table(kind: WtKind) -> &'static Wavetable {
    &bank()[kind.index()]
}

fn saw(t: f64) -> f64 {
    2.0 * t - 1.0
}
fn square(t: f64, w: f64) -> f64 {
    if t < w {
        1.0
    } else {
        -1.0
    }
}
fn tri(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t - 1.0
    } else {
        3.0 - 4.0 * t
    }
}
fn lerp(a: f64, b: f64, x: f64) -> f64 {
    a + (b - a) * x
}

/// Time-domain value of frame position `x` (0..1) at phase `t` (0..1).
fn shape(kind: WtKind, frame: usize, x: f64, t: f64) -> f64 {
    let s = (TAU * t).sin();
    match kind {
        WtKind::Basic => match frame {
            0 => s,
            1 => saw(t),
            2 => square(t, 0.5),
            _ => tri(t),
        },
        WtKind::Morph => {
            let seg = x * 4.0;
            let i = (seg.floor() as usize).min(3);
            let f = seg - i as f64;
            let w = [s, tri((t + 0.25) % 1.0), saw((t + 0.5) % 1.0), square(t, 0.5), square(t, 0.12)];
            lerp(w[i], w[i + 1], f)
        }
        WtKind::Pulse => square(t, 0.5 - 0.47 * x),
        WtKind::Digital => {
            let steps = lerp(64.0, 6.0, x).round();
            let levels = lerp(32.0, 1.5, x * x);
            let tq = (t * steps).floor() / steps;
            ((TAU * tq).sin() * levels).round() / levels
        }
        WtKind::SyncSweep => {
            let r = 1.0 + 7.0 * x;
            // window hides the reset discontinuity a little, like an analog sync sweep
            saw((t * r) % 1.0) * (1.0 - t).powf(0.3)
        }
        WtKind::Fold => {
            let g = 1.0 + 9.0 * x;
            (std::f64::consts::FRAC_PI_2 * g * s).sin()
        }
        WtKind::Fm => {
            let idx = 8.0 * x * x;
            (TAU * t + idx * (TAU * 3.0 * t).sin()).sin()
        }
        WtKind::DriveSine => {
            let g = 1.0 + 30.0 * x * x;
            (g * s).tanh() / g.tanh()
        }
        WtKind::Hollow => {
            // two saws an octave and a fifth apart with PWM-like offset
            let o = 0.5 * x;
            0.5 * saw(t) + 0.35 * saw((2.0 * t + o) % 1.0) - 0.3 * saw((3.0 * t + 0.25 + o) % 1.0)
        }
        // additive kinds are built in the spectrum
        WtKind::Harmonics | WtKind::Vowel => 0.0,
    }
}

/// Spectral (harmonic amplitude, sine phase) for the additive kinds.
fn additive(kind: WtKind, x: f64, h: usize) -> f64 {
    let k = h as f64;
    match kind {
        WtKind::Harmonics => {
            let n = 1.0 + 63.0 * x * x;
            let fade = (n - k + 1.0).clamp(0.0, 1.0);
            fade / k.powf(0.6)
        }
        WtKind::Vowel => {
            // formants in harmonics of a ~110 Hz fundamental
            const V: [[f64; 3]; 5] = [[800.0, 1150.0, 2900.0], [400.0, 1600.0, 2700.0], [250.0, 1750.0, 3000.0], [450.0, 800.0, 2830.0], [325.0, 700.0, 2530.0]];
            let seg = x * 4.0;
            let i = (seg.floor() as usize).min(3);
            let f = seg - i as f64;
            let mut amp = 0.0;
            for j in 0..3 {
                let fc = lerp(V[i][j], V[i + 1][j], f) / 110.0;
                let bw = 0.6 + fc * 0.08;
                let g = [1.0, 0.6, 0.3][j];
                amp += g * (-((k - fc) / bw).powi(2)).exp();
            }
            (amp + 0.02) / k.sqrt()
        }
        _ => 0.0,
    }
}

fn build(kind: WtKind) -> Wavetable {
    let frames = kind.frames();
    let mut data = vec![0.0f32; frames * MIPS * TABLE_LEN];
    let half = TABLE_LEN / 2;
    let mut re = vec![0.0f64; GEN_LEN];
    let mut im = vec![0.0f64; GEN_LEN];
    let mut spec_re = vec![0.0f64; half];
    let mut spec_im = vec![0.0f64; half];
    let mut out_re = vec![0.0f64; TABLE_LEN];
    let mut out_im = vec![0.0f64; TABLE_LEN];
    for fi in 0..frames {
        let x = if frames > 1 { fi as f64 / (frames - 1) as f64 } else { 0.0 };
        spec_re.iter_mut().for_each(|v| *v = 0.0);
        spec_im.iter_mut().for_each(|v| *v = 0.0);
        if matches!(kind, WtKind::Harmonics | WtKind::Vowel) {
            // sine phase: x(t) = Σ a_k sin(2πkt)  ⇒  X[k] = -i·a_k·N/2
            for h in 1..half {
                spec_im[h] = -additive(kind, x, h) * TABLE_LEN as f64 / 2.0;
            }
        } else {
            for i in 0..GEN_LEN {
                re[i] = shape(kind, fi, x, i as f64 / GEN_LEN as f64);
                im[i] = 0.0;
            }
            fft(&mut re, &mut im, false);
            let scale = TABLE_LEN as f64 / GEN_LEN as f64;
            for h in 1..half {
                spec_re[h] = re[h] * scale;
                spec_im[h] = im[h] * scale;
            }
        }
        let mut norm = 1.0f64;
        for mip in 0..MIPS {
            let max_h = (half >> mip).max(2);
            out_re.iter_mut().for_each(|v| *v = 0.0);
            out_im.iter_mut().for_each(|v| *v = 0.0);
            for h in 1..max_h.min(half) {
                out_re[h] = spec_re[h];
                out_im[h] = spec_im[h];
                out_re[TABLE_LEN - h] = spec_re[h];
                out_im[TABLE_LEN - h] = -spec_im[h];
            }
            fft(&mut out_re, &mut out_im, true);
            if mip == 0 && kind != WtKind::Basic {
                let peak = out_re.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                norm = if peak > 1e-9 { 1.0 / peak } else { 1.0 };
            }
            let o = (fi * MIPS + mip) * TABLE_LEN;
            for i in 0..TABLE_LEN {
                data[o + i] = (out_re[i] * norm) as f32;
            }
        }
    }
    let _ = PI;
    Wavetable { frames, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_finite_and_normalised() {
        for k in WtKind::ALL {
            let t = table(k);
            for f in 0..t.frames {
                let fr = t.frame(f);
                assert!(fr.iter().all(|v| v.is_finite()));
                let peak = fr.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(peak > 0.3 && peak < 1.25, "{:?} frame {f}: peak {peak}", k);
            }
        }
    }

    #[test]
    fn high_mips_are_band_limited() {
        // top mip keeps only the fundamental: a saw becomes a sine
        let t = table(WtKind::Basic);
        let top: Vec<f32> = (0..TABLE_LEN).map(|i| t.sample_frame(1, i as f32 / TABLE_LEN as f32, MIPS - 1)).collect();
        let peak = top.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let rms = (top.iter().map(|v| v * v).sum::<f32>() / TABLE_LEN as f32).sqrt();
        assert!((peak / rms - 2f32.sqrt()).abs() < 0.02, "crest {}", peak / rms);
    }
}
