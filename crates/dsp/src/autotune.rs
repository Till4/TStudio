//! Autotune: real-time pitch correction to a key/scale.
//!
//! Pitch detection: YIN on a decimated (~16 kHz) mono signal every ~8 ms.
//! Pitch shifting: streaming TD-PSOLA – grains of two periods (Hann) are taken one *analysis*
//! period apart and laid down one *synthesis* period apart (= period / ratio), so the formants
//! stay put and the voice does not get the "chipmunk" sound. Unvoiced parts (s, t, breaths) pass
//! through unchanged. Fixed latency of [`latency_frames`] samples; the engine starts audio
//! clips on such inserts that much earlier, so recordings stay in time.
//!
//! Everything is allocated in `new`; `process` never allocates.

use crate::NOTE_NAMES;
use serde::{Deserialize, Serialize};

/// Lowest / highest detected voice pitch.
pub const MIN_HZ: f32 = 75.0;
pub const MAX_HZ: f32 = 1000.0;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Scale {
    Chromatic,
    Major,
    #[default]
    Minor,
    HarmonicMinor,
    Phrygian,
    MinorPentatonic,
    MajorPentatonic,
}

impl Scale {
    pub const ALL: [Scale; 7] = [Scale::Chromatic, Scale::Major, Scale::Minor, Scale::HarmonicMinor, Scale::Phrygian, Scale::MinorPentatonic, Scale::MajorPentatonic];

    pub fn name(self) -> &'static str {
        match self {
            Scale::Chromatic => "Chromatisch",
            Scale::Major => "Dur",
            Scale::Minor => "Moll",
            Scale::HarmonicMinor => "Harm. Moll",
            Scale::Phrygian => "Phrygisch",
            Scale::MinorPentatonic => "Moll-Pentatonik",
            Scale::MajorPentatonic => "Dur-Pentatonik",
        }
    }

    /// Semitones above the root that belong to the scale.
    pub fn steps(self) -> &'static [i32] {
        match self {
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
        }
    }

    pub fn contains(self, root: u8, key: i32) -> bool {
        self.steps().contains(&(key - root as i32).rem_euclid(12))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AutotuneParams {
    /// root note 0 = C … 11 = B
    pub key: u8,
    pub scale: Scale,
    /// how fast a note is pulled to the target: 0 ms = hard/robotic, 50–150 ms = natural
    pub retune_ms: f32,
    /// 0..1 how much of the correction is applied
    pub amount: f32,
    /// extra transpose in semitones (e.g. -12 / +12 for doubles)
    pub transpose: f32,
    /// dry/wet
    pub mix: f32,
}

impl Default for AutotuneParams {
    fn default() -> Self {
        Self { key: 9, scale: Scale::Minor, retune_ms: 20.0, amount: 1.0, transpose: 0.0, mix: 1.0 }
    }
}

crate::fx::numeric_fields!(AutotuneParams { retune_ms, amount, transpose, mix });

impl AutotuneParams {
    pub fn key_name(&self) -> &'static str {
        NOTE_NAMES[(self.key % 12) as usize]
    }

    /// Nearest scale note (MIDI number) to a detected pitch (fractional MIDI).
    pub fn target_note(&self, midi: f32) -> i32 {
        let base = midi.round() as i32;
        let mut best = base;
        let mut best_d = f32::MAX;
        for k in base - 6..=base + 6 {
            if self.scale.contains(self.key % 12, k) {
                let d = (k as f32 - midi).abs();
                if d < best_d {
                    best_d = d;
                    best = k;
                }
            }
        }
        best
    }
}

/// Longest period in samples.
fn max_period(sr: f32) -> usize {
    (sr / MIN_HZ).ceil() as usize
}

/// Delay the effect adds, in samples.
pub fn latency_frames(sr: f32) -> usize {
    2 * max_period(sr) + 2
}

const YIN_WINDOW: usize = 512;
const YIN_HOP: usize = 128;
const YIN_THRESHOLD: f32 = 0.15;

pub struct AutotuneFx {
    sr: f32,
    mask: usize,
    /// input history: mono (analysis + grains), dry left/right
    mono: Vec<f32>,
    dry_l: Vec<f32>,
    dry_r: Vec<f32>,
    /// overlap-add accumulators (output time)
    acc: Vec<f32>,
    wsum: Vec<f32>,
    n: i64,
    latency: i64,
    /// next synthesis grain centre / last analysis mark (input time)
    syn: f64,
    ana: f64,
    // pitch detection
    dec: usize,
    dec_acc: f32,
    dec_cnt: usize,
    dbuf: Vec<f32>,
    dpos: usize,
    dfill: usize,
    since_detect: usize,
    scratch: Vec<f32>,
    diff: Vec<f32>,
    /// current analysis period (samples) – 0 when unvoiced
    period: f32,
    /// smoothed correction in semitones
    shift: f32,
    /// detected pitch (Hz) for display, 0 = none
    pub detected_hz: f32,
}

impl AutotuneFx {
    pub fn new(sr: f32) -> Self {
        let tmax = max_period(sr);
        let latency = latency_frames(sr);
        let size = (latency + 2 * tmax + 64).next_power_of_two();
        let dec = ((sr / 16000.0).round() as usize).max(1);
        let dsr = sr / dec as f32;
        let tau_max = (dsr / MIN_HZ).ceil() as usize + 2;
        let dlen = (YIN_WINDOW + tau_max).next_power_of_two();
        Self {
            sr,
            mask: size - 1,
            mono: vec![0.0; size],
            dry_l: vec![0.0; size],
            dry_r: vec![0.0; size],
            acc: vec![0.0; size],
            wsum: vec![0.0; size],
            n: 0,
            latency: latency as i64,
            syn: 0.0,
            ana: 0.0,
            dec,
            dec_acc: 0.0,
            dec_cnt: 0,
            dbuf: vec![0.0; dlen],
            dpos: 0,
            dfill: 0,
            since_detect: 0,
            scratch: vec![0.0; YIN_WINDOW + tau_max],
            diff: vec![0.0; tau_max + 1],
            period: 0.0,
            shift: 0.0,
            detected_hz: 0.0,
        }
    }

    pub fn latency(&self) -> usize {
        self.latency as usize
    }

    /// YIN on the decimated buffer → period in full-rate samples (None = unvoiced).
    fn detect(&mut self) -> Option<f32> {
        let dsr = self.sr / self.dec as f32;
        let tau_min = ((dsr / MAX_HZ).floor() as usize).max(2);
        let tau_max = self.diff.len() - 1;
        let len = self.scratch.len();
        if self.dfill < len {
            return None;
        }
        let dmask = self.dbuf.len() - 1;
        let start = self.dpos.wrapping_sub(len) & dmask;
        let mut energy = 0.0;
        for i in 0..len {
            let v = self.dbuf[(start + i) & dmask];
            self.scratch[i] = v;
            energy += v * v;
        }
        if energy / (len as f32) < 1e-6 {
            return None; // below about -60 dBFS
        }
        let x = &self.scratch;
        let w = YIN_WINDOW;
        self.diff[0] = 1.0;
        let mut running = 0.0;
        for tau in 1..=tau_max {
            let mut d = 0.0;
            for j in 0..w {
                let e = x[j] - x[j + tau];
                d += e * e;
            }
            running += d;
            self.diff[tau] = if running > 0.0 { d * tau as f32 / running } else { 1.0 };
        }
        let mut tau = tau_min;
        let mut found = None;
        while tau < tau_max {
            if self.diff[tau] < YIN_THRESHOLD {
                while tau + 1 < tau_max && self.diff[tau + 1] < self.diff[tau] {
                    tau += 1;
                }
                found = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = found?;
        // parabolic interpolation
        let (a, b, c) = (self.diff[tau - 1], self.diff[tau], self.diff[(tau + 1).min(tau_max)]);
        let den = a - 2.0 * b + c;
        let frac = if den.abs() > 1e-9 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
        Some((tau as f32 + frac) * self.dec as f32)
    }

    fn update_pitch(&mut self, p: &AutotuneParams) {
        let hop_s = (YIN_HOP * self.dec) as f32 / self.sr;
        let target = match self.detect() {
            Some(period) => {
                self.period = period;
                let hz = self.sr / period;
                self.detected_hz = hz;
                let midi = 69.0 + 12.0 * (hz / 440.0).log2();
                let note = p.target_note(midi);
                (note as f32 - midi) * p.amount.clamp(0.0, 1.0) + p.transpose
            }
            None => {
                self.period = 0.0;
                self.detected_hz = 0.0;
                p.transpose
            }
        };
        let alpha = if p.retune_ms <= 0.5 { 1.0 } else { 1.0 - (-hop_s / (p.retune_ms * 0.001)).exp() };
        self.shift += (target - self.shift) * alpha;
    }

    fn emit_grains(&mut self) {
        let unvoiced_period = (self.sr * 0.005).max(16.0);
        let tmax = max_period(self.sr) as f32;
        loop {
            let (period, ratio) = if self.period > 0.0 {
                (self.period.min(tmax), 2f32.powf(self.shift / 12.0).clamp(0.6, 1.7))
            } else {
                (unvoiced_period, 1.0)
            };
            let half = period.round().max(2.0) as i64;
            let c = self.syn.round() as i64;
            if c - half > self.n - self.latency + 1 {
                break;
            }
            // analysis mark: last mark ≤ c, marks one period apart
            if (self.syn - self.ana) > 4.0 * tmax as f64 || self.ana > self.syn {
                self.ana = self.syn;
            }
            while self.ana + period as f64 <= self.syn {
                self.ana += period as f64;
            }
            let a = self.ana.round() as i64;
            let inv = std::f32::consts::PI / half as f32;
            for i in -half..=half {
                let w = 0.5 + 0.5 * (i as f32 * inv).cos();
                let src = self.mono[((a + i) as usize) & self.mask];
                let dst = ((c + i) as usize) & self.mask;
                self.acc[dst] += src * w;
                self.wsum[dst] += w;
            }
            self.syn += (period / ratio) as f64;
        }
    }

    pub fn process(&mut self, p: &AutotuneParams, l: &mut [f32], r: &mut [f32]) {
        let mix = p.mix.clamp(0.0, 1.0);
        for i in 0..l.len().min(r.len()) {
            let idx = (self.n as usize) & self.mask;
            let m = 0.5 * (l[i] + r[i]);
            self.mono[idx] = m;
            self.dry_l[idx] = l[i];
            self.dry_r[idx] = r[i];
            // decimate for detection
            self.dec_acc += m;
            self.dec_cnt += 1;
            if self.dec_cnt == self.dec {
                let dmask = self.dbuf.len() - 1;
                self.dbuf[self.dpos & dmask] = self.dec_acc / self.dec as f32;
                self.dpos = self.dpos.wrapping_add(1);
                self.dfill = (self.dfill + 1).min(self.dbuf.len());
                self.dec_acc = 0.0;
                self.dec_cnt = 0;
                self.since_detect += 1;
                if self.since_detect >= YIN_HOP {
                    self.since_detect = 0;
                    self.update_pitch(p);
                }
            }
            self.emit_grains();
            // read output delayed by the latency
            let out_t = self.n - self.latency;
            let wet = if out_t >= 0 {
                let o = (out_t as usize) & self.mask;
                let ws = self.wsum[o];
                let y = if ws > 1e-3 { self.acc[o] / ws.max(0.35) } else { 0.0 };
                self.acc[o] = 0.0;
                self.wsum[o] = 0.0;
                y
            } else {
                0.0
            };
            let (dl, dr) = if out_t >= 0 {
                let o = (out_t as usize) & self.mask;
                (self.dry_l[o], self.dry_r[o])
            } else {
                (0.0, 0.0)
            };
            l[i] = dl + (wet - dl) * mix;
            r[i] = dr + (wet - dr) * mix;
            self.n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, sr: f32, n: usize) -> Vec<f32> {
        // slightly rich tone (voice-like): fundamental + 2 harmonics
        (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                let w = 2.0 * std::f32::consts::PI * hz * t;
                0.5 * w.sin() + 0.2 * (2.0 * w).sin() + 0.1 * (3.0 * w).sin()
            })
            .collect()
    }

    /// Fundamental by zero-crossing count over the tail of the buffer.
    fn freq_of(x: &[f32], sr: f32) -> f32 {
        // use YIN of the effect itself on the output
        let mut fx = AutotuneFx::new(sr);
        let p = AutotuneParams { scale: Scale::Chromatic, amount: 0.0, mix: 0.0, ..Default::default() };
        let mut l = x.to_vec();
        let mut r = x.to_vec();
        fx.process(&p, &mut l, &mut r);
        fx.detected_hz
    }

    #[test]
    fn scale_snapping() {
        let p = AutotuneParams { key: 9, scale: Scale::Minor, ..Default::default() }; // A minor
        assert_eq!(p.target_note(60.3), 60); // C
        assert_eq!(p.target_note(61.4), 62); // C# (sharp) → D, C# is not in A minor
        assert_eq!(p.target_note(60.8), 60); // flat C# → C
        assert_eq!(p.target_note(65.8), 65); // F# (flat) → F
        let c = AutotuneParams { scale: Scale::Chromatic, ..Default::default() };
        assert_eq!(c.target_note(61.4), 61);
    }

    #[test]
    fn detects_and_corrects_pitch() {
        let sr = 48000.0;
        // 450 Hz is between A4 (440) and A#4 (466); in A minor it must go to 440 Hz
        let input = sine(450.0, sr, 48000);
        assert!((freq_of(&input, sr) - 450.0).abs() < 4.0, "detector sees {}", freq_of(&input, sr));
        let mut fx = AutotuneFx::new(sr);
        let p = AutotuneParams { key: 9, scale: Scale::Minor, retune_ms: 0.0, amount: 1.0, ..Default::default() };
        let mut l = input.clone();
        let mut r = input.clone();
        fx.process(&p, &mut l, &mut r);
        assert!(l.iter().all(|x| x.is_finite()));
        let out_hz = freq_of(&l[24000..], sr);
        assert!((out_hz - 440.0).abs() < 4.0, "corrected to {out_hz} Hz");
        // level is kept
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let ratio = rms(&l[24000..]) / rms(&input[24000..]);
        assert!(ratio > 0.7 && ratio < 1.3, "level ratio {ratio}");
    }

    #[test]
    fn in_tune_signal_passes_with_latency() {
        let sr = 48000.0;
        let input = sine(440.0, sr, 24000);
        let mut fx = AutotuneFx::new(sr);
        let p = AutotuneParams { key: 9, scale: Scale::Minor, mix: 0.0, ..Default::default() };
        let mut l = input.clone();
        let mut r = input.clone();
        fx.process(&p, &mut l, &mut r);
        let lat = fx.latency();
        for i in lat..24000 {
            assert!((l[i] - input[i - lat]).abs() < 1e-6);
        }
        // silence stays silent (no noise from the shifter)
        let mut fx = AutotuneFx::new(sr);
        let mut z = vec![0.0; 9000];
        let mut z2 = vec![0.0; 9000];
        fx.process(&AutotuneParams::default(), &mut z, &mut z2);
        assert!(z.iter().all(|x| x.abs() < 1e-9));
    }
}
