//! Offline analysis: onset envelope, BPM, key and transient detection.

/// Onset strength envelope (positive log-energy flux of a high-passed signal).
pub struct OnsetEnv {
    pub env: Vec<f32>,
    pub hop: usize,
    pub sr: u32,
}

pub fn onset_envelope(mono: &[f32], sr: u32, hop: usize) -> OnsetEnv {
    let hop = hop.max(16);
    let frames = mono.len() / hop;
    let mut energies = Vec::with_capacity(frames);
    let mut prev = 0.0f32;
    for f in 0..frames {
        let mut e = 0.0f32;
        for &x in &mono[f * hop..(f + 1) * hop] {
            let d = x - prev;
            prev = x;
            e += d * d + 0.25 * x * x;
        }
        energies.push((e + 1e-7).ln());
    }
    let mut env = Vec::with_capacity(frames);
    for i in 0..frames {
        let d = if i == 0 { 0.0 } else { energies[i] - energies[i - 1] };
        env.push(d.max(0.0));
    }
    OnsetEnv { env, hop, sr }
}

/// Parses tempo from file names like `loop_140bpm.wav`, `Drums 92 BPM`, `bpm128_kit`.
pub fn bpm_from_name(name: &str) -> Option<f64> {
    let lower = name.to_lowercase();
    let bytes = lower.as_bytes();
    let mut search = 0;
    while let Some(rel) = lower[search..].find("bpm") {
        let at = search + rel;
        // digits before "bpm" (skipping separators)
        let mut j = at;
        while j > 0 && matches!(bytes[j - 1], b' ' | b'_' | b'-' | b'.') {
            j -= 1;
        }
        let end = j;
        while j > 0 && (bytes[j - 1].is_ascii_digit() || bytes[j - 1] == b'.') {
            j -= 1;
        }
        if j < end {
            if let Ok(v) = lower[j..end].trim_matches('.').parse::<f64>() {
                if (40.0..=300.0).contains(&v) {
                    return Some(v);
                }
            }
        }
        // digits after "bpm"
        let mut k = at + 3;
        while k < bytes.len() && matches!(bytes[k], b' ' | b'_' | b'-' | b'.') {
            k += 1;
        }
        let start = k;
        while k < bytes.len() && (bytes[k].is_ascii_digit() || bytes[k] == b'.') {
            k += 1;
        }
        if k > start {
            if let Ok(v) = lower[start..k].trim_matches('.').parse::<f64>() {
                if (40.0..=300.0).contains(&v) {
                    return Some(v);
                }
            }
        }
        search = at + 3;
    }
    None
}

fn interp(env: &[f32], pos: f32) -> f32 {
    let i = pos.floor() as usize;
    let t = pos - i as f32;
    let a = env.get(i).copied().unwrap_or(0.0);
    let b = env.get(i + 1).copied().unwrap_or(0.0);
    a + (b - a) * t
}

/// Estimates tempo. Returns None for very short / non-rhythmic material.
pub fn detect_bpm(mono: &[f32], sr: u32) -> Option<f64> {
    let dur = mono.len() as f64 / sr.max(1) as f64;
    if dur < 1.0 {
        return None;
    }
    // analyse at most 40 s
    let max_n = (40.0 * sr as f64) as usize;
    let x = &mono[..mono.len().min(max_n)];
    let hop = ((sr as f32 / 86.0) as usize).max(64);
    let oe = onset_envelope(x, sr, hop);
    let env = &oe.env;
    if env.len() < 32 {
        return None;
    }
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    if mean < 1e-4 {
        return None;
    }
    let env: Vec<f32> = env.iter().map(|v| v - mean).collect();
    let fps = sr as f32 / hop as f32;

    let score_at = |bpm: f32| -> f32 {
        let lag = 60.0 / bpm * fps;
        let n = env.len() as f32 - lag - 1.0;
        if n < 8.0 {
            return 0.0;
        }
        let mut acc = 0.0;
        let mut i = 0usize;
        while (i as f32) < n {
            acc += env[i] * interp(&env, i as f32 + lag);
            i += 1;
        }
        acc / n
    };

    let mut best_bpm = 0.0f32;
    let mut best = f32::MIN;
    let mut b = 60.0f32;
    while b <= 200.0 {
        // reward periodicity at the beat and its double (half-beat pulse)
        let s = score_at(b) + 0.5 * score_at(b * 2.0).max(0.0) + 0.25 * score_at(b * 4.0).max(0.0);
        let w = (-0.5 * ((b / 120.0).log2() / 0.9).powi(2)).exp();
        let v = s * w;
        if v > best {
            best = v;
            best_bpm = b;
        }
        b += 0.25;
    }
    if best <= 0.0 {
        return None;
    }
    let mut bpm = best_bpm as f64;
    while bpm < 75.0 {
        bpm *= 2.0;
    }
    while bpm >= 175.0 {
        bpm /= 2.0;
    }

    // Loops: snap to a tempo that makes the file an exact number of beats.
    let full_dur = mono.len() as f64 / sr as f64;
    let mut snapped = None;
    for beats in [1.0, 2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0] {
        let cand = beats * 60.0 / full_dur;
        for c in [cand, cand * 2.0, cand / 2.0] {
            if (c - bpm).abs() / bpm < 0.03 {
                snapped = Some(c);
            }
        }
        if snapped.is_some() {
            break;
        }
    }
    let bpm = snapped.unwrap_or(bpm);
    let rounded = (bpm * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < 0.15 {
        Some(rounded.round())
    } else {
        Some(rounded)
    }
}

const MAJOR_PROFILE: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR_PROFILE: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

fn pearson(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let ma = a.iter().sum::<f32>() / 12.0;
    let mb = b.iter().sum::<f32>() / 12.0;
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    num / (da * db).sqrt().max(1e-9)
}

/// Chromagram via Goertzel on a downsampled signal.
pub fn chroma(mono: &[f32], sr: u32) -> Option<[f32; 12]> {
    let dec = ((sr as f32 / 11025.0).floor() as usize).max(1);
    let fs = sr as f32 / dec as f32;
    let x: Vec<f32> = mono.chunks(dec).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let frame = 4096usize;
    if x.len() < frame {
        return None;
    }
    let n_frames = x.len() / frame;
    let take = n_frames.min(48);
    let window: Vec<f32> = (0..frame).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos()).collect();
    let mut ch = [0.0f32; 12];
    let mut total = 0.0;
    for t in 0..take {
        let f = t * n_frames / take;
        let seg = &x[f * frame..(f + 1) * frame];
        for midi in 36..96u32 {
            let freq = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
            if freq > fs * 0.45 {
                continue;
            }
            let coeff = 2.0 * (std::f32::consts::TAU * freq / fs).cos();
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for (i, &v) in seg.iter().enumerate() {
                let s = v * window[i] + coeff * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
            let mag = power.max(0.0).sqrt();
            ch[(midi % 12) as usize] += mag;
            total += mag;
        }
    }
    if total < 1e-3 {
        return None;
    }
    Some(ch)
}

/// Returns e.g. "Am" or "F#" (FFT chromagram, see [`crate::music::detect_key`]).
pub fn detect_key(mono: &[f32], sr: u32) -> Option<String> {
    crate::music::detect_key(mono, sr).map(|k| k.0)
}

/// Older Goertzel-based key estimate (kept for comparison).
pub fn detect_key_goertzel(mono: &[f32], sr: u32) -> Option<String> {
    let max_n = (45.0 * sr as f64) as usize;
    let ch = chroma(&mono[..mono.len().min(max_n)], sr)?;
    let mut best = (f32::MIN, 0usize, false);
    for root in 0..12 {
        let mut maj = [0.0; 12];
        let mut min = [0.0; 12];
        for i in 0..12 {
            maj[(i + root) % 12] = MAJOR_PROFILE[i];
            min[(i + root) % 12] = MINOR_PROFILE[i];
        }
        let a = pearson(&ch, &maj);
        let b = pearson(&ch, &min);
        if a > best.0 {
            best = (a, root, false);
        }
        if b > best.0 {
            best = (b, root, true);
        }
    }
    if best.0 < 0.3 {
        return None;
    }
    Some(format!("{}{}", crate::NOTE_NAMES[best.1], if best.2 { "m" } else { "" }))
}

/// Transient positions (in samples) for auto-chopping. Always starts with 0.
/// `sensitivity` 0..1 (higher = more slices).
pub fn transients(mono: &[f32], sr: u32, sensitivity: f32, min_gap_ms: f32) -> Vec<usize> {
    let hop = 256usize;
    let oe = onset_envelope(mono, sr, hop);
    let env = &oe.env;
    let mut out = vec![0usize];
    if env.len() < 3 {
        return out;
    }
    let min_gap = ((min_gap_ms * 0.001 * sr as f32) as usize / hop).max(1);
    let global_mean = env.iter().sum::<f32>() / env.len() as f32;
    let k = 3.0 - 2.5 * sensitivity.clamp(0.0, 1.0);
    let radius = 12usize;
    let mut last = 0usize;
    for i in 1..env.len() - 1 {
        let lo = i.saturating_sub(radius);
        let hi = (i + radius).min(env.len());
        let local = env[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        let thr = local * k + global_mean * 0.5 * k;
        if env[i] > thr && env[i] >= env[i - 1] && env[i] >= env[i + 1] && i - last >= min_gap {
            let pos = (i.saturating_sub(1)) * hop;
            if pos > 0 {
                out.push(pos);
                last = i;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_track(bpm: f64, secs: f64, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f64) as usize;
        let period = 60.0 / bpm * sr as f64;
        let mut x = vec![0.0f32; n];
        let mut t = 0.0;
        let mut beat = 0;
        while (t as usize) < n {
            let s = t as usize;
            let amp = if beat % 4 == 0 { 1.0 } else { 0.6 };
            for i in 0..(0.03 * sr as f64) as usize {
                if s + i < n {
                    let env = (-(i as f32) / (0.005 * sr as f32)).exp();
                    x[s + i] += amp * env * ((i as f32) * 0.9).sin();
                }
            }
            t += period;
            beat += 1;
        }
        x
    }

    #[test]
    fn detects_click_track_tempo() {
        for bpm in [90.0, 128.0, 140.0, 100.0] {
            let x = click_track(bpm, 8.0, 22050);
            let got = detect_bpm(&x, 22050).expect("bpm");
            assert!((got - bpm).abs() < 1.5, "expected {bpm}, got {got}");
        }
    }

    #[test]
    fn parses_bpm_from_names() {
        assert_eq!(bpm_from_name("Drum Loop 140bpm.wav"), Some(140.0));
        assert_eq!(bpm_from_name("vox_92_BPM_Am"), Some(92.0));
        assert_eq!(bpm_from_name("bpm128 kit"), Some(128.0));
        assert_eq!(bpm_from_name("kick 01"), None);
    }

    #[test]
    fn finds_transients_of_clicks() {
        let x = click_track(120.0, 4.0, 22050);
        let t = transients(&x, 22050, 0.5, 60.0);
        assert!((7..=9).contains(&t.len()), "got {} slices", t.len());
    }

    #[test]
    fn detects_a_minor_chord() {
        let sr = 22050;
        let n = sr as usize * 4;
        let freqs = [220.0f32, 261.63, 329.63, 440.0];
        let x: Vec<f32> = (0..n).map(|i| freqs.iter().map(|f| (std::f32::consts::TAU * f * i as f32 / sr as f32).sin()).sum::<f32>() * 0.2).collect();
        let k = detect_key(&x, sr).unwrap();
        assert!(k == "Am" || k == "C", "got {k}");
    }
}
