//! Phase-vocoder time-stretch with identity phase locking (Laroche & Dolson) and
//! transient phase reset. Much cleaner than WSOLA on full mixes: no warbling or
//! doubled drum hits. Left and right get the same phase rotation per bin, so the
//! stereo image stays intact.

use crate::fft::{fft, hann};
use crate::sample::SampleData;
use std::f64::consts::{PI, TAU};

#[inline]
fn wrap(x: f64) -> f64 {
    x - TAU * ((x + PI) / TAU).floor()
}

/// FFT size for a sample rate: 4096 at 44.1/48 kHz (≈ 90 ms) – fine enough to separate
/// close partials; transients are kept sharp by the phase reset instead.
pub(super) fn frame_size(sr: u32) -> usize {
    ((0.06 * sr.max(8000) as f64) as usize).next_power_of_two().clamp(1024, 8192)
}

/// Phase-vocoder stretch. Output length ≈ `input.len() * factor`, pitch preserved.
pub fn pv_stretch(input: &SampleData, factor: f64) -> SampleData {
    let n_in = input.len();
    let n = frame_size(input.sample_rate);
    let half = n / 2;
    let bins = half + 1;
    let hs = n / 4; // synthesis hop (75 % overlap)
    let ha = hs as f64 / factor; // analysis hop (fractional)
    let out_len = (n_in as f64 * factor).round() as usize;
    let win = hann(n);

    // output buffer is shifted by `half` so frame k is centred on output sample k*hs
    let total = out_len + 2 * n + hs;
    let mut out_l = vec![0.0f64; total];
    let mut out_r = vec![0.0f64; total];
    let mut wsum = vec![0.0f64; total];

    let (mut re, mut im) = (vec![0.0f64; n], vec![0.0f64; n]);
    let mut prev_in_phase = vec![0.0f64; bins];
    let mut prev_out_phase = vec![0.0f64; bins];
    let mut mag = vec![0.0f64; bins];
    let mut phase = vec![0.0f64; bins];
    let mut out_phase = vec![0.0f64; bins];
    let (mut xl, mut xr) = (vec![(0.0f64, 0.0f64); bins], vec![(0.0f64, 0.0f64); bins]);
    let onsets = detect_onsets(input);
    let mut next_onset = 0usize;
    let mut prev_pos: i64 = 0;
    // Analysis position (frame centre). Around an onset the frames advance by exactly
    // `hs` (no stretching → the hit is reproduced 1:1). Before that the position is
    // steered so the hit comes out exactly at `onset * factor`; afterwards it returns
    // to the nominal timeline.
    let mut pos_f = 0.0f64;
    let mut hold_until: Option<f64> = None; // identity mode until this input position leaves the window
    let budget = 2.0 * n as f64; // max. drift from the nominal position
    // frame in which a hold for onset `o` has to start, and where that frame must sit:
    // output time of the hit = K·hs + (o − pos_K) must equal o·factor
    let plan = |o: f64| -> (usize, f64) {
        let kk = ((o * factor - half as f64) / hs as f64).ceil().max(0.0);
        (kk as usize, kk * hs as f64 + o * (1.0 - factor))
    };

    let mut k = 0usize;
    while k * hs < out_len + half {
        let nominal = k as f64 * ha;
        if k > 0 {
            pos_f += if hold_until.is_some() {
                hs as f64
            } else {
                match onsets.get(next_onset).map(|&o| plan(o as f64)) {
                    // approaching a hit: glide to the planned position of its first frame
                    Some((kk, target)) if kk >= k && kk - k < 4 => ((target - pos_f) / (kk - k + 1) as f64).clamp(0.25 * ha, 3.0 * ha),
                    // otherwise follow the nominal timeline (`nominal - pos_f` contains this hop)
                    _ => (ha + (nominal - pos_f - ha) * 0.5).clamp(0.25 * ha, 3.0 * ha),
                }
            };
        }
        let pos = pos_f.round() as i64;
        if hold_until.is_some_and(|o| o < pos_f - half as f64) {
            hold_until = None;
        }
        // onsets in the window start (or extend) the identity stretch
        let mut transient = false;
        while next_onset < onsets.len() && (onsets[next_onset] as f64) <= pos_f + half as f64 {
            let o = onsets[next_onset] as f64;
            if hold_until.is_none() && k < plan(o).0 {
                break; // its planned start frame is still ahead
            }
            next_onset += 1;
            if o < pos_f - half as f64 || (nominal - pos_f).abs() > budget {
                continue; // already passed, or too far off the timeline
            }
            if hold_until.is_none() {
                transient = true; // fresh hit: take the input phases as they are
            }
            hold_until = Some(o);
        }
        // ---- analysis: L + iR in one FFT
        for i in 0..n {
            let src = pos - half as i64 + i as i64;
            let (l, r) = if src >= 0 && (src as usize) < n_in { (input.left[src as usize] as f64, input.right[src as usize] as f64) } else { (0.0, 0.0) };
            re[i] = l * win[i];
            im[i] = r * win[i];
        }
        fft(&mut re, &mut im, false);
        for b in 0..bins {
            let m = (n - b) % n;
            let (zr, zi, cr, ci) = (re[b], im[b], re[m], -im[m]);
            xl[b] = (0.5 * (zr + cr), 0.5 * (zi + ci));
            xr[b] = (0.5 * (zi - ci), -0.5 * (zr - cr));
            // the louder channel of each bin drives the phase decisions (works for
            // mono, wide and phase-inverted material alike)
            let el = xl[b].0 * xl[b].0 + xl[b].1 * xl[b].1;
            let er = xr[b].0 * xr[b].0 + xr[b].1 * xr[b].1;
            let (mr, mi) = if el >= er { xl[b] } else { xr[b] };
            mag[b] = (mr * mr + mi * mi).sqrt();
            phase[b] = mi.atan2(mr);
        }

        // ---- synthesis phases
        if k == 0 || transient {
            out_phase.copy_from_slice(&phase);
        } else {
            let hop = (pos - prev_pos).max(1) as f64;
            // peaks of the magnitude spectrum
            let mut peaks: Vec<usize> = Vec::new();
            for b in 2..bins.saturating_sub(2) {
                let v = mag[b];
                if v > mag[b - 1] && v >= mag[b + 1] && v > mag[b - 2] && v >= mag[b + 2] {
                    peaks.push(b);
                }
            }
            if peaks.is_empty() {
                peaks.push(mag.iter().enumerate().fold(0, |bi, (i, v)| if *v > mag[bi] { i } else { bi }));
            }
            for &p in &peaks {
                let omega = TAU * p as f64 / n as f64;
                let dphi = wrap(phase[p] - prev_in_phase[p] - omega * hop);
                let inst = omega + dphi / hop;
                out_phase[p] = prev_out_phase[p] + inst * hs as f64;
            }
            // identity phase locking: every bin follows its nearest peak
            let mut pi = 0usize;
            for b in 0..bins {
                while pi + 1 < peaks.len() && (peaks[pi + 1] as i64 - b as i64).abs() <= (peaks[pi] as i64 - b as i64).abs() {
                    pi += 1;
                }
                let p = peaks[pi];
                if b != p {
                    out_phase[b] = out_phase[p] + phase[b] - phase[p];
                }
            }
        }
        prev_in_phase.copy_from_slice(&phase);
        prev_out_phase.copy_from_slice(&out_phase);
        prev_pos = pos;

        // ---- rotate L and R by the same angle, inverse FFT (YL + iYR)
        for b in 0..bins {
            // DC and Nyquist stay real (otherwise they would leak between L and R)
            let rot = if b == 0 || b == half { 0.0 } else { out_phase[b] - phase[b] };
            let (c, s) = (rot.cos(), rot.sin());
            let yl = (xl[b].0 * c - xl[b].1 * s, xl[b].0 * s + xl[b].1 * c);
            let yr = (xr[b].0 * c - xr[b].1 * s, xr[b].0 * s + xr[b].1 * c);
            // Z = YL + i·YR, with YL/YR Hermitian
            re[b] = yl.0 - yr.1;
            im[b] = yl.1 + yr.0;
            if b != 0 && b != half {
                let m = n - b;
                re[m] = yl.0 + yr.1;
                im[m] = -yl.1 + yr.0;
            }
        }
        fft(&mut re, &mut im, true);
        let o = k * hs;
        for i in 0..n {
            let w = win[i];
            out_l[o + i] += re[i] * w;
            out_r[o + i] += im[i] * w;
            wsum[o + i] += w * w;
        }
        k += 1;
    }

    let mut l = Vec::with_capacity(out_len);
    let mut r = Vec::with_capacity(out_len);
    for i in half..half + out_len {
        let w = wsum[i].max(1e-3);
        l.push((out_l[i] / w) as f32);
        r.push((out_r[i] / w) as f32);
    }
    SampleData::new(l, r, input.sample_rate)
}

/// Onsets (sample positions) of percussive hits: the high-passed energy jumps by
/// more than 6 dB over the recent past. At least 50 ms apart.
fn detect_onsets(input: &SampleData) -> Vec<usize> {
    let sr = input.sample_rate.max(8000) as usize;
    let blk = (sr / 200).max(32); // 5 ms
    let n = input.len();
    let mut env: Vec<f64> = Vec::with_capacity(n / blk + 1);
    let (mut pl, mut pr) = (0.0f32, 0.0f32);
    for start in (0..n).step_by(blk) {
        let end = (start + blk).min(n);
        let mut e = 0.0f64;
        for i in start..end {
            // first difference = gentle high-pass; both channels (side-only hits count too)
            let (dl, dr) = ((input.left[i] - pl) as f64, (input.right[i] - pr) as f64);
            pl = input.left[i];
            pr = input.right[i];
            e += dl * dl + dr * dr;
        }
        env.push(e / (end - start) as f64);
    }
    let mean = env.iter().sum::<f64>() / env.len().max(1) as f64;
    let floor = mean * 0.05;
    let min_gap = (sr / 20) / blk; // 50 ms
    let mut out = Vec::new();
    let mut last: Option<usize> = None;
    for i in 4..env.len() {
        let past = env[i - 4..i].iter().cloned().fold(0.0, f64::max);
        if env[i] > floor && env[i] > 4.0 * past.max(1e-12) && last.is_none_or(|l| i - l >= min_gap) {
            // refine to the first sample of the block that carries the jump
            out.push(i * blk);
            last = Some(i);
        }
    }
    out
}
