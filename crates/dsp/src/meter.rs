//! Mix metering: EBU R128 loudness (LUFS), true peak, frequency balance, stereo/mono-bass
//! checks, masking between tracks and comparison against a reference track.

use crate::fft::Stft;
use crate::SampleData;
use serde::Serialize;

// ------------------------------------------------------------------ loudness

#[derive(Clone, Copy)]
struct Bq {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Bq {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// BS.1770 K-weighting filter pair for any sample rate.
fn k_weighting(sr: f64) -> [Bq; 2] {
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (std::f64::consts::PI * f0 / sr).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Bq {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (std::f64::consts::PI * f0 / sr).tan();
    let a0 = 1.0 + k / q + k * k;
    let hp = Bq { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], z: [0.0; 2] };
    [shelf, hp]
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Loudness {
    /// integrated loudness (LUFS), gated
    pub integrated_lufs: f64,
    /// loudest 3 s window (LUFS)
    pub short_term_max_lufs: f64,
    /// loudest 400 ms window (LUFS)
    pub momentary_max_lufs: f64,
    /// loudness range (LU)
    pub lra: f64,
    pub true_peak_dbtp: f64,
    pub sample_peak_db: f64,
    /// peak-to-loudness ratio (dB): small = heavily limited
    pub plr: f64,
}

fn lufs_of(ms: f64) -> f64 {
    if ms <= 0.0 {
        -120.0
    } else {
        -0.691 + 10.0 * ms.log10()
    }
}

/// Mean-square of the K-weighted signal per 100 ms step (sum of both channels).
fn kw_power_steps(d: &SampleData) -> (Vec<f64>, usize) {
    let sr = d.sample_rate as f64;
    let step = (sr * 0.1).round() as usize;
    let mut fl = k_weighting(sr);
    let mut fr = k_weighting(sr);
    let mut out = Vec::with_capacity(d.len() / step.max(1) + 1);
    let mut acc = 0.0;
    let mut n = 0;
    for i in 0..d.len() {
        let l0 = fl[0].run(d.left[i] as f64);
        let l = fl[1].run(l0);
        let r0 = fr[0].run(d.right[i] as f64);
        let r = fr[1].run(r0);
        acc += l * l + r * r;
        n += 1;
        if n == step {
            out.push(acc / step as f64);
            acc = 0.0;
            n = 0;
        }
    }
    (out, step)
}

fn windows(steps: &[f64], len: usize) -> Vec<f64> {
    if steps.len() < len {
        return if steps.is_empty() { vec![] } else { vec![steps.iter().sum::<f64>() / steps.len() as f64] };
    }
    let mut out = Vec::with_capacity(steps.len() - len + 1);
    let mut s: f64 = steps[..len].iter().sum();
    out.push(s / len as f64);
    for i in len..steps.len() {
        s += steps[i] - steps[i - len];
        out.push(s / len as f64);
    }
    out
}

/// 4× oversampled peak (dBTP), windowed-sinc interpolation.
pub fn true_peak(d: &SampleData) -> f64 {
    const TAPS: usize = 12;
    const OS: usize = 4;
    let mut coef = [[0.0f32; TAPS]; OS];
    for (p, row) in coef.iter_mut().enumerate() {
        let frac = p as f64 / OS as f64;
        for (t, c) in row.iter_mut().enumerate() {
            let x = t as f64 - (TAPS / 2 - 1) as f64 - frac;
            let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
            let w = 0.5 + 0.5 * (std::f64::consts::PI * x / (TAPS as f64 / 2.0 + 1.0)).cos();
            *c = (sinc * w) as f32;
        }
    }
    let mut peak = 0.0f32;
    for ch in [&d.left, &d.right] {
        for &v in ch.iter() {
            peak = peak.max(v.abs());
        }
        if ch.len() < TAPS {
            continue;
        }
        for i in 0..ch.len() - TAPS {
            let w = &ch[i..i + TAPS];
            // quick skip: quiet neighbourhoods cannot overshoot the current peak
            let m = w[TAPS / 2 - 1].abs().max(w[TAPS / 2].abs());
            if m < peak * 0.7 {
                continue;
            }
            for row in coef.iter().skip(1) {
                let mut s = 0.0f32;
                for t in 0..TAPS {
                    s += w[t] * row[t];
                }
                peak = peak.max(s.abs());
            }
        }
    }
    20.0 * (peak.max(1e-9) as f64).log10()
}

pub fn loudness(d: &SampleData) -> Loudness {
    let (steps, _) = kw_power_steps(d);
    let mom = windows(&steps, 4);
    let short = windows(&steps, 30);
    // integrated: absolute gate -70 LUFS, then relative gate -10 LU
    let abs_gated: Vec<f64> = mom.iter().copied().filter(|m| lufs_of(*m) > -70.0).collect();
    let integrated = if abs_gated.is_empty() {
        -120.0
    } else {
        let mean = abs_gated.iter().sum::<f64>() / abs_gated.len() as f64;
        let rel = lufs_of(mean) - 10.0;
        let g: Vec<f64> = abs_gated.into_iter().filter(|m| lufs_of(*m) > rel).collect();
        lufs_of(g.iter().sum::<f64>() / g.len().max(1) as f64)
    };
    // LRA from short-term values (abs -70, rel -20), 10th..95th percentile
    let mut st: Vec<f64> = short.iter().map(|m| lufs_of(*m)).filter(|l| *l > -70.0).collect();
    let lra = if st.len() > 2 {
        let mean = lufs_of(short.iter().copied().filter(|m| lufs_of(*m) > -70.0).sum::<f64>() / st.len() as f64);
        st.retain(|l| *l > mean - 20.0);
        st.sort_by(f64::total_cmp);
        let p = |q: f64| st[((st.len() - 1) as f64 * q).round() as usize];
        if st.len() > 2 {
            p(0.95) - p(0.10)
        } else {
            0.0
        }
    } else {
        0.0
    };
    let sp = 20.0 * (d.peak().max(1e-9) as f64).log10();
    let tp = true_peak(d);
    let r1 = |v: f64| (v * 10.0).round() / 10.0;
    Loudness {
        integrated_lufs: r1(integrated),
        short_term_max_lufs: r1(short.iter().map(|m| lufs_of(*m)).fold(-120.0, f64::max)),
        momentary_max_lufs: r1(mom.iter().map(|m| lufs_of(*m)).fold(-120.0, f64::max)),
        lra: r1(lra),
        true_peak_dbtp: r1(tp),
        sample_peak_db: r1(sp),
        plr: r1(tp - integrated),
    }
}

// ------------------------------------------------------------------ frequency balance

/// (name, lo Hz, hi Hz, typical share in dB for loud electronic dance music)
pub const BANDS: [(&str, f64, f64, f64); 8] = [
    ("Sub 20–60", 20.0, 60.0, -7.5),
    ("Bass 60–120", 60.0, 120.0, -5.5),
    ("Low-Mid 120–250", 120.0, 250.0, -9.0),
    ("Mulm 250–500", 250.0, 500.0, -12.5),
    ("Mitten 500–2k", 500.0, 2000.0, -12.0),
    ("Präsenz 2k–5k", 2000.0, 5000.0, -16.0),
    ("Höhen 5k–10k", 5000.0, 10000.0, -19.5),
    ("Air 10k–20k", 10000.0, 20000.0, -25.0),
];

#[derive(Clone, Debug, Serialize)]
pub struct Band {
    pub name: &'static str,
    pub lo_hz: f64,
    pub hi_hz: f64,
    /// share of the total energy in dB (0 = everything in this band)
    pub share_db: f64,
}

/// Average power spectrum (n/2+1 bins) of up to `max_frames` frames spread over the signal.
pub fn avg_power(mono: &[f32], n: usize, max_frames: usize) -> Vec<f64> {
    let mut st = Stft::new(n);
    let mut acc = vec![0.0f64; n / 2 + 1];
    if mono.is_empty() {
        return acc;
    }
    let frames = (mono.len() / (n / 2)).clamp(1, max_frames);
    let hop = if frames > 1 { mono.len().saturating_sub(n) / (frames - 1) } else { 0 };
    let mut mag = Vec::new();
    for f in 0..frames {
        st.mag(mono, (f * hop) as isize, &mut mag);
        for (a, m) in acc.iter_mut().zip(&mag) {
            *a += m * m;
        }
    }
    acc
}

pub fn bands_of_power(p: &[f64], sr: u32, n: usize) -> Vec<Band> {
    let bin_hz = sr as f64 / n as f64;
    let total: f64 = p.iter().enumerate().filter(|(i, _)| (*i as f64 * bin_hz) >= 20.0).map(|(_, v)| *v).sum::<f64>().max(1e-30);
    BANDS
        .iter()
        .map(|(name, lo, hi, _)| {
            let s: f64 = p.iter().enumerate().filter(|(i, _)| {
                let f = *i as f64 * bin_hz;
                f >= *lo && f < *hi
            }).map(|(_, v)| *v).sum();
            Band { name, lo_hz: *lo, hi_hz: *hi, share_db: ((10.0 * (s / total).max(1e-12).log10()) * 10.0).round() / 10.0 }
        })
        .collect()
}

pub fn bands(d: &SampleData) -> Vec<Band> {
    let n = 4096;
    bands_of_power(&avg_power(&d.mono(), n, 400), d.sample_rate, n)
}

// ------------------------------------------------------------------ stereo

#[derive(Clone, Debug, Serialize)]
pub struct Stereo {
    /// correlation of the whole signal (1 = mono, 0 = wide, <0 = phase problems)
    pub correlation: f64,
    /// correlation below 120 Hz (should be close to 1)
    pub bass_correlation: f64,
    /// side energy relative to mid (dB); -inf = mono
    pub width_db: f64,
    pub bass_width_db: f64,
}

pub fn stereo(d: &SampleData) -> Stereo {
    let sr = d.sample_rate as f32;
    let mut lp = [crate::filter::OnePole::default(); 4];
    for f in lp.iter_mut() {
        f.set(120.0, sr);
    }
    let (mut lr, mut ll, mut rr, mut m2, mut s2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut blr, mut bll, mut brr, mut bm2, mut bs2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for i in 0..d.len() {
        let (l, r) = (d.left[i] as f64, d.right[i] as f64);
        lr += l * r;
        ll += l * l;
        rr += r * r;
        m2 += (l + r).powi(2);
        s2 += (l - r).powi(2);
        let bl0 = lp[0].process(d.left[i]);
        let bl = lp[1].process(bl0) as f64;
        let br0 = lp[2].process(d.right[i]);
        let br = lp[3].process(br0) as f64;
        blr += bl * br;
        bll += bl * bl;
        brr += br * br;
        bm2 += (bl + br).powi(2);
        bs2 += (bl - br).powi(2);
    }
    let corr = |a: f64, b: f64, c: f64| if b > 0.0 && c > 0.0 { (a / (b * c).sqrt() * 100.0).round() / 100.0 } else { 1.0 };
    let w = |s: f64, m: f64| if s <= 1e-12 { -99.0 } else { ((10.0 * (s / m.max(1e-12)).log10()) * 10.0).round() / 10.0 };
    Stereo { correlation: corr(lr, ll, rr), bass_correlation: corr(blr, bll, brr), width_db: w(s2, m2), bass_width_db: w(bs2, bm2) }
}

// ------------------------------------------------------------------ report

#[derive(Clone, Debug, Serialize)]
pub struct MixReport {
    pub loudness: Loudness,
    pub bands: Vec<Band>,
    pub stereo: Stereo,
    pub clipped_samples: usize,
    pub warnings: Vec<String>,
}

pub fn report(d: &SampleData, master: bool) -> MixReport {
    let loud = loudness(d);
    let b = bands(d);
    let st = stereo(d);
    let clipped = d.left.iter().chain(&d.right).filter(|x| x.abs() >= 0.999).count();
    let mut w = Vec::new();
    if loud.integrated_lufs <= -100.0 {
        w.push("Stille – nichts zu analysieren".into());
        return MixReport { loudness: loud, bands: b, stereo: st, clipped_samples: clipped, warnings: w };
    }
    if master {
        if loud.true_peak_dbtp > -0.1 {
            w.push(format!("True Peak {:+.1} dBTP – Limiter-Ceiling auf -1 dB stellen, sonst verzerrt es nach MP3/Streaming", loud.true_peak_dbtp));
        }
        if clipped > 0 {
            w.push(format!("{clipped} Samples auf 0 dBFS (Clipping) – Master-Limiter prüfen"));
        }
        if loud.integrated_lufs < -12.0 {
            w.push(format!("{:.1} LUFS ist leise für Hardtekk/Schranz (Releases liegen bei etwa -8 bis -5 LUFS) – Limiter-Gain hoch", loud.integrated_lufs));
        } else if loud.integrated_lufs > -4.5 {
            w.push(format!("{:.1} LUFS ist extrem laut – Pumpen/Verzerrung wahrscheinlich, Limiter-Gain runter", loud.integrated_lufs));
        }
        if loud.plr < 5.0 && loud.integrated_lufs > -20.0 {
            w.push(format!("PLR {:.1} dB – sehr stark komprimiert/limitiert, die Kick verliert Punch", loud.plr));
        }
        for (band, (_, _, _, typical)) in b.iter().zip(BANDS.iter()) {
            let diff = band.share_db - typical;
            if diff > 5.0 {
                w.push(format!("{}: {:+.1} dB über dem Richtwert – zu viel (EQ absenken)", band.name, diff));
            } else if diff < -6.0 {
                w.push(format!("{}: {:+.1} dB unter dem Richtwert – zu wenig", band.name, diff));
            }
        }
    }
    if st.bass_correlation < 0.85 && b[0].share_db.max(b[1].share_db) > -25.0 {
        w.push(format!("Bass unter 120 Hz nicht mono (Korrelation {:.2}) – im Club löscht sich das aus", st.bass_correlation));
    }
    if st.correlation < 0.0 {
        w.push(format!("Stereo-Korrelation {:.2} – Phasenprobleme, in Mono wird es dünn", st.correlation));
    }
    MixReport { loudness: loud, bands: b, stereo: st, clipped_samples: clipped, warnings: w }
}

// ------------------------------------------------------------------ masking

/// Per-frame energy (dB) of a frequency band.
fn band_env(mono: &[f32], sr: u32, lo: f64, hi: f64) -> Vec<f64> {
    let n = 2048;
    let hop = 1024;
    let mut st = Stft::new(n);
    let bin_hz = sr as f64 / n as f64;
    let (a, b) = ((lo / bin_hz).floor() as usize, ((hi / bin_hz).ceil() as usize).min(n / 2));
    let mut mag = Vec::new();
    let frames = mono.len() / hop;
    (0..frames)
        .map(|f| {
            st.mag(mono, (f * hop) as isize, &mut mag);
            let e: f64 = mag[a..b.max(a + 1)].iter().map(|m| m * m).sum();
            10.0 * e.max(1e-20).log10()
        })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct Overlap {
    pub band: &'static str,
    /// fraction of the time both tracks are loud in this band at once (0..1)
    pub simultaneous: f64,
}

/// How much two tracks fight over the same frequency range at the same time.
pub fn overlap(a: &SampleData, b: &SampleData) -> Vec<Overlap> {
    let (ma, mb) = (a.mono(), b.mono());
    let sr = a.sample_rate;
    BANDS[..5]
        .iter()
        .filter_map(|(name, lo, hi, _)| {
            let ea = band_env(&ma, sr, *lo, *hi);
            let eb = band_env(&mb, sr, *lo, *hi);
            let n = ea.len().min(eb.len());
            if n == 0 {
                return None;
            }
            let maxa = ea.iter().cloned().fold(-200.0, f64::max);
            let maxb = eb.iter().cloned().fold(-200.0, f64::max);
            if maxa < -40.0 || maxb < -40.0 {
                return None;
            }
            let (mut both, mut any) = (0usize, 0usize);
            for i in 0..n {
                let (x, y) = (ea[i] > maxa - 9.0, eb[i] > maxb - 9.0);
                if x || y {
                    any += 1;
                }
                if x && y {
                    both += 1;
                }
            }
            Some(Overlap { band: name, simultaneous: if any > 0 { (both as f64 / any as f64 * 100.0).round() / 100.0 } else { 0.0 } })
        })
        .collect()
}

// ------------------------------------------------------------------ reference comparison

#[derive(Clone, Debug, Serialize)]
pub struct BandDiff {
    pub name: &'static str,
    pub mine_db: f64,
    pub reference_db: f64,
    /// mine - reference (dB); positive = more than the reference
    pub diff_db: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    pub lufs_mine: f64,
    pub lufs_reference: f64,
    pub plr_mine: f64,
    pub plr_reference: f64,
    pub width_mine_db: f64,
    pub width_reference_db: f64,
    pub bands: Vec<BandDiff>,
    pub suggestions: Vec<String>,
}

pub fn compare(mine: &SampleData, reference: &SampleData) -> Comparison {
    let (lm, lr) = (loudness(mine), loudness(reference));
    let (bm, br) = (bands(mine), bands(reference));
    let (sm, sr) = (stereo(mine), stereo(reference));
    let mut sug = Vec::new();
    let d = lm.integrated_lufs - lr.integrated_lufs;
    if d.abs() > 1.5 {
        sug.push(format!("Lautheit {:+.1} LU gegenüber der Referenz – Master-Limiter-Gain um {:+.1} dB ändern", d, -d));
    }
    let bands: Vec<BandDiff> = bm
        .iter()
        .zip(&br)
        .map(|(a, b)| BandDiff { name: a.name, mine_db: a.share_db, reference_db: b.share_db, diff_db: ((a.share_db - b.share_db) * 10.0).round() / 10.0 })
        .collect();
    for b in &bands {
        if b.diff_db > 3.0 {
            sug.push(format!("{}: {:+.1} dB mehr als die Referenz – absenken (EQ oder Spurlautstärke)", b.name, b.diff_db));
        } else if b.diff_db < -3.0 {
            sug.push(format!("{}: {:+.1} dB weniger als die Referenz – anheben", b.name, b.diff_db));
        }
    }
    let p = lm.plr - lr.plr;
    if p > 3.0 {
        sug.push(format!("PLR {:+.1} dB höher – Referenz ist dichter, mehr Kompression/Clipping auf dem Master", p));
    } else if p < -3.0 {
        sug.push(format!("PLR {:+.1} dB niedriger – du limitierst stärker als die Referenz, Kick verliert Punch", p));
    }
    if sm.width_db - sr.width_db > 4.0 {
        sug.push("deutlich breiter als die Referenz – Stereo-Effekte/Chorus zurücknehmen".into());
    } else if sm.width_db - sr.width_db < -4.0 {
        sug.push("deutlich schmaler als die Referenz – Pads/Leads breiter machen (Chorus, Reverb)".into());
    }
    Comparison {
        lufs_mine: lm.integrated_lufs,
        lufs_reference: lr.integrated_lufs,
        plr_mine: lm.plr,
        plr_reference: lr.plr,
        width_mine_db: sm.width_db,
        width_reference_db: sr.width_db,
        bands,
        suggestions: sug,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, amp: f32, secs: f32, sr: u32) -> SampleData {
        let v: Vec<f32> = (0..(secs * sr as f32) as usize).map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32).sin() * amp).collect();
        SampleData::new(v.clone(), v, sr)
    }

    #[test]
    fn lufs_of_reference_sine() {
        // EBU: 1 kHz sine at -18 dBFS (stereo) ≈ -18 LUFS... -3.01 for dual channel summing → -15? Spec: 0 dBFS 1 kHz in one channel = -3.01 LUFS
        let d = sine(997.0, 1.0, 5.0, 48000);
        let l = loudness(&d);
        // both channels full-scale → +3 dB vs. one channel → ≈ 0 LUFS
        assert!((l.integrated_lufs - 0.0).abs() < 0.3, "{}", l.integrated_lufs);
        assert!((l.true_peak_dbtp).abs() < 0.3);
    }

    #[test]
    fn true_peak_sees_intersample_overs() {
        // fs/4 sine with 45° phase: samples at 0.707, true peak 1.0
        let sr = 48000;
        let v: Vec<f32> = (0..4800).map(|i| (std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4).sin()).collect();
        let d = SampleData::new(v.clone(), v, sr);
        assert!(d.peak() < 0.75);
        assert!(true_peak(&d) > -0.6, "{}", true_peak(&d));
    }

    #[test]
    fn bands_put_energy_where_it_is() {
        let d = sine(80.0, 0.5, 2.0, 48000);
        let b = bands(&d);
        assert!(b[1].share_db > -1.0, "bass {:?}", b[1]);
        assert!(b[5].share_db < -40.0);
    }

    #[test]
    fn stereo_detects_wide_bass() {
        let sr = 48000;
        let l: Vec<f32> = (0..sr).map(|i| (2.0 * std::f32::consts::PI * 60.0 * i as f32 / sr as f32).sin()).collect();
        let r: Vec<f32> = l.iter().map(|x| -x).collect();
        let s = stereo(&SampleData::new(l, r, sr));
        assert!(s.bass_correlation < -0.9);
        let rep = report(&SampleData::new(vec![0.0; 10], vec![0.0; 10], sr), true);
        assert!(!rep.warnings.is_empty());
    }

    #[test]
    fn overlap_finds_shared_bass() {
        let a = sine(70.0, 0.5, 2.0, 48000);
        let b = sine(90.0, 0.5, 2.0, 48000);
        let o = overlap(&a, &b);
        assert!(o.iter().any(|x| x.band.starts_with("Bass") && x.simultaneous > 0.9));
    }
}
