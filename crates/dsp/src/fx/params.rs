//! Effect kinds and parameter structs (the serialised side of the FX chain).

use super::*;

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
    Width,
}

impl EffectKind {
    pub const ALL: [EffectKind; 14] = [
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
        EffectKind::Width,
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
            EffectKind::Width => "Width",
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
            EffectKind::Width => EffectParams::Width(WidthParams::default()),
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
    Width(WidthParams),
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
            EffectParams::Width(_) => EffectKind::Width,
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
            EffectParams::Width(_) => WidthParams::FIELDS,
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
            EffectParams::Width(p) => p.field_mut(i),
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
            EffectParams::Width(p) => p.field(i),
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
