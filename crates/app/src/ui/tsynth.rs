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

// ------------------------------------------------------------------ layout helpers

/// Lays out `n` cards in rows. Columns = as many as fit with at least `min_w` each (max `max_cols`).
/// Cards of one row get the same width and (from the previous frame) the same height.
fn card_grid(ui: &mut Ui, id: &str, min_w: f32, max_cols: usize, n: usize, mut add: impl FnMut(&mut Ui, usize)) {
    let avail = ui.available_width();
    let cols = (((avail + GAP) / (min_w + GAP)).floor() as usize).clamp(1, max_cols.min(n).max(1));
    let w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
    let mut start = 0;
    while start < n {
        let end = (start + cols).min(n);
        let hid = ui.id().with(("card_row", id, start, cols));
        let target: f32 = ui.data(|d| d.get_temp(hid)).unwrap_or(0.0);
        let mut max_h = 0.0f32;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for i in start..end {
                ui.vertical(|ui| {
                    ui.set_width(w);
                    egui::Frame::default().fill(BG_DARK).stroke(Stroke::new(1.0f32, CARD_EDGE)).rounding(6.0).inner_margin(egui::Margin::same(PAD)).show(ui, |ui| {
                        ui.set_width(w - 2.0 * PAD - 2.0);
                        let top = ui.min_rect().top();
                        add(ui, i);
                        let h = ui.min_rect().bottom() - top;
                        max_h = max_h.max(h);
                        if target > h {
                            ui.add_space(target - h);
                        }
                    });
                });
            }
        });
        if (max_h - target).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(hid, max_h));
            ui.ctx().request_repaint();
        }
        ui.add_space(GAP);
        start = end;
    }
}

/// Full-width card.
fn wide_card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    egui::Frame::default().fill(BG_DARK).stroke(Stroke::new(1.0f32, CARD_EDGE)).rounding(6.0).inner_margin(egui::Margin::same(PAD)).show(ui, |ui| {
        ui.set_width(w - 2.0 * PAD - 2.0);
        add(ui);
    });
    ui.add_space(GAP);
}

/// Card title: colour bar, optional on/off checkbox, title, extra widgets right-aligned
/// (added right to left). Returns whether the card's content is active.
fn head(ui: &mut Ui, on: Option<&mut bool>, title: &str, col: Color32, extra: impl FnOnce(&mut Ui)) -> bool {
    let mut active = true;
    ui.horizontal(|ui| {
        let lit = on.as_ref().map_or(true, |b| **b);
        let (r, _) = ui.allocate_exact_size(vec2(3.0, 16.0), Sense::hover());
        ui.painter().rect_filled(r, 1.5, if lit { col } else { BG_LIGHT });
        let text = RichText::new(title).strong().color(if lit { TITLE } else { TEXT_DIM });
        match on {
            Some(b) => {
                ui.checkbox(b, text);
                active = *b;
            }
            None => {
                ui.label(text);
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), extra);
    });
    ui.add_space(4.0);
    active
}

fn knobs(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        add(ui)
    });
}

/// Framed chip inside a `horizontal_wrapped` row; frames do not wrap by themselves, so the row
/// is broken manually when the chip would not fit.
fn chip(ui: &mut Ui, text: &str, fill: Color32, color: Color32) {
    let w = ui.painter().layout_no_wrap(text.to_string(), egui::TextStyle::Small.resolve(ui.style()), color).size().x + 16.0;
    if ui.max_rect().right() - ui.cursor().left() < w {
        ui.end_row();
    }
    egui::Frame::default().fill(fill).stroke(Stroke::new(1.0f32, CARD_EDGE)).rounding(8.0).inner_margin(egui::Margin::symmetric(7.0, 2.0)).show(ui, |ui| {
        ui.label(RichText::new(text).small().color(color));
    });
}

fn small_label(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).small().color(TEXT_DIM));
}

/// Draws a polyline of `ys` (-1..1 when `bipolar`, else 0..1) across the full width.
fn curve(ui: &mut Ui, height: f32, ys: &[f32], bipolar: bool, color: Color32) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, CURVE_BG);
    let inner = rect.shrink2(vec2(4.0, 5.0));
    for k in 1..4 {
        let x = inner.left() + inner.width() * k as f32 / 4.0;
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0f32, Color32::from_rgb(30, 33, 38)));
    }
    if bipolar {
        p.line_segment([pos2(inner.left(), inner.center().y), pos2(inner.right(), inner.center().y)], Stroke::new(1.0f32, BG_LIGHT));
    }
    let n = ys.len().max(2);
    let pts: Vec<egui::Pos2> = ys
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let x = inner.left() + inner.width() * i as f32 / (n - 1) as f32;
            let v = if bipolar { 0.5 - 0.5 * y.clamp(-1.2, 1.2) } else { 1.0 - y.clamp(0.0, 1.0) };
            pos2(x, inner.top() + inner.height() * v)
        })
        .collect();
    // soft fill under the curve
    let base = if bipolar { inner.center().y } else { inner.bottom() };
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28);
    for w in pts.windows(2) {
        p.add(egui::Shape::convex_polygon(vec![w[0], w[1], pos2(w[1].x, base), pos2(w[0].x, base)], fill, Stroke::NONE));
    }
    p.add(egui::Shape::line(pts, Stroke::new(1.8f32, color)));
    (rect, resp)
}

fn env_curve(e: &AdsrParams, n: usize) -> Vec<f32> {
    let total = (e.attack + e.hold + e.decay * 0.8 + e.release * 0.8 + 0.15).max(0.02);
    let sr = n as f32 / total;
    let mut a = Adsr::new(sr);
    a.set_params(e);
    a.trigger();
    let gate = ((e.attack + e.hold + e.decay * 0.6 + 0.1) * sr) as usize;
    (0..n)
        .map(|i| {
            if i == gate {
                a.release();
            }
            a.next()
        })
        .collect()
}

/// Magnitude response (display units 0..1 = -36..+18 dB) of a filter slot over 20 Hz..20 kHz,
/// measured from its impulse response.
fn filter_response(kind: FilterType, mode: tstudio_dsp::filter::FilterMode, cutoff: f32, res: f32, n_pts: usize) -> Vec<f32> {
    const SR: f32 = 48000.0;
    const N: usize = 4096; // long enough for the bass region
    let mut f = tstudio_dsp::filter::SynthFilter::new(4096);
    f.set(kind, cutoff, res, SR);
    let amp = 0.02; // small impulse keeps the saturating ladder linear
    let h: Vec<f32> = (0..N).map(|i| f.process(if i == 0 { amp } else { 0.0 }, kind, mode) / amp).collect();
    (0..n_pts)
        .map(|k| {
            let hz = 20.0 * 1000f32.powf(k as f32 / (n_pts - 1) as f32);
            let w = std::f32::consts::TAU * hz / SR;
            let (sw, cw) = w.sin_cos();
            let (mut c, mut s, mut re, mut im) = (1.0f32, 0.0f32, 0.0f32, 0.0f32);
            for &x in &h {
                re += x * c;
                im -= x * s;
                let c2 = c * cw - s * sw;
                s = s * cw + c * sw;
                c = c2;
            }
            let db = 10.0 * (re * re + im * im).max(1e-12).log10();
            ((db + 36.0) / 54.0).clamp(0.0, 1.0)
        })
        .collect()
}

/// x position (0..1) of a frequency on the 20 Hz..20 kHz log axis
fn freq_x(hz: f32) -> f32 {
    ((hz.max(20.0) / 20.0).log10() / 3.0).clamp(0.0, 1.0)
}

fn filter_display(ui: &mut Ui, kind: FilterType, mode: tstudio_dsp::filter::FilterMode, cutoff: f32, res: f32, color: Color32) {
    let ys = filter_response(kind, mode, cutoff, res, 110);
    let (rect, _) = curve(ui, 70.0, &ys, false, color);
    let p = ui.painter_at(rect);
    let inner = rect.shrink2(vec2(4.0, 5.0));
    // 0 dB line and decade labels
    let y0 = inner.top() + inner.height() * (1.0 - 36.0 / 54.0);
    p.line_segment([pos2(inner.left(), y0), pos2(inner.right(), y0)], Stroke::new(1.0f32, Color32::from_rgb(40, 44, 50)));
    for (hz, l) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        let x = inner.left() + inner.width() * freq_x(hz);
        p.text(pos2(x + 2.0, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, l, FontId::proportional(9.0), Color32::from_rgb(90, 96, 106));
    }
    let x = inner.left() + inner.width() * freq_x(cutoff);
    p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0f32, color.gamma_multiply(0.5)));
    p.text(pos2(rect.right() - 4.0, rect.top() + 3.0), Align2::RIGHT_TOP, widgets::fmt_hz(cutoff), FontId::proportional(10.0), TEXT_DIM);
}

/// Saturation transfer curve (input -1..1 -> output).
fn drive_display(ui: &mut Ui, mode: DriveMode, drive_db: f32, color: Color32) {
    let ys: Vec<f32> = (0..120).map(|i| mode.transfer(-1.0 + 2.0 * i as f32 / 119.0, drive_db)).collect();
    let (rect, _) = curve(ui, 56.0, &ys, true, color);
    ui.painter_at(rect).text(rect.right_top() + vec2(-4.0, 3.0), Align2::RIGHT_TOP, format!("{} · {}", mode.name(), widgets::fmt_db(drive_db)), FontId::proportional(10.0), TEXT_DIM);
}

fn env_card(ui: &mut Ui, title: &str, e: &mut AdsrParams, color: Color32) {
    head(ui, None, title, color, |_| {});
    curve(ui, 64.0, &env_curve(e, 220), false, color);
    ui.add_space(4.0);
    knobs(ui, |ui| {
        knob(ui, &mut e.attack, 0.0005, 10.0, 0.002, "Attack", true, &widgets::fmt_s);
        knob(ui, &mut e.hold, 0.0, 2.0, 0.0, "Hold", false, &widgets::fmt_s);
        knob(ui, &mut e.decay, 0.001, 10.0, 0.2, "Decay", true, &widgets::fmt_s);
        knob(ui, &mut e.sustain, 0.0, 1.0, 1.0, "Sustain", false, &widgets::fmt_pct);
        knob(ui, &mut e.release, 0.001, 10.0, 0.05, "Release", true, &widgets::fmt_s);
        knob(ui, &mut e.attack_curve, -1.0, 1.0, 0.0, "A-Kurve", false, &|v| format!("{v:+.2}")).on_hover_text("> 0: schneller Anstieg, < 0: langsam");
        knob(ui, &mut e.decay_curve, -1.0, 1.0, 0.0, "D/R-Kurve", false, &|v| format!("{v:+.2}")).on_hover_text("> 0: knackiger (Kicks), < 0: runder");
    });
}

fn combo<T: PartialEq + Copy>(ui: &mut Ui, id: impl std::hash::Hash, v: &mut T, all: &[T], name: fn(T) -> &'static str, width: f32) {
    egui::ComboBox::from_id_salt(id).selected_text(name(*v)).width(width).show_ui(ui, |ui| {
        for x in all {
            ui.selectable_value(v, *x, name(*x));
        }
    });
}

fn note_drag(ui: &mut Ui, v: &mut f32, lo: f32, hi: f32) -> egui::Response {
    ui.add(egui::DragValue::new(v).speed(0.05).range(lo..=hi).custom_formatter(|n, _| {
        let r = n.round();
        let cents = ((n - r) * 100.0).round();
        let name = tstudio_dsp::note_name(r.clamp(0.0, 127.0) as u8);
        if cents.abs() >= 1.0 {
            format!("{name} {cents:+.0}ct")
        } else {
            name
        }
    }))
}

// ------------------------------------------------------------------ entry

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

// ------------------------------------------------------------------ pages

fn osc_card(ui: &mut Ui, i: usize, o: &mut OscParams) {
    let col = OSC_COL[i];
    let on = head(ui, Some(&mut o.enabled), &format!("Osc {}", i + 1), col, |ui| {
        if o.table == WtKind::Basic {
            combo(ui, ("wave", i), &mut o.wave, &Waveform::ALL, Waveform::name, 64.0);
        }
        combo(ui, ("wt", i), &mut o.table, &WtKind::ALL, WtKind::name, 84.0);
    });
    ui.add_enabled_ui(on, |ui| {
        let (rect, resp) = curve(ui, 70.0, &o.preview(200), true, col);
        if o.table != WtKind::Basic {
            // drag horizontally over the display to scan the wavetable
            if resp.dragged() {
                o.position = (o.position + resp.drag_delta().x / rect.width()).clamp(0.0, 1.0);
            }
            let x = rect.left() + 4.0 + (rect.width() - 8.0) * o.position;
            ui.painter().line_segment([pos2(x, rect.bottom() - 7.0), pos2(x, rect.bottom())], Stroke::new(3.0f32, col));
            ui.painter().text(rect.left_top() + vec2(5.0, 3.0), Align2::LEFT_TOP, format!("Pos {:.0} %", o.position * 100.0), FontId::proportional(10.0), TEXT_DIM);
            resp.on_hover_text("Ziehen: Wavetable-Position");
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            small_label(ui, "Warp");
            combo(ui, ("warp", i), &mut o.warp, &WarpMode::ALL, WarpMode::name, 84.0);
            ui.add(egui::DragValue::new(&mut o.octave).range(-4..=4).prefix("Okt "));
            ui.add(egui::DragValue::new(&mut o.semi).range(-12..=12).prefix("St "));
        });
        knobs(ui, |ui| {
            if o.table != WtKind::Basic {
                knob(ui, &mut o.position, 0.0, 1.0, 0.0, "Position", false, &widgets::fmt_pct);
            }
            knob(ui, &mut o.warp_amount, 0.0, 1.0, 0.0, "Warp", false, &widgets::fmt_pct);
            knob(ui, &mut o.fine, -100.0, 100.0, 0.0, "Fine", false, &|v| format!("{v:+.0} ct"));
            knob(ui, &mut o.level, 0.0, 1.0, 0.7, "Level", false, &widgets::fmt_pct);
            knob(ui, &mut o.pan, -1.0, 1.0, 0.0, "Pan", false, &widgets::fmt_pan);
        });
        ui.add_space(2.0);
        small_label(ui, "Unison");
        knobs(ui, |ui| {
            let mut uni = o.unison as f32;
            knob(ui, &mut uni, 1.0, MAX_UNISON as f32, 1.0, "Stimmen", false, &|v| format!("{:.0}", v.round()));
            o.unison = uni.round() as u32;
            knob(ui, &mut o.detune, 0.0, 100.0, 20.0, "Detune", false, &|v| format!("{v:.0} ct"));
            knob(ui, &mut o.blend, 0.0, 1.0, 1.0, "Blend", false, &widgets::fmt_pct).on_hover_text("Lautstärke der äußeren Unison-Stimmen");
            knob(ui, &mut o.spread, 0.0, 1.0, 0.6, "Stereo", false, &widgets::fmt_pct);
            knob(ui, &mut o.rand_phase, 0.0, 1.0, 1.0, "Rnd Phase", false, &widgets::fmt_pct).on_hover_text("0 = jede Note startet gleich (Bässe, Kicks)");
        });
    });
}

fn sub_noise_card(ui: &mut Ui, p: &mut SynthParams) {
    head(ui, None, "Sub", OSC_COL[0], |ui| {
        ui.add(egui::DragValue::new(&mut p.sub_octave).range(-3..=0).prefix("Okt "));
        combo(ui, "subw", &mut p.sub_wave, &Waveform::ALL, Waveform::name, 64.0);
    });
    knobs(ui, |ui| {
        knob(ui, &mut p.sub_level, 0.0, 1.0, 0.0, "Sub", false, &widgets::fmt_pct);
    });
    ui.add_space(6.0);
    head(ui, None, "Noise", TEXT_DIM, |ui| {
        combo(ui, "noise", &mut p.noise_type, &NoiseType::ALL, NoiseType::name, 72.0);
    });
    knobs(ui, |ui| {
        knob(ui, &mut p.noise_level, 0.0, 1.0, 0.0, "Noise", false, &widgets::fmt_pct);
        knob(ui, &mut p.noise_tone, 200.0, 20000.0, 20000.0, "Tone", true, &widgets::fmt_hz);
    });
}

fn sound_page(ui: &mut Ui, p: &mut SynthParams) {
    card_grid(ui, "osc", 300.0, 4, NUM_OSC + 1, |ui, i| {
        if i < NUM_OSC {
            osc_card(ui, i, p.osc_mut(i));
        } else {
            sub_noise_card(ui, p);
        }
    });
}

fn kick_page(ui: &mut Ui, p: &mut SynthParams) {
    if !p.kick.enabled {
        wide_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut p.kick.enabled, RichText::new("Kick-Engine einschalten").strong().color(ACCENT));
                small_label(ui, "Pitch-Sweep, Click, Drive und Rumble für Schranz-/Hardtekk-Kicks. Oder ein Preset aus der Kategorie „Kick“ laden.");
            });
        });
    }
    card_grid(ui, "kick", 290.0, 3, 6, |ui, i| {
        let on = p.kick.enabled;
        let k = &mut p.kick;
        match i {
            0 => {
                head(ui, Some(&mut k.enabled), "Kick-Engine · Pitch-Sweep", ACCENT, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    // pitch sweep display (log frequency, 0..300 ms)
                    let ys: Vec<f32> = (0..200)
                        .map(|i| {
                            let f = k.freq_at(60, i as f32 / 200.0 * 0.3);
                            ((f / 20.0).log2() / (5000f32 / 20.0).log2()).clamp(0.0, 1.0)
                        })
                        .collect();
                    let (rect, _) = curve(ui, 70.0, &ys, false, ACCENT);
                    let f0 = tstudio_dsp::midi_to_freq(k.tune);
                    ui.painter().text(rect.right_bottom() - vec2(5.0, 4.0), Align2::RIGHT_BOTTOM, format!("{f0:.1} Hz · 300 ms"), FontId::proportional(10.0), TEXT_DIM);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        small_label(ui, "Grundton");
                        note_drag(ui, &mut k.tune, 12.0, 60.0).on_hover_text("Tonhöhe des Kick-Körpers nach dem Pitch-Sweep");
                        ui.checkbox(&mut k.key_follow, "Taste transponiert").on_hover_text("Aus: jede Taste spielt die gestimmte Kick");
                    });
                    knobs(ui, |ui| {
                        knob(ui, &mut k.pitch_start, 0.0, 72.0, 40.0, "Start", false, &|v| format!("+{v:.0} st")).on_hover_text("Halbtöne über dem Grundton beim Anschlag");
                        knob(ui, &mut k.pitch_time, 0.005, 0.5, 0.07, "Zeit", true, &widgets::fmt_s);
                        knob(ui, &mut k.pitch_curve, -1.0, 1.0, 0.6, "Kurve", false, &|v| format!("{v:+.2}")).on_hover_text("> 0: schnell fallen, dann landen (Punch)");
                    });
                });
            }
            1 => {
                head(ui, None, "Click", CLICK_COL, |ui| {
                    ui.add_enabled_ui(on, |ui| combo(ui, "click", &mut k.click_type, &ClickType::ALL, ClickType::name, 64.0));
                });
                ui.add_enabled_ui(on, |ui| {
                    // click envelope over the first 50 ms
                    let ys: Vec<f32> = (0..120).map(|i| k.click_level.min(1.0) * (-(i as f32 / 119.0 * 0.05) / k.click_decay.max(1e-4)).exp()).collect();
                    let (rect, _) = curve(ui, 56.0, &ys, false, CLICK_COL);
                    ui.painter().text(rect.right_top() + vec2(-4.0, 3.0), Align2::RIGHT_TOP, "50 ms", FontId::proportional(10.0), TEXT_DIM);
                    ui.add_space(4.0);
                    knobs(ui, |ui| {
                        knob(ui, &mut k.click_level, 0.0, 1.0, 0.3, "Level", false, &widgets::fmt_pct);
                        knob(ui, &mut k.click_tone, 300.0, 12000.0, 2800.0, "Tone", true, &widgets::fmt_hz);
                        knob(ui, &mut k.click_decay, 0.0005, 0.05, 0.006, "Decay", true, &widgets::fmt_s);
                    });
                });
            }
            2 => {
                head(ui, None, "Body / Drive", FX_COL, |ui| {
                    ui.add_enabled_ui(on, |ui| combo(ui, "kdrive", &mut k.drive_mode, &DriveMode::ALL, DriveMode::name, 60.0));
                });
                ui.add_enabled_ui(on, |ui| {
                    drive_display(ui, k.drive_mode, k.drive_db, FX_COL);
                    ui.add_space(4.0);
                    knobs(ui, |ui| {
                        knob(ui, &mut k.drive_db, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db).on_hover_text("Schranz: 15–24 dB Hard, Hardtekk: Asym");
                        knob(ui, &mut k.body_hold, 0.0, 0.3, 0.0, "Hold", false, &widgets::fmt_s);
                        knob(ui, &mut k.body_decay, 0.0, 2.0, 0.0, "Decay", false, &|v| if v <= 0.0 { "Amp-Env".into() } else { widgets::fmt_s(v) }).on_hover_text("0 = Amp-Hüllkurve wirkt VOR dem Drive: Drive hält den Bass lang und fett (Standard).
> 0: eigene Körper-Hüllkurve, Amp-Hüllkurve danach.");
                    });
                });
            }
            3 => env_card(ui, "Env 1 · Amp (Kick-Länge)", &mut p.amp_env, OSC_COL[0]),
            4 => {
                head(ui, None, "Rumble", OSC_COL[1], |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut k.rumble_level, 0.0, 1.0, 0.0, "Level", false, &widgets::fmt_pct);
                        knob(ui, &mut k.rumble_delay, 0.0, 0.3, 0.06, "Delay", false, &widgets::fmt_s);
                        knob(ui, &mut k.rumble_decay, 0.05, 3.0, 0.4, "Decay", true, &widgets::fmt_s);
                        knob(ui, &mut k.rumble_tone, 40.0, 600.0, 160.0, "Tone", true, &widgets::fmt_hz);
                    });
                    small_label(ui, "Tiefpassgefilterte Hallfahne des Körpers. Für langen Rumble die Amp-Hüllkurve lang lassen.");
                });
            }
            _ => {
                head(ui, None, "Pitch-Hüllkurve (klassisch)", TEXT_DIM, |_| {});
                knobs(ui, |ui| {
                    knob(ui, &mut p.pitch_env_amount, -24.0, 72.0, 0.0, "Pitch", false, &|v| format!("{v:+.0} st")).on_hover_text("Tonhöhe beim Anschlag, fällt auf 0 ab – Zaps, Toms");
                    knob(ui, &mut p.pitch_env_decay, 0.002, 0.5, 0.04, "Decay", true, &widgets::fmt_s);
                });
                small_label(ui, "Wirkt auf alle Oszillatoren, auch ohne Kick-Engine.");
            }
        }
    });
}

fn filter_slot(ui: &mut Ui, id: &str, kind: &mut FilterType, mode: &mut tstudio_dsp::filter::FilterMode) {
    // called inside a right-to-left layout: mode first so it ends up on the right
    if kind.uses_mode() {
        ui.push_id(id, |ui| filter_mode_combo(ui, mode));
    }
    combo(ui, (id, "type"), kind, &FilterType::ALL, FilterType::name, 70.0);
}

fn filter_page(ui: &mut Ui, p: &mut SynthParams) {
    card_grid(ui, "filter", 330.0, 2, 4, |ui, i| match i {
        0 => {
            let mut on = p.filter_enabled;
            let active = head(ui, Some(&mut on), "Filter 1", OSC_COL[1], |ui| {
                ui.add_enabled_ui(p.filter_enabled, |ui| filter_slot(ui, "f1", &mut p.filter_type, &mut p.filter_mode));
            });
            p.filter_enabled = on;
            ui.add_enabled_ui(active, |ui| {
                filter_display(ui, p.filter_type, p.filter_mode, p.cutoff, p.resonance, OSC_COL[1]);
                ui.add_space(4.0);
                knobs(ui, |ui| {
                    let label = if p.filter_type == FilterType::Formant { "Vokal" } else { "Cutoff" };
                    knob(ui, &mut p.cutoff, 20.0, 20000.0, 4000.0, label, true, &widgets::fmt_hz);
                    knob(ui, &mut p.resonance, 0.0, 1.0, 0.15, "Reso", false, &widgets::fmt_pct);
                    knob(ui, &mut p.filter_drive, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db);
                    knob(ui, &mut p.filter_env_amount, -8.0, 8.0, 1.5, "Env 2", false, &|v| format!("{v:+.1} oct"));
                    knob(ui, &mut p.key_track, 0.0, 1.0, 0.3, "Keytrack", false, &widgets::fmt_pct);
                });
            });
        }
        1 => {
            let f2 = &mut p.filter2;
            let mut on = f2.enabled;
            let active = head(ui, Some(&mut on), "Filter 2", OSC_COL[2], |ui| {
                ui.add_enabled_ui(f2.enabled, |ui| filter_slot(ui, "f2", &mut f2.kind, &mut f2.mode));
            });
            f2.enabled = on;
            ui.add_enabled_ui(active, |ui| {
                filter_display(ui, f2.kind, f2.mode, f2.cutoff, f2.resonance, OSC_COL[2]);
                ui.add_space(4.0);
                knobs(ui, |ui| {
                    knob(ui, &mut f2.cutoff, 20.0, 20000.0, 120.0, "Cutoff", true, &widgets::fmt_hz);
                    knob(ui, &mut f2.resonance, 0.0, 1.0, 0.1, "Reso", false, &widgets::fmt_pct);
                    knob(ui, &mut f2.drive_db, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db);
                    knob(ui, &mut f2.env_amount, -8.0, 8.0, 0.0, "Env 2", false, &|v| format!("{v:+.1} oct"));
                    knob(ui, &mut f2.key_track, 0.0, 1.0, 0.0, "Keytrack", false, &widgets::fmt_pct);
                });
                ui.horizontal(|ui| {
                    small_label(ui, "Routing");
                    ui.selectable_value(&mut p.filter_routing, FilterRouting::Serial, "Seriell (1 → 2)");
                    ui.selectable_value(&mut p.filter_routing, FilterRouting::Parallel, "Parallel");
                });
            });
        }
        2 => env_card(ui, "Env 2 · Filter", &mut p.filter_env, OSC_COL[1]),
        _ => {
            head(ui, None, "Sättigung (nach Filter)", FX_COL, |ui| {
                combo(ui, "pdrive", &mut p.drive_mode, &DriveMode::ALL, DriveMode::name, 60.0);
            });
            drive_display(ui, p.drive_mode, p.drive_db, FX_COL);
            ui.add_space(4.0);
            knobs(ui, |ui| {
                knob(ui, &mut p.drive_db, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db);
            });
        }
    });
}

fn lfo_card(ui: &mut Ui, i: usize, l: &mut LfoParams) {
    head(ui, None, &format!("LFO {}", i + 1), MOD_COL, |ui| {
        if l.shape == LfoShape::Wave {
            combo(ui, ("lwave", i), &mut l.wave, &Waveform::ALL, Waveform::name, 64.0);
        }
        combo(ui, ("lshape", i), &mut l.shape, &LfoShape::ALL, LfoShape::name, 64.0);
    });
    let ys: Vec<f32> = (0..160).map(|k| l.value_at(k as f32 / 160.0)).collect();
    let (rect, resp) = curve(ui, 56.0, &ys, true, MOD_COL);
    if l.shape == LfoShape::Custom {
        // draw the 16 steps: click / drag sets a step
        if let Some(pt) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.clicked() {
                let s = (((pt.x - rect.left()) / rect.width()) * 16.0).clamp(0.0, 15.0) as usize;
                l.custom[s] = (1.0 - 2.0 * (pt.y - rect.top()) / rect.height()).clamp(-1.0, 1.0);
            }
        }
        resp.on_hover_text("Klicken/ziehen: Stufen zeichnen");
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut l.sync, "Sync");
        ui.checkbox(&mut l.retrig, "Retrig");
        if l.shape == LfoShape::Custom {
            ui.checkbox(&mut l.smooth, "Glatt");
        }
    });
    ui.horizontal(|ui| {
        small_label(ui, "Direkt →");
        combo(ui, ("ltarget", i), &mut l.target, &LfoTarget::ALL, LfoTarget::name, 64.0);
        if l.sync {
            const DIV: [(f32, &str); 9] = [(0.125, "1/32"), (0.25, "1/16"), (0.5, "1/8"), (1.0, "1/4"), (2.0, "1/2"), (4.0, "1 Takt"), (8.0, "2 Takte"), (16.0, "4 Takte"), (32.0, "8 Takte")];
            let cur = DIV.iter().find(|(b, _)| (b - l.beats).abs() < 1e-4).map(|(_, n)| *n).unwrap_or("frei");
            egui::ComboBox::from_id_salt(("ldiv", i)).selected_text(cur).width(64.0).show_ui(ui, |ui| {
                for (b, n) in DIV {
                    ui.selectable_value(&mut l.beats, b, n);
                }
            });
        }
    });
    knobs(ui, |ui| {
        if !l.sync {
            knob(ui, &mut l.rate, 0.02, 30.0, 5.0, "Rate", true, &|v| format!("{v:.2} Hz"));
        }
        knob(ui, &mut l.amount, 0.0, 1.0, 0.0, "Direkt", false, &widgets::fmt_pct).on_hover_text("Menge für das direkte Ziel (Matrix-Mengen stellt man in der Matrix ein)");
        knob(ui, &mut l.phase, 0.0, 1.0, 0.0, "Phase", false, &widgets::fmt_pct);
    });
}

fn mod_page(ui: &mut Ui, p: &mut SynthParams) {
    card_grid(ui, "envs", 400.0, 3, 3, |ui, i| match i {
        0 => env_card(ui, "Env 1 · Amp", &mut p.amp_env, OSC_COL[0]),
        1 => env_card(ui, "Env 2 · Filter", &mut p.filter_env, OSC_COL[1]),
        _ => env_card(ui, "Env 3 · Mod", &mut p.mod_env, MOD_COL),
    });
    card_grid(ui, "lfos", 250.0, 4, NUM_LFO, |ui, i| lfo_card(ui, i, p.lfo_mut(i)));
    wide_card(ui, |ui| mod_matrix(ui, p));
}

fn mod_matrix(ui: &mut Ui, p: &mut SynthParams) {
    let used = p.mod_matrix.iter().filter(|s| s.active()).count();
    head(ui, None, "Mod-Matrix", MOD_COL, |ui| {
        small_label(ui, &format!("{used} / {NUM_MOD_SLOTS} aktiv"));
    });
    // drag sources
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        small_label(ui, "Quelle auf eine Zeile ziehen:");
        for s in ModSource::ALL.iter().skip(1) {
            let id = egui::Id::new(("modsrc", *s as u8));
            let w = ui.painter().layout_no_wrap(s.name().to_string(), egui::TextStyle::Small.resolve(ui.style()), TITLE).size().x + 16.0;
            if ui.max_rect().right() - ui.cursor().left() < w {
                ui.end_row();
            }
            ui.dnd_drag_source(id, *s, |ui| chip(ui, s.name(), BG_PANEL, if s.is_macro() { MOD_COL } else { TITLE }));
        }
    });
    ui.add_space(6.0);
    let last_used = p.mod_matrix.iter().rposition(|s| s.source != ModSource::Off || s.dest != ModDest::Off).map(|i| i + 1).unwrap_or(0);
    let show = (last_used + 1).min(NUM_MOD_SLOTS);
    let avail = ui.available_width();
    let narrow = avail < 720.0;
    let (src_w, dst_w) = if narrow { (92.0, 118.0) } else { (110.0, 150.0) };
    let slider_w = (avail - if narrow { 420.0 } else { 520.0 }).clamp(50.0, 360.0);
    egui::Grid::new("modmatrix").num_columns(5).spacing([10.0, 4.0]).striped(true).min_col_width(0.0).show(ui, |ui| {
        small_label(ui, "#");
        small_label(ui, "Quelle");
        small_label(ui, "Ziel");
        small_label(ui, "Menge");
        ui.label("");
        ui.end_row();
        for (i, slot) in p.mod_matrix.iter_mut().enumerate().take(show) {
            ui.label(RichText::new(format!("{}", i + 1)).color(TEXT_DIM));
            let r = ui.push_id(("src", i), |ui| combo(ui, ("msrc", i), &mut slot.source, &ModSource::ALL, ModSource::name, src_w)).response;
            if let Some(src) = r.dnd_release_payload::<ModSource>() {
                slot.source = *src;
                if slot.amount == 0.0 {
                    slot.amount = 0.5;
                }
            }
            ui.push_id(("dst", i), |ui| combo(ui, ("mdst", i), &mut slot.dest, &ModDest::ALL, ModDest::name, dst_w));
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = slider_w;
                ui.add(egui::Slider::new(&mut slot.amount, -1.0..=1.0).show_value(false));
                ui.add_sized(vec2(48.0, 18.0), egui::Label::new(RichText::new(format!("{:+.0} %", slot.amount * 100.0)).monospace()));
            });
            ui.horizontal(|ui| {
                if slot.dest.is_global() && slot.source != ModSource::Off && !slot.source.is_macro() {
                    ui.colored_label(FX_COL, "⚠ nur Makros").on_hover_text("FX-Ziele gelten für alle Stimmen und nehmen nur Makros");
                }
                if (slot.source != ModSource::Off || slot.dest != ModDest::Off) && ui.small_button("✕").on_hover_text("Slot leeren").clicked() {
                    *slot = ModSlot::default();
                }
            });
            ui.end_row();
        }
    });
}

fn fx_page(ui: &mut Ui, p: &mut SynthParams) {
    // signal chain overview
    wide_card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            small_label(ui, "Kette:");
            let fx = &p.fx;
            let chain = [("Dist", fx.dist_on), ("Comp", fx.comp_on), ("OTT", fx.ott_on), ("Chorus", fx.chorus_on), ("Delay", fx.delay_on), ("Reverb", fx.reverb_on), ("Clipper", fx.clip_on)];
            for (k, (name, on)) in chain.iter().enumerate() {
                if k > 0 {
                    small_label(ui, "→");
                }
                chip(ui, name, if *on { FX_COL.gamma_multiply(0.35) } else { BG_PANEL }, if *on { TITLE } else { TEXT_DIM });
            }
        });
    });
    card_grid(ui, "fx", 300.0, 4, 8, |ui, i| {
        let fx = &mut p.fx;
        match i {
            0 => {
                let on = head(ui, Some(&mut fx.dist_on), "Distortion", FX_COL, |ui| {
                    combo(ui, "fxdist", &mut fx.dist.mode, &DistMode::ALL, DistMode::name, 80.0);
                });
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.dist.drive_db, 0.0, 40.0, 12.0, "Drive", false, &widgets::fmt_db);
                        knob(ui, &mut fx.dist.tone, 500.0, 20000.0, 9000.0, "Tone", true, &widgets::fmt_hz);
                        knob(ui, &mut fx.dist.output_db, -24.0, 6.0, -3.0, "Out", false, &widgets::fmt_db);
                        knob(ui, &mut fx.dist.mix, 0.0, 1.0, 1.0, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            1 => {
                let on = head(ui, Some(&mut fx.comp_on), "Kompressor", FX_COL, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.comp.threshold, -60.0, 0.0, -18.0, "Thresh", false, &widgets::fmt_db);
                        knob(ui, &mut fx.comp.ratio, 1.0, 20.0, 4.0, "Ratio", false, &|v| format!("{v:.1}:1"));
                        knob(ui, &mut fx.comp.attack_ms, 0.1, 200.0, 10.0, "Attack", true, &widgets::fmt_ms);
                        knob(ui, &mut fx.comp.release_ms, 5.0, 2000.0, 120.0, "Release", true, &widgets::fmt_ms);
                        knob(ui, &mut fx.comp.makeup, -12.0, 24.0, 0.0, "Makeup", false, &widgets::fmt_db);
                    });
                });
            }
            2 => {
                let on = head(ui, Some(&mut fx.ott_on), "OTT", FX_COL, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.ott.depth, 0.0, 1.0, 0.4, "Depth", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.ott.time, 0.2, 3.0, 1.0, "Time", true, &widgets::fmt_x);
                        knob(ui, &mut fx.ott.upward, 0.0, 1.0, 1.0, "Up", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.ott.downward, 0.0, 1.0, 1.0, "Down", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.ott.out_gain_db, -24.0, 12.0, 0.0, "Out", false, &widgets::fmt_db);
                    });
                });
            }
            3 => {
                let on = head(ui, Some(&mut fx.chorus_on), "Chorus", OSC_COL[1], |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.chorus.rate, 0.05, 5.0, 0.8, "Rate", true, &|v| format!("{v:.2} Hz"));
                        knob(ui, &mut fx.chorus.depth_ms, 0.0, 10.0, 3.0, "Depth", false, &widgets::fmt_ms);
                        knob(ui, &mut fx.chorus.mix, 0.0, 1.0, 0.35, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            4 => {
                let on = head(ui, Some(&mut fx.delay_on), "Delay", OSC_COL[1], |ui| {
                    ui.checkbox(&mut fx.delay.ping_pong, "Ping-Pong");
                });
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.delay.time_beats, 0.0625, 2.0, 0.75, "Zeit", false, &|v| format!("{v:.3} Beats"));
                        knob(ui, &mut fx.delay.feedback, 0.0, 0.95, 0.4, "Feedback", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.delay.tone, 500.0, 20000.0, 6000.0, "Tone", true, &widgets::fmt_hz);
                        knob(ui, &mut fx.delay.mix, 0.0, 1.0, 0.2, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            5 => {
                let on = head(ui, Some(&mut fx.reverb_on), "Reverb", OSC_COL[1], |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.reverb.size, 0.0, 1.0, 0.6, "Größe", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.reverb.damping, 0.0, 1.0, 0.5, "Dämpf.", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.reverb.predelay_ms, 0.0, 200.0, 10.0, "Pre", false, &widgets::fmt_ms);
                        knob(ui, &mut fx.reverb.mix, 0.0, 1.0, 0.2, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            6 => {
                let on = head(ui, Some(&mut fx.clip_on), "Clipper (Ende)", FX_COL, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.clip.drive_db, 0.0, 24.0, 0.0, "Drive", false, &widgets::fmt_db);
                        knob(ui, &mut fx.clip.ceiling_db, -12.0, 0.0, -0.8, "Ceiling", false, &widgets::fmt_db);
                        knob(ui, &mut fx.clip.softness, 0.0, 1.0, 0.25, "Soft", false, &widgets::fmt_pct);
                    });
                });
            }
            _ => {
                head(ui, None, "Voicing / Ausgang", ACCENT, |_| {});
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.mono, "Mono");
                    ui.add_enabled(p.mono, egui::Checkbox::new(&mut p.legato, "Legato")).on_hover_text("An: gebundene Noten gleiten ohne Neuanschlag · Aus: jede Note triggert neu (Kicks, Stabs)");
                    ui.add_enabled(p.mono && p.glide > 0.0, egui::Checkbox::new(&mut p.slide_only, "303-Slide")).on_hover_text("Glide nur zwischen gebundenen (überlappenden) Noten – wie bei der TB-303");
                });
                knobs(ui, |ui| {
                    let mut poly = p.polyphony as f32;
                    ui.add_enabled_ui(!p.mono, |ui| {
                        knob(ui, &mut poly, 1.0, 16.0, 16.0, "Stimmen", false, &|v| format!("{:.0}", v.round()));
                    });
                    p.polyphony = poly.round().clamp(1.0, 16.0) as u32;
                    knob(ui, &mut p.glide, 0.0, 1.0, 0.0, "Glide", false, &widgets::fmt_s);
                    knob(ui, &mut p.velocity_sens, 0.0, 1.0, 0.6, "Velocity", false, &widgets::fmt_pct);
                    knob(ui, &mut p.gain, 0.0, 1.5, 0.5, "Gain", false, &widgets::fmt_gain);
                });
            }
        }
    });
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// Runs a few headless frames of one page; returns the last output and whether it overflowed.
    fn run_page(width: f32, page: Page, p: &mut SynthParams) -> (egui::FullOutput, Option<String>) {
        let ctx = egui::Context::default();
        widgets::apply_theme(&ctx);
        let mut st = SynthUiState { page, loaded: "Schranz Kick".into(), ..Default::default() };
        let dir = std::env::temp_dir().join("tstudio_layout_test_presets");
        let mut status = None;
        let mut out = None;
        let mut err = None;
        // several frames so the equal-height rows settle
        for _ in 0..5 {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 1400.0))), ..Default::default() };
            out = Some(ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let avail = ui.available_rect_before_wrap();
                    header(ui, &mut st, p, &dir, &mut status);
                    page_tabs(ui, &mut st, p);
                    ui.add_space(GAP);
                    match page {
                        Page::Sound => sound_page(ui, p),
                        Page::Kick => kick_page(ui, p),
                        Page::Filter => filter_page(ui, p),
                        Page::Mod => mod_page(ui, p),
                        Page::Fx => fx_page(ui, p),
                    }
                    let used = ui.min_rect();
                    err = (used.right() > avail.right() + 1.0).then(|| format!("{:.0} > {:.0}", used.right(), avail.right()));
                });
            }));
        }
        (out.unwrap(), err)
    }

    fn preset(name: &str) -> SynthParams {
        SynthParams::presets().into_iter().find(|(n, _)| *n == name).map(|(_, p)| p).unwrap()
    }

    /// Nothing may stick out to the right (the old wrapped layout pushed LFOs off screen).
    #[test]
    fn pages_fit_the_width_at_all_sizes() {
        for width in [700.0f32, 1000.0, 1300.0, 1620.0, 2400.0] {
            for (page, name) in Page::ALL {
                for pr in ["Init", "Schranz Kick", "Acid Ladder"] {
                    let mut p = preset(pr);
                    let (_, err) = run_page(width, page, &mut p);
                    assert!(err.is_none(), "{name} / {pr} at {width}px overflows: {}", err.unwrap());
                }
            }
        }
    }

    /// Debug aid (no screen control): `TSYNTH_SVG=dir cargo test … dump_svg -- --ignored`
    /// writes every page as an SVG of egui's paint shapes.
    #[test]
    #[ignore]
    fn dump_svg() {
        use egui::epaint::Shape;
        use std::fmt::Write as _;
        let dir = std::path::PathBuf::from(std::env::var("TSYNTH_SVG").expect("TSYNTH_SVG"));
        let width: f32 = std::env::var("TSYNTH_W").ok().and_then(|w| w.parse().ok()).unwrap_or(1620.0);
        let col = |c: Color32| if c.a() == 0 { "none".to_string() } else { format!("rgba({},{},{},{:.3})", c.r(), c.g(), c.b(), c.a() as f32 / 255.0) };
        fn walk(s: &Shape, out: &mut Vec<Shape>) {
            match s {
                Shape::Vec(v) => v.iter().for_each(|x| walk(x, out)),
                x => out.push(x.clone()),
            }
        }
        for (page, name) in Page::ALL {
            let mut p = preset(if page == Page::Kick { "Schranz Kick" } else { "Acid Ladder" });
            let (out, _) = run_page(width, page, &mut p);
            let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"1400\" font-family=\"Segoe UI, sans-serif\"><rect width=\"100%\" height=\"100%\" fill=\"{}\"/>", col(BG_PANEL));
            let mut flat = vec![];
            for cs in &out.shapes {
                walk(&cs.shape, &mut flat);
            }
            for s in flat {
                match s {
                    Shape::Rect(r) => {
                        let _ = write!(svg, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>", r.rect.left(), r.rect.top(), r.rect.width(), r.rect.height(), r.rounding.nw, col(r.fill), col(r.stroke.color), r.stroke.width);
                    }
                    Shape::Circle(c) => {
                        let _ = write!(svg, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>", c.center.x, c.center.y, c.radius, col(c.fill), col(c.stroke.color), c.stroke.width);
                    }
                    Shape::LineSegment { points, stroke } => {
                        let _ = write!(svg, "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>", points[0].x, points[0].y, points[1].x, points[1].y, col(match stroke.color { egui::epaint::ColorMode::Solid(c) => c, _ => Color32::WHITE }), stroke.width);
                    }
                    Shape::Path(ps) => {
                        let pts: String = ps.points.iter().map(|q| format!("{},{} ", q.x, q.y)).collect();
                        let sc = match ps.stroke.color {
                            egui::epaint::ColorMode::Solid(c) => c,
                            _ => Color32::WHITE,
                        };
                        let tag = if ps.closed { "polygon" } else { "polyline" };
                        let _ = write!(svg, "<{tag} points=\"{pts}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>", if ps.closed { col(ps.fill) } else { "none".into() }, col(sc), ps.stroke.width);
                    }
                    Shape::Text(t) => {
                        for row in &t.galley.rows {
                            let text: String = row.glyphs.iter().map(|g| g.chr).collect();
                            if text.trim().is_empty() {
                                continue;
                            }
                            let sec = row.glyphs.first().map(|g| g.section_index as usize).unwrap_or(0);
                            let fmt = t.galley.job.sections.get(sec).map(|s| s.format.clone()).unwrap_or_default();
                            let c = t.override_text_color.unwrap_or(if fmt.color == Color32::PLACEHOLDER { t.fallback_color } else { fmt.color });
                            let x = t.pos.x + row.glyphs.first().map(|g| g.pos.x).unwrap_or(row.rect.min.x);
                            let y = t.pos.y + row.rect.min.y;
                            let esc = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                            let _ = write!(svg, "<text x=\"{x}\" y=\"{y}\" font-size=\"{}\" fill=\"{}\" dominant-baseline=\"text-before-edge\">{esc}</text>", fmt.font_id.size, col(c));
                        }
                    }
                    _ => {}
                }
            }
            svg.push_str("</svg>");
            std::fs::write(dir.join(format!("tsynth_{}.svg", name.replace([' ', '/'], "_"))), svg).unwrap();
        }
    }
}
