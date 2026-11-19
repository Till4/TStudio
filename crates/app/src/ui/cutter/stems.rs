//! Stem separation bar and job handling of the cutter view.

use super::*;

/// Free folder <output>/Stems/<name> (adds " 2", " 3" … if it exists).
pub(super) fn stem_dir(app: &TStudio, name: &str) -> PathBuf {
    let base = app.settings.output_dir().join("Stems");
    let first = base.join(name);
    if !first.exists() {
        return first;
    }
    (2..).map(|i| base.join(format!("{name} {i}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Starts Demucs on the loaded file (CPU only, low priority) or explains how to install it.
pub(super) fn start_stem_job(app: &mut TStudio) {
    use tstudio_fetch::stems;
    let Some(data) = app.fetch.trim.data.clone() else { return };
    let Some(python) = stems::find_python(app.settings.python_path.as_deref()) else {
        app.fetch.stem_result = Some(Err("Python wurde nicht gefunden".into()));
        return;
    };
    let name = app.fetch.trim.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Song".into());
    let dest = stem_dir(app, &name);
    let threads = stems::thread_count(app.settings.stem_threads);
    match stems::start_stems(&python, &data, &dest, &name, threads) {
        Ok(h) => {
            app.fetch.stems = Some(h);
            app.fetch.stem_progress = 0.0;
            app.fetch.stem_log = format!("startet auf {threads} CPU-Kern(en)…");
            app.fetch.stem_started = Some(std::time::Instant::now());
            app.fetch.stem_result = None;
        }
        Err(e) => app.fetch.stem_result = Some(Err(e)),
    }
}

/// "🎚 Stems trennen" row: progress while Demucs runs, buttons for the finished stems.
pub(super) fn stems_bar(app: &mut TStudio, ui: &mut Ui) {
    use tstudio_fetch::stems;
    if app.fetch.trim.data.is_none() && app.fetch.stems.is_none() && app.fetch.stem_result.is_none() {
        return;
    }
    let mut open: Option<PathBuf> = None;
    let mut start = false;
    ui.horizontal_wrapped(|ui| {
        if let Some(h) = &app.fetch.stems {
            ui.spinner();
            ui.add(egui::ProgressBar::new(app.fetch.stem_progress).desired_width(220.0).show_percentage());
            let secs = app.fetch.stem_started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            ui.weak(format!("Stems werden getrennt (nur CPU) · {}:{:02}", secs / 60, secs % 60));
            if ui.small_button("Abbrechen").clicked() {
                h.cancel();
            }
            let log: String = app.fetch.stem_log.chars().take(90).collect();
            if !log.is_empty() {
                ui.label(RichText::new(log).small().color(TEXT_DIM));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        if ui
            .add_enabled(app.fetch.trim.data.is_some(), egui::Button::new("🎚 Stems trennen"))
            .on_hover_text("KI-Trennung (Demucs) in Drums, Bass, Vocals und Rest – läuft nur auf der CPU mit niedriger Priorität.\nDauert je nach Song 1–4 Minuten; TStudio bleibt bedienbar.")
            .clicked()
        {
            start = true;
        }
        match &app.fetch.stem_result {
            Some(Ok(dir)) => {
                ui.label(RichText::new("Stems:").strong());
                let files: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
                for (_, label) in stems::STEMS {
                    let suffix = format!(" - {label}");
                    if let Some(f) = files.iter().find(|p| p.file_stem().map(|s| s.to_string_lossy().ends_with(&suffix)).unwrap_or(false)) {
                        if ui.button(label).on_hover_text(format!("{} im Sample-Cutter öffnen", f.display())).clicked() {
                            open = Some(f.clone());
                        }
                    }
                }
                if ui.small_button("📂").on_hover_text(dir.display().to_string()).clicked() {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
                ui.weak("Tipp: Drums öffnen → Preset „Kick/Sub“ → Taste 1 speichert den Kick");
            }
            Some(Err(e)) => {
                ui.colored_label(Color32::from_rgb(240, 110, 90), format!("⚠ {e}"));
            }
            None => {}
        }
    });
    if let Some(Err(_)) = &app.fetch.stem_result {
        ui.horizontal_wrapped(|ui| {
            ui.weak("Einmalig installieren (Eingabeaufforderung, nacheinander):");
            if ui.small_button("📋 Befehle kopieren").clicked() {
                ui.ctx().copy_text(stems::INSTALL_HINT.to_string());
            }
        });
        ui.label(RichText::new(stems::INSTALL_HINT).monospace().small());
    }
    if start {
        start_stem_job(app);
    }
    if let Some(f) = open {
        app.fetch.open_in_trim(&f);
    }
}
