//! TSynth editor in the instrument tab: preset browser with categories, macros and five
//! pages (sound, kick, filter, modulation, FX / voicing).
//!
//! Layout: every page is a grid of equally sized cards. The number of columns follows the
//! available width, all cards of a row share width and height, displays stretch to the card.

use crate::app::TStudio;
use crate::ui::mixer::filter_mode_combo;
use crate::ui::widgets::{self, knob, ACCENT, BG_DARK, BG_LIGHT, BG_PANEL, TEXT_DIM};
use egui::{pos2, vec2, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Ui};
use tstudio_dsp::env::{Adsr, AdsrParams};
use tstudio_dsp::filter::FilterType;
use tstudio_dsp::fx::DistMode;
use tstudio_dsp::osc::Waveform;
use tstudio_dsp::synth::*;
use tstudio_dsp::wavetable::WtKind;
use tstudio_project::Instrument;

mod helpers;
mod pages;
#[cfg(test)]
mod layout_tests;

use helpers::*;
use pages::*;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Sound,
    Kick,
    Filter,
    Mod,
    Fx,
}

impl Page {
    const ALL: [(Page, &'static str); 5] = [(Page::Sound, "Oszillatoren"), (Page::Kick, "Kick"), (Page::Filter, "Filter"), (Page::Mod, "Modulation"), (Page::Fx, "FX / Voicing")];
}

pub struct SynthUiState {
    pub page: Page,
    pub category: &'static str,
    pub preset_name: String,
    /// name of the last loaded preset (shown in the header)
    pub loaded: String,
}

impl Default for SynthUiState {
    fn default() -> Self {
        Self { page: Page::Sound, category: "Kick", preset_name: String::new(), loaded: String::new() }
    }
}

const OSC_COL: [Color32; 3] = [Color32::from_rgb(255, 150, 60), Color32::from_rgb(110, 190, 250), Color32::from_rgb(150, 220, 120)];
const MOD_COL: Color32 = Color32::from_rgb(200, 140, 255);
const FX_COL: Color32 = Color32::from_rgb(240, 110, 120);
const CLICK_COL: Color32 = Color32::from_rgb(250, 210, 90);
const CURVE_BG: Color32 = Color32::from_rgb(20, 22, 26);
const CARD_EDGE: Color32 = Color32::from_rgb(44, 49, 57);
const TITLE: Color32 = Color32::from_rgb(226, 230, 236);
const GAP: f32 = 8.0;
const PAD: f32 = 10.0;

pub fn show(app: &mut TStudio, ui: &mut Ui, ci: usize) {
    let presets_dir = app.paths.presets_dir();
    let mut status = None;
    let st = &mut app.instrument_ui.synth;
    let ch = &mut app.project.channels[ci];
    let Instrument::Synth(p) = &mut ch.instrument else { return };

    header(ui, st, p, &presets_dir, &mut status);
    page_tabs(ui, st, p);
    ui.add_space(GAP);
    match st.page {
        Page::Sound => sound_page(ui, p),
        Page::Kick => kick_page(ui, p),
        Page::Filter => filter_page(ui, p),
        Page::Mod => mod_page(ui, p),
        Page::Fx => fx_page(ui, p),
    }
    if let Some(s) = status {
        app.set_status(s);
    }
}

fn page_tabs(ui: &mut Ui, st: &mut SynthUiState, p: &SynthParams) {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().line_segment([pos2(bar.left(), bar.bottom() - 0.5), pos2(bar.right(), bar.bottom() - 0.5)], Stroke::new(1.0f32, CARD_EDGE));
    let mut x = bar.left() + 2.0;
    for (pg, name) in Page::ALL {
        let sel = st.page == pg;
        let galley = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(14.0), Color32::WHITE);
        let r = Rect::from_min_size(pos2(x, bar.top()), vec2(galley.size().x + 24.0, bar.height()));
        let resp = ui.interact(r, ui.id().with(("tsynth_page", name)), Sense::click());
        if resp.clicked() {
            st.page = pg;
        }
        let col = if sel { TITLE } else if resp.hovered() { Color32::from_gray(200) } else { TEXT_DIM };
        if sel || resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(0.0, 2.0)), egui::Rounding { nw: 5.0, ne: 5.0, sw: 0.0, se: 0.0 }, if sel { BG_DARK } else { BG_PANEL });
        }
        ui.painter().galley(pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0), galley, col);
        if sel {
            ui.painter().rect_filled(Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 2.5), pos2(r.right() - 6.0, r.bottom())), 1.0, ACCENT);
        }
        // little status dot: kick engine on
        if pg == Page::Kick && p.kick.enabled {
            ui.painter().circle_filled(pos2(r.right() - 7.0, r.top() + 9.0), 3.0, ACCENT);
        }
        x = r.right() + 2.0;
    }
    // voicing summary on the right
    let info = format!(
        "{} · {}",
        if p.mono { if p.legato { "Mono Legato" } else { "Mono" } } else { "Poly" },
        if p.kick.enabled { "Kick-Engine" } else { "Synth" }
    );
    ui.painter().text(pos2(bar.right() - 6.0, bar.center().y), Align2::RIGHT_CENTER, info, FontId::proportional(11.0), TEXT_DIM);
}

fn header(ui: &mut Ui, st: &mut SynthUiState, p: &mut SynthParams, presets_dir: &std::path::Path, status: &mut Option<String>) {
    wide_card(ui, |ui| {
        let avail = ui.available_width();
        let macro_w = 250.0;
        let side_by_side = avail > 640.0;
        let left_w = if side_by_side { avail - macro_w - GAP * 2.0 } else { avail };
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                head(ui, None, "Presets", ACCENT, |ui| {
                    if !st.loaded.is_empty() {
                        ui.label(RichText::new(&st.loaded).strong().color(ACCENT));
                        small_label(ui, "geladen:");
                    }
                });
                // category tabs
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let mut tab = |ui: &mut Ui, cat: &'static str, label: &str| {
                        let sel = st.category == cat;
                        let text = RichText::new(label).color(if sel { Color32::BLACK } else { TEXT_DIM });
                        let b = egui::Button::new(text).fill(if sel { ACCENT } else { BG_PANEL }).rounding(10.0).min_size(vec2(0.0, 20.0));
                        if ui.add(b).clicked() {
                            st.category = cat;
                        }
                    };
                    tab(ui, "Init", "Init");
                    for c in PRESET_CATEGORIES {
                        tab(ui, c, c);
                    }
                    tab(ui, "User", "★ Eigene");
                });
                ui.add_space(4.0);
                // presets of the category
                egui::Frame::default().fill(CURVE_BG).rounding(4.0).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
                    ui.set_width(left_w - 14.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        if st.category == "User" {
                            let list = super::instrument::user_presets(presets_dir);
                            if list.is_empty() {
                                ui.weak("Noch keine eigenen Presets – unten speichern.");
                            }
                            for (name, path) in list {
                                if ui.selectable_label(st.loaded == name, format!("★ {name}")).clicked() {
                                    match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| ron::from_str::<SynthParams>(&s).map_err(|e| e.to_string())) {
                                        Ok(sp) => {
                                            *p = sp;
                                            st.loaded = name.clone();
                                        }
                                        Err(e) => *status = Some(format!("Preset {name}: {e}")),
                                    }
                                }
                            }
                        } else {
                            for (cat, name, preset) in SynthParams::presets_by_category() {
                                if cat == st.category && ui.selectable_label(st.loaded == name, name).clicked() {
                                    *p = preset;
                                    st.loaded = name.to_string();
                                }
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.preset_name).hint_text("Name für eigenes Preset").desired_width(180.0));
                    if ui.button("💾 Speichern").clicked() && !st.preset_name.trim().is_empty() {
                        let _ = std::fs::create_dir_all(presets_dir);
                        let file = tstudio_library::unique_path(presets_dir, st.preset_name.trim(), "ron");
                        match ron::ser::to_string_pretty(p, ron::ser::PrettyConfig::default()) {
                            Ok(s) => {
                                let _ = std::fs::write(&file, s);
                                *status = Some(format!("Preset gespeichert: {}", file.display()));
                            }
                            Err(e) => *status = Some(e.to_string()),
                        }
                    }
                });
            });
            if side_by_side {
                ui.add_space(GAP);
                ui.vertical(|ui| {
                    ui.set_width(macro_w);
                    macro_panel(ui, p);
                });
            }
        });
        if !side_by_side {
            ui.add_space(6.0);
            macro_panel(ui, p);
        }
    });
}

fn macro_panel(ui: &mut Ui, p: &mut SynthParams) {
    egui::Frame::default().fill(CURVE_BG).rounding(5.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        head(ui, None, "Makros", MOD_COL, |_| {});
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for i in 0..4 {
                knob(ui, p.macro_mut(i), 0.0, 1.0, 0.0, &format!("M{}", i + 1), false, &widgets::fmt_pct).on_hover_text("Makro: in der Mod-Matrix als Quelle „Makro N“ zuweisen, automatisierbar");
            }
        });
        let used: Vec<String> = (0..4)
            .filter(|&i| p.mod_matrix.iter().any(|s| s.active() && s.source == [ModSource::Macro1, ModSource::Macro2, ModSource::Macro3, ModSource::Macro4][i]))
            .map(|i| format!("M{}", i + 1))
            .collect();
        small_label(ui, &if used.is_empty() { "keine Makros zugewiesen".to_string() } else { format!("zugewiesen: {}", used.join(", ")) });
    });
}
