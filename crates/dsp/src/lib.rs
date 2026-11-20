//! TStudio DSP: pure audio processing, no I/O, no allocation in `process`/`render` paths.

pub mod analysis;
pub mod autotune;
pub mod env;
pub mod fft;
pub mod filter;
pub mod formant;
pub mod fx;
pub mod fx_extra;
pub mod fx_width;
pub mod isolate;
pub mod meter;
pub mod music;
pub mod osc;
pub mod sample;
pub mod sampler;
pub mod spectrum;
pub mod slicer;
pub mod stretch;
pub mod synth;
pub mod synth_presets;
pub mod wavetable;

pub use sample::SampleData;

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

#[inline]
pub fn midi_to_freq(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 69.0) / 12.0)
}

/// Balance-style pan law: centre = unity on both sides.
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
}

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub fn note_name(key: u8) -> String {
    format!("{}{}", NOTE_NAMES[(key % 12) as usize], key as i32 / 12 - 1)
}

/// Tiny deterministic RNG (xorshift32) usable on the audio thread.
#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u32);

impl Rng {
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}
