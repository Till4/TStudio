//! Musical analysis on top of the FFT: precise beat grid + downbeat, chord timeline,
//! key from an FFT chromagram and the fundamental of one-shots (kick tuning).

use crate::fft::Stft;
use crate::NOTE_NAMES;
use serde::Serialize;

const FLUX_N: usize = 1024;
const FLUX_HOP: usize = 256;

/// Onset strength (log-magnitude spectral flux) plus a low-band variant, one value per hop.
pub struct Flux {
    pub all: Vec<f32>,
    pub low: Vec<f32>,
    pub fps: f64,
    pub hop: usize,
}

pub fn spectral_flux(mono: &[f32], sr: u32) -> Flux {
    let mut st = Stft::new(FLUX_N);
    let frames = mono.len() / FLUX_HOP;
    let bin_hz = sr as f64 / FLUX_N as f64;
    let low_bins = ((150.0 / bin_hz) as usize).max(2);
    let mut prev: Vec<f64> = vec![0.0; FLUX_N / 2 + 1];
    let mut mag = Vec::new();
    let (mut all, mut low) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    for f in 0..frames {
        // frame centred on f*hop, so a flux peak lands at the onset itself
        st.mag(mono, (f * FLUX_HOP) as isize - FLUX_N as isize / 2, &mut mag);
        let (mut s, mut sl) = (0.0f64, 0.0f64);
        for (i, m) in mag.iter_mut().enumerate() {
            *m = (1.0 + 1000.0 * *m).ln();
            let d = (*m - prev[i]).max(0.0);
            s += d;
            if i < low_bins {
                sl += d;
            }
        }
        std::mem::swap(&mut prev, &mut mag);
        all.push(s as f32);
        low.push(sl as f32);
    }
    // remove the slowly varying part (local mean over ~0.4 s)
    let detrend = |v: &mut Vec<f32>| {
        let w = ((0.2 * sr as f64 / FLUX_HOP as f64) as usize).max(1);
        let mut cs = vec![0.0f64; v.len() + 1];
        for (i, x) in v.iter().enumerate() {
            cs[i + 1] = cs[i] + *x as f64;
        }
        let out: Vec<f32> = (0..v.len())
            .map(|i| {
                let a = i.saturating_sub(w);
                let b = (i + w + 1).min(v.len());
                (v[i] as f64 - (cs[b] - cs[a]) / (b - a) as f64).max(0.0) as f32
            })
            .collect();
        *v = out;
    };
    detrend(&mut all);
    detrend(&mut low);
    Flux { all, low, fps: sr as f64 / FLUX_HOP as f64, hop: FLUX_HOP }
}

fn interp(env: &[f32], pos: f64) -> f64 {
    if pos < 0.0 {
        return 0.0;
    }
    let i = pos.floor() as usize;
    let t = pos - i as f64;
    let a = env.get(i).copied().unwrap_or(0.0) as f64;
    let b = env.get(i + 1).copied().unwrap_or(0.0) as f64;
    a + (b - a) * t
}

/// Best phase (frames) and its score for a beat period (frames).
fn comb(env: &[f32], period: f64) -> (f64, f64) {
    let mut best = (0.0, f64::MIN);
    let steps = (period * 2.0).ceil() as usize;
    for s in 0..steps {
        let ph = s as f64 * 0.5;
        let mut acc = 0.0;
        let mut k = ph;
        let mut n = 0usize;
        while k < env.len() as f64 - 1.0 {
            acc += interp(env, k);
            k += period;
            n += 1;
        }
        let sc = if n > 0 { acc / n as f64 } else { 0.0 };
        if sc > best.1 {
            best = (ph, sc);
        }
    }
    best
}

#[derive(Clone, Debug, Serialize)]
pub struct BeatGrid {
    pub bpm: f64,
    /// first beat at or after 0 s
    pub first_beat_s: f64,
    /// first bar start ("the 1") at or after 0 s
    pub downbeat_s: f64,
    /// index (0-based) of the first downbeat among the beats
    pub downbeat_index: usize,
    pub beats_per_bar: u32,
    pub beat_len_s: f64,
    pub bar_len_s: f64,
    /// 0..1: how clearly the beats line up with the onsets
    pub confidence: f64,
    /// 0..1: how sure the downbeat choice is
    pub downbeat_confidence: f64,
    /// median distance (ms) of strong onsets to the grid
    pub onset_deviation_ms: f64,
    pub beats: usize,
}

impl BeatGrid {
    pub fn beat_time(&self, i: f64) -> f64 {
        self.first_beat_s + i * self.beat_len_s
    }
    /// Time of beat `b` counted from the first downbeat (b = 0 → the first "1").
    pub fn from_downbeat(&self, b: f64) -> f64 {
        self.downbeat_s + b * self.beat_len_s
    }
}

/// Precise tempo (±0.01 BPM on long material), beat phase and downbeat.
/// `hint`: known/approximate tempo (the search stays within ±4 % of it).
pub fn beat_grid(mono: &[f32], sr: u32, hint: Option<f64>, beats_per_bar: u32) -> Option<BeatGrid> {
    let max_n = (240.0 * sr as f64) as usize;
    let x = &mono[..mono.len().min(max_n)];
    if x.len() < sr as usize * 2 {
        return None;
    }
    let fl = spectral_flux(x, sr);
    let env = &fl.all;
    let coarse = match hint {
        Some(h) if h > 0.0 => h,
        _ => crate::analysis::detect_bpm(x, sr)?,
    };
    let fps = fl.fps;
    let search = |lo: f64, hi: f64, step: f64| -> (f64, f64, f64) {
        let mut best = (coarse, 0.0, f64::MIN);
        let mut b = lo;
        while b <= hi {
            let (ph, sc) = comb(env, 60.0 / b * fps);
            if sc > best.2 {
                best = (b, ph, sc);
            }
            b += step;
        }
        best
    };
    let span = if hint.is_some() { 0.04 } else { 0.03 };
    let (b1, _, _) = search(coarse * (1.0 - span), coarse * (1.0 + span), 0.05);
    let (b2, _, _) = search(b1 - 0.06, b1 + 0.06, 0.005);
    let (mut bpm, mut phase, mut score) = search(b2 - 0.006, b2 + 0.006, 0.001);
    // loops: the file is (almost) exactly N beats long → that tempo is exact
    let dur = x.len() as f64 / sr as f64;
    if dur < 90.0 {
        for beats in [4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0, 96.0, 128.0, 192.0, 256.0] {
            let cand = beats * 60.0 / dur;
            if (cand - bpm).abs() / bpm < 0.006 {
                let (ph, sc) = comb(env, 60.0 / cand * fps);
                bpm = cand;
                phase = ph;
                score = sc;
                break;
            }
        }
    }
    let period = 60.0 / bpm * fps;
    // sub-frame phase refinement (parabola through neighbours)
    {
        let at = |p: f64| {
            let mut acc = 0.0;
            let mut k = p;
            while k < env.len() as f64 - 1.0 {
                acc += interp(env, k);
                k += period;
            }
            acc
        };
        let (a, b, c) = (at(phase - 0.5), at(phase), at(phase + 0.5));
        let den = a - 2.0 * b + c;
        if den.abs() > 1e-12 {
            phase += (0.5 * (a - c) / den).clamp(-0.5, 0.5) * 0.5;
        }
        phase = phase.rem_euclid(period);
    }
    // confidence: on-beat score vs. the off-beat (half period shifted) score
    let mean = env.iter().map(|v| *v as f64).sum::<f64>() / env.len() as f64;
    let confidence = if score > 0.0 { ((score - mean) / score).clamp(0.0, 1.0) } else { 0.0 };
    let beat_len = 60.0 / bpm;
    // the log-flux of a Hann frame peaks about one hop before the actual attack
    let first = (phase / fps + FLUX_HOP as f64 / sr as f64).rem_euclid(60.0 / bpm);
    let n_beats = (((mono.len() as f64 / sr as f64) - first) / beat_len).floor().max(0.0) as usize + 1;

    // onset deviation: strong flux peaks vs nearest grid line
    let thr = {
        let mut v: Vec<f32> = env.clone();
        v.sort_by(|a, b| b.total_cmp(a));
        v.get(v.len() / 50).copied().unwrap_or(0.0)
    };
    let mut devs: Vec<f64> = Vec::new();
    for i in 1..env.len().saturating_sub(1) {
        if env[i] >= thr && env[i] > env[i - 1] && env[i] >= env[i + 1] {
            let t = i as f64 / fps;
            let k = ((t - first) / beat_len).round();
            let half = ((t - first) / (beat_len / 2.0)).round();
            let d = (t - (first + k * beat_len)).abs().min((t - (first + half * beat_len / 2.0)).abs());
            devs.push(d * 1000.0);
        }
    }
    devs.sort_by(f64::total_cmp);
    let onset_deviation_ms = devs.get(devs.len() / 2).copied().unwrap_or(0.0);

    // downbeat: harmonic change + low-band accent per beat position in the bar
    let bpb = beats_per_bar.max(1) as usize;
    let (mut dbi, dconf) = downbeat(x, sr, first, beat_len, bpb, &fl);
    // unclear downbeat in a short loop: loops are cut on the "1"
    if dconf < 0.2 && dur < 60.0 && first < beat_len * 0.25 {
        dbi = 0;
    }
    let downbeat_s = first + dbi as f64 * beat_len;
    Some(BeatGrid {
        bpm: (bpm * 1000.0).round() / 1000.0,
        first_beat_s: first,
        downbeat_s,
        downbeat_index: dbi,
        beats_per_bar: bpb as u32,
        beat_len_s: beat_len,
        bar_len_s: beat_len * bpb as f64,
        confidence,
        downbeat_confidence: dconf,
        onset_deviation_ms: (onset_deviation_ms * 10.0).round() / 10.0,
        beats: n_beats,
    })
}

fn downbeat(x: &[f32], sr: u32, first: f64, beat_len: f64, bpb: usize, fl: &Flux) -> (usize, f64) {
    if bpb < 2 {
        return (0, 1.0);
    }
    let n_beats = ((x.len() as f64 / sr as f64 - first) / beat_len) as usize;
    if n_beats < bpb * 2 {
        return (0, 0.0);
    }
    let chromas: Vec<[f64; 12]> = (0..n_beats).map(|i| segment_chroma(x, sr, first + i as f64 * beat_len, first + (i + 1) as f64 * beat_len).0).collect();
    let mut nov = vec![0.0f64; n_beats];
    for i in 1..n_beats {
        nov[i] = (0..12).map(|k| (chromas[i][k] - chromas[i - 1][k]).max(0.0)).sum();
    }
    // accent around each beat (flux peaks sit about one hop before the attack)
    let peak_at = |env: &[f32], t: f64| -> f64 {
        let c = t * fl.fps;
        (-3..=1).map(|o| interp(env, c + o as f64)).fold(0.0, f64::max)
    };
    let accents: Vec<f64> = (0..n_beats).map(|i| {
        let t = first + i as f64 * beat_len;
        peak_at(&fl.low, t) + 0.5 * peak_at(&fl.all, t)
    }).collect();
    // per bar position: mean of each feature, weighted by how significant (t-statistic like)
    // its variation across the bar is compared to its beat-to-beat noise
    let per_pos = |v: &[f64]| {
        let mut s = vec![0.0f64; bpb];
        let mut c = vec![0usize; bpb];
        for (i, x) in v.iter().enumerate() {
            s[i % bpb] += x;
            c[i % bpb] += 1;
        }
        for (a, n) in s.iter_mut().zip(&c) {
            *a /= (*n).max(1) as f64;
        }
        let mut var = 0.0;
        for (i, x) in v.iter().enumerate() {
            var += (x - s[i % bpb]).powi(2);
        }
        let sd = (var / v.len().max(1) as f64).sqrt();
        let per = (v.len() / bpb).max(1) as f64;
        let (mn, mx) = s.iter().fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
        let range = (mx - mn).max(1e-12);
        let weight = range / (sd / per.sqrt() + range * 0.02 + 1e-12);
        let scaled: Vec<f64> = s.iter().map(|x| (x - mn) / range).collect();
        (scaled, weight)
    };
    let (pn, cn) = per_pos(&nov);
    let (pa, ca) = per_pos(&accents);
    let scores: Vec<f64> = (0..bpb).map(|k| cn * pn[k] + ca * pa[k]).collect();
    let mut order: Vec<usize> = (0..bpb).collect();
    order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    let best = order[0];
    let conf = if scores[best] > 0.0 { ((scores[best] - scores[order[1]]) / scores[best]).clamp(0.0, 1.0) } else { 0.0 };
    (best, conf)
}

/// Normalised chroma of [t0, t1) seconds plus a bass chroma (40–220 Hz) and the energy.
pub fn segment_chroma(x: &[f32], sr: u32, t0: f64, t1: f64) -> ([f64; 12], [f64; 12], f64) {
    let n = 8192usize;
    let a = (t0.max(0.0) * sr as f64) as usize;
    let b = ((t1 * sr as f64) as usize).min(x.len());
    let mut ch = [0.0f64; 12];
    let mut bass = [0.0f64; 12];
    if b <= a {
        return (ch, bass, 0.0);
    }
    let mut st = Stft::new(n);
    let hop = n / 2;
    let bin_hz = sr as f64 / n as f64;
    let mut mag = Vec::new();
    let mut energy = 0.0;
    let mut start = a as isize;
    // short segments: one frame centred on the segment
    if b - a < n {
        start = ((a + b) / 2) as isize - n as isize / 2;
    }
    loop {
        st.mag(x, start, &mut mag);
        let fmax = mag.iter().cloned().fold(0.0, f64::max).max(1e-9);
        for (i, m) in mag.iter().enumerate().skip(1) {
            let f = i as f64 * bin_hz;
            if !(40.0..=2500.0).contains(&f) {
                continue;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            let pc = (midi.round() as i64).rem_euclid(12) as usize;
            let dev = (midi - midi.round()).abs();
            let w = (1.0 - dev * 2.0).max(0.0); // off-pitch bins count less
            let v = (m / fmax).sqrt() * w;
            if f >= 80.0 {
                ch[pc] += v;
            }
            if f <= 220.0 {
                bass[pc] += v;
            }
            energy += m * m;
        }
        start += hop as isize;
        if start + n as isize / 2 >= b as isize {
            break;
        }
    }
    let nrm = |c: &mut [f64; 12]| {
        let s = c.iter().cloned().fold(0.0, f64::max);
        if s > 0.0 {
            for v in c.iter_mut() {
                *v /= s;
            }
        }
    };
    nrm(&mut ch);
    nrm(&mut bass);
    (ch, bass, energy)
}

#[derive(Clone, Debug, Serialize)]
pub struct ChordSeg {
    pub start_s: f64,
    pub end_s: f64,
    /// bar number (1-based, counted from the downbeat) where the chord starts
    pub bar: f64,
    /// e.g. "Cm", "G#", "N" (no chord / silence)
    pub chord: String,
    pub root: Option<u8>,
    pub minor: bool,
    pub bass_note: Option<String>,
    pub confidence: f64,
}

pub fn chord_name(root: u8, minor: bool) -> String {
    format!("{}{}", NOTE_NAMES[(root % 12) as usize], if minor { "m" } else { "" })
}

/// Best triad for a chroma (+ bass chroma). Returns (root, minor, score).
pub fn match_chord(ch: &[f64; 12], bass: &[f64; 12]) -> (u8, bool, f64) {
    let mean = ch.iter().sum::<f64>() / 12.0;
    let mut best = (0u8, false, f64::MIN);
    for root in 0..12usize {
        for minor in [false, true] {
            let third = if minor { 3 } else { 4 };
            let tones = [root, (root + third) % 12, (root + 7) % 12];
            let mut s = 0.0;
            for k in 0..12 {
                let t = if tones.contains(&k) { 1.0 } else { -0.35 };
                s += t * (ch[k] - mean);
            }
            s += 0.35 * bass[root];
            if s > best.2 {
                best = (root as u8, minor, s);
            }
        }
    }
    best
}

/// Chord per segment of `beats_per_seg` beats, starting at the downbeat; equal neighbours merged.
pub fn chord_timeline(mono: &[f32], sr: u32, grid: &BeatGrid, beats_per_seg: f64) -> Vec<ChordSeg> {
    let seg = grid.beat_len_s * beats_per_seg.max(0.25);
    let dur = mono.len() as f64 / sr as f64;
    // start at the first downbeat, minus whole segments that still fit before it
    let mut t = grid.downbeat_s;
    while t - seg >= -1e-6 {
        t -= seg;
    }
    let mut raw: Vec<ChordSeg> = Vec::new();
    let mut energies = Vec::new();
    while t + seg * 0.5 < dur {
        let (ch, bass, e) = segment_chroma(mono, sr, t, t + seg);
        let (root, minor, score) = match_chord(&ch, &bass);
        let bass_pc = (0..12).max_by(|a, b| bass[*a].total_cmp(&bass[*b])).unwrap_or(0);
        energies.push(e);
        raw.push(ChordSeg {
            start_s: t.max(0.0),
            end_s: (t + seg).min(dur),
            bar: ((t - grid.downbeat_s) / grid.bar_len_s * 100.0).round() / 100.0 + 1.0,
            chord: chord_name(root, minor),
            root: Some(root),
            minor,
            bass_note: if bass.iter().sum::<f64>() > 0.0 { Some(NOTE_NAMES[bass_pc].to_string()) } else { None },
            confidence: (score / 2.0).clamp(0.0, 1.0),
        });
        t += seg;
    }
    // quiet segments → "N"
    let mut sorted = energies.clone();
    sorted.sort_by(f64::total_cmp);
    let loud = sorted.get(sorted.len() * 9 / 10).copied().unwrap_or(0.0);
    for (s, e) in raw.iter_mut().zip(&energies) {
        if *e < loud * 1e-3 || s.confidence < 0.05 {
            s.chord = "N".into();
            s.root = None;
        }
    }
    let mut out: Vec<ChordSeg> = Vec::new();
    for s in raw {
        if let Some(last) = out.last_mut() {
            if last.chord == s.chord {
                last.end_s = s.end_s;
                last.confidence = last.confidence.max(s.confidence);
                continue;
            }
        }
        out.push(s);
    }
    out
}

// Albrecht & Shanahan (2013) key profiles – more reliable on pop/electronic material than
// Krumhansl's (tested on the user's samples: 4/4 correct vs 3/4)
const MAJOR_PROFILE: [f64; 12] = [0.238, 0.006, 0.111, 0.006, 0.137, 0.094, 0.016, 0.214, 0.009, 0.080, 0.008, 0.081];
const MINOR_PROFILE: [f64; 12] = [0.220, 0.006, 0.104, 0.123, 0.019, 0.103, 0.012, 0.214, 0.062, 0.022, 0.061, 0.052];

fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let ma = a.iter().sum::<f64>() / 12.0;
    let mb = b.iter().sum::<f64>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    num / (da * db).sqrt().max(1e-12)
}

/// Key ("Cm", "F#") from the FFT chromagram of up to 90 s; None if no tonal content.
pub fn detect_key(mono: &[f32], sr: u32) -> Option<(String, f64)> {
    let max_n = (90.0 * sr as f64) as usize;
    let x = &mono[..mono.len().min(max_n)];
    if x.len() < 8192 {
        return None;
    }
    let (c, _bass, e) = segment_chroma(x, sr, 0.0, x.len() as f64 / sr as f64);
    if e <= 1e-9 {
        return None;
    }
    let mut best = (f64::MIN, 0usize, false);
    for root in 0..12 {
        let mut maj = [0.0; 12];
        let mut min = [0.0; 12];
        for i in 0..12 {
            maj[(i + root) % 12] = MAJOR_PROFILE[i];
            min[(i + root) % 12] = MINOR_PROFILE[i];
        }
        let a = pearson(&c, &maj);
        let b = pearson(&c, &min);
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
    Some((chord_name(best.1 as u8, best.2), best.0))
}

#[derive(Clone, Debug, Serialize)]
pub struct Fundamental {
    pub hz: f64,
    pub midi: f64,
    pub note: String,
    /// deviation from the nearest note in cents
    pub cents: f64,
}

/// Body pitch of a one-shot (kick, 808, bass hit): spectrum peak 25–250 Hz of the part after the transient.
pub fn fundamental(mono: &[f32], sr: u32) -> Option<Fundamental> {
    if mono.is_empty() {
        return None;
    }
    let peak = (0..mono.len()).max_by(|a, b| mono[*a].abs().total_cmp(&mono[*b].abs()))?;
    let a = (peak + sr as usize / 50).min(mono.len());
    let b = (a + sr as usize / 4).min(mono.len());
    let seg = if b > a + sr as usize / 40 { &mono[a..b] } else { &mono[peak..mono.len().min(peak + sr as usize / 4)] };
    if seg.len() < 64 {
        return None;
    }
    let n = 65536usize.max(seg.len().next_power_of_two());
    let mut re = vec![0.0f64; n];
    let mut im = vec![0.0f64; n];
    let w = crate::fft::hann(seg.len());
    for (i, v) in seg.iter().enumerate() {
        re[i] = *v as f64 * w[i];
    }
    crate::fft::fft(&mut re, &mut im, false);
    let bin_hz = sr as f64 / n as f64;
    let (lo, hi) = ((25.0 / bin_hz) as usize, (250.0 / bin_hz) as usize);
    let mag = |i: usize| re[i].hypot(im[i]);
    let k = (lo..hi).max_by(|a, b| mag(*a).total_cmp(&mag(*b)))?;
    if mag(k) < 1e-6 {
        return None;
    }
    let (y0, y1, y2) = (mag(k - 1), mag(k), mag(k + 1));
    let den = y0 - 2.0 * y1 + y2;
    let off = if den.abs() > 1e-12 { 0.5 * (y0 - y2) / den } else { 0.0 };
    let hz = (k as f64 + off) * bin_hz;
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let near = midi.round();
    Some(Fundamental {
        hz: (hz * 10.0).round() / 10.0,
        midi: (midi * 100.0).round() / 100.0,
        note: crate::note_name(near.clamp(0.0, 127.0) as u8),
        cents: ((midi - near) * 100.0).round(),
    })
}

/// Pitch class of a note name ("C", "F#", "Bb", "es", "H"); None if unknown.
pub fn pitch_class(s: &str) -> Option<u8> {
    let s = s.trim();
    let mut ch = s.chars();
    let first = ch.next()?.to_ascii_uppercase();
    let base: i32 = match first {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        'H' => 11,
        _ => return None,
    };
    let rest: String = ch.collect::<String>().to_lowercase();
    let acc = match rest.as_str() {
        "" => 0,
        "#" | "is" | "sharp" => 1,
        "b" | "es" | "s" | "flat" => -1,
        _ => return None,
    };
    Some((base + acc).rem_euclid(12) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_track(bpm: f64, offset: f64, secs: f64, sr: u32) -> Vec<f32> {
        let mut v = vec![0.0f32; (secs * sr as f64) as usize];
        let beat = 60.0 / bpm;
        let mut t = offset;
        let mut i = 0;
        while t < secs - 0.1 {
            let s = (t * sr as f64) as usize;
            let accent = if i % 4 == 0 { 1.0 } else { 0.5 };
            for k in 0..(sr as usize / 20) {
                let env = (-(k as f32) / (sr as f32 * 0.01)).exp();
                if s + k < v.len() {
                    v[s + k] += accent * env * (2.0 * std::f32::consts::PI * 60.0 * k as f32 / sr as f32).sin() + accent * 0.3 * env * if k % 7 < 3 { 1.0 } else { -1.0 };
                }
            }
            t += beat;
            i += 1;
        }
        v
    }

    #[test]
    fn grid_finds_odd_tempo_and_phase() {
        for (sr, bpm, off) in [(44100, 181.43, 0.137), (48000, 128.0, 0.31)] {
            let x = click_track(bpm, off, 40.0, sr);
            let g = beat_grid(&x, sr, Some(bpm.round()), 4).unwrap();
            assert!((g.bpm - bpm).abs() < 0.03, "bpm {}", g.bpm);
            assert!((g.first_beat_s - off).abs() < 0.004, "phase {} vs {off}", g.first_beat_s);
            // accented clicks every 4 beats starting with the first → downbeat = first beat
            assert!((g.downbeat_s - off).abs() < 0.004, "downbeat {}", g.downbeat_s);
        }
    }

    #[test]
    fn chord_and_key_from_triads() {
        let sr = 22050;
        // C minor triad (C4 Eb4 G4) with a C2 bass
        let f = [261.63f32, 311.13, 392.0, 65.41];
        let x: Vec<f32> = (0..sr * 4).map(|i| f.iter().map(|hz| (2.0 * std::f32::consts::PI * hz * i as f32 / sr as f32).sin() * 0.2).sum()).collect();
        let (ch, bass, _) = segment_chroma(&x, sr, 0.0, 4.0);
        let (root, minor, _) = match_chord(&ch, &bass);
        assert_eq!((root, minor), (0, true));
        assert_eq!(detect_key(&x, sr).unwrap().0, "Cm");
    }

    #[test]
    fn kick_fundamental() {
        let sr = 48000;
        // kick: pitch sweep 150 → 49 Hz (G1 ≈ 49 Hz)
        let mut ph = 0.0f32;
        let x: Vec<f32> = (0..sr / 2)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let f = 49.0 + 100.0 * (-t / 0.015).exp();
                ph += 2.0 * std::f32::consts::PI * f / sr as f32;
                ph.sin() * (-t / 0.25).exp()
            })
            .collect();
        let f = fundamental(&x, sr).unwrap();
        assert!((f.hz - 49.0).abs() < 1.5, "{}", f.hz);
        assert_eq!(f.note, "G1");
    }

    #[test]
    fn pitch_classes() {
        assert_eq!(pitch_class("C#"), Some(1));
        assert_eq!(pitch_class("Bb"), Some(10));
        assert_eq!(pitch_class("es"), Some(3));
        assert_eq!(pitch_class("H"), Some(11));
    }
}
