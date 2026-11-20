//! Step-sequencer view of a pattern: one row per (channel, key), one cell per 16th.
//! Writes the same notes as `tstudio-cli notes steps` (one 16th long).

use crate::{Id, Note, Project, PPQ};

/// Ticks per step (a 16th note).
pub const STEP: i64 = PPQ / 4;

impl Project {
    /// Number of 16th steps the step grid shows for `pattern` (its length, 16..=64).
    pub fn step_count(&self, pattern: Id) -> usize {
        ((self.pattern_length(pattern) / STEP) as usize).clamp(16, 64)
    }

    /// Velocity of the note on each of `n` steps (None = empty step).
    pub fn step_row(&self, pattern: Id, channel: Id, key: u8, n: usize) -> Vec<Option<f32>> {
        let mut row = vec![None; n];
        if let Some(p) = self.pattern(pattern) {
            for note in p.notes.iter().filter(|x| x.channel == channel && x.key == key && x.start % STEP == 0) {
                if let Some(cell) = row.get_mut((note.start / STEP) as usize) {
                    *cell = Some(note.velocity);
                }
            }
        }
        row
    }

    /// Toggles step `i`: removes the note there or adds a 16th note. Returns true if the step is now set.
    pub fn toggle_step(&mut self, pattern: Id, channel: Id, key: u8, i: usize, velocity: f32) -> bool {
        let start = i as i64 * STEP;
        let Some(p) = self.pattern_mut(pattern) else { return false };
        let before = p.notes.len();
        p.notes.retain(|n| !(n.channel == channel && n.key == key && n.start == start));
        if p.notes.len() != before {
            return false;
        }
        p.notes.push(Note { channel, key, start, len: STEP, velocity: velocity.clamp(0.0, 1.0) });
        p.notes.sort_by_key(|n| (n.start, n.key));
        true
    }

    /// Sets the velocity of every note of (channel, key) in `pattern`.
    pub fn set_row_velocity(&mut self, pattern: Id, channel: Id, key: u8, velocity: f32) {
        if let Some(p) = self.pattern_mut(pattern) {
            for n in p.notes.iter_mut().filter(|n| n.channel == channel && n.key == key) {
                n.velocity = velocity.clamp(0.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_steps_writes_sixteenths() {
        let mut p = Project::new_default();
        let pat = p.patterns[0].id;
        let ch = p.channels[0].id;
        assert_eq!(p.step_count(pat), 16);
        assert!(p.toggle_step(pat, ch, 36, 0, 0.8));
        assert!(p.toggle_step(pat, ch, 36, 4, 0.8));
        assert!(p.toggle_step(pat, ch, 38, 4, 1.0));
        let row = p.step_row(pat, ch, 36, 16);
        assert_eq!(row.iter().filter(|c| c.is_some()).count(), 2);
        assert_eq!(row[4], Some(0.8));
        let n = p.pattern(pat).unwrap().notes.iter().find(|n| n.key == 36 && n.start == 4 * STEP).copied().unwrap();
        assert_eq!(n.len, STEP);
        // toggling again clears only that cell
        assert!(!p.toggle_step(pat, ch, 36, 4, 0.8));
        assert_eq!(p.step_row(pat, ch, 36, 16)[4], None);
        assert_eq!(p.step_row(pat, ch, 38, 16)[4], Some(1.0));
        p.set_row_velocity(pat, ch, 38, 0.5);
        assert_eq!(p.step_row(pat, ch, 38, 16)[4], Some(0.5));
    }

    #[test]
    fn long_patterns_show_more_steps() {
        let mut p = Project::new_default();
        let pat = p.patterns[0].id;
        p.pattern_mut(pat).unwrap().length = Some(PPQ * 8);
        assert_eq!(p.step_count(pat), 32);
    }
}
