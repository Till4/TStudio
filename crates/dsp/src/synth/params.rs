//! `SynthParams` (the serialised preset) with the second filter and built-in FX chain settings.

use super::*;

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
    pub(super) fn tail_secs(&self) -> f32 {
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
