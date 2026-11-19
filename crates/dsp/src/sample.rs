/// Decoded, de-interleaved stereo audio. Mono files are duplicated into both channels.
#[derive(Clone, Debug, Default)]
pub struct SampleData {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub sample_rate: u32,
}

#[inline]
pub fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c = (x1 - xm1) * 0.5;
    let v = x0 - x1;
    let w = c + v;
    let a = w + v + (x2 - x0) * 0.5;
    let b = w + a;
    ((a * t - b) * t + c) * t + x0
}

/// Cubic-interpolated read at a fractional position (zero outside the buffer).
#[inline]
pub fn read_cubic(buf: &[f32], pos: f64) -> f32 {
    let n = buf.len();
    if n == 0 || pos < -1.0 || pos >= n as f64 {
        return 0.0;
    }
    let i = pos.floor() as isize;
    let t = (pos - i as f64) as f32;
    let get = |k: isize| -> f32 {
        if k < 0 || k as usize >= n {
            0.0
        } else {
            buf[k as usize]
        }
    };
    hermite(get(i - 1), get(i), get(i + 1), get(i + 2), t)
}

impl SampleData {
    pub fn new(left: Vec<f32>, right: Vec<f32>, sample_rate: u32) -> Self {
        Self { left, right, sample_rate }
    }

    pub fn silent(frames: usize, sample_rate: u32) -> Self {
        Self { left: vec![0.0; frames], right: vec![0.0; frames], sample_rate }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.len() as f64 / self.sample_rate as f64
        }
    }

    pub fn mono(&self) -> Vec<f32> {
        self.left.iter().zip(&self.right).map(|(l, r)| 0.5 * (l + r)).collect()
    }

    /// Change playback speed by `speed` (2.0 = twice as fast, pitch goes up). Keeps sample rate.
    pub fn resample_speed(&self, speed: f64) -> SampleData {
        let speed = speed.max(1e-4);
        let out_len = ((self.len() as f64) / speed).floor() as usize;
        let mut l = Vec::with_capacity(out_len);
        let mut r = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let p = i as f64 * speed;
            l.push(read_cubic(&self.left, p));
            r.push(read_cubic(&self.right, p));
        }
        SampleData::new(l, r, self.sample_rate)
    }

    /// Sample-rate conversion (cubic; adequate for one-time conversion at load).
    pub fn resample_to(&self, sr: u32) -> SampleData {
        if sr == self.sample_rate || self.sample_rate == 0 {
            let mut c = self.clone();
            c.sample_rate = sr;
            return c;
        }
        let ratio = self.sample_rate as f64 / sr as f64;
        let mut out = self.resample_speed(ratio);
        out.sample_rate = sr;
        out
    }

    pub fn slice(&self, start: usize, end: usize) -> SampleData {
        let e = end.min(self.len());
        let s = start.min(e);
        SampleData::new(self.left[s..e].to_vec(), self.right[s..e].to_vec(), self.sample_rate)
    }

    pub fn peak(&self) -> f32 {
        self.left.iter().chain(&self.right).fold(0.0f32, |m, v| m.max(v.abs()))
    }

    pub fn normalize(&mut self, target: f32) {
        let p = self.peak();
        if p > 1e-6 {
            let g = target / p;
            self.left.iter_mut().chain(self.right.iter_mut()).for_each(|v| *v *= g);
        }
    }

    pub fn fade(&mut self, fade_in: usize, fade_out: usize) {
        let n = self.len();
        let fi = fade_in.min(n);
        for i in 0..fi {
            let g = i as f32 / fi as f32;
            self.left[i] *= g;
            self.right[i] *= g;
        }
        let fo = fade_out.min(n);
        for i in 0..fo {
            let g = i as f32 / fo as f32;
            self.left[n - 1 - i] *= g;
            self.right[n - 1 - i] *= g;
        }
    }

    /// Min/max pairs for drawing a waveform, `buckets` long.
    pub fn peaks(&self, buckets: usize) -> Vec<(f32, f32)> {
        peaks_of(&self.left, &self.right, 0, self.len(), buckets)
    }
}

/// Min/max of the mono mix in `buckets` slices of `[start, end)`.
pub fn peaks_of(l: &[f32], r: &[f32], start: usize, end: usize, buckets: usize) -> Vec<(f32, f32)> {
    let end = end.min(l.len());
    let start = start.min(end);
    let n = end - start;
    let mut out = Vec::with_capacity(buckets);
    if buckets == 0 || n == 0 {
        return vec![(0.0, 0.0); buckets];
    }
    for b in 0..buckets {
        let s = start + b * n / buckets;
        let e = (start + (b + 1) * n / buckets).max(s + 1).min(end);
        let mut mn = 0.0f32;
        let mut mx = 0.0f32;
        // stride keeps very large buckets cheap
        let step = ((e - s) / 256).max(1);
        let mut i = s;
        while i < e {
            let v = 0.5 * (l[i] + r[i]);
            mn = mn.min(v);
            mx = mx.max(v);
            i += step;
        }
        out.push((mn, mx));
    }
    out
}
