//! Project → engine snapshot (shared by the GUI and the CLI).

use crate::core::MAX_TRACKS;
use crate::snapshot::{AutoClipSnap, AutoLane, ChannelSnap, ClipSnap, FxSnap, InstrumentSnap, NoteSnap, PatternSnap, Snapshot, TrackSnap};
use crate::soundfont::load_soundfont;
use tstudio_project::automation::field_range;
use std::path::Path;
use std::sync::Arc;
use tstudio_dsp::SampleData;
use tstudio_project::{ClipKind, Instrument, Project, StretchSettings, PPQ};

/// Supplies tempo-fitted sample data at the engine rate (None = missing / not ready yet).
pub type SampleSource<'a> = dyn FnMut(&Path, &StretchSettings, f64) -> Option<Arc<SampleData>> + 'a;

pub fn compile_project(p: &Project, samples: &mut SampleSource<'_>, for_export: bool) -> Snapshot {
    let any_channel_solo = p.channels.iter().any(|c| c.solo);
    let channels = p
        .channels
        .iter()
        .map(|c| ChannelSnap {
            instrument: match &c.instrument {
                Instrument::Synth(s) => InstrumentSnap::Synth(s.clone()),
                Instrument::Sampler(s) => InstrumentSnap::Sampler {
                    params: s.params.clone(),
                    sample: s.sample.as_ref().and_then(|path| samples(path, &s.stretch, p.bpm)),
                },
                Instrument::SoundFont(s) => InstrumentSnap::SoundFont {
                    font: load_soundfont(&s.path).ok(),
                    bank: s.bank,
                    program: s.program,
                    gain: s.gain,
                },
            },
            mixer_track: c.mixer_track,
            volume: c.volume,
            pan: c.pan,
            muted: c.muted || (any_channel_solo && !c.solo),
        })
        .collect();

    // swing: every second 16th (24 ticks) is pushed back by up to an eighth of a beat
    let swing_ticks = (p.swing.clamp(0.0, 1.0) * 12.0).round() as i64;
    let sixteenth = PPQ / 4;
    let patterns = p
        .patterns
        .iter()
        .map(|pat| {
            let mut notes: Vec<NoteSnap> = pat
                .notes
                .iter()
                .filter_map(|n| {
                    let swung = swing_ticks > 0 && n.start % (2 * sixteenth) == sixteenth;
                    let start = if swung { n.start + swing_ticks } else { n.start };
                    let len = if swung { (n.len - swing_ticks).max(n.len / 2).max(1) } else { n.len };
                    Some(NoteSnap { channel: p.channel_index(n.channel)?, key: n.key, start, len, velocity: n.velocity })
                })
                .collect();
            notes.sort_by_key(|n| n.start);
            PatternSnap { length: p.pattern_length(pat.id), notes }
        })
        .collect();

    let mut clips = Vec::new();
    let mut autos: Vec<AutoLane> = Vec::new();
    for c in &p.playlist.clips {
        if c.muted || !p.playlist_track_audible(c.track) {
            continue;
        }
        let track_gain = p.playlist.tracks.get(c.track).map(|t| t.volume.max(0.0)).unwrap_or(1.0);
        match &c.kind {
            ClipKind::Pattern { pattern } => {
                if let Some(idx) = p.patterns.iter().position(|x| x.id == *pattern) {
                    clips.push(ClipSnap::Pattern { pattern: idx, start: c.start, len: c.len, gain: track_gain });
                }
            }
            ClipKind::Audio(a) => {
                // fitted to the tempo at the clip start (= project tempo without a tempo track)
                let fit_bpm = p.bpm_at(c.start);
                let follow = a.stretch.is_active() && a.stretch.source_bpm > 0.0 && a.stretch.target_bpm.is_none();
                if let Some(data) = samples(&a.path, &a.stretch, fit_bpm) {
                    clips.push(ClipSnap::Audio {
                        sample: data,
                        start: c.start,
                        len: c.len,
                        offset: a.offset,
                        gain: a.gain * track_gain,
                        mixer_track: a.mixer_track,
                        fade_in: a.fade_in,
                        fade_out: a.fade_out,
                        fit_bpm,
                        follow,
                    });
                }
            }
            ClipKind::Automation(a) => {
                let Ok(target) = p.parse_auto_target(&a.target) else { continue };
                if a.points.is_empty() {
                    continue;
                }
                let clip = AutoClipSnap { start: c.start, len: c.len, points: a.points.iter().map(|q| (q.t, q.v)).collect() };
                match autos.iter_mut().find(|l| l.target == target) {
                    Some(l) => l.clips.push(clip),
                    None => autos.push(AutoLane {
                        target,
                        log: field_range(tstudio_project::Project::auto_field_name(&a.target)).2,
                        clips: vec![clip],
                    }),
                }
            }
        }
    }
    for l in autos.iter_mut() {
        l.clips.sort_by_key(|c| c.start);
    }

    let n_tracks = p.mixer.len().min(MAX_TRACKS);
    let mixer = p
        .mixer
        .iter()
        .take(MAX_TRACKS)
        .enumerate()
        .map(|(i, t)| TrackSnap {
            volume: t.volume,
            pan: t.pan,
            muted: t.muted,
            solo: t.solo,
            fx: t.fx.iter().map(|f| FxSnap { enabled: f.enabled, params: f.params.clone() }).collect(),
            output: if t.output > i && t.output < n_tracks { t.output } else { 0 },
            sends: t.sends.iter().filter(|s| s.target > i && s.target < n_tracks).map(|s| (s.target, s.amount)).collect(),
            is_bus: i > 0 && p.mixer.iter().take(i).any(|o| o.output == i || o.sends.iter().any(|s| s.target == i)),
        })
        .collect();

    Snapshot {
        bpm: p.bpm,
        tempo: p.tempo_map(),
        beats_per_bar: p.beats_per_bar,
        bars: p.bar_map(),
        channels,
        patterns,
        clips,
        song_end: p.song_length(),
        loop_region: if for_export { None } else { p.playlist.loop_region },
        mixer,
        autos,
    }
}
