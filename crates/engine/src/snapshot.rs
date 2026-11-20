use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tstudio_dsp::fx::{Effect, EffectParams};
use tstudio_dsp::sampler::{Sampler, SamplerParams};
use tstudio_dsp::synth::{Synth, SynthParams};
use tstudio_dsp::SampleData;
use tstudio_project::automation::value_at;
use tstudio_project::{AutoTarget, TempoMap};

use crate::soundfont::{SfProc, Sf2};

/// Immutable, engine-ready view of the project. Indices replace ids.
pub struct Snapshot {
    /// start tempo (pattern mode always plays at this tempo)
    pub bpm: f64,
    /// song tempo curve (tempo track); constant `bpm` when the project has no tempo points
    pub tempo: TempoMap,
    pub beats_per_bar: u32,
    pub channels: Vec<ChannelSnap>,
    pub patterns: Vec<PatternSnap>,
    pub clips: Vec<ClipSnap>,
    /// end of the song in ticks
    pub song_end: i64,
    pub loop_region: Option<(i64, i64)>,
    /// index 0 = master
    pub mixer: Vec<TrackSnap>,
    /// one lane per automated parameter
    pub autos: Vec<AutoLane>,
}

pub struct AutoLane {
    pub target: AutoTarget,
    pub log: bool,
    /// sorted by start
    pub clips: Vec<AutoClipSnap>,
}

pub struct AutoClipSnap {
    pub start: i64,
    pub len: i64,
    /// (tick relative to clip start, value)
    pub points: Vec<(i64, f32)>,
}

impl AutoLane {
    /// Value at song position `pos` (ticks): interpolated inside a clip, held after it,
    /// None before the first clip.
    pub fn value(&self, pos: f64) -> Option<f32> {
        let i = self.clips.partition_point(|c| c.start as f64 <= pos);
        let c = self.clips.get(i.checked_sub(1)?)?;
        let local = (pos - c.start as f64).min(c.len as f64);
        value_at(&c.points, local, self.log)
    }
}

pub struct ChannelSnap {
    pub instrument: InstrumentSnap,
    pub mixer_track: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
}

pub enum InstrumentSnap {
    Sampler { params: SamplerParams, sample: Option<Arc<SampleData>> },
    Synth(SynthParams),
    SoundFont { font: Option<Arc<Sf2>>, bank: u16, program: u8, gain: f32 },
}

pub struct PatternSnap {
    pub length: i64,
    /// sorted by start
    pub notes: Vec<NoteSnap>,
}

#[derive(Clone, Copy, Debug)]
pub struct NoteSnap {
    pub channel: usize,
    pub key: u8,
    pub start: i64,
    pub len: i64,
    pub velocity: f32,
}

pub enum ClipSnap {
    Pattern {
        pattern: usize,
        start: i64,
        len: i64,
        /// playlist-track volume, applied to every note
        gain: f32,
    },
    Audio {
        /// already tempo-fitted and at engine sample rate
        sample: Arc<SampleData>,
        start: i64,
        len: i64,
        offset: i64,
        gain: f32,
        mixer_track: usize,
        fade_in: i64,
        fade_out: i64,
        /// tempo the sample was fitted to (tempo at the clip start)
        fit_bpm: f64,
        /// follows the project tempo: on tempo changes inside the clip the read position
        /// tracks the song position (varispeed); otherwise the sample plays in real time
        follow: bool,
    },
}

pub struct TrackSnap {
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub fx: Vec<FxSnap>,
    /// 0 = master, else a higher insert
    pub output: usize,
    /// (target insert, linear amount)
    pub sends: Vec<(usize, f32)>,
    /// receives other tracks (never silenced by solo)
    pub is_bus: bool,
}

impl TrackSnap {
    pub fn new(volume: f32) -> TrackSnap {
        TrackSnap { volume, pan: 0.0, muted: false, solo: false, fx: Vec::new(), output: 0, sends: Vec::new(), is_bus: false }
    }
}

pub struct FxSnap {
    pub enabled: bool,
    pub params: EffectParams,
}

impl Snapshot {
    pub fn empty() -> Snapshot {
        Snapshot {
            bpm: 120.0,
            tempo: TempoMap::constant(120.0),
            beats_per_bar: 4,
            channels: Vec::new(),
            patterns: Vec::new(),
            clips: Vec::new(),
            song_end: 0,
            loop_region: None,
            mixer: vec![TrackSnap::new(1.0)],
            autos: Vec::new(),
        }
    }

    /// Hash of everything that requires rebuilding processors.
    pub fn signature(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.channels.len().hash(&mut h);
        for c in &self.channels {
            match &c.instrument {
                InstrumentSnap::Sampler { .. } => 1u8.hash(&mut h),
                InstrumentSnap::Synth(_) => 2u8.hash(&mut h),
                InstrumentSnap::SoundFont { font, .. } => {
                    3u8.hash(&mut h);
                    font.as_ref().map(|f| Arc::as_ptr(f) as usize).unwrap_or(0).hash(&mut h);
                }
            }
        }
        self.mixer.len().hash(&mut h);
        for t in &self.mixer {
            t.fx.len().hash(&mut h);
            for f in &t.fx {
                f.params.kind().hash(&mut h);
            }
        }
        h.finish()
    }
}

pub enum ChannelProc {
    Sampler(Sampler),
    Synth(Synth),
    SoundFont(Box<SfProc>),
    /// SoundFont that could not be loaded
    Empty,
}

impl ChannelProc {
    pub fn release_all(&mut self) {
        match self {
            ChannelProc::Sampler(s) => s.release_all(),
            ChannelProc::Synth(s) => s.release_all(),
            ChannelProc::SoundFont(s) => s.release_all(),
            ChannelProc::Empty => {}
        }
    }
    pub fn kill_all(&mut self) {
        match self {
            ChannelProc::Sampler(s) => s.kill_all(),
            ChannelProc::Synth(s) => s.kill_all(),
            ChannelProc::SoundFont(s) => s.kill_all(),
            ChannelProc::Empty => {}
        }
    }
    pub fn is_active(&self) -> bool {
        match self {
            ChannelProc::Sampler(s) => s.active_voices() > 0,
            ChannelProc::Synth(s) => s.is_sounding(),
            ChannelProc::SoundFont(s) => s.is_active(),
            ChannelProc::Empty => false,
        }
    }
}

pub struct TrackProc {
    pub fx: Vec<Effect>,
    pub l: Vec<f32>,
    pub r: Vec<f32>,
}

/// Stateful processors matching a snapshot's structure. Built off the audio thread.
pub struct Rack {
    pub signature: u64,
    pub channels: Vec<ChannelProc>,
    pub tracks: Vec<TrackProc>,
    /// per track: sidechain key buffer (empty when the track has no sidechain effect)
    pub keys: Vec<Vec<f32>>,
    pub scratch_l: Vec<f32>,
    pub scratch_r: Vec<f32>,
}

impl Rack {
    pub fn build(snap: &Snapshot, sr: f32, max_chunk: usize) -> Rack {
        let channels = snap
            .channels
            .iter()
            .map(|c| match &c.instrument {
                InstrumentSnap::Sampler { .. } => ChannelProc::Sampler(Sampler::new(sr, 32)),
                InstrumentSnap::Synth(_) => ChannelProc::Synth(Synth::new(sr, 16)),
                InstrumentSnap::SoundFont { font, .. } => match font.as_ref().and_then(|f| SfProc::new(f, sr)) {
                    Some(p) => ChannelProc::SoundFont(Box::new(p)),
                    None => ChannelProc::Empty,
                },
            })
            .collect();
        let tracks = snap
            .mixer
            .iter()
            .map(|t| TrackProc {
                fx: t.fx.iter().map(|f| Effect::new(f.params.kind(), sr)).collect(),
                l: vec![0.0; max_chunk],
                r: vec![0.0; max_chunk],
            })
            .collect();
        let keys = snap
            .mixer
            .iter()
            .map(|t| if t.fx.iter().any(|f| f.params.sidechain_source().is_some()) { vec![0.0; max_chunk] } else { Vec::new() })
            .collect();
        Rack { signature: snap.signature(), channels, tracks, keys, scratch_l: vec![0.0; max_chunk], scratch_r: vec![0.0; max_chunk] }
    }
}
