//! Lightweight real-time spectrum analyzer for the mixer display.
//!
//! Collects a mono sum in a ring buffer and, every `hop` samples, runs a Hann-windowed
//! 2048-point FFT and reduces it to [`SPECTRUM_BANDS`] log-spaced bands (dBFS, peak per band,
//! with a short release). All buffers are allocated in [`SpectrumAnalyzer::new`]; `push`
//! never allocates, so it can run on the audio thread.

use crate::fft::{fft, hann};

/// FFT size (frames).
pub const SPECTRUM_FFT: usize = 2048;
/// Number of log-spaced display bands between [`SPECTRUM_LO_HZ`] and [`SPECTRUM_HI_HZ`].
pub const SPECTRUM_BANDS: usize = 64;
pub const SPECTRUM_LO_HZ: f32 = 20.0;
pub const SPECTRUM_HI_HZ: f32 = 20000.0;
/// Bottom of the display range; silence reads as this value.
pub const SPECTRUM_FLOOR_DB: f32 = -90.0;

/// Lower edge frequency of band `i` (`i == SPECTRUM_BANDS` gives the top edge).
pub fn band_edge_hz(i: usize) -> f32 {
    SPECTRUM_LO_HZ * (SPECTRUM_HI_HZ / SPECTRUM_LO_HZ).powf(i as f32 / SPECTRUM_BANDS as f32)
}

/// Geometric centre frequency of band `i`.
pub fn band_center_hz(i: usize) -> f32 {
    (band_edge_hz(i) * band_edge_hz(i + 1)).sqrt()
}

pub struct SpectrumAnalyzer {
    ring: Vec<f32>,
    pos: usize,
    filled: usize,
    since: usize,
    hop: usize,
    win: Vec<f64>,
    win_sum: f64,
    re: Vec<f64>,
    im: Vec<f64>,
    /// FFT bin range [lo, hi) per band
    bins: [(usize, usize); SPECTRUM_BANDS],
    bands: [f32; SPECTRUM_BANDS],
}

impl SpectrumAnalyzer {
    /// `hop` = samples between two analyses (e.g. 2048 ≈ 23 updates/s at 48 kHz).
    pub fn new(sr: f32, hop: usize) -> Self {
        let n = SPECTRUM_FFT;
        let win = hann(n);
        let win_sum = win.iter().sum();
        let bin_hz = sr.max(1.0) / n as f32;
        let mut bins = [(0usize, 1usize); SPECTRUM_BANDS];
        for (i, b) in bins.iter_mut().enumerate() {
            let half = n / 2;
            let lo = ((band_edge_hz(i) / bin_hz).round() as usize).clamp(1, half);
            let hi = ((band_edge_hz(i + 1) / bin_hz).round() as usize).clamp(lo + 1, half + 1);
            *b = (lo, hi);
        }
        Self {
            ring: vec![0.0; n],
            pos: 0,
            filled: 0,
            since: 0,
            hop: hop.max(64),
            win,
            win_sum,
            re: vec![0.0; n],
            im: vec![0.0; n],
            bins,
            bands: [SPECTRUM_FLOOR_DB; SPECTRUM_BANDS],
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.pos = 0;
        self.filled = 0;
        self.since = 0;
        self.bands = [SPECTRUM_FLOOR_DB; SPECTRUM_BANDS];
    }

    /// Current band levels in dBFS (a full-scale sine reads about 0 dB).
    pub fn bands(&self) -> &[f32; SPECTRUM_BANDS] {
        &self.bands
    }

    /// Feeds one block (mono sum of `l` and `r`). Returns true when new bands were computed.
    pub fn push(&mut self, l: &[f32], r: &[f32]) -> bool {
        let n = SPECTRUM_FFT;
        let mut updated = false;
        for (a, b) in l.iter().zip(r) {
            self.ring[self.pos] = 0.5 * (a + b);
            self.pos = (self.pos + 1) % n;
            self.filled = (self.filled + 1).min(n);
            self.since += 1;
            if self.since >= self.hop && self.filled == n {
                self.since = 0;
                self.analyze();
                updated = true;
            }
        }
        updated
    }

    fn analyze(&mut self) {
        let n = SPECTRUM_FFT;
        for i in 0..n {
            // oldest sample first
            self.re[i] = self.ring[(self.pos + i) % n] as f64 * self.win[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im, false);
        // a sine of amplitude A peaks at A * win_sum / 2
        let scale = 2.0 / self.win_sum;
        for (b, &(lo, hi)) in self.bands.iter_mut().zip(self.bins.iter()) {
            let mut peak = 0.0f64;
            for k in lo..hi {
                peak = peak.max(self.re[k].hypot(self.im[k]));
            }
            let db = (20.0 * (peak * scale).max(1e-9).log10()) as f32;
            let db = db.max(SPECTRUM_FLOOR_DB);
            // instant attack, smooth release
            *b = if db >= *b { db } else { *b + (db - *b) * 0.35 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32, sr: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| (std::f32::consts::TAU * freq * i as f32 / sr).sin() * amp).collect()
    }

    fn band_of(f: f32) -> usize {
        (0..SPECTRUM_BANDS).find(|&i| band_edge_hz(i) <= f && f < band_edge_hz(i + 1)).unwrap()
    }

    #[test]
    fn sine_peaks_in_its_band() {
        let sr = 48000.0;
        let mut a = SpectrumAnalyzer::new(sr, 2048);
        let x = sine(1000.0, 0.5, sr, 8192);
        assert!(a.push(&x, &x));
        let b = a.bands();
        let k = band_of(1000.0);
        // 0.5 amplitude = -6 dBFS (band edges may split the main lobe a little)
        assert!((b[k] + 6.0).abs() < 1.5, "1 kHz band {}", b[k]);
        assert!(b[band_of(100.0)] < -50.0, "100 Hz band {}", b[band_of(100.0)]);
        assert!(b[band_of(10000.0)] < -50.0, "10 kHz band {}", b[band_of(10000.0)]);
        let loudest = (0..SPECTRUM_BANDS).max_by(|x, y| b[*x].total_cmp(&b[*y])).unwrap();
        assert!(loudest.abs_diff(k) <= 1);
    }

    #[test]
    fn low_bass_is_resolved() {
        let sr = 48000.0;
        let mut a = SpectrumAnalyzer::new(sr, 2048);
        let x = sine(50.0, 0.8, sr, 8192);
        a.push(&x, &x);
        let b = a.bands();
        assert!(b[band_of(50.0)] > -6.0, "50 Hz band {}", b[band_of(50.0)]);
        assert!(b[band_of(2000.0)] < -50.0);
    }

    #[test]
    fn silence_reads_floor_and_hop_controls_updates() {
        let mut a = SpectrumAnalyzer::new(48000.0, 2048);
        let z = vec![0.0f32; 1024];
        // ring not full yet → no analysis
        assert!(!a.push(&z, &z));
        assert!(a.push(&z, &z));
        assert!(a.bands().iter().all(|v| *v == SPECTRUM_FLOOR_DB));
        // fewer than `hop` new samples → no update
        assert!(!a.push(&z[..512], &z[..512]));
    }

    #[test]
    fn release_is_smoothed_and_reset_clears() {
        let sr = 48000.0;
        let mut a = SpectrumAnalyzer::new(sr, 2048);
        let x = sine(1000.0, 0.5, sr, 4096);
        a.push(&x, &x);
        let k = band_of(1000.0);
        let loud = a.bands()[k];
        let z = vec![0.0f32; 2048];
        a.push(&z, &z);
        let after = a.bands()[k];
        assert!(after < loud && after > SPECTRUM_FLOOR_DB, "{loud} → {after}");
        a.reset();
        assert!(a.bands().iter().all(|v| *v == SPECTRUM_FLOOR_DB));
    }

    #[test]
    fn band_edges_are_monotonic() {
        assert!((band_edge_hz(0) - 20.0).abs() < 1e-3);
        assert!((band_edge_hz(SPECTRUM_BANDS) - 20000.0).abs() < 1.0);
        for i in 0..SPECTRUM_BANDS {
            assert!(band_center_hz(i) > band_edge_hz(i) && band_center_hz(i) < band_edge_hz(i + 1));
        }
    }
}
