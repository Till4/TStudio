//! Automation clips in the playlist: curve drawing, creation from audio clips, property panel.

use super::*;

pub(super) const AUTO_COLOR: Color32 = Color32::from_rgb(235, 130, 200);

/// Polyline of an automation curve (flat before the first and after the last point).
pub(super) fn curve_points(points: &[AutoPoint], len: i64, range: (f32, f32, bool), x_of: impl Fn(i64) -> f32, body: Rect) -> Vec<Pos2> {
    let y_of = |v: f32| body.bottom() - normalize(v, range) * body.height();
    let mut out = Vec::new();
    let Some(first) = points.first() else { return out };
    out.push(pos2(x_of(0), y_of(first.v)));
    let log = range.2;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a.t >= len {
            break;
        }
        out.push(pos2(x_of(a.t.max(0)), y_of(a.v)));
        // log curves are drawn with a few intermediate points
        if log && b.t > a.t {
            for k in 1..8 {
                let t = a.t + (b.t - a.t) * k / 8;
                if t >= len {
                    break;
                }
                let v = tstudio_project::automation::value_at(&[(a.t, a.v), (b.t, b.v)], t as f64, true).unwrap_or(a.v);
                out.push(pos2(x_of(t), y_of(v)));
            }
        }
    }
    let last = points.iter().rev().find(|p| p.t <= len).copied().unwrap_or(*first);
    out.push(pos2(x_of(last.t.clamp(0, len)), y_of(last.v)));
    let end_v = tstudio_project::automation::value_at(&points.iter().map(|p| (p.t, p.v)).collect::<Vec<_>>(), len as f64, log).unwrap_or(last.v);
    out.push(pos2(x_of(len), y_of(end_v)));
    out
}

/// "Automation erstellen" from a parameter's right-click menu (mixer / TSynth):
/// new clip at the playhead, then the playlist opens with it selected.
pub fn create_automation(app: &mut TStudio, path: &str) {
    if app.project.parse_auto_target(path).is_err() {
        return;
    }
    add_automation_clip(app, path);
    app.view = View::Playlist;
}

/// Right-click menu entry for an automatable control; sets `req` to the target path when chosen.
pub fn automation_menu_item(ui: &mut Ui, path: String, req: &mut Option<String>) {
    if ui.button("〰 Automation erstellen").on_hover_text(format!("Automation-Clip für {path} am Playhead anlegen")).clicked() {
        *req = Some(path);
        ui.close_menu();
    }
}

/// New automation clip at the playhead (4 bars, flat at the current value).
pub(super) fn add_automation_clip(app: &mut TStudio, path: &str) {
    let Ok(target) = app.project.parse_auto_target(path) else { return };
    let bar = app.project.ticks_per_bar();
    let start = floor_to(app.position() as i64, bar).max(0);
    let len = bar * 4;
    let v = app.project.auto_base_value(target);
    let track = app.playlist_ui.first_free_track(&app.project, start);
    let clip = AutomationClip { target: path.to_string(), points: vec![AutoPoint { t: 0, v }, AutoPoint { t: len, v }] };
    let id = app.project.add_clip(track, start, len, ClipKind::Automation(clip));
    app.playlist_ui.selected = [id].into_iter().collect();
    app.song_mode = true;
    app.set_status("Automation-Clip angelegt – Kurve direkt im Clip zeichnen (Klick: Punkt, Ziehen: verschieben, Rechtsklick: löschen, Alt: ohne Raster)");
}

/// Gives an audio clip its own mixer insert (if it still plays on the master) and lays an automation
/// clip of the same position/length on a free track below it. `kind`: volume | filter | pan.
pub(super) fn automate_audio_clip(app: &mut TStudio, id: Id, kind: &str) {
    let Some(idx) = app.project.playlist.clips.iter().position(|c| c.id == id) else { return };
    let (start, len, track) = {
        let c = &app.project.playlist.clips[idx];
        (c.start, c.len, c.track)
    };
    let ClipKind::Audio(a) = &app.project.playlist.clips[idx].kind else { return };
    let mut mt = a.mixer_track;
    if mt == 0 || mt >= app.project.mixer.len() {
        let stem = a.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sample".into());
        mt = app.project.add_mixer_track(Some(stem));
        if let ClipKind::Audio(a) = &mut app.project.playlist.clips[idx].kind {
            a.mixer_track = mt;
        }
    }
    let (path, points) = match kind {
        "filter" => {
            let fx = &mut app.project.mixer[mt].fx;
            let slot = match fx.iter().position(|s| matches!(s.params, tstudio_dsp::fx::EffectParams::Filter(_))) {
                Some(s) => s,
                None => {
                    let mut s = tstudio_project::FxSlot::new(tstudio_dsp::fx::EffectKind::Filter);
                    if let tstudio_dsp::fx::EffectParams::Filter(f) = &mut s.params {
                        f.cutoff = 20000.0;
                    }
                    fx.push(s);
                    fx.len() - 1
                }
            };
            (format!("mixer.{mt}.fx.{slot}.cutoff"), vec![AutoPoint { t: 0, v: 300.0 }, AutoPoint { t: len, v: 20000.0 }])
        }
        "pan" => (format!("mixer.{mt}.pan"), vec![AutoPoint { t: 0, v: -0.8 }, AutoPoint { t: len / 2, v: 0.8 }, AutoPoint { t: len, v: -0.8 }]),
        _ => {
            let v = app.project.mixer[mt].volume;
            (format!("mixer.{mt}.volume"), vec![AutoPoint { t: 0, v: 0.0 }, AutoPoint { t: len, v }])
        }
    };
    // first track below the sample that is free for the whole clip range
    let end = start + len;
    let free = |t: usize, p: &Project| !p.playlist.clips.iter().any(|c| c.track == t && c.start < end && c.end() > start);
    let auto_track = (track + 1..track + 200).find(|t| free(*t, &app.project)).unwrap_or(track + 1);
    while app.project.playlist.tracks.len() <= auto_track {
        app.project.add_playlist_track(None);
    }
    let new = app.project.add_clip(auto_track, start, len, ClipKind::Automation(AutomationClip { target: path, points }));
    app.playlist_ui.selected = [new].into_iter().collect();
    app.song_mode = true;
    let name = app.project.mixer[mt].name.clone();
    app.set_status(format!("Automation angelegt (Mixer-Insert {mt} „{name}“) – Kurve unten bearbeiten; Clips zusammen länger ziehen zum Wiederholen"));
}

pub(super) fn automation_properties(app: &mut TStudio, ui: &mut Ui, id: Id) {
    let Some(idx) = app.project.playlist.clips.iter().position(|c| c.id == id) else { return };
    let targets = app.project.automation_targets();
    let clip_len = app.project.playlist.clips[idx].len;
    let ClipKind::Automation(a) = &app.project.playlist.clips[idx].kind else { return };
    let label = app.project.auto_label(&a.target);
    let mut new_target: Option<String> = None;
    let mut shape: Option<&str> = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("〰 Automation").strong().color(AUTO_COLOR));
        egui::ComboBox::from_id_salt("auto_target").selected_text(label).width(260.0).show_ui(ui, |ui| {
            for (path, l) in &targets {
                if ui.selectable_label(*path == a.target, l).clicked() {
                    new_target = Some(path.clone());
                }
            }
        });
        ui.separator();
        for (s, tip) in [("⟋", "Rampe hoch"), ("⟍", "Rampe runter"), ("▬", "Flach (aktueller Wert)"), ("∧", "Dreieck"), ("⩘ 1/4", "Sägezahn je Beat (Pump-Effekt)")] {
            if ui.button(s).on_hover_text(tip).clicked() {
                shape = Some(s);
            }
        }
        ui.weak("Klick: Punkt setzen · Ziehen: verschieben · Rechtsklick: Punkt löschen");
    });
    if let Some(t) = new_target {
        if let Ok(tg) = app.project.parse_auto_target(&t) {
            let v = app.project.auto_base_value(tg);
            if let ClipKind::Automation(a) = &mut app.project.playlist.clips[idx].kind {
                a.target = t;
                a.points = vec![AutoPoint { t: 0, v }, AutoPoint { t: clip_len, v }];
            }
        }
    }
    let base = match &app.project.playlist.clips[idx].kind {
        ClipKind::Automation(a) => app.project.parse_auto_target(&a.target).map(|t| app.project.auto_base_value(t)).unwrap_or(0.0),
        _ => return,
    };
    let ClipKind::Automation(a) = &mut app.project.playlist.clips[idx].kind else { return };
    let range = field_range(Project::auto_field_name(&a.target));
    let (lo, hi, _) = range;
    if let Some(s) = shape {
        a.points = match s {
            "⟋" => vec![AutoPoint { t: 0, v: lo }, AutoPoint { t: clip_len, v: hi }],
            "⟍" => vec![AutoPoint { t: 0, v: hi }, AutoPoint { t: clip_len, v: lo }],
            "▬" => vec![AutoPoint { t: 0, v: base }, AutoPoint { t: clip_len, v: base }],
            "∧" => vec![AutoPoint { t: 0, v: lo }, AutoPoint { t: clip_len / 2, v: hi }, AutoPoint { t: clip_len, v: lo }],
            _ => {
                let mut v = Vec::new();
                let mut t = 0;
                while t < clip_len {
                    v.push(AutoPoint { t, v: hi });
                    v.push(AutoPoint { t: (t + PPQ - 1).min(clip_len), v: lo });
                    t += PPQ;
                }
                v
            }
        };
    }

    // curve editor
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, BG_DARK);
    let body = rect.shrink2(vec2(6.0, 8.0));
    let x_of = |t: i64| body.left() + (t as f32 / clip_len.max(1) as f32) * body.width();
    let t_of = |x: f32| (((x - body.left()) / body.width()).clamp(0.0, 1.0) * clip_len as f32) as i64;
    let y_of = |v: f32| body.bottom() - normalize(v, range) * body.height();
    let v_of = |y: f32| denormalize((body.bottom() - y) / body.height(), range);
    // beat grid
    let mut t = 0;
    while t <= clip_len {
        let strong = t % (PPQ * 4) == 0;
        p.line_segment([pos2(x_of(t), body.top()), pos2(x_of(t), body.bottom())], Stroke::new(1.0, if strong { Color32::from_gray(70) } else { Color32::from_gray(45) }));
        t += PPQ;
    }
    p.add(egui::Shape::line(curve_points(&a.points, clip_len, range, x_of, body), Stroke::new(2.0, AUTO_COLOR)));
    let drag_id = ui.id().with(("auto_drag", id));
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id)).flatten();
    let near = |pt: Pos2, pts: &[AutoPoint]| pts.iter().position(|q| pos2(x_of(q.t), y_of(q.v)).distance(pt) < 7.0);
    let step = PPQ / 8;
    if resp.drag_started() || resp.clicked() {
        if let Some(pt) = resp.interact_pointer_pos() {
            dragging = match near(pt, &a.points) {
                Some(i) => Some(i),
                None => {
                    let t = (t_of(pt.x) + step / 2) / step * step;
                    a.points.push(AutoPoint { t, v: v_of(pt.y) });
                    a.points.sort_by_key(|q| q.t);
                    a.points.iter().position(|q| q.t == t)
                }
            };
        }
    }
    if resp.dragged() {
        if let (Some(i), Some(pt)) = (dragging, resp.interact_pointer_pos()) {
            if i < a.points.len() {
                let t = (t_of(pt.x) + step / 2) / step * step;
                let v = v_of(pt.y);
                a.points[i] = AutoPoint { t, v };
                // keep order while dragging past neighbours
                let moved = a.points[i];
                a.points.sort_by_key(|q| q.t);
                dragging = a.points.iter().position(|q| *q == moved);
            }
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        dragging = None;
    }
    if resp.secondary_clicked() && a.points.len() > 1 {
        if let Some(pt) = resp.interact_pointer_pos() {
            if let Some(i) = near(pt, &a.points) {
                a.points.remove(i);
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(drag_id, dragging));
    for (i, q) in a.points.iter().enumerate() {
        let c = pos2(x_of(q.t.min(clip_len)), y_of(q.v));
        p.circle_filled(c, if dragging == Some(i) { 5.5 } else { 4.0 }, Color32::WHITE);
    }
    let fmt = |v: f32| if range.2 { widgets::fmt_hz(v) } else { format!("{v:.2}") };
    p.text(rect.left_top() + vec2(8.0, 4.0), Align2::LEFT_TOP, fmt(hi), FontId::proportional(10.0), TEXT_DIM);
    p.text(rect.left_bottom() + vec2(8.0, -4.0), Align2::LEFT_BOTTOM, fmt(lo), FontId::proportional(10.0), TEXT_DIM);
    if let Some(i) = dragging {
        if let Some(q) = a.points.get(i) {
            p.text(rect.right_top() + vec2(-8.0, 4.0), Align2::RIGHT_TOP, format!("Beat {:.2} · {}", q.t as f32 / PPQ as f32, fmt(q.v)), FontId::proportional(11.0), Color32::WHITE);
        }
    } else if let Some(hp) = resp.hover_pos() {
        p.text(rect.right_top() + vec2(-8.0, 4.0), Align2::RIGHT_TOP, fmt(v_of(hp.y)), FontId::proportional(11.0), TEXT_DIM);
    }
}
