//! Automation clips: a parameter path plus a breakpoint curve in the playlist.
//!
//! Target paths (indices, not ids):
//! * `mixer.N.volume`, `mixer.N.pan` (N = 0 is the master)
//! * `mixer.N.fx.S.FIELD` (FIELD = a numeric effect parameter, e.g. `cutoff`, `mix`)
//! * `channel.N.volume`, `channel.N.pan`
//! * `channel.N.synth.FIELD` (TSynth channels, e.g. `cutoff`, `resonance`, `drive_db`)

use crate::{Instrument, Project};
use serde::{Deserialize, Serialize};
use tstudio_dsp::synth::SynthParams;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AutomationClip {
    pub target: String,
    /// sorted by `t`; `t` in ticks relative to the clip start, `v` in the parameter's own unit
    pub points: Vec<AutoPoint>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutoPoint {
    pub t: i64,
    pub v: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutoTarget {
    TrackVolume(usize),
    TrackPan(usize),
    ChannelVolume(usize),
    ChannelPan(usize),
    Fx { track: usize, slot: usize, field: usize },
    Synth { channel: usize, field: usize },
}

/// Value range of a parameter: (min, max, logarithmic).
pub fn field_range(field: &str) -> (f32, f32, bool) {
    match field {
        "volume" => (0.0, 1.25, false),
        "pan" => (-1.0, 1.0, false),
        "cutoff" | "low_freq" | "mid_freq" | "high_freq" | "tone" => (20.0, 20000.0, true),
        "low_gain" | "mid_gain" | "high_gain" | "gain_db" | "makeup" => (-24.0, 24.0, false),
        "output_db" => (-24.0, 12.0, false),
        "drive_db" => (0.0, 40.0, false),
        "threshold" | "threshold_db" => (-60.0, 0.0, false),
        "ceiling_db" => (-12.0, 0.0, false),
        "ratio" => (1.0, 20.0, false),
        "knee" => (0.0, 24.0, false),
        "attack_ms" => (0.1, 200.0, true),
        "release_ms" => (5.0, 2000.0, true),
        "predelay_ms" => (0.0, 200.0, false),
        "time_beats" => (0.0625, 2.0, false),
        "feedback" => (0.0, 0.95, false),
        "rate" => (0.05, 10.0, true),
        "depth_ms" => (0.0, 10.0, false),
        "delay_ms" => (1.0, 30.0, false),
        "mid_q" => (0.1, 10.0, true),
        "bits" => (1.0, 16.0, false),
        "downsample" => (1.0, 32.0, false),
        "filter_env_amount" => (-8.0, 8.0, false),
        "pitch_env_amount" => (-24.0, 72.0, false),
        "pitch_env_decay" => (0.001, 1.0, true),
        "glide" => (0.0, 1.0, false),
        "filter2_cutoff" => (20.0, 20000.0, true),
        "filter_drive" | "kick_drive_db" | "fx_dist_drive_db" => (0.0, 40.0, false),
        "kick_tune" => (12.0, 48.0, false),
        "kick_pitch_start" => (0.0, 72.0, false),
        "kick_pitch_time" => (0.005, 0.5, true),
        "gain" => (0.0, 1.5, false),
        _ => (0.0, 1.0, false),
    }
}

/// Maps a value into 0..1 of its range (log-aware), for drawing.
pub fn normalize(v: f32, (lo, hi, log): (f32, f32, bool)) -> f32 {
    let n = if log && lo > 0.0 { (v.max(lo).ln() - lo.ln()) / (hi.ln() - lo.ln()) } else { (v - lo) / (hi - lo) };
    n.clamp(0.0, 1.0)
}

pub fn denormalize(n: f32, (lo, hi, log): (f32, f32, bool)) -> f32 {
    let n = n.clamp(0.0, 1.0);
    if log && lo > 0.0 {
        (lo.ln() + (hi.ln() - lo.ln()) * n).exp()
    } else {
        lo + (hi - lo) * n
    }
}

impl AutomationClip {
    /// Value at `t` ticks relative to the clip start (holds the first/last point outside).
    pub fn value_at(&self, t: f64, log: bool) -> Option<f32> {
        value_at(&self.points.iter().map(|p| (p.t, p.v)).collect::<Vec<_>>(), t, log)
    }
}

/// Interpolates sorted `(tick, value)` points; log interpolation for frequency-like ranges.
pub fn value_at(points: &[(i64, f32)], t: f64, log: bool) -> Option<f32> {
    let first = points.first()?;
    if t <= first.0 as f64 {
        return Some(first.1);
    }
    let i = points.partition_point(|p| (p.0 as f64) <= t);
    if i >= points.len() {
        return Some(points[points.len() - 1].1);
    }
    let (a, b) = (points[i - 1], points[i]);
    let span = (b.0 - a.0) as f64;
    let f = if span <= 0.0 { 1.0 } else { ((t - a.0 as f64) / span) as f32 };
    Some(if log && a.1 > 0.0 && b.1 > 0.0 { (a.1.ln() + (b.1.ln() - a.1.ln()) * f).exp() } else { a.1 + (b.1 - a.1) * f })
}

impl Project {
    /// Resolves a target path against the current project.
    pub fn parse_auto_target(&self, path: &str) -> Result<AutoTarget, String> {
        let parts: Vec<&str> = path.trim().split('.').collect();
        let idx = |s: &str| s.parse::<usize>().map_err(|_| format!("'{s}' ist kein Index in '{path}'"));
        match parts.as_slice() {
            ["mixer", n, rest @ ..] => {
                let t = idx(n)?;
                let track = self.mixer.get(t).ok_or_else(|| format!("Mixer-Spur {t} gibt es nicht"))?;
                match rest {
                    ["volume"] => Ok(AutoTarget::TrackVolume(t)),
                    ["pan"] => Ok(AutoTarget::TrackPan(t)),
                    ["fx", s, field] => {
                        let slot = idx(s)?;
                        let fx = track.fx.get(slot).ok_or_else(|| format!("Mixer-Spur {t} hat keinen FX-Slot {slot}"))?;
                        let names = fx.params.field_names();
                        let field = names
                            .iter()
                            .position(|f| f == field)
                            .ok_or_else(|| format!("{} hat keinen Parameter '{field}' (verfügbar: {})", fx.params.kind().name(), names.join(", ")))?;
                        Ok(AutoTarget::Fx { track: t, slot, field })
                    }
                    _ => Err(format!("unbekanntes Ziel '{path}' (mixer.N.volume | mixer.N.pan | mixer.N.fx.S.FELD)")),
                }
            }
            ["channel", n, rest @ ..] => {
                let c = idx(n)?;
                let ch = self.channels.get(c).ok_or_else(|| format!("Kanal {c} gibt es nicht"))?;
                match rest {
                    ["volume"] => Ok(AutoTarget::ChannelVolume(c)),
                    ["pan"] => Ok(AutoTarget::ChannelPan(c)),
                    ["synth", field] => {
                        if !matches!(ch.instrument, Instrument::Synth(_)) {
                            return Err(format!("Kanal {c} ist kein TSynth"));
                        }
                        let field = SynthParams::FIELDS
                            .iter()
                            .position(|f| f == field)
                            .ok_or_else(|| format!("TSynth hat keinen Parameter '{field}' (verfügbar: {})", SynthParams::FIELDS.join(", ")))?;
                        Ok(AutoTarget::Synth { channel: c, field })
                    }
                    _ => Err(format!("unbekanntes Ziel '{path}' (channel.N.volume | channel.N.pan | channel.N.synth.FELD)")),
                }
            }
            _ => Err(format!("unbekanntes Ziel '{path}' (beginnt mit mixer. oder channel.)")),
        }
    }

    /// Name of the last path segment (for ranges).
    pub fn auto_field_name(path: &str) -> &str {
        path.rsplit('.').next().unwrap_or("")
    }

    /// Current (un-automated) value of a target.
    pub fn auto_base_value(&self, t: AutoTarget) -> f32 {
        match t {
            AutoTarget::TrackVolume(i) => self.mixer[i].volume,
            AutoTarget::TrackPan(i) => self.mixer[i].pan,
            AutoTarget::ChannelVolume(i) => self.channels[i].volume,
            AutoTarget::ChannelPan(i) => self.channels[i].pan,
            AutoTarget::Fx { track, slot, field } => self.mixer[track].fx[slot].params.field(field).unwrap_or(0.0),
            AutoTarget::Synth { channel, field } => match &self.channels[channel].instrument {
                Instrument::Synth(p) => p.field(field).unwrap_or(0.0),
                _ => 0.0,
            },
        }
    }

    /// Readable label of a target path ("Bass · Filter · cutoff"), or the path if invalid.
    pub fn auto_label(&self, path: &str) -> String {
        let field = Project::auto_field_name(path);
        match self.parse_auto_target(path) {
            Ok(AutoTarget::TrackVolume(i)) => format!("{} · Lautstärke", self.mixer[i].name),
            Ok(AutoTarget::TrackPan(i)) => format!("{} · Pan", self.mixer[i].name),
            Ok(AutoTarget::ChannelVolume(i)) => format!("{} · Lautstärke", self.channels[i].name),
            Ok(AutoTarget::ChannelPan(i)) => format!("{} · Pan", self.channels[i].name),
            Ok(AutoTarget::Fx { track, slot, .. }) => format!("{} · {} · {field}", self.mixer[track].name, self.mixer[track].fx[slot].params.kind().name()),
            Ok(AutoTarget::Synth { channel, .. }) => format!("{} · {field}", self.channels[channel].name),
            Err(_) => format!("⚠ {path}"),
        }
    }

    /// Every automatable target path with a readable label.
    pub fn automation_targets(&self) -> Vec<(String, String)> {
        let mut v = Vec::new();
        for (i, t) in self.mixer.iter().enumerate() {
            v.push((format!("mixer.{i}.volume"), format!("{} · Lautstärke", t.name)));
            v.push((format!("mixer.{i}.pan"), format!("{} · Pan", t.name)));
            for (s, fx) in t.fx.iter().enumerate() {
                for f in fx.params.field_names() {
                    v.push((format!("mixer.{i}.fx.{s}.{f}"), format!("{} · {} · {f}", t.name, fx.params.kind().name())));
                }
            }
        }
        for (i, c) in self.channels.iter().enumerate() {
            v.push((format!("channel.{i}.volume"), format!("{} · Lautstärke", c.name)));
            v.push((format!("channel.{i}.pan"), format!("{} · Pan", c.name)));
            if matches!(c.instrument, Instrument::Synth(_)) {
                for f in SynthParams::FIELDS {
                    v.push((format!("channel.{i}.synth.{f}"), format!("{} · {f}", c.name)));
                }
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FxSlot;
    use tstudio_dsp::fx::EffectKind;

    #[test]
    fn interpolates_and_holds() {
        let pts = [(0, 0.0), (100, 1.0)];
        assert_eq!(value_at(&pts, -5.0, false), Some(0.0));
        assert!((value_at(&pts, 50.0, false).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(value_at(&pts, 500.0, false), Some(1.0));
        let f = value_at(&[(0, 100.0), (100, 10000.0)], 50.0, true).unwrap();
        assert!((f - 1000.0).abs() < 1.0, "{f}");
    }

    #[test]
    fn parses_targets() {
        let mut p = Project::new_default();
        p.mixer[2].fx.push(FxSlot::new(EffectKind::Filter));
        assert_eq!(p.parse_auto_target("mixer.2.fx.0.cutoff"), Ok(AutoTarget::Fx { track: 2, slot: 0, field: 0 }));
        assert_eq!(p.parse_auto_target("mixer.0.volume"), Ok(AutoTarget::TrackVolume(0)));
        assert!(matches!(p.parse_auto_target("channel.0.synth.cutoff"), Ok(AutoTarget::Synth { channel: 0, .. })));
        assert!(p.parse_auto_target("mixer.2.fx.0.nope").is_err());
        assert!(p.parse_auto_target("mixer.99.volume").is_err());
        assert!(p.automation_targets().iter().any(|(t, _)| t == "mixer.2.fx.0.cutoff"));
    }
}
