use crate::app::TStudio;
use crate::ui::widgets::{self, draw_meter, knob, small_toggle, ACCENT, BG_DARK, BG_LIGHT, TEXT_DIM};
use egui::{vec2, Color32, RichText, Sense, Stroke, Ui};
use tstudio_dsp::filter::FilterMode;
use tstudio_dsp::fx::{DistMode, EffectKind, EffectParams};
use tstudio_project::{FxSlot, MixerSend, MAX_FX_SLOTS};

mod fx_params;
mod spectrum;
pub use fx_params::{filter_mode_combo, fx_params};

#[derive(Default)]
pub struct MixerState {
    pub selected: usize,
    /// spectrum of the selected insert is shown (and analysed by the engine)
    pub spectrum_open: bool,
    /// spectrum of the master is shown
    pub master_spectrum_open: bool,
}

pub use spectrum::sync_watch as sync_spectrum;

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.heading("Mixer");
        if ui.button("+ Insert").clicked() {
            app.mixer_ui.selected = app.project.add_mixer_track(None);
        }
        if app.project.mixer.iter().any(|t| t.solo) && ui.button("Solos aufheben").clicked() {
            for t in app.project.mixer.iter_mut() {
                t.solo = false;
            }
        }
        ui.weak("Kanäle routen über „→“ im Channel Rack. Strip anklicken → Effekte, Sends & Bus rechts. Rechtsklick → Umbenennen, Einfügen, Verschieben, Löschen.");
    });
    ui.separator();
    let sel = app.mixer_ui.selected.min(app.project.mixer.len() - 1);
    app.mixer_ui.selected = sel;

    egui::SidePanel::right("fx_panel").resizable(true).default_width(380.0).show_inside(ui, |ui| fx_chain(app, ui, sel));

    egui::ScrollArea::horizontal().auto_shrink([false; 2]).id_salt("mixer_strips").show(ui, |ui| {
        ui.horizontal(|ui| {
            let n = app.project.mixer.len();
            for t in 0..n {
                // a strip menu may delete inserts this frame
                if t >= app.project.mixer.len() {
                    break;
                }
                strip(app, ui, t);
            }
        });
    });
}

fn strip(app: &mut TStudio, ui: &mut Ui, t: usize) {
    let mut auto_req: Option<String> = None;
    let selected = app.mixer_ui.selected == t;
    let (l, r) = app.meters.get(t).copied().unwrap_or((0.0, 0.0));
    let users: Vec<String> = app.project.channels.iter().filter(|c| c.mixer_track == t).map(|c| c.name.clone()).collect();
    // click/right-click area behind the strip (last frame's rect), registered before the
    // contents so the M/S buttons, fader and knob on top still get their own clicks
    let bg_id = ui.id().with(("strip", t));
    let bg_rect: Option<egui::Rect> = ui.data(|d| d.get_temp(bg_id));
    let bg_resp = bg_rect.map(|r| ui.interact(r, bg_id, Sense::click()));
    let frame = egui::Frame::default()
        .fill(if selected { BG_LIGHT } else { BG_DARK })
        .stroke(Stroke::new(1.0, if selected { ACCENT } else { Color32::from_gray(50) }))
        .rounding(4.0)
        .inner_margin(egui::Margin::same(4.0));
    let resp = frame
        .show(ui, |ui| {
            ui.set_width(70.0);
            ui.vertical_centered(|ui| {
                let tr = &mut app.project.mixer[t];
                let label = if t == 0 { "MASTER".to_string() } else { format!("{t}") };
                ui.label(RichText::new(label).small().color(if t == 0 { ACCENT } else { TEXT_DIM }));
                let short: String = tr.name.chars().take(10).collect();
                ui.label(RichText::new(short).size(11.0).strong());
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 150.0), Sense::hover());
                    draw_meter(ui.painter(), rect, l, r);
                    // fader in dB
                    let mut db = tstudio_dsp::gain_to_db(tr.volume).max(-60.0);
                    let resp = ui.add(egui::Slider::new(&mut db, -60.0..=6.0).vertical().show_value(false));
                    if resp.changed() {
                        tr.volume = if db <= -59.9 { 0.0 } else { tstudio_dsp::db_to_gain(db) };
                    }
                    if resp.double_clicked() {
                        tr.volume = if t == 0 { 1.0 } else { 0.8 };
                    }
                    resp.context_menu(|ui| crate::ui::playlist::automation_menu_item(ui, format!("mixer.{t}.volume"), &mut auto_req));
                });
                ui.label(RichText::new(widgets::fmt_gain(tr.volume)).small().monospace());
                knob(ui, &mut tr.pan, -1.0, 1.0, 0.0, "Pan", false, &widgets::fmt_pan)
                    .context_menu(|ui| crate::ui::playlist::automation_menu_item(ui, format!("mixer.{t}.pan"), &mut auto_req));
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    small_toggle(ui, &mut tr.muted, "M", Color32::from_rgb(230, 110, 100), "Stumm (Mute)");
                    if t > 0 {
                        small_toggle(ui, &mut tr.solo, "S", Color32::from_rgb(240, 200, 70), "Solo – nur Spuren mit S sind zu hören");
                    }
                });
                let fx_n = tr.fx.len();
                ui.label(RichText::new(if fx_n > 0 { format!("FX {fx_n}") } else { "–".into() }).small().color(TEXT_DIM));
                let mut route = String::new();
                if tr.output > 0 {
                    route.push_str(&format!("→{} ", tr.output));
                }
                for s in &tr.sends {
                    route.push_str(&format!("⤳{} ", s.target));
                }
                if !route.is_empty() {
                    ui.label(RichText::new(route.trim_end()).small().color(ACCENT));
                }
            });
        })
        .response;
    ui.data_mut(|d| d.insert_temp(bg_id, resp.rect));
    if let Some(path) = auto_req {
        crate::ui::playlist::create_automation(app, &path);
        return;
    }
    let Some(resp) = bg_resp else { return };
    if resp.clicked() {
        app.mixer_ui.selected = t;
    }
    let resp = resp.context_menu(|ui| strip_menu(app, ui, t)).map(|m| m.response).unwrap_or(resp);
    let tip = if users.is_empty() { "keine Kanäle".to_string() } else { users.join(", ") };
    resp.on_hover_text(tip);
}

/// Right-click menu of a mixer strip.
fn strip_menu(app: &mut TStudio, ui: &mut Ui, t: usize) {
    if t >= app.project.mixer.len() {
        ui.close_menu();
        return;
    }
    app.mixer_ui.selected = t;
    if t > 0 {
        ui.horizontal(|ui| {
            ui.label("Name");
            let r = ui.text_edit_singleline(&mut app.project.mixer[t].name);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ui.close_menu();
            }
        });
    }
    ui.horizontal(|ui| {
        let tr = &mut app.project.mixer[t];
        ui.toggle_value(&mut tr.muted, "Mute");
        if t > 0 {
            ui.toggle_value(&mut tr.solo, "Solo");
        }
    });
    if t > 0 && ui.button("Nur diese Spur solo").clicked() {
        for (i, x) in app.project.mixer.iter_mut().enumerate() {
            x.solo = i == t;
        }
        ui.close_menu();
    }
    ui.separator();
    if t > 0 && ui.button("Insert davor einfügen").clicked() {
        app.mixer_ui.selected = app.project.insert_mixer_track(t, None);
        ui.close_menu();
    }
    if ui.button("Insert danach einfügen").clicked() {
        app.mixer_ui.selected = app.project.insert_mixer_track(t + 1, None);
        ui.close_menu();
    }
    if t > 0 {
        let n = app.project.mixer.len();
        ui.horizontal(|ui| {
            if ui.add_enabled(t > 1, egui::Button::new("◀ nach links")).clicked() && app.project.move_mixer_track(t, t - 1).is_ok() {
                app.mixer_ui.selected = t - 1;
                ui.close_menu();
            }
            if ui.add_enabled(t + 1 < n, egui::Button::new("nach rechts ▶")).clicked() && app.project.move_mixer_track(t, t + 1).is_ok() {
                app.mixer_ui.selected = t + 1;
                ui.close_menu();
            }
        });
        ui.separator();
        let users = app.project.channels.iter().filter(|c| c.mixer_track == t).count();
        let label = if users > 0 { format!("🗑 Insert löschen ({users} Kanäle → Master)") } else { "🗑 Insert löschen".to_string() };
        if ui.button(RichText::new(label).color(Color32::from_rgb(240, 120, 110))).clicked() {
            if app.project.delete_mixer_track(t).is_ok() {
                app.mixer_ui.selected = t.min(app.project.mixer.len() - 1);
            }
            ui.close_menu();
        }
    }
}

fn fx_chain(app: &mut TStudio, ui: &mut Ui, t: usize) {
    let names: Vec<String> = app.project.mixer.iter().enumerate().map(|(i, m)| if i == 0 { "Master".to_string() } else { format!("{i} {}", m.name) }).collect();
    let n_tracks = names.len();
    ui.horizontal(|ui| {
        ui.label(RichText::new(if t == 0 { "Master".to_string() } else { format!("Insert {t}") }).strong());
        ui.text_edit_singleline(&mut app.project.mixer[t].name);
    });
    spectrum::panels(app, ui, t);
    let tr = &mut app.project.mixer[t];
    if t > 0 {
        // routing: output bus + post-fader sends (only to higher inserts, like a signal flow from left to right)
        egui::CollapsingHeader::new(RichText::new("Routing: Ausgang & Sends").strong()).id_salt("routing").default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Ausgang");
                egui::ComboBox::from_id_salt("route_out").selected_text(names[tr.output.min(n_tracks - 1)].clone()).width(160.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut tr.output, 0, "Master");
                    for (i, n) in names.iter().enumerate().skip(t + 1) {
                        ui.selectable_value(&mut tr.output, i, n);
                    }
                })
                .response
                .on_hover_text("Spur auf einen Bus legen (z.B. Kick + Rumble → Bus mit Kompressor)");
            });
            let mut remove = None;
            for (si, s) in tr.sends.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("Send → {}", names.get(s.target).cloned().unwrap_or_default()));
                    let mut db = tstudio_dsp::gain_to_db(s.amount).max(-60.0);
                    if ui.add(egui::Slider::new(&mut db, -60.0..=6.0).suffix(" dB").fixed_decimals(1)).changed() {
                        s.amount = tstudio_dsp::db_to_gain(db);
                    }
                    if ui.small_button("✕").clicked() {
                        remove = Some(si);
                    }
                });
            }
            if let Some(i) = remove {
                tr.sends.remove(i);
            }
            if t + 1 < n_tracks {
                ui.menu_button("+ Send", |ui| {
                    for (i, n) in names.iter().enumerate().skip(t + 1) {
                        if !tr.sends.iter().any(|s| s.target == i) && ui.button(n).clicked() {
                            tr.sends.push(MixerSend { target: i, amount: 0.5 });
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Post-Fader-Send, z.B. Kick → Hall-Bus (Rumble) oder Snare → Delay");
            }
        });
    }
    ui.separator();
    let mut remove = None;
    let mut swap = None;
    let mut auto_req: Option<String> = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("fx_scroll").show(ui, |ui| {
        let n = tr.fx.len();
        for i in 0..n {
            let slot = &mut tr.fx[i];
            let name = slot.params.kind().name();
            egui::Frame::default().fill(BG_DARK).rounding(4.0).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut slot.enabled, RichText::new(format!("{} {}", i + 1, name)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Entfernen").clicked() {
                            remove = Some(i);
                        }
                        if i + 1 < n && ui.small_button("▼").clicked() {
                            swap = Some((i, i + 1));
                        }
                        if i > 0 && ui.small_button("▲").clicked() {
                            swap = Some((i, i - 1));
                        }
                        ui.menu_button("〰", |ui| {
                            ui.label(RichText::new("Automation erstellen:").small());
                            for f in slot.params.field_names() {
                                if ui.button(*f).clicked() {
                                    auto_req = Some(format!("mixer.{t}.fx.{i}.{f}"));
                                    ui.close_menu();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Automation für einen Parameter dieses Effekts anlegen");
                    });
                });
                ui.add_enabled_ui(slot.enabled, |ui| fx_params(ui, &mut slot.params, &names));
            });
            ui.add_space(4.0);
        }
        if n < MAX_FX_SLOTS {
            ui.menu_button("+ Effekt hinzufügen", |ui| {
                for k in EffectKind::ALL {
                    if ui.button(k.name()).clicked() {
                        tr.fx.push(FxSlot::new(k));
                        ui.close_menu();
                    }
                }
            });
        } else {
            ui.weak("Maximal 8 Effekte pro Spur");
        }
    });
    if let Some(i) = remove {
        tr.fx.remove(i);
    }
    if let Some((a, b)) = swap {
        tr.fx.swap(a, b);
    }
    if let Some(path) = auto_req {
        crate::ui::playlist::create_automation(app, &path);
    }
}

