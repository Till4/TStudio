//! Snapshot-based undo/redo. The UI calls [`History::commit`] (or [`History::commit_with`])
//! once per frame; changes made while the mouse is held are merged into one undo step, and
//! consecutive commits of the same continuous control (same coalesce key, within
//! [`COALESCE_WINDOW`]) are merged too – e.g. mouse-wheel or keyboard nudges of a fader.

use crate::Project;
use std::time::{Duration, Instant};

/// Commits of the same control closer together than this become one undo step.
pub const COALESCE_WINDOW: Duration = Duration::from_millis(1500);

pub struct History {
    /// earlier states + label of the edit that left them
    past: Vec<(Project, String)>,
    /// undone states + label of the edit that leads to them
    future: Vec<(Project, String)>,
    committed: Project,
    /// coalesce key and time of the last commit
    last: Option<(String, Instant)>,
    limit: usize,
}

impl History {
    pub fn new(project: &Project) -> Self {
        Self { past: Vec::new(), future: Vec::new(), committed: project.clone(), last: None, limit: 200 }
    }

    /// The state of the last undo step (what the next commit is compared with).
    pub fn committed(&self) -> &Project {
        &self.committed
    }

    /// Records `current` as a new undo step if it differs from the last committed state.
    /// Call when no interaction (drag) is in progress.
    pub fn commit(&mut self, current: &Project) -> bool {
        self.commit_with(current, "", None, Instant::now())
    }

    /// Like [`commit`](Self::commit) with a label for the step; with a `key`, a commit that
    /// follows a commit with the same key within [`COALESCE_WINDOW`] extends that step.
    pub fn commit_with(&mut self, current: &Project, label: &str, key: Option<&str>, now: Instant) -> bool {
        if *current == self.committed {
            return false;
        }
        let merge = match (key, &self.last) {
            (Some(k), Some((lk, t))) => k == lk && now.saturating_duration_since(*t) < COALESCE_WINDOW && !self.past.is_empty(),
            _ => false,
        };
        let prev = std::mem::replace(&mut self.committed, current.clone());
        if !merge {
            self.past.push((prev, label.to_string()));
            if self.past.len() > self.limit {
                self.past.remove(0);
            }
        }
        self.last = key.map(|k| (k.to_string(), now));
        self.future.clear();
        true
    }

    pub fn undo(&mut self, current: &mut Project) -> bool {
        // fold any uncommitted change in first so it can be redone
        self.commit(current);
        self.last = None;
        if let Some((prev, label)) = self.past.pop() {
            let now = std::mem::replace(current, prev);
            self.future.push((now, label));
            self.committed = current.clone();
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, current: &mut Project) -> bool {
        self.last = None;
        if let Some((next, label)) = self.future.pop() {
            let now = std::mem::replace(current, next);
            self.past.push((now, label));
            self.committed = current.clone();
            true
        } else {
            false
        }
    }

    /// Label of the step [`undo`](Self::undo) would revert ("" = unlabelled).
    pub fn undo_label(&self) -> Option<&str> {
        self.past.last().map(|(_, l)| l.as_str())
    }

    /// Label of the step [`redo`](Self::redo) would restore.
    pub fn redo_label(&self) -> Option<&str> {
        self.future.last().map(|(_, l)| l.as_str())
    }

    /// Forget all history (after loading a project).
    pub fn reset(&mut self, project: &Project) {
        self.past.clear();
        self.future.clear();
        self.last = None;
        self.committed = project.clone();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo() {
        let mut p = Project::new_default();
        let mut h = History::new(&p);
        p.bpm = 100.0;
        h.commit(&p);
        p.bpm = 90.0;
        h.commit(&p);
        assert!(h.undo(&mut p));
        assert_eq!(p.bpm, 100.0);
        assert!(h.undo(&mut p));
        assert_eq!(p.bpm, 140.0);
        assert!(!h.undo(&mut p));
        assert!(h.redo(&mut p));
        assert_eq!(p.bpm, 100.0);
    }

    #[test]
    fn same_control_within_window_is_one_step() {
        let mut p = Project::new_default();
        let mut h = History::new(&p);
        let t0 = Instant::now();
        for (k, v) in [0.7, 0.6, 0.5].into_iter().enumerate() {
            p.mixer[1].volume = v;
            h.commit_with(&p, "Insert 1: Lautstärke", Some("mixer:1:volume"), t0 + Duration::from_millis(300 * k as u64));
        }
        assert_eq!(h.undo_label(), Some("Insert 1: Lautstärke"));
        assert!(h.undo(&mut p));
        assert_eq!(p.mixer[1].volume, 0.8);
        assert!(!h.can_undo());
        assert_eq!(h.redo_label(), Some("Insert 1: Lautstärke"));
        assert!(h.redo(&mut p));
        assert_eq!(p.mixer[1].volume, 0.5);
    }

    #[test]
    fn different_control_or_pause_starts_a_new_step() {
        let mut p = Project::new_default();
        let mut h = History::new(&p);
        let t0 = Instant::now();
        p.mixer[1].volume = 0.5;
        h.commit_with(&p, "vol", Some("mixer:1:volume"), t0);
        p.mixer[1].pan = 0.5;
        h.commit_with(&p, "pan", Some("mixer:1:pan"), t0);
        p.mixer[1].pan = 0.7;
        h.commit_with(&p, "pan", Some("mixer:1:pan"), t0 + COALESCE_WINDOW * 2);
        p.mixer[1].muted = true;
        h.commit_with(&p, "mute", None, t0 + COALESCE_WINDOW * 2);
        p.mixer[1].muted = false;
        h.commit_with(&p, "mute", None, t0 + COALESCE_WINDOW * 2);
        let mut steps = 0;
        while h.undo(&mut p) {
            steps += 1;
        }
        assert_eq!(steps, 5);
        assert_eq!(p, Project::new_default());
    }
}
