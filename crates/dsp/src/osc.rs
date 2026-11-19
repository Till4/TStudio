use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Waveform {
    Sine,
    #[default]
    Saw,
    Square,
    Triangle,
}

impl Waveform {
    pub const ALL: [Waveform; 4] = [Waveform::Sine, Waveform::Saw, Waveform::Square, Waveform::Triangle];
    pub fn name(self) -> &'static str {
        match self {
            Waveform::Sine => "Sine",
            Waveform::Saw => "Saw",
            Waveform::Square => "Square",
            Waveform::Triangle => "Triangle",
        }
    }
}

#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Band-limited (PolyBLEP) oscillator. `phase` in 0..1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Osc {
    pub phase: f32,
}

impl Osc {
    /// `dt` = freq / sample_rate
    #[inline]
    pub fn next(&mut self, wf: Waveform, dt: f32) -> f32 {
        let dt = dt.clamp(0.0, 0.49);
        let t = self.phase;
        let out = match wf {
            Waveform::Sine => (t * TAU).sin(),
            Waveform::Saw => (2.0 * t - 1.0) - poly_blep(t, dt),
            Waveform::Square => {
                let mut v = if t < 0.5 { 1.0 } else { -1.0 };
                v += poly_blep(t, dt);
                let t2 = (t + 0.5) % 1.0;
                v -= poly_blep(t2, dt);
                v
            }
            Waveform::Triangle => {
                if t < 0.5 {
                    4.0 * t - 1.0
                } else {
                    3.0 - 4.0 * t
                }
            }
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        out
    }
}
