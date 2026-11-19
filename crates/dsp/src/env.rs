use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AdsrParams {
    /// seconds
    pub attack: f32,
    /// seconds
    pub decay: f32,
    /// 0..1
    pub sustain: f32,
    /// seconds
    pub release: f32,
    /// seconds at full level between attack and decay (punchy kicks)
    pub hold: f32,
    /// -1..1: 0 = linear attack, >0 = fast/convex rise, <0 = slow/concave rise
    pub attack_curve: f32,
    /// -1..1: shapes decay and release. 0 = classic exponential, >0 = snappier, <0 = rounder
    pub decay_curve: f32,
}

impl Default for AdsrParams {
    fn default() -> Self {
        Self { attack: 0.002, decay: 0.2, sustain: 1.0, release: 0.05, hold: 0.0, attack_curve: 0.0, decay_curve: 0.0 }
    }
}

impl AdsrParams {
    pub const fn adsr(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self { attack, decay, sustain, release, hold: 0.0, attack_curve: 0.0, decay_curve: 0.0 }
    }

    /// Envelope value at `t` seconds for a note held `gate` seconds (for drawing).
    pub fn value_at(&self, t: f32, gate: f32) -> f32 {
        let sr = 2000.0;
        let mut e = Adsr::new(sr);
        e.set_params(self);
        e.trigger();
        let n = (t * sr) as usize;
        let g = (gate * sr) as usize;
        let mut v = 0.0;
        for i in 0..=n {
            if i == g {
                e.release();
            }
            v = e.next();
        }
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Debug)]
pub struct Adsr {
    pub stage: Stage,
    /// current output level
    pub level: f32,
    sr: f32,
    params: AdsrParams,
    attack_inc: f32,
    decay_coef: f32,
    release_coef: f32,
    /// progress variables (linear attack ramp / exponential decay & release distance)
    x: f32,
    hold_left: u32,
    rel_start: f32,
    a_exp: f32,
    d_exp: f32,
}

impl Adsr {
    pub fn new(sr: f32) -> Self {
        let mut a = Self {
            stage: Stage::Idle,
            level: 0.0,
            sr,
            params: AdsrParams::default(),
            attack_inc: 1.0,
            decay_coef: 0.0,
            release_coef: 0.0,
            x: 0.0,
            hold_left: 0,
            rel_start: 0.0,
            a_exp: 1.0,
            d_exp: 1.0,
        };
        a.recompute();
        a
    }

    fn recompute(&mut self) {
        let p = self.params;
        self.attack_inc = 1.0 / (p.attack.max(0.0005) * self.sr);
        // exponential segments: reach ~ -60 dB after roughly the given time
        self.decay_coef = (-7.0 / (p.decay.max(0.001) * self.sr)).exp();
        self.release_coef = (-7.0 / (p.release.max(0.001) * self.sr)).exp();
        self.a_exp = 2f32.powf(-3.0 * p.attack_curve.clamp(-1.0, 1.0));
        self.d_exp = 2f32.powf(2.0 * p.decay_curve.clamp(-1.0, 1.0));
    }

    pub fn set_params(&mut self, p: &AdsrParams) {
        if *p != self.params {
            self.params = *p;
            self.recompute();
        }
    }

    pub fn trigger(&mut self) {
        self.stage = Stage::Attack;
        // continue from the current level without a jump
        self.x = if self.a_exp == 1.0 { self.level } else { self.level.max(0.0).powf(1.0 / self.a_exp) };
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
            self.rel_start = self.level;
            self.x = 1.0;
        }
    }

    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
        self.x = 0.0;
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    #[inline]
    fn shape_d(&self, d: f32) -> f32 {
        if self.d_exp == 1.0 {
            d
        } else {
            d.powf(self.d_exp)
        }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.x += self.attack_inc;
                if self.x >= 1.0 {
                    self.x = 1.0;
                    self.level = 1.0;
                    let h = (self.params.hold.max(0.0) * self.sr) as u32;
                    if h > 0 {
                        self.hold_left = h;
                        self.stage = Stage::Hold;
                    } else {
                        self.stage = Stage::Decay;
                    }
                } else {
                    self.level = if self.a_exp == 1.0 { self.x } else { self.x.powf(self.a_exp) };
                }
            }
            Stage::Hold => {
                self.level = 1.0;
                self.hold_left = self.hold_left.saturating_sub(1);
                if self.hold_left == 0 {
                    self.stage = Stage::Decay;
                    self.x = 1.0;
                }
            }
            Stage::Decay => {
                let s = self.params.sustain;
                if self.d_exp == 1.0 {
                    // classic path (kept bit-exact for old projects)
                    self.level = s + (self.level - s) * self.decay_coef;
                    if (self.level - s).abs() < 1e-4 {
                        self.level = s;
                        self.stage = if s <= 1e-4 { Stage::Idle } else { Stage::Sustain };
                    }
                } else {
                    self.x *= self.decay_coef;
                    let d = (1.0 - s) * self.shape_d(self.x);
                    self.level = s + d;
                    if d.abs() < 1e-4 {
                        self.level = s;
                        self.stage = if s <= 1e-4 { Stage::Idle } else { Stage::Sustain };
                    }
                }
            }
            Stage::Sustain => self.level = self.params.sustain,
            Stage::Release => {
                if self.d_exp == 1.0 {
                    self.level *= self.release_coef;
                } else {
                    self.x *= self.release_coef;
                    self.level = self.rel_start * self.shape_d(self.x);
                }
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_and_curves_behave() {
        let sr = 48000.0;
        let mut e = Adsr::new(sr);
        e.set_params(&AdsrParams { attack: 0.001, hold: 0.05, decay: 0.1, sustain: 0.0, release: 0.05, ..Default::default() });
        e.trigger();
        let v: Vec<f32> = (0..(0.3 * sr) as usize).map(|_| e.next()).collect();
        // still at full level during the hold
        assert!((v[(0.04 * sr) as usize] - 1.0).abs() < 1e-6);
        assert!(v.iter().all(|x| x.is_finite() && *x >= 0.0 && *x <= 1.0));
        assert!(!e.is_active(), "decays to idle");
        // snappier curve is lower in the middle of the decay
        let mid = |c: f32| AdsrParams { attack: 0.0005, decay: 0.2, sustain: 0.0, decay_curve: c, ..Default::default() }.value_at(0.05, 1.0);
        assert!(mid(0.8) < mid(0.0) && mid(0.0) < mid(-0.8));
    }
}
