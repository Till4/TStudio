//! Music visualizer → MP4. Frames are drawn on the CPU (no GPU load) and piped as raw RGB into
//! ffmpeg (libx264 + AAC). Look: dark background, a ring of mirrored spectrum bars that pulses
//! with the kick, an oscilloscope inside the ring, particles flying outward, the DJ name with a
//! kick-reactive glow, the track title and a progress bar.

#[path = "../../app/src/logo.rs"]
mod logo;

use crate::command;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::SystemTime;
use tstudio_dsp::SampleData;
use tstudio_library::{write_wav, WavFormat};

#[derive(Clone, Debug, PartialEq)]
pub struct VideoOpts {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// DJ / artist name (big, glowing). Empty = not shown.
    pub artist: String,
    /// Track title (below the name). Empty = not shown.
    pub title: String,
    pub accent: [u8; 3],
    /// x264 quality (lower = better/bigger, 18–23 sensible)
    pub crf: u32,
    /// small TStudio logo in the corner
    pub watermark: bool,
}

impl Default for VideoOpts {
    fn default() -> Self {
        Self { width: 1920, height: 1080, fps: 30, artist: String::new(), title: String::new(), accent: [255, 128, 40], crf: 20, watermark: true }
    }
}

/// Accent colour presets (name, rgb) for the UI / CLI.
pub const ACCENTS: [(&str, [u8; 3]); 6] = [
    ("Orange", [255, 128, 40]),
    ("Rot", [255, 40, 60]),
    ("Pink", [255, 50, 170]),
    ("Lila", [150, 80, 255]),
    ("Cyan", [30, 210, 255]),
    ("Grün", [60, 255, 120]),
];

/// Video size presets (name, width, height).
pub const SIZES: [(&str, u32, u32); 4] = [("1080p", 1920, 1080), ("720p", 1280, 720), ("vertical", 1080, 1920), ("square", 1080, 1080)];

pub fn size_by_name(name: &str) -> Option<(u32, u32)> {
    let n = name.trim().to_lowercase();
    let n = match n.as_str() {
        "9:16" | "hochkant" | "reel" | "tiktok" => "vertical",
        "1:1" | "quadrat" => "square",
        "16:9" | "full-hd" | "fullhd" => "1080p",
        x => x,
    };
    SIZES.iter().find(|(s, _, _)| *s == n).map(|(_, w, h)| (*w, *h))
}

pub fn accent_by_name(name: &str) -> Option<[u8; 3]> {
    let n = name.trim().to_lowercase();
    if let Some(h) = n.strip_prefix('#').filter(|h| h.len() == 6) {
        let v = u32::from_str_radix(h, 16).ok()?;
        return Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    ACCENTS.iter().find(|(a, _)| a.to_lowercase() == n || (n == "gruen" && *a == "Grün")).map(|(_, c)| *c)
}

// ---------------------------------------------------------------------------------------------
// analysis

const NB: usize = 48; // spectrum bands per side
const FFT_N: usize = 4096;

struct Fft {
    tw: Vec<(f32, f32)>,
}

impl Fft {
    fn new(n: usize) -> Self {
        Self { tw: (0..n / 2).map(|k| { let a = -2.0 * std::f64::consts::PI * k as f64 / n as f64; (a.cos() as f32, a.sin() as f32) }).collect() }
    }
    fn run(&self, re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        let mut j = 0;
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
        let mut len = 2;
        while len <= n {
            let step = n / len;
            let half = len / 2;
            for i in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = self.tw[k * step];
                    let (br, bi) = (re[i + k + half], im[i + k + half]);
                    let tr = br * wr - bi * wi;
                    let ti = br * wi + bi * wr;
                    let (ar, ai) = (re[i + k], im[i + k]);
                    re[i + k] = ar + tr;
                    im[i + k] = ai + ti;
                    re[i + k + half] = ar - tr;
                    im[i + k + half] = ai - ti;
                }
            }
            len <<= 1;
        }
    }
}

/// Per-frame spectrum (0..1 per band) and bass level (0..1), normalized over the whole track.
struct Analysis {
    bands: Vec<[f32; NB]>,
    bass: Vec<f32>,
    mono: Vec<f32>,
    sr: f32,
    peak: f32,
    frames: usize,
}

fn percentile(mut v: Vec<f32>, p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[((v.len() - 1) as f32 * p) as usize]
}

fn analyze(d: &SampleData, fps: u32) -> Analysis {
    let sr = d.sample_rate as f32;
    let mono: Vec<f32> = d.left.iter().zip(&d.right).map(|(l, r)| 0.5 * (l + r)).collect();
    let frames = ((mono.len() as f64 / sr as f64) * fps as f64).ceil().max(1.0) as usize;
    let fft = Fft::new(FFT_N);
    let win: Vec<f32> = (0..FFT_N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_N as f32).cos()).collect();
    // log-spaced band edges in bins
    let (f0, f1) = (35.0f32, 16000.0f32.min(sr * 0.45));
    let bin = |f: f32| ((f / sr * FFT_N as f32).round() as usize).clamp(1, FFT_N / 2 - 1);
    let edges: Vec<usize> = (0..=NB).map(|k| bin(f0 * (f1 / f0).powf(k as f32 / NB as f32))).collect();
    let (b0, b1) = (bin(30.0), bin(120.0));
    let mut raw = vec![[-120.0f32; NB]; frames];
    let mut bass_raw = vec![-120.0f32; frames];
    let mut re = vec![0.0f32; FFT_N];
    let mut im = vec![0.0f32; FFT_N];
    for (f, (row, bass)) in raw.iter_mut().zip(bass_raw.iter_mut()).enumerate() {
        let c = (f as f64 / fps as f64 * sr as f64) as i64 - FFT_N as i64 / 2;
        for i in 0..FFT_N {
            let s = c + i as i64;
            re[i] = if s >= 0 && (s as usize) < mono.len() { mono[s as usize] * win[i] } else { 0.0 };
            im[i] = 0.0;
        }
        fft.run(&mut re, &mut im);
        let pw = |k: usize| re[k] * re[k] + im[k] * im[k];
        for b in 0..NB {
            let (lo, hi) = (edges[b], edges[b + 1].max(edges[b] + 1));
            let s: f32 = (lo..hi).map(pw).sum();
            row[b] = 10.0 * (s + 1e-12).log10();
        }
        let s: f32 = (b0..=b1).map(pw).sum();
        *bass = 10.0 * (s + 1e-12).log10();
    }
    // normalisation: mix of per-band and global reference (p97), 36 dB visible range
    let tops: Vec<f32> = (0..NB).map(|b| percentile(raw.iter().map(|r| r[b]).filter(|v| *v > -100.0).collect(), 0.97)).collect();
    let gtop = tops.iter().cloned().fold(-120.0f32, f32::max);
    let bands = raw
        .iter()
        .map(|r| {
            let mut o = [0.0f32; NB];
            for b in 0..NB {
                let top = 0.7 * tops[b] + 0.3 * gtop;
                o[b] = ((r[b] - (top - 36.0)) / 36.0).clamp(0.0, 1.0).powf(1.5);
            }
            o
        })
        .collect();
    let btop = percentile(bass_raw.iter().cloned().filter(|v| *v > -100.0).collect(), 0.97);
    let bass = bass_raw.iter().map(|v| ((v - (btop - 18.0)) / 18.0).clamp(0.0, 1.0).powi(2)).collect();
    let peak = mono.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-4);
    Analysis { bands, bass, mono, sr, peak, frames }
}

// ---------------------------------------------------------------------------------------------
// text

struct Mask {
    w: usize,
    h: usize,
    data: Vec<f32>,
}

fn load_font(path: &str, axes: &[(&[u8; 4], f32)]) -> Option<FontVec> {
    let bytes = std::fs::read(path).ok()?;
    let mut f = FontVec::try_from_vec(bytes).ok()?;
    for (tag, v) in axes {
        f.set_variation(tag, *v);
    }
    Some(f)
}

fn fallback_font() -> FontVec {
    FontVec::try_from_vec(epaint_default_fonts::UBUNTU_LIGHT.to_vec()).expect("bundled font")
}

/// Bold condensed display font for the name, regular for the rest.
fn fonts() -> (FontVec, FontVec) {
    let dir = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts\\")).unwrap_or_else(|_| "C:\\Windows\\Fonts\\".into());
    let bold = load_font(&format!("{dir}bahnschrift.ttf"), &[(b"wght", 700.0), (b"wdth", 75.0)])
        .or_else(|| load_font(&format!("{dir}ariblk.ttf"), &[]))
        .unwrap_or_else(fallback_font);
    let regular = load_font(&format!("{dir}bahnschrift.ttf"), &[(b"wght", 350.0), (b"wdth", 100.0)])
        .or_else(|| load_font(&format!("{dir}segoeui.ttf"), &[]))
        .unwrap_or_else(fallback_font);
    (bold, regular)
}

fn text_mask(font: &FontVec, px: f32, spacing_em: f32, text: &str, pad: usize) -> Mask {
    let sf = font.as_scaled(PxScale::from(px));
    let mut x = 0.0f32;
    let mut glyphs = Vec::new();
    let mut prev = None;
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        glyphs.push(id.with_scale_and_position(px, ab_glyph::point(x, sf.ascent())));
        x += sf.h_advance(id) + spacing_em * px;
        prev = Some(id);
    }
    let width = (x - spacing_em * px).max(1.0);
    let w = width.ceil() as usize + 2 * pad + 2;
    let h = (sf.ascent() - sf.descent()).ceil() as usize + 2 * pad + 2;
    let mut data = vec![0.0f32; w * h];
    for g in glyphs {
        if let Some(og) = font.outline_glyph(g) {
            let b = og.px_bounds();
            og.draw(|gx, gy, c| {
                let xx = b.min.x as i64 + gx as i64 + pad as i64;
                let yy = b.min.y as i64 + gy as i64 + pad as i64;
                if xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h {
                    let i = yy as usize * w + xx as usize;
                    data[i] = data[i].max(c.clamp(0.0, 1.0));
                }
            });
        }
    }
    Mask { w, h, data }
}

/// Text that fits into `max_w` pixels (shrinks the size if needed).
fn fitted_text(font: &FontVec, px: f32, spacing: f32, text: &str, pad: usize, max_w: f32) -> Mask {
    let m = text_mask(font, px, spacing, text, pad);
    let inner = (m.w - 2 * pad) as f32;
    if inner <= max_w {
        m
    } else {
        text_mask(font, px * max_w / inner, spacing, text, pad)
    }
}

fn blur(m: &Mask, r: usize) -> Mask {
    let (w, h) = (m.w, m.h);
    let mut a = m.data.clone();
    let mut b = vec![0.0f32; w * h];
    let norm = 1.0 / (2 * r + 1) as f32;
    for _ in 0..3 {
        for y in 0..h {
            let row = &a[y * w..(y + 1) * w];
            let mut acc: f32 = row[..r.min(w)].iter().sum();
            for x in 0..w {
                if x + r < w {
                    acc += row[x + r];
                }
                b[y * w + x] = acc * norm;
                if x >= r {
                    acc -= row[x - r];
                }
            }
        }
        for x in 0..w {
            let mut acc: f32 = (0..r.min(h)).map(|y| b[y * w + x]).sum();
            for y in 0..h {
                if y + r < h {
                    acc += b[(y + r) * w + x];
                }
                a[y * w + x] = acc * norm;
                if y >= r {
                    acc -= b[(y - r) * w + x];
                }
            }
        }
    }
    Mask { w, h, data: a }
}

// ---------------------------------------------------------------------------------------------
// canvas

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Canvas {
    #[inline]
    fn over(&mut self, i: usize, c: [f32; 3], a: f32) {
        let a = a.clamp(0.0, 1.0);
        let p = &mut self.px[i];
        for k in 0..3 {
            p[k] += (c[k] - p[k]) * a;
        }
    }
    #[inline]
    fn add(&mut self, i: usize, c: [f32; 3], a: f32) {
        let p = &mut self.px[i];
        for k in 0..3 {
            p[k] += c[k] * a;
        }
    }

    /// Anti-aliased capsule from a to b, plus an additive soft glow.
    #[allow(clippy::too_many_arguments)]
    fn capsule(&mut self, a: (f32, f32), b: (f32, f32), r: f32, c: [f32; 3], alpha: f32, glow_r: f32, glow: f32) {
        let ext = r + glow_r + 1.0;
        let x0 = (a.0.min(b.0) - ext).floor().max(0.0) as usize;
        let x1 = ((a.0.max(b.0) + ext).ceil() as usize).min(self.w);
        let y0 = (a.1.min(b.1) - ext).floor().max(0.0) as usize;
        let y1 = ((a.1.max(b.1) + ext).ceil() as usize).min(self.h);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let ll = (dx * dx + dy * dy).max(1e-6);
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5 - a.0, y as f32 + 0.5 - a.1);
                let t = ((px * dx + py * dy) / ll).clamp(0.0, 1.0);
                let (ex, ey) = (px - dx * t, py - dy * t);
                let d = (ex * ex + ey * ey).sqrt();
                let i = y * self.w + x;
                if glow > 0.0 && d < r + glow_r {
                    let g = 1.0 - ((d - r).max(0.0) / glow_r);
                    self.add(i, c, g * g * glow);
                }
                let cov = (r + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.over(i, c, cov * alpha);
                }
            }
        }
    }

    fn dot(&mut self, cx: f32, cy: f32, r: f32, c: [f32; 3], a: f32) {
        let ext = r * 2.5 + 1.0;
        let x0 = (cx - ext).max(0.0) as usize;
        let x1 = ((cx + ext).max(0.0) as usize).min(self.w);
        let y0 = (cy - ext).max(0.0) as usize;
        let y1 = ((cy + ext).max(0.0) as usize).min(self.h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt() / (r * 2.5);
                if d < 1.0 {
                    let i = y * self.w + x;
                    self.add(i, c, (1.0 - d).powi(3) * a);
                }
            }
        }
    }

    fn mask(&mut self, m: &Mask, cx: f32, cy: f32, c: [f32; 3], a: f32, additive: bool) {
        let x0 = (cx - m.w as f32 / 2.0).round() as i64;
        let y0 = (cy - m.h as f32 / 2.0).round() as i64;
        self.mask_at(m, x0, y0, c, a, additive);
    }

    fn mask_at(&mut self, m: &Mask, x0: i64, y0: i64, c: [f32; 3], a: f32, additive: bool) {
        for my in 0..m.h {
            let y = y0 + my as i64;
            if y < 0 || y >= self.h as i64 {
                continue;
            }
            for mx in 0..m.w {
                let x = x0 + mx as i64;
                if x < 0 || x >= self.w as i64 {
                    continue;
                }
                let v = m.data[my * m.w + mx];
                if v <= 0.002 {
                    continue;
                }
                let i = y as usize * self.w + x as usize;
                if additive {
                    self.add(i, c, v * a);
                } else {
                    self.over(i, c, v * a);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// visualizer

struct Particle {
    ang: f32,
    r: f32,
    speed: f32,
    size: f32,
    white: f32,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

struct Visualizer {
    o: VideoOpts,
    an: Analysis,
    w: usize,
    h: usize,
    m: f32, // min(w, h)
    u: f32, // pixel scale (1.0 at 1080)
    cx: f32,
    cy: f32,
    r0: f32,
    bar_max: f32,
    base: Vec<[f32; 3]>,
    glow_map: Vec<f32>,
    accent: [f32; 3],
    accent_hi: [f32; 3],
    name: Option<(Mask, Mask, f32)>,
    title: Option<(Mask, f32)>,
    small_font: FontVec,
    logo: Option<(Mask, Vec<[f32; 3]>, Mask)>,
    // state
    spec: [f32; NB],
    bass: f32,
    prev_bass: f32,
    flash: f32,
    parts: Vec<Particle>,
    rng: Rng,
    duration: f32,
}

fn lin(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| v as f32 / 255.0)
}

impl Visualizer {
    fn new(d: &SampleData, o: &VideoOpts) -> Self {
        let mut o = o.clone();
        o.width = (o.width.max(16) / 2) * 2;
        o.height = (o.height.max(16) / 2) * 2;
        o.fps = o.fps.clamp(1, 120);
        let an = analyze(d, o.fps);
        let (w, h) = (o.width as usize, o.height as usize);
        let m = w.min(h) as f32;
        let u = m / 1080.0;
        let cx = w as f32 / 2.0;
        let cy = h as f32 * if h > w { 0.42 } else { 0.38 };
        let r0 = 0.14 * m;
        let bar_max = 0.13 * m;
        // background: vertical gradient + vignette
        let top = [10.0 / 255.0, 11.0 / 255.0, 15.0 / 255.0];
        let bot = [20.0 / 255.0, 21.0 / 255.0, 27.0 / 255.0];
        let diag = ((w * w + h * h) as f32).sqrt() / 2.0;
        let mut base = Vec::with_capacity(w * h);
        let mut glow_map = Vec::with_capacity(w * h);
        for y in 0..h {
            let t = y as f32 / h as f32;
            for x in 0..w {
                let dc = ((x as f32 - w as f32 / 2.0).powi(2) + (y as f32 - h as f32 / 2.0).powi(2)).sqrt() / diag;
                let vig = 1.0 - 0.55 * dc * dc;
                base.push([0, 1, 2].map(|k| (top[k] + (bot[k] - top[k]) * t) * vig));
                let dr = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / (r0 * 2.2);
                glow_map.push((-dr * dr).exp());
            }
        }
        let accent = lin(o.accent);
        let accent_hi = accent.map(|v| v + (1.0 - v) * 0.55);
        let (bold, regular) = fonts();
        let name_y = cy + r0 + bar_max + 0.14 * m;
        let glow_pad = (14.0 * u) as usize + 2;
        let name = (!o.artist.trim().is_empty()).then(|| {
            let mk = fitted_text(&bold, 0.105 * m, 0.06, o.artist.trim(), glow_pad * 3, w as f32 * 0.88);
            let g = blur(&mk, glow_pad);
            (mk, g, name_y)
        });
        let title_y = if name.is_some() { name_y + 0.10 * m } else { name_y };
        let title = (!o.title.trim().is_empty()).then(|| (fitted_text(&regular, if name.is_some() { 0.042 * m } else { 0.08 * m }, 0.12, o.title.trim(), 2, w as f32 * 0.9), title_y));
        let logo = o.watermark.then(|| {
            let s = (0.045 * m).round().max(16.0) as usize;
            let px = logo::rgba(s as u32);
            let mask = Mask { w: s, h: s, data: (0..s * s).map(|i| px[i * 4 + 3] as f32 / 255.0).collect() };
            let col = (0..s * s).map(|i| [px[i * 4] as f32 / 255.0, px[i * 4 + 1] as f32 / 255.0, px[i * 4 + 2] as f32 / 255.0]).collect();
            let label = text_mask(&regular, 0.022 * m, 0.08, "made with TStudio", 1);
            (mask, col, label)
        });
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let parts = (0..240)
            .map(|_| {
                let ang = rng.next() * std::f32::consts::TAU;
                let r = r0 + rng.next() * diag;
                Particle { ang, r, speed: 0.6 + rng.next() * 1.8, size: 0.8 + rng.next() * 1.8, white: rng.next() }
            })
            .collect();
        let duration = d.left.len() as f32 / d.sample_rate.max(1) as f32;
        Self {
            o,
            an,
            w,
            h,
            m,
            u,
            cx,
            cy,
            r0,
            bar_max,
            base,
            glow_map,
            accent,
            accent_hi,
            name,
            title,
            small_font: regular,
            logo,
            spec: [0.0; NB],
            bass: 0.0,
            prev_bass: 0.0,
            flash: 0.0,
            parts,
            rng,
            duration,
        }
    }

    fn frames(&self) -> usize {
        self.an.frames
    }

    /// Advances smoothing, kick detection and particles to frame `f` (call once per frame, in order).
    fn step(&mut self, f: usize) {
        let f = f.min(self.an.frames - 1);
        let raw = self.an.bands[f];
        for b in 0..NB {
            let k = if raw[b] > self.spec[b] { 0.75 } else { 0.22 };
            self.spec[b] += (raw[b] - self.spec[b]) * k;
        }
        let bv = self.an.bass[f];
        if bv - self.prev_bass > 0.22 {
            self.flash = 1.0;
        }
        self.prev_bass = bv;
        self.bass += (bv - self.bass) * if bv > self.bass { 0.8 } else { 0.25 };
        self.flash *= 0.82;
        let diag = ((self.w * self.w + self.h * self.h) as f32).sqrt() / 2.0;
        let boost = 0.35 + 3.2 * self.bass + 2.0 * self.flash;
        for p in &mut self.parts {
            p.r += p.speed * boost * self.u * (60.0 / self.o.fps as f32);
            if p.r > diag {
                p.ang = self.rng.next() * std::f32::consts::TAU;
                p.r = self.r0 * (1.0 + self.rng.next() * 0.3);
                p.speed = 0.6 + self.rng.next() * 1.8;
            }
        }
    }

    fn draw(&self, f: usize, out: &mut [u8]) {
        let (w, h, m, u) = (self.w, self.h, self.m, self.u);
        let t = f as f32 / self.o.fps as f32;
        let glow_k = 0.10 + 0.30 * self.bass + 0.25 * self.flash;
        let mut c = Canvas { w, h, px: Vec::with_capacity(w * h) };
        c.px.extend(self.base.iter().zip(&self.glow_map).map(|(b, g)| {
            let k = g * glow_k;
            [b[0] + self.accent[0] * k, b[1] + self.accent[1] * k, b[2] + self.accent[2] * k]
        }));
        // particles
        for p in &self.parts {
            let fade = ((p.r - self.r0) / (120.0 * u)).clamp(0.0, 1.0);
            let col = [0, 1, 2].map(|k| self.accent_hi[k] + (1.0 - self.accent_hi[k]) * p.white);
            let x = self.cx + p.ang.cos() * p.r;
            let y = self.cy + p.ang.sin() * p.r;
            c.dot(x, y, p.size * u, col, 0.55 * fade);
        }
        // ring radius pulses with the kick
        let r = self.r0 * (1.0 + 0.07 * self.bass + 0.03 * self.flash);
        // darken the disc inside the ring, then the ring itself
        {
            let ext = r + 30.0 * u;
            let x0 = (self.cx - ext).max(0.0) as usize;
            let x1 = ((self.cx + ext) as usize).min(w);
            let y0 = (self.cy - ext).max(0.0) as usize;
            let y1 = ((self.cy + ext) as usize).min(h);
            let th = 3.0 * u;
            for y in y0..y1 {
                for x in x0..x1 {
                    let d = ((x as f32 + 0.5 - self.cx).powi(2) + (y as f32 + 0.5 - self.cy).powi(2)).sqrt();
                    let i = y * w + x;
                    let inside = (r - d).clamp(0.0, 1.0);
                    if inside > 0.0 {
                        let p = &mut c.px[i];
                        for k in 0..3 {
                            p[k] *= 1.0 - 0.6 * inside;
                        }
                    }
                    let dd = (d - r).abs();
                    let g = (1.0 - dd / (24.0 * u)).max(0.0);
                    if g > 0.0 {
                        c.add(i, self.accent, g * g * (0.25 + 0.5 * self.bass));
                    }
                    let cov = (th + 0.5 - dd).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        c.over(i, self.accent_hi, cov);
                    }
                }
            }
        }
        // oscilloscope inside the ring
        {
            let n = 1024usize;
            let center = (t * self.an.sr) as i64;
            let seg = 128;
            let span = 0.8 * r;
            let amp = 0.42 * r / self.an.peak;
            let pt = |j: usize| -> (f32, f32) {
                let s = center - n as i64 / 2 + (j * n / seg) as i64;
                let v = if s >= 0 && (s as usize) < self.an.mono.len() { self.an.mono[s as usize] } else { 0.0 };
                let win = (std::f32::consts::PI * j as f32 / seg as f32).sin();
                (self.cx - span + 2.0 * span * j as f32 / seg as f32, self.cy - v * amp * win)
            };
            let mut prev = pt(0);
            for j in 1..=seg {
                let p = pt(j);
                c.capsule(prev, p, 1.4 * u, self.accent_hi, 0.9, 7.0 * u, 0.10);
                prev = p;
            }
        }
        // mirrored spectrum bars around the ring (bass at the top)
        {
            let gap = 7.0 * u;
            let circ = std::f32::consts::PI * 2.0 * r;
            let bw = (circ / (2 * NB) as f32 * 0.28).max(1.0);
            for b in 0..NB {
                let v = self.spec[b];
                let len = 3.0 * u + v * self.bar_max * (1.0 - 0.35 * b as f32 / NB as f32);
                let th = std::f32::consts::PI * (b as f32 + 0.5) / NB as f32;
                let col = [0, 1, 2].map(|k| self.accent[k] + (self.accent_hi[k] - self.accent[k]) * v);
                for side in [-1.0f32, 1.0] {
                    let (sx, sy) = (side * th.sin(), -th.cos());
                    let a = (self.cx + sx * (r + gap), self.cy + sy * (r + gap));
                    let e = (self.cx + sx * (r + gap + len), self.cy + sy * (r + gap + len));
                    c.capsule(a, e, bw, col, 1.0, 10.0 * u, 0.12 + 0.25 * v);
                }
            }
        }
        // DJ name with kick-reactive glow, title
        if let Some((mk, glow, y)) = &self.name {
            c.mask(glow, self.cx, *y, self.accent, 0.9 + 1.6 * self.bass + 1.2 * self.flash, true);
            c.mask(mk, self.cx, *y, [0.97, 0.97, 0.99], 1.0, false);
        }
        if let Some((mk, y)) = &self.title {
            c.mask(mk, self.cx, *y, [0.72, 0.73, 0.78], 1.0, false);
        }
        // progress bar + time
        {
            let y = h as f32 - 0.05 * m;
            let (x0, x1) = (w as f32 * 0.06, w as f32 * 0.94);
            let prog = (t / self.duration.max(0.01)).clamp(0.0, 1.0);
            let xp = x0 + (x1 - x0) * prog;
            c.capsule((x0, y), (x1, y), 1.5 * u, [0.25, 0.26, 0.3], 1.0, 0.0, 0.0);
            c.capsule((x0, y), (xp, y), 1.5 * u, self.accent, 1.0, 8.0 * u, 0.15);
            c.dot(xp, y, 3.0 * u, self.accent_hi, 0.9);
            let fmt = |s: f32| format!("{}:{:02}", (s as u32) / 60, (s as u32) % 60);
            let sz = 0.022 * m;
            let tl = text_mask(&self.small_font, sz, 0.05, &fmt(t), 1);
            let tr = text_mask(&self.small_font, sz, 0.05, &fmt(self.duration), 1);
            let ty = (y - 0.012 * m - tl.h as f32) as i64;
            c.mask_at(&tl, x0 as i64, ty, [0.6, 0.62, 0.68], 1.0, false);
            c.mask_at(&tr, (x1 - tr.w as f32) as i64, ty, [0.6, 0.62, 0.68], 1.0, false);
        }
        // watermark (top left)
        if let Some((mk, col, label)) = &self.logo {
            let (x0, y0) = ((0.03 * m) as usize, (0.03 * m) as usize);
            for yy in 0..mk.h {
                for xx in 0..mk.w {
                    let a = mk.data[yy * mk.w + xx] * 0.85;
                    if a > 0.0 && x0 + xx < w && y0 + yy < h {
                        c.over((y0 + yy) * w + x0 + xx, col[yy * mk.w + xx], a);
                    }
                }
            }
            let lx = (x0 + mk.w) as i64 + (0.012 * m) as i64;
            let ly = y0 as i64 + (mk.h as i64 - label.h as i64) / 2;
            c.mask_at(label, lx, ly, [0.55, 0.56, 0.62], 1.0, false);
        }
        // fade in / out
        let fade = (t / 1.0).min((self.duration - t) / 1.5).clamp(0.0, 1.0);
        for (o, p) in out.chunks_exact_mut(3).zip(&c.px) {
            for k in 0..3 {
                // light tone-mapping so stacked glows don't clip harshly
                let v = p[k] * fade;
                let v = if v > 0.8 { 0.8 + 0.2 * (1.0 - (-(v - 0.8) * 5.0).exp()) } else { v };
                o[k] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// public API

/// Renders one frame at `secs` as RGB24 (width × height × 3). For previews and tests.
pub fn render_still(d: &SampleData, o: &VideoOpts, secs: f32) -> (u32, u32, Vec<u8>) {
    let mut v = Visualizer::new(d, o);
    let target = ((secs * v.o.fps as f32) as usize).min(v.frames() - 1);
    for f in 0..=target {
        v.step(f);
    }
    let mut buf = vec![0u8; v.w * v.h * 3];
    v.draw(target, &mut buf);
    (v.w as u32, v.h as u32, buf)
}

/// Writes one frame as PNG (via ffmpeg).
pub fn export_still(ffmpeg: &Path, d: &SampleData, o: &VideoOpts, secs: f32, out: &Path) -> Result<(), String> {
    let (w, h, buf) = render_still(d, o, secs);
    let mut child = command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
        .arg(format!("{w}x{h}"))
        .args(["-i", "-", "-frames:v", "1"])
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg konnte nicht gestartet werden: {e}"))?;
    child.stdin.take().unwrap().write_all(&buf).map_err(|e| e.to_string())?;
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// Renders the visualizer for `d` and muxes it with the audio into an MP4 (H.264 + AAC 320k).
/// `progress(0..1)` returning false cancels (the partial file is removed).
pub fn export_mp4(ffmpeg: &Path, d: &SampleData, out: &Path, o: &VideoOpts, mut progress: impl FnMut(f32) -> bool) -> Result<(), String> {
    if d.left.is_empty() {
        return Err("Leeres Audio".into());
    }
    let tmp = std::env::temp_dir().join(format!("tstudio-video-{}-{}.wav", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
    write_wav(&tmp, d, WavFormat::Float32)?;
    if !progress(0.0) {
        let _ = std::fs::remove_file(&tmp);
        return Err("Abgebrochen".into());
    }
    let mut v = Visualizer::new(d, o);
    let res = (|| {
        let mut child = command(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostats", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
            .arg(format!("{}x{}", v.w, v.h))
            .args(["-r", &v.o.fps.to_string(), "-i", "-", "-i"])
            .arg(&tmp)
            .args(["-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-preset", "medium", "-crf", &v.o.crf.to_string()])
            .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "320k", "-shortest", "-movflags", "+faststart"])
            .arg(out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg konnte nicht gestartet werden: {e}"))?;
        let mut err_pipe = child.stderr.take().unwrap();
        let err_thread = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err_pipe.read_to_string(&mut s);
            s
        });
        let mut stdin = std::io::BufWriter::with_capacity(1 << 20, child.stdin.take().unwrap());
        let mut buf = vec![0u8; v.w * v.h * 3];
        let n = v.frames();
        let mut failed = None;
        for f in 0..n {
            v.step(f);
            v.draw(f, &mut buf);
            if let Err(e) = stdin.write_all(&buf) {
                failed = Some(e.to_string());
                break;
            }
            if !progress((f + 1) as f32 / n as f32) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(out);
                return Err("Abgebrochen".into());
            }
        }
        let _ = stdin.flush();
        drop(stdin);
        let status = child.wait().map_err(|e| e.to_string())?;
        let log = err_thread.join().unwrap_or_default();
        if status.success() && failed.is_none() {
            Ok(())
        } else {
            let msg = log.lines().filter(|l| !l.trim().is_empty()).last().map(str::to_string).or(failed).unwrap_or_else(|| "ffmpeg-Fehler".into());
            Err(format!("Video-Export fehlgeschlagen: {msg}"))
        }
    })();
    let _ = std::fs::remove_file(&tmp);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kick_track(secs: f32, bpm: f32) -> SampleData {
        let sr = 48000;
        let n = (secs * sr as f32) as usize;
        let beat = (60.0 / bpm * sr as f32) as usize;
        let s: Vec<f32> = (0..n)
            .map(|i| {
                let p = (i % beat) as f32 / sr as f32;
                let kick = (2.0 * std::f32::consts::PI * (50.0 + 120.0 * (-p * 30.0).exp()) * p).sin() * (-p * 9.0).exp();
                let hat = ((i * 7919 % 1000) as f32 / 500.0 - 1.0) * 0.05;
                0.8 * kick + hat
            })
            .collect();
        SampleData { left: s.clone(), right: s, sample_rate: sr }
    }

    #[test]
    fn fft_finds_a_sine() {
        let fft = Fft::new(1024);
        let mut re: Vec<f32> = (0..1024).map(|i| (2.0 * std::f32::consts::PI * 64.0 * i as f32 / 1024.0).sin()).collect();
        let mut im = vec![0.0; 1024];
        fft.run(&mut re, &mut im);
        let mags: Vec<f32> = (0..512).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect();
        let best = mags.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
        assert_eq!(best, 64);
    }

    #[test]
    fn bass_follows_the_kick() {
        let d = kick_track(4.0, 150.0);
        let an = analyze(&d, 30);
        assert_eq!(an.frames, 120);
        // frame on a kick (t = 0.4 s = beat 1) vs. just before the next kick (t = 0.78 s)
        assert!(an.bass[12] > an.bass[23] + 0.3, "{} vs {}", an.bass[12], an.bass[23]);
    }

    #[test]
    fn renders_frames_with_name_and_bars() {
        let d = kick_track(3.0, 150.0);
        let o = VideoOpts { width: 320, height: 180, artist: "DJ TEST".into(), title: "Track".into(), ..Default::default() };
        let (w, h, px) = render_still(&d, &o, 1.6);
        assert_eq!((w, h), (320, 180));
        assert_eq!(px.len(), 320 * 180 * 3);
        let lum = |x: usize, y: usize| px[(y * 320 + x) * 3..][..3].iter().map(|&v| v as u32).sum::<u32>();
        // name area (centre, lower part) has bright text pixels, corners stay dark
        let name_y = (0.38 * 180.0 + 0.14 * 180.0 + 0.13 * 180.0 + 0.14 * 180.0) as usize;
        let bright = (100..220).filter(|&x| lum(x, name_y) > 500).count();
        assert!(bright > 5, "name not drawn ({bright})");
        assert!(lum(2, 178) < 60);
        // fade-in: frame 0 is black
        let (_, _, first) = render_still(&d, &o, 0.0);
        assert!(first.iter().all(|&v| v < 3));
    }

    #[test]
    fn accent_parsing() {
        assert_eq!(accent_by_name("cyan"), Some([30, 210, 255]));
        assert_eq!(accent_by_name("#ff0080"), Some([255, 0, 128]));
        assert_eq!(accent_by_name("nope"), None);
        assert_eq!(size_by_name("Hochkant"), Some((1080, 1920)));
    }
}
