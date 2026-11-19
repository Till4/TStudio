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

mod kick;
mod lfo;
mod modmatrix;
mod osc_params;
mod params;
mod voice;

pub use kick::*;
pub use lfo::*;
pub use modmatrix::*;
pub use osc_params::*;
pub use params::*;
// `Synth` (below) is the public entry point; voices stay internal.
use voice::*;

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
mod tests;
