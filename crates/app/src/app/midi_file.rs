//! Datei > MIDI: import a .mid as new pattern, export the current pattern or the whole song.

use super::*;
use tstudio_project::midi;

impl TStudio {
    /// Entries of the "MIDI" sub menu.
    pub(super) fn midi_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("MIDI importieren…").on_hover_text("Neues Pattern, ein Kanal pro MIDI-Spur, Tempo aus der Datei").clicked() {
            ui.close_menu();
            self.import_midi_dialog();
        }
        if ui.button("Pattern als MIDI exportieren…").clicked() {
            ui.close_menu();
            self.export_midi_dialog(false);
        }
        if ui.button("Song als MIDI exportieren…").clicked() {
            ui.close_menu();
            self.export_midi_dialog(true);
        }
    }

    fn midi_dir(&self) -> Option<PathBuf> {
        self.project_path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf())
    }

    pub fn import_midi_dialog(&mut self) {
        let mut d = rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi"]);
        if let Some(dir) = self.midi_dir() {
            d = d.set_directory(dir);
        }
        let Some(path) = d.pick_file() else { return };
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "MIDI".into());
        let res = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| midi::import(&mut self.project, &bytes, &name, true));
        match res {
            Ok(r) => {
                self.current_pattern = r.pattern;
                if let Some(c) = r.channels.first() {
                    self.selected_channel = *c;
                }
                let tempo = r.bpm.map(|b| format!(", Tempo {b:.2} BPM")).unwrap_or_default();
                self.set_status(format!("MIDI importiert: {} Noten, {} Kanäle{tempo}", r.notes, r.channels.len()));
            }
            Err(e) => self.set_error(format!("MIDI-Import fehlgeschlagen: {e}")),
        }
    }

    pub fn export_midi_dialog(&mut self, song: bool) {
        let label = if song { self.project.name.clone() } else { self.project.pattern(self.current_pattern).map(|p| p.name.clone()).unwrap_or_else(|| "Pattern".into()) };
        let mut d = rfd::FileDialog::new().add_filter("MIDI", &["mid"]).set_file_name(format!("{label}.mid"));
        if let Some(dir) = self.midi_dir() {
            d = d.set_directory(dir);
        }
        let Some(mut path) = d.save_file() else { return };
        if path.extension().is_none() {
            path.set_extension("mid");
        }
        let bytes = if song { midi::export_song(&self.project) } else { midi::export_pattern(&self.project, self.current_pattern) };
        match bytes.and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
            Ok(()) => self.set_status(format!("MIDI exportiert: {}", path.display())),
            Err(e) => self.set_error(format!("MIDI-Export fehlgeschlagen: {e}")),
        }
    }
}
