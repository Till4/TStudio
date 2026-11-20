use crate::app::{TStudio, View};
use crate::ui::widgets::{self, floor_to, rgb, round_to, SampleDrag, Snap, ACCENT, BG_DARK, BG_LIGHT, GRID_BAR, GRID_BEAT, PLAYHEAD, TEXT_DIM};
use egui::{pos2, vec2, Align2, Color32, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Ui};
use std::collections::HashSet;
use tstudio_dsp::stretch::StretchMode;
use tstudio_engine::Command;
use tstudio_project::automation::{denormalize, field_range, normalize};
use tstudio_project::{AutoPoint, AutomationClip, ClipKind, Id, Project, PPQ};

mod menus;
mod automation;
mod properties;
mod tempo;

use menus::*;
use automation::*;
use properties::*;
use tempo::*;

enum PDrag {
    Move { anchor_tick: f64, anchor_track: i64, orig: Vec<(Id, i64, usize)> },
    ResizeR { anchor_tick: f64, orig: Vec<(Id, i64)> },
    ResizeL { anchor_tick: f64, orig: Vec<(Id, i64, i64, i64)> },
    Loop { from: i64 },
}

pub struct PlaylistState {
    pub px_per_beat: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub track_h: f32,
    pub snap: Snap,
    pub selected: HashSet<Id>,
    drag: Option<PDrag>,
    /// playlist track whose header menu is open
    pub menu_track: Option<usize>,
    /// (track, tick) of a right click on an empty spot of the grid
    empty_menu: Option<(usize, i64)>,
    /// Strg+C / Strg+X: copied clips (pattern clips keep pointing at their pattern)
    clipboard: Vec<tstudio_project::Clip>,
}

impl Default for PlaylistState {
    fn default() -> Self {
        Self { px_per_beat: 26.0, scroll_x: 0.0, scroll_y: 0.0, track_h: 48.0, snap: Snap::Beat, selected: HashSet::new(), drag: None, menu_track: None, empty_menu: None, clipboard: Vec::new() }
    }
}

impl PlaylistState {
    /// First track with no clip overlapping one bar from `start`.
    pub fn first_free_track(&self, p: &Project, start: i64) -> usize {
        let end = start + p.ticks_per_bar();
        (0..p.playlist.tracks.len().max(1) + 1)
            .find(|t| !p.playlist.clips.iter().any(|c| c.track == *t && c.start < end && c.end() > start))
            .unwrap_or(0)
    }
}

fn clip_duration_secs(app: &mut TStudio, path: &std::path::Path) -> Option<f64> {
    app.library.analysis.get(path).map(|a| a.duration as f64).or_else(|| app.cache.get_raw(path).map(|d| d.duration_secs()))
}

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    let bar = app.project.ticks_per_bar();
    // ---------------------------------------------------------------- toolbar
    ui.horizontal(|ui| {
        ui.heading("Playlist");
        widgets::snap_combo(ui, "pl_snap", &mut app.playlist_ui.snap);
        if ui.button("−").on_hover_text("Rauszoomen (Strg+Mausrad)").clicked() {
            app.playlist_ui.px_per_beat = (app.playlist_ui.px_per_beat / 1.25).max(3.0);
        }
        if ui.button("+").on_hover_text("Reinzoomen").clicked() {
            app.playlist_ui.px_per_beat = (app.playlist_ui.px_per_beat * 1.25).min(400.0);
        }
        ui.checkbox(&mut app.settings.follow_playhead, "Folgen");
        ui.menu_button("+ Automation", |ui| {
            ui.label(RichText::new("Parameter automatisieren (wirkt im Song-Modus):").small());
            let targets = app.project.automation_targets();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (path, label) in targets {
                    if ui.button(&label).on_hover_text(&path).clicked() {
                        add_automation_clip(app, &path);
                        ui.close_menu();
                    }
                }
            });
        })
        .response
        .on_hover_text("Automation-Clip am Playhead anlegen (Filter-Sweeps, Lautstärke-Fahrten …)");
        let looping = app.project.playlist.loop_region.is_some();
        let loop_txt = if looping { RichText::new("⟲ Loop an").color(ACCENT).strong() } else { RichText::new("⟲ Loop") };
        if ui
            .button(loop_txt)
            .on_hover_text("Loop-Bereich an/aus (Strg+L): um die markierten Clips, sonst 4 Takte ab Playhead.\nIm Lineal ziehen = Bereich setzen, Ränder ziehen = verschieben, Rechtsklick = aus.")
            .clicked()
        {
            toggle_loop(app);
        }
        let pname = app.project.pattern(app.current_pattern).map(|p| p.name.clone()).unwrap_or_default();
        ui.weak(format!("Klick in leere Fläche malt „{pname}“ · Im Lineal ziehen = Loop · Strg+C/V kopiert Clips · Samples aus der Library hierher ziehen"));
    });

    let selected_audio: Option<Id> = if app.playlist_ui.selected.len() == 1 {
        let id = *app.playlist_ui.selected.iter().next().unwrap();
        app.project.playlist.clips.iter().find(|c| c.id == id && matches!(c.kind, ClipKind::Audio(_))).map(|c| c.id)
    } else {
        None
    };
    if let Some(id) = selected_audio {
        egui::TopBottomPanel::bottom("clip_props").resizable(false).show_inside(ui, |ui| clip_properties(app, ui, id));
    }
    let selected_auto: Option<Id> = if app.playlist_ui.selected.len() == 1 {
        let id = *app.playlist_ui.selected.iter().next().unwrap();
        app.project.playlist.clips.iter().find(|c| c.id == id && matches!(c.kind, ClipKind::Automation(_))).map(|c| c.id)
    } else {
        None
    };
    if let Some(id) = selected_auto {
        egui::TopBottomPanel::bottom("auto_props").resizable(false).show_inside(ui, |ui| automation_properties(app, ui, id));
    }
    let selected_pat: Option<Id> = if app.playlist_ui.selected.len() == 1 {
        let id = *app.playlist_ui.selected.iter().next().unwrap();
        app.project.playlist.clips.iter().find(|c| c.id == id && matches!(c.kind, ClipKind::Pattern { .. })).map(|c| c.id)
    } else {
        None
    };
    if let Some(id) = selected_pat {
        egui::TopBottomPanel::bottom("pat_props").resizable(false).show_inside(ui, |ui| pattern_clip_properties(app, ui, id));
    }

    // ---------------------------------------------------------------- layout
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let head_w = 130.0;
    let ruler_h = 22.0;
    let ruler = Rect::from_min_max(pos2(full.left() + head_w, full.top()), pos2(full.right(), full.top() + ruler_h));
    // tempo track under the ruler
    let lane = Rect::from_min_max(pos2(full.left() + head_w, ruler.bottom()), pos2(full.right(), ruler.bottom() + TEMPO_LANE_H));
    let heads = Rect::from_min_max(pos2(full.left(), lane.bottom()), pos2(full.left() + head_w, full.bottom()));
    let grid = Rect::from_min_max(pos2(full.left() + head_w, lane.bottom()), full.max);

    let n_tracks = app.project.playlist.tracks.len();
    {
        let st = &mut app.playlist_ui;
        if ui.rect_contains_pointer(full) {
            let (scroll, zoom, mods, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers, i.pointer.hover_pos()));
            if zoom != 1.0 {
                let mx = hover.map(|p| p.x).unwrap_or(grid.left());
                let tick_at = st.scroll_x + (mx - grid.left()) / st.px_per_beat * PPQ as f32;
                st.px_per_beat = (st.px_per_beat * zoom).clamp(3.0, 400.0);
                st.scroll_x = (tick_at - (mx - grid.left()) / st.px_per_beat * PPQ as f32).max(0.0);
            } else if mods.shift {
                st.scroll_x = (st.scroll_x - (scroll.y + scroll.x) / st.px_per_beat * PPQ as f32).max(0.0);
            } else {
                st.scroll_y = (st.scroll_y - scroll.y).clamp(0.0, (n_tracks as f32 * st.track_h - grid.height() + 40.0).max(0.0));
                st.scroll_x = (st.scroll_x - scroll.x / st.px_per_beat * PPQ as f32).max(0.0);
            }
        }
        // follow playhead
        if app.settings.follow_playhead && app.engine.shared.is_playing() && app.song_mode && st.drag.is_none() {
            let pos = app.engine.shared.position() as f32;
            let visible = grid.width() / st.px_per_beat * PPQ as f32;
            if pos < st.scroll_x || pos > st.scroll_x + visible * 0.9 {
                st.scroll_x = (pos - visible * 0.1).max(0.0);
            }
        }
    }
    let (ppb, sx, sy, th) = (app.playlist_ui.px_per_beat, app.playlist_ui.scroll_x, app.playlist_ui.scroll_y, app.playlist_ui.track_h);
    let x_of = move |t: f64| grid.left() + ((t as f32 - sx) / PPQ as f32) * ppb;
    let tick_of = move |x: f32| (sx + (x - grid.left()) / ppb * PPQ as f32) as f64;
    let track_of = move |y: f32| ((y - grid.top() + sy) / th).floor() as i64;
    let track_top = move |t: usize| grid.top() + t as f32 * th - sy;
    let snap_t = app.playlist_ui.snap.ticks(bar);

    let clip_rect = |c: &tstudio_project::Clip| Rect::from_min_max(pos2(x_of(c.start as f64), track_top(c.track) + 1.0), pos2(x_of(c.end() as f64), track_top(c.track) + th - 1.0));
    let hit = |p: &Project, pt: Pos2| -> Option<(Id, i8)> {
        for c in p.playlist.clips.iter().rev() {
            let r = clip_rect(c);
            if r.contains(pt) {
                let edge = (r.width() * 0.2).clamp(3.0, 8.0);
                let side = if pt.x > r.right() - edge {
                    1
                } else if pt.x < r.left() + edge {
                    -1
                } else {
                    0
                };
                return Some((c.id, side));
            }
        }
        None
    };

    // ---------------------------------------------------------------- interaction
    let grid_resp = ui.interact(grid, ui.id().with("pl_grid"), Sense::click_and_drag());
    let (pointer, pressed, down, released, sec_down, mods) =
        ui.input(|i| (i.pointer.interact_pos().or(i.pointer.hover_pos()), i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.secondary_down(), i.modifiers));
    let grid_top = ui.rect_contains_pointer(grid);
    let mut open_pattern: Option<Id> = None;

    if let Some(pt) = pointer {
        if grid_top && app.playlist_ui.drag.is_none() {
            if let Some((_, side)) = hit(&app.project, pt) {
                if side != 0 {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
    }
    if grid_resp.double_clicked() {
        if let Some(pt) = pointer {
            if let Some((id, _)) = hit(&app.project, pt) {
                if let Some(ClipKind::Pattern { pattern }) = app.project.playlist.clips.iter().find(|c| c.id == id).map(|c| c.kind.clone()) {
                    open_pattern = Some(pattern);
                }
            }
        }
    }
    if pressed && grid_top {
        if let Some(pt) = pointer {
            let t = tick_of(pt.x);
            let tr = track_of(pt.y).max(0);
            let st = &mut app.playlist_ui;
            if let Some((id, side)) = hit(&app.project, pt) {
                if !st.selected.contains(&id) {
                    if !mods.shift {
                        st.selected.clear();
                    }
                    st.selected.insert(id);
                }
                // Alt + drag = drag copies (originals stay)
                if side == 0 && mods.alt {
                    let originals: Vec<tstudio_project::Clip> = app.project.playlist.clips.iter().filter(|c| st.selected.contains(&c.id)).cloned().collect();
                    st.selected.clear();
                    for c in originals {
                        let nid = app.project.add_clip(c.track, c.start, c.len, c.kind.clone());
                        if let Some(n) = app.project.playlist.clips.iter_mut().find(|x| x.id == nid) {
                            n.muted = c.muted;
                        }
                        st.selected.insert(nid);
                    }
                }
                let sel: Vec<&tstudio_project::Clip> = app.project.playlist.clips.iter().filter(|c| st.selected.contains(&c.id)).collect();
                st.drag = Some(match side {
                    1 => PDrag::ResizeR { anchor_tick: t, orig: sel.iter().map(|c| (c.id, c.len)).collect() },
                    -1 => PDrag::ResizeL {
                        anchor_tick: t,
                        orig: sel.iter().map(|c| (c.id, c.start, c.len, if let ClipKind::Audio(a) = &c.kind { a.offset } else { 0 })).collect(),
                    },
                    _ => PDrag::Move { anchor_tick: t, anchor_track: tr, orig: sel.iter().map(|c| (c.id, c.start, c.track)).collect() },
                });
            } else {
                st.selected.clear();
                let start = floor_to(t as i64, snap_t).max(0);
                let pid = app.current_pattern;
                let len = app.project.pattern_length(pid);
                let id = app.project.add_clip(tr as usize, start, len, ClipKind::Pattern { pattern: pid });
                let st = &mut app.playlist_ui;
                st.selected.insert(id);
                st.drag = Some(PDrag::Move { anchor_tick: t, anchor_track: tr, orig: vec![(id, start, tr as usize)] });
                app.song_mode = true;
            }
        }
    }
    if down {
        if let Some(pt) = pointer {
            let t = tick_of(pt.x);
            let st = &mut app.playlist_ui;
            match st.drag.as_ref() {
                Some(PDrag::Move { anchor_tick, anchor_track, orig }) => {
                    let min_start = orig.iter().map(|o| o.1).min().unwrap_or(0);
                    let min_track = orig.iter().map(|o| o.2).min().unwrap_or(0) as i64;
                    let dt = round_to((t - anchor_tick) as i64, snap_t).max(-min_start);
                    let dtr = (track_of(pt.y) - anchor_track).max(-min_track);
                    for (id, s, tr) in orig {
                        if let Some(c) = app.project.playlist.clips.iter_mut().find(|c| c.id == *id) {
                            c.start = s + dt;
                            c.track = (*tr as i64 + dtr) as usize;
                        }
                    }
                    let max_track = app.project.playlist.clips.iter().map(|c| c.track).max().unwrap_or(0);
                    while app.project.playlist.tracks.len() <= max_track {
                        let n = app.project.playlist.tracks.len() + 1;
                        app.project.playlist.tracks.push(tstudio_project::PlaylistTrack { name: format!("Track {n}"), ..Default::default() });
                    }
                }
                Some(PDrag::ResizeR { anchor_tick, orig }) => {
                    let dt = round_to((t - anchor_tick) as i64, snap_t);
                    for (id, len) in orig {
                        if let Some(c) = app.project.playlist.clips.iter_mut().find(|c| c.id == *id) {
                            c.len = (len + dt).max(snap_t.max(PPQ / 4));
                        }
                    }
                }
                Some(PDrag::ResizeL { anchor_tick, orig }) => {
                    let dt = round_to((t - anchor_tick) as i64, snap_t);
                    for (id, start, len, off) in orig {
                        let min_len = snap_t.max(PPQ / 4);
                        let d = dt.max(-start).min(len - min_len);
                        if let Some(c) = app.project.playlist.clips.iter_mut().find(|c| c.id == *id) {
                            let d = if let ClipKind::Audio(_) = c.kind { d.max(-off) } else { d };
                            c.start = start + d;
                            c.len = len - d;
                            if let ClipKind::Audio(a) = &mut c.kind {
                                a.offset = off + d;
                            }
                        }
                    }
                }
                Some(PDrag::Loop { from }) => {
                    let to = round_to(t as i64, snap_t.max(PPQ)).max(0);
                    let (a, b) = if to >= *from { (*from, to) } else { (to, *from) };
                    app.project.playlist.loop_region = if b > a { Some((a, b)) } else { None };
                }
                None => {}
            }
        }
    }
    if released {
        app.playlist_ui.drag = None;
    }
    // right click on an empty spot: menu for a new pattern with an instrument
    let sec_pressed = ui.input(|i| i.pointer.secondary_pressed());
    if sec_pressed && grid_top && app.playlist_ui.drag.is_none() {
        app.playlist_ui.empty_menu = pointer
            .filter(|pt| hit(&app.project, *pt).is_none())
            .map(|pt| (track_of(pt.y).max(0) as usize, floor_to(tick_of(pt.x) as i64, bar).max(0)));
    }
    if let Some((tr, at)) = app.playlist_ui.empty_menu {
        let shown = grid_resp.context_menu(|ui| empty_spot_menu(app, ui, tr, at)).is_some();
        if !shown && !sec_pressed && !grid_resp.secondary_clicked() {
            app.playlist_ui.empty_menu = None;
        }
    }
    if sec_down && grid_top && app.playlist_ui.drag.is_none() {
        if let Some(pt) = pointer {
            if let Some((id, _)) = hit(&app.project, pt) {
                app.project.playlist.clips.retain(|c| c.id != id);
                app.playlist_ui.selected.remove(&id);
            }
        }
    }
    // drops from the library
    if let Some(drop) = grid_resp.dnd_release_payload::<SampleDrag>() {
        if tstudio_library::is_soundfont_file(&drop.path) {
            app.add_soundfont_channel(&drop.path);
        } else if let Some(pt) = pointer {
            let start = floor_to(tick_of(pt.x) as i64, snap_t).max(0);
            let tr = track_of(pt.y).max(0) as usize;
            let id = app.add_audio_clip(&drop.path, tr, start);
            app.playlist_ui.selected = [id].into_iter().collect();
            app.song_mode = true;
        }
    }
    // keys
    if ui.rect_contains_pointer(full) && !ui.ctx().wants_keyboard_input() {
        let (del, dup) = ui.input(|i| (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace), i.modifiers.command && i.key_pressed(Key::D)));
        if del {
            let sel = std::mem::take(&mut app.playlist_ui.selected);
            app.project.playlist.clips.retain(|c| !sel.contains(&c.id));
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(Key::L)) {
            toggle_loop(app);
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(Key::A)) {
            app.playlist_ui.selected = app.project.playlist.clips.iter().map(|c| c.id).collect();
        }
        let (copy, cut, paste) = widgets::edit_keys(ui);
        if (copy || cut) && !app.playlist_ui.selected.is_empty() {
            let sel = app.playlist_ui.selected.clone();
            app.playlist_ui.clipboard = app.project.playlist.clips.iter().filter(|c| sel.contains(&c.id)).cloned().collect();
            widgets::mark_copied(ui, "Clips");
            let n = app.playlist_ui.clipboard.len();
            if cut {
                app.project.playlist.clips.retain(|c| !sel.contains(&c.id));
                app.playlist_ui.selected.clear();
                app.set_status(format!("{n} Clip(s) ausgeschnitten – Strg+V fügt an der Maus (oder am Playhead) ein"));
            } else {
                app.set_status(format!("{n} Clip(s) kopiert – Strg+V fügt an der Maus (oder am Playhead) ein"));
            }
        }
        if paste && !app.playlist_ui.clipboard.is_empty() {
            let clips = app.playlist_ui.clipboard.clone();
            let min_start = clips.iter().map(|c| c.start).min().unwrap_or(0);
            let min_track = clips.iter().map(|c| c.track).min().unwrap_or(0);
            let (at, at_track) = match pointer.filter(|pt| grid.contains(*pt)) {
                Some(pt) => (floor_to(tick_of(pt.x) as i64, snap_t).max(0), track_of(pt.y).max(0) as usize),
                None => (floor_to(app.position() as i64, bar).max(0), min_track),
            };
            app.playlist_ui.selected.clear();
            for c in clips {
                let track = c.track - min_track + at_track;
                while app.project.playlist.tracks.len() <= track {
                    app.project.add_playlist_track(None);
                }
                let id = app.project.add_clip(track, c.start - min_start + at, c.len, c.kind.clone());
                if let Some(n) = app.project.playlist.clips.iter_mut().find(|x| x.id == id) {
                    n.muted = c.muted;
                }
                app.playlist_ui.selected.insert(id);
            }
            app.song_mode = true;
        }
        if dup && !app.playlist_ui.selected.is_empty() {
            let sel: Vec<tstudio_project::Clip> = app.project.playlist.clips.iter().filter(|c| app.playlist_ui.selected.contains(&c.id)).cloned().collect();
            let min = sel.iter().map(|c| c.start).min().unwrap_or(0);
            let max = sel.iter().map(|c| c.end()).max().unwrap_or(0);
            let span = (max - min + bar - 1) / bar * bar;
            app.playlist_ui.selected.clear();
            for c in sel {
                let id = app.project.add_clip(c.track, c.start + span, c.len, c.kind.clone());
                app.playlist_ui.selected.insert(id);
            }
        }
    }
    // ruler
    // ruler: click = playhead, drag = loop region, drag a loop edge = move it, right click = loop off
    let ruler_resp = ui.interact(ruler, ui.id().with("pl_ruler"), Sense::click_and_drag());
    let loop_snap = snap_t.max(PPQ);
    let region = app.project.playlist.loop_region;
    // the opposite edge when x is on a loop edge
    let edge_hit = move |x: f32| {
        region.and_then(|(a, b)| {
            if (x - x_of(b as f64)).abs() < 7.0 {
                Some(a)
            } else if (x - x_of(a as f64)).abs() < 7.0 {
                Some(b)
            } else {
                None
            }
        })
    };
    if let Some(hp) = ruler_resp.hover_pos() {
        if edge_hit(hp.x).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }
    if ruler_resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(ruler_resp.interact_pointer_pos());
        if let Some(o) = origin {
            let from = edge_hit(o.x).unwrap_or_else(|| round_to(tick_of(o.x) as i64, loop_snap).max(0));
            app.playlist_ui.drag = Some(PDrag::Loop { from });
        }
    } else if ruler_resp.is_pointer_button_down_on() && app.playlist_ui.drag.is_none() && !ruler_resp.dragged() {
        if let Some(pt) = ruler_resp.interact_pointer_pos() {
            if edge_hit(pt.x).is_none() {
                let t = floor_to(tick_of(pt.x) as i64, snap_t).max(0);
                app.song_mode = true;
                app.send(Command::SetPosition(t as f64));
            }
        }
    }
    let _ = mods;
    if ruler_resp.secondary_clicked() {
        app.project.playlist.loop_region = None;
    }
    if let Some(pid) = open_pattern {
        app.current_pattern = pid;
        app.view = View::PianoRoll;
        app.song_mode = false;
    }

    // track headers: LED/M = mute, S = solo (Ctrl = add to solos), right click = track menu
    let head_resp = ui.interact(heads, ui.id().with("pl_heads"), Sense::click());
    let head_btn = |t: usize, k: usize| {
        let y = track_top(t);
        Rect::from_center_size(pos2(heads.right() - 14.0 - k as f32 * 20.0, y + th * 0.5), vec2(17.0, 15.0))
    };
    if head_resp.clicked() {
        if let Some(pt) = head_resp.interact_pointer_pos() {
            let tr = track_of(pt.y);
            if tr >= 0 && (tr as usize) < n_tracks {
                let t = tr as usize;
                let ctrl = ui.input(|i| i.modifiers.command);
                if head_btn(t, 0).expand(2.0).contains(pt) {
                    app.project.toggle_playlist_solo(t, !ctrl);
                } else if head_btn(t, 1).expand(2.0).contains(pt) || pt.x < heads.left() + 24.0 {
                    let m = &mut app.project.playlist.tracks[t].muted;
                    *m = !*m;
                }
            } else if tr >= 0 && tr as usize == n_tracks {
                // "+ Spur" row below the last track
                app.project.add_playlist_track(None);
            }
        }
    }
    if head_resp.secondary_clicked() {
        if let Some(pt) = head_resp.interact_pointer_pos() {
            let tr = track_of(pt.y);
            app.playlist_ui.menu_track = if tr >= 0 && (tr as usize) < n_tracks { Some(tr as usize) } else { None };
        }
    }
    head_resp.context_menu(|ui| track_menu(app, ui));
    // the menu may have added or deleted tracks this frame
    let n_tracks = app.project.playlist.tracks.len();
    // volume bar in each header: drag = volume, double click = 100 %
    const VOL_MAX: f32 = 1.25;
    let vol_bar = |t: usize| {
        let y = track_top(t);
        Rect::from_min_max(pos2(heads.left() + 24.0, y + th - 11.0), pos2(heads.right() - 50.0, y + th - 6.0))
    };
    for t in 0..n_tracks {
        let r = vol_bar(t);
        if r.bottom() < heads.top() || r.top() > heads.bottom() {
            continue;
        }
        let resp = ui.interact(r.expand2(vec2(2.0, 3.0)).intersect(heads), ui.id().with(("tvol", t)), Sense::click_and_drag());
        let vol = &mut app.project.playlist.tracks[t].volume;
        if resp.double_clicked() {
            *vol = 1.0;
        } else if resp.dragged() || resp.clicked() {
            if let Some(pt) = resp.interact_pointer_pos() {
                let f = ((pt.x - r.left()) / r.width()).clamp(0.0, 1.0) * VOL_MAX;
                // snap to 100 % near the mark
                *vol = if (f - 1.0).abs() < 0.03 { 1.0 } else { f };
            }
        }
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            let db = tstudio_dsp::gain_to_db(*vol);
            resp.on_hover_text(format!("Spur-Lautstärke {:.0} % ({:+.1} dB) – ziehen, Doppelklick = 100 %", *vol * 100.0, db.max(-60.0)));
        }
    }

    // ---------------------------------------------------------------- painting
    let painter = ui.painter_at(full);
    painter.rect_filled(full, 0.0, BG_DARK);
    let gp = painter.with_clip_rect(grid);
    for t in 0..n_tracks {
        let y = track_top(t);
        if y > grid.bottom() || y + th < grid.top() {
            continue;
        }
        let r = Rect::from_min_max(pos2(grid.left(), y), pos2(grid.right(), y + th));
        gp.rect_filled(r, 0.0, if t % 2 == 0 { Color32::from_rgb(38, 42, 48) } else { Color32::from_rgb(34, 38, 43) });
    }
    let t0 = tick_of(grid.left()).max(0.0) as i64;
    let t1 = tick_of(grid.right()) as i64 + 1;
    let beat_px = ppb;
    let mut t = floor_to(t0, PPQ);
    while t <= t1 {
        let x = x_of(t as f64);
        if t % bar == 0 {
            gp.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0, GRID_BAR));
        } else if beat_px > 8.0 {
            gp.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0, GRID_BEAT));
        }
        t += PPQ;
    }
    if let Some((a, b)) = app.project.playlist.loop_region {
        gp.rect_filled(Rect::from_min_max(pos2(x_of(a as f64), grid.top()), pos2(x_of(b as f64), grid.bottom())), 0.0, ACCENT.gamma_multiply(0.06));
    }

    let sr = app.engine.sample_rate as f64;
    let clips = app.project.playlist.clips.clone();
    for c in &clips {
        let r = clip_rect(c);
        if !r.intersects(grid) {
            continue;
        }
        let sel = app.playlist_ui.selected.contains(&c.id);
        let track_muted = app.project.playlist.tracks.get(c.track).map(|t| t.muted).unwrap_or(false);
        let alpha = if c.muted || track_muted { 0.35 } else { 1.0 };
        match &c.kind {
            ClipKind::Pattern { pattern } => {
                let Some(p) = app.project.pattern(*pattern) else { continue };
                let col = rgb(p.color);
                gp.rect_filled(r, 3.0, col.gamma_multiply(0.35 * alpha));
                let head = Rect::from_min_max(r.min, pos2(r.right(), r.top() + 13.0));
                gp.rect_filled(head, 3.0, col.gamma_multiply(0.8 * alpha));
                gp.with_clip_rect(r.intersect(grid)).text(head.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, &p.name, FontId::proportional(10.5), Color32::BLACK);
                // note preview, repeated when the clip is longer than the pattern
                let plen = app.project.pattern_length(*pattern).max(1);
                let body = Rect::from_min_max(pos2(r.left(), head.bottom() + 2.0), pos2(r.right(), r.bottom() - 2.0));
                let reps = (c.len + plen - 1) / plen;
                for k in 1..reps {
                    let x = x_of((c.start + k * plen) as f64);
                    if x > grid.left() && x < grid.right() {
                        gp.line_segment([pos2(x, r.top() + 1.0), pos2(x, r.bottom() - 1.0)], Stroke::new(1.0, col.gamma_multiply(0.9 * alpha)));
                        gp.text(pos2(x + 3.0, head.center().y), Align2::LEFT_CENTER, "↻", FontId::proportional(10.0), Color32::BLACK.gamma_multiply(0.7));
                    }
                }
                if !p.notes.is_empty() {
                    let (kmin, kmax) = p.notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.key), b.max(n.key)));
                    let span = (kmax - kmin) as f32 + 1.0;
                    let np = gp.with_clip_rect(body.intersect(grid));
                    for k in 0..reps {
                        let base = c.start + k * plen;
                        let rep_end = (k * plen + plen).min(c.len);
                        if x_of(base as f64) > grid.right() {
                            break;
                        }
                        if x_of((c.start + rep_end) as f64) < grid.left() {
                            continue;
                        }
                        for n in &p.notes {
                            if k * plen + n.start >= rep_end {
                                continue;
                            }
                            let y = body.bottom() - ((n.key - kmin) as f32 + 1.0) / span * body.height();
                            let x0 = x_of((base + n.start) as f64);
                            let x1 = x_of((base + n.end()).min(c.start + rep_end) as f64).max(x0 + 1.5);
                            np.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + (body.height() / span).clamp(1.5, 4.0))), 0.0, col.gamma_multiply(alpha));
                        }
                    }
                }
                // grip on the right edge: drag to repeat the pattern
                if r.right() > grid.left() && r.right() < grid.right() + 1.0 && r.width() > 14.0 {
                    let g = Rect::from_min_max(pos2(r.right() - 6.0, r.top() + 15.0), pos2(r.right() - 1.0, r.bottom() - 2.0));
                    gp.rect_filled(g, 2.0, col.gamma_multiply(0.9 * alpha));
                    for dy in [-4.0, 0.0, 4.0] {
                        gp.circle_filled(pos2(g.center().x, g.center().y + dy), 1.0, Color32::BLACK.gamma_multiply(0.6));
                    }
                }
            }
            ClipKind::Audio(a) => {
                let col = Color32::from_rgb(90, 160, 230);
                gp.rect_filled(r, 3.0, col.gamma_multiply(0.28 * alpha));
                let head = Rect::from_min_max(r.min, pos2(r.right(), r.top() + 13.0));
                gp.rect_filled(head, 3.0, col.gamma_multiply(0.7 * alpha));
                let name = a.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let tag = match a.stretch.mode {
                    StretchMode::Off => String::new(),
                    StretchMode::Stretch => format!(" ⟷{:.0}", a.stretch.source_bpm),
                    StretchMode::Resample => format!(" ◎{:.0}", a.stretch.source_bpm),
                };
                gp.with_clip_rect(r.intersect(grid)).text(head.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, format!("{name}{tag}"), FontId::proportional(10.5), Color32::BLACK);
                let body = Rect::from_min_max(pos2(r.left(), head.bottom()), r.max);
                // fitted to the tempo at the clip start, like the engine does
                let bpm = app.project.bpm_at(c.start);
                let spt = sr * 60.0 / (bpm * PPQ as f64);
                if let Some(l) = app.cache.get_prepared(&a.path, &a.stretch, bpm) {
                    // the sample loops when the clip is longer: draw it segment by segment
                    let slen = l.data.len().max(1) as f64;
                    let mut u0 = 0.0f64;
                    let mut segs = 0;
                    while u0 < c.len as f64 && segs < 4000 {
                        segs += 1;
                        let pos = ((u0 + a.offset as f64) * spt).rem_euclid(slen);
                        let u1 = (u0 + (slen - pos) / spt).min(c.len as f64).max(u0 + 1.0 / spt);
                        let (x0, x1) = (x_of(c.start as f64 + u0), x_of(c.start as f64 + u1));
                        if x0 > grid.right() {
                            break;
                        }
                        if u0 > 0.0 && x0 >= grid.left() {
                            gp.line_segment([pos2(x0, r.top() + 1.0), pos2(x0, r.bottom() - 1.0)], Stroke::new(1.0, col.gamma_multiply(0.9 * alpha)));
                            gp.text(pos2(x0 + 3.0, head.center().y), Align2::LEFT_CENTER, "↻", FontId::proportional(10.0), Color32::BLACK.gamma_multiply(0.7));
                        }
                        let seg = Rect::from_min_max(pos2(x0, body.top()), pos2(x1, body.bottom()));
                        let vis = seg.intersect(grid);
                        if vis.width() > 1.0 {
                            let (s0, s1) = (pos, pos + (u1 - u0) * spt);
                            let f0 = ((vis.left() - seg.left()) / seg.width().max(1e-3)) as f64;
                            let f1 = ((vis.right() - seg.left()) / seg.width().max(1e-3)) as f64;
                            widgets::draw_waveform(&gp, vis, &l.peaks, s0 + (s1 - s0) * f0, s0 + (s1 - s0) * f1, col.gamma_multiply(alpha));
                        }
                        u0 = u1;
                    }
                } else if let Some(e) = app.cache.error(&a.path) {
                    gp.text(body.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, format!("⚠ {e}"), FontId::proportional(10.0), Color32::from_rgb(240, 110, 90));
                } else {
                    gp.text(body.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, "lädt…", FontId::proportional(10.0), TEXT_DIM);
                }
            }
            ClipKind::Automation(a) => {
                let col = AUTO_COLOR;
                gp.rect_filled(r, 3.0, col.gamma_multiply(0.22 * alpha));
                let head = Rect::from_min_max(r.min, pos2(r.right(), r.top() + 13.0));
                gp.rect_filled(head, 3.0, col.gamma_multiply(0.7 * alpha));
                let label = app.project.auto_label(&a.target);
                gp.with_clip_rect(r.intersect(grid)).text(head.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, format!("〰 {label}"), FontId::proportional(10.5), Color32::BLACK);
                let body = Rect::from_min_max(pos2(r.left(), head.bottom() + 2.0), pos2(r.right(), r.bottom() - 2.0));
                let range = field_range(Project::auto_field_name(&a.target));
                let pts = curve_points(&a.points, c.len, range, |t| x_of((c.start + t) as f64), body);
                gp.with_clip_rect(body.intersect(grid)).add(egui::Shape::line(pts, Stroke::new(1.5, col.gamma_multiply(alpha))));
            }
        }
        gp.rect_stroke(r, 3.0, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { Color32::from_black_alpha(120) }));
    }

    // playhead
    if app.song_mode || app.is_playing() {
        let x = x_of(app.position());
        if x >= grid.left() && x <= grid.right() {
            painter.line_segment([pos2(x, ruler.top()), pos2(x, grid.bottom())], Stroke::new(1.5, PLAYHEAD));
        }
    }
    // drop hint
    if egui::DragAndDrop::has_payload_of_type::<SampleDrag>(ui.ctx()) && grid_top {
        if let Some(pt) = pointer {
            let start = floor_to(tick_of(pt.x) as i64, snap_t).max(0);
            let tr = track_of(pt.y).max(0) as usize;
            let x = x_of(start as f64);
            let y = track_top(tr);
            gp.rect_stroke(Rect::from_min_size(pos2(x, y + 1.0), vec2(ppb * 4.0, th - 2.0)), 3.0, Stroke::new(1.5, ACCENT));
        }
    }

    // ruler
    let rp = painter.with_clip_rect(ruler);
    rp.rect_filled(ruler, 0.0, Color32::from_rgb(30, 33, 38));
    if let Some((a, b)) = app.project.playlist.loop_region {
        let (xa, xb) = (x_of(a as f64), x_of(b as f64));
        rp.rect_filled(Rect::from_min_max(pos2(xa, ruler.top()), pos2(xb, ruler.bottom())), 0.0, ACCENT.gamma_multiply(0.45));
        // edge handles
        for x in [xa, xb] {
            rp.rect_filled(Rect::from_center_size(pos2(x, ruler.center().y), vec2(4.0, ruler.height())), 1.0, ACCENT);
        }
        gp.line_segment([pos2(xa, grid.top()), pos2(xa, grid.bottom())], Stroke::new(1.0, ACCENT.gamma_multiply(0.6)));
        gp.line_segment([pos2(xb, grid.top()), pos2(xb, grid.bottom())], Stroke::new(1.0, ACCENT.gamma_multiply(0.6)));
    }
    let bar_px = ppb * (bar / PPQ) as f32;
    let label_every = ((40.0 / bar_px).ceil() as i64).max(1);
    let mut b = floor_to(t0, bar);
    while b <= t1 {
        let x = x_of(b as f64);
        let n = b / bar;
        if n % label_every == 0 {
            rp.line_segment([pos2(x, ruler.top() + 10.0), pos2(x, ruler.bottom())], Stroke::new(1.0, TEXT_DIM));
            rp.text(pos2(x + 3.0, ruler.center().y - 2.0), Align2::LEFT_CENTER, format!("{}", n + 1), FontId::proportional(11.0), TEXT_DIM);
        }
        b += bar;
    }
    // song markers (Intro, Drop …): flag in the ruler + faint line through the tracks
    const MARKER: Color32 = Color32::from_rgb(255, 196, 70);
    for m in &app.project.playlist.markers {
        if m.tick < t0 - bar * 8 || m.tick > t1 {
            continue;
        }
        let x = x_of(m.tick as f64);
        gp.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0, MARKER.gamma_multiply(0.35)));
        let galley = rp.layout_no_wrap(m.name.clone(), FontId::proportional(10.5), Color32::BLACK);
        let tag = Rect::from_min_size(pos2(x, ruler.top() + 1.0), vec2(galley.size().x + 8.0, 12.0));
        rp.rect_filled(tag, 2.0, MARKER);
        rp.galley(pos2(tag.left() + 4.0, tag.center().y - galley.size().y / 2.0), galley, Color32::BLACK);
    }

    tempo_lane(app, ui, &painter, lane, x_of, tick_of, snap_t, t1);
    painter.text(pos2(full.left() + 8.0, lane.center().y), Align2::LEFT_CENTER, "Tempo", FontId::proportional(10.5), TEXT_DIM);

    // headers
    let hp = painter.with_clip_rect(heads);
    {
        let y = track_top(n_tracks);
        if y < heads.bottom() {
            let r = Rect::from_min_max(pos2(heads.left(), y), pos2(heads.right() - 2.0, y + th.min(26.0)));
            let hover = ui.rect_contains_pointer(r);
            hp.rect_stroke(r.shrink(2.0), 3.0, Stroke::new(1.0, if hover { ACCENT } else { Color32::from_gray(70) }));
            hp.text(r.center(), Align2::CENTER_CENTER, "+ Spur", FontId::proportional(12.0), if hover { Color32::WHITE } else { TEXT_DIM });
        }
    }
    let any_solo = app.project.playlist.tracks.iter().any(|t| t.solo);
    let menu_open = head_resp.context_menu_opened();
    for t in 0..n_tracks {
        let y = track_top(t);
        if y > heads.bottom() || y + th < heads.top() {
            continue;
        }
        let r = Rect::from_min_max(pos2(heads.left(), y), pos2(heads.right() - 2.0, y + th - 1.0));
        let tr = &app.project.playlist.tracks[t];
        let audible = app.project.playlist_track_audible(t);
        let marked = menu_open && app.playlist_ui.menu_track == Some(t);
        hp.rect_filled(r, 2.0, BG_LIGHT);
        if marked {
            hp.rect_stroke(r, 2.0, Stroke::new(1.0, ACCENT));
        }
        let led = Rect::from_center_size(pos2(r.left() + 11.0, r.center().y), vec2(10.0, 10.0));
        hp.circle_filled(led.center(), 5.0, if !audible { Color32::from_gray(70) } else { Color32::from_rgb(120, 230, 140) });
        let name_w = r.width() - 24.0 - 44.0;
        let max = (name_w / 6.5).max(3.0) as usize;
        let name: String = if tr.name.chars().count() > max { tr.name.chars().take(max.saturating_sub(1)).chain(std::iter::once('…')).collect() } else { tr.name.clone() };
        hp.text(pos2(r.left() + 24.0, r.center().y - 5.0), Align2::LEFT_CENTER, name, FontId::proportional(12.0), if audible { Color32::WHITE } else { TEXT_DIM });
        let btn = |k: usize, label: &str, on: bool, col: Color32| {
            let b = head_btn(t, k);
            hp.rect_filled(b, 3.0, if on { col } else { BG_DARK });
            hp.text(b.center(), Align2::CENTER_CENTER, label, FontId::proportional(10.0), if on { Color32::BLACK } else { TEXT_DIM });
        };
        // volume bar
        let vb = vol_bar(t);
        hp.rect_filled(vb, 2.0, BG_DARK);
        let fill = (tr.volume / VOL_MAX).clamp(0.0, 1.0);
        let vcol = if tr.volume > 1.0 { Color32::from_rgb(240, 170, 70) } else { Color32::from_rgb(110, 190, 240) };
        hp.rect_filled(Rect::from_min_max(vb.min, pos2(vb.left() + vb.width() * fill, vb.bottom())), 2.0, if audible { vcol } else { vcol.gamma_multiply(0.4) });
        let unity = vb.left() + vb.width() / VOL_MAX;
        hp.line_segment([pos2(unity, vb.top() - 1.0), pos2(unity, vb.bottom() + 1.0)], Stroke::new(1.0, Color32::from_gray(160)));
        btn(1, "M", tr.muted, Color32::from_rgb(230, 110, 100));
        btn(0, "S", tr.solo, Color32::from_rgb(240, 200, 70));
        if any_solo && !tr.solo && !tr.muted {
            hp.line_segment([pos2(r.left() + 24.0, r.bottom() - 4.0), pos2(r.right() - 48.0, r.bottom() - 4.0)], Stroke::new(1.0, Color32::from_gray(70)));
        }
    }
}
