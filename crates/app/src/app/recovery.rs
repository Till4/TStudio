//! Auto-backup in the app: every [`backup::INTERVAL`] unsaved changes are written in the
//! background to the backups folder (last [`backup::KEEP`] kept); after a crash the next start
//! offers to restore them ("Backups" window, also under Datei > Backups…).

use super::*;
use crate::backup::{self, Backup};

/// State of the backup feature.
#[derive(Default)]
pub struct BackupState {
    /// project as it was last backed up (no new backup while nothing changed)
    last: Option<Project>,
    pub show_window: bool,
    /// the previous session ended without closing TStudio
    pub crashed: bool,
}

impl TStudio {
    /// Called once at start: marks the session and opens the restore offer after a crash.
    pub(super) fn init_backups(&mut self) {
        let crashed = backup::start_session(&self.paths.session_lock());
        if crashed && !backup::list(&self.paths.backups_dir()).is_empty() {
            self.backups.crashed = true;
            self.backups.show_window = true;
        }
    }

    /// Periodic background backup of unsaved changes (plus the library cache).
    pub(super) fn auto_backup(&mut self) {
        let elapsed = self.last_autosave.elapsed();
        if elapsed < backup::INTERVAL {
            return;
        }
        let changed = self.backups.last.as_ref() != Some(&self.project);
        if backup::due(self.is_dirty(), changed, elapsed, backup::INTERVAL) {
            if let Ok(ron) = self.project.to_ron() {
                let dir = self.paths.backups_dir();
                let name = self.project.name.clone();
                let path = self.project_path.clone();
                std::thread::spawn(move || {
                    let _ = backup::write(&dir, &name, &ron, path.as_deref(), backup::now_secs(), backup::KEEP);
                });
                self.backups.last = Some(self.project.clone());
            }
        }
        self.last_autosave = Instant::now();
        self.library.save_cache();
    }

    /// Clean exit: no restore offer next time.
    pub(super) fn end_backup_session(&mut self) {
        backup::end_session(&self.paths.session_lock());
    }

    /// Loads a backup as unsaved changes of its original project file (if known).
    pub fn restore_backup(&mut self, b: &Backup) {
        if !self.confirm_discard() {
            return;
        }
        match Project::load(&b.path) {
            Ok(p) => {
                let target = b.project_path.clone().filter(|p| p.exists());
                self.load_project(p, target.clone());
                // keep it dirty: the restored state is not on disk yet
                self.saved = Project::default();
                self.backups.last = Some(self.project.clone());
                let at = backup::pretty_stamp(&b.stamp);
                match target {
                    Some(t) => self.set_status(format!("Backup vom {at} (UTC) wiederhergestellt – Strg+S speichert nach {}", t.display())),
                    None => self.set_status(format!("Backup vom {at} (UTC) wiederhergestellt – noch nicht gespeichert")),
                }
                self.backups.show_window = false;
                self.backups.crashed = false;
            }
            Err(e) => self.set_error(format!("Backup nicht lesbar: {e}")),
        }
    }

    /// "Backups" window: list (newest first) with restore buttons.
    pub(super) fn backup_window(&mut self, ctx: &egui::Context) {
        if !self.backups.show_window {
            return;
        }
        let list = backup::list(&self.paths.backups_dir());
        let legacy = self.paths.autosave();
        let mut open = true;
        let mut restore: Option<Backup> = None;
        let mut restore_legacy = false;
        let mut close = false;
        egui::Window::new("Backups").open(&mut open).collapsible(false).default_width(460.0).show(ctx, |ui| {
            if self.backups.crashed {
                ui.colored_label(Color32::from_rgb(240, 180, 90), "TStudio wurde beim letzten Mal nicht normal beendet. Ungespeicherte Arbeit wiederherstellen?");
                ui.add_space(4.0);
            }
            ui.weak(format!("Alle {} Minuten werden ungespeicherte Änderungen gesichert (die letzten {} bleiben).", backup::INTERVAL.as_secs() / 60, backup::KEEP));
            if list.is_empty() {
                ui.weak("(noch keine Backups)");
            }
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for b in &list {
                    ui.horizontal(|ui| {
                        if ui.button("Wiederherstellen").clicked() {
                            restore = Some(b.clone());
                        }
                        ui.label(RichText::new(&b.name).strong());
                        ui.weak(format!("{} UTC", backup::pretty_stamp(&b.stamp)));
                        if let Some(p) = &b.project_path {
                            ui.label(RichText::new(p.display().to_string()).small().color(TEXT_DIM));
                        }
                    });
                }
            });
            if legacy.exists() && ui.button("Altes Autosave laden").clicked() {
                restore_legacy = true;
            }
            ui.horizontal(|ui| {
                if ui.button("Schließen").clicked() {
                    close = true;
                }
                if ui.button("Ordner öffnen").clicked() {
                    let dir = self.paths.backups_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            });
        });
        if let Some(b) = restore {
            self.restore_backup(&b);
        }
        if restore_legacy && self.confirm_discard() {
            match Project::load(&legacy) {
                Ok(p) => {
                    self.load_project(p, None);
                    self.set_status("Autosave wiederhergestellt");
                    self.backups.show_window = false;
                }
                Err(e) => self.set_error(e),
            }
        }
        if !open || close {
            self.backups.show_window = false;
            self.backups.crashed = false;
        }
    }
}
