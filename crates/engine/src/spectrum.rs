//! Per-track spectrum analysis published to the UI without locks.
//!
//! The UI chooses which mixer track each of the [`SPECTRUM_SLOTS`] watches
//! ([`SpectrumShared::watch`]); the audio thread only analyses watched tracks (post-fader)
//! and writes the band levels as atomics, like the peak meters.

use std::sync::atomic::{AtomicU32, Ordering};
use tstudio_dsp::spectrum::{SpectrumAnalyzer, SPECTRUM_BANDS, SPECTRUM_FLOOR_DB};

/// How many tracks can be analysed at once (e.g. selected insert + master).
pub const SPECTRUM_SLOTS: usize = 4;
/// Samples between two analyses (2048-point FFT, no overlap).
const HOP: usize = 2048;
const OFF: u32 = u32::MAX;

struct Slot {
    track: AtomicU32,
    /// bumped after every published analysis
    gen: AtomicU32,
    bands: [AtomicU32; SPECTRUM_BANDS],
}

/// UI ↔ audio thread exchange (part of `SharedState`).
pub struct SpectrumShared {
    slots: [Slot; SPECTRUM_SLOTS],
}

/// A published spectrum.
#[derive(Clone, Debug, PartialEq)]
pub struct SpectrumFrame {
    pub track: usize,
    /// increases with every new analysis
    pub gen: u32,
    /// dBFS per band (see `tstudio_dsp::spectrum::band_edge_hz`)
    pub bands: [f32; SPECTRUM_BANDS],
}

impl Default for SpectrumShared {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| Slot {
                track: AtomicU32::new(OFF),
                gen: AtomicU32::new(0),
                bands: std::array::from_fn(|_| AtomicU32::new(SPECTRUM_FLOOR_DB.to_bits())),
            }),
        }
    }
}

impl SpectrumShared {
    /// Selects the mixer track analysed in `slot` (None = off, costs nothing).
    pub fn watch(&self, slot: usize, track: Option<usize>) {
        if let Some(s) = self.slots.get(slot) {
            s.track.store(track.map(|t| t as u32).unwrap_or(OFF), Ordering::Relaxed);
        }
    }

    /// Latest analysis of `slot`, if it watches a track.
    pub fn read(&self, slot: usize) -> Option<SpectrumFrame> {
        let s = self.slots.get(slot)?;
        let track = s.track.load(Ordering::Relaxed);
        if track == OFF {
            return None;
        }
        let gen = s.gen.load(Ordering::Acquire);
        let bands = std::array::from_fn(|i| f32::from_bits(s.bands[i].load(Ordering::Relaxed)));
        Some(SpectrumFrame { track: track as usize, gen, bands })
    }
}

/// Audio-thread side: one preallocated analyzer per slot.
pub(crate) struct SpectrumTaps {
    taps: Vec<(u32, SpectrumAnalyzer)>,
}

impl SpectrumTaps {
    pub(crate) fn new(sr: f32) -> Self {
        Self { taps: (0..SPECTRUM_SLOTS).map(|_| (OFF, SpectrumAnalyzer::new(sr, HOP))).collect() }
    }

    /// Picks up track changes from the UI (call once per chunk).
    pub(crate) fn sync(&mut self, shared: &SpectrumShared) {
        for ((cur, an), slot) in self.taps.iter_mut().zip(shared.slots.iter()) {
            let want = slot.track.load(Ordering::Relaxed);
            if want != *cur {
                *cur = want;
                an.reset();
                for b in slot.bands.iter() {
                    b.store(SPECTRUM_FLOOR_DB.to_bits(), Ordering::Relaxed);
                }
            }
        }
    }

    /// Feeds post-fader audio of `track`; publishes when an analysis completes.
    #[inline]
    pub(crate) fn feed(&mut self, shared: &SpectrumShared, track: usize, l: &[f32], r: &[f32]) {
        for ((cur, an), slot) in self.taps.iter_mut().zip(shared.slots.iter()) {
            if *cur != track as u32 {
                continue;
            }
            if an.push(l, r) {
                for (dst, v) in slot.bands.iter().zip(an.bands()) {
                    dst.store(v.to_bits(), Ordering::Relaxed);
                }
                slot.gen.fetch_add(1, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_watched_track_is_published() {
        let shared = SpectrumShared::default();
        let mut taps = SpectrumTaps::new(48000.0);
        assert!(shared.read(0).is_none());
        shared.watch(0, Some(2));
        taps.sync(&shared);
        let x: Vec<f32> = (0..4096).map(|i| (i as f32 * 0.13).sin() * 0.5).collect();
        taps.feed(&shared, 1, &x, &x);
        assert_eq!(shared.read(0).unwrap().gen, 0);
        taps.feed(&shared, 2, &x, &x);
        let f = shared.read(0).unwrap();
        assert_eq!(f.track, 2);
        assert!(f.gen >= 1);
        assert!(f.bands.iter().any(|v| *v > -20.0));
        shared.watch(0, None);
        taps.sync(&shared);
        assert!(shared.read(0).is_none());
    }

    #[test]
    fn engine_publishes_post_fader_track_and_master_spectra() {
        use crate::core::{Command, EngineCore, Garbage, PlayMode, SharedState, MAX_CHUNK};
        use crate::snapshot::{ClipSnap, Rack, Snapshot, TrackSnap};
        use std::sync::Arc;
        use tstudio_dsp::spectrum::band_edge_hz;
        use tstudio_dsp::SampleData;
        let sr = 48000u32;
        let mut s = Snapshot::empty();
        s.bpm = 120.0;
        s.mixer.push(TrackSnap::new(1.0));
        s.mixer.push(TrackSnap::new(1.0));
        s.mixer[2].volume = 0.0; // fader down: nothing post-fader
        s.song_end = tstudio_project::PPQ * 4;
        let v: Vec<f32> = (0..sr as usize * 2).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / sr as f32).sin() * 0.5).collect();
        for track in [1, 2] {
            s.clips.push(ClipSnap::Audio { sample: Arc::new(SampleData::new(v.clone(), v.clone(), sr)), start: 0, len: tstudio_project::PPQ * 4, offset: 0, gain: 1.0, mixer_track: track, fade_in: 0, fade_out: 0, fit_bpm: 120.0, follow: false });
        }
        let snap = Arc::new(s);
        let shared = Arc::new(SharedState::new(sr));
        let mut core = EngineCore::new(sr as f32, Arc::clone(&snap), Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK)), Arc::clone(&shared));
        let mut gc = |_g: Garbage| {};
        core.handle(Command::SetMode(PlayMode::Song), &mut gc);
        core.handle(Command::Play, &mut gc);
        shared.spectrum.watch(0, Some(1));
        shared.spectrum.watch(1, Some(0));
        shared.spectrum.watch(2, Some(2));
        let (mut l, mut r) = (vec![0.0; 8192], vec![0.0; 8192]);
        core.process(&mut l, &mut r);
        let k = (0..SPECTRUM_BANDS).find(|&i| band_edge_hz(i) <= 1000.0 && 1000.0 < band_edge_hz(i + 1)).unwrap();
        let track = shared.spectrum.read(0).unwrap();
        let master = shared.spectrum.read(1).unwrap();
        let muted = shared.spectrum.read(2).unwrap();
        assert!(track.gen >= 2 && master.gen >= 2);
        assert!(track.bands[k] > -10.0, "track {}", track.bands[k]);
        assert!(master.bands[k] > -10.0, "master {}", master.bands[k]);
        assert!(muted.bands[k] <= SPECTRUM_FLOOR_DB + 1.0, "fader down {}", muted.bands[k]);
    }
}
