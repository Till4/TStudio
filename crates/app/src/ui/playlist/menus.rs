//! Context menus of the playlist: track header menu, new pattern, empty-spot menu.

use super::*;

/// Context menu of a playlist track header (right click).
pub(super) fn track_menu(app: &mut TStudio, ui: &mut Ui) {
    let n = app.project.playlist.tracks.len();
    let Some(t) = app.playlist_ui.menu_track.filter(|t| *t < n) else {
        if ui.button("+ Spur hinzufügen").clicked() {
            app.project.add_playlist_track(None);
            ui.close_menu();
        }
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Name");
        let r = ui.text_edit_singleline(&mut app.project.playlist.tracks[t].name);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ui.close_menu();
        }
    });
    ui.horizontal(|ui| {
        ui.label("Lautstärke");
        let tr = &mut app.project.playlist.tracks[t];
        let mut pct = tr.volume * 100.0;
        if ui.add(egui::Slider::new(&mut pct, 0.0..=125.0).suffix(" %").fixed_decimals(0)).changed() {
            tr.volume = pct / 100.0;
        }
    });
    ui.horizontal(|ui| {
        let tr = &mut app.project.playlist.tracks[t];
        ui.toggle_value(&mut tr.muted, "Mute");
        let mut solo = tr.solo;
        if ui.toggle_value(&mut solo, "Solo").clicked() {
            app.project.toggle_playlist_solo(t, true);
        }
    });
    if app.project.playlist.tracks.iter().any(|x| x.solo || x.muted) && ui.button("Alle Mutes/Solos aufheben").clicked() {
        for x in app.project.playlist.tracks.iter_mut() {
            x.muted = false;
            x.solo = false;
        }
        ui.close_menu();
    }
    ui.separator();
    if ui.button("Spur darüber einfügen").clicked() {
        app.project.insert_playlist_track(t, None);
        ui.close_menu();
    }
    if ui.button("Spur darunter einfügen").clicked() {
        app.project.insert_playlist_track(t + 1, None);
        ui.close_menu();
    }
    if ui.button("+ Spur am Ende").clicked() {
        app.project.add_playlist_track(None);
        ui.close_menu();
    }
    if ui.button("Spur duplizieren (mit Clips)").clicked() {
        let _ = app.project.duplicate_playlist_track(t);
        ui.close_menu();
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(t > 0, egui::Button::new("▲ nach oben")).clicked() {
            let _ = app.project.move_playlist_track(t, t - 1);
            app.playlist_ui.menu_track = Some(t - 1);
        }
        if ui.add_enabled(t + 1 < n, egui::Button::new("▼ nach unten")).clicked() {
            let _ = app.project.move_playlist_track(t, t + 1);
            app.playlist_ui.menu_track = Some(t + 1);
        }
    });
    ui.separator();
    let n_clips = app.project.playlist.clips.iter().filter(|c| c.track == t).count();
    if ui.add_enabled(n_clips > 0, egui::Button::new(format!("Clips der Spur löschen ({n_clips})"))).clicked() {
        app.project.playlist.clips.retain(|c| c.track != t);
        app.playlist_ui.selected.clear();
        ui.close_menu();
    }
    let label = if n_clips > 0 { format!("🗑 Spur löschen (inkl. {n_clips} Clips)") } else { "🗑 Spur löschen".to_string() };
    if ui.add_enabled(n > 1, egui::Button::new(RichText::new(label).color(Color32::from_rgb(240, 120, 110)))).clicked() {
        let _ = app.project.delete_playlist_track(t);
        app.playlist_ui.selected.clear();
        app.playlist_ui.menu_track = None;
        ui.close_menu();
    }
}

/// Creates an empty 1-bar pattern for channel `ch`, places it at (track, at) and opens it in the piano roll.
pub(super) fn new_pattern_here(app: &mut TStudio, ch: usize, track: usize, at: i64) {
    let Some(channel) = app.project.channels.get(ch) else { return };
    let (name, color) = (channel.name.clone(), channel.color);
    let taken = |n: &str, p: &Project| p.patterns.iter().any(|x| x.name == n);
    let name = if !taken(&name, &app.project) {
        name
    } else {
        (2..).map(|i| format!("{name} {i}")).find(|n| !taken(n, &app.project)).unwrap_or(name)
    };
    let pid = app.project.add_pattern();
    if let Some(p) = app.project.pattern_mut(pid) {
        p.name = name.clone();
        p.color = color;
    }
    let len = app.project.pattern_length(pid);
    let id = app.project.add_clip(track, at, len, ClipKind::Pattern { pattern: pid });
    app.playlist_ui.selected = [id].into_iter().collect();
    app.current_pattern = pid;
    app.selected_channel = ch;
    app.song_mode = false;
    app.view = View::PianoRoll;
    app.set_status(format!("Neues Pattern „{name}“ – Noten zeichnen, dann in der Playlist am rechten Rand länger ziehen zum Wiederholen"));
}

pub(super) fn empty_spot_menu(app: &mut TStudio, ui: &mut Ui, track: usize, at: i64) {
    let bar = app.project.ticks_per_bar();
    ui.label(RichText::new(format!("Spur {} · Takt {}", track + 1, at / bar + 1)).small().color(TEXT_DIM));
    ui.label(RichText::new("Neues Pattern mit …").strong());
    let mut pick: Option<usize> = None;
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        for (i, ch) in app.project.channels.iter().enumerate() {
            let kind = match &ch.instrument {
                tstudio_project::Instrument::Synth(_) => "TSynth",
                tstudio_project::Instrument::Sampler(_) => "Sampler",
                tstudio_project::Instrument::SoundFont(_) => "SoundFont",
            };
            let label = RichText::new(format!("■ {}", ch.name)).color(rgb(ch.color));
            if ui.button(label).on_hover_text(kind).clicked() {
                pick = Some(i);
            }
        }
    });
    ui.menu_button("+ Neues Instrument", |ui| {
        ui.label(RichText::new("TSynth-Preset").small().color(TEXT_DIM));
        for (name, params) in tstudio_dsp::synth::SynthParams::presets() {
            if ui.button(name).clicked() {
                pick = Some(app.project.add_channel(name, tstudio_project::Instrument::Synth(params)));
            }
        }
        ui.separator();
        if ui.button("Sampler aus Datei…").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("Audio", tstudio_library::AUDIO_EXTS).pick_file() {
                pick = Some(app.add_sampler_channel(&p));
            }
        }
        if ui.button("SoundFont (.sf2)…").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("SoundFont", &["sf2", "SF2"]).pick_file() {
                pick = app.add_soundfont_channel(&p);
            }
        }
    });
    if let Some(ch) = pick {
        new_pattern_here(app, ch, track, at);
        app.playlist_ui.empty_menu = None;
        ui.close_menu();
        return;
    }
    ui.separator();
    let cur = app.current_pattern;
    if let Some(name) = app.project.pattern(cur).map(|p| p.name.clone()) {
        if ui.button(format!("„{name}“ hier einfügen")).clicked() {
            let len = app.project.pattern_length(cur);
            let id = app.project.add_clip(track, at, len, ClipKind::Pattern { pattern: cur });
            app.playlist_ui.selected = [id].into_iter().collect();
            app.song_mode = true;
            app.playlist_ui.empty_menu = None;
            ui.close_menu();
        }
    }
}
