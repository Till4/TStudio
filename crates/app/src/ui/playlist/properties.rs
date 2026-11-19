//! Clip property panels (pattern clips, audio clips) and loop toggle.

use super::*;

/// Loop region on/off: around the selected clips, otherwise 4 bars from the playhead's bar.
pub(super) fn toggle_loop(app: &mut TStudio) {
    if app.project.playlist.loop_region.take().is_some() {
        app.set_status("Loop aus");
        return;
    }
    let bar = app.project.ticks_per_bar();
    let sel: Vec<&tstudio_project::Clip> = app.project.playlist.clips.iter().filter(|c| app.playlist_ui.selected.contains(&c.id)).collect();
    let (a, b) = if sel.is_empty() {
        let a = floor_to(app.position() as i64, bar).max(0);
        (a, a + 4 * bar)
    } else {
        let a = sel.iter().map(|c| c.start).min().unwrap_or(0);
        let b = sel.iter().map(|c| c.end()).max().unwrap_or(a + bar);
        (floor_to(a, bar), (b + bar - 1) / bar * bar)
    };
    app.project.playlist.loop_region = Some((a, b.max(a + bar)));
    app.song_mode = true;
    app.set_status(format!("Loop Takt {} – {}", a / bar + 1, b.max(a + bar) / bar));
}

/// Bottom panel for a selected pattern clip: open, clone (own copy), repeat.
pub(super) fn pattern_clip_properties(app: &mut TStudio, ui: &mut Ui, id: Id) {
    let Some(idx) = app.project.playlist.clips.iter().position(|c| c.id == id) else { return };
    let ClipKind::Pattern { pattern } = app.project.playlist.clips[idx].kind else { return };
    let uses = app.project.playlist.clips.iter().filter(|c| matches!(c.kind, ClipKind::Pattern { pattern: p } if p == pattern)).count();
    let plen = app.project.pattern_length(pattern).max(1);
    let mut action: Option<&str> = None;
    let mut repeat: Option<i64> = None;
    ui.horizontal_wrapped(|ui| {
        let Some(p) = app.project.pattern_mut(pattern) else { return };
        ui.label(RichText::new("▦").color(rgb(p.color)));
        ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(140.0)).on_hover_text("Pattern umbenennen");
        ui.weak(format!("{uses}× in der Playlist"));
        ui.separator();
        if ui.button("🎹 Öffnen").on_hover_text("In der Piano Roll bearbeiten (oder Doppelklick auf den Clip)").clicked() {
            action = Some("open");
        }
        if ui
            .button("⧉ Eigene Kopie")
            .on_hover_text("Macht aus diesem Clip ein eigenes Pattern (Klon). Änderungen wirken dann nur hier, nicht auf die anderen Clips.")
            .clicked()
        {
            action = Some("clone");
        }
        ui.separator();
        ui.label(RichText::new("↻ Wiederholen").strong());
        for n in [2, 4, 8] {
            if ui.button(format!("×{n}")).on_hover_text(format!("Clip {n}× so lang – Pattern läuft {n}× durch")).clicked() {
                repeat = Some(n);
            }
        }
        if ui.button("1×").on_hover_text("Clip-Länge = Pattern-Länge").clicked() {
            repeat = Some(0);
        }
        ui.weak("Strg+C / Strg+V kopiert Clips · Strg+D dupliziert");
    });
    if let Some(n) = repeat {
        let c = &mut app.project.playlist.clips[idx];
        c.len = if n == 0 { plen } else { c.len.max(plen) * n };
    }
    match action {
        Some("open") => {
            app.current_pattern = pattern;
            app.view = View::PianoRoll;
            app.song_mode = false;
        }
        Some("clone") => {
            let Some(src) = app.project.pattern(pattern).cloned() else { return };
            let taken = |n: &str, p: &Project| p.patterns.iter().any(|x| x.name == n);
            let base = src.name.clone();
            let name = (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n, &app.project)).unwrap_or(base);
            let new_id = app.project.add_pattern();
            if let Some(p) = app.project.pattern_mut(new_id) {
                let keep = p.id;
                *p = src;
                p.id = keep;
                p.name = name.clone();
            }
            app.project.playlist.clips[idx].kind = ClipKind::Pattern { pattern: new_id };
            app.current_pattern = new_id;
            app.set_status(format!("Eigene Kopie „{name}“ angelegt – Änderungen wirken nur auf diesen Clip"));
        }
        _ => {}
    }
}

pub(super) fn clip_properties(app: &mut TStudio, ui: &mut Ui, id: Id) {
    let bpm = app.project.bpm;
    let detected = {
        let Some(c) = app.project.playlist.clips.iter().find(|c| c.id == id) else { return };
        let ClipKind::Audio(a) = &c.kind else { return };
        app.library.analysis.get(&a.path).and_then(|x| x.bpm)
    };
    let max_track = app.project.mixer.len() - 1;
    let Some(idx) = app.project.playlist.clips.iter().position(|c| c.id == id) else { return };
    let before = app.project.playlist.clips[idx].clone();
    let mut open_trim = None;
    let mut fit = false;
    let mut open_mixer = false;
    let insert_names: Vec<String> = app.project.mixer.iter().map(|m| m.name.clone()).collect();
    {
        let clip = &mut app.project.playlist.clips[idx];
        let ClipKind::Audio(a) = &mut clip.kind else { return };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("🔊 {}", a.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())).strong());
            ui.separator();
            ui.label("Tempo:");
            egui::ComboBox::from_id_salt("clip_stretch").selected_text(a.stretch.mode.name()).show_ui(ui, |ui| {
                for m in StretchMode::ALL {
                    ui.selectable_value(&mut a.stretch.mode, m, m.name());
                }
            });
            if a.stretch.mode != StretchMode::Off {
                ui.label("Sample-BPM");
                ui.add(egui::DragValue::new(&mut a.stretch.source_bpm).speed(0.1).range(20.0..=300.0).fixed_decimals(2));
                if ui.small_button("×2").clicked() {
                    a.stretch.source_bpm *= 2.0;
                }
                if ui.small_button("÷2").clicked() {
                    a.stretch.source_bpm /= 2.0;
                }
                if let Some(d) = detected {
                    if ui.small_button(format!("erkannt: {d:.1}")).clicked() {
                        a.stretch.source_bpm = d;
                    }
                }
                let mut follow = a.stretch.target_bpm.is_none();
                if ui.checkbox(&mut follow, format!("→ Projekt ({bpm:.1})")).changed() {
                    a.stretch.target_bpm = if follow { None } else { Some(bpm) };
                }
                if let Some(t) = a.stretch.target_bpm.as_mut() {
                    ui.add(egui::DragValue::new(t).speed(0.1).range(20.0..=300.0).prefix("Ziel ").fixed_decimals(2));
                }
            }
            ui.add(egui::DragValue::new(&mut a.stretch.pitch).speed(0.05).range(-24.0..=24.0).prefix("Pitch ").suffix(" st"))
                .on_hover_text("Tonhöhe in Halbtönen – in jedem Tempo-Modus; die Länge bleibt gleich (bei Resample zusätzlich zur Platten-Tonhöhe)");
            if a.stretch.pitch != 0.0 && ui.small_button("0").on_hover_text("Pitch zurücksetzen").clicked() {
                a.stretch.pitch = 0.0;
            }
            if a.stretch.pitch != 0.0 {
                ui.checkbox(&mut a.stretch.formant, "Formant halten")
                    .on_hover_text("Stimmen behalten ihren Charakter: kein Micky-Maus-Effekt beim Hochpitchen, kein Monster beim Runterpitchen");
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut db = tstudio_dsp::gain_to_db(a.gain).max(-60.0);
            ui.label("Gain");
            if ui.add(egui::DragValue::new(&mut db).speed(0.1).range(-60.0..=12.0).suffix(" dB")).changed() {
                a.gain = tstudio_dsp::db_to_gain(db);
            }
            ui.label("Mixer");
            ui.add(egui::DragValue::new(&mut a.mixer_track).range(0..=max_track).prefix("→"));
            ui.label(RichText::new(insert_names.get(a.mixer_track).cloned().unwrap_or_default()).color(TEXT_DIM));
            let tip = if a.mixer_track == 0 { "Sample bekommt einen eigenen Mixer-Insert und der Mixer öffnet sich – dort Effekte in die Slots laden" } else { "Mixer mit diesem Insert öffnen – dort Effekte in die Slots laden" };
            if ui.button("🎚 Effekte im Mixer").on_hover_text(tip).clicked() {
                open_mixer = true;
            }
            ui.label("Fade In");
            let mut fi = a.fade_in as f32 / PPQ as f32;
            if ui.add(egui::DragValue::new(&mut fi).speed(0.05).range(0.0..=64.0).suffix(" Beats")).changed() {
                a.fade_in = (fi * PPQ as f32) as i64;
            }
            ui.label("Fade Out");
            let mut fo = a.fade_out as f32 / PPQ as f32;
            if ui.add(egui::DragValue::new(&mut fo).speed(0.05).range(0.0..=64.0).suffix(" Beats")).changed() {
                a.fade_out = (fo * PPQ as f32) as i64;
            }
            if ui.button("Länge an Sample anpassen").clicked() {
                fit = true;
            }
            if ui.button("✂ Sample-Cutter").clicked() {
                open_trim = Some(a.path.clone());
            }
            ui.checkbox(&mut clip.muted, "Stumm");
        });
    }
    // repeat + automation right on the sample
    let mut auto_kind: Option<&str> = None;
    let mut repeat_times: Option<i64> = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("↻ Wiederholen").strong());
        for n in [2, 4, 8] {
            if ui.button(format!("×{n}")).on_hover_text(format!("Clip {n}× so lang – das Sample läuft in Schleife (oder rechten Rand ziehen)")).clicked() {
                repeat_times = Some(n);
            }
        }
        ui.separator();
        ui.label(RichText::new("〰 Automation").strong().color(AUTO_COLOR));
        for (k, label, tip) in [
            ("volume", "Lautstärke", "Lautstärke-Kurve über die ganze Clip-Länge"),
            ("filter", "Filter-Sweep", "Tiefpass-Filter auf die Spur und einen Sweep zu, der sich öffnet"),
            ("pan", "Pan", "Panorama-Kurve"),
        ] {
            if ui.button(label).on_hover_text(format!("{tip}. Das Sample bekommt dafür einen eigenen Mixer-Insert.")).clicked() {
                auto_kind = Some(k);
            }
        }
    });
    if let Some(n) = repeat_times {
        let c = &mut app.project.playlist.clips[idx];
        c.len *= n;
    }
    if let Some(kind) = auto_kind {
        automate_audio_clip(app, id, kind);
        return;
    }
    if open_mixer {
        let mut mt = match &app.project.playlist.clips[idx].kind {
            ClipKind::Audio(a) => a.mixer_track,
            _ => 0,
        };
        if mt == 0 || mt >= app.project.mixer.len() {
            let path = match &app.project.playlist.clips[idx].kind {
                ClipKind::Audio(a) => a.path.clone(),
                _ => return,
            };
            mt = app.project.sample_insert(&path);
            // all clips of this file that still play on the master move along
            for c in app.project.playlist.clips.iter_mut() {
                if let ClipKind::Audio(a) = &mut c.kind {
                    if a.path == path && a.mixer_track == 0 {
                        a.mixer_track = mt;
                    }
                }
            }
        }
        app.mixer_ui.selected = mt;
        app.view = View::Mixer;
        return;
    }
    // re-fit length when the tempo mapping changed
    let after = app.project.playlist.clips[idx].clone();
    let stretch_changed = match (&before.kind, &after.kind) {
        (ClipKind::Audio(a), ClipKind::Audio(b)) => a.stretch.mode != b.stretch.mode || a.stretch.source_bpm != b.stretch.source_bpm,
        _ => false,
    };
    if fit || stretch_changed {
        if let ClipKind::Audio(a) = &after.kind {
            if let Some(dur) = clip_duration_secs(app, &a.path) {
                let len = match a.stretch.mode {
                    StretchMode::Off => app.project.secs_to_ticks(dur),
                    _ => dur * a.stretch.source_bpm / 60.0 * PPQ as f64,
                };
                app.project.playlist.clips[idx].len = (len.round() as i64 - a.offset).max(1);
            }
        }
    }
    if let Some(p) = open_trim {
        app.fetch.open_in_trim(&p);
        app.view = View::Fetch;
    }
}
