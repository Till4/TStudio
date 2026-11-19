use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    LowPass,
    HighPass,
    BandPass,
    Notch,
}

impl FilterMode {
    pub const ALL: [FilterMode; 4] = [FilterMode::LowPass, FilterMode::HighPass, FilterMode::BandPass, FilterMode::Notch];
    pub fn name(self) -> &'static str {
        match self {
            FilterMode::LowPass => "Low Pass",
            FilterMode::HighPass => "High Pass",
            FilterMode::BandPass => "Band Pass",
            FilterMode::Notch => "Notch",
        }
    }
}

/// Cytomic / Simper trapezoidal state variable filter.
#[derive(Clone, Copy, Debug)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Default for Svf {
    fn default() -> Self {
        let mut s = Self { ic1: 0.0, ic2: 0.0, a1: 0.0, a2: 0.0, a3: 0.0, k: 1.0 };
        s.set(1000.0, 0.0, 48000.0);
        s
    }
}

impl Svf {
    /// `resonance` 0..1
    pub fn set(&mut self, cutoff: f32, resonance: f32, sr: f32) {
        let fc = cutoff.clamp(16.0, sr * 0.49);
        let g = (PI * fc / sr).tan();
        let q = 0.5 + resonance.clamp(0.0, 1.0) * 19.5;
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    #[inline]
    pub fn process(&mut self, v0: f32, mode: FilterMode) -> f32 {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match mode {
            FilterMode::LowPass => v2,
            FilterMode::BandPass => v1,
            FilterMode::HighPass => v0 - self.k * v1 - v2,
            FilterMode::Notch => v0 - self.k * v1,
        }
    }
}

/// RBJ biquad (transposed direct form II).
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }
}

impl Biquad {
    fn set_coefs(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub fn peaking(&mut self, f: f32, q: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f.clamp(10.0, sr * 0.49) / sr;
        let alpha = w0.sin() / (2.0 * q.max(0.05));
        let c = w0.cos();
        self.set_coefs(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }

    pub fn low_shelf(&mut self, f: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f.clamp(10.0, sr * 0.49) / sr;
        let c = w0.cos();
        let alpha = w0.sin() / 2.0 * (2.0f32).sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        self.set_coefs(
            a * ((a + 1.0) - (a - 1.0) * c + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - sa),
            (a + 1.0) + (a - 1.0) * c + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - sa,
        );
    }

    pub fn high_shelf(&mut self, f: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f.clamp(10.0, sr * 0.49) / sr;
        let c = w0.cos();
        let alpha = w0.sin() / 2.0 * (2.0f32).sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        self.set_coefs(
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        );
    }

    /// Magnitude response in dB at `f` Hz.
    pub fn mag_db(&self, f: f32, sr: f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * f as f64 / sr as f64;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (b0, b1, b2, a1, a2) = (self.b0 as f64, self.b1 as f64, self.b2 as f64, self.a1 as f64, self.a2 as f64);
        let nr = b0 + b1 * c1 + b2 * c2;
        let ni = -(b1 * s1 + b2 * s2);
        let dr = 1.0 + a1 * c1 + a2 * c2;
        let di = -(a1 * s1 + a2 * s2);
        (10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di).max(1e-30)).log10()) as f32
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// One-pole lowpass, handy for smoothing and tone controls.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    pub z: f32,
    pub a: f32,
}

impl OnePole {
    pub fn set(&mut self, cutoff: f32, sr: f32) {
        self.a = (-2.0 * PI * cutoff.clamp(1.0, sr * 0.49) / sr).exp();
    }
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.z = x + (self.z - x) * self.a;
        self.z
    }
}

// ------------------------------------------------------------------ synth filter

/// Filter models of the TSynth filter slots.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
pub enum FilterType {
    /// 12 dB/oct state variable (LP/HP/BP/Notch) – the classic TSynth filter
    #[default]
    Svf12,
    /// 24 dB/oct (two cascaded SVFs)
    Svf24,
    /// 4-pole transistor-ladder low pass with saturating feedback (acid)
    Ladder,
    /// Feedback comb tuned to the cutoff (High Pass mode = negative polarity)
    Comb,
    /// Vowel formant bank; cutoff morphs A-E-I-O-U
    Formant,
}

impl FilterType {
    pub const ALL: [FilterType; 5] = [FilterType::Svf12, FilterType::Svf24, FilterType::Ladder, FilterType::Comb, FilterType::Formant];
    pub fn name(self) -> &'static str {
        match self {
            FilterType::Svf12 => "12 dB",
            FilterType::Svf24 => "24 dB",
            FilterType::Ladder => "Ladder",
            FilterType::Comb => "Comb",
            FilterType::Formant => "Formant",
        }
    }
    /// whether LP/HP/BP/Notch applies
    pub fn uses_mode(self) -> bool {
        matches!(self, FilterType::Svf12 | FilterType::Svf24 | FilterType::Comb)
    }
}

const FORMANTS: [[f32; 3]; 5] = [[800.0, 1150.0, 2900.0], [400.0, 1600.0, 2700.0], [250.0, 1750.0, 3000.0], [450.0, 800.0, 2830.0], [325.0, 700.0, 2530.0]];

/// One channel of a TSynth filter slot. `set` at control rate, `process` per sample.
#[derive(Clone, Debug)]
pub struct SynthFilter {
    a: Svf,
    b: Svf,
    form: [Svf; 3],
    form_g: [f32; 3],
    ladder: [f32; 4],
    lg: f32,
    lk: f32,
    comb: Vec<f32>,
    comb_idx: usize,
    comb_d: f32,
    comb_fb: f32,
    comb_lp: f32,
}

impl SynthFilter {
    /// `max_delay`: comb buffer length in samples (sr / lowest cutoff).
    pub fn new(max_delay: usize) -> Self {
        Self {
            a: Svf::default(),
            b: Svf::default(),
            form: [Svf::default(); 3],
            form_g: [1.0; 3],
            ladder: [0.0; 4],
            lg: 0.5,
            lk: 0.0,
            comb: vec![0.0; max_delay.max(4).next_power_of_two()],
            comb_idx: 0,
            comb_d: 100.0,
            comb_fb: 0.0,
            comb_lp: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.a.reset();
        self.b.reset();
        for f in self.form.iter_mut() {
            f.reset();
        }
        self.ladder = [0.0; 4];
        self.comb.iter_mut().for_each(|v| *v = 0.0);
        self.comb_lp = 0.0;
    }

    pub fn set(&mut self, kind: FilterType, cutoff: f32, res: f32, sr: f32) {
        let res = res.clamp(0.0, 1.0);
        match kind {
            FilterType::Svf12 => self.a.set(cutoff, res, sr),
            FilterType::Svf24 => {
                // Butterworth stage Qs (0.54 / 1.31) at zero resonance; resonance raises the second stage only
                self.a.set(cutoff, 0.002, sr);
                self.b.set(cutoff, 0.04 + res * 0.96, sr);
            }
            FilterType::Ladder => {
                let fc = cutoff.clamp(16.0, sr * 0.45);
                let g = (PI * fc / sr).tan();
                self.lg = g / (1.0 + g);
                self.lk = res * 3.95;
            }
            FilterType::Comb => {
                let fc = cutoff.clamp(20.0, sr * 0.45);
                self.comb_d = (sr / fc).clamp(2.0, (self.comb.len() - 2) as f32);
                self.comb_fb = res * 0.97;
            }
            FilterType::Formant => {
                // cutoff 100 Hz .. 10 kHz -> vowel position 0..1
                let pos = ((cutoff.max(1.0) / 100.0).log2() / 100f32.log2()).clamp(0.0, 1.0) * 4.0;
                let i = (pos as usize).min(3);
                let f = pos - i as f32;
                for j in 0..3 {
                    let fc = FORMANTS[i][j] + (FORMANTS[i + 1][j] - FORMANTS[i][j]) * f;
                    let r = 0.45 + res * 0.5;
                    self.form[j].set(fc, r, sr);
                    // band pass peak gain equals Q: normalise to unity, keep the vowel balance
                    let q = 0.5 + r * 19.5;
                    self.form_g[j] = [1.0, 0.7, 0.45][j] * 2.0 / q;
                }
            }
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32, kind: FilterType, mode: FilterMode) -> f32 {
        match kind {
            FilterType::Svf12 => self.a.process(x, mode),
            FilterType::Svf24 => {
                let y = self.a.process(x, mode);
                self.b.process(y, mode)
            }
            FilterType::Ladder => {
                let g = self.lg;
                let b = 1.0 - g;
                let s = &mut self.ladder;
                let sum = g * g * g * b * s[0] + g * g * b * s[1] + g * b * s[2] + b * s[3];
                let g4 = g * g * g * g;
                let y4 = (g4 * x + sum) / (1.0 + self.lk * g4);
                // saturating feedback + bass compensation
                let mut u = (x * (1.0 + 0.5 * self.lk) - self.lk * y4).tanh();
                for st in s.iter_mut() {
                    let v = (u - *st) * g;
                    let y = v + *st;
                    *st = y + v;
                    u = y;
                }
                u
            }
            FilterType::Comb => {
                let n = self.comb.len();
                let pos = self.comb_idx as f32 + n as f32 - self.comb_d;
                let i0 = pos as usize;
                let t = pos - i0 as f32;
                let a = self.comb[i0 & (n - 1)];
                let bb = self.comb[(i0 + 1) & (n - 1)];
                let d = a + (bb - a) * t;
                // gentle damping in the loop keeps high resonance musical
                self.comb_lp += (d - self.comb_lp) * 0.7;
                let fb = if mode == FilterMode::HighPass { -self.comb_fb } else { self.comb_fb };
                let w = (x + self.comb_lp * fb).clamp(-4.0, 4.0);
                self.comb[self.comb_idx] = w;
                self.comb_idx = (self.comb_idx + 1) & (n - 1);
                0.5 * (x + d)
            }
            FilterType::Formant => {
                let mut y = 0.0;
                for j in 0..3 {
                    y += self.form[j].process(x, FilterMode::BandPass) * self.form_g[j];
                }
                y
            }
        }
    }
}

#[cfg(test)]
mod synth_filter_tests {
    use super::*;

    /// steady-state gain (dB) of a sine through the filter
    fn gain_db(kind: FilterType, mode: FilterMode, cutoff: f32, res: f32, f: f32) -> f32 {
        let sr = 48000.0;
        let mut flt = SynthFilter::new(4096);
        flt.set(kind, cutoff, res, sr);
        let n = 48000;
        let mut acc_in = 0.0f64;
        let mut acc_out = 0.0f64;
        for i in 0..n {
            let x = 0.1 * (std::f32::consts::TAU * f * i as f32 / sr).sin();
            let y = flt.process(x, kind, mode);
            assert!(y.is_finite());
            if i > n / 2 {
                acc_in += (x * x) as f64;
                acc_out += (y * y) as f64;
            }
        }
        10.0 * (acc_out / acc_in).max(1e-20).log10() as f32
    }

    #[test]
    fn frequency_responses() {
        use FilterMode::*;
        // 12 dB LP: pass band flat, about -24 dB two octaves above
        assert!(gain_db(FilterType::Svf12, LowPass, 1000.0, 0.0, 100.0).abs() < 1.0);
        let lp12 = gain_db(FilterType::Svf12, LowPass, 1000.0, 0.0, 4000.0);
        let lp24 = gain_db(FilterType::Svf24, LowPass, 1000.0, 0.0, 4000.0);
        assert!(lp12 < -18.0 && lp12 > -30.0, "lp12 {lp12}");
        assert!(lp24 < -40.0, "lp24 {lp24}");
        assert!(gain_db(FilterType::Svf24, LowPass, 1000.0, 0.0, 100.0).abs() < 1.5);
        // HP mirrors it
        assert!(gain_db(FilterType::Svf24, HighPass, 1000.0, 0.0, 250.0) < -40.0);
        assert!(gain_db(FilterType::Svf24, HighPass, 1000.0, 0.0, 8000.0).abs() < 1.5);
        // BP peaks at the centre
        let bp_c = gain_db(FilterType::Svf12, BandPass, 1000.0, 0.5, 1000.0);
        let bp_o = gain_db(FilterType::Svf12, BandPass, 1000.0, 0.5, 8000.0);
        assert!(bp_c > bp_o + 15.0, "bp {bp_c} {bp_o}");
        // ladder: 24 dB slope, resonance boosts the cutoff region
        assert!(gain_db(FilterType::Ladder, LowPass, 1000.0, 0.0, 100.0).abs() < 2.0);
        assert!(gain_db(FilterType::Ladder, LowPass, 1000.0, 0.0, 8000.0) < -40.0);
        assert!(gain_db(FilterType::Ladder, LowPass, 1000.0, 0.8, 1000.0) > gain_db(FilterType::Ladder, LowPass, 1000.0, 0.0, 1000.0) + 6.0);
        // comb: peak at the tuned frequency, notch half way between
        let peak = gain_db(FilterType::Comb, LowPass, 500.0, 0.8, 500.0);
        let notch = gain_db(FilterType::Comb, LowPass, 500.0, 0.8, 750.0);
        assert!(peak > notch + 10.0, "comb {peak} {notch}");
        // formant vowel A passes ~800 Hz much more than 5 kHz
        assert!(gain_db(FilterType::Formant, LowPass, 100.0, 0.5, 800.0) > gain_db(FilterType::Formant, LowPass, 100.0, 0.5, 5000.0) + 12.0);
    }
}
