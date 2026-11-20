//! "MIDI-Learn" window: assign hardware knobs/faders (CC) to mixer volume/pan and
//! TSynth macro 1–4 / cutoff. Click „Lernen“, move a knob – done. Assignments live in the
//! app settings (`Settings::midi_mappings`); the mapping logic is in `crate::midi_map`.

use crate::app::TStudio;
use crate::midi_map::{self, MidiTarget};
use crate::ui::widgets::{ACCENT, TEXT_DIM};
use egui::{Color32, RichText, Ui};
use tstudio_project::Instrument;

#[derive(Default)]
pub struct MidiLearnState {
    pub open: bool,
    /// waiting for a CC for this target
    pub learning: Option<MidiTarget>,
    /// last CC seen (channel, cc, value)
    pub last_cc: Option<(u8, u8, u8)>,
}

/// A control change from the MIDI thread (called on the UI thread).
pub fn on_cc(app: &mut TStudio, ch: u8, cc: u8, value: u8) {
    app.midi_learn.last_cc = Some((ch, cc, value));
    if let Some(target) = app.midi_learn.learning.take() {
        midi_map::learn(&mut app.settings.midi_mappings, ch, cc, target);
        app.settings.save(&app.paths.settings());
        let label = target.label(&app.project);
        app.set_status(format!("MIDI-Learn: CC {cc} (Kanal {}) → {label}", ch + 1));
    }
    for t in midi_map::targets(&app.settings.midi_mappings, ch, cc) {
        t.apply(&mut app.project, value);
    }
}

fn learn_button(app: &mut TStudio, ui: &mut Ui, target: MidiTarget, text: &str) {
    let waiting = app.midi_learn.learning == Some(target);
    let mapped = app.settings.midi_mappings.iter().find(|m| m.target == target).map(|m| format!(" (CC {})", m.cc)).unwrap_or_default();
    let label = if waiting { RichText::new(format!("{text} … Regler bewegen")).color(ACCENT) } else { RichText::new(format!("{text}{mapped}")) };
    if ui.selectable_label(waiting, label).on_hover_text("Klicken, dann am Controller einen Regler bewegen").clicked() {
        app.midi_learn.learning = if waiting { None } else { Some(target) };
    }
}

pub fn window(app: &mut TStudio, ctx: &egui::Context) {
    if !app.midi_learn.open {
        return;
    }
    let mut open = true;
    egui::Window::new("🎛 MIDI-Learn").open(&mut open).resizable(false).default_width(380.0).show(ctx, |ui| {
        match &app.midi.port_name {
            Some(p) => ui.weak(format!("Eingang: {p}")),
            None => ui.colored_label(Color32::from_rgb(240, 200, 70), "Kein MIDI-Eingang gewählt (Datei > Einstellungen)"),
        };
        if let Some((ch, cc, v)) = app.midi_learn.last_cc {
            ui.weak(format!("Zuletzt: CC {cc} = {v} (Kanal {})", ch + 1));
        }
        ui.separator();
        let t = app.mixer_ui.selected.min(app.project.mixer.len().saturating_sub(1));
        ui.label(RichText::new(format!("Mixer-Spur {t} „{}“ (im Mixer auswählen)", app.project.mixer[t].name)).strong());
        ui.horizontal(|ui| {
            learn_button(app, ui, MidiTarget::MixerVolume(t), "Lautstärke");
            learn_button(app, ui, MidiTarget::MixerPan(t), "Pan");
        });
        ui.separator();
        let c = app.selected_channel;
        let synth = matches!(app.project.channels.get(c).map(|x| &x.instrument), Some(Instrument::Synth(_)));
        match app.project.channels.get(c) {
            Some(ch) if synth => {
                ui.label(RichText::new(format!("TSynth „{}“ (ausgewählter Kanal)", ch.name)).strong());
                ui.horizontal_wrapped(|ui| {
                    for i in 0..4u8 {
                        learn_button(app, ui, MidiTarget::SynthMacro { channel: c, index: i }, &format!("Makro {}", i + 1));
                    }
                    learn_button(app, ui, MidiTarget::SynthCutoff(c), "Cutoff");
                });
            }
            _ => {
                ui.weak("Für Makros/Cutoff einen TSynth-Kanal im Channel Rack auswählen.");
            }
        }
        ui.separator();
        ui.label(RichText::new("Zuweisungen").strong());
        if app.settings.midi_mappings.is_empty() {
            ui.weak("(noch keine)");
        }
        let mut remove = None;
        for (i, m) in app.settings.midi_mappings.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("CC {:>3} · K{:<2}", m.cc, m.midi_channel + 1)).monospace().color(TEXT_DIM));
                ui.label(m.target.label(&app.project));
                if ui.small_button("✕").on_hover_text("Zuweisung löschen").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            app.settings.midi_mappings.remove(i);
            app.settings.save(&app.paths.settings());
        }
        ui.weak("Noten vom MIDI-Keyboard spielen immer den ausgewählten Kanal.");
    });
    if !open {
        app.midi_learn.open = false;
        app.midi_learn.learning = None;
    }
}
