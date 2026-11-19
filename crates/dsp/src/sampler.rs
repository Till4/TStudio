//! Sample playback instrument with pitch tracking, looping, reverse and slice (chop) mode.

use crate::env::{Adsr, AdsrParams};
use crate::sample::{read_cubic, SampleData};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SamplerMode {
    /// Whole sample (start..end), pitched by key relative to `root_note`.
    #[default]
    Normal,
    /// Each key from `slice_root` upward plays one slice at original pitch.
    Chop,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SamplerParams {
    pub root_note: u8,
    /// 0..1 of sample length
    pub start: f32,
    /// 0..1 of sample length
    pub end: f32,
    pub reverse: bool,
    pub looped: bool,
    /// Loop region start, 0..1 of sample length (clamped into start..end). Playback runs from `start`
    /// into the loop and then repeats `loop_start..loop_end` while the note is held.
    pub loop_start: f32,
    /// Loop region end, 0..1 of sample length.
    pub loop_end: f32,
    /// Crossfade at the loop seam, 0..0.5 of the loop length (hides clicks).
    pub loop_xfade: f32,
    /// After note release: leave the loop and play the rest of the sample (natural tail).
    pub loop_tail: bool,
    /// Play to the end regardless of note length (classic drum-sample behaviour).
    pub one_shot: bool,
    /// semitones (fractional = fine tune)
    pub pitch: f32,
    pub amp_env: AdsrParams,
    pub gain: f32,
    pub mode: SamplerMode,
    /// Slice start positions, 0..1, sorted.
    pub slices: Vec<f32>,
    pub slice_root: u8,
    /// New notes cut previous ones (hi-hat choke / chop style).
    pub mono: bool,
    pub velocity_sens: f32,
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self {
            root_note: 60,
            start: 0.0,
            end: 1.0,
            reverse: false,
            looped: false,
            loop_start: 0.0,
            loop_end: 1.0,
            loop_xfade: 0.0,
            loop_tail: false,
            one_shot: true,
            pitch: 0.0,
            amp_env: AdsrParams::adsr(0.001, 0.1, 1.0, 0.03),
            gain: 1.0,
            mode: SamplerMode::Normal,
            slices: Vec::new(),
            slice_root: 60,
            mono: false,
            velocity_sens: 0.7,
        }
    }
}

impl SamplerParams {
    /// Sample-index range `[start, end)` that key `key` plays, or None if the key is unmapped.
    pub fn region_for_key(&self, key: u8, len: usize) -> Option<(f64, f64)> {
        let len_f = len as f64;
        let (s, e) = match self.mode {
            SamplerMode::Normal => (self.start as f64, self.end as f64),
            SamplerMode::Chop => {
                if self.slices.is_empty() || key < self.slice_root {
                    return None;
                }
                let idx = (key - self.slice_root) as usize;
                let s = *self.slices.get(idx)? as f64;
                let e = self.slices.get(idx + 1).map(|v| *v as f64).unwrap_or(self.end as f64);
                (s, e)
            }
        };
        let (s, e) = (s.clamp(0.0, 1.0) * len_f, e.clamp(0.0, 1.0) * len_f);
        if e - s < 2.0 {
            None
        } else {
            Some((s, e))
        }
    }

    /// Loop region `(loop_start, loop_end, crossfade)` in samples inside the played region `(s, e)`.
    /// Chop mode loops the whole slice.
    pub fn loop_region(&self, s: f64, e: f64, len: usize) -> (f64, f64, f64) {
        let (ls, le) = match self.mode {
            SamplerMode::Normal => {
                let len_f = len as f64;
                let a = (self.loop_start.clamp(0.0, 1.0) as f64 * len_f).clamp(s, e);
                let b = (self.loop_end.clamp(0.0, 1.0) as f64 * len_f).clamp(s, e);
                if b - a < 16.0 { (s, e) } else { (a, b) }
            }
            SamplerMode::Chop => (s, e),
        };
        let l = le - ls;
        // the crossfade reads material before the loop start, so it can't be longer than that
        let xf = (self.loop_xfade.clamp(0.0, 0.5) as f64 * l).min(ls - s).max(0.0);
        (ls, le, xf)
    }
}

const FADE_SECS: f32 = 0.004;

struct SamplerVoice {
    active: bool,
    key: u8,
    sample: Option<Arc<SampleData>>,
    pos: f64,
    step: f64,
    start: f64,
    end: f64,
    reverse: bool,
    looped: bool,
    loop_start: f64,
    loop_end: f64,
    xfade: f64,
    loop_tail: bool,
    env: Adsr,
    gain: f32,
    remaining: Option<u64>,
    held: bool,
    age: u64,
    /// >0 while fading out after being choked/stolen
    fade: f32,
    fading: bool,
}

impl SamplerVoice {
    fn new(sr: f32) -> Self {
        Self {
            active: false,
            key: 0,
            sample: None,
            pos: 0.0,
            step: 1.0,
            start: 0.0,
            end: 0.0,
            reverse: false,
            looped: false,
            loop_start: 0.0,
            loop_end: 0.0,
            xfade: 0.0,
            loop_tail: false,
            env: Adsr::new(sr),
            gain: 1.0,
            remaining: None,
            held: false,
            age: 0,
            fade: 1.0,
            fading: false,
        }
    }

    fn release(&mut self) {
        self.env.release();
        self.held = false;
        self.remaining = None;
    }
}

pub struct Sampler {
    voices: Vec<SamplerVoice>,
    sr: f32,
    counter: u64,
    /// extra gain for the next started notes (playlist-track volume)
    pub note_gain: f32,
}

impl Sampler {
    pub fn new(sr: f32, polyphony: usize) -> Self {
        Self { voices: (0..polyphony.max(1)).map(|_| SamplerVoice::new(sr)).collect(), sr, counter: 0, note_gain: 1.0 }
    }

    pub fn note_on(&mut self, p: &SamplerParams, sample: Option<&Arc<SampleData>>, key: u8, vel: f32, dur: Option<u64>) {
        let Some(sample) = sample else { return };
        let Some((start, end)) = p.region_for_key(key, sample.len()) else { return };
        self.counter += 1;
        if p.mono {
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.fading = true;
            }
        }
        let idx = self
            .voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        let semis = match p.mode {
            SamplerMode::Normal => key as f32 - p.root_note as f32 + p.pitch,
            SamplerMode::Chop => p.pitch,
        };
        let sr_ratio = sample.sample_rate as f64 / self.sr as f64;
        let v = &mut self.voices[idx];
        v.active = true;
        v.key = key;
        v.sample = Some(Arc::clone(sample));
        v.step = 2f64.powf(semis as f64 / 12.0) * sr_ratio;
        v.start = start;
        v.end = end;
        v.reverse = p.reverse;
        v.looped = p.looped;
        let (ls, le, xf) = p.loop_region(start, end, sample.len());
        v.loop_start = ls;
        v.loop_end = le;
        v.xfade = xf;
        v.loop_tail = p.loop_tail;
        v.pos = if p.reverse { end - 1.0 } else { start };
        v.env.set_params(&p.amp_env);
        v.env.reset();
        v.env.trigger();
        v.gain = (1.0 - p.velocity_sens + p.velocity_sens * vel) * p.gain * self.note_gain;
        // one-shots ignore note length unless looping
        let ignore_len = p.one_shot && !p.looped;
        v.remaining = if ignore_len { None } else { dur };
        v.held = !ignore_len && dur.is_none();
        v.age = self.counter;
        v.fade = 1.0;
        v.fading = false;
    }

    pub fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.active && v.held && v.key == key) {
            v.release();
        }
    }

    pub fn release_all(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.release();
            // one-shots would otherwise ring on after stop
            v.fading = true;
        }
    }

    pub fn kill_all(&mut self) {
        for v in self.voices.iter_mut() {
            v.active = false;
            v.sample = None;
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Adds output into `l`/`r`.
    pub fn render(&mut self, p: &SamplerParams, l: &mut [f32], r: &mut [f32]) {
        let fade_step = 1.0 / (FADE_SECS * self.sr);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.env.set_params(&p.amp_env);
            let Some(sample) = v.sample.take() else {
                v.active = false;
                continue;
            };
            let mut finished = false;
            for i in 0..l.len() {
                let e = v.env.next();
                if v.fading {
                    v.fade -= fade_step;
                    if v.fade <= 0.0 {
                        finished = true;
                        break;
                    }
                }
                let g = e * v.gain * v.fade;
                let looping = v.looped && !(v.loop_tail && !v.held && v.remaining.is_none());
                let llen = v.loop_end - v.loop_start;
                let (mut sl, mut sr) = (read_cubic(&sample.left, v.pos), read_cubic(&sample.right, v.pos));
                if looping && !v.reverse && v.xfade > 0.0 && v.pos >= v.loop_end - v.xfade && v.pos < v.loop_end {
                    // equal-power blend towards the material just before the loop start, which is
                    // exactly what continues after the jump
                    let w = ((v.pos - (v.loop_end - v.xfade)) / v.xfade) as f32;
                    let (a, b) = ((1.0 - w).sqrt(), w.sqrt());
                    sl = sl * a + read_cubic(&sample.left, v.pos - llen) * b;
                    sr = sr * a + read_cubic(&sample.right, v.pos - llen) * b;
                }
                l[i] += sl * g;
                r[i] += sr * g;
                if v.reverse {
                    v.pos -= v.step;
                    if looping && v.pos < v.loop_start && v.pos + v.step >= v.loop_start {
                        v.pos += llen;
                    } else if v.pos < v.start {
                        finished = true;
                        break;
                    }
                } else {
                    v.pos += v.step;
                    if looping && v.pos >= v.loop_end && v.pos - v.step < v.loop_end {
                        v.pos -= llen;
                    } else if v.pos >= v.end {
                        finished = true;
                        break;
                    }
                }
                if let Some(rem) = v.remaining.as_mut() {
                    if *rem == 0 {
                        v.env.release();
                        v.remaining = None;
                        v.held = false;
                    } else {
                        *rem -= 1;
                    }
                }
                if !v.env.is_active() {
                    finished = true;
                    break;
                }
            }
            if finished {
                v.active = false;
            } else {
                v.sample = Some(sample);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_points_sustain_held_note() {
        let s = ramp(4800); // 0.1 s
        let p = SamplerParams { looped: true, loop_start: 0.5, loop_end: 0.9, loop_xfade: 0.2, ..Default::default() };
        let mut sm = Sampler::new(48000.0, 4);
        sm.note_on(&p, Some(&s), 60, 1.0, Some(48000)); // 1 s note, sample is only 0.1 s
        let (mut l, mut r) = (vec![0.0; 40000], vec![0.0; 40000]);
        sm.render(&p, &mut l, &mut r);
        assert_eq!(sm.active_voices(), 1, "loop must keep the voice alive");
        let tail: f32 = l[30000..].iter().map(|x| x.abs()).sum::<f32>() / 10000.0;
        assert!(tail > 0.1, "still sounding: {tail}");
        // no jumps bigger than the waveform's own slope at the seam
        let maxd = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(maxd < 0.08, "click at loop seam: {maxd}");
    }

    #[test]
    fn loop_tail_plays_out_after_release() {
        let s = ramp(4800);
        let p = SamplerParams { looped: true, loop_tail: true, loop_start: 0.2, loop_end: 0.4,
            amp_env: AdsrParams::adsr(0.001, 0.1, 1.0, 5.0), ..Default::default() };
        let mut sm = Sampler::new(48000.0, 4);
        sm.note_on(&p, Some(&s), 60, 1.0, Some(9600)); // released after 0.2 s, long release
        let (mut l, mut r) = (vec![0.0; 48000], vec![0.0; 48000]);
        sm.render(&p, &mut l, &mut r);
        assert_eq!(sm.active_voices(), 0, "after release the sample runs to its end and stops");
    }

    fn ramp(n: usize) -> Arc<SampleData> {
        let v: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        Arc::new(SampleData::new(v.clone(), v, 48000))
    }

    #[test]
    fn octave_up_plays_half_as_long() {
        let s = ramp(4800);
        let p = SamplerParams::default();
        let mut sm = Sampler::new(48000.0, 4);
        sm.note_on(&p, Some(&s), 72, 1.0, None);
        let mut l = vec![0.0; 4800];
        let mut r = vec![0.0; 4800];
        sm.render(&p, &mut l, &mut r);
        let last_nonzero = l.iter().rposition(|v| v.abs() > 1e-6).unwrap();
        assert!((2380..=2420).contains(&last_nonzero), "got {last_nonzero}");
        assert_eq!(sm.active_voices(), 0);
    }

    #[test]
    fn chop_maps_slices_to_keys() {
        let p = SamplerParams { mode: SamplerMode::Chop, slices: vec![0.0, 0.25, 0.5, 0.75], ..Default::default() };
        assert_eq!(p.region_for_key(60, 1000), Some((0.0, 250.0)));
        assert_eq!(p.region_for_key(63, 1000), Some((750.0, 1000.0)));
        assert_eq!(p.region_for_key(64, 1000), None);
        assert_eq!(p.region_for_key(59, 1000), None);
    }
}
