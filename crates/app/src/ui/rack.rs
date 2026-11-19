use crate::app::{TStudio, View};
use crate::ui::widgets::{self, rgb, small_toggle, SampleDrag, ACCENT, BG_DARK, BG_LIGHT, TEXT_DIM};
use egui::{vec2, Align2, Color32, FontId, RichText, Sense, Stroke, Ui};
use tstudio_project::{Instrument, Note, SamplerInstrument, PPQ};

const STEP: i64 = PPQ / 4;

/// Pattern combo + new/clone/delete, used in the transport bar.
pub fn pattern_selector(app: &mut TStudio, ui: &mut Ui) {
    let cur_name = app.project.pattern(app.current_pattern).map(|p| p.name.clone()).unwrap_or_default();
    egui::ComboBox::from_id_salt("pattern_sel").selected_text(RichText::new(cur_name).strong()).width(130.0).show_ui(ui, |ui| {
        for p in &app.project.patterns {
            ui.selectable_value(&mut app.current_pattern, p.id, &p.name);
        }
    });
    if ui.small_button("+").on_hover_text("Neues Pattern").clicked() {
        app.current_pattern = app.project.add_pattern();
    }
    ui.menu_button("⋯", |ui| {
        if let Some(p) = app.project.pattern_mut(app.current_pattern) {
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut p.name);
            });
            ui.horizontal(|ui| {
                ui.label("Länge:");
                let bar = 4 * PPQ;
                let mut sel = p.length.map(|l| l / bar).unwrap_or(0);
                for (v, t) in [(0, "Auto"), (1, "1 Takt"), (2, "2"), (4, "4"), (8, "8")] {
                    if ui.selectable_value(&mut sel, v, t).clicked() {
                        p.length = if v == 0 { None } else { Some(v * bar) };
                    }
                }
            });
        }
        if ui.button("Pattern duplizieren").clicked() {
            if let Some(src) = app.project.pattern(app.current_pattern).cloned() {
                let id = app.project.add_pattern();
                if let Some(p) = app.project.pattern_mut(id) {
                    p.notes = src.notes;
                    p.length = src.length;
                    p.name = format!("{} (Kopie)", src.name);
                }
                app.current_pattern = id;
            }
            ui.close_menu();
        }
        if ui.add_enabled(app.project.patterns.len() > 1, egui::Button::new("Pattern löschen")).clicked() {
            let id = app.current_pattern;
            app.project.remove_pattern(id);
            app.current_pattern = app.project.patterns[0].id;
            ui.close_menu();
        }
        if ui.button("Pattern in Playlist einfügen (am Ende)").clicked() {
            let start = app.project.song_length();
            let len = app.project.pattern_length(app.current_pattern);
            let pid = app.current_pattern;
            app.project.add_clip(0, start, len, tstudio_project::ClipKind::Pattern { pattern: pid });
            ui.close_menu();
        }
    });
}

enum RowAction {
    Select(usize),
    OpenPiano(usize),
    OpenInstrument(usize),
    Delete(usize),
    Clone(usize),
    MoveUp(usize),
    MoveDown(usize),
    Preview(usize),
    ReplaceSample(usize, std::path::PathBuf),
    /// solo toggle; true = add to the existing solos (Ctrl), false = exclusive
    Solo(usize, bool),
}

/// Strg+C / Strg+X / Strg+V in the channel rack: copy the selected channel's notes of the current
/// pattern and paste them onto the selected channel (replacing its notes there), like FL's step copy.
fn rack_clipboard(app: &mut TStudio, ui: &mut Ui) {
    if !ui.ui_contains_pointer() || ui.ctx().wants_keyboard_input() {
        return;
    }
    let (copy, cut, paste) = widgets::edit_keys(ui);
    if !(copy || cut || paste) {
        return;
    }
    let clip_id = egui::Id::new("rack_clipboard");
    let Some(cid) = app.project.channels.get(app.selected_channel).map(|c| c.id) else { return };
    let ch_name = app.project.channels[app.selected_channel].name.clone();
    let pid = app.current_pattern;
    if copy || cut {
        let notes: Vec<Note> = app.project.pattern(pid).map(|p| p.notes.iter().filter(|n| n.channel == cid).copied().collect()).unwrap_or_default();
        let n = notes.len();
        ui.data_mut(|d| d.insert_temp(clip_id, notes));
        widgets::mark_copied(ui, "Steps");
        if cut {
            if let Some(p) = app.project.pattern_mut(pid) {
                p.notes.retain(|x| x.channel != cid);
            }
        }
        app.set_status(format!("{n} Noten von „{ch_name}“ {} – Kanal wählen und Strg+V", if cut { "ausgeschnitten" } else { "kopiert" }));
    } else if let Some(notes) = ui.data(|d| d.get_temp::<Vec<Note>>(clip_id)) {
        if let Some(p) = app.project.pattern_mut(pid) {
            p.notes.retain(|x| x.channel != cid);
            p.notes.extend(notes.iter().map(|n| Note { channel: cid, ..*n }));
            p.notes.sort_by_key(|n| n.start);
        }
        app.set_status(format!("{} Noten auf „{ch_name}“ eingefügt", notes.len()));
    }
}

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    rack_clipboard(app, ui);
    let bar = app.project.ticks_per_bar();
    let pat_len = app.project.pattern_length(app.current_pattern).max(bar);
    let steps = (pat_len / STEP) as usize;
    let steps_per_bar = (bar / STEP) as usize;

    ui.horizontal(|ui| {
        ui.heading("Channel Rack");
        let pat_playing = app.is_playing() && !app.song_mode;
        let txt = if pat_playing { RichText::new("⏸ Pattern").color(ACCENT) } else { RichText::new("▶ Pattern") };
        if ui.button(txt).on_hover_text("Nur dieses Pattern abspielen (Leertaste spielt hier im Channel Rack ebenfalls das Pattern)").clicked() {
            app.play_pattern_toggle();
        }
        ui.weak(format!("{} Steps", steps));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+ Sampler…").on_hover_text("Sampler-Kanal aus Datei").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Audio", tstudio_library::AUDIO_EXTS).pick_file() {
                    app.add_sampler_channel(&p);
                }
            }
            if ui.button("+ SoundFont…").on_hover_text("SoundFont-Kanal aus .sf2-Datei").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("SoundFont", &["sf2", "SF2"]).pick_file() {
                    app.add_soundfont_channel(&p);
                }
            }
            if ui.button("+ TSynth").clicked() {
                let i = app.project.add_channel("TSynth", Instrument::Synth(Default::default()));
                app.selected_channel = i;
            }
            if ui.button("+ Leerer Sampler").clicked() {
                let i = app.project.add_channel("Sampler", Instrument::Sampler(SamplerInstrument::default()));
                app.selected_channel = i;
            }
        });
    });
    ui.separator();

    let playing = app.is_playing() && !app.song_mode;
    let play_step = if playing { Some(((app.position() as i64).rem_euclid(pat_len) / STEP) as usize) } else { None };
    let mut actions = Vec::new();
    let pointer_down = ui.input(|i| i.pointer.primary_down());
    let paint_id = egui::Id::new("step_paint");
    let mut paint: Option<bool> = ui.data(|d| d.get_temp(paint_id)).flatten();
    if !pointer_down {
        paint = None;
    }

    let scroll = egui::ScrollArea::both().auto_shrink([false; 2]).id_salt("rack_scroll").show(ui, |ui| {
        let n = app.project.channels.len();
        for ci in 0..n {
            let selected = ci == app.selected_channel;
            let row = ui.horizontal(|ui| {
                ui.set_height(30.0);
                let any_solo = app.project.channels.iter().any(|c| c.solo);
                let ch = &mut app.project.channels[ci];
                // LED: green = audible
                let audible = !ch.muted && (!any_solo || ch.solo);
                let (led, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                ui.painter().circle_filled(led.center(), 4.0, if audible { Color32::from_rgb(120, 230, 140) } else { Color32::from_gray(70) });
                small_toggle(ui, &mut ch.muted, "M", Color32::from_rgb(230, 110, 100), "Stumm (Mute)");
                let mut solo = ch.solo;
                if small_toggle(ui, &mut solo, "S", Color32::from_rgb(240, 200, 70), "Solo – Klick: nur dieser Kanal · Strg+Klick: zu den Solos hinzufügen").clicked() {
                    actions.push(RowAction::Solo(ci, ui.input(|i| i.modifiers.command)));
                }
                knob_small(ui, &mut ch.volume, 0.0, 1.25, 0.8, "Vol", &widgets::fmt_gain);
                knob_small(ui, &mut ch.pan, -1.0, 1.0, 0.0, "Pan", &widgets::fmt_pan);
                ui.add(egui::DragValue::new(&mut ch.mixer_track).range(0..=app.project.mixer.len() - 1).prefix("→"))
                    .on_hover_text("Mixer-Insert (0 = Master)");
                // name button
                let (rect, resp) = ui.allocate_exact_size(vec2(130.0, 24.0), Sense::click());
                let fill = if selected { ACCENT.gamma_multiply(0.45) } else if resp.hovered() { BG_LIGHT } else { BG_DARK };
                ui.painter().rect_filled(rect, 3.0, fill);
                ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(4.0, rect.height())), 2.0, rgb(ch.color));
                let label = if ch.name.chars().count() > 17 { format!("{}…", ch.name.chars().take(16).collect::<String>()) } else { ch.name.clone() };
                ui.painter().text(rect.left_center() + vec2(9.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(12.5), Color32::WHITE);
                let kind = ch.instrument.kind_name();
                if resp.clicked() {
                    actions.push(RowAction::Select(ci));
                    actions.push(RowAction::Preview(ci));
                }
                if resp.double_clicked() {
                    actions.push(RowAction::OpenInstrument(ci));
                }
                if let Some(drop) = resp.dnd_release_payload::<SampleDrag>() {
                    actions.push(RowAction::ReplaceSample(ci, drop.path.clone()));
                }
                let resp = resp.on_hover_text(format!("{kind} – Klick: auswählen · Doppelklick: Instrument öffnen\nSample hierher ziehen ersetzt das Sample"));
                resp.context_menu(|ui| {
                    ui.text_edit_singleline(&mut app.project.channels[ci].name);
                    if ui.button("🎹 Piano Roll").clicked() {
                        actions.push(RowAction::OpenPiano(ci));
                        ui.close_menu();
                    }
                    if ui.button("🎛 Instrument bearbeiten").clicked() {
                        actions.push(RowAction::OpenInstrument(ci));
                        ui.close_menu();
                    }
                    if ui.button("⧉ Duplizieren").clicked() {
                        actions.push(RowAction::Clone(ci));
                        ui.close_menu();
                    }
                    if ui.button("▲ Nach oben").clicked() {
                        actions.push(RowAction::MoveUp(ci));
                        ui.close_menu();
                    }
                    if ui.button("▼ Nach unten").clicked() {
                        actions.push(RowAction::MoveDown(ci));
                        ui.close_menu();
                    }
                    if ui.button("🗑 Löschen").clicked() {
                        actions.push(RowAction::Delete(ci));
                        ui.close_menu();
                    }
                });
                if ui.small_button("🎹").on_hover_text("Piano Roll").clicked() {
                    actions.push(RowAction::OpenPiano(ci));
                }

                // steps
                let cid = app.project.channels[ci].id;
                let root = app.project.channels[ci].instrument.default_key();
                let color = rgb(app.project.channels[ci].color);
                let pid = app.current_pattern;
                let pointer = ui.input(|i| i.pointer.interact_pos());
                let mut on_step = vec![false; steps];
                let mut off_grid = vec![false; steps];
                if let Some(p) = app.project.pattern(pid) {
                    for n in p.notes.iter().filter(|n| n.channel == cid && n.start >= 0) {
                        let s = (n.start / STEP) as usize;
                        if s < steps {
                            if n.start % STEP == 0 {
                                on_step[s] = true;
                            } else {
                                off_grid[s] = true;
                            }
                        }
                    }
                }
                for s in 0..steps {
                    if s > 0 && s % 4 == 0 {
                        ui.add_space(3.0);
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(17.0, 24.0), Sense::click());
                    let start = s as i64 * STEP;
                    let active = on_step[s];
                    let other_note = off_grid[s];
                    let group_dark = (s / 4) % 2 == 1;
                    let bar_start = s % steps_per_bar == 0;
                    let base = if group_dark { Color32::from_rgb(52, 46, 46) } else { Color32::from_rgb(62, 66, 74) };
                    let fill = if active { color } else { base };
                    ui.painter().rect_filled(r, 3.0, fill);
                    if other_note {
                        ui.painter().circle_filled(r.center(), 2.5, color);
                    }
                    if bar_start {
                        ui.painter().line_segment([r.left_top() - vec2(2.0, 0.0), r.left_bottom() - vec2(2.0, 0.0)], Stroke::new(1.0, TEXT_DIM));
                    }
                    if play_step == Some(s) {
                        ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, Color32::from_rgb(120, 230, 140)));
                    }
                    // click toggles; dragging paints the same state
                    let hovered_down = pointer_down && pointer.map(|p| r.contains(p)).unwrap_or(false);
                    // press event (not "is down"): fast clicks press+release within one frame
                    let pressed_here = ui.input(|i| {
                        i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } if r.contains(*pos)))
                    }) && ui.is_rect_visible(r)
                        && ui.clip_rect().contains(r.center());
                    let want = if pressed_here && paint.is_none() {
                        let v = !active;
                        paint = Some(v);
                        Some(v)
                    } else if hovered_down {
                        paint
                    } else {
                        None
                    };
                    if resp.secondary_clicked() && active {
                        if let Some(p) = app.project.pattern_mut(pid) {
                            p.notes.retain(|n| !(n.channel == cid && n.start == start));
                        }
                    }
                    if let Some(v) = want {
                        if v != active {
                            if let Some(p) = app.project.pattern_mut(pid) {
                                if v {
                                    p.notes.push(Note { channel: cid, key: root, start, len: STEP, velocity: 0.8 });
                                } else {
                                    p.notes.retain(|n| !(n.channel == cid && n.start == start));
                                }
                            }
                            if v {
                                actions.push(RowAction::Preview(ci));
                            }
                        }
                    }
                }
            });
            if selected {
                ui.painter().rect_stroke(row.response.rect.expand(1.0), 3.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.5)));
            }
        }
        ui.add_space(8.0);
        ui.weak("Tipp: Samples aus der Library hierher ziehen = neuer Sampler-Kanal. Klick auf Steps setzt Noten, Ziehen malt, Rechtsklick löscht.");
    });
    ui.data_mut(|d| d.insert_temp(paint_id, paint));

    // drop on empty rack area → new channel
    let rack_resp = ui.interact(scroll.inner_rect, egui::Id::new("rack_drop"), Sense::hover());
    if let Some(drop) = rack_resp.dnd_release_payload::<SampleDrag>() {
        app.add_sampler_channel(&drop.path);
        app.set_status("Sampler-Kanal hinzugefügt");
    }

    for a in actions {
        match a {
            RowAction::Select(i) => app.selected_channel = i,
            RowAction::OpenPiano(i) => {
                app.selected_channel = i;
                app.view = View::PianoRoll;
            }
            RowAction::OpenInstrument(i) => {
                app.selected_channel = i;
                app.view = View::Instrument;
            }
            RowAction::Delete(i) => {
                app.project.remove_channel(i);
                app.selected_channel = app.selected_channel.min(app.project.channels.len().saturating_sub(1));
            }
            RowAction::Clone(i) => {
                let mut c = app.project.channels[i].clone();
                c.id = app.project.alloc_id();
                c.name = format!("{} 2", c.name);
                app.project.channels.insert(i + 1, c);
                app.selected_channel = i + 1;
            }
            RowAction::MoveUp(i) => {
                if i > 0 {
                    app.project.channels.swap(i, i - 1);
                    app.selected_channel = i - 1;
                }
            }
            RowAction::MoveDown(i) => {
                if i + 1 < app.project.channels.len() {
                    app.project.channels.swap(i, i + 1);
                    app.selected_channel = i + 1;
                }
            }
            RowAction::Solo(i, add) => {
                let was = app.project.channels[i].solo;
                let others = app.project.channels.iter().enumerate().any(|(j, c)| j != i && c.solo);
                if !add {
                    for c in app.project.channels.iter_mut() {
                        c.solo = false;
                    }
                }
                // plain click on the only solo channel turns solo off; otherwise it becomes (one of) the solo(s)
                app.project.channels[i].solo = if add { !was } else { !was || others };
            }
            RowAction::Preview(i) => {
                let key = app.project.channels[i].instrument.default_key();
                // short audition: on now, off next frame is too short for synths; use a held note released on pointer up
                app.note_on(i, key, 0.8);
                app.piano.pending_off.push((i, key));
            }
            RowAction::ReplaceSample(i, p) => {
                if let Instrument::Sampler(s) = &mut app.project.channels[i].instrument {
                    s.sample = Some(p.clone());
                    app.project.channels[i].name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    app.cache.get_raw(&p);
                } else {
                    app.add_sampler_channel(&p);
                }
            }
        }
    }
    crate::ui::piano_roll::release_pending(app, ui.ctx());
}

fn knob_small(ui: &mut Ui, v: &mut f32, min: f32, max: f32, def: f32, label: &str, fmt: &dyn Fn(f32) -> String) {
    // compact horizontal drag knob for rack rows
    let (rect, mut resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click_and_drag());
    if resp.dragged() {
        let d = -resp.drag_delta().y * (max - min) * 0.006;
        *v = (*v + d).clamp(min, max);
        resp.mark_changed();
    }
    if resp.double_clicked() {
        *v = def;
    }
    let p = ui.painter();
    let c = rect.center();
    p.circle_filled(c, 10.0, BG_DARK);
    let n = (*v - min) / (max - min);
    let a = std::f32::consts::PI * (0.75 + 1.5 * n);
    p.line_segment([c, c + vec2(a.cos(), a.sin()) * 8.0], Stroke::new(2.0, ACCENT));
    resp.on_hover_text(format!("{label}: {}", fmt(*v)));
}
