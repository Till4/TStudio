//! Formant-preserving pitch shift: pitch-shift as usual, then put the original spectral
//! envelope (cepstrally smoothed) back onto every STFT frame, so voices don't turn into
//! chipmunks when pitched up (or into giants when pitched down).

use crate::fft::{fft, hann};
use crate::SampleData;

const N: usize = 2048;
const HOP: usize = N / 4;

/// Cepstrally smoothed log-magnitude envelope of a magnitude spectrum (N/2+1 bins).
fn envelope(mag: &[f64], lifter: usize, re: &mut [f64], im: &mut [f64]) -> Vec<f64> {
    for i in 0..N {
        let k = if i <= N / 2 { i } else { N - i };
        re[i] = (mag[k] + 1e-9).ln();
        im[i] = 0.0;
    }
    fft(re, im, true);
    for i in 0..N {
        let q = if i <= N / 2 { i } else { N - i };
        if q > lifter {
            re[i] = 0.0;
            im[i] = 0.0;
        } else if q == lifter {
            re[i] *= 0.5;
            im[i] *= 0.5;
        }
    }
    fft(re, im, false);
    (0..=N / 2).map(|i| re[i]).collect()
}

fn process_channel(orig: &[f32], shifted: &[f32], sr: u32) -> Vec<f32> {
    let len = shifted.len();
    let win = hann(N);
    // envelope detail: ~1.2 ms quefrency keeps formants but not harmonics
    let lifter = ((sr as f64 * 0.0012) as usize).clamp(12, N / 8);
    let mut out = vec![0.0f64; len + N];
    let mut norm = vec![0.0f64; len + N];
    let (mut ore, mut oim) = (vec![0.0; N], vec![0.0; N]);
    let (mut sre, mut sim) = (vec![0.0; N], vec![0.0; N]);
    let (mut tre, mut tim) = (vec![0.0; N], vec![0.0; N]);
    let max_corr = (18.0f64 / 20.0 * std::f64::consts::LN_10).abs();
    let mut pos: isize = -(N as isize) / 2;
    while pos < len as isize {
        for i in 0..N {
            let k = pos + i as isize;
            let g = |x: &[f32]| if k >= 0 && (k as usize) < x.len() { x[k as usize] as f64 } else { 0.0 };
            ore[i] = g(orig) * win[i];
            oim[i] = 0.0;
            sre[i] = g(shifted) * win[i];
            sim[i] = 0.0;
        }
        fft(&mut ore, &mut oim, false);
        fft(&mut sre, &mut sim, false);
        let om: Vec<f64> = (0..=N / 2).map(|i| ore[i].hypot(oim[i])).collect();
        let sm: Vec<f64> = (0..=N / 2).map(|i| sre[i].hypot(sim[i])).collect();
        let eo = envelope(&om, lifter, &mut tre, &mut tim);
        let es = envelope(&sm, lifter, &mut tre, &mut tim);
        for i in 0..=N / 2 {
            let g = (eo[i] - es[i]).clamp(-max_corr, max_corr).exp();
            sre[i] *= g;
            sim[i] *= g;
            if i > 0 && i < N / 2 {
                sre[N - i] = sre[i];
                sim[N - i] = -sim[i];
            }
        }
        fft(&mut sre, &mut sim, true);
        for i in 0..N {
            let k = pos + i as isize;
            if k >= 0 && (k as usize) < len {
                out[k as usize] += sre[i] * win[i];
                norm[k as usize] += win[i] * win[i];
            }
        }
        pos += HOP as isize;
    }
    (0..len).map(|i| if norm[i] > 1e-6 { (out[i] / norm[i]) as f32 } else { 0.0 }).collect()
}

/// Pitch shift by `semis` keeping both the length and the formants (vocal character).
pub fn pitch_shift_formant(input: &SampleData, semis: f32) -> SampleData {
    let shifted = crate::stretch::pitch_shift(input, semis);
    if (semis).abs() < 1e-3 {
        return shifted;
    }
    preserve_formants(input, &shifted)
}

/// Gives `shifted` (same length as `orig`, already pitch-shifted) the formants of `orig`.
pub fn preserve_formants(orig: &SampleData, shifted: &SampleData) -> SampleData {
    let sr = shifted.sample_rate;
    let n = shifted.len().min(orig.len());
    let l = process_channel(&orig.left[..n], &shifted.left[..n], sr);
    let r = if orig.left == orig.right && shifted.left == shifted.right { l.clone() } else { process_channel(&orig.right[..n], &shifted.right[..n], sr) };
    SampleData::new(l, r, sr)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Harmonic tone through a fixed "formant" resonance at ~1 kHz.
    fn vowel(f0: f32, sr: u32, secs: f32) -> SampleData {
        let n = (sr as f32 * secs) as usize;
        let v: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (1..40)
                    .map(|h| {
                        let f = f0 * h as f32;
                        let g = (-((f - 1000.0) / 250.0).powi(2)).exp() + 0.05;
                        g * (2.0 * std::f32::consts::PI * f * t).sin()
                    })
                    .sum::<f32>()
                    * 0.1
            })
            .collect();
        SampleData::new(v.clone(), v, sr)
    }

    fn centroid(d: &SampleData) -> f64 {
        let p = crate::meter::avg_power(&d.mono(), 4096, 40);
        let bin = d.sample_rate as f64 / 4096.0;
        let (mut a, mut b) = (0.0, 0.0);
        for (i, v) in p.iter().enumerate().take_while(|(i, _)| (*i as f64 * bin) < 3000.0) {
            a += i as f64 * bin * v;
            b += v;
        }
        a / b
    }

    #[test]
    fn formants_stay_put() {
        let sr = 44100;
        let x = vowel(150.0, sr, 1.0);
        let plain = crate::stretch::pitch_shift(&x, 7.0);
        let kept = pitch_shift_formant(&x, 7.0);
        let (c0, c1, c2) = (centroid(&x), centroid(&plain), centroid(&kept));
        assert!(kept.len().abs_diff(x.len()) <= 2);
        assert!(c1 > c0 * 1.3, "plain shift moves the formant up: {c0} → {c1}");
        assert!((c2 / c0 - 1.0).abs() < 0.15, "formant kept: {c0} → {c2}");
    }
}
