//! Drum pads (F11): 16 pads with a step grid each, writing into the current pattern.
//!
//! Two pad sets: one Sampler channel per pad ("Kanäle"), or the slices of one chopped Sampler
//! (`mode: Chop`, slice i on key `slice_root + i`). Clicking a pad plays it live; the step grid
//! writes the same 16th notes as `tstudio-cli notes steps`. The Slicer chops the selected
//! Sampler's sample (transients or evenly) and maps the slices onto the pads.

use crate::app::TStudio;
use crate::ui::widgets::{self, rgb, ACCENT, BG_DARK, BG_LIGHT, PLAYHEAD, TEXT_DIM};
use egui::{vec2, Align2, Color32, FontId, RichText, Sense, Stroke, Ui};
use tstudio_dsp::sampler::SamplerMode;
use tstudio_dsp::slicer;
use tstudio_project::steps::STEP;
use tstudio_project::{Instrument, Project};

pub const PADS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadSet {
    /// pads follow the selected channel: slices if it is chopped, else the Sampler channels
    Auto,
    Channels,
    Chop,
}

pub struct DrumPadState {
    pub set: PadSet,
    pub selected: usize,
    /// pad being held down (channel, key)
    held: Option<(usize, u8)>,
    /// velocity of new steps per pad
    pub velocity: [f32; PADS],
    pub sensitivity: f32,
    pub even: usize,
    pub root: u8,
}

impl Default for DrumPadState {
    fn default() -> Self {
        Self { set: PadSet::Auto, selected: 0, held: None, velocity: [0.8; PADS], sensitivity: 0.5, even: 16, root: 36 }
    }
}

/// One pad: which channel and key it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub channel: usize,
    pub key: u8,
    pub label: String,
}

fn is_chopped(p: &Project, ch: usize) -> bool {
    matches!(p.channels.get(ch).map(|c| &c.instrument), Some(Instrument::Sampler(s)) if s.params.mode == SamplerMode::Chop && !s.params.slices.is_empty())
}

/// The pads for `set` with `selected` as the current channel (at most 16).
pub fn pad_map(p: &Project, set: PadSet, selected: usize) -> Vec<Pad> {
    let chop = match set {
        PadSet::Chop => true,
        PadSet::Channels => false,
        PadSet::Auto => is_chopped(p, selected),
    };
    if chop {
        let Some(Instrument::Sampler(s)) = p.channels.get(selected).map(|c| &c.instrument) else { return Vec::new() };
        return (0..s.params.slices.len().min(PADS))
            .filter_map(|i| slicer::slice_key(&s.params, i).map(|key| Pad { channel: selected, key, label: format!("Slice {}", i + 1) }))
            .collect();
    }
    p.channels
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.instrument, Instrument::Sampler(_)))
        .take(PADS)
        .map(|(i, c)| Pad { channel: i, key: c.instrument.default_key(), label: c.name.clone() })
        .collect()
}

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    let pads = pad_map(&app.project, app.drumpad.set, app.selected_channel);
    let chop_view = pads.first().map(|p| pads.iter().all(|q| q.channel == p.channel) && is_chopped(&app.project, p.channel)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.heading("Drum Pads");
        ui.label("Pads:");
        let st = &mut app.drumpad;
        ui.selectable_value(&mut st.set, PadSet::Auto, "Auto").on_hover_text("Gechoppter Sampler ausgewählt → Slices, sonst ein Sampler-Kanal pro Pad");
        ui.selectable_value(&mut st.set, PadSet::Channels, "Kanäle").on_hover_text("Ein Sampler-Kanal pro Pad (die ersten 16)");
        ui.selectable_value(&mut st.set, PadSet::Chop, "Slices").on_hover_text("Die Slices des ausgewählten Samplers (Chop-Modus)");
        let pname = app.project.pattern(app.current_pattern).map(|p| p.name.clone()).unwrap_or_default();
        ui.weak(format!("Steps schreiben in „{pname}“ · Pad klicken = spielen"));
    });
    if pads.is_empty() {
        ui.add_space(20.0);
        ui.label("Keine Pads: Samples aus der Library in den Channel Rack ziehen (ein Sampler pro Pad) oder einen Sampler auswählen und unten slicen.");
    }
    if app.drumpad.selected >= pads.len() {
        app.drumpad.selected = 0;
    }
    release_held(app, ui);

    ui.horizontal_top(|ui| {
        pad_grid(app, ui, &pads);
        ui.separator();
        ui.vertical(|ui| {
            ui.set_width(300.0);
            if let Some(pad) = pads.get(app.drumpad.selected).cloned() {
                pad_controls(app, ui, &pad, chop_view);
            }
            ui.separator();
            slicer_panel(app, ui);
        });
    });
    ui.separator();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| step_grid(app, ui, &pads));
}

/// Note-off for a pad when the mouse button is released anywhere.
fn release_held(app: &mut TStudio, ui: &Ui) {
    if !ui.input(|i| i.pointer.primary_down()) {
        if let Some((ch, key)) = app.drumpad.held.take() {
            app.note_off(ch, key);
        }
    }
}

fn pad_grid(app: &mut TStudio, ui: &mut Ui, pads: &[Pad]) {
    let size = 74.0;
    let gap = 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(4.0 * size + 3.0 * gap, 4.0 * size + 3.0 * gap), Sense::hover());
    for i in 0..PADS {
        // pad 1 bottom left, like hardware pads
        let (col, row) = (i % 4, 3 - i / 4);
        let r = egui::Rect::from_min_size(rect.min + vec2(col as f32 * (size + gap), row as f32 * (size + gap)), vec2(size, size));
        let resp = ui.interact(r, ui.id().with(("pad", i)), Sense::click_and_drag());
        let pad = pads.get(i);
        if let (Some(pad), true) = (pad, resp.is_pointer_button_down_on() && ui.input(|x| x.pointer.primary_pressed())) {
            app.drumpad.selected = i;
            if let Some((ch, key)) = app.drumpad.held.take() {
                app.note_off(ch, key);
            }
            app.note_on(pad.channel, pad.key, app.drumpad.velocity[i]);
            app.drumpad.held = Some((pad.channel, pad.key));
        }
        let held = pad.map(|p| app.drumpad.held == Some((p.channel, p.key))).unwrap_or(false);
        let color = pad.and_then(|p| app.project.channels.get(p.channel)).map(|c| rgb(c.color)).unwrap_or(Color32::from_gray(60));
        let painter = ui.painter();
        let fill = if pad.is_none() { BG_DARK } else if held { color } else { color.gamma_multiply(0.35) };
        painter.rect_filled(r, 6.0, fill);
        let stroke = if i == app.drumpad.selected && pad.is_some() { Stroke::new(2.0_f32, ACCENT) } else { Stroke::new(1.0_f32, BG_LIGHT) };
        painter.rect_stroke(r, 6.0, stroke);
        painter.text(r.left_top() + vec2(6.0, 4.0), Align2::LEFT_TOP, format!("{}", i + 1), FontId::proportional(10.0), TEXT_DIM);
        if let Some(p) = pad {
            let name: String = p.label.chars().take(11).collect();
            painter.text(r.center(), Align2::CENTER_CENTER, name, FontId::proportional(11.0), Color32::WHITE);
            painter.text(r.center_bottom() - vec2(0.0, 10.0), Align2::CENTER_CENTER, tstudio_dsp::note_name(p.key), FontId::monospace(9.5), TEXT_DIM);
        }
    }
}

fn pad_controls(app: &mut TStudio, ui: &mut Ui, pad: &Pad, chop_view: bool) {
    let i = app.drumpad.selected;
    ui.label(RichText::new(format!("Pad {} · {}", i + 1, pad.label)).strong());
    let pid = app.current_pattern;
    let Some(cid) = app.project.channels.get(pad.channel).map(|c| c.id) else { return };
    if chop_view {
        // the slices share one channel: the pad volume is the velocity of its steps
        let mut v = app.drumpad.velocity[i];
        if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Lautstärke (Velocity)")).changed() {
            app.drumpad.velocity[i] = v;
            app.project.set_row_velocity(pid, cid, pad.key, v);
        }
        ui.weak("Pan/Pitch gelten für den ganzen gechoppten Sampler:");
    }
    let ch = &mut app.project.channels[pad.channel];
    ui.horizontal(|ui| {
        widgets::knob(ui, &mut ch.volume, 0.0, 1.25, 0.8, "Vol", false, &widgets::fmt_pct);
        widgets::knob(ui, &mut ch.pan, -1.0, 1.0, 0.0, "Pan", false, &widgets::fmt_pan);
        if let Instrument::Sampler(s) = &mut ch.instrument {
            widgets::knob(ui, &mut s.params.pitch, -24.0, 24.0, 0.0, "Pitch", false, &widgets::fmt_semi);
        }
    });
    if !chop_view {
        ui.add(egui::Slider::new(&mut app.drumpad.velocity[i], 0.0..=1.0).text("Velocity neuer Steps"));
    }
    if ui.button("Kanal auswählen (Instrument F8)").clicked() {
        app.selected_channel = pad.channel;
        app.view = crate::app::View::Instrument;
    }
}

fn slicer_panel(app: &mut TStudio, ui: &mut Ui) {
    ui.label(RichText::new("Slicer").strong());
    let ci = app.selected_channel;
    let Some(Instrument::Sampler(s)) = app.project.channels.get(ci).map(|c| &c.instrument) else {
        ui.weak("Einen Sampler-Kanal auswählen (Channel Rack), dann hier slicen.");
        return;
    };
    let Some(path) = s.sample.clone() else {
        ui.weak("Der ausgewählte Sampler hat noch kein Sample.");
        return;
    };
    let (start, end) = (s.params.start, s.params.end);
    ui.weak(format!("{} · {}", app.project.channels[ci].name, path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    let st = &mut app.drumpad;
    ui.horizontal(|ui| {
        ui.label("Erste Taste");
        ui.add(egui::DragValue::new(&mut st.root).range(0..=112).custom_formatter(|v, _| tstudio_dsp::note_name(v as u8)));
    });
    ui.add(egui::Slider::new(&mut st.sensitivity, 0.0..=1.0).text("Empfindlichkeit"));
    let mut slices = None;
    ui.horizontal(|ui| {
        if ui.button("✂ Transienten").on_hover_text("Schneidet an den Anschlägen").clicked() {
            match app.cache.get_raw(&path) {
                Some(d) => slices = Some(slicer::transient_slices(&d, app.drumpad.sensitivity, start, end)),
                None => app.set_status("Sample lädt noch – gleich nochmal versuchen"),
            }
        }
        ui.add(egui::DragValue::new(&mut app.drumpad.even).range(2..=64).suffix(" ×"));
        if ui.button("✂ Gleichmäßig").clicked() {
            slices = Some(slicer::even_slices(start, end, app.drumpad.even));
        }
    });
    if let Some(sl) = slices {
        let root = app.drumpad.root;
        if let Instrument::Sampler(s) = &mut app.project.channels[ci].instrument {
            let n = slicer::apply_slices(&mut s.params, sl, Some(root));
            s.params.mono = true;
            app.drumpad.set = PadSet::Auto;
            app.set_status(format!("{n} Slices ab {} – je eine Taste, die ersten 16 auf den Pads", tstudio_dsp::note_name(root)));
        }
    }
}

fn step_grid(app: &mut TStudio, ui: &mut Ui, pads: &[Pad]) {
    let pid = app.current_pattern;
    let n = app.project.step_count(pid);
    let cell = vec2(22.0, 20.0);
    let label_w = 120.0;
    let playing_step = (app.is_playing() && !app.song_mode).then(|| (app.position() / STEP as f64).floor() as usize);
    for (i, pad) in pads.iter().enumerate() {
        let Some(cid) = app.project.channels.get(pad.channel).map(|c| c.id) else { continue };
        let row = app.project.step_row(pid, cid, pad.key, n);
        ui.horizontal(|ui| {
            let sel = i == app.drumpad.selected;
            let txt = RichText::new(format!("{:>2} {}", i + 1, pad.label)).monospace().color(if sel { ACCENT } else { Color32::WHITE });
            if ui.add_sized(vec2(label_w, cell.y), egui::SelectableLabel::new(sel, txt)).clicked() {
                app.drumpad.selected = i;
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(cell.x * n as f32, cell.y), Sense::click());
            if resp.clicked() {
                if let Some(pt) = resp.interact_pointer_pos() {
                    let step = ((pt.x - rect.left()) / cell.x) as usize;
                    if step < n {
                        let v = app.drumpad.velocity[i];
                        app.project.toggle_step(pid, cid, pad.key, step, v);
                        app.drumpad.selected = i;
                    }
                }
            }
            let painter = ui.painter();
            let color = app.project.channels.get(pad.channel).map(|c| rgb(c.color)).unwrap_or(ACCENT);
            for (k, v) in row.iter().enumerate() {
                let r = egui::Rect::from_min_size(rect.min + vec2(k as f32 * cell.x, 0.0), cell).shrink(1.5);
                let beat_shade = if (k / 4) % 2 == 0 { BG_LIGHT } else { Color32::from_rgb(40, 44, 51) };
                let fill = match v {
                    Some(vel) => color.gamma_multiply(0.35 + 0.65 * vel),
                    None => beat_shade,
                };
                painter.rect_filled(r, 3.0, fill);
                if playing_step == Some(k) {
                    painter.rect_stroke(r, 3.0, Stroke::new(1.5_f32, PLAYHEAD));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
