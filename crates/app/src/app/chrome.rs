//! Window chrome: shortcuts, drag & drop, MIDI input, transport bar, menu bar, status bar.

use super::*;

impl TStudio {
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let (space, z, y, s, o, n, l, f5, f6, f7, f8, f9, f10, ctrl, shift, r) = ctx.input(|i| {
            (
                i.key_pressed(Key::Space),
                i.key_pressed(Key::Z),
                i.key_pressed(Key::Y),
                i.key_pressed(Key::S),
                i.key_pressed(Key::O),
                i.key_pressed(Key::N),
                i.key_pressed(Key::L),
                i.key_pressed(Key::F5),
                i.key_pressed(Key::F6),
                i.key_pressed(Key::F7),
                i.key_pressed(Key::F8),
                i.key_pressed(Key::F9),
                i.key_pressed(Key::F10),
                i.modifiers.command,
                i.modifiers.shift,
                i.key_pressed(Key::R),
            )
        });
        // the sample cutter uses Space / L itself while a file is loaded
        let cutter_keys = self.view == View::Fetch && self.fetch.trim.data.is_some();
        if space && !cutter_keys {
            self.play_toggle();
        }
        if ctrl && z && !shift {
            self.undo();
        }
        if ctrl && (y || (z && shift)) {
            self.redo();
        }
        if ctrl && s {
            if shift {
                self.save_as();
            } else {
                self.save();
            }
        }
        if ctrl && o {
            self.open_dialog();
        }
        if ctrl && n {
            self.new_project();
        }
        if l && !ctrl && !cutter_keys {
            self.song_mode = !self.song_mode;
        }
        if r && ctrl && shift {
            ui::record::toggle(self);
        } else if r && ctrl {
            self.recording = !self.recording;
        }
        for (pressed, v) in [(f5, View::Playlist), (f6, View::Rack), (f7, View::PianoRoll), (f8, View::Instrument), (f9, View::Mixer), (f10, View::Fetch)] {
            if pressed {
                self.view = v;
            }
        }
    }

    pub(super) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if dropped.is_empty() {
            return;
        }
        for p in dropped {
            if p.extension().map(|e| e.eq_ignore_ascii_case("tsp")).unwrap_or(false) {
                if self.confirm_discard() {
                    self.open_path(&p);
                }
                return;
            }
            if tstudio_library::is_soundfont_file(&p) {
                self.add_soundfont_channel(&p);
                continue;
            }
            if !tstudio_library::is_audio_file(&p) {
                continue;
            }
            match self.view {
                View::Playlist => {
                    let start = crate::ui::widgets::floor_to(self.position() as i64, self.project.ticks_per_bar());
                    let track = self.playlist_ui.first_free_track(&self.project, start);
                    self.add_audio_clip(&p, track, start);
                    self.song_mode = true;
                }
                View::Fetch => self.fetch.open_in_trim(&p),
                _ => {
                    self.add_sampler_channel(&p);
                }
            }
        }
        self.set_status("Datei(en) hinzugefügt");
    }

    pub(super) fn handle_midi(&mut self) {
        let msgs: Vec<MidiMsg> = self.midi.rx.try_iter().collect();
        if msgs.is_empty() {
            return;
        }
        let pos = self.position();
        let playing = self.is_playing();
        let pat_len = self.project.pattern_length(self.current_pattern) as f64;
        for m in msgs {
            match m {
                MidiMsg::NoteOn(k, v) => {
                    self.piano.flash_key = Some((k, Instant::now()));
                    if self.recording && playing && !self.song_mode {
                        self.rec_notes.insert(k, (pos, v));
                    }
                }
                MidiMsg::NoteOff(k) => {
                    if let Some((start, vel)) = self.rec_notes.remove(&k) {
                        let mut len = pos - start;
                        if len <= 0.0 {
                            len += pat_len;
                        }
                        let ch = self.selected_channel;
                        if let Some(cid) = self.project.channels.get(ch).map(|c| c.id) {
                            let snap = self.piano.snap.ticks(self.project.ticks_per_bar());
                            let start_q = crate::ui::widgets::round_to(start.round() as i64, snap).rem_euclid(pat_len as i64);
                            if let Some(p) = self.project.pattern_mut(self.current_pattern) {
                                p.notes.push(Note { channel: cid, key: k, start: start_q, len: (len.round() as i64).max(6), velocity: vel });
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn transport_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let playing = self.is_playing();
            let play_txt = if playing { RichText::new("⏸").size(18.0).color(ACCENT) } else { RichText::new("▶").size(18.0) };
            if ui.add(egui::Button::new(play_txt).min_size(egui::vec2(34.0, 26.0))).on_hover_text("Play / Stop (Leertaste)").clicked() {
                self.play_toggle();
            }
            if ui.add(egui::Button::new(RichText::new("■").size(16.0)).min_size(egui::vec2(30.0, 26.0))).on_hover_text("Stop").clicked() {
                self.send(Command::Stop);
            }
            let rec_col = if self.recording { Color32::from_rgb(240, 70, 60) } else { TEXT_DIM };
            if ui
                .add(egui::Button::new(RichText::new("⏺").size(16.0).color(rec_col)).min_size(egui::vec2(30.0, 26.0)))
                .on_hover_text("MIDI-Aufnahme in das aktuelle Pattern (Strg+R)")
                .clicked()
            {
                self.recording = !self.recording;
            }
            ui::record::transport_button(self, ui);
            ui.separator();
            let pat_sel = ui.selectable_label(!self.song_mode, RichText::new("PAT").strong()).on_hover_text("Pattern-Modus: aktuelles Pattern loopen (L)");
            if pat_sel.clicked() {
                self.song_mode = false;
            }
            if ui.selectable_label(self.song_mode, RichText::new("SONG").strong()).on_hover_text("Song-Modus: Playlist abspielen (L)").clicked() {
                self.song_mode = true;
            }
            ui.separator();
            ui.label("BPM");
            ui.add(egui::DragValue::new(&mut self.project.bpm).speed(0.1).range(20.0..=300.0).fixed_decimals(2));
            ui.toggle_value(&mut self.metronome, "🔔").on_hover_text("Metronom");
            ui.separator();
            // master volume (= mixer master fader), in dB
            ui.label("Master");
            let master = &mut self.project.mixer[0].volume;
            let mut db = tstudio_dsp::gain_to_db(*master).max(-60.0);
            let r = ui
                .add(egui::Slider::new(&mut db, -60.0..=6.0).show_value(false).trailing_fill(true))
                .on_hover_text("Master-Lautstärke (Doppelklick: 0 dB)");
            if r.changed() {
                *master = if db <= -59.9 { 0.0 } else { tstudio_dsp::db_to_gain(db) };
            }
            if r.double_clicked() {
                *master = 1.0;
            }
            ui.label(RichText::new(widgets::fmt_gain(*master)).monospace().small());
            ui.label("Swing");
            let mut sw = self.project.swing * 100.0;
            if ui
                .add(egui::DragValue::new(&mut sw).range(0.0..=100.0).speed(0.5).suffix("%").fixed_decimals(0))
                .on_hover_text("Verzögert jede zweite 16tel (Shuffle). 0 % = gerade")
                .changed()
            {
                self.project.swing = (sw / 100.0).clamp(0.0, 1.0);
            }
            ui.separator();
            let pos = self.position();
            // song: bar/beat follow time-signature changes; patterns use the project meter
            let bars = if self.song_mode { self.project.bar_map() } else { tstudio_project::BarMap::new(self.project.beats_per_bar, &[]) };
            let b = bars.tick_to_bar(pos).floor() as i64 + 1;
            let (bar_start, _) = bars.bar_bounds(pos as i64);
            let beat_t = bars.meter_at(pos).beat_ticks as f64;
            let in_bar = pos - bar_start as f64;
            let beat = (in_bar / beat_t).floor() as i64 + 1;
            let step = ((in_bar % beat_t) / (PPQ as f64 / 4.0)).floor() as i64 + 1;
            ui.label(RichText::new(format!("{b:>3}:{beat}:{step}")).monospace().size(15.0).color(ACCENT));
            if self.song_mode && !self.project.tempo.is_empty() {
                ui.label(RichText::new(format!("♩ {:.1}", self.project.bpm_at(pos as i64))).monospace().color(TEXT_DIM)).on_hover_text("aktuelles Tempo (Tempo-Spur)");
            }
            ui.separator();
            ui::rack::pattern_selector(self, ui);
            ui.separator();
            for (v, name, key) in View::ALL {
                if ui.selectable_label(self.view == v, name).on_hover_text(key).clicked() {
                    self.view = v;
                }
            }
            ui.separator();
            if ui.add_enabled(self.history.can_undo() || self.project != self.saved, egui::Button::new("↶")).on_hover_text("Rückgängig (Strg+Z)").clicked() {
                self.undo();
            }
            if ui.add_enabled(self.history.can_redo(), egui::Button::new("↷")).on_hover_text("Wiederholen (Strg+Y)").clicked() {
                self.redo();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cpu = self.engine.shared.cpu_load();
                let col = if cpu > 0.8 { Color32::from_rgb(240, 80, 60) } else { TEXT_DIM };
                ui.label(RichText::new(format!("CPU {:>3.0}%", cpu * 100.0)).small().color(col));
                let (l, r) = self.meters.first().copied().unwrap_or((0.0, 0.0));
                let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 24.0), egui::Sense::hover());
                widgets::draw_meter(ui.painter(), rect, l, r);
            });
        });
    }

    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            widgets::logo(ui, 18.0);
            ui.menu_button("Datei", |ui| {
                if ui.button("Neu  (Strg+N)").clicked() {
                    ui.close_menu();
                    self.new_project();
                }
                ui.menu_button("Neu aus Vorlage", |ui| self.template_menu(ui));
                if ui.button("Öffnen…  (Strg+O)").clicked() {
                    ui.close_menu();
                    self.open_dialog();
                }
                ui.menu_button("Zuletzt geöffnet", |ui| {
                    let recent = self.settings.recent_projects.clone();
                    if recent.is_empty() {
                        ui.weak("(leer)");
                    }
                    for p in recent {
                        if ui.button(p.display().to_string()).clicked() {
                            ui.close_menu();
                            if self.confirm_discard() {
                                self.open_path(&p);
                            }
                        }
                    }
                });
                if ui.button("Speichern  (Strg+S)").clicked() {
                    ui.close_menu();
                    self.save();
                }
                if ui.button("Speichern unter…  (Strg+Shift+S)").clicked() {
                    ui.close_menu();
                    self.save_as();
                }
                if ui.button("Als Vorlage speichern…").clicked() {
                    ui.close_menu();
                    self.ask_template_name();
                }
                ui.separator();
                if ui.button("Exportieren (WAV/MP3)…").clicked() {
                    ui.close_menu();
                    self.export.open = true;
                }
                ui.menu_button("MIDI", |ui| self.midi_menu(ui));
                if ui.button("Backups wiederherstellen…").clicked() {
                    ui.close_menu();
                    self.backups.show_window = true;
                }
                ui.separator();
                if ui.button("Einstellungen…").clicked() {
                    ui.close_menu();
                    self.show_settings = true;
                }
            });
            ui.menu_button("Bearbeiten", |ui| {
                if ui.button("Rückgängig  (Strg+Z)").clicked() {
                    ui.close_menu();
                    self.undo();
                }
                if ui.button("Wiederholen  (Strg+Y)").clicked() {
                    ui.close_menu();
                    self.redo();
                }
            });
            ui.menu_button("Ansicht", |ui| {
                ui.checkbox(&mut self.show_library, "Sample-Library");
                ui.checkbox(&mut self.settings.follow_playhead, "Playhead folgen");
            });
            ui.menu_button("Hinzufügen", |ui| {
                if ui.button("TSynth-Kanal").clicked() {
                    ui.close_menu();
                    let i = self.project.add_channel("TSynth", Instrument::Synth(Default::default()));
                    self.selected_channel = i;
                }
                if ui.button("Sampler-Kanal (Datei wählen…)").clicked() {
                    ui.close_menu();
                    if let Some(p) = rfd::FileDialog::new().add_filter("Audio", tstudio_library::AUDIO_EXTS).pick_file() {
                        self.add_sampler_channel(&p);
                    }
                }
                if ui.button("SoundFont-Kanal (.sf2 wählen…)").clicked() {
                    ui.close_menu();
                    if let Some(p) = rfd::FileDialog::new().add_filter("SoundFont", &["sf2", "SF2"]).pick_file() {
                        self.add_soundfont_channel(&p);
                    }
                }
                if ui.button("Neues Pattern").clicked() {
                    ui.close_menu();
                    self.current_pattern = self.project.add_pattern();
                }
            });
            ui.menu_button("Hilfe", |ui| {
                ui.label(RichText::new("Tastenkürzel").strong());
                for (k, v) in [
                    ("Leertaste", "Play / Stop"),
                    ("L", "Pattern ↔ Song"),
                    ("F5 / F6 / F7", "Playlist / Channel Rack / Piano Roll"),
                    ("F8 / F9 / F10", "Instrument / Mixer / Sample-Cutter"),
                    ("Strg+Z / Strg+Y", "Rückgängig / Wiederholen"),
                    ("Piano Roll: Strg+Ziehen", "Auswahlrechteck"),
                    ("Piano Roll: Entf, Strg+A/C/V/D, ↑↓, Q", "Löschen, Auswahl, Kopieren, Einfügen, Duplizieren, Transponieren, Quantisieren"),
                    ("Rechtsklick", "Note/Clip löschen"),
                    ("Strg+Mausrad", "Zoom"),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(k).monospace());
                        ui.weak(v);
                    });
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = if self.is_dirty() { " •" } else { "" };
                ui.weak(format!("{}{}", self.project.name, dirty));
            });
        });
    }

    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some((msg, t, err)) = &self.status {
                if t.elapsed() < Duration::from_secs(8) {
                    let col = if *err { Color32::from_rgb(240, 110, 90) } else { TEXT_DIM };
                    ui.label(RichText::new(msg).color(col).small());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} · {} Hz · Samples {:.0} MB{}",
                        self.engine.device_name,
                        self.engine.sample_rate,
                        self.cache.memory_mb(),
                        self.midi.port_name.as_ref().map(|p| format!(" · MIDI: {p}")).unwrap_or_default()
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
                if self.engine.shared.stream_error.load(Ordering::Relaxed) && ui.small_button("Audio neu starten").clicked() {
                    self.restart_engine();
                }
            });
        });
    }

    pub(super) fn update_meters(&mut self) {
        let n = self.project.mixer.len();
        self.meters.resize(n, (0.0, 0.0));
        for (i, m) in self.meters.iter_mut().enumerate() {
            let (l, r) = self.engine.shared.take_meter(i);
            m.0 = l.max(m.0 * 0.85);
            m.1 = r.max(m.1 * 0.85);
        }
    }
}
