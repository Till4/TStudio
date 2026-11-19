//! TSynth factory presets, grouped for the preset browser. The first nine names come from
//! the original TSynth and keep their names (CLI scripts and projects refer to them).

use crate::env::AdsrParams;
use crate::filter::{FilterMode, FilterType};
use crate::fx::{ChorusParams, DelayParams, DistParams, ReverbParams};
use crate::fx_extra::OttParams;
use crate::osc::Waveform;
use crate::synth::*;
use crate::wavetable::WtKind;

/// Browser categories (Init is shown separately).
pub const PRESET_CATEGORIES: [&str; 7] = ["Kick", "Bass", "Lead", "Stab", "Pad", "Acid", "FX"];

fn env(a: f32, d: f32, s: f32, r: f32) -> AdsrParams {
    AdsrParams::adsr(a, d, s, r)
}

/// Neutral starting point for kicks: kick engine on, one sine body, open filter.
fn kick_base() -> SynthParams {
    SynthParams {
        osc1: OscParams { wave: Waveform::Sine, level: 1.0, rand_phase: 0.0, ..OscParams::default() },
        osc2: OscParams::off(),
        cutoff: 14000.0,
        resonance: 0.0,
        filter_env_amount: 0.0,
        key_track: 0.0,
        amp_env: AdsrParams { attack: 0.0005, hold: 0.02, decay: 0.4, sustain: 0.0, release: 0.04, decay_curve: 0.2, ..Default::default() },
        velocity_sens: 0.25,
        gain: 0.75,
        polyphony: 4,
        // each hit chokes the previous tail: no stacked bass bodies at 150–180 BPM
        mono: true,
        legato: false,
        kick: KickParams { enabled: true, ..KickParams::default() },
        ..SynthParams::default()
    }
}

pub fn all() -> Vec<(&'static str, &'static str, SynthParams)> {
    let init = SynthParams::default();

    // ------------------------------------------------------------ legacy (unchanged)
    let bass = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, octave: -1, level: 0.8, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Square, octave: -1, fine: 7.0, level: 0.5, ..OscParams::default() },
        sub_level: 0.6,
        cutoff: 400.0,
        resonance: 0.25,
        filter_env_amount: 3.0,
        amp_env: env(0.002, 0.25, 0.8, 0.08),
        filter_env: env(0.001, 0.2, 0.1, 0.1),
        mono: true,
        glide: 0.04,
        gain: 0.3,
        ..init.clone()
    };
    let lead = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, unison: 5, detune: 25.0, spread: 0.7, level: 0.7, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Square, octave: 1, level: 0.25, ..OscParams::default() },
        cutoff: 3500.0,
        resonance: 0.2,
        filter_env_amount: 1.0,
        lfo1: LfoParams { wave: Waveform::Sine, rate: 5.5, amount: 0.04, target: LfoTarget::Pitch, ..LfoParams::default() },
        glide: 0.03,
        ..init.clone()
    };
    let pad = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, unison: 7, detune: 35.0, spread: 1.0, level: 0.6, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Triangle, octave: 1, unison: 3, detune: 15.0, level: 0.4, ..OscParams::default() },
        cutoff: 1800.0,
        resonance: 0.1,
        filter_env_amount: 0.8,
        amp_env: env(0.8, 1.0, 0.8, 1.6),
        filter_env: env(1.2, 1.5, 0.5, 1.5),
        lfo1: LfoParams { wave: Waveform::Triangle, rate: 0.3, amount: 0.3, target: LfoTarget::Cutoff, ..LfoParams::default() },
        gain: 0.4,
        ..init.clone()
    };
    let pluck = SynthParams {
        osc1: OscParams { wave: Waveform::Square, unison: 3, detune: 12.0, level: 0.7, ..OscParams::default() },
        cutoff: 600.0,
        resonance: 0.3,
        filter_env_amount: 4.0,
        amp_env: env(0.001, 0.35, 0.0, 0.2),
        filter_env: env(0.001, 0.18, 0.0, 0.15),
        ..init.clone()
    };
    let sub808 = SynthParams {
        osc1: OscParams { wave: Waveform::Sine, octave: -1, level: 1.0, ..OscParams::default() },
        filter_mode: FilterMode::LowPass,
        cutoff: 2000.0,
        resonance: 0.0,
        filter_env_amount: 0.0,
        amp_env: env(0.001, 1.8, 0.0, 0.3),
        mono: true,
        glide: 0.06,
        gain: 0.8,
        ..init.clone()
    };
    let acid = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, octave: -1, level: 0.8, ..OscParams::default() },
        cutoff: 300.0,
        resonance: 0.75,
        filter_env_amount: 4.5,
        key_track: 0.5,
        amp_env: env(0.001, 0.3, 0.7, 0.05),
        filter_env: env(0.001, 0.18, 0.0, 0.1),
        mono: true,
        glide: 0.06,
        drive_db: 6.0,
        gain: 0.45,
        ..init.clone()
    };
    let hard_stab = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, unison: 5, detune: 30.0, spread: 0.8, level: 0.7, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Square, octave: 1, level: 0.3, ..OscParams::default() },
        cutoff: 2500.0,
        resonance: 0.25,
        filter_env_amount: 2.5,
        amp_env: env(0.001, 0.22, 0.0, 0.08),
        filter_env: env(0.001, 0.15, 0.1, 0.1),
        drive_db: 10.0,
        gain: 0.4,
        ..init.clone()
    };

    // ------------------------------------------------------------ kicks
    let kb = kick_base();
    // G1 body, 4-octave drop, tick and soft saturation (upgraded to the kick engine)
    let hard_kick = SynthParams {
        kick: KickParams { tune: 31.0, pitch_start: 48.0, pitch_time: 0.06, pitch_curve: 0.7, click_level: 0.35, click_type: ClickType::Tick, click_tone: 2800.0, click_decay: 0.005, drive_db: 14.0, ..kb.kick.clone() },
        amp_env: AdsrParams { hold: 0.03, decay: 0.8, decay_curve: 0.0, ..kb.amp_env },
        ..kb.clone()
    };
    // F#1, very short and hard clipped, noise click, tamed fizz, final clipper
    let schranz_kick = SynthParams {
        osc1: OscParams { table: WtKind::DriveSine, position: 0.35, ..kb.osc1.clone() },
        kick: KickParams {
            tune: 30.0,
            pitch_start: 44.0,
            pitch_time: 0.05,
            pitch_curve: 0.75,
            click_level: 0.45,
            click_type: ClickType::Noise,
            click_tone: 4000.0,
            click_decay: 0.004,
            drive_db: 20.0,
            drive_mode: DriveMode::Hard,
            ..kb.kick.clone()
        },
        amp_env: AdsrParams { hold: 0.04, decay: 0.75, decay_curve: 0.0, ..kb.amp_env },
        filter_type: FilterType::Svf24,
        cutoff: 7000.0,
        fx: SynthFxParams { clip_on: true, ..SynthFxParams::default() },
        gain: 0.7,
        ..kb.clone()
    };
    // G1, longer pitchy tail, asymmetric drive through a ladder, OTT
    let hardtekk_kick = SynthParams {
        osc1: OscParams { table: WtKind::DriveSine, position: 0.6, ..kb.osc1.clone() },
        kick: KickParams {
            tune: 31.0,
            pitch_start: 36.0,
            pitch_time: 0.11,
            pitch_curve: 0.45,
            click_level: 0.3,
            click_type: ClickType::Snap,
            click_tone: 1800.0,
            click_decay: 0.01,
            drive_db: 16.0,
            drive_mode: DriveMode::Asym,
            ..kb.kick.clone()
        },
        amp_env: AdsrParams { hold: 0.06, decay: 0.8, decay_curve: -0.1, ..kb.amp_env },
        filter_type: FilterType::Ladder,
        cutoff: 5000.0,
        resonance: 0.2,
        fx: SynthFxParams { clip_on: true, ..SynthFxParams::default() },
        gain: 0.8,
        ..kb.clone()
    };
    // F1 punch with its own body envelope and a long saturated rumble tail
    let rumble_kick = SynthParams {
        kick: KickParams {
            tune: 29.0,
            pitch_start: 40.0,
            pitch_time: 0.06,
            pitch_curve: 0.7,
            click_level: 0.3,
            body_hold: 0.02,
            body_decay: 0.25,
            drive_db: 8.0,
            rumble_level: 0.55,
            rumble_delay: 0.07,
            rumble_decay: 0.55,
            rumble_tone: 140.0,
            ..kb.kick.clone()
        },
        amp_env: AdsrParams { hold: 0.0, decay: 1.2, decay_curve: 0.0, ..kb.amp_env },
        cutoff: 6000.0,
        fx: SynthFxParams { clip_on: true, ..SynthFxParams::default() },
        gain: 0.7,
        ..kb.clone()
    };
    // E1, folded body + grit, noisy click, OTT
    let industrial_kick = SynthParams {
        osc1: OscParams { table: WtKind::DriveSine, position: 0.3, ..kb.osc1.clone() },
        noise_level: 0.12,
        noise_tone: 3000.0,
        kick: KickParams {
            tune: 28.0,
            pitch_start: 52.0,
            pitch_time: 0.045,
            pitch_curve: 0.8,
            click_level: 0.5,
            click_type: ClickType::Noise,
            click_tone: 3000.0,
            click_decay: 0.006,
            drive_db: 7.0,
            drive_mode: DriveMode::Fold,
            ..kb.kick.clone()
        },
        amp_env: AdsrParams { hold: 0.03, decay: 0.7, decay_curve: 0.0, ..kb.amp_env },
        filter_type: FilterType::Svf24,
        cutoff: 6000.0,
        fx: SynthFxParams { clip_on: true, ..SynthFxParams::default() },
        gain: 0.6,
        ..kb.clone()
    };
    // A1 techno kick, clean and punchy
    let punch_kick = SynthParams {
        kick: KickParams { tune: 33.0, pitch_start: 36.0, pitch_time: 0.045, pitch_curve: 0.6, click_level: 0.25, click_tone: 3500.0, drive_db: 6.0, ..kb.kick.clone() },
        amp_env: AdsrParams { hold: 0.02, decay: 0.55, decay_curve: 0.0, ..kb.amp_env },
        gain: 0.8,
        ..kb.clone()
    };
    // G1, long round body: soft drive keeps the sub loud for ~300 ms
    let fat_kick = SynthParams {
        osc1: OscParams { table: WtKind::DriveSine, position: 0.2, ..kb.osc1.clone() },
        kick: KickParams {
            tune: 31.0,
            pitch_start: 38.0,
            pitch_time: 0.06,
            pitch_curve: 0.6,
            click_level: 0.3,
            click_type: ClickType::Tick,
            click_tone: 3000.0,
            click_decay: 0.005,
            drive_db: 16.0,
            drive_mode: DriveMode::Soft,
            ..kb.kick.clone()
        },
        amp_env: AdsrParams { hold: 0.08, decay: 0.7, decay_curve: -0.2, ..kb.amp_env },
        filter_type: FilterType::Svf24,
        cutoff: 4500.0,
        fx: SynthFxParams { clip_on: true, ..SynthFxParams::default() },
        gain: 0.75,
        ..kb.clone()
    };
    // Hardtekk kick-bass: plays the key (C4 = G1), asym drive into a resonant ladder, long tonal tail
    let tekk_bass_kick = SynthParams {
        osc1: OscParams { table: WtKind::DriveSine, position: 0.5, ..kb.osc1.clone() },
        kick: KickParams {
            tune: 31.0,
            key_follow: true,
            pitch_start: 30.0,
            pitch_time: 0.09,
            pitch_curve: 0.5,
            click_level: 0.25,
            click_type: ClickType::Snap,
            click_tone: 2000.0,
            click_decay: 0.008,
            drive_db: 18.0,
            drive_mode: DriveMode::Asym,
            ..kb.kick.clone()
        },
        amp_env: AdsrParams { hold: 0.08, decay: 0.6, decay_curve: 0.0, ..kb.amp_env },
        filter_type: FilterType::Ladder,
        cutoff: 3500.0,
        resonance: 0.25,
        fx: SynthFxParams { clip_on: true, ..SynthFxParams::default() },
        gain: 0.7,
        ..kb.clone()
    };

    // ------------------------------------------------------------ bass
    let offbeat_bass = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, octave: -1, unison: 3, detune: 12.0, spread: 0.3, level: 0.7, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Square, octave: -1, level: 0.4, ..OscParams::default() },
        sub_level: 0.3,
        filter_type: FilterType::Svf24,
        cutoff: 900.0,
        resonance: 0.2,
        filter_env_amount: 2.5,
        key_track: 0.3,
        filter_env: env(0.001, 0.12, 0.05, 0.08),
        amp_env: env(0.001, 0.15, 0.3, 0.06),
        drive_db: 6.0,
        velocity_sens: 0.5,
        gain: 0.6,
        ..init.clone()
    };
    let hardtekk_bass = SynthParams {
        osc1: OscParams { table: WtKind::Hollow, position: 0.4, octave: -1, unison: 3, detune: 15.0, spread: 0.4, level: 0.7, ..OscParams::default() },
        osc2: OscParams::off(),
        sub_level: 0.4,
        sub_octave: 0,
        filter_type: FilterType::Ladder,
        cutoff: 1200.0,
        resonance: 0.35,
        filter_env_amount: 2.0,
        filter_env: env(0.001, 0.15, 0.1, 0.1),
        amp_env: env(0.001, 0.2, 0.5, 0.06),
        drive_db: 8.0,
        drive_mode: DriveMode::Asym,
        gain: 0.55,
        mod_matrix: {
            let mut m = [ModSlot::default(); NUM_MOD_SLOTS];
            m[0] = ModSlot::new(ModSource::FilterEnv, ModDest::Osc1Pos, 0.3);
            m[1] = ModSlot::new(ModSource::Macro1, ModDest::Cutoff1, 0.5);
            m
        },
        ..init.clone()
    };
    let reese_bass = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, octave: -1, unison: 5, detune: 25.0, spread: 0.5, blend: 0.7, level: 0.6, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Saw, octave: -1, fine: 15.0, level: 0.5, ..OscParams::default() },
        filter_type: FilterType::Svf24,
        cutoff: 700.0,
        resonance: 0.15,
        filter_env_amount: 0.5,
        amp_env: env(0.005, 0.5, 0.9, 0.15),
        lfo3: LfoParams { rate: 0.2, ..LfoParams::default() },
        mod_matrix: {
            let mut m = [ModSlot::default(); NUM_MOD_SLOTS];
            m[0] = ModSlot::new(ModSource::Lfo3, ModDest::Cutoff1, 0.15);
            m
        },
        mono: true,
        glide: 0.05,
        fx: SynthFxParams { chorus_on: true, chorus: ChorusParams { mix: 0.3, ..ChorusParams::default() }, ..SynthFxParams::default() },
        gain: 0.65,
        ..init.clone()
    };

    // ------------------------------------------------------------ leads
    let hoover_lead = SynthParams {
        osc1: OscParams { table: WtKind::Hollow, position: 0.7, unison: 7, detune: 45.0, spread: 0.9, blend: 0.8, level: 0.7, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Saw, octave: -1, level: 0.3, ..OscParams::default() },
        cutoff: 6000.0,
        resonance: 0.1,
        filter_env_amount: 0.5,
        pitch_env_amount: -7.0,
        pitch_env_decay: 0.08,
        amp_env: env(0.005, 0.4, 0.8, 0.25),
        mono: true,
        glide: 0.08,
        fx: SynthFxParams { chorus_on: true, reverb_on: true, reverb: ReverbParams { mix: 0.15, ..ReverbParams::default() }, ..SynthFxParams::default() },
        gain: 0.9,
        ..init.clone()
    };
    let rave_lead = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, unison: 9, detune: 45.0, spread: 1.0, blend: 0.6, level: 0.7, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Saw, octave: 1, unison: 5, detune: 30.0, level: 0.3, ..OscParams::default() },
        filter_type: FilterType::Svf24,
        cutoff: 7000.0,
        resonance: 0.1,
        filter_env_amount: 0.5,
        amp_env: env(0.002, 0.5, 0.7, 0.3),
        fx: SynthFxParams {
            delay_on: true,
            delay: DelayParams { time_beats: 0.75, feedback: 0.35, mix: 0.2, ..DelayParams::default() },
            reverb_on: true,
            reverb: ReverbParams { mix: 0.25, ..ReverbParams::default() },
            ..SynthFxParams::default()
        },
        gain: 0.3,
        ..init.clone()
    };

    // ------------------------------------------------------------ stabs
    let schranz_stab = SynthParams {
        osc1: OscParams { table: WtKind::Fold, position: 0.45, warp: WarpMode::Sync, warp_amount: 0.3, unison: 3, detune: 15.0, level: 0.7, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Square, octave: 1, level: 0.25, ..OscParams::default() },
        filter_mode: FilterMode::BandPass,
        cutoff: 1400.0,
        resonance: 0.35,
        filter_env_amount: 2.0,
        filter_env: env(0.001, 0.1, 0.0, 0.08),
        amp_env: env(0.001, 0.14, 0.0, 0.05),
        drive_db: 10.0,
        filter2: Filter2Params { enabled: true, kind: FilterType::Svf24, mode: FilterMode::LowPass, cutoff: 6000.0, ..Filter2Params::default() },
        drive_mode: DriveMode::Hard,
        mod_matrix: {
            let mut m = [ModSlot::default(); NUM_MOD_SLOTS];
            m[0] = ModSlot::new(ModSource::Macro1, ModDest::Osc1Pos, 0.5);
            m[1] = ModSlot::new(ModSource::Velocity, ModDest::Cutoff1, 0.2);
            m
        },
        fx: SynthFxParams {
            ott_on: true,
            ott: OttParams { depth: 0.35, ..OttParams::default() },
            reverb_on: true,
            reverb: ReverbParams { size: 0.4, mix: 0.12, ..ReverbParams::default() },
            ..SynthFxParams::default()
        },
        gain: 0.27,
        ..init.clone()
    };
    let industrial_stab = SynthParams {
        osc1: OscParams { table: WtKind::Fm, position: 0.55, warp: WarpMode::FmOsc2, warp_amount: 0.3, level: 0.7, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Sine, octave: 1, semi: 7, level: 0.0, ..OscParams::default() },
        filter_type: FilterType::Comb,
        cutoff: 300.0,
        resonance: 0.6,
        filter_env_amount: 1.0,
        filter_env: env(0.001, 0.15, 0.0, 0.1),
        filter2: Filter2Params { enabled: true, kind: FilterType::Svf24, mode: FilterMode::LowPass, cutoff: 5000.0, ..Filter2Params::default() },
        amp_env: env(0.001, 0.2, 0.0, 0.08),
        drive_db: 10.0,
        fx: SynthFxParams { delay_on: true, delay: DelayParams { time_beats: 0.5, feedback: 0.35, mix: 0.2, ..DelayParams::default() }, ..SynthFxParams::default() },
        gain: 0.34,
        ..init.clone()
    };
    let chord_stab = SynthParams {
        osc1: OscParams { table: WtKind::Morph, position: 0.55, unison: 3, detune: 10.0, level: 0.6, ..OscParams::default() },
        osc2: OscParams { enabled: true, wave: Waveform::Saw, octave: 1, level: 0.3, ..OscParams::default() },
        filter_type: FilterType::Svf24,
        cutoff: 1200.0,
        resonance: 0.3,
        filter_env_amount: 1.5,
        filter_env: env(0.001, 0.25, 0.0, 0.2),
        amp_env: env(0.001, 0.35, 0.0, 0.2),
        lfo1: LfoParams { wave: Waveform::Triangle, sync: true, beats: 4.0, amount: 0.15, target: LfoTarget::Cutoff, ..LfoParams::default() },
        fx: SynthFxParams {
            delay_on: true,
            delay: DelayParams { time_beats: 0.75, feedback: 0.45, mix: 0.3, ..DelayParams::default() },
            reverb_on: true,
            reverb: ReverbParams { mix: 0.3, ..ReverbParams::default() },
            ..SynthFxParams::default()
        },
        gain: 0.6,
        ..init.clone()
    };

    // ------------------------------------------------------------ pads
    let dark_pad = SynthParams {
        osc1: OscParams { table: WtKind::Vowel, position: 0.3, unison: 5, detune: 18.0, spread: 0.8, level: 0.6, ..OscParams::default() },
        osc2: OscParams { enabled: true, table: WtKind::Harmonics, position: 0.2, octave: -1, level: 0.4, ..OscParams::default() },
        filter_type: FilterType::Formant,
        cutoff: 600.0,
        resonance: 0.4,
        filter_env_amount: 0.0,
        key_track: 0.0,
        amp_env: env(1.0, 1.0, 0.8, 2.0),
        lfo3: LfoParams { sync: true, beats: 8.0, ..LfoParams::default() },
        mod_matrix: {
            let mut m = [ModSlot::default(); NUM_MOD_SLOTS];
            m[0] = ModSlot::new(ModSource::Lfo3, ModDest::Osc1Pos, 0.4);
            m
        },
        fx: SynthFxParams {
            chorus_on: true,
            reverb_on: true,
            reverb: ReverbParams { size: 0.85, mix: 0.35, ..ReverbParams::default() },
            ..SynthFxParams::default()
        },
        gain: 0.9,
        ..init.clone()
    };

    // ------------------------------------------------------------ acid
    let acid_ladder = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, octave: -1, level: 0.8, ..OscParams::default() },
        filter_type: FilterType::Ladder,
        cutoff: 350.0,
        resonance: 0.78,
        filter_env_amount: 4.0,
        key_track: 0.5,
        filter_env: env(0.001, 0.2, 0.0, 0.1),
        amp_env: env(0.001, 0.3, 0.7, 0.05),
        mono: true,
        glide: 0.06,
        slide_only: true,
        drive_db: 8.0,
        mod_matrix: {
            let mut m = [ModSlot::default(); NUM_MOD_SLOTS];
            m[0] = ModSlot::new(ModSource::Velocity, ModDest::Cutoff1, 0.25);
            m[1] = ModSlot::new(ModSource::Macro1, ModDest::Cutoff1, 0.4);
            m[2] = ModSlot::new(ModSource::Macro2, ModDest::FxDistDrive, 0.75);
            m
        },
        fx: SynthFxParams {
            dist_on: true,
            dist: DistParams { drive_db: 6.0, mix: 0.5, output_db: -3.0, ..DistParams::default() },
            delay_on: true,
            delay: DelayParams { time_beats: 0.75, feedback: 0.3, mix: 0.15, ..DelayParams::default() },
            ..SynthFxParams::default()
        },
        gain: 0.7,
        ..init.clone()
    };
    let acid_squelch = SynthParams {
        osc1: OscParams { wave: Waveform::Square, octave: -1, warp: WarpMode::Pwm, warp_amount: 0.2, level: 0.8, ..OscParams::default() },
        filter_type: FilterType::Ladder,
        cutoff: 500.0,
        resonance: 0.85,
        filter_env_amount: 3.5,
        key_track: 0.4,
        filter_drive: 9.0,
        filter_env: env(0.001, 0.15, 0.0, 0.08),
        amp_env: env(0.001, 0.25, 0.6, 0.05),
        mono: true,
        glide: 0.05,
        slide_only: true,
        fx: SynthFxParams { delay_on: true, delay: DelayParams { time_beats: 0.5, feedback: 0.35, mix: 0.18, ..DelayParams::default() }, ..SynthFxParams::default() },
        gain: 0.9,
        ..init.clone()
    };

    // ------------------------------------------------------------ fx
    let riser = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, unison: 7, detune: 40.0, spread: 1.0, level: 0.5, ..OscParams::default() },
        noise_level: 0.4,
        noise_tone: 8000.0,
        filter_type: FilterType::Svf24,
        cutoff: 300.0,
        resonance: 0.3,
        filter_env_amount: 0.0,
        amp_env: env(2.0, 1.0, 1.0, 1.0),
        mod_env: env(4.0, 1.0, 1.0, 1.0),
        mod_matrix: {
            let mut m = [ModSlot::default(); NUM_MOD_SLOTS];
            m[0] = ModSlot::new(ModSource::ModEnv, ModDest::Pitch, 0.5);
            m[1] = ModSlot::new(ModSource::ModEnv, ModDest::Cutoff1, 0.75);
            m
        },
        fx: SynthFxParams { reverb_on: true, reverb: ReverbParams { size: 0.8, mix: 0.4, ..ReverbParams::default() }, ..SynthFxParams::default() },
        gain: 0.3,
        ..init.clone()
    };
    let laser_zap = SynthParams {
        osc1: OscParams { wave: Waveform::Saw, level: 0.7, ..OscParams::default() },
        pitch_env_amount: 48.0,
        pitch_env_decay: 0.05,
        filter_mode: FilterMode::BandPass,
        cutoff: 2000.0,
        resonance: 0.5,
        filter_env_amount: 0.0,
        amp_env: env(0.001, 0.25, 0.0, 0.1),
        fx: SynthFxParams { delay_on: true, delay: DelayParams { time_beats: 0.375, feedback: 0.4, mix: 0.25, ..DelayParams::default() }, ..SynthFxParams::default() },
        gain: 0.18,
        ..init.clone()
    };
    let siren = SynthParams {
        osc1: OscParams { wave: Waveform::Square, level: 0.6, ..OscParams::default() },
        cutoff: 3000.0,
        filter_env_amount: 0.0,
        amp_env: env(0.01, 0.2, 1.0, 0.4),
        lfo1: LfoParams { wave: Waveform::Triangle, sync: true, beats: 2.0, amount: 0.5, target: LfoTarget::Pitch, retrig: true, ..LfoParams::default() },
        fx: SynthFxParams { reverb_on: true, reverb: ReverbParams { mix: 0.3, ..ReverbParams::default() }, ..SynthFxParams::default() },
        gain: 0.19,
        ..init.clone()
    };

    vec![
        ("Init", "Init", init),
        ("Bass", "Bass", bass),
        ("Lead", "Lead", lead),
        ("Pad", "Pad", pad),
        ("Stab", "Pluck", pluck),
        ("Bass", "808 Sub", sub808),
        ("Kick", "Hard Kick", hard_kick),
        ("Acid", "Acid", acid),
        ("Stab", "Hard Stab", hard_stab),
        ("Kick", "Schranz Kick", schranz_kick),
        ("Kick", "Hardtekk Kick", hardtekk_kick),
        ("Kick", "Rumble Kick", rumble_kick),
        ("Kick", "Industrial Kick", industrial_kick),
        ("Kick", "Punch Kick", punch_kick),
        ("Kick", "Fat Kick", fat_kick),
        ("Kick", "Tekk Bass Kick", tekk_bass_kick),
        ("Bass", "Offbeat Bass", offbeat_bass),
        ("Bass", "Hardtekk Bass", hardtekk_bass),
        ("Bass", "Reese Bass", reese_bass),
        ("Lead", "Hoover Lead", hoover_lead),
        ("Lead", "Rave Lead", rave_lead),
        ("Stab", "Schranz Stab", schranz_stab),
        ("Stab", "Industrial Stab", industrial_stab),
        ("Stab", "Chord Stab", chord_stab),
        ("Pad", "Dark Pad", dark_pad),
        ("Acid", "Acid Ladder", acid_ladder),
        ("Acid", "Acid Squelch", acid_squelch),
        ("FX", "Riser", riser),
        ("FX", "Laser Zap", laser_zap),
        ("FX", "Siren", siren),
    ]
}
