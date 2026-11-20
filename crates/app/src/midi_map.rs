//! MIDI controller mapping (pure logic): parsing raw MIDI bytes, CC → parameter targets,
//! value scaling and "MIDI learn". The MIDI thread only parses and forwards messages
//! through a channel; the UI thread applies them to the project (never the audio thread).

use serde::{Deserialize, Serialize};
use tstudio_project::{Instrument, Project};

/// A parsed MIDI message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiMsg {
    NoteOn(u8, f32),
    NoteOff(u8),
    /// control change: (MIDI channel 0..15, controller, value 0..127)
    Cc(u8, u8, u8),
}

/// Parses one raw MIDI message (running status is resolved by midir).
pub fn parse(msg: &[u8]) -> Option<MidiMsg> {
    if msg.len() < 3 {
        return None;
    }
    let status = msg[0] & 0xF0;
    let ch = msg[0] & 0x0F;
    let (d1, d2) = (msg[1].min(127), msg[2].min(127));
    match status {
        0x90 if d2 > 0 => Some(MidiMsg::NoteOn(d1, d2 as f32 / 127.0)),
        0x80 | 0x90 => Some(MidiMsg::NoteOff(d1)),
        0xB0 => Some(MidiMsg::Cc(ch, d1, d2)),
        _ => None,
    }
}

/// A parameter a controller can drive.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MidiTarget {
    /// mixer insert volume (0 = master)
    MixerVolume(usize),
    MixerPan(usize),
    /// TSynth macro 1..4 of a channel (index 0..3)
    SynthMacro { channel: usize, index: u8 },
    /// TSynth filter cutoff of a channel
    SynthCutoff(usize),
}

pub const VOL_MAX: f32 = 1.25;

impl MidiTarget {
    /// Parameter value for a CC value 0..127.
    pub fn scale(&self, v: u8) -> f32 {
        let x = v.min(127) as f32 / 127.0;
        match self {
            MidiTarget::MixerVolume(_) => x * VOL_MAX,
            // 64 = centre
            MidiTarget::MixerPan(_) => ((v.min(127) as f32 - 64.0) / 63.0).clamp(-1.0, 1.0),
            MidiTarget::SynthMacro { .. } => x,
            // exponential 20 Hz .. 20 kHz
            MidiTarget::SynthCutoff(_) => 20.0 * 1000f32.powf(x),
        }
    }

    /// Writes the scaled value into the project. Returns false if the target does not exist.
    pub fn apply(&self, p: &mut Project, v: u8) -> bool {
        let val = self.scale(v);
        match *self {
            MidiTarget::MixerVolume(t) => p.mixer.get_mut(t).map(|m| m.volume = val).is_some(),
            MidiTarget::MixerPan(t) => p.mixer.get_mut(t).map(|m| m.pan = val).is_some(),
            MidiTarget::SynthMacro { channel, index } => match p.channels.get_mut(channel).map(|c| &mut c.instrument) {
                Some(Instrument::Synth(s)) => {
                    match index {
                        0 => s.macro1 = val,
                        1 => s.macro2 = val,
                        2 => s.macro3 = val,
                        _ => s.macro4 = val,
                    }
                    true
                }
                _ => false,
            },
            MidiTarget::SynthCutoff(channel) => match p.channels.get_mut(channel).map(|c| &mut c.instrument) {
                Some(Instrument::Synth(s)) => {
                    s.cutoff = val;
                    true
                }
                _ => false,
            },
        }
    }

    /// German label for the UI.
    pub fn label(&self, p: &Project) -> String {
        let track = |t: usize| p.mixer.get(t).map(|m| format!("{t} {}", m.name)).unwrap_or_else(|| format!("{t} (fehlt)"));
        let chan = |c: usize| p.channels.get(c).map(|x| x.name.clone()).unwrap_or_else(|| format!("Kanal {c} (fehlt)"));
        match *self {
            MidiTarget::MixerVolume(t) => format!("Mixer {} · Lautstärke", track(t)),
            MidiTarget::MixerPan(t) => format!("Mixer {} · Pan", track(t)),
            MidiTarget::SynthMacro { channel, index } => format!("{} · Makro {}", chan(channel), index + 1),
            MidiTarget::SynthCutoff(channel) => format!("{} · Cutoff", chan(channel)),
        }
    }
}

/// One learned controller.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CcMapping {
    pub midi_channel: u8,
    pub cc: u8,
    pub target: MidiTarget,
}

/// Stores a learned mapping: a controller drives one target and a target listens to one controller.
pub fn learn(maps: &mut Vec<CcMapping>, midi_channel: u8, cc: u8, target: MidiTarget) {
    maps.retain(|m| !(m.midi_channel == midi_channel && m.cc == cc) && m.target != target);
    maps.push(CcMapping { midi_channel, cc, target });
}

/// Targets driven by a controller.
pub fn targets(maps: &[CcMapping], midi_channel: u8, cc: u8) -> Vec<MidiTarget> {
    maps.iter().filter(|m| m.midi_channel == midi_channel && m.cc == cc).map(|m| m.target).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notes_and_cc() {
        assert_eq!(parse(&[0x90, 60, 127]), Some(MidiMsg::NoteOn(60, 1.0)));
        assert_eq!(parse(&[0x90, 60, 0]), Some(MidiMsg::NoteOff(60)));
        assert_eq!(parse(&[0x83, 61, 10]), Some(MidiMsg::NoteOff(61)));
        assert_eq!(parse(&[0xB2, 74, 100]), Some(MidiMsg::Cc(2, 74, 100)));
        assert_eq!(parse(&[0xE0, 0, 64]), None, "pitch bend ignored");
        assert_eq!(parse(&[0xB0, 1]), None);
    }

    #[test]
    fn scaling() {
        assert_eq!(MidiTarget::MixerVolume(1).scale(0), 0.0);
        assert!((MidiTarget::MixerVolume(1).scale(127) - VOL_MAX).abs() < 1e-6);
        assert_eq!(MidiTarget::MixerPan(1).scale(64), 0.0);
        assert_eq!(MidiTarget::MixerPan(1).scale(0), -1.0);
        assert_eq!(MidiTarget::MixerPan(1).scale(127), 1.0);
        assert_eq!(MidiTarget::SynthMacro { channel: 0, index: 0 }.scale(127), 1.0);
        let c = MidiTarget::SynthCutoff(0);
        assert!((c.scale(0) - 20.0).abs() < 1e-3);
        assert!((c.scale(127) - 20000.0).abs() < 1.0);
        assert!(c.scale(64) > 500.0 && c.scale(64) < 1000.0, "exponential");
    }

    #[test]
    fn apply_writes_parameters() {
        let mut p = Project::new_default(); // channel 0 = TSynth
        assert!(MidiTarget::MixerVolume(1).apply(&mut p, 127));
        assert!((p.mixer[1].volume - VOL_MAX).abs() < 1e-6);
        assert!(MidiTarget::MixerPan(1).apply(&mut p, 0));
        assert_eq!(p.mixer[1].pan, -1.0);
        assert!(MidiTarget::SynthMacro { channel: 0, index: 2 }.apply(&mut p, 127));
        assert!(MidiTarget::SynthCutoff(0).apply(&mut p, 0));
        match &p.channels[0].instrument {
            Instrument::Synth(s) => {
                assert_eq!(s.macro3, 1.0);
                assert!((s.cutoff - 20.0).abs() < 1e-3);
            }
            _ => unreachable!(),
        }
        assert!(!MidiTarget::MixerVolume(99).apply(&mut p, 10));
        assert!(!MidiTarget::SynthCutoff(5).apply(&mut p, 10));
    }

    #[test]
    fn learning_replaces_old_assignments() {
        let mut m = Vec::new();
        learn(&mut m, 0, 7, MidiTarget::MixerVolume(1));
        learn(&mut m, 0, 10, MidiTarget::MixerPan(1));
        assert_eq!(targets(&m, 0, 7), vec![MidiTarget::MixerVolume(1)]);
        // same knob re-learned to another target
        learn(&mut m, 0, 7, MidiTarget::MixerVolume(2));
        assert_eq!(targets(&m, 0, 7), vec![MidiTarget::MixerVolume(2)]);
        // same target moved to another knob
        learn(&mut m, 1, 20, MidiTarget::MixerPan(1));
        assert!(targets(&m, 0, 10).is_empty());
        assert_eq!(targets(&m, 1, 20), vec![MidiTarget::MixerPan(1)]);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn mappings_roundtrip_in_settings_format() {
        let m = vec![CcMapping { midi_channel: 0, cc: 74, target: MidiTarget::SynthCutoff(3) }];
        let s = ron::to_string(&m).unwrap();
        assert_eq!(ron::from_str::<Vec<CcMapping>>(&s).unwrap(), m);
    }
}
