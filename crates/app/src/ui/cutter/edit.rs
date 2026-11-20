//! Pure selection/rendering helpers of the sample cutter.

use super::*;

/// Moves a selection to `start`, keeping its width and staying inside `0..len`.
pub(super) fn moved_selection(sel: (usize, usize), start: usize, len: usize) -> (usize, usize) {
    let width = sel.1.saturating_sub(sel.0);
    let start = start.min(len.saturating_sub(width));
    (start, start + width)
}

/// Moves a selection by `delta` samples (negative = left), keeping its width inside `0..len`.
pub(super) fn nudged_selection(sel: (usize, usize), delta: i64, len: usize) -> (usize, usize) {
    let start = if delta < 0 { sel.0.saturating_sub(delta.unsigned_abs() as usize) } else { sel.0.saturating_add(delta as usize) };
    moved_selection(sel, start, len)
}

/// One beat for Ctrl+←/→: the selection's own grid (width / `beats`), or – while the whole file
/// is selected – one beat of the project tempo.
pub(super) fn beat_step(sel: (usize, usize), beats: u32, len: usize, sr: f64, bpm: f64) -> usize {
    let width = sel.1.saturating_sub(sel.0);
    let whole = sel.0 == 0 && sel.1 >= len;
    let step = if whole || width == 0 { 60.0 / bpm.max(1.0) * sr } else { width as f64 / beats.max(1) as f64 };
    (step.round() as usize).max(1)
}

/// Fine step for Ctrl+Shift+←/→: 1 ms.
pub(super) fn fine_step(sr: f64) -> usize {
    ((sr / 1000.0).round() as usize).max(1)
}

/// Nearest zero crossing of the mono sum within `radius` samples.
pub(super) fn zero_near(d: &SampleData, i: usize, radius: usize) -> usize {
    let m = |k: usize| d.left[k] + d.right[k];
    if d.len() < 2 || i == 0 || i >= d.len() {
        return i.min(d.len());
    }
    for r in 0..radius {
        for k in [i.saturating_sub(r), i + r] {
            if k >= 1 && k < d.len() && (m(k - 1) <= 0.0) != (m(k) <= 0.0) {
                return k;
            }
        }
    }
    i
}

/// Audio of [a, b) with isolation, tail trim, fades and normalisation applied.
pub(super) fn render_range(t: &Trim, a: usize, b: usize) -> Option<SampleData> {
    let d = t.data.as_ref()?;
    let (mut a, mut b) = (a.min(b), a.max(b).min(d.len()));
    let zr = (d.sample_rate / 1000) as usize;
    a = zero_near(d, a, zr);
    b = zero_near(d, b, zr).max(a + 1).min(d.len());
    if b <= a + 1 {
        return None;
    }
    let mut out = match t.processed_now() {
        Some((_, full, _)) => full.slice(a, b),
        None if t.iso_active() => t.iso.process_range(d, a, b),
        None => d.slice(a, b),
    };
    if t.trim_tail {
        // drop trailing audio below -60 dBFS (keep 5 ms)
        let thr = 0.001f32;
        let keep = (d.sample_rate / 200) as usize;
        if let Some(last) = (0..out.len()).rev().find(|i| out.left[*i].abs() > thr || out.right[*i].abs() > thr) {
            let end = (last + keep).min(out.len()).max(1);
            out.left.truncate(end);
            out.right.truncate(end);
        }
    }
    let sr = d.sample_rate as f32;
    out.fade((t.fade_in_ms * 0.001 * sr) as usize, (t.fade_out_ms * 0.001 * sr) as usize);
    if t.normalize {
        out.normalize(0.98);
    }
    Some(out)
}

/// `dir/stem NN.ext` with the first free number.
pub(super) fn numbered_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let clean: String = stem.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().to_string();
    for n in 1..10000 {
        let p = dir.join(format!("{clean} {n:02}.{ext}"));
        if !p.exists() {
            return p;
        }
    }
    unique_path(dir, &clean, ext)
}
