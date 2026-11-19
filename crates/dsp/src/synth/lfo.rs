//! LFO parameters, shapes and targets.

use super::*;

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
