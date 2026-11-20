//! Tempo changes for samples: pitch-preserving time-stretch (phase vocoder,
//! WSOLA for very short sounds) and vinyl-style resampling.

use crate::sample::SampleData;

mod pvoc;
pub use pvoc::pv_stretch;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
pub enum StretchMode {
    /// Play at original speed.
    #[default]
    Off,
    /// Change tempo, keep pitch.
    Stretch,
    /// Change tempo and pitch together (like a record).
    Resample,
}

impl StretchMode {
    pub const ALL: [StretchMode; 3] = [StretchMode::Off, StretchMode::Stretch, StretchMode::Resample];
    pub fn name(self) -> &'static str {
        match self {
            StretchMode::Off => "Off",
            StretchMode::Stretch => "Stretch (Tonhöhe halten)",
            StretchMode::Resample => "Resample (Vinyl)",
        }
    }
}

/// Converts a sample recorded at `source_bpm` so it plays in time at `target_bpm`.
/// In `Stretch` mode `pitch_semis` additionally shifts pitch without changing length.
pub fn change_tempo(input: &SampleData, mode: StretchMode, source_bpm: f64, target_bpm: f64, pitch_semis: f32) -> SampleData {
    change_tempo_ex(input, mode, source_bpm, target_bpm, pitch_semis, false)
}

/// Like [`change_tempo`]; `formant` keeps the vocal character (formants) when pitching.
pub fn change_tempo_ex(input: &SampleData, mode: StretchMode, source_bpm: f64, target_bpm: f64, pitch_semis: f32, formant: bool) -> SampleData {
    if formant && pitch_semis.abs() > 1e-3 {
        let plain = change_tempo_ex(input, mode, source_bpm, target_bpm, 0.0, false);
        let shifted = change_tempo_ex(input, mode, source_bpm, target_bpm, pitch_semis, false);
        return crate::formant::preserve_formants(&plain, &shifted);
    }
    if source_bpm <= 0.0 || target_bpm <= 0.0 {
        return input.clone();
    }
    let speed = target_bpm / source_bpm;
    match mode {
        // tempo untouched, only the extra pitch shift
        StretchMode::Off => pitch_shift(input, pitch_semis),
        // like a record (tempo and pitch together), plus the extra pitch shift on top
        StretchMode::Resample => {
            let r = if (speed - 1.0).abs() < 1e-9 { input.clone() } else { input.resample_speed(speed) };
            pitch_shift(&r, pitch_semis)
        }
        StretchMode::Stretch => {
            let pr = 2f64.powf(pitch_semis as f64 / 12.0);
            if (pr - 1.0).abs() < 1e-6 {
                time_stretch(input, 1.0 / speed)
            } else {
                time_stretch(input, pr / speed).resample_speed(pr)
            }
        }
    }
}

/// Pitch shift that keeps duration.
pub fn pitch_shift(input: &SampleData, semis: f32) -> SampleData {
    let pr = 2f64.powf(semis as f64 / 12.0);
    if (pr - 1.0).abs() < 1e-6 {
        return input.clone();
    }
    time_stretch(input, pr).resample_speed(pr)
}

/// Time-stretch, pitch preserved. Output length ≈ `input.len() * factor`.
/// Phase vocoder for anything longer than a few frames, WSOLA for tiny one-shots.
pub fn time_stretch(input: &SampleData, factor: f64) -> SampleData {
    let n_in = input.len();
    if n_in == 0 || (factor - 1.0).abs() < 1e-6 || factor <= 0.0 {
        return input.clone();
    }
    if n_in >= 4 * pvoc::frame_size(input.sample_rate) {
        return pv_stretch(input, factor);
    }
    wsola_stretch(input, factor)
}

/// WSOLA time-stretch (the previous algorithm; still used for very short sounds).
pub fn wsola_stretch(input: &SampleData, factor: f64) -> SampleData {
    let n_in = input.len();
    if n_in == 0 || (factor - 1.0).abs() < 1e-6 || factor <= 0.0 {
        return input.clone();
    }
    let sr = input.sample_rate.max(8000) as f64;
    let win = (((0.03 * sr) as usize).max(128)) & !1;
    let hs = win / 2;
    let ha = hs as f64 / factor;
    let tol = (win / 4) as isize;
    let out_len = (n_in as f64 * factor).round() as usize;

    let mono = input.mono();
    let window: Vec<f32> = (0..win).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / win as f32).cos()).collect();
    let total = out_len + win;
    let mut out_l = vec![0.0f32; total];
    let mut out_r = vec![0.0f32; total];
    let mut wsum = vec![0.0f32; total];

    let mut prev: isize = 0;
    let mut k = 0usize;
    loop {
        let out_pos = k * hs;
        if out_pos >= out_len {
            break;
        }
        let nominal = (k as f64 * ha).round() as isize;
        let pos = if k == 0 { 0 } else { best_offset(&mono, prev + hs as isize, nominal, tol, win) };
        for i in 0..win {
            let src = pos + i as isize;
            if src >= 0 && (src as usize) < n_in {
                let w = window[i];
                out_l[out_pos + i] += input.left[src as usize] * w;
                out_r[out_pos + i] += input.right[src as usize] * w;
            }
            wsum[out_pos + i] += window[i];
        }
        prev = pos;
        k += 1;
    }
    for i in 0..total {
        let w = wsum[i];
        if w > 1e-3 {
            out_l[i] /= w;
            out_r[i] /= w;
        }
    }
    out_l.truncate(out_len);
    out_r.truncate(out_len);
    SampleData::new(out_l, out_r, input.sample_rate)
}

#[inline]
fn sample_at(x: &[f32], i: isize) -> f32 {
    if i >= 0 && (i as usize) < x.len() {
        x[i as usize]
    } else {
        0.0
    }
}

/// Finds the input position near `nominal` whose waveform best continues `natural`.
fn best_offset(x: &[f32], natural: isize, nominal: isize, tol: isize, win: usize) -> isize {
    let len = win as isize;
    let corr = |cand: isize, stride: usize| -> f32 {
        let mut dot = 0.0f32;
        let mut energy = 1e-9f32;
        let mut i = 0isize;
        while i < len {
            let a = sample_at(x, natural + i);
            let b = sample_at(x, cand + i);
            dot += a * b;
            energy += b * b;
            i += stride as isize;
        }
        dot / energy.sqrt()
    };
    let lo = (nominal - tol).max(0);
    let hi = (nominal + tol).min(x.len() as isize - 1).max(lo);
    // coarse search
    let mut best = nominal.clamp(lo, hi);
    let mut best_v = f32::MIN;
    let mut c = lo;
    while c <= hi {
        let v = corr(c, 4);
        if v > best_v {
            best_v = v;
            best = c;
        }
        c += 4;
    }
    // refine
    let center = best;
    for c in (center - 4).max(lo)..=(center + 4).min(hi) {
        let v = corr(c, 2);
        if v > best_v {
            best_v = v;
            best = c;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, secs: f32, sr: u32) -> SampleData {
        let n = (secs * sr as f32) as usize;
        let v: Vec<f32> = (0..n).map(|i| (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin() * 0.5).collect();
        SampleData::new(v.clone(), v, sr)
    }

    fn zero_crossing_freq(x: &[f32], sr: u32) -> f32 {
        let skip = x.len() / 10;
        let seg = &x[skip..x.len() - skip];
        let zc = seg.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        zc as f32 / (seg.len() as f32 / sr as f32)
    }

    #[test]
    fn stretch_changes_length_but_not_pitch() {
        let s = sine(440.0, 1.0, 22050);
        for factor in [0.7, 1.3, 2.0] {
            let out = time_stretch(&s, factor);
            let expected = (s.len() as f64 * factor) as usize;
            assert!((out.len() as i64 - expected as i64).abs() <= 2);
            let f = zero_crossing_freq(&out.left, 22050);
            assert!((f - 440.0).abs() < 440.0 * 0.03, "factor {factor}: got {f} Hz");
        }
    }

    #[test]
    fn resample_changes_pitch_and_length() {
        let s = sine(440.0, 1.0, 22050);
        let out = change_tempo(&s, StretchMode::Resample, 100.0, 200.0, 0.0);
        assert!((out.len() as i64 - s.len() as i64 / 2).abs() <= 2);
        let f = zero_crossing_freq(&out.left, 22050);
        assert!((f - 880.0).abs() < 880.0 * 0.03, "got {f}");
    }

    #[test]
    fn pitch_works_in_every_mode() {
        let s = sine(440.0, 1.0, 22050);
        // Off: same length, one octave up
        let out = change_tempo(&s, StretchMode::Off, 120.0, 150.0, 12.0);
        assert!((out.len() as i64 - s.len() as i64).abs() <= 4);
        let f = zero_crossing_freq(&out.left, 22050);
        assert!((f - 880.0).abs() < 880.0 * 0.04, "off: got {f}");
        // Resample ×2 (one octave) plus -12 st = original pitch, half length
        let out = change_tempo(&s, StretchMode::Resample, 100.0, 200.0, -12.0);
        assert!((out.len() as i64 - s.len() as i64 / 2).abs() <= 4);
        let f = zero_crossing_freq(&out.left, 22050);
        assert!((f - 440.0).abs() < 440.0 * 0.04, "resample: got {f}");
    }

    #[test]
    fn stretch_tempo_with_pitch_shift() {
        let s = sine(440.0, 1.0, 22050);
        let out = change_tempo(&s, StretchMode::Stretch, 120.0, 150.0, 12.0);
        let expected = (s.len() as f64 * 120.0 / 150.0) as i64;
        assert!((out.len() as i64 - expected).abs() <= 4);
        let f = zero_crossing_freq(&out.left, 22050);
        assert!((f - 880.0).abs() < 880.0 * 0.04, "got {f}");
    }

    /// Fraction of the spectral energy that lies within ±`bw` Hz of the given partials.
    fn purity(x: &[f32], sr: u32, partials: &[f32], bw: f32) -> f64 {
        let n = 1 << 15;
        let off = (x.len() - n) / 2;
        let w = crate::fft::hann(n);
        let mut re: Vec<f64> = (0..n).map(|i| x[off + i] as f64 * w[i]).collect();
        let mut im = vec![0.0; n];
        crate::fft::fft(&mut re, &mut im, false);
        let (mut near, mut all) = (0.0, 1e-12);
        for b in 1..n / 2 {
            let e = re[b] * re[b] + im[b] * im[b];
            let f = b as f32 * sr as f32 / n as f32;
            all += e;
            if partials.iter().any(|p| (f - p).abs() <= bw) {
                near += e;
            }
        }
        near / all
    }

    fn chord(secs: f32, sr: u32) -> SampleData {
        let n = (secs * sr as f32) as usize;
        let fs = [220.0f32, 277.18, 329.63, 440.0];
        let v: Vec<f32> = (0..n).map(|i| fs.iter().map(|f| (std::f32::consts::TAU * f * i as f32 / sr as f32).sin()).sum::<f32>() * 0.2).collect();
        SampleData::new(v.clone(), v, sr)
    }

    #[test]
    fn phase_vocoder_is_cleaner_than_wsola_on_chords() {
        let sr = 44100;
        let s = chord(3.0, sr);
        let fs = [220.0f32, 277.18, 329.63, 440.0];
        for factor in [0.67, 0.85, 1.3] {
            let pv = purity(&pv_stretch(&s, factor).left, sr, &fs, 6.0);
            let ws = purity(&wsola_stretch(&s, factor).left, sr, &fs, 6.0);
            assert!(pv > 0.995, "factor {factor}: phase vocoder purity {pv}");
            assert!(pv >= ws, "factor {factor}: pv {pv} < wsola {ws}");
        }
    }

    #[test]
    fn phase_vocoder_keeps_stereo_and_transients() {
        let sr = 44100;
        let n = sr as usize * 2;
        // clicks every 0.25 s, right channel inverted
        let l: Vec<f32> = (0..n).map(|i| if i % (sr as usize / 4) < 20 { 0.9 } else { 0.0 }).collect();
        let r: Vec<f32> = l.iter().map(|v| -v).collect();
        let out = pv_stretch(&SampleData::new(l.clone(), r, sr), 0.7);
        // stereo: still inverted
        let corr: f32 = out.left.iter().zip(&out.right).map(|(a, b)| a * b).sum();
        assert!(corr < 0.0, "stereo image lost");
        check_clicks(&out, sr);
        // same clicks in mono on top of a sustained chord (kick over a pad)
        let pad = chord(2.0, sr);
        let mixed: Vec<f32> = l.iter().zip(&pad.left).map(|(c, p)| c + 0.3 * p).collect();
        let out = pv_stretch(&SampleData::new(mixed.clone(), mixed, sr), 0.7);
        check_clicks(&out, sr);
    }

    fn check_clicks(out: &SampleData, sr: u32) {
        // transients: every click comes out as a distinct peak at its stretched place
        // (measured on the first difference, so a low pad underneath does not count)
        let x: Vec<f32> = out.left.windows(2).map(|w| w[1] - w[0]).collect();
        let step = (sr as f64 / 4.0 * 0.7) as usize;
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        for k in 1..7 {
            let c = k * step;
            let local = x[c - 200..c + 400].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let between = x[c + step / 2 - 300..c + step / 2 + 300].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(local > 0.5 * peak, "click {k} lost ({local} vs {peak})");
            assert!(between < 0.3 * local, "click {k} smeared ({between} vs {local})");
        }
    }
}
