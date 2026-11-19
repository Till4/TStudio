//! Small radix-2 FFT for offline analysis (beat grid, chords, loudness bands, formants).

use std::f64::consts::PI;

/// In-place complex FFT. `re.len()` must be a power of two. `inverse` scales by 1/n.
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n);
    if n < 2 {
        return;
    }
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * 2.0 * PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = i + k;
                let b = a + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
            i += len;
        }
        len <<= 1;
    }
    if inverse {
        let s = 1.0 / n as f64;
        for v in re.iter_mut().chain(im.iter_mut()) {
            *v *= s;
        }
    }
}

pub fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()).collect()
}

/// Reusable magnitude-spectrum helper for frames of a fixed size.
pub struct Stft {
    pub n: usize,
    win: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
}

impl Stft {
    pub fn new(n: usize) -> Self {
        Self { n, win: hann(n), re: vec![0.0; n], im: vec![0.0; n] }
    }

    /// Magnitudes (n/2+1 bins) of the windowed frame starting at `start` (zero-padded at the ends).
    pub fn mag(&mut self, x: &[f32], start: isize, out: &mut Vec<f64>) {
        for i in 0..self.n {
            let k = start + i as isize;
            let v = if k >= 0 && (k as usize) < x.len() { x[k as usize] as f64 } else { 0.0 };
            self.re[i] = v * self.win[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im, false);
        out.clear();
        out.extend((0..=self.n / 2).map(|i| (self.re[i] * self.re[i] + self.im[i] * self.im[i]).sqrt()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_peak() {
        let n = 1024;
        let mut re: Vec<f64> = (0..n).map(|i| (2.0 * PI * 37.0 * i as f64 / n as f64).sin()).collect();
        let orig = re.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        let peak = (0..n / 2).max_by(|a, b| (re[*a].hypot(im[*a])).total_cmp(&re[*b].hypot(im[*b]))).unwrap();
        assert_eq!(peak, 37);
        fft(&mut re, &mut im, true);
        assert!(re.iter().zip(&orig).all(|(a, b)| (a - b).abs() < 1e-9));
    }
}
