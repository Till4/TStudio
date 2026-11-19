//! Snapshot-based undo/redo. The UI calls [`History::commit`] once per frame;
//! changes made while the mouse is held are merged into one undo step.

use crate::Project;

pub struct History {
    past: Vec<Project>,
    future: Vec<Project>,
    committed: Project,
    limit: usize,
}

impl History {
    pub fn new(project: &Project) -> Self {
        Self { past: Vec::new(), future: Vec::new(), committed: project.clone(), limit: 200 }
    }

    /// Records `current` as a new undo step if it differs from the last committed state.
    /// Call when no interaction (drag) is in progress.
    pub fn commit(&mut self, current: &Project) -> bool {
        if *current == self.committed {
            return false;
        }
        let prev = std::mem::replace(&mut self.committed, current.clone());
        self.past.push(prev);
        if self.past.len() > self.limit {
            self.past.remove(0);
        }
        self.future.clear();
        true
    }

    pub fn undo(&mut self, current: &mut Project) -> bool {
        // fold any uncommitted change in first so it can be redone
        self.commit(current);
        if let Some(prev) = self.past.pop() {
            let now = std::mem::replace(current, prev);
            self.future.push(now);
            self.committed = current.clone();
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, current: &mut Project) -> bool {
        if let Some(next) = self.future.pop() {
            let now = std::mem::replace(current, next);
            self.past.push(now);
            self.committed = current.clone();
            true
        } else {
            false
        }
    }

    /// Forget all history (after loading a project).
    pub fn reset(&mut self, project: &Project) {
        self.past.clear();
        self.future.clear();
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
}
