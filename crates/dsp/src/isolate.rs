//! Offline "isolation" of parts of a mix for sample cutting: steep zero-phase high/low cut,
//! a peaking band and mid/side extraction (center = vocals/kick/bass, side = vocal remover).
//! Zero-phase (forward + backward) filtering keeps transients in place, which matters for drum hits.

use crate::SampleData;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StereoMode {
    #[default]
    Stereo,
    /// (L+R)/2 – what sits in the center (vocals, kick, bass)
    Mid,
    /// (L−R)/2 on both channels – everything except the center (removes centered vocals), mono
    Side,
    Left,
    Right,
}

impl StereoMode {
    pub const ALL: [StereoMode; 5] = [StereoMode::Stereo, StereoMode::Mid, StereoMode::Side, StereoMode::Left, StereoMode::Right];

    pub fn label(self) -> &'static str {
        match self {
            StereoMode::Stereo => "Stereo",
            StereoMode::Mid => "Mitte",
            StereoMode::Side => "Seiten",
            StereoMode::Left => "Links",
            StereoMode::Right => "Rechts",
        }
    }

    pub fn parse(s: &str) -> Option<StereoMode> {
        match s.to_ascii_lowercase().as_str() {
            "stereo" | "lr" => Some(StereoMode::Stereo),
            "mid" | "mitte" | "center" | "mono" => Some(StereoMode::Mid),
            "side" | "seiten" | "sides" => Some(StereoMode::Side),
            "left" | "links" | "l" => Some(StereoMode::Left),
            "right" | "rechts" | "r" => Some(StereoMode::Right),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IsolateParams {
    /// high-pass (low cut) in Hz, 0 = off
    pub hp_hz: f32,
    /// low-pass (high cut) in Hz, 0 = off
    pub lp_hz: f32,
    /// steepness: Butterworth sections per filter (1 = 24, 2 = 48, 4 = 96 dB/oct, zero-phase)
    pub slope: u8,
    pub peak_hz: f32,
    /// total boost/cut of the peaking band in dB (0 = off)
    pub peak_db: f32,
    pub peak_q: f32,
    pub stereo: StereoMode,
    /// output gain in dB
    pub gain_db: f32,
}

impl Default for IsolateParams {
    fn default() -> Self {
        Self { hp_hz: 0.0, lp_hz: 0.0, slope: 2, peak_hz: 1000.0, peak_db: 0.0, peak_q: 1.0, stereo: StereoMode::Stereo, gain_db: 0.0 }
    }
}

/// Steepness choices shown in the UI: (sections, label).
pub const SLOPES: [(u8, &str); 3] = [(1, "24 dB/Okt"), (2, "48 dB/Okt"), (4, "96 dB/Okt")];

impl IsolateParams {
    pub fn is_neutral(&self) -> bool {
        self.hp_hz <= 0.0 && self.lp_hz <= 0.0 && self.peak_db.abs() < 0.01 && self.stereo == StereoMode::Stereo && self.gain_db.abs() < 0.01
    }

    /// Ready-made settings for typical parts of a track.
    pub fn presets() -> Vec<(&'static str, IsolateParams)> {
        let d = IsolateParams::default();
        vec![
            ("Aus", d),
            ("Kick / Sub", IsolateParams { hp_hz: 25.0, lp_hz: 150.0, stereo: StereoMode::Mid, ..d }),
            ("Bass", IsolateParams { hp_hz: 40.0, lp_hz: 400.0, stereo: StereoMode::Mid, ..d }),
            ("Snare / Clap", IsolateParams { hp_hz: 160.0, lp_hz: 9000.0, peak_hz: 220.0, peak_db: 3.0, peak_q: 1.2, ..d }),
            ("Hi-Hats / Becken", IsolateParams { hp_hz: 6000.0, ..d }),
            ("Vocals (Mitte)", IsolateParams { hp_hz: 180.0, lp_hz: 9000.0, peak_hz: 2500.0, peak_db: 2.0, stereo: StereoMode::Mid, ..d }),
            ("Synths / Mitten", IsolateParams { hp_hz: 250.0, lp_hz: 6000.0, ..d }),
            ("Vocal entfernen", IsolateParams { stereo: StereoMode::Side, ..d }),
            ("Telefon", IsolateParams { hp_hz: 400.0, lp_hz: 3200.0, slope: 4, stereo: StereoMode::Mid, ..d }),
        ]
    }

    /// Short description, e.g. "HP 150 Hz · LP 9.0 kHz · Mitte".
    pub fn describe(&self) -> String {
        let hz = |f: f32| if f >= 1000.0 { format!("{:.1} kHz", f / 1000.0) } else { format!("{f:.0} Hz") };
        let mut v = Vec::new();
        if self.hp_hz > 0.0 {
            v.push(format!("HP {}", hz(self.hp_hz)));
        }
        if self.lp_hz > 0.0 {
            v.push(format!("LP {}", hz(self.lp_hz)));
        }
        if self.peak_db.abs() >= 0.01 {
            v.push(format!("{:+.1} dB @ {}", self.peak_db, hz(self.peak_hz)));
        }
        if self.stereo != StereoMode::Stereo {
            v.push(self.stereo.label().to_string());
        }
        if self.gain_db.abs() >= 0.01 {
            v.push(format!("{:+.1} dB", self.gain_db));
        }
        if v.is_empty() {
            "neutral".into()
        } else {
            v.join(" · ")
        }
    }

    fn sections(&self, sr: f64) -> Vec<Biquad> {
        let nyq = sr * 0.5;
        let k = self.slope.clamp(1, 8) as usize;
        let mut v = Vec::new();
        // Butterworth order 2k as k biquads with Q_i = 1 / (2 cos θ_i)
        let qs: Vec<f64> = (0..k).map(|i| 1.0 / (2.0 * (PI * (2 * i + 1) as f64 / (4 * k) as f64).cos())).collect();
        if self.hp_hz > 0.0 && (self.hp_hz as f64) < nyq * 0.95 {
            for q in &qs {
                v.push(Biquad::highpass(self.hp_hz as f64, *q, sr));
            }
        }
        if self.lp_hz > 0.0 && (self.lp_hz as f64) < nyq * 0.95 {
            for q in &qs {
                v.push(Biquad::lowpass(self.lp_hz as f64, *q, sr));
            }
        }
        if self.peak_db.abs() >= 0.01 && (self.peak_hz as f64) < nyq * 0.95 {
            // applied twice (forward + backward) → half the gain per pass
            v.push(Biquad::peaking(self.peak_hz.max(10.0) as f64, self.peak_q.clamp(0.1, 20.0) as f64, self.peak_db as f64 * 0.5, sr));
        }
        v
    }

    /// Magnitude response of the whole chain in dB at `freq` (stereo mode not included).
    pub fn response_db(&self, freq: f32, sr: u32) -> f32 {
        let sr = sr.max(1) as f64;
        let mut db = self.gain_db as f64;
        for s in self.sections(sr) {
            db += 2.0 * s.mag_db(freq as f64, sr);
        }
        db as f32
    }

    /// Processes the whole buffer.
    pub fn process(&self, d: &SampleData) -> SampleData {
        let (mut l, mut r) = self.stereo_split(&d.left, &d.right);
        let secs = self.sections(d.sample_rate as f64);
        let g = crate::db_to_gain(self.gain_db);
        for ch in [&mut l, &mut r] {
            for s in &secs {
                s.run_forward(ch);
                s.run_backward(ch);
            }
            if (g - 1.0).abs() > 1e-6 {
                for x in ch.iter_mut() {
                    *x *= g;
                }
            }
        }
        SampleData::new(l, r, d.sample_rate)
    }

    /// Processes `d[a..b]`, with some audio before and after so the zero-phase filters settle
    /// exactly like in the full-length result.
    pub fn process_range(&self, d: &SampleData, a: usize, b: usize) -> SampleData {
        let b = b.min(d.len());
        let a = a.min(b);
        let roll = (d.sample_rate as usize / 4).max(1);
        let (ra, rb) = (a.saturating_sub(roll), (b + roll).min(d.len()));
        let part = self.process(&d.slice(ra, rb));
        part.slice(a - ra, b - ra)
    }

    fn stereo_split(&self, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        match self.stereo {
            StereoMode::Stereo => (l.to_vec(), r.to_vec()),
            StereoMode::Mid => {
                let m: Vec<f32> = l.iter().zip(r).map(|(a, b)| (a + b) * 0.5).collect();
                (m.clone(), m)
            }
            StereoMode::Side => {
                // (L−R)/2 on both channels: mono-safe (L / −R would cancel when summed)
                let s: Vec<f32> = l.iter().zip(r).map(|(a, b)| (a - b) * 0.5).collect();
                (s.clone(), s)
            }
            StereoMode::Left => (l.to_vec(), l.to_vec()),
            StereoMode::Right => (r.to_vec(), r.to_vec()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    fn norm(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    fn lowpass(f: f64, q: f64, sr: f64) -> Self {
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        Self::norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }

    fn highpass(f: f64, q: f64, sr: f64) -> Self {
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        Self::norm((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }

    fn peaking(f: f64, q: f64, db: f64, sr: f64) -> Self {
        let a = 10f64.powf(db / 40.0);
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        Self::norm(1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a)
    }

    fn mag_db(&self, f: f64, sr: f64) -> f64 {
        let w = 2.0 * PI * f / sr;
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = self.b1 * s1 + self.b2 * s2;
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = self.a1 * s1 + self.a2 * s2;
        let m = ((nr * nr + ni * ni) / (dr * dr + di * di).max(1e-30)).sqrt();
        20.0 * m.max(1e-12).log10()
    }

    #[inline]
    fn step(&self, x: f64, z1: &mut f64, z2: &mut f64) -> f64 {
        let y = self.b0 * x + *z1;
        *z1 = self.b1 * x - self.a1 * y + *z2;
        *z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn run_forward(&self, buf: &mut [f32]) {
        let (mut z1, mut z2) = (0.0, 0.0);
        for x in buf.iter_mut() {
            *x = self.step(*x as f64, &mut z1, &mut z2) as f32;
        }
    }

    fn run_backward(&self, buf: &mut [f32]) {
        let (mut z1, mut z2) = (0.0, 0.0);
        for x in buf.iter_mut().rev() {
            *x = self.step(*x as f64, &mut z1, &mut z2) as f32;
        }
    }
}

/// Log-spaced frequencies from `lo` to `hi`.
pub fn log_freqs(n: usize, lo: f32, hi: f32) -> Vec<f32> {
    let n = n.max(2);
    (0..n).map(|i| lo * (hi / lo).powf(i as f32 / (n - 1) as f32)).collect()
}

/// Average magnitude spectrum (dBFS) of `mono` at the given frequencies: up to 12 Hann windows
/// of 4096 samples spread over the signal, Goertzel per frequency.
pub fn spectrum_db(mono: &[f32], sr: u32, freqs: &[f32]) -> Vec<f32> {
    const N: usize = 4096;
    let mut power = vec![0.0f64; freqs.len()];
    if mono.is_empty() {
        return vec![-120.0; freqs.len()];
    }
    let windows = if mono.len() <= N { 1 } else { ((mono.len() - N) / (N / 2) + 1).min(12) };
    let hop = if windows > 1 { (mono.len() - N) / (windows - 1) } else { 0 };
    let hann: Vec<f64> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / (N - 1) as f64).cos()).collect();
    let mut frame = vec![0.0f64; N];
    for w in 0..windows {
        let start = w * hop;
        for i in 0..N {
            frame[i] = mono.get(start + i).copied().unwrap_or(0.0) as f64 * hann[i];
        }
        for (k, f) in freqs.iter().enumerate() {
            let coeff = 2.0 * (2.0 * PI * *f as f64 / sr as f64).cos();
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for x in &frame {
                let s = x + coeff * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            power[k] += s1 * s1 + s2 * s2 - coeff * s1 * s2;
        }
    }
    // Hann coherent gain 0.5 → full-scale sine ≈ 0 dB
    let norm = (N as f64 * 0.5 / 2.0).powi(2) * windows as f64;
    power.iter().map(|p| (10.0 * (p / norm).max(1e-12).log10()) as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, sr: u32, secs: f32) -> SampleData {
        let n = (sr as f32 * secs) as usize;
        let v: Vec<f32> = (0..n).map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32).sin() * 0.5).collect();
        SampleData::new(v.clone(), v, sr)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn lowpass_keeps_bass_kills_highs() {
        let p = IsolateParams { lp_hz: 200.0, ..Default::default() };
        let lo = p.process(&sine(60.0, 44100, 1.0));
        let hi = p.process(&sine(3000.0, 44100, 1.0));
        assert!((rms(&lo.left) / 0.3535 - 1.0).abs() < 0.05, "bass passes");
        assert!(rms(&hi.left) < 0.001, "highs removed: {}", rms(&hi.left));
        assert!(p.response_db(60.0, 44100).abs() < 0.5);
        assert!(p.response_db(3000.0, 44100) < -80.0);
    }

    #[test]
    fn highpass_and_zero_phase_alignment() {
        let p = IsolateParams { hp_hz: 5000.0, slope: 4, ..Default::default() };
        assert!(rms(&p.process(&sine(500.0, 48000, 0.5)).left) < 0.001);
        // an impulse stays where it is (zero phase)
        let mut l = vec![0.0f32; 4800];
        l[2400] = 1.0;
        let d = SampleData::new(l.clone(), l, 48000);
        let o = IsolateParams { hp_hz: 3000.0, ..Default::default() }.process(&d);
        let peak = o.left.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(peak, 2400);
    }

    #[test]
    fn mid_side() {
        let n = 1000;
        let l: Vec<f32> = vec![0.5; n];
        let r: Vec<f32> = vec![0.1; n];
        let d = SampleData::new(l, r, 44100);
        let m = IsolateParams { stereo: StereoMode::Mid, ..Default::default() }.process(&d);
        assert!((m.left[10] - 0.3).abs() < 1e-6 && (m.right[10] - 0.3).abs() < 1e-6);
        let s = IsolateParams { stereo: StereoMode::Side, ..Default::default() }.process(&d);
        assert!((s.left[10] - 0.2).abs() < 1e-6 && (s.right[10] - 0.2).abs() < 1e-6);
        // pure center content disappears with Side
        let c = SampleData::new(vec![0.4; n], vec![0.4; n], 44100);
        assert!(rms(&IsolateParams { stereo: StereoMode::Side, ..Default::default() }.process(&c).left) < 1e-6);
    }

    #[test]
    fn peak_gain_and_range_matches_full() {
        let p = IsolateParams { peak_hz: 1000.0, peak_db: 6.0, ..Default::default() };
        assert!((p.response_db(1000.0, 44100) - 6.0).abs() < 0.05);
        let d = sine(1000.0, 44100, 2.0);
        let full = IsolateParams { hp_hz: 300.0, ..p }.process(&d);
        let part = IsolateParams { hp_hz: 300.0, ..p }.process_range(&d, 30000, 40000);
        for i in 0..10000 {
            assert!((full.left[30000 + i] - part.left[i]).abs() < 1e-3);
        }
    }

    #[test]
    fn spectrum_finds_sine() {
        let d = sine(440.0, 44100, 1.0);
        let fs = [110.0, 440.0, 1760.0];
        let s = spectrum_db(&d.left, 44100, &fs);
        assert!(s[1] > -10.0 && s[1] < 0.0, "{s:?}");
        assert!(s[0] < s[1] - 40.0 && s[2] < s[1] - 40.0, "{s:?}");
    }
}
