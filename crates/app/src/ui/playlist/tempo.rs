//! Tempo lane under the playlist ruler: the tempo track (BPM points, optional linear ramps)
//! drawn as a curve and edited with the mouse, plus the time-signature changes (tags at the bar).
//!
//! Doppelklick = Punkt setzen · Ziehen = verschieben (waagerecht) / BPM (senkrecht, Shift = fein)
//! · Rechtsklick = Menü (BPM, Rampe, Löschen).

use super::*;
use tstudio_project::TempoPoint;

pub(super) const TEMPO_LANE_H: f32 = 18.0;
const TEMPO_COLOR: Color32 = Color32::from_rgb(110, 215, 195);

/// Drag in progress: (point index, press position, tick and bpm at the press, axis: 0 = undecided, 1 = time, 2 = bpm).
type Drag = (usize, Pos2, i64, f64, u8);

/// Index of the tempo point drawn within 6 px of `x`.
fn hit_point(points: &[TempoPoint], x: f32, x_of: &impl Fn(f64) -> f32) -> Option<usize> {
    points.iter().enumerate().map(|(i, p)| (i, (x_of(p.tick as f64) - x).abs())).filter(|(_, d)| *d < 6.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
}

/// Value range shown in the lane (all tempos plus a little headroom).
pub(super) fn lane_range(base: f64, points: &[TempoPoint]) -> (f64, f64) {
    let lo = points.iter().map(|p| p.bpm).fold(base, f64::min);
    let hi = points.iter().map(|p| p.bpm).fold(base, f64::max);
    let pad = ((hi - lo) * 0.15).max(5.0);
    (lo - pad, hi + pad)
}

/// Polyline of the tempo curve in (tick, bpm): jumps are vertical steps, ramps straight lines.
pub(super) fn tempo_curve(base: f64, points: &[TempoPoint], end: i64) -> Vec<(i64, f64)> {
    let mut out = vec![(0, base)];
    let mut cur = base;
    for p in points {
        if !p.ramp {
            out.push((p.tick, cur));
        }
        out.push((p.tick, p.bpm));
        cur = p.bpm;
    }
    let last = out.last().map(|l| l.0).unwrap_or(0);
    out.push((end.max(last), cur));
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn tempo_lane(app: &mut TStudio, ui: &mut Ui, painter: &egui::Painter, lane: Rect, x_of: impl Fn(f64) -> f32, tick_of: impl Fn(f32) -> f64, snap_t: i64, t1: i64) {
    let id = ui.id().with("pl_tempo");
    let resp = ui.interact(lane, id, Sense::click_and_drag());
    let drag_id = id.with("drag");
    let menu_id = id.with("menu");
    let mut drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id)).flatten();
    let snap = |t: f64| round_to(t as i64, snap_t.max(1)).max(1);

    if resp.double_clicked() {
        if let Some(pt) = resp.interact_pointer_pos() {
            if hit_point(&app.project.tempo, pt.x, &x_of).is_none() {
                let tick = snap(tick_of(pt.x));
                let bpm = (app.project.bpm_at(tick) * 100.0).round() / 100.0;
                app.project.set_tempo_point(tick, bpm, false);
                app.set_status(format!("Tempo-Punkt bei Takt {:.2}: {bpm:.2} BPM – senkrecht ziehen ändert das Tempo, Rechtsklick = Rampe/Löschen", tick as f64 / app.project.ticks_per_bar() as f64 + 1.0));
            }
        }
    }
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        if let Some(o) = origin {
            drag = hit_point(&app.project.tempo, o.x, &x_of).map(|i| {
                let p = app.project.tempo[i];
                (i, o, p.tick, p.bpm, 0)
            });
        }
    }
    if resp.dragged() {
        if let (Some((i, o, tick0, bpm0, mut axis)), Some(pt)) = (drag, resp.interact_pointer_pos()) {
            let d = pt - o;
            if axis == 0 && d.length() > 4.0 {
                axis = if d.x.abs() > d.y.abs() { 1 } else { 2 };
            }
            let fine = ui.input(|i| i.modifiers.shift);
            let n = app.project.tempo.len();
            if i < n {
                // neighbours keep the order while dragging
                let lo = if i > 0 { app.project.tempo[i - 1].tick + 1 } else { 1 };
                let hi = if i + 1 < n { app.project.tempo[i + 1].tick - 1 } else { i64::MAX };
                let p = &mut app.project.tempo[i];
                match axis {
                    1 => p.tick = snap(tick0 as f64 + (tick_of(pt.x) - tick_of(o.x))).clamp(lo, hi.max(lo)),
                    2 => {
                        let per_px = if fine { 0.05 } else { 0.5 };
                        p.bpm = ((bpm0 - d.y as f64 * per_px) * 100.0).round() / 100.0;
                        p.bpm = p.bpm.clamp(tstudio_project::tempo::MIN_BPM, tstudio_project::tempo::MAX_BPM);
                    }
                    _ => {}
                }
            }
            drag = Some((i, o, tick0, bpm0, axis));
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        drag = None;
    }
    ui.data_mut(|d| d.insert_temp(drag_id, drag));

    // right click: menu for the point under the pointer (or "add here")
    if resp.secondary_clicked() {
        if let Some(pt) = resp.interact_pointer_pos() {
            let target: (Option<usize>, i64) = (hit_point(&app.project.tempo, pt.x, &x_of), snap(tick_of(pt.x)));
            ui.data_mut(|d| {
                d.insert_temp(menu_id, target);
                d.remove::<(u32, u32)>(menu_id.with("meter"));
            });
        }
    }
    resp.context_menu(|ui| {
        let (point, tick): (Option<usize>, i64) = ui.data(|d| d.get_temp(menu_id)).unwrap_or((None, 0));
        match point.filter(|i| *i < app.project.tempo.len()) {
            Some(i) => {
                let bar = app.project.ticks_per_bar() as f64;
                ui.label(RichText::new(format!("Tempo-Punkt · Takt {:.2}", app.project.tempo[i].tick as f64 / bar + 1.0)).strong());
                ui.horizontal(|ui| {
                    ui.label("BPM");
                    ui.add(egui::DragValue::new(&mut app.project.tempo[i].bpm).speed(0.1).range(tstudio_project::tempo::MIN_BPM..=tstudio_project::tempo::MAX_BPM).fixed_decimals(2));
                });
                ui.checkbox(&mut app.project.tempo[i].ramp, "Rampe (gleitend vom vorigen Punkt)");
                if ui.button("🗑 Löschen").clicked() {
                    app.project.tempo.remove(i);
                    ui.close_menu();
                }
            }
            None => {
                if ui.button("♩ Tempo-Punkt hier setzen").clicked() {
                    let bpm = (app.project.bpm_at(tick) * 100.0).round() / 100.0;
                    app.project.set_tempo_point(tick, bpm, false);
                    ui.close_menu();
                }
                if !app.project.tempo.is_empty() && ui.button("Alle Tempo-Punkte löschen").clicked() {
                    app.project.tempo.clear();
                    ui.close_menu();
                }
                meter_menu(app, ui, menu_id.with("meter"), tick);
            }
        }
    });
    if resp.hovered() && drag.is_none() {
        let hint = match resp.hover_pos().and_then(|p| hit_point(&app.project.tempo, p.x, &x_of)) {
            Some(i) => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                let p = app.project.tempo[i];
                format!("{:.2} BPM{} – waagerecht ziehen = verschieben, senkrecht = Tempo (Shift fein), Rechtsklick = Menü", p.bpm, if p.ramp { " (Rampe)" } else { "" })
            }
            None => "Tempo-Spur: Doppelklick = Tempo-Punkt setzen · Rechtsklick = Menü (Tempo, Taktart ab hier)".to_string(),
        };
        resp.on_hover_text(hint);
    }

    // ---- painting
    let p = painter.with_clip_rect(lane);
    p.rect_filled(lane, 0.0, Color32::from_rgb(26, 30, 34));
    p.line_segment([pos2(lane.left(), lane.bottom() - 0.5), pos2(lane.right(), lane.bottom() - 0.5)], Stroke::new(1.0_f32, Color32::from_gray(50)));
    let pts = &app.project.tempo;
    let base = app.project.bpm;
    let (lo, hi) = lane_range(base, pts);
    let y_of = |b: f64| lane.bottom() - 3.0 - ((b - lo) / (hi - lo)) as f32 * (lane.height() - 6.0);
    let line: Vec<Pos2> = tempo_curve(base, pts, t1).into_iter().map(|(t, b)| pos2(x_of(t as f64), y_of(b))).collect();
    p.add(egui::Shape::line(line, Stroke::new(1.5_f32, TEMPO_COLOR)));
    p.text(pos2(lane.left() + 3.0, lane.center().y), Align2::LEFT_CENTER, format!("♩ {base:.1}"), FontId::proportional(10.0), TEMPO_COLOR.gamma_multiply(0.8));
    // time-signature changes: tag at the bar start (bottom of the lane)
    const METER_COLOR: Color32 = Color32::from_rgb(200, 170, 255);
    let bars = app.project.bar_map();
    for (k, s) in bars.segments().iter().enumerate() {
        if k == 0 && s.unit == 4 {
            continue;
        }
        let x = x_of(s.tick as f64);
        if x < lane.left() - 30.0 || x > lane.right() {
            continue;
        }
        p.line_segment([pos2(x, lane.top()), pos2(x, lane.bottom())], Stroke::new(1.0_f32, METER_COLOR.gamma_multiply(0.7)));
        p.text(pos2(x + 2.0, lane.bottom() - 1.0), Align2::LEFT_BOTTOM, format!("{}/{}", s.beats, s.unit), FontId::proportional(9.5), METER_COLOR);
    }
    for (i, q) in pts.iter().enumerate() {
        let x = x_of(q.tick as f64);
        if x < lane.left() - 40.0 || x > lane.right() + 4.0 {
            continue;
        }
        let c = pos2(x, y_of(q.bpm));
        let active = matches!(drag, Some((d, ..)) if d == i);
        let r = if active { 5.0 } else { 3.5 };
        p.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)], if active { Color32::WHITE } else { TEMPO_COLOR }, Stroke::NONE));
        p.text(pos2(x + 5.0, lane.top() + 1.0), Align2::LEFT_TOP, format!("{:.1}{}", q.bpm, if q.ramp { "↗" } else { "" }), FontId::proportional(9.5), Color32::from_gray(210));
    }
}

/// Time-signature section of the lane menu: set / remove the meter from the bar nearest to `tick`.
fn meter_menu(app: &mut TStudio, ui: &mut Ui, key: egui::Id, tick: i64) {
    let bars = app.project.bar_map();
    let bar = bars.tick_to_bar(tick as f64).round().max(0.0) as u32;
    let cur = bars.meter_at(bars.bar_to_tick(bar as f64));
    ui.separator();
    ui.label(RichText::new(format!("Taktart ab Takt {}", bar + 1)).strong());
    let (mut n, mut d): (u32, u32) = ui.data(|x| x.get_temp(key)).unwrap_or((cur.beats, cur.unit));
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut n).range(1..=64));
        ui.label("/");
        egui::ComboBox::from_id_salt(key.with("unit")).selected_text(d.to_string()).width(48.0).show_ui(ui, |ui| {
            for u in [1u32, 2, 4, 8, 16, 32] {
                ui.selectable_value(&mut d, u, u.to_string());
            }
        });
    });
    ui.data_mut(|x| x.insert_temp(key, (n, d)));
    if ui.button(format!("Taktart {n}/{d} setzen")).on_hover_text("gilt bis zum nächsten Taktartwechsel; Clips bleiben an ihrer Beat-Position").clicked() {
        app.project.set_meter(bar, n, d);
        ui.close_menu();
    }
    if app.project.meters.iter().any(|m| m.bar == bar) && ui.button("Taktartwechsel entfernen").clicked() {
        app.project.meters.retain(|m| m.bar != bar);
        ui.close_menu();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_has_steps_for_jumps_and_lines_for_ramps() {
        let pts = [TempoPoint { tick: 100, bpm: 150.0, ramp: false }, TempoPoint { tick: 200, bpm: 170.0, ramp: true }];
        let c = tempo_curve(140.0, &pts, 300);
        assert_eq!(c, vec![(0, 140.0), (100, 140.0), (100, 150.0), (200, 170.0), (300, 170.0)]);
        let (lo, hi) = lane_range(140.0, &pts);
        assert!(lo < 140.0 && hi > 170.0);
    }
}
