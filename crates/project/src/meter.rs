//! Time-signature changes: from a given bar on, a bar has `beats_per_bar` beats of `1/beat_unit`
//! (4/4 → 7/8 → 4/4 …). `Project::beats_per_bar` (quarter beats) is the meter from bar 1 on.
//! [`BarMap`] converts between bar numbers and ticks and yields the metronome beats.

use crate::{Project, PPQ};
use serde::{Deserialize, Serialize};

fn default_unit() -> u32 {
    4
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct MeterChange {
    /// 0-based bar index where the new meter starts (> 0; bar 0 uses `Project::beats_per_bar`)
    pub bar: u32,
    pub beats_per_bar: u32,
    /// note value of one beat: 4 = quarter, 8 = eighth …
    #[serde(default = "default_unit")]
    pub beat_unit: u32,
}

impl MeterChange {
    pub fn label(&self) -> String {
        format!("{}/{}", self.beats_per_bar, self.beat_unit)
    }
}

/// Valid beat unit (power of two, 1..=32).
pub fn clamp_unit(u: u32) -> u32 {
    match u {
        0 | 1 => 1,
        2 | 3 => 2,
        4..=5 => 4,
        6..=11 => 8,
        12..=23 => 16,
        _ => 32,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterSeg {
    /// first bar (0-based) and its tick
    pub bar: i64,
    pub tick: i64,
    pub beats: u32,
    pub unit: u32,
    /// ticks of one beat / one bar
    pub beat_ticks: i64,
    pub bar_ticks: i64,
}

/// Precomputed bar grid (no allocation when queried; safe for the audio thread).
#[derive(Clone, Debug, PartialEq)]
pub struct BarMap {
    segs: Vec<MeterSeg>,
}

impl Default for BarMap {
    fn default() -> Self {
        BarMap::new(4, &[])
    }
}

fn seg(bar: i64, tick: i64, beats: u32, unit: u32) -> MeterSeg {
    let beats = beats.clamp(1, 64);
    let unit = clamp_unit(unit);
    let beat_ticks = (PPQ * 4 / unit as i64).max(1);
    MeterSeg { bar, tick, beats, unit, beat_ticks, bar_ticks: beat_ticks * beats as i64 }
}

impl BarMap {
    /// `base` = quarter beats per bar from bar 0; changes in any order (bar 0 entries replace the base).
    pub fn new(base: u32, changes: &[MeterChange]) -> BarMap {
        let mut ch: Vec<MeterChange> = changes.to_vec();
        ch.sort_by_key(|c| c.bar);
        let mut segs = vec![seg(0, 0, base, 4)];
        for c in ch {
            let last = *segs.last().unwrap();
            let bar = c.bar as i64;
            if bar <= last.bar {
                // same bar: the later entry wins
                let s = seg(last.bar, last.tick, c.beats_per_bar, c.beat_unit);
                *segs.last_mut().unwrap() = s;
            } else {
                let tick = last.tick + (bar - last.bar) * last.bar_ticks;
                segs.push(seg(bar, tick, c.beats_per_bar, c.beat_unit));
            }
        }
        // drop changes that do not change anything
        segs.dedup_by(|b, a| a.beats == b.beats && a.unit == b.unit);
        BarMap { segs }
    }

    pub fn is_constant(&self) -> bool {
        self.segs.len() == 1
    }

    pub fn segments(&self) -> &[MeterSeg] {
        &self.segs
    }

    fn seg_for_tick(&self, tick: f64) -> &MeterSeg {
        let i = self.segs.partition_point(|s| s.tick as f64 <= tick);
        &self.segs[i.saturating_sub(1)]
    }

    fn seg_for_bar(&self, bar: f64) -> &MeterSeg {
        let i = self.segs.partition_point(|s| s.bar as f64 <= bar);
        &self.segs[i.saturating_sub(1)]
    }

    /// Meter in effect at `tick`.
    pub fn meter_at(&self, tick: f64) -> MeterSeg {
        *self.seg_for_tick(tick)
    }

    /// Tick of a (0-based, possibly fractional) bar position.
    pub fn bar_to_tick(&self, bar: f64) -> f64 {
        let s = self.seg_for_bar(bar);
        s.tick as f64 + (bar - s.bar as f64) * s.bar_ticks as f64
    }

    /// 0-based fractional bar position of a tick.
    pub fn tick_to_bar(&self, tick: f64) -> f64 {
        let s = self.seg_for_tick(tick);
        s.bar as f64 + (tick - s.tick as f64) / s.bar_ticks as f64
    }

    /// Start tick of the bar containing `tick` and that bar's length.
    pub fn bar_bounds(&self, tick: i64) -> (i64, i64) {
        let s = self.seg_for_tick(tick as f64);
        let k = (tick - s.tick).div_euclid(s.bar_ticks);
        (s.tick + k * s.bar_ticks, s.bar_ticks)
    }

    /// First beat at or after `t`: (tick, is the bar's downbeat).
    pub fn beat_at_or_after(&self, t: f64) -> (i64, bool) {
        let i = self.segs.partition_point(|s| s.tick as f64 <= t).saturating_sub(1);
        let s = &self.segs[i];
        let k_bar = ((t - s.tick as f64) / s.bar_ticks as f64).floor() as i64;
        let bar_start = s.tick + k_bar * s.bar_ticks;
        let k = ((t - bar_start as f64) / s.beat_ticks as f64).ceil() as i64;
        let tick = bar_start + k * s.beat_ticks;
        if let Some(next) = self.segs.get(i + 1) {
            if tick >= next.tick {
                return (next.tick, true);
            }
        }
        (tick, (tick - s.tick).rem_euclid(s.bar_ticks) == 0)
    }
}

impl Project {
    pub fn bar_map(&self) -> BarMap {
        BarMap::new(self.beats_per_bar, &self.meters)
    }

    /// Tick of a 1-based (fractional) bar number, honouring time-signature changes.
    pub fn bar_number_to_tick(&self, bar: f64) -> i64 {
        self.bar_map().bar_to_tick(bar - 1.0).round() as i64
    }

    /// 1-based (fractional) bar number of a tick.
    pub fn tick_to_bar_number(&self, tick: i64) -> f64 {
        self.bar_map().tick_to_bar(tick as f64) + 1.0
    }

    /// Ticks covered by `bars` bars starting at `from` (bar lengths may differ).
    pub fn bars_to_ticks_from(&self, from: i64, bars: f64) -> i64 {
        let m = self.bar_map();
        let b = m.tick_to_bar(from as f64);
        (m.bar_to_tick(b + bars) - from as f64).round() as i64
    }

    /// Sets (or replaces) a meter change at a 0-based bar; bar 0 changes the base meter
    /// (only quarter-note meters can be the base; others become a change at bar 0 is not allowed).
    pub fn set_meter(&mut self, bar: u32, beats: u32, unit: u32) {
        let beats = beats.clamp(1, 64);
        let unit = clamp_unit(unit);
        if bar == 0 && unit == 4 {
            self.beats_per_bar = beats;
            return;
        }
        self.meters.retain(|m| m.bar != bar);
        self.meters.push(MeterChange { bar, beats_per_bar: beats, beat_unit: unit });
        self.sort_meters();
    }

    pub fn sort_meters(&mut self) {
        for m in self.meters.iter_mut() {
            m.beats_per_bar = m.beats_per_bar.clamp(1, 64);
            m.beat_unit = clamp_unit(m.beat_unit);
        }
        self.meters.sort_by_key(|m| m.bar);
        self.meters.dedup_by(|b, a| {
            if a.bar == b.bar {
                *a = *b;
                true
            } else {
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_4_78_4() -> BarMap {
        // bars 1-2: 4/4, bar 3: 7/8, from bar 4: 4/4
        BarMap::new(4, &[MeterChange { bar: 2, beats_per_bar: 7, beat_unit: 8 }, MeterChange { bar: 3, beats_per_bar: 4, beat_unit: 4 }])
    }

    #[test]
    fn bars_and_ticks_across_changes() {
        let m = map_4_78_4();
        let bar = 4 * PPQ;
        let b78 = 7 * PPQ / 2;
        assert_eq!(m.bar_to_tick(2.0), (2 * bar) as f64);
        assert_eq!(m.bar_to_tick(3.0), (2 * bar + b78) as f64);
        assert_eq!(m.bar_to_tick(4.0), (3 * bar + b78) as f64);
        assert_eq!(m.tick_to_bar((2 * bar + b78) as f64), 3.0);
        assert_eq!(m.bar_bounds(2 * bar + 10), (2 * bar, b78));
        assert_eq!(m.meter_at((2 * bar) as f64).unit, 8);
    }

    #[test]
    fn metronome_beats_follow_meter() {
        let m = map_4_78_4();
        let mut beats = Vec::new();
        let mut t = 0.0;
        while t < (4 * 4 * PPQ) as f64 {
            let (b, down) = m.beat_at_or_after(t);
            beats.push((b, down));
            t = b as f64 + 1.0;
        }
        let downs: Vec<i64> = beats.iter().filter(|b| b.1).map(|b| b.0).collect();
        assert_eq!(downs, vec![0, 384, 768, 768 + 336, 768 + 336 + 384]);
        // 7 eighth clicks in the 7/8 bar
        assert_eq!(beats.iter().filter(|b| b.0 >= 768 && b.0 < 768 + 336).count(), 7);
    }

    #[test]
    fn constant_map_matches_old_grid() {
        let m = BarMap::new(3, &[]);
        assert!(m.is_constant());
        assert_eq!(m.beat_at_or_after(97.0), (192, false));
        assert_eq!(m.beat_at_or_after(200.0), (288, true));
        let mut p = Project::default();
        assert_eq!(p.bar_number_to_tick(3.0), 2 * p.ticks_per_bar());
        p.set_meter(1, 7, 8);
        p.set_meter(2, 4, 4);
        assert_eq!(p.bar_number_to_tick(3.0), 384 + 336);
        assert_eq!(p.tick_to_bar_number(384 + 336), 3.0);
        assert_eq!(p.bars_to_ticks_from(0, 2.0), 384 + 336);
    }
}
