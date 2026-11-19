//! Kick-drum engine parameters (click, drive, pitch sweep).

use super::*;

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
    pub(super) fn norm(self, g: f32) -> f32 {
        match self {
            DriveMode::Asym => 1.0 / ((g + 0.35).tanh() - ASYM_OFS).max(1e-3),
            _ => 1.0 / g.max(1e-3).tanh(),
        }
    }
    #[inline]
    pub(super) fn apply(self, x: f32, g: f32, norm: f32) -> f32 {
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
