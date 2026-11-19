//! TSynth: wavetable / subtractive synthesizer with a dedicated kick engine, two filters,
//! three envelopes, four LFOs, a modulation matrix, macros and a built-in FX chain.
//!
//! Compatibility: every field added after the first TSynth version defaults to "off" /
//! neutral, so old projects (which only store the original fields) load and sound the same.

use crate::env::{Adsr, AdsrParams};
use crate::filter::{FilterMode, FilterType, OnePole, Svf, SynthFilter};
use crate::fx::{ChorusFx, ChorusParams, CompFx, CompParams, DelayFx, DelayParams, DistFx, DistParams, FxContext, ReverbFx, ReverbParams};
use crate::fx_extra::{ClipperFx, ClipperParams, OttFx, OttParams};
use crate::osc::{Osc, Waveform};
use crate::wavetable::{self, Wavetable, WtKind};
use crate::{midi_to_freq, Rng};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

pub use crate::synth_presets::PRESET_CATEGORIES;

pub const MAX_UNISON: usize = 9;
/// unison slots of the first TSynth version (their random start phases are drawn first)
const LEGACY_UNISON: usize = 7;
const CONTROL_RATE: u32 = 16;
pub const NUM_OSC: usize = 3;
pub const NUM_LFO: usize = 4;
pub const NUM_MOD_SLOTS: usize = 16;
/// lowest comb-filter frequency (sets the delay buffer size)
const COMB_MIN_HZ: f32 = 20.0;
/// internal block for the FX chain
const FX_BLOCK: usize = 256;
/// tanh(0.35): offset of the asymmetric drive curve
const ASYM_OFS: f32 = 0.336_375_8;

// ------------------------------------------------------------------ oscillators

/// Phase warp applied before the wavetable lookup.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
pub enum WarpMode {
    #[default]
    Off,
    /// phase bent towards the start (brighter front)
    BendPlus,
    /// phase bent towards the end
    BendMinus,
    /// hard sync to a virtual master (1× → 8×)
    Sync,
    /// squeezes the cycle into the first part (pulse-width style)
    Pwm,
    /// steps the phase (lo-fi / digital)
    Quantize,
    /// phase modulation from oscillator 1 / 2 / 3 / sub
    FmOsc1,
    FmOsc2,
    FmOsc3,
    FmSub,
}

impl WarpMode {
    pub const ALL: [WarpMode; 10] = [
        WarpMode::Off,
        WarpMode::BendPlus,
        WarpMode::BendMinus,
        WarpMode::Sync,
        WarpMode::Pwm,
        WarpMode::Quantize,
        WarpMode::FmOsc1,
        WarpMode::FmOsc2,
        WarpMode::FmOsc3,
        WarpMode::FmSub,
    ];
    pub fn name(self) -> &'static str {
        match self {
            WarpMode::Off => "Kein Warp",
            WarpMode::BendPlus => "Bend +",
            WarpMode::BendMinus => "Bend −",
            WarpMode::Sync => "Sync",
            WarpMode::Pwm => "PWM",
            WarpMode::Quantize => "Quantize",
            WarpMode::FmOsc1 => "FM ← Osc 1",
            WarpMode::FmOsc2 => "FM ← Osc 2",
            WarpMode::FmOsc3 => "FM ← Osc 3",
            WarpMode::FmSub => "FM ← Sub",
        }
    }
    fn fm_source(self) -> Option<usize> {
        match self {
            WarpMode::FmOsc1 => Some(0),
            WarpMode::FmOsc2 => Some(1),
            WarpMode::FmOsc3 => Some(2),
            WarpMode::FmSub => Some(3),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OscParams {
    pub enabled: bool,
    /// classic shape (used by the `Basic` table)
    pub wave: Waveform,
    pub octave: i32,
    pub semi: i32,
    /// cents
    pub fine: f32,
    pub level: f32,
    pub unison: u32,
    /// cents spread between outermost unison voices
    pub detune: f32,
    /// stereo spread 0..1
    pub spread: f32,
    /// wavetable (Basic = classic analog shapes)
    pub table: WtKind,
    /// 0..1 position inside the wavetable
    pub position: f32,
    pub warp: WarpMode,
    /// 0..1
    pub warp_amount: f32,
    /// unison: level of the side voices relative to the centre (1 = all equal)
    pub blend: f32,
    /// start phase 0..1 (used when `rand_phase` < 1 and always in kick mode)
    pub phase: f32,
    /// 0..1 how much the start phase is randomised per note
    pub rand_phase: f32,
    /// -1..1
    pub pan: f32,
}

impl Default for OscParams {
    fn default() -> Self {
        Self {
            enabled: true,
            wave: Waveform::Saw,
            octave: 0,
            semi: 0,
            fine: 0.0,
            level: 0.7,
            unison: 1,
            detune: 20.0,
            spread: 0.6,
            table: WtKind::Basic,
            position: 0.0,
            warp: WarpMode::Off,
            warp_amount: 0.0,
            blend: 1.0,
            phase: 0.0,
            rand_phase: 1.0,
            pan: 0.0,
        }
    }
}

impl OscParams {
    pub fn off() -> Self {
        Self { enabled: false, ..Self::default() }
    }
    /// The frame index of the `Basic` table matching `wave`.
    fn basic_frame(&self) -> usize {
        match self.wave {
            Waveform::Sine => 0,
            Waveform::Saw => 1,
            Waveform::Square => 2,
            Waveform::Triangle => 3,
        }
    }
    /// One cycle of this oscillator (for drawing).
    pub fn preview(&self, n: usize) -> Vec<f32> {
        let tbl = wavetable::table(self.table);
        let k = warp_param(self.warp, self.warp_amount);
        (0..n)
            .map(|i| {
                let p = i as f32 / n as f32;
                let fm = if self.warp.fm_source().is_some() { (TAU * p).sin() } else { 0.0 };
                let q = warp_phase(self.warp, k, self.warp_amount, p, fm);
                if self.table == WtKind::Basic {
                    tbl.sample_frame(self.basic_frame(), q, 0)
                } else {
                    tbl.sample(self.position, q, 0)
                }
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum NoiseType {
    #[default]
    White,
    /// clicky random impulses
    Crackle,
    /// stepped random values (sample & hold at 4× the note pitch)
    Digital,
}

impl NoiseType {
    pub const ALL: [NoiseType; 3] = [NoiseType::White, NoiseType::Crackle, NoiseType::Digital];
    pub fn name(self) -> &'static str {
        match self {
            NoiseType::White => "White",
            NoiseType::Crackle => "Crackle",
            NoiseType::Digital => "Digital",
        }
    }
}

// ------------------------------------------------------------------ kick engine

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ClickType {
    /// band-passed noise burst
    #[default]
    Noise,
    /// short sine tick at the click tone
    Tick,
    /// falling sine chirp (punchy "snap")
    Snap,
}

impl ClickType {
    pub const ALL: [ClickType; 3] = [ClickType::Noise, ClickType::Tick, ClickType::Snap];
    pub fn name(self) -> &'static str {
        match self {
            ClickType::Noise => "Noise",
            ClickType::Tick => "Tick",
            ClickType::Snap => "Snap",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum DriveMode {
    #[default]
    Soft,
    Hard,
    Fold,
    /// asymmetric tube-style (even harmonics)
    Asym,
}

impl DriveMode {
    pub const ALL: [DriveMode; 4] = [DriveMode::Soft, DriveMode::Hard, DriveMode::Fold, DriveMode::Asym];
    pub fn name(self) -> &'static str {
        match self {
            DriveMode::Soft => "Soft",
            DriveMode::Hard => "Hard",
            DriveMode::Fold => "Fold",
            DriveMode::Asym => "Asym",
        }
    }
    /// Transfer curve for displays: output for input `x` at `drive_db` (0 dB = clean).
    pub fn transfer(self, x: f32, drive_db: f32) -> f32 {
        if drive_db <= 0.0 {
            return x;
        }
        let g = crate::db_to_gain(drive_db);
        self.apply(x, g, self.norm(g))
    }
    /// output normalisation for gain `g` (keeps a full-scale input near full scale)
    fn norm(self, g: f32) -> f32 {
        match self {
            DriveMode::Asym => 1.0 / ((g + 0.35).tanh() - ASYM_OFS).max(1e-3),
            _ => 1.0 / g.max(1e-3).tanh(),
        }
    }
    #[inline]
    fn apply(self, x: f32, g: f32, norm: f32) -> f32 {
        match self {
            DriveMode::Soft => (x * g).tanh() * norm,
            DriveMode::Hard => (x * g).clamp(-1.0, 1.0),
            DriveMode::Fold => (x * g * std::f32::consts::FRAC_PI_2).sin(),
            DriveMode::Asym => ((x * g + 0.35).tanh() - ASYM_OFS) * norm,
        }
    }
}

/// Kick engine: when enabled every note plays the kick tuned to `tune` (MIDI note of the
/// body) with its own pitch sweep, click layer, body envelope, rumble and drive.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KickParams {
    pub enabled: bool,
    /// MIDI note of the body after the sweep (29 = F1 ≈ 43.7 Hz)
    pub tune: f32,
    /// played key transposes the kick (relative to C4). With amp sustain 0 the kick is a
    /// one-shot: it ignores the note length.
    pub key_follow: bool,
    /// semitones above the body at the hit
    pub pitch_start: f32,
    /// seconds until the sweep lands on the body note
    pub pitch_time: f32,
    /// -1..1: >0 drops fast then settles (punchy), 0 linear in semitones, <0 slow start
    pub pitch_curve: f32,
    pub click_level: f32,
    pub click_type: ClickType,
    /// Hz
    pub click_tone: f32,
    /// seconds
    pub click_decay: f32,
    /// seconds at full level for the body
    pub body_hold: f32,
    /// seconds; 0 = body follows the amp envelope only
    pub body_decay: f32,
    /// dB, saturation of body + click before the filters
    pub drive_db: f32,
    pub drive_mode: DriveMode,
    /// rumble layer (distorted, low-passed drone at the body pitch)
    pub rumble_level: f32,
    /// seconds before the rumble swells in
    pub rumble_delay: f32,
    /// seconds
    pub rumble_decay: f32,
    /// Hz, low pass of the rumble
    pub rumble_tone: f32,
}

impl Default for KickParams {
    fn default() -> Self {
        Self {
            enabled: false,
            tune: 29.0,
            key_follow: false,
            pitch_start: 40.0,
            pitch_time: 0.07,
            pitch_curve: 0.6,
            click_level: 0.3,
            click_type: ClickType::Tick,
            click_tone: 2800.0,
            click_decay: 0.006,
            body_hold: 0.0,
            body_decay: 0.0,
            drive_db: 0.0,
            drive_mode: DriveMode::Soft,
            rumble_level: 0.0,
            rumble_delay: 0.06,
            rumble_decay: 0.4,
            rumble_tone: 160.0,
        }
    }
}

impl KickParams {
    /// Normalised sweep 1 → 0 over `pitch_time`.
    #[inline]
    pub fn sweep(&self, t: f32) -> f32 {
        let x = (t / self.pitch_time.max(0.001)).clamp(0.0, 1.0);
        let c = self.pitch_curve.clamp(-1.0, 1.0);
        let e = if c >= 0.0 { 1.0 + 7.0 * c } else { 1.0 / (1.0 - 3.0 * c) };
        (1.0 - x).powf(e)
    }
    /// Frequency of the body at `t` seconds for `key` (no modulation).
    pub fn freq_at(&self, key: u8, t: f32) -> f32 {
        midi_to_freq(self.body_note(key) + self.pitch_start * self.sweep(t))
    }
    pub fn body_note(&self, key: u8) -> f32 {
        self.tune + if self.key_follow { key as f32 - 60.0 } else { 0.0 }
    }
}

// ------------------------------------------------------------------ filters

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum FilterRouting {
    #[default]
    Serial,
    Parallel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Filter2Params {
    pub enabled: bool,
    pub kind: FilterType,
    pub mode: FilterMode,
    /// Hz
    pub cutoff: f32,
    /// 0..1
    pub resonance: f32,
    /// dB pre-filter saturation
    pub drive_db: f32,
    /// octaves at full filter envelope
    pub env_amount: f32,
    /// 0..1
    pub key_track: f32,
}

impl Default for Filter2Params {
    fn default() -> Self {
        Self { enabled: false, kind: FilterType::Svf12, mode: FilterMode::HighPass, cutoff: 120.0, resonance: 0.1, drive_db: 0.0, env_amount: 0.0, key_track: 0.0 }
    }
}

// ------------------------------------------------------------------ LFOs

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LfoTarget {
    #[default]
    Off,
    Pitch,
    Cutoff,
    Amp,
    Pan,
}

impl LfoTarget {
    pub const ALL: [LfoTarget; 5] = [LfoTarget::Off, LfoTarget::Pitch, LfoTarget::Cutoff, LfoTarget::Amp, LfoTarget::Pan];
    pub fn name(self) -> &'static str {
        match self {
            LfoTarget::Off => "Off",
            LfoTarget::Pitch => "Pitch",
            LfoTarget::Cutoff => "Cutoff",
            LfoTarget::Amp => "Amp",
            LfoTarget::Pan => "Pan",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LfoShape {
    /// the classic shape in `wave`
    #[default]
    Wave,
    RampDown,
    SampleHold,
    /// 16 user steps (`custom`)
    Custom,
}

impl LfoShape {
    pub const ALL: [LfoShape; 4] = [LfoShape::Wave, LfoShape::RampDown, LfoShape::SampleHold, LfoShape::Custom];
    pub fn name(self) -> &'static str {
        match self {
            LfoShape::Wave => "Form",
            LfoShape::RampDown => "Ramp ↓",
            LfoShape::SampleHold => "S&H",
            LfoShape::Custom => "Eigene",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LfoParams {
    pub wave: Waveform,
    /// Hz (free mode)
    pub rate: f32,
    /// 0..1 depth of the direct `target`
    pub amount: f32,
    /// direct target (the mod matrix can use the LFO as well)
    pub target: LfoTarget,
    pub shape: LfoShape,
    /// tempo sync: one cycle lasts `beats`
    pub sync: bool,
    /// cycle length in beats when synced (0.25 = 1/16)
    pub beats: f32,
    /// restart the cycle at every note
    pub retrig: bool,
    /// start phase 0..1 when retriggered
    pub phase: f32,
    /// custom shape, -1..1 per step
    pub custom: [f32; 16],
    /// glide between custom steps
    pub smooth: bool,
}

impl Default for LfoParams {
    fn default() -> Self {
        let mut custom = [0.0; 16];
        for (i, c) in custom.iter_mut().enumerate() {
            *c = 1.0 - 2.0 * (i % 4) as f32 / 3.0;
        }
        Self { wave: Waveform::Sine, rate: 5.0, amount: 0.0, target: LfoTarget::Off, shape: LfoShape::Wave, sync: false, beats: 1.0, retrig: false, phase: 0.0, custom, smooth: false }
    }
}

impl LfoParams {
    /// Value at phase `p` (0..1) for drawing (S&H shown as a fixed pattern).
    pub fn value_at(&self, p: f32) -> f32 {
        match self.shape {
            LfoShape::Wave => match self.wave {
                Waveform::Sine => (TAU * p).sin(),
                Waveform::Saw => 2.0 * p - 1.0,
                Waveform::Square => {
                    if p < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                Waveform::Triangle => {
                    if p < 0.5 {
                        4.0 * p - 1.0
                    } else {
                        3.0 - 4.0 * p
                    }
                }
            },
            LfoShape::RampDown => 1.0 - 2.0 * p,
            LfoShape::SampleHold => {
                let mut r = Rng(0x1234_5677 ^ ((p * 8.0) as u32 + 1).wrapping_mul(2_654_435_761));
                r.next_f32()
            }
            LfoShape::Custom => {
                let x = p.clamp(0.0, 0.9999) * 16.0;
                let i = (x as usize).min(15);
                if self.smooth {
                    let f = x - i as f32;
                    self.custom[i] + (self.custom[(i + 1) % 16] - self.custom[i]) * f
                } else {
                    self.custom[i]
                }
            }
        }
    }
    /// cycles per second
    pub fn hz(&self, bpm: f32) -> f32 {
        if self.sync {
            bpm.max(1.0) / 60.0 / self.beats.max(1.0 / 64.0)
        } else {
            self.rate
        }
    }
}

// ------------------------------------------------------------------ mod matrix

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

// ------------------------------------------------------------------ FX

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SynthFxParams {
    pub dist_on: bool,
    pub dist: DistParams,
    pub comp_on: bool,
    pub comp: CompParams,
    pub ott_on: bool,
    pub ott: OttParams,
    pub chorus_on: bool,
    pub chorus: ChorusParams,
    pub delay_on: bool,
    pub delay: DelayParams,
    pub reverb_on: bool,
    pub reverb: ReverbParams,
    /// final clipper (keeps loud kicks under 0 dBFS)
    pub clip_on: bool,
    pub clip: ClipperParams,
}

impl Default for SynthFxParams {
    fn default() -> Self {
        Self {
            dist_on: false,
            dist: DistParams { output_db: -3.0, ..DistParams::default() },
            comp_on: false,
            comp: CompParams::default(),
            ott_on: false,
            ott: OttParams::default(),
            chorus_on: false,
            chorus: ChorusParams { mix: 0.35, ..ChorusParams::default() },
            delay_on: false,
            delay: DelayParams { mix: 0.2, ..DelayParams::default() },
            reverb_on: false,
            reverb: ReverbParams { mix: 0.2, ..ReverbParams::default() },
            clip_on: false,
            clip: ClipperParams { drive_db: 0.0, ceiling_db: -0.8, ..ClipperParams::default() },
        }
    }
}

impl SynthFxParams {
    pub fn any_on(&self) -> bool {
        self.dist_on || self.comp_on || self.ott_on || self.chorus_on || self.delay_on || self.reverb_on || self.clip_on
    }
    /// seconds the chain keeps ringing after the last voice
    fn tail_secs(&self) -> f32 {
        if self.reverb_on || self.delay_on {
            6.0
        } else if self.chorus_on || self.comp_on || self.ott_on {
            0.5
        } else {
            0.05
        }
    }
}

// ------------------------------------------------------------------ params

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SynthParams {
    pub osc1: OscParams,
    pub osc2: OscParams,
    pub sub_level: f32,
    pub noise_level: f32,
    pub filter_mode: FilterMode,
    /// Hz
    pub cutoff: f32,
    /// 0..1
    pub resonance: f32,
    /// octaves of cutoff modulation at full envelope
    pub filter_env_amount: f32,
    /// 0..1 how much cutoff follows the key
    pub key_track: f32,
    pub amp_env: AdsrParams,
    pub filter_env: AdsrParams,
    pub lfo1: LfoParams,
    pub lfo2: LfoParams,
    /// seconds
    pub glide: f32,
    pub mono: bool,
    /// linear output gain
    pub gain: f32,
    /// 0..1
    pub velocity_sens: f32,
    /// semitones added at note start, decaying to 0 (toms, zaps)
    pub pitch_env_amount: f32,
    /// seconds (time constant of the pitch drop)
    pub pitch_env_decay: f32,
    /// saturation after the filter in dB (0 = clean)
    pub drive_db: f32,

    // ---- added with the TSynth expansion (all neutral by default)
    pub osc3: OscParams,
    /// sub oscillator shape and octave (relative to oscillator 1)
    pub sub_wave: Waveform,
    pub sub_octave: i32,
    pub noise_type: NoiseType,
    /// Hz low pass on the noise (≥ 19 kHz = off)
    pub noise_tone: f32,
    pub kick: KickParams,
    pub filter_enabled: bool,
    pub filter_type: FilterType,
    /// dB pre-filter saturation of filter 1
    pub filter_drive: f32,
    pub filter2: Filter2Params,
    pub filter_routing: FilterRouting,
    /// envelope 3 (free modulation source)
    pub mod_env: AdsrParams,
    pub lfo3: LfoParams,
    pub lfo4: LfoParams,
    pub mod_matrix: [ModSlot; NUM_MOD_SLOTS],
    pub macro1: f32,
    pub macro2: f32,
    pub macro3: f32,
    pub macro4: f32,
    /// mono only: true = legato (no retrigger while a note is held), false = retrigger every note
    pub legato: bool,
    /// maximum simultaneous voices (1..16)
    pub polyphony: u32,
    /// character of the saturation after the filter
    pub drive_mode: DriveMode,
    pub fx: SynthFxParams,
    /// mono: glide only between tied (overlapping) notes – the classic 303 slide
    pub slide_only: bool,
}

/// Automatable numeric fields. The first block is the original list – its order is part
/// of the project format (automation lanes store indices), so only append.
macro_rules! synth_fields {
    ($( $name:literal => $($path:ident).+ ),* $(,)?) => {
        impl SynthParams {
            pub const FIELDS: &'static [&'static str] = &[$($name),*];
            #[allow(unused_assignments)]
            pub fn field_mut(&mut self, i: usize) -> Option<&mut f32> {
                let mut k = 0usize;
                $( if i == k { return Some(&mut self.$($path).+); } k += 1; )*
                None
            }
            #[allow(unused_assignments)]
            pub fn field(&self, i: usize) -> Option<f32> {
                let mut k = 0usize;
                $( if i == k { return Some(self.$($path).+); } k += 1; )*
                None
            }
        }
    };
}

synth_fields!(
    "cutoff" => cutoff,
    "resonance" => resonance,
    "filter_env_amount" => filter_env_amount,
    "key_track" => key_track,
    "sub_level" => sub_level,
    "noise_level" => noise_level,
    "glide" => glide,
    "gain" => gain,
    "velocity_sens" => velocity_sens,
    "pitch_env_amount" => pitch_env_amount,
    "pitch_env_decay" => pitch_env_decay,
    "drive_db" => drive_db,
    // ---- expansion
    "macro1" => macro1,
    "macro2" => macro2,
    "macro3" => macro3,
    "macro4" => macro4,
    "osc1_position" => osc1.position,
    "osc2_position" => osc2.position,
    "osc3_position" => osc3.position,
    "osc1_warp" => osc1.warp_amount,
    "osc2_warp" => osc2.warp_amount,
    "osc3_warp" => osc3.warp_amount,
    "osc1_level" => osc1.level,
    "osc2_level" => osc2.level,
    "osc3_level" => osc3.level,
    "filter_drive" => filter_drive,
    "filter2_cutoff" => filter2.cutoff,
    "filter2_resonance" => filter2.resonance,
    "kick_tune" => kick.tune,
    "kick_pitch_start" => kick.pitch_start,
    "kick_pitch_time" => kick.pitch_time,
    "kick_click" => kick.click_level,
    "kick_drive_db" => kick.drive_db,
    "kick_rumble" => kick.rumble_level,
    "fx_dist_drive_db" => fx.dist.drive_db,
    "fx_reverb_mix" => fx.reverb.mix,
    "fx_delay_mix" => fx.delay.mix,
);

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            osc1: OscParams::default(),
            osc2: OscParams { enabled: false, wave: Waveform::Square, octave: -1, level: 0.5, ..OscParams::default() },
            sub_level: 0.0,
            noise_level: 0.0,
            filter_mode: FilterMode::LowPass,
            cutoff: 4000.0,
            resonance: 0.15,
            filter_env_amount: 1.5,
            key_track: 0.3,
            amp_env: AdsrParams::adsr(0.005, 0.3, 0.7, 0.25),
            filter_env: AdsrParams::adsr(0.005, 0.4, 0.3, 0.3),
            lfo1: LfoParams::default(),
            lfo2: LfoParams { rate: 0.5, ..LfoParams::default() },
            glide: 0.0,
            mono: false,
            gain: 0.5,
            velocity_sens: 0.6,
            pitch_env_amount: 0.0,
            pitch_env_decay: 0.04,
            drive_db: 0.0,
            osc3: OscParams { enabled: false, wave: Waveform::Sine, octave: 1, level: 0.4, ..OscParams::default() },
            sub_wave: Waveform::Sine,
            sub_octave: -1,
            noise_type: NoiseType::White,
            noise_tone: 20000.0,
            kick: KickParams::default(),
            filter_enabled: true,
            filter_type: FilterType::Svf12,
            filter_drive: 0.0,
            filter2: Filter2Params::default(),
            filter_routing: FilterRouting::Serial,
            mod_env: AdsrParams::adsr(0.005, 0.5, 0.0, 0.3),
            lfo3: LfoParams { rate: 2.0, ..LfoParams::default() },
            lfo4: LfoParams { rate: 0.25, ..LfoParams::default() },
            mod_matrix: [ModSlot::default(); NUM_MOD_SLOTS],
            macro1: 0.0,
            macro2: 0.0,
            macro3: 0.0,
            macro4: 0.0,
            legato: true,
            polyphony: 16,
            drive_mode: DriveMode::Soft,
            fx: SynthFxParams::default(),
            slide_only: false,
        }
    }
}

impl SynthParams {
    pub fn osc(&self, i: usize) -> &OscParams {
        match i {
            0 => &self.osc1,
            1 => &self.osc2,
            _ => &self.osc3,
        }
    }
    pub fn osc_mut(&mut self, i: usize) -> &mut OscParams {
        match i {
            0 => &mut self.osc1,
            1 => &mut self.osc2,
            _ => &mut self.osc3,
        }
    }
    pub fn lfo(&self, i: usize) -> &LfoParams {
        match i {
            0 => &self.lfo1,
            1 => &self.lfo2,
            2 => &self.lfo3,
            _ => &self.lfo4,
        }
    }
    pub fn lfo_mut(&mut self, i: usize) -> &mut LfoParams {
        match i {
            0 => &mut self.lfo1,
            1 => &mut self.lfo2,
            2 => &mut self.lfo3,
            _ => &mut self.lfo4,
        }
    }
    pub fn macros(&self) -> [f32; 4] {
        [self.macro1, self.macro2, self.macro3, self.macro4]
    }
    pub fn macro_mut(&mut self, i: usize) -> &mut f32 {
        match i {
            0 => &mut self.macro1,
            1 => &mut self.macro2,
            2 => &mut self.macro3,
            _ => &mut self.macro4,
        }
    }
    /// Adds a routing in the first free matrix slot. Returns false when the matrix is full.
    pub fn add_mod(&mut self, source: ModSource, dest: ModDest, amount: f32) -> bool {
        if let Some(s) = self.mod_matrix.iter_mut().find(|s| s.source == ModSource::Off || s.dest == ModDest::Off) {
            *s = ModSlot::new(source, dest, amount);
            true
        } else {
            false
        }
    }

    /// Factory presets (name, params). Names of the first TSynth version are kept.
    pub fn presets() -> Vec<(&'static str, SynthParams)> {
        crate::synth_presets::all().into_iter().map(|(_, n, p)| (n, p)).collect()
    }

    /// Factory presets with their browser category: (category, name, params).
    pub fn presets_by_category() -> Vec<(&'static str, &'static str, SynthParams)> {
        crate::synth_presets::all()
    }

    /// Category of a factory preset name.
    pub fn preset_category(name: &str) -> Option<&'static str> {
        crate::synth_presets::all().into_iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name)).map(|(c, _, _)| c)
    }
}

// ------------------------------------------------------------------ voice

#[inline]
fn warp_param(mode: WarpMode, a: f32) -> f32 {
    let a = a.clamp(0.0, 1.0);
    match mode {
        WarpMode::BendPlus => 1.0 + 8.0 * a,
        WarpMode::BendMinus => 1.0 / (1.0 + 8.0 * a),
        WarpMode::Sync => 1.0 + 7.0 * a,
        WarpMode::Pwm => 1.0 / (1.0 - 0.95 * a),
        WarpMode::Quantize => 2f32.powf(1.0 + 9.0 * (1.0 - a)).round(),
        _ => 0.0,
    }
}

/// Warped phase. `k` from `warp_param`, `a` = amount, `fm` = modulator sample.
#[inline]
fn warp_phase(mode: WarpMode, k: f32, a: f32, p: f32, fm: f32) -> f32 {
    match mode {
        WarpMode::Off => p,
        WarpMode::BendPlus | WarpMode::BendMinus => p * k / (1.0 + p * (k - 1.0)),
        WarpMode::Sync => {
            let q = p * k;
            q - q.floor()
        }
        WarpMode::Pwm => {
            let q = p * k;
            if q < 1.0 {
                q
            } else {
                0.0
            }
        }
        WarpMode::Quantize => (p * k).floor() / k,
        WarpMode::FmOsc1 | WarpMode::FmOsc2 | WarpMode::FmOsc3 | WarpMode::FmSub => {
            let q = p + a.clamp(0.0, 1.0) * 1.5 * fm;
            q - q.floor()
        }
    }
}

/// how much a warp widens the spectrum (for the mip choice)
#[inline]
fn warp_bandwidth(mode: WarpMode, k: f32, a: f32) -> f32 {
    match mode {
        WarpMode::Sync | WarpMode::Pwm => k,
        WarpMode::BendPlus => k.sqrt(),
        WarpMode::BendMinus => (1.0 / k).sqrt(),
        WarpMode::FmOsc1 | WarpMode::FmOsc2 | WarpMode::FmOsc3 | WarpMode::FmSub => 1.0 + 6.0 * a,
        _ => 1.0,
    }
}

#[inline]
fn start_phase(op: &OscParams, r: f32, kick: bool) -> f32 {
    if kick {
        return op.phase.rem_euclid(1.0);
    }
    let ph = op.phase + op.rand_phase * r;
    if ph > 1.0 {
        ph - ph.floor()
    } else {
        ph
    }
}

#[derive(Clone)]
struct OscState {
    osc: [Osc; MAX_UNISON],
    dt: [f32; MAX_UNISON],
    /// per unison voice (left, right) gain
    g: [(f32, f32); MAX_UNISON],
    n: usize,
    mip: usize,
    pos: f32,
    warp_k: f32,
    warp_a: f32,
    /// last raw sample of the centre voice (FM source)
    last: f32,
}

impl Default for OscState {
    fn default() -> Self {
        Self { osc: [Osc::default(); MAX_UNISON], dt: [0.0; MAX_UNISON], g: [(0.0, 0.0); MAX_UNISON], n: 1, mip: 0, pos: 0.0, warp_k: 0.0, warp_a: 0.0, last: 0.0 }
    }
}

#[derive(Clone)]
struct SynthVoice {
    active: bool,
    key: u8,
    vel: f32,
    vel_gain: f32,
    age: u64,
    pitch: f32,
    target_pitch: f32,
    oscs: [OscState; NUM_OSC],
    sub: Osc,
    sub_last: f32,
    lfo: [Osc; NUM_LFO],
    lfo_val: [f32; NUM_LFO],
    lfo_sh: [f32; NUM_LFO],
    /// rng of the original voice (start phases of the first 7 unison slots, noise)
    rng: Rng,
    /// rng for everything added later (keeps the original sequence intact)
    rng2: Rng,
    flt1: [SynthFilter; 2],
    flt2: [SynthFilter; 2],
    amp_env: Adsr,
    flt_env: Adsr,
    mod_env: Adsr,
    remaining: Option<u64>,
    held: bool,
    ctr: u32,
    // control-rate state
    dt_sub: f32,
    sub_gain: f32,
    noise_gain: f32,
    amp_mod: f32,
    pan_l: f32,
    pan_r: f32,
    flt_env_val: f32,
    /// seconds since note start (pitch envelopes)
    t_since: f32,
    random: f32,
    mods: [f32; ModDest::COUNT],
    f1_drive: f32,
    f1_norm: f32,
    f2_drive: f32,
    f2_norm: f32,
    // noise
    noise_lp: OnePole,
    noise_hold: f32,
    noise_ph: f32,
    // kick
    kick_on: bool,
    kick_drive: f32,
    kick_norm: f32,
    click_env: f32,
    click_coef: f32,
    click_ph: f32,
    click_dt: f32,
    click_gain: f32,
    click_bp: Svf,
    body_env: f32,
    body_hold: u32,
    body_coef: f32,
    rumble_ph: f32,
    rumble_dt: f32,
    rumble_rise: f32,
    rumble_env: f32,
    rumble_rel: f32,
    rumble_delay: u32,
    rumble_lp: [OnePole; 2],
    rumble_wob: OnePole,
    dc: [(f32, f32); 2],
}

impl SynthVoice {
    fn new(sr: f32, seed: u32) -> Self {
        let comb = (sr / COMB_MIN_HZ) as usize + 2;
        Self {
            active: false,
            key: 0,
            vel: 1.0,
            vel_gain: 1.0,
            age: 0,
            pitch: 60.0,
            target_pitch: 60.0,
            oscs: [OscState::default(), OscState::default(), OscState::default()],
            sub: Osc::default(),
            sub_last: 0.0,
            lfo: [Osc::default(); NUM_LFO],
            lfo_val: [0.0; NUM_LFO],
            lfo_sh: [0.0; NUM_LFO],
            rng: Rng(0x9E37_79B9 ^ seed.wrapping_mul(2_654_435_761)),
            rng2: Rng(0x85EB_CA6B ^ seed.wrapping_mul(0x27D4_EB2F)),
            flt1: [SynthFilter::new(comb), SynthFilter::new(comb)],
            flt2: [SynthFilter::new(comb), SynthFilter::new(comb)],
            amp_env: Adsr::new(sr),
            flt_env: Adsr::new(sr),
            mod_env: Adsr::new(sr),
            remaining: None,
            held: false,
            ctr: 0,
            dt_sub: 0.0,
            sub_gain: 0.0,
            noise_gain: 0.0,
            amp_mod: 1.0,
            pan_l: 1.0,
            pan_r: 1.0,
            flt_env_val: 0.0,
            t_since: 0.0,
            random: 0.0,
            mods: [0.0; ModDest::COUNT],
            f1_drive: 0.0,
            f1_norm: 1.0,
            f2_drive: 0.0,
            f2_norm: 1.0,
            noise_lp: OnePole::default(),
            noise_hold: 0.0,
            noise_ph: 0.0,
            kick_on: false,
            kick_drive: 0.0,
            kick_norm: 1.0,
            click_env: 0.0,
            click_coef: 0.0,
            click_ph: 0.0,
            click_dt: 0.0,
            click_gain: 0.0,
            click_bp: Svf::default(),
            body_env: 1.0,
            body_hold: 0,
            body_coef: 1.0,
            rumble_ph: 0.0,
            rumble_dt: 0.0,
            rumble_rise: 0.0,
            rumble_env: 1.0,
            rumble_rel: 1.0,
            rumble_delay: 0,
            rumble_lp: [OnePole::default(); 2],
            rumble_wob: OnePole::default(),
            dc: [(0.0, 0.0); 2],
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(&mut self, p: &SynthParams, sr: f32, key: u8, vel: f32, dur: Option<u64>, from_pitch: f32, age: u64) {
        self.active = true;
        self.key = key;
        self.vel = vel;
        self.vel_gain = 1.0 - p.velocity_sens + p.velocity_sens * vel;
        self.age = age;
        self.kick_on = p.kick.enabled;
        self.target_pitch = if self.kick_on { p.kick.body_note(key) } else { key as f32 };
        self.pitch = if p.glide > 0.0 && !self.kick_on && !(p.mono && p.slide_only) { from_pitch } else { self.target_pitch };
        self.remaining = dur;
        self.held = dur.is_none();
        if self.kick_on && p.amp_env.sustain <= 0.0 {
            // kicks are one-shots: the envelope plays out regardless of the note length
            self.remaining = None;
            self.held = false;
        }
        self.ctr = 0;
        self.t_since = 0.0;
        // random start phases give unison its width. The first 7 slots of osc 1 and 2
        // draw from the original generator in the original order.
        for oi in 0..2 {
            let op = p.osc(oi);
            for u in 0..LEGACY_UNISON {
                let r = self.rng.next_f32() * 0.5 + 0.5;
                self.oscs[oi].osc[u].phase = start_phase(op, r, self.kick_on);
            }
        }
        for oi in 0..NUM_OSC {
            let op = p.osc(oi);
            let from = if oi < 2 { LEGACY_UNISON } else { 0 };
            for u in from..MAX_UNISON {
                let r = self.rng2.next_f32() * 0.5 + 0.5;
                self.oscs[oi].osc[u].phase = start_phase(op, r, self.kick_on);
            }
            self.oscs[oi].last = 0.0;
        }
        self.sub.phase = 0.0;
        self.sub_last = 0.0;
        self.random = self.rng2.next_f32();
        for (i, l) in self.lfo.iter_mut().enumerate() {
            let lp = p.lfo(i);
            if lp.retrig {
                l.phase = lp.phase.rem_euclid(1.0);
                self.lfo_sh[i] = self.rng2.next_f32();
            }
        }
        for f in self.flt1.iter_mut().chain(self.flt2.iter_mut()) {
            f.reset();
        }
        self.noise_lp.z = 0.0;
        self.dc = [(0.0, 0.0); 2];
        // kick layers
        let k = &p.kick;
        self.click_env = 1.0;
        self.click_coef = (-1.0 / (k.click_decay.max(0.0005) * sr)).exp();
        self.click_ph = 0.0;
        self.click_bp.reset();
        self.click_bp.set(k.click_tone, 0.25, sr);
        self.body_env = 1.0;
        self.body_hold = (k.body_hold.max(0.0) * sr) as u32;
        self.body_coef = if k.body_decay > 0.0 { (-7.0 / (k.body_decay * sr)).exp() } else { 1.0 };
        self.rumble_ph = 0.0;
        self.rumble_rise = 0.0;
        self.rumble_env = 1.0;
        self.rumble_rel = (-7.0 / (k.rumble_decay.max(0.01) * sr)).exp();
        self.rumble_delay = (k.rumble_delay.max(0.0) * sr) as u32;
        self.rumble_lp = [OnePole::default(); 2];
        self.rumble_wob.z = 0.0;
        self.amp_env.set_params(&p.amp_env);
        self.flt_env.set_params(&p.filter_env);
        self.mod_env.set_params(&p.mod_env);
        self.amp_env.reset();
        self.flt_env.reset();
        self.mod_env.reset();
        self.amp_env.trigger();
        self.flt_env.trigger();
        self.mod_env.trigger();
    }

    fn release(&mut self) {
        self.amp_env.release();
        self.flt_env.release();
        self.mod_env.release();
        self.held = false;
        self.remaining = None;
    }

    fn control(&mut self, p: &SynthParams, sr: f32, bpm: f32) {
        let ctl_dt = CONTROL_RATE as f32 / sr;
        // ---- LFOs (advance by one control period)
        let mut pitch_mod = 0.0;
        let mut cutoff_mod = 0.0;
        let mut amp_mod = 1.0;
        let mut pan_mod = 0.0;
        const RATE_DEST: [ModDest; NUM_LFO] = [ModDest::Lfo1Rate, ModDest::Lfo2Rate, ModDest::Lfo3Rate, ModDest::Lfo4Rate];
        for i in 0..NUM_LFO {
            let lp = p.lfo(i);
            let rm = self.mods[RATE_DEST[i] as usize];
            let inc = if !lp.sync && rm == 0.0 {
                // original expression (bit-exact for old projects)
                lp.rate * CONTROL_RATE as f32 / sr
            } else {
                lp.hz(bpm) * 2f32.powf(rm * 4.0) * ctl_dt
            };
            let v = match lp.shape {
                LfoShape::Wave => self.lfo[i].next(lp.wave, inc),
                shape => {
                    let ph = self.lfo[i].phase;
                    let mut np = ph + inc.clamp(0.0, 0.49);
                    if np >= 1.0 {
                        np -= 1.0;
                        self.lfo_sh[i] = self.rng2.next_f32();
                    }
                    self.lfo[i].phase = np;
                    if shape == LfoShape::SampleHold {
                        self.lfo_sh[i]
                    } else {
                        lp.value_at(ph)
                    }
                }
            };
            self.lfo_val[i] = v;
            match lp.target {
                LfoTarget::Off => {}
                LfoTarget::Pitch => pitch_mod += v * lp.amount * 12.0,
                LfoTarget::Cutoff => cutoff_mod += v * lp.amount * 5.0,
                LfoTarget::Amp => amp_mod *= 1.0 - lp.amount * (0.5 - 0.5 * v),
                LfoTarget::Pan => pan_mod += v * lp.amount,
            }
        }

        // ---- mod matrix
        let sweep = if self.kick_on { p.kick.sweep(self.t_since) } else { 0.0 };
        if p.mod_matrix.iter().any(|s| s.active()) {
            let mut m = [0.0f32; ModDest::COUNT];
            for s in p.mod_matrix.iter().filter(|s| s.active() && !s.dest.is_global()) {
                let v = match s.source {
                    ModSource::Off => 0.0,
                    ModSource::AmpEnv => self.amp_env.level,
                    ModSource::FilterEnv => self.flt_env.level,
                    ModSource::ModEnv => self.mod_env.level,
                    ModSource::Lfo1 => self.lfo_val[0],
                    ModSource::Lfo2 => self.lfo_val[1],
                    ModSource::Lfo3 => self.lfo_val[2],
                    ModSource::Lfo4 => self.lfo_val[3],
                    ModSource::Velocity => self.vel,
                    ModSource::Note => ((self.pitch - 60.0) / 48.0).clamp(-1.5, 1.5),
                    ModSource::Random => self.random,
                    ModSource::Macro1 => p.macro1,
                    ModSource::Macro2 => p.macro2,
                    ModSource::Macro3 => p.macro3,
                    ModSource::Macro4 => p.macro4,
                    ModSource::KickSweep => sweep,
                };
                m[s.dest as usize] += v * s.amount;
            }
            self.mods = m;
        } else if self.mods.iter().any(|v| *v != 0.0) {
            self.mods = [0.0; ModDest::COUNT];
        }
        let m = self.mods;
        let md = |d: ModDest| m[d as usize];

        let amp_m = md(ModDest::Amp);
        if amp_m != 0.0 {
            amp_mod *= (1.0 + amp_m).max(0.0);
        }
        self.amp_mod = amp_mod;
        let pan_m = md(ModDest::Pan);
        let (pl, pr) = crate::pan_gains(if pan_m != 0.0 { pan_mod + pan_m } else { pan_mod });
        self.pan_l = pl;
        self.pan_r = pr;

        // ---- glide
        if p.glide > 0.0 && !self.kick_on {
            let coef = 1.0 - (-(CONTROL_RATE as f32) / (p.glide * sr / 4.0)).exp();
            self.pitch += (self.target_pitch - self.pitch) * coef;
        } else {
            self.pitch = self.target_pitch;
        }

        // ---- pitch
        let pitch_env = if p.pitch_env_amount != 0.0 { p.pitch_env_amount * (-self.t_since / p.pitch_env_decay.max(0.001)).exp() } else { 0.0 };
        let kick_env = if self.kick_on { (p.kick.pitch_start + md(ModDest::KickPitch) * 48.0) * sweep } else { 0.0 };
        self.t_since += ctl_dt;
        let mut base = self.pitch + pitch_mod + pitch_env;
        let gp = md(ModDest::Pitch) * 24.0;
        if kick_env != 0.0 || gp != 0.0 {
            base += kick_env + gp;
        }
        const OSC_PITCH: [ModDest; 3] = [ModDest::Osc1Pitch, ModDest::Osc2Pitch, ModDest::Osc3Pitch];
        const OSC_POS: [ModDest; 3] = [ModDest::Osc1Pos, ModDest::Osc2Pos, ModDest::Osc3Pos];
        const OSC_WARP: [ModDest; 3] = [ModDest::Osc1Warp, ModDest::Osc2Warp, ModDest::Osc3Warp];
        const OSC_LVL: [ModDest; 3] = [ModDest::Osc1Level, ModDest::Osc2Level, ModDest::Osc3Level];
        let det_m = md(ModDest::Detune);
        for (oi, st) in self.oscs.iter_mut().enumerate() {
            let osc = p.osc(oi);
            if !osc.enabled {
                continue;
            }
            let n = osc.unison.clamp(1, MAX_UNISON as u32) as usize;
            st.n = n;
            let op = md(OSC_PITCH[oi]) * 24.0;
            let mut k = base + (osc.octave * 12 + osc.semi) as f32 + osc.fine / 100.0;
            if op != 0.0 {
                k += op;
            }
            let detune = if det_m != 0.0 { osc.detune * (1.0 + det_m).max(0.0) } else { osc.detune };
            for (u, dt) in st.dt.iter_mut().enumerate().take(n) {
                let det = if n > 1 { detune / 100.0 * (u as f32 / (n - 1) as f32 - 0.5) } else { 0.0 };
                *dt = midi_to_freq(k + det) / sr;
            }
            st.pos = (osc.position + md(OSC_POS[oi])).clamp(0.0, 1.0);
            st.warp_a = (osc.warp_amount + md(OSC_WARP[oi])).clamp(0.0, 1.0);
            st.warp_k = warp_param(osc.warp, st.warp_a);
            let max_dt = st.dt[..n].iter().fold(0.0f32, |a, b| a.max(*b));
            st.mip = Wavetable::mip_for(max_dt * warp_bandwidth(osc.warp, st.warp_k, st.warp_a));
            // unison gains (blend: centre voices at 1, side voices at `blend`)
            let lm = md(OSC_LVL[oi]);
            let level = if lm != 0.0 { (osc.level + lm).max(0.0) } else { osc.level };
            let blend = osc.blend.clamp(0.0, 1.0);
            let centres = if n % 2 == 1 { 1usize } else { 2 };
            let norm = if blend == 1.0 { level / (n as f32).sqrt() } else { level / (centres as f32 + (n - centres) as f32 * blend * blend).sqrt() };
            for u in 0..n {
                let mut pan = if n > 1 { osc.spread * (2.0 * u as f32 / (n - 1) as f32 - 1.0) } else { 0.0 };
                if osc.pan != 0.0 {
                    pan += osc.pan;
                }
                let (a, b) = crate::pan_gains(pan);
                let centre = n == 1 || (n % 2 == 1 && u == n / 2) || (n % 2 == 0 && (u == n / 2 || u + 1 == n / 2));
                let w = if centre { 1.0 } else { blend };
                st.g[u] = (a * norm * w, b * norm * w);
            }
        }
        self.dt_sub = midi_to_freq(base + (p.osc1.octave * 12 + p.osc1.semi) as f32 + (p.sub_octave * 12) as f32) / sr;
        let sm = md(ModDest::SubLevel);
        self.sub_gain = if sm != 0.0 { (p.sub_level + sm).max(0.0) } else { p.sub_level };
        let nm = md(ModDest::NoiseLevel);
        self.noise_gain = if nm != 0.0 { (p.noise_level + nm).max(0.0) } else { p.noise_level };
        if p.noise_tone < 19000.0 {
            self.noise_lp.set(p.noise_tone, sr);
        }

        // ---- kick
        if self.kick_on {
            let k = &p.kick;
            let d = k.drive_db + md(ModDest::KickDrive) * 24.0;
            self.kick_drive = if d > 0.0 { crate::db_to_gain(d) } else { 0.0 };
            self.kick_norm = k.drive_mode.norm(self.kick_drive);
            self.click_gain = (k.click_level + md(ModDest::ClickLevel)).max(0.0);
            let chirp = if k.click_type == ClickType::Snap { 1.0 + 3.0 * self.click_env } else { 1.0 };
            self.click_dt = k.click_tone * chirp / sr;
            self.rumble_dt = midi_to_freq(self.target_pitch) / sr;
            self.rumble_lp[0].set(k.rumble_tone, sr);
            self.rumble_lp[1].set(k.rumble_tone, sr);
            self.rumble_wob.set(18.0, sr);
        }

        // ---- filters
        let key_oct = (self.pitch - 60.0) / 12.0 * p.key_track;
        let mut oct = key_oct + self.flt_env_val * p.filter_env_amount + cutoff_mod;
        let c1 = md(ModDest::Cutoff1);
        if c1 != 0.0 {
            oct += c1 * 6.0;
        }
        let fc = p.cutoff * 2f32.powf(oct);
        let r1 = md(ModDest::Reso1);
        let res1 = if r1 != 0.0 { (p.resonance + r1).clamp(0.0, 1.0) } else { p.resonance };
        self.flt1[0].set(p.filter_type, fc, res1, sr);
        self.flt1[1].set(p.filter_type, fc, res1, sr);
        let fdm = md(ModDest::FilterDrive) * 24.0;
        let fd1 = p.filter_drive + fdm;
        self.f1_drive = if fd1 > 0.0 { crate::db_to_gain(fd1) } else { 0.0 };
        self.f1_norm = 1.0 / self.f1_drive.max(1e-3).tanh();
        if p.filter2.enabled {
            let f2 = &p.filter2;
            let oct2 = (self.pitch - 60.0) / 12.0 * f2.key_track + self.flt_env_val * f2.env_amount + cutoff_mod + md(ModDest::Cutoff2) * 6.0;
            let fc2 = f2.cutoff * 2f32.powf(oct2);
            let res2 = (f2.resonance + md(ModDest::Reso2)).clamp(0.0, 1.0);
            self.flt2[0].set(f2.kind, fc2, res2, sr);
            self.flt2[1].set(f2.kind, fc2, res2, sr);
            let fd2 = f2.drive_db + fdm;
            self.f2_drive = if fd2 > 0.0 { crate::db_to_gain(fd2) } else { 0.0 };
            self.f2_norm = 1.0 / self.f2_drive.max(1e-3).tanh();
        }
    }

    fn render(&mut self, p: &SynthParams, sr: f32, bpm: f32, l: &mut [f32], r: &mut [f32]) {
        self.amp_env.set_params(&p.amp_env);
        self.flt_env.set_params(&p.filter_env);
        self.mod_env.set_params(&p.mod_env);
        let drive = if p.drive_db > 0.0 { crate::db_to_gain(p.drive_db) } else { 0.0 };
        let classic_drive = p.drive_mode == DriveMode::Soft;
        // classic expression kept for old projects
        let drive_norm = if drive > 0.0 {
            if classic_drive {
                1.0 / drive.tanh()
            } else {
                p.drive_mode.norm(drive)
            }
        } else {
            1.0
        };
        let tables = [wavetable::table(p.osc1.table), wavetable::table(p.osc2.table), wavetable::table(p.osc3.table)];
        let fm_used = [p.osc1.warp.fm_source(), p.osc2.warp.fm_source(), p.osc3.warp.fm_source()];
        let sub_needed = fm_used.contains(&Some(3));
        let k = &p.kick;
        let f2_on = p.filter2.enabled;

        for i in 0..l.len() {
            if self.ctr % CONTROL_RATE == 0 {
                self.control(p, sr, bpm);
            }
            self.ctr = self.ctr.wrapping_add(1);

            let mut sl = 0.0;
            let mut sr_ = 0.0;
            for oi in 0..NUM_OSC {
                let op = p.osc(oi);
                if !op.enabled {
                    continue;
                }
                let fm = match fm_used[oi] {
                    Some(3) => self.sub_last,
                    Some(j) => self.oscs[j].last,
                    None => 0.0,
                };
                let st = &mut self.oscs[oi];
                let n = st.n;
                let centre = n / 2;
                let classic = op.table == WtKind::Basic && op.warp == WarpMode::Off;
                let tbl = tables[oi];
                let frame = op.basic_frame();
                for u in 0..n {
                    let s = if classic {
                        st.osc[u].next(op.wave, st.dt[u])
                    } else {
                        let o = &mut st.osc[u];
                        let q = warp_phase(op.warp, st.warp_k, st.warp_a, o.phase, fm);
                        let v = if op.table == WtKind::Basic { tbl.sample_frame(frame, q, st.mip) } else { tbl.sample(st.pos, q, st.mip) };
                        o.phase += st.dt[u].clamp(0.0, 0.49);
                        if o.phase >= 1.0 {
                            o.phase -= 1.0;
                        }
                        v
                    };
                    if u == centre {
                        st.last = s;
                    }
                    sl += s * st.g[u].0;
                    sr_ += s * st.g[u].1;
                }
            }
            if self.sub_gain > 0.0 || sub_needed {
                let raw = self.sub.next(p.sub_wave, self.dt_sub);
                self.sub_last = raw;
                let s = raw * self.sub_gain;
                sl += s;
                sr_ += s;
            }
            // kick: without its own body envelope the amp envelope shapes the body *before* the
            // drive, so the distortion holds the decaying tail up (fat, long bass) instead of the
            // envelope fading a constant distorted tone
            let amp_v = self.amp_env.next();
            let env_pre = self.kick_on && k.body_decay <= 0.0;
            if self.kick_on && k.body_decay > 0.0 {
                if self.body_hold > 0 {
                    self.body_hold -= 1;
                } else {
                    self.body_env *= self.body_coef;
                }
                sl *= self.body_env;
                sr_ *= self.body_env;
            }
            if self.noise_gain > 0.0 {
                let raw = match p.noise_type {
                    NoiseType::White => self.rng.next_f32(),
                    NoiseType::Crackle => {
                        let v = self.rng.next_f32();
                        if v.abs() > 0.97 {
                            v.signum()
                        } else {
                            0.0
                        }
                    }
                    NoiseType::Digital => {
                        self.noise_ph += self.oscs[0].dt[0] * 4.0;
                        if self.noise_ph >= 1.0 {
                            self.noise_ph -= self.noise_ph.floor();
                            self.noise_hold = self.rng.next_f32();
                        }
                        self.noise_hold
                    }
                };
                let raw = if p.noise_tone < 19000.0 { self.noise_lp.process(raw) } else { raw };
                let s = raw * self.noise_gain;
                sl += s;
                sr_ += s;
            }
            if self.kick_on {
                if env_pre {
                    sl *= amp_v;
                    sr_ *= amp_v;
                }
                // click / transient
                if self.click_gain > 0.0 && self.click_env > 1e-4 {
                    let c = match k.click_type {
                        ClickType::Noise => self.click_bp.process(self.rng2.next_f32(), FilterMode::BandPass) * 2.5,
                        ClickType::Tick | ClickType::Snap => {
                            self.click_ph += self.click_dt;
                            if self.click_ph >= 1.0 {
                                self.click_ph -= 1.0;
                            }
                            (TAU * self.click_ph).sin()
                        }
                    };
                    let s = c * self.click_env * self.click_gain;
                    self.click_env *= self.click_coef;
                    sl += s;
                    sr_ += s;
                }
                // drive
                if self.kick_drive > 0.0 {
                    sl = k.drive_mode.apply(sl, self.kick_drive, self.kick_norm);
                    sr_ = k.drive_mode.apply(sr_, self.kick_drive, self.kick_norm);
                    if k.drive_mode == DriveMode::Asym {
                        for (x, d) in [(&mut sl, 0usize), (&mut sr_, 1)] {
                            let (x1, y1) = self.dc[d];
                            let y = *x - x1 + 0.995 * y1;
                            self.dc[d] = (*x, y);
                            *x = y;
                        }
                    }
                }
                // rumble: delayed, wobbling, saturated drone at the body pitch
                if k.rumble_level > 0.0 {
                    if self.rumble_delay > 0 {
                        self.rumble_delay -= 1;
                    } else {
                        self.rumble_rise += (1.0 - self.rumble_rise) * 0.003;
                        self.rumble_env *= self.rumble_rel;
                    }
                    self.rumble_ph += self.rumble_dt;
                    if self.rumble_ph >= 1.0 {
                        self.rumble_ph -= 1.0;
                    }
                    let wob = self.rumble_wob.process(self.rng2.next_f32());
                    let x = (TAU * self.rumble_ph).sin() * (1.0 + 12.0 * wob) * self.rumble_rise * self.rumble_env * k.rumble_level;
                    let y0 = self.rumble_lp[0].process((x * 4.0).tanh());
                    let y = self.rumble_lp[1].process(y0);
                    sl += y * 1.5;
                    sr_ += y * 1.5;
                }
            }
            // filters
            let (mut fl, mut fr) = if p.filter_enabled {
                let (il, ir) = if self.f1_drive > 0.0 { ((sl * self.f1_drive).tanh() * self.f1_norm, (sr_ * self.f1_drive).tanh() * self.f1_norm) } else { (sl, sr_) };
                (self.flt1[0].process(il, p.filter_type, p.filter_mode), self.flt1[1].process(ir, p.filter_type, p.filter_mode))
            } else {
                (sl, sr_)
            };
            if f2_on {
                let f2 = &p.filter2;
                let parallel = p.filter_routing == FilterRouting::Parallel && p.filter_enabled;
                let (il, ir) = if parallel { (sl, sr_) } else { (fl, fr) };
                let (il, ir) = if self.f2_drive > 0.0 { ((il * self.f2_drive).tanh() * self.f2_norm, (ir * self.f2_drive).tanh() * self.f2_norm) } else { (il, ir) };
                let ol = self.flt2[0].process(il, f2.kind, f2.mode);
                let or = self.flt2[1].process(ir, f2.kind, f2.mode);
                if parallel {
                    fl += ol;
                    fr += or;
                } else {
                    fl = ol;
                    fr = or;
                }
            }
            if drive > 0.0 {
                if classic_drive {
                    fl = (fl * drive).tanh() * drive_norm;
                    fr = (fr * drive).tanh() * drive_norm;
                } else {
                    fl = p.drive_mode.apply(fl, drive, drive_norm);
                    fr = p.drive_mode.apply(fr, drive, drive_norm);
                }
            }
            self.flt_env_val = self.flt_env.next();
            self.mod_env.next();
            let a = if env_pre { 1.0 } else { amp_v } * self.vel_gain * self.amp_mod * p.gain;
            l[i] += fl * a * self.pan_l;
            r[i] += fr * a * self.pan_r;

            if let Some(rem) = self.remaining.as_mut() {
                if *rem == 0 {
                    self.release();
                } else {
                    *rem -= 1;
                }
            }
            if !self.amp_env.is_active() {
                self.active = false;
                return;
            }
        }
    }
}

// ------------------------------------------------------------------ synth

struct FxChain {
    dist: DistFx,
    comp: CompFx,
    ott: OttFx,
    chorus: ChorusFx,
    delay: DelayFx,
    reverb: Box<ReverbFx>,
    clip: ClipperFx,
    l: Vec<f32>,
    r: Vec<f32>,
}

/// Voice pool for one synth channel.
pub struct Synth {
    voices: Vec<SynthVoice>,
    sr: f32,
    counter: u64,
    last_pitch: f32,
    /// extra gain for the next started notes (playlist-track volume)
    pub note_gain: f32,
    /// project tempo (tempo-synced LFOs and delay)
    pub bpm: f32,
    fx: FxChain,
    /// samples the FX chain keeps running after the last voice ended
    tail: usize,
}

impl Synth {
    pub fn new(sr: f32, polyphony: usize) -> Self {
        // build the wavetables here, never on the audio thread
        let _ = wavetable::bank();
        Self {
            voices: (0..polyphony.max(1)).map(|i| SynthVoice::new(sr, i as u32 + 1)).collect(),
            sr,
            counter: 0,
            last_pitch: 60.0,
            note_gain: 1.0,
            bpm: 120.0,
            fx: FxChain {
                dist: DistFx::default(),
                comp: CompFx::default(),
                ott: OttFx::default(),
                chorus: ChorusFx::new(sr),
                delay: DelayFx::new(sr),
                reverb: Box::new(ReverbFx::new(sr)),
                clip: ClipperFx::default(),
                l: vec![0.0; FX_BLOCK],
                r: vec![0.0; FX_BLOCK],
            },
            tail: 0,
        }
    }

    /// `dur`: length in samples after which the note releases (None = held until `note_off`).
    pub fn note_on(&mut self, p: &SynthParams, key: u8, vel: f32, dur: Option<u64>) {
        self.counter += 1;
        let from = self.last_pitch;
        self.last_pitch = key as f32;
        let sr = self.sr;
        if p.mono {
            let v = &mut self.voices[0];
            if p.legato && v.active && v.amp_env.is_active() && v.amp_env.stage != crate::env::Stage::Release {
                // legato: glide to the new note without retriggering
                v.key = key;
                v.target_pitch = if v.kick_on { p.kick.body_note(key) } else { key as f32 };
                if p.glide <= 0.0 {
                    v.pitch = v.target_pitch;
                }
                v.remaining = dur;
                v.held = dur.is_none();
                return;
            }
            v.start(p, sr, key, vel, dur, from, self.counter);
            v.vel_gain *= self.note_gain;
            return;
        }
        let poly = (p.polyphony.max(1) as usize).min(self.voices.len());
        let pool = &self.voices[..poly];
        let idx = pool
            .iter()
            .position(|v| !v.active)
            .or_else(|| {
                pool.iter()
                    .enumerate()
                    .filter(|(_, v)| v.amp_env.stage == crate::env::Stage::Release)
                    .min_by(|a, b| a.1.amp_env.level.total_cmp(&b.1.amp_env.level))
                    .map(|(i, _)| i)
            })
            .unwrap_or_else(|| pool.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        // voices beyond a lowered polyphony fade out on their own
        let counter = self.counter;
        self.voices[idx].start(p, sr, key, vel, dur, from, counter);
        self.voices[idx].vel_gain *= self.note_gain;
    }

    pub fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.active && v.held && v.key == key) {
            v.release();
        }
    }

    pub fn release_all(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.release();
        }
    }

    pub fn kill_all(&mut self) {
        for v in self.voices.iter_mut() {
            v.active = false;
            v.amp_env.reset();
        }
        self.tail = 0;
        self.fx.delay.reset();
        self.fx.reverb.reset();
        self.fx.chorus.reset();
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Voices or an FX tail still produce sound.
    pub fn is_sounding(&self) -> bool {
        self.tail > 0 || self.active_voices() > 0
    }

    /// Adds the output to `l`/`r`.
    pub fn render(&mut self, p: &SynthParams, l: &mut [f32], r: &mut [f32]) {
        let sr = self.sr;
        let bpm = self.bpm;
        if !p.fx.any_on() {
            self.tail = 0;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.render(p, sr, bpm, l, r);
            }
            return;
        }
        // macro → global FX destinations (stack copies, no allocation)
        let macros = p.macros();
        let mut g = [0.0f32; ModDest::COUNT];
        for s in p.mod_matrix.iter().filter(|s| s.active() && s.dest.is_global() && s.source.is_macro()) {
            let v = match s.source {
                ModSource::Macro1 => macros[0],
                ModSource::Macro2 => macros[1],
                ModSource::Macro3 => macros[2],
                _ => macros[3],
            };
            g[s.dest as usize] += v * s.amount;
        }
        let mut dist = p.fx.dist.clone();
        dist.drive_db += g[ModDest::FxDistDrive as usize] * 24.0;
        let mut rev = p.fx.reverb.clone();
        rev.mix = (rev.mix + g[ModDest::FxReverbMix as usize]).clamp(0.0, 1.0);
        let mut del = p.fx.delay.clone();
        del.mix = (del.mix + g[ModDest::FxDelayMix as usize]).clamp(0.0, 1.0);
        let mut cho = p.fx.chorus.clone();
        cho.mix = (cho.mix + g[ModDest::FxChorusMix as usize]).clamp(0.0, 1.0);
        let ctx = FxContext { sr, bpm, key: &[] };
        let tail_len = (p.fx.tail_secs() * sr) as usize;

        let mut done = 0;
        while done < l.len() {
            let n = (l.len() - done).min(FX_BLOCK);
            let fx = &mut self.fx;
            let (bl, br) = (&mut fx.l[..n], &mut fx.r[..n]);
            bl.fill(0.0);
            br.fill(0.0);
            let mut any = false;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.render(p, sr, bpm, bl, br);
                any = true;
            }
            if any {
                self.tail = tail_len;
            } else {
                self.tail = self.tail.saturating_sub(n);
            }
            if p.fx.dist_on {
                fx.dist.process(&dist, &ctx, bl, br);
            }
            if p.fx.comp_on {
                fx.comp.process(&p.fx.comp, &ctx, bl, br);
            }
            if p.fx.ott_on {
                fx.ott.process(&p.fx.ott, &ctx, bl, br);
            }
            if p.fx.chorus_on {
                fx.chorus.process(&cho, &ctx, bl, br);
            }
            if p.fx.delay_on {
                fx.delay.process(&del, &ctx, bl, br);
            }
            if p.fx.reverb_on {
                fx.reverb.process(&rev, &ctx, bl, br);
            }
            if p.fx.clip_on {
                fx.clip.process(&p.fx.clip, &ctx, bl, br);
            }
            for k in 0..n {
                // flush denormals FX tails may leave behind
                let (a, b) = (bl[k], br[k]);
                l[done + k] += if a.abs() < 1e-30 { 0.0 } else { a };
                r[done + k] += if b.abs() < 1e-30 { 0.0 } else { b };
            }
            done += n;
        }
    }
}

#[cfg(test)]
#[path = "synth_tests.rs"]
mod tests;
