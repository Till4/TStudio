//! Slicing a sample for the Sampler's Chop mode: slice i plays on key `slice_root + i`.
//! Shared by the instrument view, the drum pads and `tstudio-cli channel slice`.

use crate::sampler::{SamplerMode, SamplerParams};
use crate::sample::SampleData;

/// Shortest gap between two transient slices.
pub const MIN_GAP_MS: f32 = 70.0;

/// `n` equal slices between `start` and `end` (fractions 0..1).
pub fn even_slices(start: f32, end: f32, n: usize) -> Vec<f32> {
    let (s, e) = (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0));
    let n = n.max(1);
    (0..n).map(|i| s + (e - s) * i as f32 / n as f32).collect()
}

/// Slices at the transients between `start` and `end` (fractions 0..1); the first slice
/// always begins at `start`. `sensitivity` 0..1 (higher = more slices).
pub fn transient_slices(data: &SampleData, sensitivity: f32, start: f32, end: f32) -> Vec<f32> {
    let len = data.len().max(1) as f32;
    let pos = crate::analysis::transients(&data.mono(), data.sample_rate, sensitivity, MIN_GAP_MS);
    let fr: Vec<f32> = pos.iter().map(|p| *p as f32 / len).collect();
    clip_to_region(fr, start, end)
}

/// Keeps slice points inside `start..end`, sorted and deduplicated, starting at `start`.
pub fn clip_to_region(mut fr: Vec<f32>, start: f32, end: f32) -> Vec<f32> {
    let (s, e) = (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0));
    fr.retain(|f| *f > s + 1e-4 && *f < e - 1e-4);
    fr.push(s);
    fr.sort_by(|a, b| a.total_cmp(b));
    fr.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    fr
}

/// Puts the sampler into Chop mode with `slices`, mapped from `root` upward. Slices that would
/// land above key 127 are dropped. Returns the number of mapped slices.
pub fn apply_slices(p: &mut SamplerParams, slices: Vec<f32>, root: Option<u8>) -> usize {
    if let Some(r) = root {
        p.slice_root = r.min(127);
    }
    let room = 128 - p.slice_root as usize;
    let mut s = slices;
    s.sort_by(|a, b| a.total_cmp(b));
    s.truncate(room);
    p.slices = s;
    p.mode = SamplerMode::Chop;
    p.slices.len()
}

/// Key of slice `i` (None if it does not exist).
pub fn slice_key(p: &SamplerParams, i: usize) -> Option<u8> {
    (i < p.slices.len()).then(|| p.slice_root as usize + i).filter(|k| *k <= 127).map(|k| k as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_slices_cover_the_region() {
        assert_eq!(even_slices(0.0, 1.0, 4), vec![0.0, 0.25, 0.5, 0.75]);
        assert_eq!(even_slices(0.5, 1.0, 2), vec![0.5, 0.75]);
        assert_eq!(even_slices(0.0, 1.0, 0), vec![0.0]);
    }

    #[test]
    fn region_clipping_starts_at_start() {
        assert_eq!(clip_to_region(vec![0.0, 0.1, 0.3, 0.9], 0.2, 0.8), vec![0.2, 0.3]);
        assert_eq!(clip_to_region(vec![0.5, 0.25, 0.25], 0.0, 1.0), vec![0.0, 0.25, 0.5]);
    }

    #[test]
    fn transients_of_clicks_become_slices() {
        // four clicks in one second of silence
        let sr = 44100;
        let mut v = vec![0.0f32; sr as usize];
        for k in 0..4 {
            let at = 2000 + k * 10000;
            for i in 0..400 {
                v[at + i] = if i % 2 == 0 { 0.9 } else { -0.9 } * (1.0 - i as f32 / 400.0);
            }
        }
        let d = SampleData::new(v.clone(), v, sr);
        let s = transient_slices(&d, 0.5, 0.0, 1.0);
        assert_eq!(s[0], 0.0);
        assert!(s.len() >= 4, "{s:?}");
    }

    #[test]
    fn apply_maps_keys_from_root() {
        let mut p = SamplerParams::default();
        assert_eq!(apply_slices(&mut p, vec![0.5, 0.0, 0.25], Some(36)), 3);
        assert_eq!(p.mode, SamplerMode::Chop);
        assert_eq!(p.slices, vec![0.0, 0.25, 0.5]);
        assert_eq!(slice_key(&p, 0), Some(36));
        assert_eq!(slice_key(&p, 2), Some(38));
        assert_eq!(slice_key(&p, 3), None);
        // key 36 plays slice 0, key 38 slice 2
        assert!(p.region_for_key(38, 1000).is_some());
        assert!(p.region_for_key(39, 1000).is_none());
        // no keys above 127
        assert_eq!(apply_slices(&mut p, even_slices(0.0, 1.0, 20), Some(120)), 8);
    }
}
