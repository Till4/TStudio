//! Creating, deleting and reordering playlist tracks and mixer inserts, plus mute/solo logic.
//! Every operation keeps the references into the moved tracks intact (clips, channel routing,
//! sends, bus outputs, sidechain keys and automation targets).

use crate::{ClipKind, MixerTrack, PlaylistTrack, Project};
use tstudio_dsp::fx::EffectParams;

impl Project {
    // ------------------------------------------------------------------ playlist tracks

    /// True when clips on playlist track `t` should play (not muted and not hidden by another track's solo).
    pub fn playlist_track_audible(&self, t: usize) -> bool {
        let tracks = &self.playlist.tracks;
        let Some(tr) = tracks.get(t) else { return !tracks.iter().any(|x| x.solo) };
        if tr.muted {
            return false;
        }
        !tracks.iter().any(|x| x.solo) || tr.solo
    }

    /// Inserts an empty playlist track at `at` (clamped to the end); clips below move down. Returns its index.
    pub fn insert_playlist_track(&mut self, at: usize, name: Option<String>) -> usize {
        let at = at.min(self.playlist.tracks.len());
        let name = name.unwrap_or_else(|| self.next_track_name());
        self.playlist.tracks.insert(at, PlaylistTrack { name, ..Default::default() });
        for c in self.playlist.clips.iter_mut() {
            if c.track >= at {
                c.track += 1;
            }
        }
        at
    }

    /// Mixer insert for an audio sample placed in the playlist: reuses the insert other clips of the
    /// same file already play on, otherwise appends a new one named after the file.
    pub fn sample_insert(&mut self, path: &std::path::Path) -> usize {
        let existing = self.playlist.clips.iter().find_map(|c| match &c.kind {
            ClipKind::Audio(a) if a.path == path && a.mixer_track > 0 && a.mixer_track < self.mixer.len() => Some(a.mixer_track),
            _ => None,
        });
        existing.unwrap_or_else(|| {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sample".into());
            self.add_mixer_track(Some(stem))
        })
    }

    /// Appends an empty playlist track. Returns its index.
    pub fn add_playlist_track(&mut self, name: Option<String>) -> usize {
        let at = self.playlist.tracks.len();
        self.insert_playlist_track(at, name)
    }

    /// Removes playlist track `t` together with its clips; clips below move up.
    /// Returns the number of clips deleted. At least one track always remains.
    pub fn delete_playlist_track(&mut self, t: usize) -> Result<usize, String> {
        if t >= self.playlist.tracks.len() {
            return Err(format!("Playlist-Spur {t} gibt es nicht"));
        }
        let before = self.playlist.clips.len();
        self.playlist.clips.retain(|c| c.track != t);
        let removed = before - self.playlist.clips.len();
        for c in self.playlist.clips.iter_mut() {
            if c.track > t {
                c.track -= 1;
            }
        }
        self.playlist.tracks.remove(t);
        if self.playlist.tracks.is_empty() {
            self.playlist.tracks.push(PlaylistTrack { name: "Track 1".into(), ..Default::default() });
        }
        Ok(removed)
    }

    /// Moves playlist track `from` to position `to` (clips go with it).
    pub fn move_playlist_track(&mut self, from: usize, to: usize) -> Result<(), String> {
        let n = self.playlist.tracks.len();
        if from >= n || to >= n {
            return Err(format!("Playlist-Spur {} gibt es nicht (0..{})", from.max(to), n.saturating_sub(1)));
        }
        let tr = self.playlist.tracks.remove(from);
        self.playlist.tracks.insert(to, tr);
        for c in self.playlist.clips.iter_mut() {
            c.track = moved_index(c.track, from, to);
        }
        Ok(())
    }

    /// Copies track `t` (name, state and clips) directly below it. Returns the new index.
    pub fn duplicate_playlist_track(&mut self, t: usize) -> Result<usize, String> {
        let src = self.playlist.tracks.get(t).cloned().ok_or_else(|| format!("Playlist-Spur {t} gibt es nicht"))?;
        let at = self.insert_playlist_track(t + 1, Some(format!("{} (Kopie)", src.name)));
        self.playlist.tracks[at].muted = src.muted;
        let copies: Vec<_> = self.playlist.clips.iter().filter(|c| c.track == t).cloned().collect();
        for mut c in copies {
            c.id = self.alloc_id();
            c.track = at;
            self.playlist.clips.push(c);
        }
        Ok(at)
    }

    /// Solo for a playlist track. `exclusive` clears every other solo first (plain click);
    /// otherwise solos add up (Ctrl+click).
    pub fn toggle_playlist_solo(&mut self, t: usize, exclusive: bool) {
        let Some(on) = self.playlist.tracks.get(t).map(|x| !x.solo) else { return };
        if exclusive {
            for x in self.playlist.tracks.iter_mut() {
                x.solo = false;
            }
        }
        self.playlist.tracks[t].solo = on;
    }

    fn next_track_name(&self) -> String {
        (1..)
            .map(|i| format!("Track {i}"))
            .find(|n| !self.playlist.tracks.iter().any(|t| &t.name == n))
            .unwrap_or_default()
    }

    // ------------------------------------------------------------------ mixer inserts

    /// Inserts a new mixer insert at `at` (1..=len, clamped; the master stays at 0). Returns its index.
    pub fn insert_mixer_track(&mut self, at: usize, name: Option<String>) -> usize {
        let at = at.clamp(1, self.mixer.len());
        let name = name.unwrap_or_else(|| format!("Insert {at}"));
        self.remap_mixer(|i| Some(if i >= at { i + 1 } else { i }));
        self.mixer.insert(at, MixerTrack { name, ..Default::default() });
        at
    }

    /// Appends a mixer insert. Returns its index.
    pub fn add_mixer_track(&mut self, name: Option<String>) -> usize {
        let at = self.mixer.len();
        self.insert_mixer_track(at, name)
    }

    /// Deletes mixer insert `t` (not the master). Channels and audio clips routed there go to the
    /// master, sends to it disappear, automation clips aimed at it are deleted, higher inserts move down.
    pub fn delete_mixer_track(&mut self, t: usize) -> Result<(), String> {
        if t == 0 {
            return Err("Der Master kann nicht gelöscht werden".into());
        }
        if t >= self.mixer.len() {
            return Err(format!("Mixer-Spur {t} gibt es nicht"));
        }
        self.remap_mixer(|i| match i {
            i if i == t => None,
            i if i > t => Some(i - 1),
            i => Some(i),
        });
        self.mixer.remove(t);
        Ok(())
    }

    /// Moves mixer insert `from` to position `to` (both ≥ 1). Bus outputs that would now point
    /// backwards are reset to the master by `sanitize`.
    pub fn move_mixer_track(&mut self, from: usize, to: usize) -> Result<(), String> {
        let n = self.mixer.len();
        if from == 0 || to == 0 {
            return Err("Der Master bleibt an Position 0".into());
        }
        if from >= n || to >= n {
            return Err(format!("Mixer-Spur {} gibt es nicht (1..{})", from.max(to), n - 1));
        }
        self.remap_mixer(|i| Some(if i == 0 { 0 } else { moved_index(i, from, to) }));
        let tr = self.mixer.remove(from);
        self.mixer.insert(to, tr);
        self.sanitize();
        Ok(())
    }

    /// Rewrites every reference to a mixer track index. `None` = the track disappears.
    fn remap_mixer(&mut self, f: impl Fn(usize) -> Option<usize>) {
        let to_master = |i: usize| f(i).unwrap_or(0);
        for c in self.channels.iter_mut() {
            c.mixer_track = to_master(c.mixer_track);
        }
        let mut dead_autos = Vec::new();
        for clip in self.playlist.clips.iter_mut() {
            match &mut clip.kind {
                ClipKind::Audio(a) => a.mixer_track = to_master(a.mixer_track),
                ClipKind::Automation(a) => {
                    if let Some(rest) = a.target.strip_prefix("mixer.") {
                        let (n, tail) = rest.split_once('.').unwrap_or((rest, ""));
                        if let Ok(n) = n.parse::<usize>() {
                            match f(n) {
                                Some(m) => a.target = format!("mixer.{m}.{tail}"),
                                None => dead_autos.push(clip.id),
                            }
                        }
                    }
                }
                ClipKind::Pattern { .. } => {}
            }
        }
        self.playlist.clips.retain(|c| !dead_autos.contains(&c.id));
        for tr in self.mixer.iter_mut() {
            tr.output = to_master(tr.output);
            tr.sends.retain_mut(|s| match f(s.target) {
                Some(m) => {
                    s.target = m;
                    true
                }
                None => false,
            });
            for slot in tr.fx.iter_mut() {
                if let EffectParams::Sidechain(p) = &mut slot.params {
                    p.source = to_master(p.source);
                }
            }
        }
    }
}

/// Where index `i` ends up when the element at `from` is moved to `to`.
fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioClip, AutomationClip, MixerSend, PPQ};
    use tstudio_dsp::fx::EffectKind;

    fn proj() -> Project {
        let mut p = Project::default();
        p.add_clip(0, 0, PPQ * 4, ClipKind::Audio(AudioClip::default()));
        p.add_clip(1, 0, PPQ * 4, ClipKind::Audio(AudioClip::default()));
        p.add_clip(2, 0, PPQ * 4, ClipKind::Audio(AudioClip::default()));
        p
    }

    #[test]
    fn playlist_insert_delete_move_keep_clips() {
        let mut p = proj();
        let n = p.playlist.tracks.len();
        assert_eq!(p.insert_playlist_track(1, Some("Neu".into())), 1);
        assert_eq!(p.playlist.tracks.len(), n + 1);
        let tracks: Vec<usize> = p.playlist.clips.iter().map(|c| c.track).collect();
        assert_eq!(tracks, vec![0, 2, 3]);
        assert_eq!(p.delete_playlist_track(2).unwrap(), 1);
        let tracks: Vec<usize> = p.playlist.clips.iter().map(|c| c.track).collect();
        assert_eq!(tracks, vec![0, 2]);
        p.move_playlist_track(0, 2).unwrap();
        let tracks: Vec<usize> = p.playlist.clips.iter().map(|c| c.track).collect();
        assert_eq!(tracks, vec![2, 1]);
        assert_eq!(p.playlist.tracks[2].name, "Track 1");
        let d = p.duplicate_playlist_track(2).unwrap();
        assert_eq!(p.playlist.clips.iter().filter(|c| c.track == d).count(), 1);
    }

    #[test]
    fn solo_and_mute() {
        let mut p = proj();
        assert!(p.playlist_track_audible(0) && p.playlist_track_audible(1));
        p.toggle_playlist_solo(1, true);
        assert!(!p.playlist_track_audible(0) && p.playlist_track_audible(1));
        p.toggle_playlist_solo(2, false);
        assert!(p.playlist_track_audible(1) && p.playlist_track_audible(2));
        p.toggle_playlist_solo(0, true);
        assert!(p.playlist_track_audible(0) && !p.playlist_track_audible(1) && !p.playlist_track_audible(2));
        p.playlist.tracks[0].muted = true;
        assert!(!p.playlist_track_audible(0));
    }

    #[test]
    fn mixer_delete_remaps_references() {
        let mut p = Project::default();
        while p.mixer.len() < 17 {
            p.add_mixer_track(None);
        }
        p.channels.push(crate::Channel::default());
        p.channels[0].mixer_track = 3;
        p.mixer[2].output = 5;
        p.mixer[1].sends.push(MixerSend { target: 3, amount: 0.5 });
        p.mixer[1].sends.push(MixerSend { target: 6, amount: 0.5 });
        let mut sc = tstudio_project_fx(EffectKind::Sidechain);
        if let EffectParams::Sidechain(s) = &mut sc.params {
            s.source = 4;
        }
        p.mixer[7].fx.push(sc);
        p.add_clip(0, 0, PPQ, ClipKind::Automation(AutomationClip { target: "mixer.3.volume".into(), points: vec![] }));
        p.add_clip(0, 0, PPQ, ClipKind::Automation(AutomationClip { target: "mixer.6.pan".into(), points: vec![] }));
        let n = p.mixer.len();
        p.delete_mixer_track(3).unwrap();
        assert_eq!(p.mixer.len(), n - 1);
        assert_eq!(p.channels[0].mixer_track, 0);
        assert_eq!(p.mixer[2].output, 4);
        assert_eq!(p.mixer[1].sends.len(), 1);
        assert_eq!(p.mixer[1].sends[0].target, 5);
        assert_eq!(p.mixer[6].fx[0].params.sidechain_source(), Some(3));
        let autos: Vec<String> = p
            .playlist
            .clips
            .iter()
            .filter_map(|c| if let ClipKind::Automation(a) = &c.kind { Some(a.target.clone()) } else { None })
            .collect();
        assert_eq!(autos, vec!["mixer.5.pan".to_string()]);
        assert!(p.delete_mixer_track(0).is_err());
    }

    #[test]
    fn mixer_insert_and_move() {
        let mut p = Project::default();
        p.channels.push(crate::Channel::default());
        p.channels[0].mixer_track = 2;
        assert_eq!(p.insert_mixer_track(1, Some("Bus".into())), 1);
        assert_eq!(p.channels[0].mixer_track, 3);
        assert_eq!(p.mixer[1].name, "Bus");
        p.move_mixer_track(3, 1).unwrap();
        assert_eq!(p.channels[0].mixer_track, 1);
        assert!(p.move_mixer_track(0, 2).is_err());
    }

    fn tstudio_project_fx(k: EffectKind) -> crate::FxSlot {
        crate::FxSlot::new(k)
    }
}

impl Project {
    /// Pattern clips follow their pattern's length while they have not been resized by hand:
    /// a clip exactly as long as the pattern was (`prev`) grows/shrinks with it.
    /// `prev` keeps the last known length per pattern id between calls. Returns true if a clip changed.
    pub fn follow_pattern_lengths(&mut self, prev: &mut std::collections::HashMap<crate::Id, i64>) -> bool {
        let mut changed = false;
        let lens: Vec<(crate::Id, i64)> = self.patterns.iter().map(|p| (p.id, self.pattern_length(p.id))).collect();
        for (id, len) in &lens {
            if let Some(old) = prev.get(id).copied() {
                if old != *len {
                    for c in self.playlist.clips.iter_mut() {
                        if matches!(c.kind, ClipKind::Pattern { pattern } if pattern == *id) && c.len == old {
                            c.len = *len;
                            changed = true;
                        }
                    }
                }
            }
        }
        prev.clear();
        prev.extend(lens);
        changed
    }
}

#[cfg(test)]
mod follow_tests {
    use crate::{ClipKind, Note, Project};

    #[test]
    fn clips_follow_pattern_length_unless_resized() {
        let mut p = Project::new_default();
        p.playlist.clips.clear();
        let pat = p.patterns[0].id;
        let ch = p.channels[0].id;
        let bar = p.ticks_per_bar();
        let a = p.add_clip(0, 0, bar, ClipKind::Pattern { pattern: pat });
        let b = p.add_clip(1, 0, bar * 8, ClipKind::Pattern { pattern: pat }); // stretched by hand
        let mut prev = Default::default();
        assert!(!p.follow_pattern_lengths(&mut prev));
        // a note in bar 3 makes the pattern 3 bars long
        p.pattern_mut(pat).unwrap().notes.push(Note { channel: ch, key: 60, start: bar * 2, len: 24, velocity: 1.0 });
        assert!(p.follow_pattern_lengths(&mut prev));
        let len = |id| p.playlist.clips.iter().find(|c| c.id == id).unwrap().len;
        assert_eq!(len(a), bar * 3);
        assert_eq!(len(b), bar * 8);
    }

    #[test]
    fn samples_get_own_insert_and_share_it() {
        use crate::{AudioClip, PPQ};
        let mut p = Project::default();
        let n = p.mixer.len();
        let a = std::path::Path::new("C:/s/Amen Break.wav");
        let t = p.sample_insert(a);
        assert_eq!((t, p.mixer[t].name.as_str()), (n, "Amen Break"));
        p.add_clip(0, 0, PPQ, ClipKind::Audio(AudioClip { path: a.into(), mixer_track: t, ..Default::default() }));
        assert_eq!(p.sample_insert(a), t);
        assert_eq!(p.sample_insert(std::path::Path::new("C:/s/kick.wav")), n + 1);
    }

    #[test]
    fn new_project_is_small_and_grows() {
        use crate::{Instrument, DEFAULT_MIXER_TRACKS, DEFAULT_PLAYLIST_TRACKS};
        let mut p = Project::default();
        assert_eq!(p.playlist.tracks.len(), 4);
        assert_eq!(p.mixer.len(), DEFAULT_MIXER_TRACKS + 1);
        assert_eq!(DEFAULT_PLAYLIST_TRACKS, 4);
        // 4 free inserts get used, the 5th channel appends a new one named after it
        for i in 0..5 {
            let ch = p.add_channel(&format!("Ch {i}"), Instrument::Synth(Default::default()));
            assert_eq!(p.channels[ch].mixer_track, i + 1);
        }
        assert_eq!(p.mixer.len(), 6);
        assert_eq!(p.mixer[5].name, "Ch 4");
        // a clip far below the visible tracks makes the playlist grow on load
        p.playlist.clips.push(crate::Clip { id: 999, track: 9, start: 0, len: 96, kind: ClipKind::Audio(Default::default()), muted: false });
        p.sanitize();
        assert_eq!(p.playlist.tracks.len(), 10);
    }
}
