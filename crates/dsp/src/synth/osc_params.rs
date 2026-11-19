//! Oscillator parameters: phase warp modes, per-oscillator settings, noise types.

use super::*;

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
    pub(super) fn fm_source(self) -> Option<usize> {
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
    pub(super) fn basic_frame(&self) -> usize {
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
