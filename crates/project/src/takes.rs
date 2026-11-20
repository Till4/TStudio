//! Take groups and comping for audio recording.
//!
//! A take group is a set of playlist tracks ("lanes") sharing `PlaylistTrack::take_group`.
//! Every recorded take is an ordinary audio clip on its own lane; newer takes stack below the
//! older ones. Comping = choosing which lane plays in a range: clips are split at the range
//! edges and the clips of the other lanes are muted there (plain `Clip::muted`), so the
//! engine and old projects need nothing new.

use crate::{ClipKind, Id, Project};

impl Project {
    /// Lanes (playlist track indices) of take group `group`, top to bottom.
    pub fn take_lanes(&self, group: Id) -> Vec<usize> {
        self.playlist.tracks.iter().enumerate().filter(|(_, t)| t.take_group == Some(group)).map(|(i, _)| i).collect()
    }

    /// The take group whose lanes hold audio overlapping `a..b` (re-recording the same section).
    pub fn take_group_at(&self, a: i64, b: i64) -> Option<Id> {
        self.playlist.clips.iter().filter(|c| matches!(c.kind, ClipKind::Audio(_)) && c.start < b && c.end() > a).find_map(|c| self.playlist.tracks.get(c.track).and_then(|t| t.take_group))
    }

    /// Playlist track for a new take. With `group` the lane is inserted directly below the
    /// group's last lane, otherwise a new group starts on an empty track (below every used one).
    /// Returns (group, lane).
    pub fn new_take_lane(&mut self, group: Option<Id>, base_name: &str) -> (Id, usize) {
        let lanes = group.map(|g| self.take_lanes(g)).unwrap_or_default();
        let (group, lane) = match (group, lanes.last()) {
            (Some(g), Some(&last)) => (g, self.insert_playlist_track(last + 1, None)),
            _ => {
                let g = self.alloc_id();
                let last_used = self.playlist.clips.iter().map(|c| c.track + 1).max().unwrap_or(0);
                let free = (last_used..self.playlist.tracks.len()).find(|&t| self.playlist.tracks[t].take_group.is_none());
                let t = match free {
                    Some(t) => t,
                    None => self.add_playlist_track(None),
                };
                (g, t)
            }
        };
        let n = self.take_lanes(group).len() + 1;
        let tr = &mut self.playlist.tracks[lane];
        tr.take_group = Some(group);
        tr.name = format!("{base_name} Take {n}");
        (group, lane)
    }

    /// Splits audio clip `id` at `tick` (strictly inside it); the right part keeps playing the
    /// same audio (its offset moves). Pattern and automation clips are not split.
    /// Returns the id of the right part.
    pub fn split_clip(&mut self, id: Id, tick: i64) -> Option<Id> {
        let i = self.playlist.clips.iter().position(|c| c.id == id)?;
        let c = &self.playlist.clips[i];
        if tick <= c.start || tick >= c.end() || !matches!(c.kind, ClipKind::Audio(_)) {
            return None;
        }
        let mut right = c.clone();
        let left_len = tick - c.start;
        right.start = tick;
        right.len = c.len - left_len;
        right.id = self.alloc_id();
        if let ClipKind::Audio(a) = &mut right.kind {
            a.offset += left_len;
            a.fade_in = 0;
        }
        let left = &mut self.playlist.clips[i];
        left.len = left_len;
        if let ClipKind::Audio(a) = &mut left.kind {
            a.fade_out = 0;
        }
        let rid = right.id;
        self.playlist.clips.push(right);
        Some(rid)
    }

    /// Comping: `lane` plays between `a` and `b`, the other lanes of its take group are muted
    /// there. Audio clips are split at `a` and `b`. Returns the number of clips in the range.
    pub fn comp_take(&mut self, lane: usize, a: i64, b: i64) -> Result<usize, String> {
        let (a, b) = (a.min(b), a.max(b));
        if b <= a {
            return Err("Leerer Bereich".into());
        }
        let group = self.playlist.tracks.get(lane).and_then(|t| t.take_group).ok_or_else(|| format!("Spur {lane} ist keine Take-Spur"))?;
        let lanes = self.take_lanes(group);
        let in_group = |p: &Project, id: Id| p.playlist.clips.iter().find(|c| c.id == id).map(|c| lanes.contains(&c.track) && matches!(c.kind, ClipKind::Audio(_))).unwrap_or(false);
        for edge in [a, b] {
            let ids: Vec<Id> = self.playlist.clips.iter().filter(|c| c.start < edge && c.end() > edge).map(|c| c.id).collect();
            for id in ids {
                if in_group(self, id) {
                    self.split_clip(id, edge);
                }
            }
        }
        let mut n = 0;
        for c in self.playlist.clips.iter_mut() {
            if lanes.contains(&c.track) && matches!(c.kind, ClipKind::Audio(_)) && c.start >= a && c.end() <= b {
                c.muted = c.track != lane;
                n += 1;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use crate::{AudioClip, ClipKind, Project, PPQ};

    fn audio(offset: i64) -> ClipKind {
        ClipKind::Audio(AudioClip { path: "take.wav".into(), offset, fade_in: 10, fade_out: 10, ..Default::default() })
    }

    #[test]
    fn split_keeps_audio_position() {
        let mut p = Project::default();
        let id = p.add_clip(0, 100, 400, audio(5));
        let r = p.split_clip(id, 250).unwrap();
        let l = p.playlist.clips.iter().find(|c| c.id == id).unwrap();
        let rc = p.playlist.clips.iter().find(|c| c.id == r).unwrap();
        assert_eq!((l.start, l.len), (100, 150));
        assert_eq!((rc.start, rc.len), (250, 250));
        match (&l.kind, &rc.kind) {
            (ClipKind::Audio(la), ClipKind::Audio(ra)) => {
                assert_eq!((la.offset, la.fade_in, la.fade_out), (5, 10, 0));
                assert_eq!((ra.offset, ra.fade_in, ra.fade_out), (155, 0, 10));
            }
            _ => unreachable!(),
        }
        assert!(p.split_clip(id, 100).is_none(), "edges are not inside");
        let pat = p.add_clip(1, 0, 96, ClipKind::Pattern { pattern: 1 });
        assert!(p.split_clip(pat, 48).is_none());
    }

    #[test]
    fn takes_stack_as_lanes_of_one_group() {
        let mut p = Project::default();
        p.add_clip(0, 0, PPQ * 16, ClipKind::Pattern { pattern: 1 });
        let (g, l1) = p.new_take_lane(None, "Vocals");
        assert_eq!(l1, 1, "first empty track below the used ones");
        p.add_clip(l1, 0, PPQ * 8, audio(0));
        assert_eq!(p.take_group_at(PPQ, PPQ * 2), Some(g));
        assert_eq!(p.take_group_at(PPQ * 9, PPQ * 10), None);
        // an unrelated clip below the group moves down when the next lane is inserted
        let other = p.add_clip(2, 0, PPQ, audio(0));
        let (g2, l2) = p.new_take_lane(Some(g), "Vocals");
        assert_eq!((g2, l2), (g, 2));
        assert_eq!(p.playlist.clips.iter().find(|c| c.id == other).unwrap().track, 3);
        assert_eq!(p.take_lanes(g), vec![1, 2]);
        assert_eq!(p.playlist.tracks[2].name, "Vocals Take 2");
        // a group id survives save/load and next_id stays above it
        let q = Project::from_ron(&p.to_ron().unwrap()).unwrap();
        assert!(q.next_id > g);
        assert_eq!(q.take_lanes(g), vec![1, 2]);
    }

    #[test]
    fn comping_splits_and_mutes_other_lanes() {
        let mut p = Project::default();
        let (g, l1) = p.new_take_lane(None, "Vocals");
        p.add_clip(l1, 0, 800, audio(0));
        let (_, l2) = p.new_take_lane(Some(g), "Vocals");
        p.add_clip(l2, 0, 800, audio(0));
        // newest take active everywhere
        assert_eq!(p.comp_take(l2, 0, 800).unwrap(), 2);
        // bars 200..400 from take 1
        p.comp_take(l1, 200, 400).unwrap();
        let mut v: Vec<(usize, i64, i64, bool)> = p.playlist.clips.iter().map(|c| (c.track, c.start, c.len, c.muted)).collect();
        v.sort();
        assert_eq!(
            v,
            vec![(l1, 0, 200, true), (l1, 200, 200, false), (l1, 400, 400, true), (l2, 0, 200, false), (l2, 200, 200, true), (l2, 400, 400, false)]
        );
        // exactly one lane audible at every point
        for t in [0, 199, 200, 399, 400, 799] {
            let audible = p.playlist.clips.iter().filter(|c| c.start <= t && c.end() > t && !c.muted).count();
            assert_eq!(audible, 1, "tick {t}");
        }
        assert!(p.comp_take(5, 0, 10).is_err());
    }

    #[test]
    fn old_projects_have_no_take_groups() {
        // a track written before take groups existed
        let p = Project::from_ron("(playlist: (tracks: [(name: \"Vocals\", muted: false, solo: false, volume: 1.0)]))").unwrap();
        assert_eq!(p.playlist.tracks[0].take_group, None);
    }
}
