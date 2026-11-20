//! Settings window: audio, MIDI, library folders, external tools.

use crate::app::TStudio;
use crate::midi::Midi;
use egui::RichText;

#[derive(Default)]
pub struct SettingsUiState {
    devices: Option<Vec<String>>,
    midi_ports: Option<Vec<String>>,
    /// Device enumeration can take seconds on Windows, so it runs off the UI thread.
    scanning: Option<crossbeam_channel::Receiver<(Vec<String>, Vec<String>)>>,
}

fn start_scan(ctx: &egui::Context) -> crossbeam_channel::Receiver<(Vec<String>, Vec<String>)> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send((tstudio_engine::list_output_devices(), Midi::ports()));
        ctx.request_repaint();
    });
    rx
}

pub fn window(app: &mut TStudio, ctx: &egui::Context) {
    if !app.show_settings {
        app.settings_ui = SettingsUiState::default();
        return;
    }
    let su = &mut app.settings_ui;
    if su.scanning.is_none() && (su.devices.is_none() || su.midi_ports.is_none()) {
        su.scanning = Some(start_scan(ctx));
    }
    if let Some(rx) = &su.scanning {
        if let Ok((d, m)) = rx.try_recv() {
            su.devices = Some(d);
            su.midi_ports = Some(m);
            su.scanning = None;
        }
    }
    let scanning = su.scanning.is_some();
    let mut open = true;
    let mut restart = false;
    let mut rescan = false;
    let mut redetect = false;
    let mut midi_change: Option<Option<String>> = None;
    let mut open_learn = false;
    let before = app.settings.clone();

    egui::Window::new("Einstellungen").open(&mut open).default_width(520.0).collapsible(false).show(ctx, |ui| {
        let s = &mut app.settings;
        let devices = app.settings_ui.devices.clone().unwrap_or_default();
        let ports = app.settings_ui.midi_ports.clone().unwrap_or_default();

        ui.heading("Audio");
        egui::Grid::new("set_audio").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Ausgabegerät");
            let cur = s.audio_device.clone().unwrap_or_else(|| "Standard".into());
            egui::ComboBox::from_id_salt("set_dev").selected_text(cur).width(320.0).show_ui(ui, |ui| {
                if ui.selectable_label(s.audio_device.is_none(), "Standard").clicked() {
                    s.audio_device = None;
                    restart = true;
                }
                for d in &devices {
                    if ui.selectable_label(s.audio_device.as_deref() == Some(d), d).clicked() {
                        s.audio_device = Some(d.clone());
                        restart = true;
                    }
                }
            });
            ui.end_row();
            ui.label("Puffergröße");
            let cur = s.buffer_size.map(|b| format!("{b} Samples")).unwrap_or_else(|| "Standard".into());
            egui::ComboBox::from_id_salt("set_buf").selected_text(cur).show_ui(ui, |ui| {
                if ui.selectable_label(s.buffer_size.is_none(), "Standard").clicked() {
                    s.buffer_size = None;
                    restart = true;
                }
                for b in [64u32, 128, 256, 512, 1024, 2048] {
                    if ui.selectable_label(s.buffer_size == Some(b), format!("{b} Samples")).clicked() {
                        s.buffer_size = Some(b);
                        restart = true;
                    }
                }
            });
            ui.end_row();
            ui.label("Status");
            let lat = s.buffer_size.map(|b| format!(" · ~{:.1} ms", b as f32 * 1000.0 / app.engine.sample_rate as f32)).unwrap_or_default();
            ui.label(match &app.engine.status {
                Some(e) => RichText::new(e).color(egui::Color32::from_rgb(240, 110, 90)),
                None => RichText::new(format!("{} @ {} Hz{lat}", app.engine.device_name, app.engine.sample_rate)).weak(),
            });
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui.button("Audio neu starten").clicked() {
                restart = true;
            }
            if ui.add_enabled(!scanning, egui::Button::new("Geräte neu suchen")).clicked() {
                app.settings_ui.devices = None;
            }
            if scanning {
                ui.spinner();
                ui.weak("suche Geräte…");
            }
        });

        ui.separator();
        ui.heading("MIDI-Eingang");
        ui.horizontal(|ui| {
            let cur = app.midi.port_name.clone().unwrap_or_else(|| "Kein".into());
            egui::ComboBox::from_id_salt("set_midi").selected_text(cur).width(320.0).show_ui(ui, |ui| {
                if ui.selectable_label(app.midi.port_name.is_none(), "Kein").clicked() {
                    midi_change = Some(None);
                }
                for p in &ports {
                    if ui.selectable_label(app.midi.port_name.as_deref() == Some(p), p).clicked() {
                        midi_change = Some(Some(p.clone()));
                    }
                }
            });
            if ui.button("⟳").on_hover_text("Ports neu suchen").clicked() {
                app.settings_ui.midi_ports = None;
            }
        });
        if let Some(e) = &app.midi.error {
            ui.colored_label(egui::Color32::from_rgb(240, 110, 90), e);
        }
        if ports.is_empty() && !scanning {
            ui.weak("Kein MIDI-Gerät gefunden.");
        }
        ui.horizontal(|ui| {
            if ui.button("🎛 MIDI-Learn…").on_hover_text("Regler/Fader des Controllers Mixer-Lautstärke/Pan und TSynth-Makros/Cutoff zuweisen").clicked() {
                open_learn = true;
            }
            ui.weak(format!("{} Zuweisung(en)", s.midi_mappings.len()));
        });

        ui.separator();
        ui.heading("Sample-Library");
        let mut remove = None;
        for (i, r) in s.library_roots.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(r.display().to_string()).monospace());
                if !r.exists() {
                    ui.colored_label(egui::Color32::from_rgb(240, 110, 90), "(fehlt)");
                }
                if ui.small_button("✖").on_hover_text("Ordner entfernen").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            s.library_roots.remove(i);
            rescan = true;
        }
        ui.horizontal(|ui| {
            if ui.button("➕ Ordner hinzufügen…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    if !s.library_roots.contains(&d) {
                        s.library_roots.push(d);
                        rescan = true;
                    }
                }
            }
            if ui.button("Neu scannen").clicked() {
                rescan = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Speicherort für Downloads/Cuts:");
            ui.label(RichText::new(s.output_dir().display().to_string()).monospace());
            if ui.small_button("Ändern…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    s.sample_out_dir = Some(d);
                    rescan = true;
                }
            }
        });

        ui.separator();
        ui.heading("Vorhören");
        ui.horizontal(|ui| {
            ui.label("Lautstärke");
            ui.add(egui::Slider::new(&mut s.preview_volume, 0.0..=1.0).show_value(false));
            ui.checkbox(&mut s.auto_preview, "Beim Anklicken abspielen");
            ui.checkbox(&mut s.preview_synced, "Tempo-synchron");
        });
        ui.checkbox(&mut s.follow_playhead, "Playlist/Piano Roll folgt dem Playhead");

        ui.separator();
        ui.heading("Externe Tools");
        for (name, path, hint) in [("ffmpeg", &mut s.ffmpeg_path, "winget install Gyan.FFmpeg"), ("yt-dlp", &mut s.ytdlp_path, "winget install yt-dlp.yt-dlp")] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).strong());
                match path {
                    Some(p) => {
                        ui.label(RichText::new(p.display().to_string()).monospace());
                        if ui.small_button("✖").on_hover_text("Automatisch suchen").clicked() {
                            *path = None;
                            redetect = true;
                        }
                    }
                    None => {
                        ui.weak("automatisch (PATH / WinGet)");
                    }
                }
                if ui.small_button("Pfad…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter(name, &["exe"]).pick_file() {
                        *path = Some(p);
                        redetect = true;
                    }
                }
            });
            ui.weak(format!("Installation: {hint}"));
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Stem-Trennung (Demucs, nur CPU)").strong());
        ui.horizontal(|ui| {
            ui.label("Python");
            match &s.python_path {
                Some(p) => {
                    ui.label(RichText::new(p.display().to_string()).monospace());
                    if ui.small_button("✖").on_hover_text("Automatisch suchen").clicked() {
                        s.python_path = None;
                    }
                }
                None => {
                    ui.weak(match tstudio_fetch::stems::find_python(None) {
                        Some(p) => format!("automatisch: {}", p.display()),
                        None => "nicht gefunden".into(),
                    });
                }
            }
            if ui.small_button("Pfad…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("python", &["exe"]).pick_file() {
                    s.python_path = Some(p);
                }
            }
        });
        ui.horizontal(|ui| {
            let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) as u32;
            ui.label("CPU-Kerne");
            ui.add(egui::Slider::new(&mut s.stem_threads, 0..=cores).custom_formatter(move |v, _| if v == 0.0 { format!("Auto ({})", (cores / 2).max(1)) } else { format!("{v}") }));
            ui.weak("weniger = PC bleibt flüssiger, dauert länger");
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(app.fetch.stem_check.is_none(), egui::Button::new("Installation prüfen")).clicked() {
                let py = tstudio_fetch::stems::find_python(s.python_path.as_deref());
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let r = match py {
                        Some(p) => tstudio_fetch::stems::check(&p),
                        None => Err("Python wurde nicht gefunden".into()),
                    };
                    let _ = tx.send(r);
                });
                app.fetch.stem_check = Some(rx);
            }
            if app.fetch.stem_check.is_some() {
                ui.spinner();
            }
            match &app.fetch.stem_status {
                Some(Ok(v)) => {
                    ui.colored_label(egui::Color32::from_rgb(120, 230, 140), format!("✔ Demucs / Torch {v}"));
                }
                Some(Err(e)) => {
                    ui.colored_label(egui::Color32::from_rgb(240, 110, 90), format!("✖ {e}"));
                }
                None => {}
            }
            if ui.small_button("📋 Installationsbefehle").clicked() {
                ui.ctx().copy_text(tstudio_fetch::stems::INSTALL_HINT.to_string());
            }
        });
        ui.label(RichText::new(tstudio_fetch::stems::INSTALL_HINT).monospace().small());
        let t = &app.fetch.tools;
        ui.label(format!(
            "Gefunden: ffmpeg {} · yt-dlp {}",
            if t.ffmpeg.is_some() { "✔" } else { "✖" },
            if t.ytdlp.is_some() { "✔" } else { "✖" }
        ));
        if ui.button("Tools neu suchen").clicked() {
            redetect = true;
        }
        ui.separator();
        ui.weak(format!("Konfiguration: {}", app.paths.config_dir.display()));
    });

    app.show_settings = open;
    if open_learn {
        app.midi_learn.open = true;
    }
    if let Some(m) = midi_change {
        match &m {
            Some(name) => app.midi.connect(name, ctx.clone()),
            None => app.midi.disconnect(),
        }
        app.settings.midi_port = app.midi.port_name.clone();
    }
    if redetect {
        app.fetch.redetect(&app.settings);
    }
    if rescan {
        app.library.rescan(&app.settings);
    }
    if app.settings != before {
        app.settings.save(&app.paths.settings());
    }
    if restart {
        app.restart_engine();
    }
}
