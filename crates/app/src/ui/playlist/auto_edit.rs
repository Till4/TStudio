//! Drawing automation curves directly in the playlist: on the selected automation clip
//! a click into the curve area adds a point, dragging moves it (time snaps to the playlist grid,
//! Alt = free), right click deletes it. The clip header and edges keep moving / resizing the clip.

use super::*;

/// Height of a clip's header strip (moves the clip) above the curve area.
const HEAD: f32 = 15.0;
/// Clip edges stay resize handles.
const EDGE: f32 = 8.0;

/// Curve area of an automation clip drawn in `r`.
pub(super) fn auto_body(r: Rect) -> Rect {
    Rect::from_min_max(pos2(r.left(), r.top() + HEAD), pos2(r.right(), r.bottom() - 2.0))
}

/// Inserts a point (replacing one at the same tick); returns its index.
pub(super) fn insert_point(points: &mut Vec<AutoPoint>, t: i64, v: f32) -> usize {
    points.retain(|p| p.t != t);
    let i = points.partition_point(|p| p.t < t);
    points.insert(i, AutoPoint { t, v });
    i
}

/// Moves point `i` to (t, v); `t` stays between its neighbours (equal ticks = a step) and in 0..=len.
pub(super) fn move_point(points: &mut [AutoPoint], i: usize, t: i64, v: f32, len: i64) {
    let Some(_) = points.get(i) else { return };
    let lo = if i > 0 { points[i - 1].t } else { 0 };
    let hi = points.get(i + 1).map(|p| p.t).unwrap_or(len).min(len);
    points[i] = AutoPoint { t: t.clamp(lo, hi.max(lo)), v };
}

/// Clip-local tick for a song position, snapped to the song grid (`snap` ticks) and clamped to the clip.
pub(super) fn snap_local(start: i64, song_tick: f64, snap: i64, len: i64) -> i64 {
    (round_to(song_tick.round() as i64, snap.max(1)) - start).clamp(0, len)
}

/// The single selected automation clip and its rect, if `pt` lies in its curve area (not on an edge).
fn selected_auto_at(app: &TStudio, pt: Pos2, clip_rect: &dyn Fn(&tstudio_project::Clip) -> Rect) -> Option<(usize, Rect)> {
    if app.playlist_ui.selected.len() != 1 {
        return None;
    }
    let id = *app.playlist_ui.selected.iter().next()?;
    let idx = app.project.playlist.clips.iter().position(|c| c.id == id)?;
    let c = &app.project.playlist.clips[idx];
    if !matches!(c.kind, ClipKind::Automation(_)) {
        return None;
    }
    let r = clip_rect(c);
    let body = auto_body(r);
    let inner = Rect::from_min_max(pos2(r.left() + EDGE, body.top()), pos2(r.right() - EDGE, body.bottom()));
    inner.contains(pt).then_some((idx, r))
}

fn point_pos(p: AutoPoint, len: i64, r: Rect, range: (f32, f32, bool)) -> Pos2 {
    let body = auto_body(r);
    pos2(r.left() + (p.t as f32 / len.max(1) as f32) * r.width(), body.bottom() - normalize(p.v, range) * body.height())
}

/// Index of the point of the selected automation clip under `pt`.
pub(super) fn auto_point_at(app: &TStudio, pt: Pos2, clip_rect: &dyn Fn(&tstudio_project::Clip) -> Rect) -> Option<(Id, usize)> {
    let (idx, r) = selected_auto_at(app, pt, clip_rect)?;
    let c = &app.project.playlist.clips[idx];
    let ClipKind::Automation(a) = &c.kind else { return None };
    let range = field_range(Project::auto_field_name(&a.target));
    a.points.iter().position(|q| point_pos(*q, c.len, r, range).distance(pt) < 6.0).map(|i| (c.id, i))
}

/// Primary press: grabs the point under the pointer or adds one. Returns (clip id, point index).
pub(super) fn auto_edit_press(app: &mut TStudio, pt: Pos2, clip_rect: &dyn Fn(&tstudio_project::Clip) -> Rect, tick_of: &dyn Fn(f32) -> f64, snap: i64) -> Option<(Id, usize)> {
    if let Some(hit) = auto_point_at(app, pt, clip_rect) {
        return Some(hit);
    }
    let (idx, r) = selected_auto_at(app, pt, clip_rect)?;
    let c = &mut app.project.playlist.clips[idx];
    let (id, start, len) = (c.id, c.start, c.len);
    let ClipKind::Automation(a) = &mut c.kind else { return None };
    let range = field_range(Project::auto_field_name(&a.target));
    let body = auto_body(r);
    let t = snap_local(start, tick_of(pt.x), snap, len);
    let v = denormalize((body.bottom() - pt.y) / body.height(), range);
    Some((id, insert_point(&mut a.points, t, v)))
}

/// Drag of a grabbed point.
pub(super) fn auto_edit_drag(app: &mut TStudio, clip: Id, i: usize, pt: Pos2, clip_rect: &dyn Fn(&tstudio_project::Clip) -> Rect, tick_of: &dyn Fn(f32) -> f64, snap: i64) {
    let Some(c) = app.project.playlist.clips.iter_mut().find(|c| c.id == clip) else { return };
    let r = clip_rect(c);
    let (start, len) = (c.start, c.len);
    let ClipKind::Automation(a) = &mut c.kind else { return };
    let range = field_range(Project::auto_field_name(&a.target));
    let body = auto_body(r);
    let t = snap_local(start, tick_of(pt.x), snap, len);
    let v = denormalize((body.bottom() - pt.y) / body.height(), range);
    move_point(&mut a.points, i, t, v, len);
}

/// Right click on a point: removes it (the last point stays).
pub(super) fn auto_edit_delete(app: &mut TStudio, clip: Id, i: usize) {
    if let Some(ClipKind::Automation(a)) = app.project.playlist.clips.iter_mut().find(|c| c.id == clip).map(|c| &mut c.kind) {
        if a.points.len() > 1 && i < a.points.len() {
            a.points.remove(i);
        }
    }
}

/// Point handles + value readout on the selected automation clip.
pub(super) fn paint_auto_handles(gp: &egui::Painter, a: &AutomationClip, len: i64, r: Rect, active: Option<usize>, hover: Option<Pos2>) {
    let range = field_range(Project::auto_field_name(&a.target));
    for (i, q) in a.points.iter().enumerate() {
        let c = point_pos(*q, len, r, range);
        gp.circle_filled(c, if active == Some(i) { 5.0 } else { 3.5 }, Color32::WHITE);
        gp.circle_stroke(c, if active == Some(i) { 5.0 } else { 3.5 }, Stroke::new(1.0_f32, AUTO_COLOR));
    }
    let fmt = |v: f32| if range.2 { widgets::fmt_hz(v) } else { format!("{v:.2}") };
    let body = auto_body(r);
    let shown = active.and_then(|i| a.points.get(i).map(|q| q.v)).or_else(|| hover.filter(|p| body.contains(*p)).map(|p| denormalize((body.bottom() - p.y) / body.height(), range)));
    if let Some(v) = shown {
        gp.text(pos2(r.right() - 4.0, r.top() + 1.0), Align2::RIGHT_TOP, fmt(v), FontId::proportional(10.0), Color32::BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(v: &[(i64, f32)]) -> Vec<AutoPoint> {
        v.iter().map(|&(t, v)| AutoPoint { t, v }).collect()
    }

    #[test]
    fn insert_keeps_order_and_replaces_same_tick() {
        let mut p = pts(&[(0, 0.0), (384, 1.0)]);
        assert_eq!(insert_point(&mut p, 192, 0.5), 1);
        assert_eq!(insert_point(&mut p, 192, 0.7), 1);
        assert_eq!(p, pts(&[(0, 0.0), (192, 0.7), (384, 1.0)]));
    }

    #[test]
    fn move_stays_between_neighbours() {
        let mut p = pts(&[(0, 0.0), (192, 0.5), (384, 1.0)]);
        move_point(&mut p, 1, 500, 0.2, 384);
        assert_eq!(p[1], AutoPoint { t: 384, v: 0.2 });
        move_point(&mut p, 1, -10, 0.3, 384);
        assert_eq!(p[1], AutoPoint { t: 0, v: 0.3 });
        move_point(&mut p, 2, 900, 0.9, 384);
        assert_eq!(p[2].t, 384);
    }

    #[test]
    fn local_ticks_snap_to_song_grid() {
        // clip starts off-grid at 100; a beat grid (96) snaps song positions to 96, 192 …
        assert_eq!(snap_local(100, 150.0, 96, 384), 92);
        assert_eq!(snap_local(100, 60.0, 96, 384), 0);
        assert_eq!(snap_local(0, 1000.0, 96, 384), 384);
        assert_eq!(snap_local(0, 37.0, 1, 384), 37);
    }
}
