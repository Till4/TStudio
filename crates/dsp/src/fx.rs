//! Built-in mixer effects. Processors are allocated up front (`Effect::new`) and
//! never allocate in `process`.

use crate::autotune::{AutotuneFx, AutotuneParams};
pub use crate::fx_extra::{ClipperFx, ClipperParams, MultibandFx, MultibandParams, OttFx, OttParams};
use crate::filter::{Biquad, FilterMode, OnePole, Svf};
use crate::{db_to_gain, gain_to_db};
use serde::{Deserialize, Serialize};

pub struct FxContext<'a> {
    pub sr: f32,
    pub bpm: f32,
    /// Sidechain key (mono, absolute level) for this block; empty = no key.
    pub key: &'a [f32],
}

/// Generates `FIELDS` (names of automatable f32 fields) plus indexed accessors.
macro_rules! numeric_fields {
    ($t:ty { $($f:ident),* $(,)? }) => {
        impl $t {
            pub const FIELDS: &'static [&'static str] = &[$(stringify!($f)),*];
            #[allow(unused_assignments)]
            pub fn field_mut(&mut self, i: usize) -> Option<&mut f32> {
                let mut k = 0usize;
                $( if i == k { return Some(&mut self.$f); } k += 1; )*
                None
            }
            #[allow(unused_assignments)]
            pub fn field(&self, i: usize) -> Option<f32> {
                let mut k = 0usize;
                $( if i == k { return Some(self.$f); } k += 1; )*
                None
            }
        }
    };
}
pub(crate) use numeric_fields;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectKind {
    Eq,
    Compressor,
    Reverb,
    Delay,
    Distortion,
    Chorus,
    Filter,
    Limiter,
    Sidechain,
    Autotune,
    Clipper,
    Multiband,
    Ott,
}

impl EffectKind {
    pub const ALL: [EffectKind; 13] = [
        EffectKind::Eq,
        EffectKind::Compressor,
        EffectKind::Reverb,
        EffectKind::Delay,
        EffectKind::Distortion,
        EffectKind::Chorus,
        EffectKind::Filter,
        EffectKind::Limiter,
        EffectKind::Sidechain,
        EffectKind::Autotune,
        EffectKind::Clipper,
        EffectKind::Multiband,
        EffectKind::Ott,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Eq => "EQ",
            EffectKind::Compressor => "Compressor",
            EffectKind::Reverb => "Reverb",
            EffectKind::Delay => "Delay",
            EffectKind::Distortion => "Distortion",
            EffectKind::Chorus => "Chorus",
            EffectKind::Filter => "Filter",
            EffectKind::Limiter => "Limiter",
            EffectKind::Sidechain => "Sidechain",
            EffectKind::Autotune => "Autotune",
            EffectKind::Clipper => "Clipper",
            EffectKind::Multiband => "Multiband",
            EffectKind::Ott => "OTT",
        }
    }

    pub fn default_params(self) -> EffectParams {
        match self {
            EffectKind::Eq => EffectParams::Eq(EqParams::default()),
            EffectKind::Compressor => EffectParams::Compressor(CompParams::default()),
            EffectKind::Reverb => EffectParams::Reverb(ReverbParams::default()),
            EffectKind::Delay => EffectParams::Delay(DelayParams::default()),
            EffectKind::Distortion => EffectParams::Distortion(DistParams::default()),
            EffectKind::Chorus => EffectParams::Chorus(ChorusParams::default()),
            EffectKind::Filter => EffectParams::Filter(FilterFxParams::default()),
            EffectKind::Limiter => EffectParams::Limiter(LimiterParams::default()),
            EffectKind::Sidechain => EffectParams::Sidechain(SidechainParams::default()),
            EffectKind::Autotune => EffectParams::Autotune(AutotuneParams::default()),
            EffectKind::Clipper => EffectParams::Clipper(ClipperParams::default()),
            EffectKind::Multiband => EffectParams::Multiband(MultibandParams::default()),
            EffectKind::Ott => EffectParams::Ott(OttParams::default()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum EffectParams {
    Eq(EqParams),
    Compressor(CompParams),
    Reverb(ReverbParams),
    Delay(DelayParams),
    Distortion(DistParams),
    Chorus(ChorusParams),
    Filter(FilterFxParams),
    Limiter(LimiterParams),
    Sidechain(SidechainParams),
    Autotune(AutotuneParams),
    Clipper(ClipperParams),
    Multiband(MultibandParams),
    Ott(OttParams),
}

impl EffectParams {
    pub fn kind(&self) -> EffectKind {
        match self {
            EffectParams::Eq(_) => EffectKind::Eq,
            EffectParams::Compressor(_) => EffectKind::Compressor,
            EffectParams::Reverb(_) => EffectKind::Reverb,
            EffectParams::Delay(_) => EffectKind::Delay,
            EffectParams::Distortion(_) => EffectKind::Distortion,
            EffectParams::Chorus(_) => EffectKind::Chorus,
            EffectParams::Filter(_) => EffectKind::Filter,
            EffectParams::Limiter(_) => EffectKind::Limiter,
            EffectParams::Sidechain(_) => EffectKind::Sidechain,
            EffectParams::Autotune(_) => EffectKind::Autotune,
            EffectParams::Clipper(_) => EffectKind::Clipper,
            EffectParams::Multiband(_) => EffectKind::Multiband,
            EffectParams::Ott(_) => EffectKind::Ott,
        }
    }

    /// Names of the automatable (f32) parameters of this effect.
    pub fn field_names(&self) -> &'static [&'static str] {
        match self {
            EffectParams::Eq(_) => EqParams::FIELDS,
            EffectParams::Compressor(_) => CompParams::FIELDS,
            EffectParams::Reverb(_) => ReverbParams::FIELDS,
            EffectParams::Delay(_) => DelayParams::FIELDS,
            EffectParams::Distortion(_) => DistParams::FIELDS,
            EffectParams::Chorus(_) => ChorusParams::FIELDS,
            EffectParams::Filter(_) => FilterFxParams::FIELDS,
            EffectParams::Limiter(_) => LimiterParams::FIELDS,
            EffectParams::Sidechain(_) => SidechainParams::FIELDS,
            EffectParams::Autotune(_) => AutotuneParams::FIELDS,
            EffectParams::Clipper(_) => ClipperParams::FIELDS,
            EffectParams::Multiband(_) => MultibandParams::FIELDS,
            EffectParams::Ott(_) => OttParams::FIELDS,
        }
    }

    pub fn field_mut(&mut self, i: usize) -> Option<&mut f32> {
        match self {
            EffectParams::Eq(p) => p.field_mut(i),
            EffectParams::Compressor(p) => p.field_mut(i),
            EffectParams::Reverb(p) => p.field_mut(i),
            EffectParams::Delay(p) => p.field_mut(i),
            EffectParams::Distortion(p) => p.field_mut(i),
            EffectParams::Chorus(p) => p.field_mut(i),
            EffectParams::Filter(p) => p.field_mut(i),
            EffectParams::Limiter(p) => p.field_mut(i),
            EffectParams::Sidechain(p) => p.field_mut(i),
            EffectParams::Autotune(p) => p.field_mut(i),
            EffectParams::Clipper(p) => p.field_mut(i),
            EffectParams::Multiband(p) => p.field_mut(i),
            EffectParams::Ott(p) => p.field_mut(i),
        }
    }

    pub fn field(&self, i: usize) -> Option<f32> {
        match self {
            EffectParams::Eq(p) => p.field(i),
            EffectParams::Compressor(p) => p.field(i),
            EffectParams::Reverb(p) => p.field(i),
            EffectParams::Delay(p) => p.field(i),
            EffectParams::Distortion(p) => p.field(i),
            EffectParams::Chorus(p) => p.field(i),
            EffectParams::Filter(p) => p.field(i),
            EffectParams::Limiter(p) => p.field(i),
            EffectParams::Sidechain(p) => p.field(i),
            EffectParams::Autotune(p) => p.field(i),
            EffectParams::Clipper(p) => p.field(i),
            EffectParams::Multiband(p) => p.field(i),
            EffectParams::Ott(p) => p.field(i),
        }
    }

    /// Delay (samples) this effect adds; the engine starts audio clips that much earlier.
    pub fn latency_frames(&self, sr: f32) -> usize {
        match self {
            EffectParams::Autotune(_) => crate::autotune::latency_frames(sr),
            _ => 0,
        }
    }

    /// Mixer track used as sidechain key, if this is a sidechain effect.
    pub fn sidechain_source(&self) -> Option<usize> {
        match self {
            EffectParams::Sidechain(p) => Some(p.source),
            _ => None,
        }
    }
}

numeric_fields!(EqParams { low_freq, low_gain, mid_freq, mid_gain, mid_q, high_freq, high_gain });
numeric_fields!(CompParams { threshold, ratio, attack_ms, release_ms, knee, makeup, mix });
numeric_fields!(ReverbParams { size, damping, predelay_ms, width, mix });
numeric_fields!(DelayParams { time_beats, feedback, mix, tone });
numeric_fields!(DistParams { drive_db, tone, output_db, mix, bits, downsample });
numeric_fields!(ChorusParams { rate, depth_ms, delay_ms, mix });
numeric_fields!(FilterFxParams { cutoff, resonance, mix });
numeric_fields!(LimiterParams { gain_db, ceiling_db, release_ms });
numeric_fields!(SidechainParams { depth, threshold_db, attack_ms, release_ms });

/// Ducks this track whenever the key track (e.g. the kick) plays – the classic pumping /
/// rumble technique. Full ducking is reached when the key hits `threshold_db`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SidechainParams {
    /// mixer track whose signal triggers the ducking
    pub source: usize,
    /// 0..1 maximum gain reduction
    pub depth: f32,
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

impl Default for SidechainParams {
    fn default() -> Self {
        Self { source: 1, depth: 0.85, threshold_db: -18.0, attack_ms: 1.0, release_ms: 160.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EqParams {
    pub low_freq: f32,
    pub low_gain: f32,
    pub mid_freq: f32,
    pub mid_gain: f32,
    pub mid_q: f32,
    pub high_freq: f32,
    pub high_gain: f32,
}

impl EqParams {
    /// Frequency response of the three bands in dB (for the visual EQ).
    pub fn response_db(&self, f: f32, sr: f32) -> f32 {
        let mut lo = Biquad::default();
        let mut mid = Biquad::default();
        let mut hi = Biquad::default();
        lo.low_shelf(self.low_freq, self.low_gain, sr);
        mid.peaking(self.mid_freq, self.mid_q, self.mid_gain, sr);
        hi.high_shelf(self.high_freq, self.high_gain, sr);
        lo.mag_db(f, sr) + mid.mag_db(f, sr) + hi.mag_db(f, sr)
    }
}

impl Default for EqParams {
    fn default() -> Self {
        Self { low_freq: 120.0, low_gain: 0.0, mid_freq: 1000.0, mid_gain: 0.0, mid_q: 0.8, high_freq: 8000.0, high_gain: 0.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompParams {
    pub threshold: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee: f32,
    pub makeup: f32,
    pub mix: f32,
}

impl Default for CompParams {
    fn default() -> Self {
        Self { threshold: -18.0, ratio: 4.0, attack_ms: 10.0, release_ms: 120.0, knee: 6.0, makeup: 0.0, mix: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReverbParams {
    pub size: f32,
    pub damping: f32,
    pub predelay_ms: f32,
    pub width: f32,
    pub mix: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self { size: 0.6, damping: 0.5, predelay_ms: 10.0, width: 1.0, mix: 0.25 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DelayParams {
    /// delay time in beats (0.75 = dotted eighth)
    pub time_beats: f32,
    pub feedback: f32,
    pub mix: f32,
    pub ping_pong: bool,
    /// lowpass in the feedback path, Hz
    pub tone: f32,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self { time_beats: 0.75, feedback: 0.4, mix: 0.3, ping_pong: true, tone: 6000.0 }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum DistMode {
    #[default]
    Soft,
    Hard,
    Fold,
    Crush,
}

impl DistMode {
    pub const ALL: [DistMode; 4] = [DistMode::Soft, DistMode::Hard, DistMode::Fold, DistMode::Crush];
    pub fn name(self) -> &'static str {
        match self {
            DistMode::Soft => "Soft Clip",
            DistMode::Hard => "Hard Clip",
            DistMode::Fold => "Wavefold",
            DistMode::Crush => "Bitcrush",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DistParams {
    pub mode: DistMode,
    pub drive_db: f32,
    pub tone: f32,
    pub output_db: f32,
    pub mix: f32,
    pub bits: f32,
    pub downsample: f32,
}

impl Default for DistParams {
    fn default() -> Self {
        Self { mode: DistMode::Soft, drive_db: 12.0, tone: 9000.0, output_db: -6.0, mix: 1.0, bits: 8.0, downsample: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ChorusParams {
    pub rate: f32,
    pub depth_ms: f32,
    pub delay_ms: f32,
    pub mix: f32,
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self { rate: 0.8, depth_ms: 3.0, delay_ms: 12.0, mix: 0.5 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FilterFxParams {
    pub mode: FilterMode,
    pub cutoff: f32,
    pub resonance: f32,
    pub mix: f32,
}

impl Default for FilterFxParams {
    fn default() -> Self {
        Self { mode: FilterMode::LowPass, cutoff: 2000.0, resonance: 0.2, mix: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LimiterParams {
    pub gain_db: f32,
    pub ceiling_db: f32,
    pub release_ms: f32,
}

impl Default for LimiterParams {
    fn default() -> Self {
        Self { gain_db: 0.0, ceiling_db: -0.3, release_ms: 80.0 }
    }
}

// ---------------------------------------------------------------- processors

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
            _ => {}
        }
    }
}

#[inline]
fn mix(dry: f32, wet: f32, m: f32) -> f32 {
    dry + (wet - dry) * m
}

#[derive(Default)]
pub struct EqFx {
    bands: [[Biquad; 3]; 2],
    last: Option<(EqParams, u32)>,
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
    gr_db: f32,
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

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }
    #[inline]
    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
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

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
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
    combs: [Vec<Comb>; 2],
    aps: [Vec<Allpass>; 2],
    pre: [Vec<f32>; 2],
    pre_idx: usize,
    sr: f32,
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const AP_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

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
    buf: [Vec<f32>; 2],
    idx: usize,
    lp: [OnePole; 2],
    cur: f32,
}

impl DelayFx {
    pub fn new(sr: f32) -> Self {
        let n = (4.0 * sr) as usize + 4;
        Self { buf: [vec![0.0; n], vec![0.0; n]], idx: 0, lp: [OnePole::default(); 2], cur: -1.0 }
    }

    #[inline]
    fn read(buf: &[f32], idx: usize, delay: f32) -> f32 {
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
    tone: [OnePole; 2],
    hold: [f32; 2],
    hold_ctr: f32,
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
    buf: [Vec<f32>; 2],
    idx: usize,
    phase: f32,
    sr: f32,
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
    svf: [Svf; 2],
    cur_cutoff: f32,
}

impl FilterFx {
    fn process(&mut self, p: &FilterFxParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
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
    gain: f32,
    pub last_gr_db: f32,
}

impl Default for LimiterFx {
    fn default() -> Self {
        Self { gain: 1.0, last_gr_db: 0.0 }
    }
}

impl LimiterFx {
    fn process(&mut self, p: &LimiterParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
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
    env: f32,
    /// current gain reduction 0..1 (for metering)
    pub last_duck: f32,
}

impl SidechainFx {
    fn process(&mut self, p: &SidechainParams, ctx: &FxContext, l: &mut [f32], r: &mut [f32]) {
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

#[cfg(test)]
mod tests {
    #[test]
    fn eq_response_matches_bands() {
        let flat = super::EqParams::default();
        for f in [30.0, 1000.0, 15000.0] {
            assert!(flat.response_db(f, 48000.0).abs() < 0.01);
        }
        let e = super::EqParams { low_gain: 6.0, mid_gain: -9.0, high_gain: 4.0, ..Default::default() };
        assert!((e.response_db(25.0, 48000.0) - 6.0).abs() < 0.5);
        assert!((e.response_db(1000.0, 48000.0) + 9.0).abs() < 0.3);
        assert!((e.response_db(18000.0, 48000.0) - 4.0).abs() < 0.6);
    }

    use super::*;

    #[test]
    fn all_effects_stay_finite() {
        let ctx = FxContext { sr: 48000.0, bpm: 140.0, key: &[] };
        for kind in EffectKind::ALL {
            let mut fx = Effect::new(kind, ctx.sr);
            let p = kind.default_params();
            let mut l: Vec<f32> = (0..48000).map(|i| ((i as f32) * 0.03).sin() * 0.9).collect();
            let mut r = l.clone();
            for (cl, cr) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
                fx.process(&p, &ctx, cl, cr);
            }
            assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{} produced NaN/inf", kind.name());
            assert!(l.iter().any(|v| v.abs() > 1e-3), "{} silenced the signal", kind.name());
        }
    }

    #[test]
    fn limiter_respects_ceiling() {
        let ctx = FxContext { sr: 48000.0, bpm: 120.0, key: &[] };
        let mut fx = Effect::new(EffectKind::Limiter, ctx.sr);
        let p = EffectParams::Limiter(LimiterParams { gain_db: 12.0, ceiling_db: -1.0, release_ms: 50.0 });
        let mut l: Vec<f32> = (0..4800).map(|i| ((i as f32) * 0.05).sin()).collect();
        let mut r = l.clone();
        fx.process(&p, &ctx, &mut l, &mut r);
        let c = db_to_gain(-1.0) + 1e-6;
        assert!(l.iter().all(|v| v.abs() <= c));
    }

    #[test]
    fn sidechain_ducks_on_key() {
        let key: Vec<f32> = (0..4800).map(|i| if i < 2400 { 1.0 } else { 0.0 }).collect();
        let ctx = FxContext { sr: 48000.0, bpm: 120.0, key: &key };
        let mut fx = Effect::new(EffectKind::Sidechain, ctx.sr);
        let p = EffectParams::Sidechain(SidechainParams { depth: 1.0, release_ms: 10.0, ..Default::default() });
        let mut l = vec![0.5f32; 4800];
        let mut r = l.clone();
        fx.process(&p, &ctx, &mut l, &mut r);
        assert!(l[1000].abs() < 0.01, "ducked while key plays");
        assert!(l[4700] > 0.45, "recovers after release");
    }

    #[test]
    fn field_accessors_cover_names() {
        for kind in EffectKind::ALL {
            let mut p = kind.default_params();
            let n = p.field_names().len();
            assert!(p.field_mut(n - 1).is_some() && p.field_mut(n).is_none(), "{}", kind.name());
        }
    }
}
