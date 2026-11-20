//! Effect processors: EQ, compressor, reverb, delay, distortion, chorus, filter, limiter, sidechain.

use super::*;

pub enum Effect {
    Eq(EqFx),
    Compressor(CompFx),
    Reverb(Box<ReverbFx>),
    Delay(DelayFx),
    Distortion(DistFx),
    Chorus(ChorusFx),
    Filter(FilterFx),
    Limiter(LimiterFx),
    Sidechain(SidechainFx),
    Autotune(Box<AutotuneFx>),
    Clipper(ClipperFx),
    Multiband(MultibandFx),
    Ott(OttFx),
    Width(WidthFx),
}

impl Effect {
    pub fn new(kind: EffectKind, sr: f32) -> Effect {
        match kind {
            EffectKind::Eq => Effect::Eq(EqFx::default()),
            EffectKind::Compressor => Effect::Compressor(CompFx::default()),
            EffectKind::Reverb => Effect::Reverb(Box::new(ReverbFx::new(sr))),
            EffectKind::Delay => Effect::Delay(DelayFx::new(sr)),
            EffectKind::Distortion => Effect::Distortion(DistFx::default()),
            EffectKind::Chorus => Effect::Chorus(ChorusFx::new(sr)),
            EffectKind::Filter => Effect::Filter(FilterFx::default()),
            EffectKind::Limiter => Effect::Limiter(LimiterFx::default()),
            EffectKind::Sidechain => Effect::Sidechain(SidechainFx::default()),
            EffectKind::Autotune => Effect::Autotune(Box::new(AutotuneFx::new(sr))),
            EffectKind::Clipper => Effect::Clipper(ClipperFx::default()),
            EffectKind::Multiband => Effect::Multiband(MultibandFx::default()),
            EffectKind::Ott => Effect::Ott(OttFx::default()),
            EffectKind::Width => Effect::Width(WidthFx::default()),
        }
    }

    pub fn kind(&self) -> EffectKind {
        match self {
            Effect::Eq(_) => EffectKind::Eq,
            Effect::Compressor(_) => EffectKind::Compressor,
            Effect::Reverb(_) => EffectKind::Reverb,
            Effect::Delay(_) => EffectKind::Delay,
            Effect::Distortion(_) => EffectKind::Distortion,
            Effect::Chorus(_) => EffectKind::Chorus,
            Effect::Filter(_) => EffectKind::Filter,
            Effect::Limiter(_) => EffectKind::Limiter,
            Effect::Sidechain(_) => EffectKind::Sidechain,
            Effect::Autotune(_) => EffectKind::Autotune,
            Effect::Clipper(_) => EffectKind::Clipper,
            Effect::Multiband(_) => EffectKind::Multiband,
            Effect::Ott(_) => EffectKind::Ott,
            Effect::Width(_) => EffectKind::Width,
        }
    }

    /// Processes in place. Mismatched params/processor pairs are ignored.
    pub fn process(&mut self, params: &EffectParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        match (self, params) {
            (Effect::Eq(fx), EffectParams::Eq(p)) => fx.process(p, ctx, l, r),
            (Effect::Compressor(fx), EffectParams::Compressor(p)) => fx.process(p, ctx, l, r),
            (Effect::Reverb(fx), EffectParams::Reverb(p)) => fx.process(p, ctx, l, r),
            (Effect::Delay(fx), EffectParams::Delay(p)) => fx.process(p, ctx, l, r),
            (Effect::Distortion(fx), EffectParams::Distortion(p)) => fx.process(p, ctx, l, r),
            (Effect::Chorus(fx), EffectParams::Chorus(p)) => fx.process(p, ctx, l, r),
            (Effect::Filter(fx), EffectParams::Filter(p)) => fx.process(p, ctx, l, r),
            (Effect::Limiter(fx), EffectParams::Limiter(p)) => fx.process(p, ctx, l, r),
            (Effect::Sidechain(fx), EffectParams::Sidechain(p)) => fx.process(p, ctx, l, r),
            (Effect::Autotune(fx), EffectParams::Autotune(p)) => fx.process(p, l, r),
            (Effect::Clipper(fx), EffectParams::Clipper(p)) => fx.process(p, ctx, l, r),
            (Effect::Multiband(fx), EffectParams::Multiband(p)) => fx.process(p, ctx, l, r),
            (Effect::Ott(fx), EffectParams::Ott(p)) => fx.process(p, ctx, l, r),
            (Effect::Width(fx), EffectParams::Width(p)) => fx.process(p, ctx, l, r),
            _ => {}
        }
    }
}

#[inline]
pub(super) fn mix(dry: f32, wet: f32, m: f32) -> f32 {
    dry + (wet - dry) * m
}

#[derive(Default)]
pub struct EqFx {
    pub(super) bands: [[Biquad; 3]; 2],
    pub(super) last: Option<(EqParams, u32)>,
}

impl EqFx {
    pub fn process(&mut self, p: &EqParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let sr_key = ctx.sr as u32;
        if self.last.as_ref().map(|(lp, s)| lp != p || *s != sr_key).unwrap_or(true) {
            for ch in self.bands.iter_mut() {
                ch[0].low_shelf(p.low_freq, p.low_gain, ctx.sr);
                ch[1].peaking(p.mid_freq, p.mid_q, p.mid_gain, ctx.sr);
                ch[2].high_shelf(p.high_freq, p.high_gain, ctx.sr);
            }
            self.last = Some((p.clone(), sr_key));
        }
        if p.low_gain == 0.0 && p.mid_gain == 0.0 && p.high_gain == 0.0 {
            return;
        }
        for (buf, bands) in [l, r].into_iter().zip(self.bands.iter_mut()) {
            for x in buf.iter_mut() {
                let a = bands[0].process(*x);
                let b = bands[1].process(a);
                *x = bands[2].process(b);
            }
        }
    }
}

#[derive(Default)]
pub struct CompFx {
    pub(super) gr_db: f32,
    /// current gain reduction in dB (for metering)
    pub last_gr: f32,
}

impl CompFx {
    pub fn process(&mut self, p: &CompParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let att = (-1.0 / (p.attack_ms.max(0.05) * 0.001 * ctx.sr)).exp();
        let rel = (-1.0 / (p.release_ms.max(1.0) * 0.001 * ctx.sr)).exp();
        let makeup = db_to_gain(p.makeup);
        let ratio = p.ratio.max(1.0);
        let knee = p.knee.max(0.0);
        for i in 0..l.len() {
            let x = l[i].abs().max(r[i].abs());
            let xdb = gain_to_db(x);
            let over = xdb - p.threshold;
            let target_gr = if knee > 0.0 && over.abs() <= knee * 0.5 {
                let o = over + knee * 0.5;
                (1.0 - 1.0 / ratio) * o * o / (2.0 * knee)
            } else if over > 0.0 {
                over * (1.0 - 1.0 / ratio)
            } else {
                0.0
            };
            let coef = if target_gr > self.gr_db { att } else { rel };
            self.gr_db = target_gr + (self.gr_db - target_gr) * coef;
            let g = db_to_gain(-self.gr_db) * makeup;
            l[i] = mix(l[i], l[i] * g, p.mix);
            r[i] = mix(r[i], r[i] * g, p.mix);
        }
        self.last_gr = self.gr_db;
    }
}

pub(super) struct Comb {
    pub(super) buf: Vec<f32>,
    pub(super) idx: usize,
    pub(super) store: f32,
}

impl Comb {
    pub(super) fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }
    #[inline]
    pub(super) fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = input + self.store * feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

pub(super) struct Allpass {
    pub(super) buf: Vec<f32>,
    pub(super) idx: usize,
}

impl Allpass {
    pub(super) fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    #[inline]
    pub(super) fn process(&mut self, input: f32) -> f32 {
        let b = self.buf[self.idx];
        let out = b - input;
        self.buf[self.idx] = input + b * 0.5;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

/// Freeverb-style stereo reverb with pre-delay.
pub struct ReverbFx {
    pub(super) combs: [Vec<Comb>; 2],
    pub(super) aps: [Vec<Allpass>; 2],
    pub(super) pre: [Vec<f32>; 2],
    pub(super) pre_idx: usize,
    pub(super) sr: f32,
}

pub(super) const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
pub(super) const AP_TUNING: [usize; 4] = [556, 441, 341, 225];
pub(super) const STEREO_SPREAD: usize = 23;

impl ReverbFx {
    pub fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let mk_combs = |spread: usize| COMB_TUNING.iter().map(|t| Comb::new(((t + spread) as f32 * scale) as usize)).collect();
        let mk_aps = |spread: usize| AP_TUNING.iter().map(|t| Allpass::new(((t + spread) as f32 * scale) as usize)).collect();
        let pre_len = (0.25 * sr) as usize + 1;
        Self {
            combs: [mk_combs(0), mk_combs(STEREO_SPREAD)],
            aps: [mk_aps(0), mk_aps(STEREO_SPREAD)],
            pre: [vec![0.0; pre_len], vec![0.0; pre_len]],
            pre_idx: 0,
            sr,
        }
    }

    /// Clears all delay lines (no allocation).
    pub fn reset(&mut self) {
        for c in self.combs.iter_mut().flatten() {
            c.buf.iter_mut().for_each(|v| *v = 0.0);
            c.store = 0.0;
        }
        for a in self.aps.iter_mut().flatten() {
            a.buf.iter_mut().for_each(|v| *v = 0.0);
        }
        for p in self.pre.iter_mut() {
            p.iter_mut().for_each(|v| *v = 0.0);
        }
    }

    pub fn process(&mut self, p: &ReverbParams, _ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let feedback = p.size.clamp(0.0, 1.0) * 0.28 + 0.7;
        let damp = p.damping.clamp(0.0, 1.0) * 0.4;
        let width = p.width.clamp(0.0, 1.0);
        let wet1 = width * 0.5 + 0.5;
        let wet2 = (1.0 - width) * 0.5;
        let pre_len = self.pre[0].len();
        let pre_d = ((p.predelay_ms.max(0.0) * 0.001 * self.sr) as usize).min(pre_len - 1);
        for i in 0..l.len() {
            self.pre[0][self.pre_idx] = l[i];
            self.pre[1][self.pre_idx] = r[i];
            let ri = (self.pre_idx + pre_len - pre_d) % pre_len;
            let input = (self.pre[0][ri] + self.pre[1][ri]) * 0.015;
            self.pre_idx = (self.pre_idx + 1) % pre_len;

            let mut out = [0.0f32; 2];
            for ch in 0..2 {
                let mut acc = 0.0;
                for c in self.combs[ch].iter_mut() {
                    acc += c.process(input, feedback, damp);
                }
                for a in self.aps[ch].iter_mut() {
                    acc = a.process(acc);
                }
                out[ch] = acc;
            }
            let wl = out[0] * wet1 + out[1] * wet2;
            let wr = out[1] * wet1 + out[0] * wet2;
            l[i] = l[i] * (1.0 - p.mix * 0.5) + wl * 3.0 * p.mix;
            r[i] = r[i] * (1.0 - p.mix * 0.5) + wr * 3.0 * p.mix;
        }
    }
}

pub struct DelayFx {
    pub(super) buf: [Vec<f32>; 2],
    pub(super) idx: usize,
    pub(super) lp: [OnePole; 2],
    pub(super) cur: f32,
}

impl DelayFx {
    pub fn new(sr: f32) -> Self {
        let n = (4.0 * sr) as usize + 4;
        Self { buf: [vec![0.0; n], vec![0.0; n]], idx: 0, lp: [OnePole::default(); 2], cur: -1.0 }
    }

    #[inline]
    pub(super) fn read(buf: &[f32], idx: usize, delay: f32) -> f32 {
        let n = buf.len();
        let d = delay.clamp(1.0, (n - 2) as f32);
        let pos = idx as f32 + n as f32 - d;
        let i0 = pos.floor() as usize;
        let t = pos - pos.floor();
        let a = buf[i0 % n];
        let b = buf[(i0 + 1) % n];
        a + (b - a) * t
    }

    pub fn reset(&mut self) {
        for b in self.buf.iter_mut() {
            b.iter_mut().for_each(|v| *v = 0.0);
        }
        self.lp = [OnePole::default(); 2];
    }

    pub fn process(&mut self, p: &DelayParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let target = p.time_beats.max(0.01) * 60.0 / ctx.bpm.max(1.0) * ctx.sr;
        if self.cur < 0.0 {
            self.cur = target;
        }
        for lp in self.lp.iter_mut() {
            lp.set(p.tone, ctx.sr);
        }
        let fb = p.feedback.clamp(0.0, 0.98);
        let n = self.buf[0].len();
        for i in 0..l.len() {
            self.cur += (target - self.cur) * 0.0005;
            let dl = Self::read(&self.buf[0], self.idx, self.cur);
            let dr = Self::read(&self.buf[1], self.idx, self.cur);
            let fl = self.lp[0].process(dl);
            let fr = self.lp[1].process(dr);
            let (wl, wr) = if p.ping_pong {
                (0.5 * (l[i] + r[i]) + fr * fb, fl * fb)
            } else {
                (l[i] + fl * fb, r[i] + fr * fb)
            };
            self.buf[0][self.idx] = wl;
            self.buf[1][self.idx] = wr;
            self.idx = (self.idx + 1) % n;
            l[i] += dl * p.mix;
            r[i] += dr * p.mix;
        }
    }
}

#[derive(Default)]
pub struct DistFx {
    pub(super) tone: [OnePole; 2],
    pub(super) hold: [f32; 2],
    pub(super) hold_ctr: f32,
}

impl DistFx {
    pub fn process(&mut self, p: &DistParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let drive = db_to_gain(p.drive_db);
        let out_g = db_to_gain(p.output_db);
        for t in self.tone.iter_mut() {
            t.set(p.tone, ctx.sr);
        }
        let levels = 2f32.powf(p.bits.clamp(1.0, 16.0));
        let ds = p.downsample.max(1.0);
        for i in 0..l.len() {
            let mut crush_tick = false;
            if p.mode == DistMode::Crush {
                self.hold_ctr += 1.0;
                if self.hold_ctr >= ds {
                    self.hold_ctr -= ds;
                    crush_tick = true;
                }
            }
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                let x = buf[i];
                let y = match p.mode {
                    DistMode::Soft => (x * drive).tanh(),
                    DistMode::Hard => (x * drive).clamp(-1.0, 1.0),
                    DistMode::Fold => (x * drive * std::f32::consts::FRAC_PI_2).sin(),
                    DistMode::Crush => {
                        if crush_tick {
                            self.hold[ch] = ((x * drive).clamp(-1.0, 1.0) * levels).round() / levels;
                        }
                        self.hold[ch]
                    }
                };
                let y = self.tone[ch].process(y) * out_g;
                buf[i] = mix(x, y, p.mix);
            }
        }
    }
}

pub struct ChorusFx {
    pub(super) buf: [Vec<f32>; 2],
    pub(super) idx: usize,
    pub(super) phase: f32,
    pub(super) sr: f32,
}

impl ChorusFx {
    pub fn new(sr: f32) -> Self {
        let n = (0.06 * sr) as usize + 4;
        Self { buf: [vec![0.0; n], vec![0.0; n]], idx: 0, phase: 0.0, sr }
    }

    pub fn reset(&mut self) {
        for b in self.buf.iter_mut() {
            b.iter_mut().for_each(|v| *v = 0.0);
        }
    }

    pub fn process(&mut self, p: &ChorusParams, _ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let n = self.buf[0].len();
        let inc = p.rate / self.sr;
        for i in 0..l.len() {
            self.buf[0][self.idx] = l[i];
            self.buf[1][self.idx] = r[i];
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                let ph = self.phase + ch as f32 * 0.25;
                let lfo = (ph * std::f32::consts::TAU).sin();
                let d_ms = (p.delay_ms + p.depth_ms * lfo).clamp(0.5, 55.0);
                let d = d_ms * 0.001 * self.sr;
                let wet = DelayFx::read(&self.buf[ch], self.idx, d);
                buf[i] = mix(buf[i], wet, p.mix * 0.5) * (1.0 + p.mix * 0.3);
            }
            self.idx = (self.idx + 1) % n;
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

#[derive(Default)]
pub struct FilterFx {
    pub(super) svf: [Svf; 2],
    pub(super) cur_cutoff: f32,
}

impl FilterFx {
    pub(super) fn process(&mut self, p: &FilterFxParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        if self.cur_cutoff <= 0.0 {
            self.cur_cutoff = p.cutoff;
        }
        let mut i = 0;
        while i < l.len() {
            // smooth cutoff in log domain every 32 samples to avoid zipper noise
            self.cur_cutoff *= (p.cutoff.max(16.0) / self.cur_cutoff).powf(0.2);
            for s in self.svf.iter_mut() {
                s.set(self.cur_cutoff, p.resonance, ctx.sr);
            }
            let end = (i + 32).min(l.len());
            for k in i..end {
                let fl = self.svf[0].process(l[k], p.mode);
                let fr = self.svf[1].process(r[k], p.mode);
                l[k] = mix(l[k], fl, p.mix);
                r[k] = mix(r[k], fr, p.mix);
            }
            i = end;
        }
    }
}

pub struct LimiterFx {
    pub(super) gain: f32,
    pub last_gr_db: f32,
}

impl Default for LimiterFx {
    fn default() -> Self {
        Self { gain: 1.0, last_gr_db: 0.0 }
    }
}

impl LimiterFx {
    pub(super) fn process(&mut self, p: &LimiterParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        let pre = db_to_gain(p.gain_db);
        let ceil = db_to_gain(p.ceiling_db.min(0.0));
        let rel = (-1.0 / (p.release_ms.max(1.0) * 0.001 * ctx.sr)).exp();
        for i in 0..l.len() {
            let a = l[i] * pre;
            let b = r[i] * pre;
            let peak = a.abs().max(b.abs());
            let target = if peak > ceil { ceil / peak } else { 1.0 };
            if target < self.gain {
                self.gain = target;
            } else {
                self.gain = target + (self.gain - target) * rel;
            }
            l[i] = (a * self.gain).clamp(-ceil, ceil);
            r[i] = (b * self.gain).clamp(-ceil, ceil);
        }
        self.last_gr_db = -gain_to_db(self.gain);
    }
}

#[derive(Default)]
pub struct SidechainFx {
    pub(super) env: f32,
    /// current gain reduction 0..1 (for metering)
    pub last_duck: f32,
}

impl SidechainFx {
    pub(super) fn process(&mut self, p: &SidechainParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
        if ctx.key.is_empty() {
            return;
        }
        let att = (-1.0 / (p.attack_ms.max(0.05) * 0.001 * ctx.sr)).exp();
        let rel = (-1.0 / (p.release_ms.max(1.0) * 0.001 * ctx.sr)).exp();
        let thr = db_to_gain(p.threshold_db.min(0.0)).max(1e-4);
        let depth = p.depth.clamp(0.0, 1.0);
        for i in 0..l.len() {
            let k = ctx.key.get(i).copied().unwrap_or(0.0);
            let c = if k > self.env { att } else { rel };
            self.env = k + (self.env - k) * c;
            let duck = depth * (self.env / thr).min(1.0);
            let g = 1.0 - duck;
            l[i] *= g;
            r[i] *= g;
            self.last_duck = duck;
        }
    }
}
