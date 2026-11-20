//! Project templates in the app: "Neu aus Vorlage" (built-in + user templates) and
//! "Als Vorlage speichern…" (name dialog, stored in the templates folder of the config dir).

use super::*;
use tstudio_project::templates;

impl TStudio {
    /// Replaces the project with a fresh copy of `template` (asks to save changes first).
    pub fn new_from_template(&mut self, template: Project, label: &str) {
        if self.confirm_discard() {
            self.load_project(templates::instantiate(template), None);
            self.set_status(format!("Neues Projekt aus Vorlage „{label}“"));
        }
    }

    /// Entries of the "Neu aus Vorlage" sub menu.
    pub(super) fn template_menu(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Mitgeliefert").small().color(TEXT_DIM));
        for name in templates::builtin_names() {
            if ui.button(name).clicked() {
                ui.close_menu();
                if let Some(p) = templates::builtin(name) {
                    self.new_from_template(p, name);
                }
            }
        }
        let user = templates::list_user_templates(&self.paths.templates_dir());
        ui.separator();
        ui.label(RichText::new("Eigene").small().color(TEXT_DIM));
        if user.is_empty() {
            ui.weak("(noch keine – Datei > Als Vorlage speichern…)");
        }
        for (name, path) in user {
            if ui.button(&name).on_hover_text(path.display().to_string()).clicked() {
                ui.close_menu();
                match Project::load(&path) {
                    Ok(p) => self.new_from_template(p, &name),
                    Err(e) => self.set_error(format!("Vorlage nicht lesbar: {e}")),
                }
            }
        }
        ui.separator();
        if ui.button("Vorlagen-Ordner öffnen").clicked() {
            ui.close_menu();
            let dir = self.paths.templates_dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
    }

    /// Opens the name dialog for "Als Vorlage speichern…".
    pub fn ask_template_name(&mut self) {
        self.template_dialog = Some(self.project.name.clone());
    }

    /// Name dialog; Enter / "Speichern" stores the current project as user template.
    pub(super) fn template_window(&mut self, ctx: &egui::Context) {
        let Some(mut name) = self.template_dialog.take() else { return };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Als Vorlage speichern").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Name der Vorlage:");
            let r = ui.text_edit_singleline(&mut name);
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                save = true;
            }
            r.request_focus();
            let exists = templates::list_user_templates(&self.paths.templates_dir()).iter().any(|(n, _)| n.eq_ignore_ascii_case(name.trim()));
            if exists {
                ui.colored_label(Color32::from_rgb(240, 180, 90), "Eine Vorlage mit diesem Namen wird überschrieben");
            }
            ui.horizontal(|ui| {
                if ui.button("Speichern").clicked() {
                    save = true;
                }
                if ui.button("Abbrechen").clicked() {
                    cancel = true;
                }
            });
        });
        if save {
            match templates::save_user_template(&self.paths.templates_dir(), &name, &self.project) {
                Ok(p) => self.set_status(format!("Vorlage gespeichert: {}", p.display())),
                Err(e) => self.set_error(format!("Vorlage speichern fehlgeschlagen: {e}")),
            }
        } else if open && !cancel {
            self.template_dialog = Some(name);
        }
    }
}
