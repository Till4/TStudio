use crate::app::{TStudio, View};
use crate::ui::widgets::{self, knob, SampleDrag, ACCENT, BG_DARK, TEXT_DIM};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui};
use tstudio_dsp::env::AdsrParams;
use tstudio_dsp::sampler::SamplerMode;
use tstudio_dsp::stretch::StretchMode;
use tstudio_project::{Instrument, Note, PPQ};

pub struct InstrumentState {
    pub chop_sensitivity: f32,
    pub preset_name: String,
    marker_drag: Option<u8>,
    /// presets of the last SoundFont shown (path, result)
    sf_presets: Option<(std::path::PathBuf, Result<Vec<tstudio_engine::Sf2Preset>, String>)>,
    sf_filter: String,
    pub synth: crate::ui::tsynth::SynthUiState,
}

impl Default for InstrumentState {
    fn default() -> Self {
        Self { chop_sensitivity: 0.5, preset_name: String::new(), marker_drag: None, sf_presets: None, sf_filter: String::new(), synth: Default::default() }
    }
}

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    let ci = app.selected_channel;
    if ci >= app.project.channels.len() {
        ui.heading("Instrument");
        ui.label("Kein Kanal ausgewählt.");
        return;
    }
    ui.horizontal(|ui| {
        let ch = &mut app.project.channels[ci];
        ui.heading(ch.instrument.kind_name());
        ui.add(egui::TextEdit::singleline(&mut ch.name).desired_width(180.0));
        ui.label("Vol");
        let mut db = tstudio_dsp::gain_to_db(ch.volume).max(-60.0);
        if ui.add(egui::DragValue::new(&mut db).speed(0.1).range(-60.0..=6.0).suffix(" dB")).changed() {
            ch.volume = tstudio_dsp::db_to_gain(db);
        }
        ui.label("Pan");
        ui.add(egui::DragValue::new(&mut ch.pan).speed(0.01).range(-1.0..=1.0));
        ui.label("Mixer");
        ui.add(egui::DragValue::new(&mut ch.mixer_track).range(0..=app.project.mixer.len() - 1).prefix("→"));
        if ui.button("🎹 Piano Roll").clicked() {
            app.view = View::PianoRoll;
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("inst_scroll").show(ui, |ui| {
        match &app.project.channels[ci].instrument {
            Instrument::Sampler(_) => sampler_ui(app, ui, ci),
            Instrument::Synth(_) => synth_ui(app, ui, ci),
            Instrument::SoundFont(_) => soundfont_ui(app, ui, ci),
        }
        ui.add_space(8.0);
        mini_keyboard(app, ui, ci);
    });
    crate::ui::piano_roll::release_pending(app, ui.ctx());
}

fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) -> f32 {
    egui::Frame::default()
        .fill(BG_DARK)
        .rounding(5.0)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong().color(ACCENT));
            add(ui);
        })
        .response
        .rect
        .width()
}

type SectionFn<'a> = (&'static str, Box<dyn FnOnce(&mut Ui) + 'a>);

/// Lays sections out left-to-right and wraps into new rows. Frame widths are only known
/// after drawing, so the widths measured in the previous frame decide the line breaks.
fn flow(ui: &mut Ui, id: &str, items: Vec<SectionFn<'_>>) {
    let mem_id = egui::Id::new(("flow", id));
    let prev: Vec<f32> = ui.data(|d| d.get_temp(mem_id)).unwrap_or_default();
    let avail = ui.available_width();
    let gap = ui.spacing().item_spacing.x;
    let mut rows: Vec<Vec<SectionFn<'_>>> = vec![Vec::new()];
    let mut x = 0.0;
    for (i, it) in items.into_iter().enumerate() {
        let w = prev.get(i).copied().unwrap_or(220.0);
        if x > 0.0 && x + w > avail {
            rows.push(Vec::new());
            x = 0.0;
        }
        x += w + gap;
        rows.last_mut().unwrap().push(it);
    }
    let mut widths = Vec::new();
    for row in rows {
        ui.horizontal_top(|ui| {
            for (title, f) in row {
                widths.push(section(ui, title, f));
            }
        });
    }
    ui.data_mut(|d| d.insert_temp(mem_id, widths));
}

fn adsr_knobs(ui: &mut Ui, e: &mut AdsrParams) {
    knob(ui, &mut e.attack, 0.0005, 5.0, 0.002, "Attack", true, &widgets::fmt_s);
    knob(ui, &mut e.decay, 0.001, 5.0, 0.2, "Decay", true, &widgets::fmt_s);
    knob(ui, &mut e.sustain, 0.0, 1.0, 1.0, "Sustain", false, &widgets::fmt_pct);
    knob(ui, &mut e.release, 0.001, 8.0, 0.05, "Release", true, &widgets::fmt_s);
}

fn sampler_ui(app: &mut TStudio, ui: &mut Ui, ci: usize) {
    let bpm = app.project.bpm;
    let sr = app.engine.sample_rate;
    let (path, stretch) = match &app.project.channels[ci].instrument {
        Instrument::Sampler(s) => (s.sample.clone(), s.stretch.clone()),
        _ => return,
    };
    let loaded = path.as_ref().and_then(|p| app.cache.get_prepared(p, &stretch, bpm));
    let detected = path.as_ref().and_then(|p| app.library.analysis.get(p)).and_then(|a| a.bpm);
    let mut new_sample = None;
    let mut chop_action: Option<ChopAction> = None;
    let mut preview_root = false;

    section(ui, "Sample", |ui| {
        ui.horizontal(|ui| {
            ui.label(path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(kein Sample – aus der Library hierher ziehen)".into()));
            if ui.button("Datei wählen…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Audio", tstudio_library::AUDIO_EXTS).pick_file() {
                    new_sample = Some(p);
                }
            }
            if let Some(p) = &path {
                if ui.button("✂ Trim").clicked() {
                    app.fetch.open_in_trim(p);
                    app.view = View::Fetch;
                }
                if let Some(e) = app.cache.error(p) {
                    ui.colored_label(Color32::from_rgb(240, 110, 90), format!("⚠ {e}"));
                }
            }
        });
        // waveform with start/end/slice markers
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 130.0), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, Color32::from_rgb(24, 26, 30));
        let Instrument::Sampler(s) = &mut app.project.channels[ci].instrument else { return };
        if let Some(l) = &loaded {
            let len = l.data.len() as f64;
            widgets::draw_waveform(&p, rect.shrink2(vec2(0.0, 6.0)), &l.peaks, 0.0, len, Color32::from_rgb(110, 180, 240));
            let x_of = |n: f32| rect.left() + n * rect.width();
            // dim outside start..end
            p.rect_filled(Rect::from_min_max(rect.min, pos2(x_of(s.params.start), rect.bottom())), 0.0, Color32::from_black_alpha(140));
            p.rect_filled(Rect::from_min_max(pos2(x_of(s.params.end), rect.top()), rect.max), 0.0, Color32::from_black_alpha(140));
            // loop region (green) with its two markers
            let loop_col = Color32::from_rgb(90, 220, 130);
            if s.params.looped && s.params.mode == SamplerMode::Normal {
                let (a, b) = (x_of(s.params.loop_start.max(s.params.start)), x_of(s.params.loop_end.min(s.params.end)));
                p.rect_filled(Rect::from_min_max(pos2(a, rect.top()), pos2(b, rect.bottom())), 0.0, Color32::from_rgba_unmultiplied(90, 220, 130, 28));
                let xf = (s.params.loop_xfade * (b - a)).min(a - x_of(s.params.start)).max(0.0);
                if xf > 1.0 {
                    p.rect_filled(Rect::from_min_max(pos2(b - xf, rect.top()), pos2(b, rect.top() + 5.0)), 0.0, loop_col);
                }
                for (x, t) in [(a, "L"), (b, "L")] {
                    p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, loop_col));
                    p.text(pos2(x + 2.0, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, t, FontId::proportional(10.0), loop_col);
                }
            }
            for (i, sl) in s.params.slices.iter().enumerate() {
                let x = x_of(*sl);
                p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, ACCENT));
                p.text(pos2(x + 2.0, rect.top() + 2.0), Align2::LEFT_TOP, format!("{}", i + 1), FontId::proportional(10.0), ACCENT);
            }
            for x in [x_of(s.params.start), x_of(s.params.end)] {
                p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, Color32::WHITE));
            }
            p.text(rect.right_bottom() - vec2(4.0, 4.0), Align2::RIGHT_BOTTOM, format!("{:.2} s", l.data.duration_secs()), FontId::proportional(10.0), TEXT_DIM);
            // marker dragging
            if let Some(pt) = resp.interact_pointer_pos() {
                let n = ((pt.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                if resp.drag_started() {
                    // nearest marker within 8 px; loop markers only while looping
                    let mut cands = vec![(0u8, x_of(s.params.start)), (1, x_of(s.params.end))];
                    if s.params.looped && s.params.mode == SamplerMode::Normal {
                        cands.push((2, x_of(s.params.loop_start)));
                        cands.push((3, x_of(s.params.loop_end)));
                    }
                    app.instrument_ui.marker_drag = cands
                        .into_iter()
                        .map(|(i, x)| (i, (pt.x - x).abs()))
                        .filter(|(_, d)| *d < 8.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(i, _)| i);
                }
                if resp.dragged() {
                    match app.instrument_ui.marker_drag {
                        Some(0) => s.params.start = n.min(s.params.end - 0.001),
                        Some(1) => s.params.end = n.max(s.params.start + 0.001),
                        Some(2) => s.params.loop_start = n.min(s.params.loop_end - 0.001),
                        Some(3) => s.params.loop_end = n.max(s.params.loop_start + 0.001),
                        _ => {}
                    }
                }
            }
            if resp.drag_stopped() {
                // loop markers snap to the nearest zero crossing (fewer clicks at the seam)
                let mono = &l.data.left;
                match app.instrument_ui.marker_drag {
                    Some(2) => s.params.loop_start = snap_zero(mono, s.params.loop_start, l.data.sample_rate),
                    Some(3) => s.params.loop_end = snap_zero(mono, s.params.loop_end, l.data.sample_rate),
                    _ => {}
                }
                app.instrument_ui.marker_drag = None;
            }
            if resp.clicked() {
                preview_root = true;
            }
        } else if path.is_some() {
            p.text(rect.center(), Align2::CENTER_CENTER, "lädt…", FontId::proportional(13.0), TEXT_DIM);
        } else {
            p.text(rect.center(), Align2::CENTER_CENTER, "Sample hierher ziehen", FontId::proportional(13.0), TEXT_DIM);
        }
        if let Some(d) = resp.dnd_release_payload::<SampleDrag>() {
            new_sample = Some(d.path.clone());
        }
        resp.on_hover_text("Klick: Root-Note abspielen · weiße Marker ziehen: Start/Ende · grüne Marker: Loop-Bereich (rastet am Nulldurchgang ein)");
    });
    ui.add_space(6.0);

    let Instrument::Sampler(s) = &mut app.project.channels[ci].instrument else { return };
    ui.horizontal_top(|ui| {
        section(ui, "Wiedergabe", |ui| {
            ui.horizontal(|ui| {
                knob(ui, &mut s.params.pitch, -24.0, 24.0, 0.0, "Pitch", false, &widgets::fmt_semi);
                knob(ui, &mut s.params.gain, 0.0, 2.0, 1.0, "Gain", false, &widgets::fmt_gain);
                knob(ui, &mut s.params.velocity_sens, 0.0, 1.0, 0.7, "Velocity", false, &widgets::fmt_pct);
                ui.vertical(|ui| {
                    let mut root = s.params.root_note as i32;
                    ui.add(egui::DragValue::new(&mut root).range(0..=127).custom_formatter(|v, _| tstudio_dsp::note_name(v as u8)).prefix("Root "));
                    s.params.root_note = root as u8;
                    ui.checkbox(&mut s.params.one_shot, "One-Shot").on_hover_text("Spielt bis zum Ende, egal wie lang die Note ist");
                    if ui.checkbox(&mut s.params.looped, "Loop").on_hover_text("Hält den Sound, solange die Note läuft: der grüne Bereich wiederholt sich").changed()
                        && s.params.looped
                        && s.params.loop_start <= s.params.start
                        && s.params.loop_end >= s.params.end
                    {
                        // sensible starting loop: the back half, where sustained sounds are steady
                        s.params.loop_start = s.params.start + (s.params.end - s.params.start) * 0.5;
                        s.params.loop_end = s.params.start + (s.params.end - s.params.start) * 0.9;
                        s.params.loop_xfade = 0.25;
                    }
                    ui.checkbox(&mut s.params.reverse, "Rückwärts");
                    ui.checkbox(&mut s.params.mono, "Mono / Cut").on_hover_text("Neue Noten schneiden die vorherige ab");
                });
            });
        });
        section(ui, "Hüllkurve", |ui| {
            ui.horizontal(|ui| adsr_knobs(ui, &mut s.params.amp_env));
        });
        if s.params.looped {
            section(ui, "Loop", |ui| {
                ui.horizontal(|ui| {
                    knob(ui, &mut s.params.loop_start, 0.0, 1.0, 0.5, "Start", false, &widgets::fmt_pct);
                    knob(ui, &mut s.params.loop_end, 0.0, 1.0, 0.9, "Ende", false, &widgets::fmt_pct);
                    knob(ui, &mut s.params.loop_xfade, 0.0, 0.5, 0.25, "X-Fade", false, &widgets::fmt_pct);
                    if s.params.loop_end < s.params.loop_start + 0.001 {
                        s.params.loop_end = (s.params.loop_start + 0.001).min(1.0);
                    }
                    ui.vertical(|ui| {
                        ui.checkbox(&mut s.params.loop_tail, "Ausklingen").on_hover_text("Nach dem Loslassen den Loop verlassen und das Sample zu Ende spielen");
                        if ui.small_button("Ganzes Sample").clicked() {
                            s.params.loop_start = s.params.start;
                            s.params.loop_end = s.params.end;
                        }
                    });
                });
            });
        }
    });
    ui.add_space(6.0);
    section(ui, "Tempo / BPM", |ui| {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("smp_stretch").selected_text(s.stretch.mode.name()).show_ui(ui, |ui| {
                for m in StretchMode::ALL {
                    ui.selectable_value(&mut s.stretch.mode, m, m.name());
                }
            });
            ui.label("Sample-BPM");
            ui.add(egui::DragValue::new(&mut s.stretch.source_bpm).speed(0.1).range(20.0..=300.0).fixed_decimals(2));
            if ui.small_button("×2").clicked() {
                s.stretch.source_bpm *= 2.0;
            }
            if ui.small_button("÷2").clicked() {
                s.stretch.source_bpm /= 2.0;
            }
            if let Some(d) = detected {
                if ui.small_button(format!("erkannt: {d:.1}")).clicked() {
                    s.stretch.source_bpm = d;
                }
            }
            let mut follow = s.stretch.target_bpm.is_none();
            if ui.checkbox(&mut follow, format!("Ziel = Projekt ({bpm:.1})")).changed() {
                s.stretch.target_bpm = if follow { None } else { Some(bpm) };
            }
            if let Some(t) = s.stretch.target_bpm.as_mut() {
                ui.add(egui::DragValue::new(t).speed(0.1).range(20.0..=300.0).prefix("Ziel ").fixed_decimals(2));
            }
            ui.add(egui::DragValue::new(&mut s.stretch.pitch).speed(0.05).range(-24.0..=24.0).prefix("Pitch ").suffix(" st"));
            if s.stretch.pitch != 0.0 {
                ui.checkbox(&mut s.stretch.formant, "Formant halten").on_hover_text("Stimmen behalten ihren Charakter beim Pitchen");
            }
        });
        ui.weak("Stretch hält die Tonhöhe, Resample verhält sich wie eine Platte (schneller = höher). Pitch verschiebt die Tonhöhe zusätzlich – in jedem Modus, ohne die Länge zu ändern.");
    });
    ui.add_space(6.0);
    section(ui, "Chop / Slices", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut s.params.mode, SamplerMode::Normal, "Normal (Tonhöhe per Taste)");
            ui.selectable_value(&mut s.params.mode, SamplerMode::Chop, "Chop (Slice pro Taste ab C4)");
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::Slider::new(&mut app.instrument_ui.chop_sensitivity, 0.0..=1.0).text("Empfindlichkeit"));
            if ui.button("Auto-Chop (Transienten)").clicked() {
                chop_action = Some(ChopAction::Transients);
            }
            for n in [4usize, 8, 16, 32] {
                if ui.button(format!("÷{n}")).on_hover_text(format!("In {n} gleiche Teile")).clicked() {
                    chop_action = Some(ChopAction::Even(n));
                }
            }
            if ui.button("Slices löschen").clicked() {
                chop_action = Some(ChopAction::Clear);
            }
            if ui.add_enabled(!s.params.slices.is_empty(), egui::Button::new("Slices → Pattern")).on_hover_text("Schreibt die Slices als Noten ins aktuelle Pattern").clicked() {
                chop_action = Some(ChopAction::ToPattern);
            }
        });
        ui.weak(format!("{} Slices", s.params.slices.len()));
    });

    // ---- apply deferred actions
    if let Some(p) = new_sample {
        if let Instrument::Sampler(s) = &mut app.project.channels[ci].instrument {
            s.sample = Some(p.clone());
            s.params.slices.clear();
            if let Some(a) = app.library.analysis.get(&p) {
                if let Some(b) = a.bpm {
                    s.stretch.source_bpm = b;
                }
            }
        }
        app.project.channels[ci].name = p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        app.cache.get_raw(&p);
    }
    if let (Some(action), Some(l)) = (chop_action, loaded.as_ref()) {
        let sens = app.instrument_ui.chop_sensitivity;
        let len = l.data.len().max(1);
        let cid = app.project.channels[ci].id;
        let total_ticks = app.project.secs_to_ticks(l.data.duration_secs());
        let pid = app.current_pattern;
        let mut notes_to_write = None;
        if let Instrument::Sampler(s) = &mut app.project.channels[ci].instrument {
            match action {
                ChopAction::Transients => {
                    let mono = l.data.mono();
                    let pos = tstudio_dsp::analysis::transients(&mono, l.data.sample_rate, sens, 70.0);
                    s.params.slices = pos.iter().map(|p| *p as f32 / len as f32).collect();
                    s.params.mode = SamplerMode::Chop;
                }
                ChopAction::Even(n) => {
                    s.params.slices = (0..n).map(|i| s.params.start + (s.params.end - s.params.start) * i as f32 / n as f32).collect();
                    s.params.mode = SamplerMode::Chop;
                }
                ChopAction::Clear => {
                    s.params.slices.clear();
                    s.params.mode = SamplerMode::Normal;
                }
                ChopAction::ToPattern => {
                    let sl = &s.params.slices;
                    let end = s.params.end;
                    let root = s.params.slice_root;
                    let notes: Vec<Note> = sl
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            let b = sl.get(i + 1).copied().unwrap_or(end);
                            let start = (*a as f64 * total_ticks).round() as i64;
                            let stop = (b as f64 * total_ticks).round() as i64;
                            Note { channel: cid, key: root.saturating_add(i as u8), start, len: (stop - start).max(6), velocity: 0.8 }
                        })
                        .collect();
                    s.params.mode = SamplerMode::Chop;
                    s.params.mono = true;
                    notes_to_write = Some(notes);
                }
            }
        }
        if let Some(notes) = notes_to_write {
            if let Some(p) = app.project.pattern_mut(pid) {
                p.notes.retain(|n| n.channel != cid);
                p.notes.extend(notes);
            }
            app.set_status("Slices ins Pattern geschrieben");
        }
    }
    if preview_root {
        if let Instrument::Sampler(s) = &app.project.channels[ci].instrument {
            let k = if s.params.mode == SamplerMode::Chop { s.params.slice_root } else { s.params.root_note };
            app.note_on(ci, k, 0.8);
            app.piano.pending_off.push((ci, k));
        }
    }
    let _ = sr;
    let _ = PPQ;
}

#[derive(Clone, Copy)]
enum ChopAction {
    Transients,
    Even(usize),
    Clear,
    ToPattern,
}

fn synth_ui(app: &mut TStudio, ui: &mut Ui, ci: usize) {
    crate::ui::tsynth::show(app, ui, ci);
}

fn soundfont_ui(app: &mut TStudio, ui: &mut Ui, ci: usize) {
    let Instrument::SoundFont(sf) = &app.project.channels[ci].instrument else { return };
    let path = sf.path.clone();
    if app.instrument_ui.sf_presets.as_ref().map(|(p, _)| p != &path).unwrap_or(true) {
        app.instrument_ui.sf_presets = Some((path.clone(), tstudio_engine::soundfont_presets(&path)));
    }
    let mut new_file = None;
    section(ui, "SoundFont", |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()).strong()).on_hover_text(path.display().to_string());
            if ui.button("Datei…").clicked() {
                new_file = rfd::FileDialog::new().add_filter("SoundFont", &["sf2", "SF2"]).pick_file();
            }
        });
    });
    if let Some(f) = new_file {
        if let Instrument::SoundFont(sf) = &mut app.project.channels[ci].instrument {
            sf.path = f;
            sf.bank = 0;
            sf.program = 0;
            sf.preset_name.clear();
        }
        app.instrument_ui.sf_presets = None;
        return;
    }
    let st = &mut app.instrument_ui;
    let Some((_, res)) = &st.sf_presets else { return };
    let presets = match res {
        Ok(p) => p.clone(),
        Err(e) => {
            ui.colored_label(Color32::from_rgb(240, 110, 90), format!("⚠ {e}"));
            return;
        }
    };
    let filter = &mut st.sf_filter;
    let ch = &mut app.project.channels[ci];
    let ch_name = &mut ch.name;
    let Instrument::SoundFont(sf) = &mut ch.instrument else { return };
    let cur = presets.iter().position(|p| p.bank == sf.bank && p.program == sf.program);
    let mut pick: Option<usize> = None;
    section(ui, "Preset", |ui| {
        ui.horizontal(|ui| {
            if ui.button("◀").on_hover_text("vorheriges Preset").clicked() {
                pick = Some(cur.map(|i| i.saturating_sub(1)).unwrap_or(0));
            }
            let label = cur.map(|i| format!("{:03}:{:03}  {}", presets[i].bank, presets[i].program, presets[i].name)).unwrap_or_else(|| format!("{}:{} (fehlt)", sf.bank, sf.program));
            egui::ComboBox::from_id_salt("sf_preset").selected_text(label).width(300.0).height(420.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(filter).hint_text("suchen…"));
                let f = filter.to_lowercase();
                for (i, p) in presets.iter().enumerate() {
                    if !f.is_empty() && !p.name.to_lowercase().contains(&f) {
                        continue;
                    }
                    let txt = format!("{:03}:{:03}  {}{}", p.bank, p.program, p.name, if p.bank >= 128 { "  🥁" } else { "" });
                    if ui.selectable_label(Some(i) == cur, txt).clicked() {
                        pick = Some(i);
                    }
                }
            });
            if ui.button("▶").on_hover_text("nächstes Preset").clicked() {
                pick = Some(cur.map(|i| (i + 1).min(presets.len().saturating_sub(1))).unwrap_or(0));
            }
            ui.weak(format!("{} Presets", presets.len()));
        });
        ui.horizontal(|ui| {
            knob(ui, &mut sf.gain, 0.0, 4.0, 1.0, "Gain", false, &widgets::fmt_gain);
            if sf.bank >= 128 {
                ui.label(RichText::new("Drum-Kit (GM): C2 = Kick · D2 = Snare · F#2 = HiHat zu · A#2 = HiHat offen · C#3 = Crash").small().color(TEXT_DIM));
            }
        });
    });
    if let Some(i) = pick {
        if let Some(p) = presets.get(i) {
            let renamed = *ch_name == sf.preset_name || ch_name.is_empty();
            sf.bank = p.bank;
            sf.program = p.program;
            sf.preset_name = p.name.clone();
            if renamed {
                *ch_name = p.name.clone();
            }
            let key = app.project.channels[ci].instrument.default_key();
            app.note_on(ci, key, 0.8);
            app.piano.pending_off.push((ci, key));
        }
    }
}

pub(crate) fn user_presets(dir: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut v: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "ron").unwrap_or(false))
                .map(|p| (p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), p))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Two-octave clickable keyboard for auditioning.
fn mini_keyboard(app: &mut TStudio, ui: &mut Ui, ci: usize) {
    let base: u8 = 48;
    let whites: Vec<u8> = (base..base + 25).filter(|k| !matches!(k % 12, 1 | 3 | 6 | 8 | 10)).collect();
    let kw = 26.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(kw * whites.len() as f32, 70.0), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    let mut black_rects = Vec::new();
    let mut white_rects = Vec::new();
    for (i, k) in whites.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + i as f32 * kw, rect.top()), vec2(kw - 1.0, rect.height()));
        white_rects.push((*k, r));
        if !matches!(k % 12, 4 | 11) && *k + 1 < base + 25 {
            black_rects.push((k + 1, Rect::from_min_size(pos2(r.right() - kw * 0.3, rect.top()), vec2(kw * 0.6, rect.height() * 0.6))));
        }
    }
    for (k, r) in &white_rects {
        p.rect_filled(*r, 2.0, Color32::from_gray(225));
        if k % 12 == 0 {
            p.text(pos2(r.center().x, r.bottom() - 8.0), Align2::CENTER_CENTER, tstudio_dsp::note_name(*k), FontId::proportional(9.0), Color32::from_gray(80));
        }
    }
    for (_, r) in &black_rects {
        p.rect_filled(*r, 2.0, Color32::from_gray(25));
    }
    if resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
        if let Some(pt) = resp.interact_pointer_pos() {
            let key = black_rects.iter().find(|(_, r)| r.contains(pt)).or_else(|| white_rects.iter().find(|(_, r)| r.contains(pt))).map(|(k, _)| *k);
            if let Some(k) = key {
                app.note_on(ci, k, 0.85);
                app.piano.pending_off.push((ci, k));
            }
        }
    }
    let _ = TEXT_DIM;
}

/// Moves a 0..1 position to the nearest rising zero crossing within ±5 ms.
fn snap_zero(buf: &[f32], n: f32, sr: u32) -> f32 {
    let len = buf.len();
    if len < 4 {
        return n;
    }
    let c = ((n as f64 * len as f64) as usize).min(len - 2);
    let w = (sr as usize / 200).max(1);
    let best = (c.saturating_sub(w)..(c + w).min(len - 1))
        .filter(|&i| buf[i] <= 0.0 && buf[i + 1] > 0.0)
        .min_by_key(|&i| i.abs_diff(c));
    best.map(|i| i as f32 / len as f32).unwrap_or(n)
}
