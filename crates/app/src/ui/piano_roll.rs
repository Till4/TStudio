use crate::app::TStudio;
use crate::ui::widgets::{self, floor_to, rgb, round_to, Snap, ACCENT, BG_DARK, GRID_BAR, GRID_BEAT, GRID_SUB, PLAYHEAD, TEXT_DIM};
use egui::{pos2, vec2, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui};
use std::collections::HashSet;
use std::time::Instant;
use tstudio_dsp::note_name;
use tstudio_project::{Note, PPQ};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleKind {
    None,
    Major,
    Minor,
    HarmonicMinor,
    MinorPentatonic,
}

impl ScaleKind {
    const ALL: [ScaleKind; 5] = [ScaleKind::None, ScaleKind::Major, ScaleKind::Minor, ScaleKind::HarmonicMinor, ScaleKind::MinorPentatonic];
    fn name(self) -> &'static str {
        match self {
            ScaleKind::None => "Keine Skala",
            ScaleKind::Major => "Dur",
            ScaleKind::Minor => "Moll",
            ScaleKind::HarmonicMinor => "Harm. Moll",
            ScaleKind::MinorPentatonic => "Moll-Pentatonik",
        }
    }
    fn intervals(self) -> &'static [u8] {
        match self {
            ScaleKind::None => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            ScaleKind::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleKind::Minor => &[0, 2, 3, 5, 7, 8, 10],
            ScaleKind::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            ScaleKind::MinorPentatonic => &[0, 3, 5, 7, 10],
        }
    }
    fn contains(self, root: u8, key: u8) -> bool {
        self.intervals().contains(&((key + 12 - root % 12) % 12))
    }
}

enum Drag {
    Move { anchor_tick: f64, anchor_key: i32, orig: Vec<(usize, Note)>, preview: i32 },
    Resize { anchor_tick: f64, orig: Vec<(usize, Note)> },
    Select { from: Pos2, additive: bool, base: HashSet<usize> },
}

pub struct PianoState {
    pub px_per_beat: f32,
    pub row_h: f32,
    pub scroll_x: f32,
    pub top_key: f32,
    pub snap: Snap,
    pub selection: HashSet<usize>,
    drag: Option<Drag>,
    pub last_len: i64,
    pub scale_root: u8,
    pub scale: ScaleKind,
    pub ghosts: bool,
    clipboard: Vec<Note>,
    /// Preview notes to release when the mouse button goes up: (channel, key).
    pub pending_off: Vec<(usize, u8)>,
    pub flash_key: Option<(u8, Instant)>,
    /// (pattern id, channel) the selection refers to
    sel_owner: (u64, usize),
}

impl Default for PianoState {
    fn default() -> Self {
        Self {
            px_per_beat: 90.0,
            row_h: 15.0,
            scroll_x: 0.0,
            top_key: 86.0,
            snap: Snap::Quarter,
            selection: HashSet::new(),
            drag: None,
            last_len: PPQ / 4,
            scale_root: 0,
            scale: ScaleKind::None,
            ghosts: true,
            clipboard: Vec::new(),
            pending_off: Vec::new(),
            flash_key: None,
            sel_owner: (0, 0),
        }
    }
}

impl PianoState {
    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.drag = None;
    }
}

/// Sends note-offs for preview notes once the mouse is released.
pub fn release_pending(app: &mut TStudio, ctx: &egui::Context) {
    if !app.piano.pending_off.is_empty() && !ctx.input(|i| i.pointer.any_down()) {
        let offs = std::mem::take(&mut app.piano.pending_off);
        for (c, k) in offs {
            app.note_off(c, k);
        }
    }
}

fn is_black(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    if app.project.channels.is_empty() {
        ui.heading("Piano Roll");
        ui.label("Füge zuerst im Channel Rack einen Kanal hinzu.");
        return;
    }
    let ch = app.selected_channel;
    let pid = app.current_pattern;
    if app.piano.sel_owner != (pid, ch) {
        app.piano.clear_selection();
        app.piano.sel_owner = (pid, ch);
    }
    let bar = app.project.ticks_per_bar();

    // ---------------------------------------------------------------- toolbar
    ui.horizontal(|ui| {
        ui.heading("Piano Roll");
        let names: Vec<String> = app.project.channels.iter().map(|c| c.name.clone()).collect();
        egui::ComboBox::from_id_salt("pr_channel").selected_text(&names[ch]).width(140.0).show_ui(ui, |ui| {
            for (i, n) in names.iter().enumerate() {
                ui.selectable_value(&mut app.selected_channel, i, n);
            }
        });
        widgets::snap_combo(ui, "pr_snap", &mut app.piano.snap);
        egui::ComboBox::from_id_salt("pr_scale").selected_text(app.piano.scale.name()).width(110.0).show_ui(ui, |ui| {
            for s in ScaleKind::ALL {
                ui.selectable_value(&mut app.piano.scale, s, s.name());
            }
        });
        if app.piano.scale != ScaleKind::None {
            egui::ComboBox::from_id_salt("pr_root").selected_text(tstudio_dsp::NOTE_NAMES[app.piano.scale_root as usize]).width(50.0).show_ui(ui, |ui| {
                for (i, n) in tstudio_dsp::NOTE_NAMES.iter().enumerate() {
                    ui.selectable_value(&mut app.piano.scale_root, i as u8, *n);
                }
            });
        }
        ui.toggle_value(&mut app.piano.ghosts, "Ghost-Noten");
        if ui.button("Quantisieren (Q)").clicked() {
            quantize(app);
        }
        ui.label(format!("{} ausgewählt", app.piano.selection.len()));
    });

    // ---------------------------------------------------------------- layout
    let full = ui.available_rect_before_wrap();
    let kb_w = 54.0;
    let ruler_h = 20.0;
    let vel_h = 70.0;
    let kb = Rect::from_min_max(pos2(full.left(), full.top() + ruler_h), pos2(full.left() + kb_w, full.bottom() - vel_h - 4.0));
    let ruler = Rect::from_min_max(pos2(full.left() + kb_w, full.top()), pos2(full.right(), full.top() + ruler_h));
    let grid = Rect::from_min_max(pos2(kb.right(), kb.top()), pos2(full.right(), kb.bottom()));
    let vel = Rect::from_min_max(pos2(grid.left(), full.bottom() - vel_h), pos2(full.right(), full.bottom()));
    ui.allocate_rect(full, Sense::hover());

    // scroll / zoom
    let st = &mut app.piano;
    if ui.rect_contains_pointer(full) {
        let (scroll, zoom, mods, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers, i.pointer.hover_pos()));
        if zoom != 1.0 {
            let mx = hover.map(|p| p.x).unwrap_or(grid.left());
            let tick_at = st.scroll_x + (mx - grid.left()) / st.px_per_beat * PPQ as f32;
            st.px_per_beat = (st.px_per_beat * zoom).clamp(12.0, 800.0);
            st.scroll_x = (tick_at - (mx - grid.left()) / st.px_per_beat * PPQ as f32).max(0.0);
        } else if mods.shift {
            st.scroll_x = (st.scroll_x - (scroll.y + scroll.x) / st.px_per_beat * PPQ as f32).max(0.0);
        } else {
            st.top_key += scroll.y / st.row_h;
            st.scroll_x = (st.scroll_x - scroll.x / st.px_per_beat * PPQ as f32).max(0.0);
        }
        let visible_rows = grid.height() / st.row_h;
        st.top_key = st.top_key.clamp(visible_rows.min(128.0), 128.0);
    }
    let (ppb, row_h, sx, top) = (st.px_per_beat, st.row_h, st.scroll_x, st.top_key);
    let x_of = move |t: f64| grid.left() + ((t as f32 - sx) / PPQ as f32) * ppb;
    let tick_of = move |x: f32| (sx + (x - grid.left()) / ppb * PPQ as f32) as f64;
    let key_of = move |y: f32| (top - (y - grid.top()) / row_h).floor() as i32;
    let row_top = move |k: i32| grid.top() + (top - k as f32 - 1.0) * row_h;

    let snap_t = app.piano.snap.ticks(bar);
    let pat_len = app.project.pattern_length(pid);
    let Some(cid) = app.project.channels.get(ch).map(|c| c.id) else { return };
    let color = rgb(app.project.channels[ch].color);
    let ch_ids: Vec<(u64, Color32)> = app.project.channels.iter().map(|c| (c.id, rgb(c.color))).collect();

    // ---------------------------------------------------------------- interaction
    let grid_resp = ui.interact(grid, ui.id().with("pr_grid"), Sense::click_and_drag());
    let (pointer, pressed, down, released, sec_down, mods) =
        ui.input(|i| (i.pointer.interact_pos().or(i.pointer.hover_pos()), i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.secondary_down(), i.modifiers));

    let hit_test = |notes: &[Note], p: Pos2| -> Option<(usize, bool)> {
        for (i, n) in notes.iter().enumerate().rev() {
            if n.channel != cid {
                continue;
            }
            let r = Rect::from_min_max(pos2(x_of(n.start as f64), row_top(n.key as i32)), pos2(x_of(n.end() as f64), row_top(n.key as i32) + row_h));
            if r.contains(p) {
                let edge = p.x > r.right() - (r.width() * 0.25).clamp(3.0, 8.0);
                return Some((i, edge));
            }
        }
        None
    };

    let grid_top = ui.rect_contains_pointer(grid);
    let mut preview_on: Option<u8> = None;
    let mut preview_off: Option<u8> = None;
    if let Some(p) = app.project.pattern_mut(pid) {
        let st = &mut app.piano;
        // hover cursor
        if let Some(pt) = pointer {
            if grid.contains(pt) && st.drag.is_none() {
                if let Some((_, true)) = hit_test(&p.notes, pt) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
        // press
        if pressed && grid_top {
            if let Some(pt) = pointer.filter(|pt| grid.contains(*pt)) {
                let t = tick_of(pt.x);
                let k = key_of(pt.y).clamp(0, 127);
                if mods.command {
                    let base = if mods.shift { st.selection.clone() } else { HashSet::new() };
                    st.drag = Some(Drag::Select { from: pt, additive: mods.shift, base });
                } else if let Some((i, edge)) = hit_test(&p.notes, pt) {
                    if !st.selection.contains(&i) {
                        if !mods.shift {
                            st.selection.clear();
                        }
                        st.selection.insert(i);
                    }
                    let orig: Vec<(usize, Note)> = st.selection.iter().filter_map(|&j| p.notes.get(j).map(|n| (j, *n))).collect();
                    if edge {
                        st.drag = Some(Drag::Resize { anchor_tick: t, orig });
                    } else {
                        let key = p.notes[i].key;
                        preview_on = Some(key);
                        st.drag = Some(Drag::Move { anchor_tick: t, anchor_key: k, orig, preview: key as i32 });
                    }
                } else {
                    let start = floor_to(t as i64, snap_t).max(0);
                    let note = Note { channel: cid, key: k as u8, start, len: st.last_len.max(1), velocity: 0.8 };
                    p.notes.push(note);
                    let i = p.notes.len() - 1;
                    st.selection.clear();
                    st.selection.insert(i);
                    preview_on = Some(k as u8);
                    st.drag = Some(Drag::Move { anchor_tick: t, anchor_key: k, orig: vec![(i, note)], preview: k });
                }
            }
        }
        // drag
        if down {
            if let (Some(pt), Some(drag)) = (pointer, st.drag.as_mut()) {
                match drag {
                    Drag::Move { anchor_tick, anchor_key, orig, preview } => {
                        let min_start = orig.iter().map(|(_, n)| n.start).min().unwrap_or(0);
                        let dt = round_to((tick_of(pt.x) - *anchor_tick) as i64, snap_t).max(-min_start);
                        let max_key = orig.iter().map(|(_, n)| n.key as i32).max().unwrap_or(0);
                        let min_key = orig.iter().map(|(_, n)| n.key as i32).min().unwrap_or(0);
                        let dk = (key_of(pt.y) - *anchor_key).clamp(-min_key, 127 - max_key);
                        for (i, n) in orig.iter() {
                            if let Some(t) = p.notes.get_mut(*i) {
                                t.start = n.start + dt;
                                t.key = (n.key as i32 + dk) as u8;
                            }
                        }
                        if orig.len() == 1 {
                            let nk = orig[0].1.key as i32 + dk;
                            if nk != *preview {
                                preview_off = Some(*preview as u8);
                                preview_on = Some(nk as u8);
                                *preview = nk;
                            }
                        }
                    }
                    Drag::Resize { anchor_tick, orig } => {
                        let dt = round_to((tick_of(pt.x) - *anchor_tick) as i64, snap_t);
                        for (i, n) in orig.iter() {
                            if let Some(t) = p.notes.get_mut(*i) {
                                t.len = (n.len + dt).max(snap_t.max(1));
                            }
                        }
                    }
                    Drag::Select { from, base, .. } => {
                        let r = Rect::from_two_pos(*from, pt);
                        let mut sel = base.clone();
                        for (i, n) in p.notes.iter().enumerate() {
                            if n.channel != cid {
                                continue;
                            }
                            let nr = Rect::from_min_max(pos2(x_of(n.start as f64), row_top(n.key as i32)), pos2(x_of(n.end() as f64), row_top(n.key as i32) + row_h));
                            if nr.intersects(r) {
                                sel.insert(i);
                            }
                        }
                        st.selection = sel;
                    }
                }
            }
        }
        if released {
            if let Some(d) = st.drag.take() {
                match d {
                    Drag::Move { orig, preview, .. } => {
                        preview_off = Some(preview as u8);
                        if orig.len() == 1 {
                            if let Some(n) = p.notes.get(orig[0].0) {
                                st.last_len = n.len;
                            }
                        }
                    }
                    Drag::Resize { orig, .. } => {
                        if let Some(n) = orig.first().and_then(|(i, _)| p.notes.get(*i)) {
                            st.last_len = n.len;
                        }
                    }
                    Drag::Select { .. } => {}
                }
            }
        }
        // right button erases
        if sec_down && grid_top && st.drag.is_none() {
            if let Some(pt) = pointer.filter(|pt| grid.contains(*pt)) {
                if let Some((i, _)) = hit_test(&p.notes, pt) {
                    p.notes.remove(i);
                    st.selection.clear();
                }
            }
        }
        // keyboard
        let hovered = ui.rect_contains_pointer(full);
        if hovered && !ui.ctx().wants_keyboard_input() {
            let (del, a, c, v, d, up, dn, left, right, q) = ui.input(|i| {
                (
                    i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                    i.key_pressed(Key::A),
                    i.key_pressed(Key::C),
                    i.key_pressed(Key::V),
                    i.key_pressed(Key::D),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowLeft),
                    i.key_pressed(Key::ArrowRight),
                    i.key_pressed(Key::Q),
                )
            });
            if del && !st.selection.is_empty() {
                let mut idx: Vec<usize> = st.selection.drain().collect();
                idx.sort_unstable_by(|a, b| b.cmp(a));
                for i in idx {
                    if i < p.notes.len() {
                        p.notes.remove(i);
                    }
                }
            }
            if mods.command && a {
                st.selection = p.notes.iter().enumerate().filter(|(_, n)| n.channel == cid).map(|(i, _)| i).collect();
            }
            let (copy, cut, paste) = widgets::edit_keys(ui);
            let _ = (c, v);
            if (copy || cut) && !st.selection.is_empty() {
                st.clipboard = st.selection.iter().filter_map(|&i| p.notes.get(i).copied()).collect();
                widgets::mark_copied(ui, "Noten");
                if cut {
                    let mut idx: Vec<usize> = st.selection.drain().collect();
                    idx.sort_unstable_by(|a, b| b.cmp(a));
                    for i in idx {
                        if i < p.notes.len() {
                            p.notes.remove(i);
                        }
                    }
                }
            }
            if paste && !st.clipboard.is_empty() {
                let min = st.clipboard.iter().map(|n| n.start).min().unwrap_or(0);
                let at = pointer.filter(|pt| grid.contains(*pt)).map(|pt| floor_to(tick_of(pt.x) as i64, snap_t)).unwrap_or(min).max(0);
                st.selection.clear();
                for n in st.clipboard.clone() {
                    p.notes.push(Note { channel: cid, start: n.start - min + at, ..n });
                    st.selection.insert(p.notes.len() - 1);
                }
            }
            if mods.command && d && !st.selection.is_empty() {
                let sel: Vec<Note> = st.selection.iter().filter_map(|&i| p.notes.get(i).copied()).collect();
                let min = sel.iter().map(|n| n.start).min().unwrap_or(0);
                let max = sel.iter().map(|n| n.end()).max().unwrap_or(0);
                let span = ((max - min + PPQ - 1) / PPQ).max(1) * PPQ;
                st.selection.clear();
                for n in sel {
                    p.notes.push(Note { start: n.start + span, ..n });
                    st.selection.insert(p.notes.len() - 1);
                }
            }
            if (up || dn) && !st.selection.is_empty() {
                let step: i32 = if mods.shift { 12 } else { 1 };
                let dk = if up { step } else { -step };
                for &i in st.selection.iter() {
                    if let Some(n) = p.notes.get_mut(i) {
                        n.key = (n.key as i32 + dk).clamp(0, 127) as u8;
                    }
                }
            }
            if (left || right) && !st.selection.is_empty() {
                let dt = if right { snap_t } else { -snap_t };
                for &i in st.selection.iter() {
                    if let Some(n) = p.notes.get_mut(i) {
                        n.start = (n.start + dt).max(0);
                    }
                }
            }
            if q && !mods.command {
                let sel = st.selection.clone();
                for (i, n) in p.notes.iter_mut().enumerate() {
                    if n.channel == cid && (sel.is_empty() || sel.contains(&i)) {
                        n.start = round_to(n.start, snap_t).max(0);
                    }
                }
            }
        }
    }
    if let Some(k) = preview_off {
        app.note_off(ch, k);
    }
    if let Some(k) = preview_on {
        app.note_on(ch, k, 0.8);
        app.piano.pending_off.push((ch, k));
    }

    // ruler: click sets position
    let ruler_resp = ui.interact(ruler, ui.id().with("pr_ruler"), Sense::click_and_drag());
    if ruler_resp.clicked() || ruler_resp.dragged() {
        if let Some(pt) = ruler_resp.interact_pointer_pos() {
            let t = floor_to(tick_of(pt.x) as i64, snap_t).clamp(0, pat_len - 1);
            app.send(tstudio_engine::Command::SetPosition(t as f64));
        }
    }

    // keyboard (left)
    let kb_resp = ui.interact(kb, ui.id().with("pr_kb"), Sense::click_and_drag());
    if kb_resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
        if let Some(pt) = kb_resp.interact_pointer_pos() {
            let k = key_of(pt.y).clamp(0, 127) as u8;
            app.note_on(ch, k, 0.8);
            app.piano.pending_off.push((ch, k));
        }
    }

    // velocity lane
    let vel_resp = ui.interact(vel, ui.id().with("pr_vel"), Sense::click_and_drag());
    if vel_resp.is_pointer_button_down_on() {
        if let Some(pt) = vel_resp.interact_pointer_pos() {
            let v = ((vel.bottom() - pt.y) / (vel.height() - 4.0)).clamp(0.05, 1.0);
            let sel = app.piano.selection.clone();
            if let Some(p) = app.project.pattern_mut(pid) {
                for (i, n) in p.notes.iter_mut().enumerate() {
                    if n.channel == cid && (sel.is_empty() || sel.contains(&i)) && (x_of(n.start as f64) - pt.x).abs() < 6.0 {
                        n.velocity = v;
                    }
                }
            }
        }
    }
    let _ = grid_resp;
    release_pending(app, ui.ctx());

    // ---------------------------------------------------------------- painting
    let painter = ui.painter_at(full);
    painter.rect_filled(full, 0.0, BG_DARK);
    let gp = painter.with_clip_rect(grid);
    let first_key = key_of(grid.bottom()).max(0);
    let last_key = key_of(grid.top()).min(127);
    let (scale, root) = (app.piano.scale, app.piano.scale_root);
    for k in first_key..=last_key {
        let y = row_top(k);
        let r = Rect::from_min_max(pos2(grid.left(), y), pos2(grid.right(), y + row_h));
        let mut c = if is_black(k as u8) { Color32::from_rgb(33, 36, 41) } else { Color32::from_rgb(42, 46, 52) };
        if scale != ScaleKind::None && scale.contains(root, k as u8) {
            c = c.gamma_multiply(1.25);
        }
        gp.rect_filled(r, 0.0, c);
        if k % 12 == 0 {
            gp.line_segment([pos2(grid.left(), y + row_h), pos2(grid.right(), y + row_h)], Stroke::new(1.0, GRID_BEAT));
        }
    }
    let t0 = tick_of(grid.left()).max(0.0) as i64;
    let t1 = tick_of(grid.right()) as i64 + 1;
    let line_step = if snap_t > 1 { snap_t } else { PPQ / 4 };
    let mut t = floor_to(t0, line_step);
    while t <= t1 {
        let x = x_of(t as f64);
        let (col, w) = if t % bar == 0 {
            (GRID_BAR, 1.0)
        } else if t % PPQ == 0 {
            (GRID_BEAT, 1.0)
        } else {
            (GRID_SUB, 1.0)
        };
        if (t % PPQ == 0) || (line_step as f32 / PPQ as f32 * ppb > 7.0) {
            gp.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(w, col));
        }
        t += line_step;
    }
    // area beyond pattern end
    let end_x = x_of(pat_len as f64);
    if end_x < grid.right() {
        gp.rect_filled(Rect::from_min_max(pos2(end_x.max(grid.left()), grid.top()), grid.max), 0.0, Color32::from_black_alpha(90));
    }

    if let Some(p) = app.project.pattern(pid) {
        // ghosts
        if app.piano.ghosts {
            for n in p.notes.iter().filter(|n| n.channel != cid) {
                let c = ch_ids.iter().find(|(id, _)| *id == n.channel).map(|(_, c)| *c).unwrap_or(TEXT_DIM);
                let r = Rect::from_min_max(pos2(x_of(n.start as f64), row_top(n.key as i32) + 1.0), pos2(x_of(n.end() as f64), row_top(n.key as i32) + row_h - 1.0));
                if r.intersects(grid) {
                    gp.rect_filled(r, 2.0, c.gamma_multiply(0.22));
                }
            }
        }
        for (i, n) in p.notes.iter().enumerate().filter(|(_, n)| n.channel == cid) {
            let r = Rect::from_min_max(pos2(x_of(n.start as f64), row_top(n.key as i32) + 1.0), pos2(x_of(n.end() as f64), row_top(n.key as i32) + row_h - 1.0));
            if !r.intersects(grid) {
                continue;
            }
            let sel = app.piano.selection.contains(&i);
            let fill = color.gamma_multiply(0.45 + 0.55 * n.velocity);
            gp.rect_filled(r, 3.0, fill);
            gp.rect_stroke(r, 3.0, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { color.gamma_multiply(0.5) }));
            if r.width() > 26.0 && row_h >= 11.0 {
                gp.with_clip_rect(r.intersect(grid)).text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, note_name(n.key), FontId::proportional(9.5), Color32::BLACK);
            }
        }
    }
    if let Some(Drag::Select { from, .. }) = &app.piano.drag {
        if let Some(pt) = pointer {
            let r = Rect::from_two_pos(*from, pt);
            gp.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.15));
            gp.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT));
        }
    }
    // playhead
    if app.is_playing() && !app.song_mode {
        let pos = app.position().rem_euclid(pat_len as f64);
        let x = x_of(pos);
        if x >= grid.left() && x <= grid.right() {
            painter.line_segment([pos2(x, ruler.top()), pos2(x, vel.bottom())], Stroke::new(1.5, PLAYHEAD));
        }
    }

    // keyboard
    let kp = painter.with_clip_rect(kb);
    let flash = app.piano.flash_key.filter(|(_, t)| t.elapsed().as_millis() < 250).map(|(k, _)| k as i32);
    for k in first_key..=last_key {
        let y = row_top(k);
        let r = Rect::from_min_max(pos2(kb.left(), y), pos2(kb.right(), y + row_h));
        let black = is_black(k as u8);
        let mut fill = if black { Color32::from_rgb(25, 25, 28) } else { Color32::from_rgb(222, 222, 225) };
        if flash == Some(k) {
            fill = ACCENT;
        }
        let r2 = if black { Rect::from_min_max(r.min, pos2(kb.left() + kb_w * 0.66, r.max.y)) } else { r };
        kp.rect_filled(r2.shrink2(vec2(0.0, 0.5)), 1.0, fill);
        if k % 12 == 0 {
            kp.text(pos2(kb.right() - 3.0, r.center().y), Align2::RIGHT_CENTER, note_name(k as u8), FontId::proportional(9.5), Color32::from_gray(60));
        }
    }

    // ruler
    let rp = painter.with_clip_rect(ruler);
    rp.rect_filled(ruler, 0.0, Color32::from_rgb(30, 33, 38));
    let mut b = floor_to(t0, bar);
    while b <= t1 {
        let x = x_of(b as f64);
        rp.line_segment([pos2(x, ruler.top() + 8.0), pos2(x, ruler.bottom())], Stroke::new(1.0, TEXT_DIM));
        rp.text(pos2(x + 3.0, ruler.center().y), Align2::LEFT_CENTER, format!("{}", b / bar + 1), FontId::proportional(11.0), TEXT_DIM);
        b += bar;
    }

    // velocity lane
    let vp = painter.with_clip_rect(vel);
    vp.rect_filled(vel, 0.0, Color32::from_rgb(30, 33, 38));
    vp.text(pos2(full.left() + 4.0, vel.center().y), Align2::LEFT_CENTER, "Velocity", FontId::proportional(10.0), TEXT_DIM);
    if let Some(p) = app.project.pattern(pid) {
        for (i, n) in p.notes.iter().enumerate().filter(|(_, n)| n.channel == cid) {
            let x = x_of(n.start as f64);
            if x < vel.left() - 2.0 || x > vel.right() {
                continue;
            }
            let h = (vel.height() - 4.0) * n.velocity;
            let col = if app.piano.selection.contains(&i) { Color32::WHITE } else { color };
            vp.line_segment([pos2(x + 1.0, vel.bottom()), pos2(x + 1.0, vel.bottom() - h)], Stroke::new(2.0, col));
            vp.circle_filled(pos2(x + 1.0, vel.bottom() - h), 3.0, col);
        }
    }
}

fn quantize(app: &mut TStudio) {
    let snap_t = app.piano.snap.ticks(app.project.ticks_per_bar());
    let Some(cid) = app.project.channels.get(app.selected_channel).map(|c| c.id) else { return };
    let sel = app.piano.selection.clone();
    if let Some(p) = app.project.pattern_mut(app.current_pattern) {
        for (i, n) in p.notes.iter_mut().enumerate() {
            if n.channel == cid && (sel.is_empty() || sel.contains(&i)) {
                n.start = round_to(n.start, snap_t).max(0);
            }
        }
    }
}
