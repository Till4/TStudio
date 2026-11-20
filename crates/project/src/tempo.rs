//! Tempo track: BPM changes at tick positions, optionally reached by a linear ramp.
//!
//! `Project::bpm` is the tempo at tick 0; every [`TempoPoint`] changes it from its tick on.
//! A point with `ramp = true` is reached by a linear (per tick) ramp from the previous point.
//! [`TempoMap`] turns that into a fast tick ↔ seconds conversion for engine and UI.

use crate::{Project, PPQ};
use serde::{Deserialize, Serialize};

pub const MIN_BPM: f64 = 10.0;
pub const MAX_BPM: f64 = 999.0;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct TempoPoint {
    /// song position in ticks (> 0; the tempo at 0 is `Project::bpm`)
    pub tick: i64,
    pub bpm: f64,
    /// ramp linearly from the previous point to this one (false: jump at `tick`)
    #[serde(default)]
    pub ramp: bool,
}

/// One piece of the tempo curve: constant (`bpm0 == bpm1`) or a linear ramp over `len` ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Seg {
    tick: f64,
    secs: f64,
    bpm0: f64,
    bpm1: f64,
    /// ticks; infinite for the last segment
    len: f64,
}

impl Seg {
    fn is_const(&self) -> bool {
        self.bpm0 == self.bpm1
    }
    fn bpm_at(&self, x: f64) -> f64 {
        if self.is_const() {
            self.bpm0
        } else {
            self.bpm0 + (self.bpm1 - self.bpm0) * (x / self.len).clamp(0.0, 1.0)
        }
    }
    /// Seconds from the segment start to offset `x` ticks.
    fn secs_to(&self, x: f64) -> f64 {
        if self.is_const() {
            x * 60.0 / (PPQ as f64 * self.bpm0)
        } else {
            let k = (self.bpm1 - self.bpm0) / self.len;
            60.0 / (PPQ as f64 * k) * (self.bpm_at(x) / self.bpm0).ln()
        }
    }
    /// Tick offset reached `s` seconds after the segment start.
    fn ticks_after(&self, s: f64) -> f64 {
        if self.is_const() {
            s * PPQ as f64 * self.bpm0 / 60.0
        } else {
            let k = (self.bpm1 - self.bpm0) / self.len;
            let bpm = self.bpm0 * (s * PPQ as f64 * k / 60.0).exp();
            (bpm - self.bpm0) / k
        }
    }
}

/// Precomputed tempo curve (no allocation when queried; safe for the audio thread).
#[derive(Clone, Debug, PartialEq)]
pub struct TempoMap {
    segs: Vec<Seg>,
}

impl Default for TempoMap {
    fn default() -> Self {
        TempoMap::constant(120.0)
    }
}

fn clamp_bpm(b: f64) -> f64 {
    if b.is_finite() {
        b.clamp(MIN_BPM, MAX_BPM)
    } else {
        120.0
    }
}

impl TempoMap {
    pub fn constant(bpm: f64) -> TempoMap {
        TempoMap { segs: vec![Seg { tick: 0.0, secs: 0.0, bpm0: clamp_bpm(bpm), bpm1: clamp_bpm(bpm), len: f64::INFINITY }] }
    }

    /// `base` = tempo at tick 0; `points` in any order (points at tick <= 0 are ignored,
    /// the last of several points at the same tick wins).
    pub fn new(base: f64, points: &[TempoPoint]) -> TempoMap {
        let mut pts: Vec<TempoPoint> = points.iter().copied().filter(|p| p.tick > 0).collect();
        pts.sort_by_key(|p| p.tick);
        let mut segs = Vec::with_capacity(pts.len() + 1);
        let (mut t, mut secs, mut bpm) = (0.0f64, 0.0f64, clamp_bpm(base));
        for p in &pts {
            let pt = p.tick as f64;
            let target = clamp_bpm(p.bpm);
            if pt > t {
                let seg = Seg { tick: t, secs, bpm0: bpm, bpm1: if p.ramp { target } else { bpm }, len: pt - t };
                secs += seg.secs_to(seg.len);
                segs.push(seg);
                t = pt;
            }
            bpm = target;
        }
        segs.push(Seg { tick: t, secs, bpm0: bpm, bpm1: bpm, len: f64::INFINITY });
        TempoMap { segs }
    }

    /// true when the tempo never changes.
    pub fn is_constant(&self) -> bool {
        self.segs.len() == 1
    }

    fn seg_for_tick(&self, tick: f64) -> &Seg {
        let i = self.segs.partition_point(|s| s.tick <= tick);
        &self.segs[i.saturating_sub(1)]
    }

    fn seg_for_secs(&self, secs: f64) -> &Seg {
        let i = self.segs.partition_point(|s| s.secs <= secs);
        &self.segs[i.saturating_sub(1)]
    }

    /// Tempo at `tick` (negative ticks use the start tempo).
    pub fn bpm_at(&self, tick: f64) -> f64 {
        let s = self.seg_for_tick(tick);
        s.bpm_at(tick - s.tick)
    }

    /// Song time in seconds at `tick` (before 0: extrapolated with the start tempo).
    pub fn secs_at(&self, tick: f64) -> f64 {
        let s = self.seg_for_tick(tick);
        s.secs + s.secs_to(tick - s.tick)
    }

    /// Song position in ticks at `secs` (inverse of [`secs_at`](Self::secs_at)).
    pub fn tick_at(&self, secs: f64) -> f64 {
        let s = self.seg_for_secs(secs);
        s.tick + s.ticks_after(secs - s.secs)
    }

    /// Start of the next tempo segment after `tick` (infinite if none).
    pub fn next_change_after(&self, tick: f64) -> f64 {
        let i = self.segs.partition_point(|s| s.tick <= tick);
        self.segs.get(i).map(|s| s.tick).unwrap_or(f64::INFINITY)
    }

    /// Constant tempo over [a, b] (None while a ramp is running or a change lies inside).
    pub fn constant_between(&self, a: f64, b: f64) -> Option<f64> {
        let s = self.seg_for_tick(a);
        (s.is_const() && b <= s.tick + s.len).then_some(s.bpm0)
    }
}

/// Samples per tick at a constant tempo (same formula the engine always used).
pub fn samples_per_tick(sr: f64, bpm: f64) -> f64 {
    sr * 60.0 / (bpm.max(1.0) * PPQ as f64)
}

impl Project {
    pub fn tempo_map(&self) -> TempoMap {
        TempoMap::new(self.bpm, &self.tempo)
    }

    pub fn bpm_at(&self, tick: i64) -> f64 {
        if self.tempo.is_empty() {
            self.bpm
        } else {
            self.tempo_map().bpm_at(tick as f64)
        }
    }

    /// Song time (seconds) at a tick position, honouring the tempo track.
    pub fn tick_to_time(&self, tick: f64) -> f64 {
        self.tempo_map().secs_at(tick)
    }

    /// Tick position at a song time (seconds), honouring the tempo track.
    pub fn time_to_tick(&self, secs: f64) -> f64 {
        self.tempo_map().tick_at(secs)
    }

    /// Adds (or replaces at the same tick) a tempo point; keeps the list sorted.
    pub fn set_tempo_point(&mut self, tick: i64, bpm: f64, ramp: bool) {
        let bpm = clamp_bpm(bpm);
        if tick <= 0 {
            self.bpm = bpm;
            return;
        }
        self.tempo.retain(|p| p.tick != tick);
        self.tempo.push(TempoPoint { tick, bpm, ramp });
        self.sort_tempo();
    }

    pub fn sort_tempo(&mut self) {
        self.tempo.retain(|p| p.tick > 0 && p.bpm.is_finite());
        for p in self.tempo.iter_mut() {
            p.bpm = clamp_bpm(p.bpm);
        }
        self.tempo.sort_by_key(|p| p.tick);
        self.tempo.dedup_by(|b, a| {
            // keep the later entry for duplicate ticks
            if a.tick == b.tick {
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

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * a.abs().max(1.0)
    }

    #[test]
    fn constant_map_matches_plain_formula() {
        let m = TempoMap::constant(120.0);
        assert!(m.is_constant());
        // 120 bpm: one beat = 0.5 s
        assert!(close(m.secs_at(PPQ as f64), 0.5));
        assert!(close(m.tick_at(2.0), 4.0 * PPQ as f64));
        assert!(close(m.secs_at(-(PPQ as f64)), -0.5));
    }

    #[test]
    fn jump_changes_tempo_at_point() {
        let bar = 4 * PPQ;
        let m = TempoMap::new(120.0, &[TempoPoint { tick: bar, bpm: 60.0, ramp: false }]);
        assert_eq!(m.bpm_at(bar as f64 - 1.0), 120.0);
        assert_eq!(m.bpm_at(bar as f64), 60.0);
        // bar 1 at 120 = 2 s, then one beat at 60 = 1 s
        assert!(close(m.secs_at(bar as f64), 2.0));
        assert!(close(m.secs_at((bar + PPQ) as f64), 3.0));
        assert!(close(m.tick_at(3.0), (bar + PPQ) as f64));
        assert_eq!(m.next_change_after(0.0), bar as f64);
        assert_eq!(m.constant_between(0.0, bar as f64), Some(120.0));
        assert_eq!(m.constant_between(10.0, bar as f64 + 1.0), None);
    }

    #[test]
    fn ramp_integrates_and_inverts() {
        let len = 4 * PPQ;
        let m = TempoMap::new(100.0, &[TempoPoint { tick: len, bpm: 200.0, ramp: true }]);
        assert!(close(m.bpm_at(len as f64 / 2.0), 150.0));
        // ∫ 60/(PPQ·bpm(t)) dt over a linear ramp = 60·beats/(b1-b0)·ln(b1/b0)
        let expect = 60.0 * 4.0 / 100.0 * 2f64.ln();
        assert!(close(m.secs_at(len as f64), expect), "{} vs {expect}", m.secs_at(len as f64));
        for t in [0.0, 13.0, 200.0, 383.9, 384.0, 900.0] {
            assert!((m.tick_at(m.secs_at(t)) - t).abs() < 1e-6, "roundtrip {t}");
        }
        // after the ramp: constant 200
        assert_eq!(m.bpm_at(1000.0), 200.0);
        assert!(m.constant_between(10.0, 20.0).is_none());
    }

    #[test]
    fn old_files_without_tempo_load() {
        let s = Project::new_default().to_ron().unwrap();
        assert!(s.contains("tempo: []"));
        let old: String = s.lines().filter(|l| !l.trim_start().starts_with("tempo:")).collect::<Vec<_>>().join("
");
        let p = Project::from_ron(&old).unwrap();
        assert!(p.tempo.is_empty());
        assert!(p.tempo_map().is_constant());
    }

    #[test]
    fn project_points_stay_sorted_and_unique() {
        let mut p = Project::default();
        p.set_tempo_point(768, 150.0, false);
        p.set_tempo_point(384, 130.0, true);
        p.set_tempo_point(768, 160.0, false);
        p.set_tempo_point(0, 125.0, false);
        assert_eq!(p.bpm, 125.0);
        assert_eq!(p.tempo.iter().map(|t| (t.tick, t.bpm)).collect::<Vec<_>>(), vec![(384, 130.0), (768, 160.0)]);
        assert_eq!(p.bpm_at(1000), 160.0);
        assert!(close(p.time_to_tick(p.tick_to_time(500.0)), 500.0));
    }
}
