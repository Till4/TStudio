//! TStudio project model (FL-style): channel rack, patterns, playlist, mixer.
//! Pure data + serde; the engine receives a compiled snapshot of it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tstudio_dsp::fx::{EffectKind, EffectParams, LimiterParams};
use tstudio_dsp::sampler::SamplerParams;
use tstudio_dsp::stretch::StretchMode;
use tstudio_dsp::synth::SynthParams;

pub mod automation;
pub mod change;
pub mod history;
pub mod meter;
pub mod midi;
pub mod templates;
pub mod tempo;
pub mod steps;
pub mod takes;
pub mod tracks;
pub use automation::{AutoPoint, AutoTarget, AutomationClip};
pub use change::{describe_change, Change};
pub use meter::{BarMap, MeterChange};
pub use tempo::{TempoMap, TempoPoint};
pub use history::History;

/// Ticks per quarter note.
pub const PPQ: i64 = 96;
pub const FILE_VERSION: u32 = 1;
/// A new project starts small; inserts and playlist tracks are added as needed.
pub const DEFAULT_MIXER_TRACKS: usize = 4;
pub const DEFAULT_PLAYLIST_TRACKS: usize = 4;
pub const MAX_FX_SLOTS: usize = 8;

pub type Id = u64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub bpm: f64,
    pub beats_per_bar: u32,
    /// 0..1: delays every second 16th note (0 = straight, 1 = hard shuffle)
    pub swing: f32,
    pub channels: Vec<Channel>,
    pub patterns: Vec<Pattern>,
    pub playlist: Playlist,
    /// index 0 = master
    pub mixer: Vec<MixerTrack>,
    pub next_id: Id,
    /// tempo changes after tick 0 (`bpm` is the start tempo), sorted by tick
    pub tempo: Vec<TempoPoint>,
    /// time-signature changes at bars (`beats_per_bar` is the meter of bar 1), sorted by bar
    pub meters: Vec<MeterChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Channel {
    pub id: Id,
    pub name: String,
    pub color: [u8; 3],
    pub instrument: Instrument,
    /// mixer insert index (0 = master)
    pub mixer_track: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    /// when any channel is solo, only solo channels sound
    pub solo: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Channel".into(),
            color: [120, 160, 220],
            instrument: Instrument::Synth(SynthParams::default()),
            mixer_track: 0,
            volume: 0.8,
            pan: 0.0,
            muted: false,
            solo: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Instrument {
    Sampler(SamplerInstrument),
    Synth(SynthParams),
    SoundFont(SoundFontInstrument),
}

impl Instrument {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Instrument::Sampler(_) => "Sampler",
            Instrument::Synth(_) => "TSynth",
            Instrument::SoundFont(_) => "SoundFont",
        }
    }

    /// Key used by the step sequencer and auditioning.
    pub fn default_key(&self) -> u8 {
        match self {
            Instrument::Sampler(s) => s.params.root_note,
            Instrument::Synth(_) => 60,
            // GM drum kits: 36 = kick
            Instrument::SoundFont(s) => {
                if s.bank >= 128 {
                    36
                } else {
                    60
                }
            }
        }
    }
}

/// A preset of an .sf2 file (bank 128 = drum kits).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SoundFontInstrument {
    pub path: PathBuf,
    pub bank: u16,
    pub program: u8,
    /// display only
    pub preset_name: String,
    /// linear output gain
    pub gain: f32,
}

impl Default for SoundFontInstrument {
    fn default() -> Self {
        Self { path: PathBuf::new(), bank: 0, program: 0, preset_name: String::new(), gain: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SamplerInstrument {
    pub sample: Option<PathBuf>,
    pub params: SamplerParams,
    pub stretch: StretchSettings,
}

/// How a sample is fitted to the project tempo.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StretchSettings {
    pub mode: StretchMode,
    /// Tempo the sample was recorded at.
    pub source_bpm: f64,
    /// None = follow project tempo.
    pub target_bpm: Option<f64>,
    /// Extra pitch shift (semitones), works in every mode.
    pub pitch: f32,
    /// keep formants (vocal character) when pitching
    pub formant: bool,
}

impl Default for StretchSettings {
    fn default() -> Self {
        Self { mode: StretchMode::Off, source_bpm: 120.0, target_bpm: None, pitch: 0.0, formant: false }
    }
}

impl StretchSettings {
    pub fn effective_target(&self, project_bpm: f64) -> f64 {
        self.target_bpm.unwrap_or(project_bpm)
    }
    pub fn is_active(&self) -> bool {
        self.mode != StretchMode::Off
    }
    /// (mode, source bpm, target bpm) for `change_tempo`; `pitch` applies in every mode
    /// (with Off or an unknown sample tempo only the pitch is shifted, the length stays).
    pub fn tempo_job(&self, project_bpm: f64) -> (StretchMode, f64, f64) {
        if self.mode == StretchMode::Off || self.source_bpm <= 0.0 {
            (StretchMode::Off, 1.0, 1.0)
        } else {
            (self.mode, self.source_bpm, self.effective_target(project_bpm))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Pattern {
    pub id: Id,
    pub name: String,
    pub color: [u8; 3],
    pub notes: Vec<Note>,
    /// Fixed length in ticks; None = automatic (whole bars covering all notes).
    pub length: Option<i64>,
}

impl Default for Pattern {
    fn default() -> Self {
        Self { id: 0, name: "Pattern 1".into(), color: [110, 200, 140], notes: Vec::new(), length: None }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Note {
    /// channel id
    pub channel: Id,
    pub key: u8,
    pub start: i64,
    pub len: i64,
    pub velocity: f32,
}

impl Note {
    pub fn end(&self) -> i64 {
        self.start + self.len
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Playlist {
    pub tracks: Vec<PlaylistTrack>,
    pub clips: Vec<Clip>,
    /// (start, end) in ticks
    pub loop_region: Option<(i64, i64)>,
    /// song sections ("Intro", "Drop" …), sorted by position
    pub markers: Vec<Marker>,
}

/// Named position in the song (section start).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Marker {
    pub tick: i64,
    pub name: String,
}

impl Playlist {
    /// Marker by name (case-insensitive).
    pub fn marker(&self, name: &str) -> Option<&Marker> {
        self.markers.iter().find(|m| m.name.eq_ignore_ascii_case(name.trim()))
    }
    pub fn sort_markers(&mut self) {
        self.markers.sort_by_key(|m| m.tick);
    }
}

impl Default for Playlist {
    fn default() -> Self {
        Self {
            tracks: (1..=DEFAULT_PLAYLIST_TRACKS).map(|i| PlaylistTrack { name: format!("Track {i}"), ..Default::default() }).collect(),
            clips: Vec::new(),
            loop_region: None,
            markers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlaylistTrack {
    pub name: String,
    pub muted: bool,
    pub solo: bool,
    /// linear gain for everything on this track (audio clips and pattern notes)
    pub volume: f32,
    /// Recording lanes: tracks with the same id are the takes of one take group (see `takes.rs`).
    pub take_group: Option<Id>,
}

impl Default for PlaylistTrack {
    fn default() -> Self {
        Self { name: String::new(), muted: false, solo: false, volume: 1.0, take_group: None }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub id: Id,
    pub track: usize,
    pub start: i64,
    pub len: i64,
    pub kind: ClipKind,
    #[serde(default)]
    pub muted: bool,
}

impl Clip {
    pub fn end(&self) -> i64 {
        self.start + self.len
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ClipKind {
    Pattern { pattern: Id },
    Audio(AudioClip),
    Automation(AutomationClip),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioClip {
    pub path: PathBuf,
    /// Offset into the (tempo-fitted) audio, in ticks.
    pub offset: i64,
    pub gain: f32,
    pub stretch: StretchSettings,
    pub mixer_track: usize,
    /// fade in/out in ticks
    pub fade_in: i64,
    pub fade_out: i64,
}

impl Default for AudioClip {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            offset: 0,
            gain: 1.0,
            stretch: StretchSettings::default(),
            mixer_track: 0,
            fade_in: 0,
            fade_out: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MixerTrack {
    pub name: String,
    /// linear gain
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub fx: Vec<FxSlot>,
    /// where the track's output goes: 0 = master, otherwise a higher-numbered insert (bus)
    pub output: usize,
    /// post-fader sends to higher-numbered inserts (reverb/rumble buses)
    pub sends: Vec<MixerSend>,
}

impl Default for MixerTrack {
    fn default() -> Self {
        Self { name: "Insert".into(), volume: 0.8, pan: 0.0, muted: false, solo: false, fx: Vec::new(), output: 0, sends: Vec::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MixerSend {
    pub target: usize,
    /// linear gain
    pub amount: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FxSlot {
    pub enabled: bool,
    pub params: EffectParams,
}

impl FxSlot {
    pub fn new(kind: EffectKind) -> Self {
        Self { enabled: true, params: kind.default_params() }
    }
}

const PALETTE: [[u8; 3]; 8] = [
    [230, 120, 100],
    [240, 180, 90],
    [200, 210, 100],
    [110, 200, 140],
    [100, 190, 220],
    [130, 140, 230],
    [190, 120, 220],
    [230, 120, 180],
];

pub fn palette_color(i: usize) -> [u8; 3] {
    PALETTE[i % PALETTE.len()]
}

impl Default for Project {
    fn default() -> Self {
        let mut mixer = vec![MixerTrack {
            name: "Master".into(),
            volume: 1.0,
            fx: vec![FxSlot { enabled: true, params: EffectParams::Limiter(LimiterParams::default()) }],
            ..Default::default()
        }];
        mixer.extend((1..=DEFAULT_MIXER_TRACKS).map(|i| MixerTrack { name: format!("Insert {i}"), ..Default::default() }));
        Self {
            version: FILE_VERSION,
            name: "Untitled".into(),
            bpm: 140.0,
            beats_per_bar: 4,
            swing: 0.0,
            channels: Vec::new(),
            patterns: Vec::new(),
            playlist: Playlist::default(),
            mixer,
            next_id: 1,
            tempo: Vec::new(),
            meters: Vec::new(),
        }
    }
}

impl Project {
    /// A fresh project with a synth channel and one pattern.
    pub fn new_default() -> Self {
        let mut p = Project::default();
        let pat = p.alloc_id();
        p.patterns.push(Pattern { id: pat, ..Default::default() });
        p.add_channel("TSynth", Instrument::Synth(SynthParams::default()));
        p
    }

    pub fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn ticks_per_bar(&self) -> i64 {
        PPQ * self.beats_per_bar.max(1) as i64
    }

    pub fn ticks_to_secs(&self, ticks: f64) -> f64 {
        ticks / PPQ as f64 * 60.0 / self.bpm
    }

    pub fn secs_to_ticks(&self, secs: f64) -> f64 {
        secs * self.bpm / 60.0 * PPQ as f64
    }

/// Adds a channel routed to a free, untouched mixer insert (one is appended when none is left).
    /// Returns its index.
    pub fn add_channel(&mut self, name: &str, instrument: Instrument) -> usize {
        let id = self.alloc_id();
        let idx = self.channels.len();
        let mut used: Vec<usize> = self.channels.iter().map(|c| c.mixer_track).collect();
        for c in &self.playlist.clips {
            if let ClipKind::Audio(a) = &c.kind {
                used.push(a.mixer_track);
            }
        }
        for t in &self.mixer {
            used.push(t.output);
            used.extend(t.sends.iter().map(|s| s.target));
        }
        let free = (1..self.mixer.len()).find(|t| !used.contains(t) && self.mixer[*t].name.starts_with("Insert") && self.mixer[*t].fx.is_empty());
        let track = match free {
            Some(t) => {
                self.mixer[t].name = name.to_string();
                t
            }
            None => self.add_mixer_track(Some(name.to_string())),
        };
        self.channels.push(Channel { id, name: name.to_string(), color: palette_color(idx), instrument, mixer_track: track, ..Default::default() });
        idx
    }

    pub fn remove_channel(&mut self, idx: usize) {
        if idx >= self.channels.len() {
            return;
        }
        let id = self.channels[idx].id;
        self.channels.remove(idx);
        for p in self.patterns.iter_mut() {
            p.notes.retain(|n| n.channel != id);
        }
    }

    pub fn channel_index(&self, id: Id) -> Option<usize> {
        self.channels.iter().position(|c| c.id == id)
    }

    pub fn pattern(&self, id: Id) -> Option<&Pattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    pub fn pattern_mut(&mut self, id: Id) -> Option<&mut Pattern> {
        self.patterns.iter_mut().find(|p| p.id == id)
    }

    pub fn add_pattern(&mut self) -> Id {
        let id = self.alloc_id();
        let n = self.patterns.len();
        self.patterns.push(Pattern { id, name: format!("Pattern {}", n + 1), color: palette_color(n + 3), ..Default::default() });
        id
    }

    pub fn remove_pattern(&mut self, id: Id) {
        self.patterns.retain(|p| p.id != id);
        self.playlist.clips.retain(|c| !matches!(c.kind, ClipKind::Pattern { pattern } if pattern == id));
    }

    /// Pattern length in ticks (whole bars, at least one).
    pub fn pattern_length(&self, id: Id) -> i64 {
        let bar = self.ticks_per_bar();
        match self.pattern(id) {
            None => bar,
            Some(p) => {
                if let Some(l) = p.length {
                    return l.max(PPQ / 4);
                }
                let end = p.notes.iter().map(|n| n.end()).max().unwrap_or(0);
                ((end + bar - 1) / bar).max(1) * bar
            }
        }
    }

    /// End of the last clip in the playlist.
    pub fn song_length(&self) -> i64 {
        self.playlist.clips.iter().map(|c| c.end()).max().unwrap_or(0)
    }

    pub fn add_clip(&mut self, track: usize, start: i64, len: i64, kind: ClipKind) -> Id {
        let id = self.alloc_id();
        while self.playlist.tracks.len() <= track {
            let n = self.playlist.tracks.len() + 1;
            self.playlist.tracks.push(PlaylistTrack { name: format!("Track {n}"), ..Default::default() });
        }
        self.playlist.clips.push(Clip { id, track, start: start.max(0), len: len.max(1), kind, muted: false });
        id
    }

    /// Every sample / SoundFont file referenced by the project.
    pub fn referenced_samples(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = Vec::new();
        for c in &self.channels {
            match &c.instrument {
                Instrument::Sampler(s) => {
                    if let Some(p) = &s.sample {
                        v.push(p.clone());
                    }
                }
                Instrument::SoundFont(s) => v.push(s.path.clone()),
                Instrument::Synth(_) => {}
            }
        }
        for c in &self.playlist.clips {
            if let ClipKind::Audio(a) = &c.kind {
                v.push(a.path.clone());
            }
        }
        v.sort();
        v.dedup();
        v
    }

    pub fn to_ron(&self) -> Result<String, String> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default().struct_names(false)).map_err(|e| e.to_string())
    }

    pub fn from_ron(s: &str) -> Result<Project, String> {
        let mut p: Project = ron::from_str(s).map_err(|e| e.to_string())?;
        p.sanitize();
        Ok(p)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let s = self.to_ron()?;
        let tmp = path.with_extension("tsp.tmp");
        std::fs::write(&tmp, s).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn load(path: &Path) -> Result<Project, String> {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Project::from_ron(&s)
    }

    /// Repairs indices after loading or heavy editing.
    pub fn sanitize(&mut self) {
        if self.mixer.is_empty() {
            self.mixer = Project::default().mixer;
        }
        let max_track = self.mixer.len() - 1;
        for c in self.channels.iter_mut() {
            c.mixer_track = c.mixer_track.min(max_track);
        }
        for clip in self.playlist.clips.iter_mut() {
            if let ClipKind::Audio(a) = &mut clip.kind {
                a.mixer_track = a.mixer_track.min(max_track);
            }
        }
        // every clip needs a visible playlist track
        if let Some(max) = self.playlist.clips.iter().map(|c| c.track).max() {
            while self.playlist.tracks.len() <= max {
                let n = self.playlist.tracks.len() + 1;
                self.playlist.tracks.push(PlaylistTrack { name: format!("Track {n}"), ..Default::default() });
            }
        }
        let n_tracks = self.mixer.len();
        for (i, t) in self.mixer.iter_mut().enumerate() {
            t.fx.truncate(MAX_FX_SLOTS);
            if i == 0 || t.output <= i || t.output >= n_tracks {
                t.output = 0;
            }
            t.sends.retain(|s| s.target > i && s.target < n_tracks);
            if i == 0 {
                t.sends.clear();
            }
        }
        self.swing = self.swing.clamp(0.0, 1.0);
        if self.bpm <= 0.0 {
            self.bpm = 120.0;
        }
        self.sort_tempo();
        self.sort_meters();
        let max_id = self
            .channels
            .iter()
            .map(|c| c.id)
            .chain(self.patterns.iter().map(|p| p.id))
            .chain(self.playlist.clips.iter().map(|c| c.id))
            .chain(self.playlist.tracks.iter().filter_map(|t| t.take_group))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max_id + 1);
        let ids: Vec<Id> = self.channels.iter().map(|c| c.id).collect();
        for p in self.patterns.iter_mut() {
            p.notes.retain(|n| ids.contains(&n.channel) && n.len > 0);
        }
        let pats: Vec<Id> = self.patterns.iter().map(|p| p.id).collect();
        self.playlist.clips.retain(|c| match c.kind {
            ClipKind::Pattern { pattern } => pats.contains(&pattern),
            ClipKind::Audio(_) | ClipKind::Automation(_) => true,
        });
        for c in self.playlist.clips.iter_mut() {
            if let ClipKind::Automation(a) = &mut c.kind {
                a.points.sort_by_key(|p| p.t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_without_the_newer_fields_load() {
        // projects from before tempo track, time-signature changes and take lanes
        let mut p = Project::new_default();
        p.playlist.tracks.push(PlaylistTrack { name: "Vocals".into(), ..Default::default() });
        let s = p.to_ron().unwrap();
        let old: String = s
            .lines()
            .filter(|l| !["tempo:", "meters:", "take_group:"].iter().any(|k| l.trim_start().starts_with(k)))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!old.contains("take_group") && !old.contains("meters"));
        let q = Project::from_ron(&old).unwrap();
        assert!(q.tempo.is_empty() && q.meters.is_empty());
        assert!(q.playlist.tracks.iter().all(|t| t.take_group.is_none()));
        assert_eq!(q.playlist.tracks.len(), p.playlist.tracks.len());
    }
    use tstudio_dsp::fx::EffectKind;

    #[test]
    fn roundtrip_ron() {
        let mut p = Project::new_default();
        while p.mixer.len() < 17 {
            p.add_mixer_track(None);
        }
        let ch = p.add_channel("Kick", Instrument::Sampler(SamplerInstrument { sample: Some("C:/s/kick.wav".into()), ..Default::default() }));
        let cid = p.channels[ch].id;
        let pat = p.patterns[0].id;
        p.pattern_mut(pat).unwrap().notes.push(Note { channel: cid, key: 60, start: 0, len: 24, velocity: 0.8 });
        p.add_clip(0, 0, p.ticks_per_bar(), ClipKind::Pattern { pattern: pat });
        p.add_clip(
            1,
            384,
            768,
            ClipKind::Audio(AudioClip {
                path: "C:/s/loop.wav".into(),
                stretch: StretchSettings { mode: StretchMode::Stretch, source_bpm: 92.0, ..Default::default() },
                ..Default::default()
            }),
        );
        p.mixer[1].fx.push(FxSlot::new(EffectKind::Reverb));
        p.mixer[1].sends.push(MixerSend { target: 5, amount: 0.4 });
        p.swing = 0.3;
        p.add_channel("Strings", Instrument::SoundFont(SoundFontInstrument { path: "C:/sf/x.sf2".into(), program: 48, ..Default::default() }));
        p.add_clip(
            2,
            0,
            768,
            ClipKind::Automation(AutomationClip { target: "mixer.1.fx.0.mix".into(), points: vec![AutoPoint { t: 0, v: 0.0 }, AutoPoint { t: 768, v: 1.0 }] }),
        );
        let s = p.to_ron().unwrap();
        let q = Project::from_ron(&s).unwrap();
        assert_eq!(p, q);
    }

    #[test]
    fn pattern_length_rounds_to_bars() {
        let mut p = Project::new_default();
        let pat = p.patterns[0].id;
        let cid = p.channels[0].id;
        assert_eq!(p.pattern_length(pat), 384);
        p.pattern_mut(pat).unwrap().notes.push(Note { channel: cid, key: 60, start: 380, len: 30, velocity: 1.0 });
        assert_eq!(p.pattern_length(pat), 768);
    }

    #[test]
    fn removing_channel_removes_its_notes() {
        let mut p = Project::new_default();
        let pat = p.patterns[0].id;
        let cid = p.channels[0].id;
        p.pattern_mut(pat).unwrap().notes.push(Note { channel: cid, key: 60, start: 0, len: 24, velocity: 1.0 });
        p.remove_channel(0);
        assert!(p.pattern(pat).unwrap().notes.is_empty());
    }
}
