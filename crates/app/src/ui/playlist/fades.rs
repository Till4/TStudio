//! Fade-in/-out display and handles of audio clips in the playlist.
use super::*;

/// Height of the strip at the top of a clip where the fade handles can be grabbed.
pub(super) const FADE_HANDLE_BAND: f32 = 13.0;
/// How close (px) the pointer has to be to a fade handle to grab it.
pub(super) const FADE_GRAB_PX: f32 = 6.0;
/// Narrower clips show no fade handles (the edges are needed for resizing).
pub(super) const FADE_MIN_CLIP_PX: f32 = 24.0;

/// Screen x of the fade-in / fade-out handle: at the end of the fade, but kept inside the clip.
pub(super) fn fade_handle_xs(r: Rect, x_in: f32, x_out: f32) -> (f32, f32) {
    let (lo, hi) = (r.left() + 5.0, (r.right() - 5.0).max(r.left() + 5.0));
    (x_in.clamp(lo, hi), x_out.clamp(lo, hi))
}

/// Darkens the faded part of the clip, draws the (linear, like the engine) fade line
/// and the square handles at the top.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_fades(p: &egui::Painter, r: Rect, body: Rect, x_in: f32, x_out: f32, hot_in: bool, hot_out: bool, alpha: f32) {
    let shade = Color32::from_black_alpha((120.0 * alpha) as u8);
    let line = Stroke::new(1.5, Color32::from_white_alpha((220.0 * alpha) as u8));
    let (top, bottom) = (body.top(), body.bottom());
    if x_in > r.left() + 0.5 {
        p.add(egui::Shape::convex_polygon(vec![pos2(r.left(), top), pos2(x_in, top), pos2(r.left(), bottom)], shade, Stroke::NONE));
        p.line_segment([pos2(r.left(), bottom), pos2(x_in, top)], line);
    }
    if x_out < r.right() - 0.5 {
        p.add(egui::Shape::convex_polygon(vec![pos2(x_out, top), pos2(r.right(), top), pos2(r.right(), bottom)], shade, Stroke::NONE));
        p.line_segment([pos2(x_out, top), pos2(r.right(), bottom)], line);
    }
    if r.width() < FADE_MIN_CLIP_PX {
        return;
    }
    let (hi, ho) = fade_handle_xs(r, x_in, x_out);
    let y = r.top() + FADE_HANDLE_BAND * 0.5;
    for (x, hot) in [(hi, hot_in), (ho, hot_out)] {
        let size = if hot { 8.0 } else { 6.0 };
        let sq = Rect::from_center_size(pos2(x, y), vec2(size, size));
        let fill = if hot { Color32::WHITE } else { Color32::from_white_alpha((170.0 * alpha) as u8) };
        p.rect_filled(sq, 1.0, fill);
        p.rect_stroke(sq, 1.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }
}
