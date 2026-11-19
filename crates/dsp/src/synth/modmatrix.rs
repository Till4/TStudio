//! Modulation matrix: sources, destinations and slots.

use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
pub enum ModSource {
    #[default]
    Off,
    AmpEnv,
    FilterEnv,
    ModEnv,
    Lfo1,
    Lfo2,
    Lfo3,
    Lfo4,
    Velocity,
    /// key relative to C4, ±1 at ±4 octaves
    Note,
    /// per-note random -1..1
    Random,
    Macro1,
    Macro2,
    Macro3,
    Macro4,
    /// kick pitch sweep 1 → 0
    KickSweep,
}

impl ModSource {
    pub const ALL: [ModSource; 16] = [
        ModSource::Off,
        ModSource::AmpEnv,
        ModSource::FilterEnv,
        ModSource::ModEnv,
        ModSource::Lfo1,
        ModSource::Lfo2,
        ModSource::Lfo3,
        ModSource::Lfo4,
        ModSource::Velocity,
        ModSource::Note,
        ModSource::Random,
        ModSource::Macro1,
        ModSource::Macro2,
        ModSource::Macro3,
        ModSource::Macro4,
        ModSource::KickSweep,
    ];
    pub fn name(self) -> &'static str {
        match self {
            ModSource::Off => "—",
            ModSource::AmpEnv => "Env 1 (Amp)",
            ModSource::FilterEnv => "Env 2 (Filter)",
            ModSource::ModEnv => "Env 3 (Mod)",
            ModSource::Lfo1 => "LFO 1",
            ModSource::Lfo2 => "LFO 2",
            ModSource::Lfo3 => "LFO 3",
            ModSource::Lfo4 => "LFO 4",
            ModSource::Velocity => "Velocity",
            ModSource::Note => "Note",
            ModSource::Random => "Random",
            ModSource::Macro1 => "Makro 1",
            ModSource::Macro2 => "Makro 2",
            ModSource::Macro3 => "Makro 3",
            ModSource::Macro4 => "Makro 4",
            ModSource::KickSweep => "Kick-Sweep",
        }
    }
    pub fn is_macro(self) -> bool {
        matches!(self, ModSource::Macro1 | ModSource::Macro2 | ModSource::Macro3 | ModSource::Macro4)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
pub enum ModDest {
    #[default]
    Off,
    /// all oscillators, ±24 st
    Pitch,
    Osc1Pitch,
    Osc2Pitch,
    Osc3Pitch,
    Osc1Pos,
    Osc2Pos,
    Osc3Pos,
    Osc1Warp,
    Osc2Warp,
    Osc3Warp,
    Osc1Level,
    Osc2Level,
    Osc3Level,
    /// unison detune ×(1+m)
    Detune,
    SubLevel,
    NoiseLevel,
    /// ±6 octaves
    Cutoff1,
    Reso1,
    Cutoff2,
    Reso2,
    /// ±24 dB on both filter drives
    FilterDrive,
    /// gain ×(1+m)
    Amp,
    Pan,
    /// ×2^(±4)
    Lfo1Rate,
    Lfo2Rate,
    Lfo3Rate,
    Lfo4Rate,
    /// ±48 st on the kick sweep start
    KickPitch,
    /// ±24 dB
    KickDrive,
    ClickLevel,
    // ---- global (only macro sources reach these)
    /// ±24 dB on the FX distortion
    FxDistDrive,
    FxReverbMix,
    FxDelayMix,
    FxChorusMix,
}

impl ModDest {
    pub const COUNT: usize = 35;
    pub const ALL: [ModDest; 35] = [
        ModDest::Off,
        ModDest::Pitch,
        ModDest::Osc1Pitch,
        ModDest::Osc2Pitch,
        ModDest::Osc3Pitch,
        ModDest::Osc1Pos,
        ModDest::Osc2Pos,
        ModDest::Osc3Pos,
        ModDest::Osc1Warp,
        ModDest::Osc2Warp,
        ModDest::Osc3Warp,
        ModDest::Osc1Level,
        ModDest::Osc2Level,
        ModDest::Osc3Level,
        ModDest::Detune,
        ModDest::SubLevel,
        ModDest::NoiseLevel,
        ModDest::Cutoff1,
        ModDest::Reso1,
        ModDest::Cutoff2,
        ModDest::Reso2,
        ModDest::FilterDrive,
        ModDest::Amp,
        ModDest::Pan,
        ModDest::Lfo1Rate,
        ModDest::Lfo2Rate,
        ModDest::Lfo3Rate,
        ModDest::Lfo4Rate,
        ModDest::KickPitch,
        ModDest::KickDrive,
        ModDest::ClickLevel,
        ModDest::FxDistDrive,
        ModDest::FxReverbMix,
        ModDest::FxDelayMix,
        ModDest::FxChorusMix,
    ];
    pub fn name(self) -> &'static str {
        match self {
            ModDest::Off => "—",
            ModDest::Pitch => "Pitch (alle)",
            ModDest::Osc1Pitch => "Osc 1 Pitch",
            ModDest::Osc2Pitch => "Osc 2 Pitch",
            ModDest::Osc3Pitch => "Osc 3 Pitch",
            ModDest::Osc1Pos => "Osc 1 Position",
            ModDest::Osc2Pos => "Osc 2 Position",
            ModDest::Osc3Pos => "Osc 3 Position",
            ModDest::Osc1Warp => "Osc 1 Warp",
            ModDest::Osc2Warp => "Osc 2 Warp",
            ModDest::Osc3Warp => "Osc 3 Warp",
            ModDest::Osc1Level => "Osc 1 Level",
            ModDest::Osc2Level => "Osc 2 Level",
            ModDest::Osc3Level => "Osc 3 Level",
            ModDest::Detune => "Unison Detune",
            ModDest::SubLevel => "Sub Level",
            ModDest::NoiseLevel => "Noise Level",
            ModDest::Cutoff1 => "Filter 1 Cutoff",
            ModDest::Reso1 => "Filter 1 Reso",
            ModDest::Cutoff2 => "Filter 2 Cutoff",
            ModDest::Reso2 => "Filter 2 Reso",
            ModDest::FilterDrive => "Filter Drive",
            ModDest::Amp => "Lautstärke",
            ModDest::Pan => "Pan",
            ModDest::Lfo1Rate => "LFO 1 Rate",
            ModDest::Lfo2Rate => "LFO 2 Rate",
            ModDest::Lfo3Rate => "LFO 3 Rate",
            ModDest::Lfo4Rate => "LFO 4 Rate",
            ModDest::KickPitch => "Kick Pitch",
            ModDest::KickDrive => "Kick Drive",
            ModDest::ClickLevel => "Click Level",
            ModDest::FxDistDrive => "FX Dist Drive (Makro)",
            ModDest::FxReverbMix => "FX Reverb Mix (Makro)",
            ModDest::FxDelayMix => "FX Delay Mix (Makro)",
            ModDest::FxChorusMix => "FX Chorus Mix (Makro)",
        }
    }
    /// FX destinations: shared by all voices, so only macros can drive them
    pub fn is_global(self) -> bool {
        matches!(self, ModDest::FxDistDrive | ModDest::FxReverbMix | ModDest::FxDelayMix | ModDest::FxChorusMix)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct ModSlot {
    pub source: ModSource,
    pub dest: ModDest,
    /// -1..1
    pub amount: f32,
}

impl ModSlot {
    pub const fn new(source: ModSource, dest: ModDest, amount: f32) -> Self {
        Self { source, dest, amount }
    }
    pub fn active(&self) -> bool {
        self.source != ModSource::Off && self.dest != ModDest::Off && self.amount != 0.0
    }
}
