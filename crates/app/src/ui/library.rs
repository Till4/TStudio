use crate::app::{TStudio, View};
use crate::settings::{Paths, Settings};
use crate::ui::widgets::{SampleDrag, ACCENT, BG_LIGHT, TEXT_DIM};
use crossbeam_channel::Receiver;
use egui::{pos2, vec2, Align2, Color32, FontId, RichText, Sense, Ui};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tstudio_library::{AnalysisCache, Analyzer, LibraryEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibFilter {
    All,
    Loops,
    OneShots,
    Favorites,
}

#[derive(Default)]
struct Folder {
    children: BTreeMap<String, Folder>,
    files: Vec<usize>,
}

enum LibAction {
    Select(PathBuf),
    Preview(PathBuf),
    AddToRack(PathBuf),
    AddToPlaylist(PathBuf),
    OpenTrim(PathBuf),
    ToggleFav(PathBuf),
    Reveal(PathBuf),
    RemoveRoot(usize),
}

pub struct LibraryState {
    pub entries: Vec<LibraryEntry>,
    roots_tree: Vec<Folder>,
    pub analysis: AnalysisCache,
    cache_path: PathBuf,
    analyzer: Analyzer,
    requested: HashSet<PathBuf>,
    pub search: String,
    pub filter: LibFilter,
    pub selected: Option<PathBuf>,
    scan_rx: Option<Receiver<Vec<LibraryEntry>>>,
    roots_scanned: Vec<PathBuf>,
    last_cache_save: Instant,
}

impl LibraryState {
    pub fn new(paths: &Paths, settings: &Settings) -> Self {
        let cache_path = paths.analysis_cache();
        let mut s = Self {
            entries: Vec::new(),
            roots_tree: Vec::new(),
            analysis: AnalysisCache::load(&cache_path),
            cache_path,
            analyzer: Analyzer::new(2),
            requested: HashSet::new(),
            search: String::new(),
            filter: LibFilter::All,
            selected: None,
            scan_rx: None,
            roots_scanned: Vec::new(),
            last_cache_save: Instant::now(),
        };
        s.rescan(settings);
        s
    }

    pub fn rescan(&mut self, settings: &Settings) {
        let roots = settings.library_roots.clone();
        self.roots_scanned = roots.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(tstudio_library::scan(&roots));
        });
        self.scan_rx = Some(rx);
    }

    pub fn is_busy(&self) -> bool {
        self.scan_rx.is_some() || self.analyzer.pending() > 0
    }

    pub fn save_cache(&mut self) {
        self.analysis.save(&self.cache_path);
    }

    /// Returns true when something changed.
    pub fn poll(&mut self, settings: &Settings) -> bool {
        let mut changed = false;
        if settings.library_roots != self.roots_scanned {
            self.rescan(settings);
        }
        if let Some(rx) = &self.scan_rx {
            if let Ok(entries) = rx.try_recv() {
                self.entries = entries;
                self.build_tree(settings.library_roots.len());
                self.scan_rx = None;
                changed = true;
            }
        }
        while let Ok((path, res)) = self.analyzer.results.try_recv() {
            if let Ok(a) = res {
                self.analysis.insert(&path, a);
            }
            changed = true;
        }
        if self.last_cache_save.elapsed() > Duration::from_secs(20) {
            self.last_cache_save = Instant::now();
            self.save_cache();
        }
        changed
    }

    fn build_tree(&mut self, roots: usize) {
        self.roots_tree = (0..roots).map(|_| Folder::default()).collect();
        for (i, e) in self.entries.iter().enumerate() {
            let Some(root) = self.roots_tree.get_mut(e.root) else { continue };
            let mut node = root;
            if !e.rel_dir.is_empty() {
                for part in e.rel_dir.split('/') {
                    node = node.children.entry(part.to_string()).or_default();
                }
            }
            node.files.push(i);
        }
    }

    fn matches(&self, e: &LibraryEntry, favorites: &BTreeSet<PathBuf>) -> bool {
        let words: Vec<String> = self.search.to_lowercase().split_whitespace().map(String::from).collect();
        let hay = format!("{} {}", e.rel_dir, e.name).to_lowercase();
        if !words.iter().all(|w| hay.contains(w.as_str())) {
            return false;
        }
        match self.filter {
            LibFilter::All => true,
            LibFilter::Favorites => favorites.contains(&e.path),
            LibFilter::Loops => self.analysis.get(&e.path).map(|a| a.is_loop()).unwrap_or(false),
            LibFilter::OneShots => self.analysis.get(&e.path).map(|a| !a.is_loop()).unwrap_or(false),
        }
    }

    fn row(&mut self, ui: &mut Ui, idx: usize, indent: f32, favorites: &BTreeSet<PathBuf>, actions: &mut Vec<LibAction>) {
        let e = &self.entries[idx];
        let path = e.path.clone();
        let is_sf = e.ext == "sf2";
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
        if ui.is_rect_visible(rect) {
            let analysis = self.analysis.get(&path).cloned();
            if !is_sf && analysis.is_none() && !self.requested.contains(&path) {
                self.requested.insert(path.clone());
                self.analyzer.request(path.clone());
            }
            let p = ui.painter();
            let selected = self.selected.as_ref() == Some(&path);
            if selected {
                p.rect_filled(rect, 2.0, ACCENT.gamma_multiply(0.35));
            } else if resp.hovered() {
                p.rect_filled(rect, 2.0, BG_LIGHT);
            }
            let is_loop = analysis.as_ref().map(|a| a.is_loop()).unwrap_or(false);
            let icon = if is_sf { "🎹" } else if is_loop { "⟳" } else { "♪" };
            let fav = if favorites.contains(&path) { "★ " } else { "" };
            let x0 = rect.left() + indent;
            p.text(pos2(x0, rect.center().y), Align2::LEFT_CENTER, icon, FontId::proportional(11.0), if is_loop { ACCENT } else { TEXT_DIM });
            // badges (right aligned)
            let mut badge = String::new();
            if let Some(a) = &analysis {
                if let Some(b) = a.bpm {
                    badge.push_str(&format!("{b:.0} "));
                }
                if let Some(k) = &a.key {
                    badge.push_str(&format!("{k} "));
                }
                badge.push_str(&format!("{:.1}s", a.duration));
            }
            if is_sf {
                badge.push_str("SF2");
            }
            let badge_w = badge.len() as f32 * 5.6 + 6.0;
            p.text(pos2(rect.right() - 4.0, rect.center().y), Align2::RIGHT_CENTER, &badge, FontId::monospace(9.5), TEXT_DIM);
            let name_rect = egui::Rect::from_min_max(pos2(x0 + 14.0, rect.top()), pos2(rect.right() - badge_w, rect.bottom()));
            p.with_clip_rect(name_rect).text(
                pos2(name_rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                format!("{fav}{}", e.name),
                FontId::proportional(12.0),
                if selected { Color32::WHITE } else { Color32::from_gray(215) },
            );
        }
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), SampleDrag { path: path.clone() });
        }
        if resp.clicked() {
            actions.push(LibAction::Select(path.clone()));
            if !is_sf {
                actions.push(LibAction::Preview(path.clone()));
            }
        }
        if resp.double_clicked() {
            actions.push(LibAction::AddToRack(path.clone()));
        }
        if is_sf {
            let resp = resp.on_hover_text(format!("{}\nSoundFont – Doppelklick oder ins Channel Rack ziehen: neuer SoundFont-Kanal", path.display()));
            resp.context_menu(|ui| {
                if ui.button("➕ Als SoundFont-Kanal").clicked() {
                    actions.push(LibAction::AddToRack(path.clone()));
                    ui.close_menu();
                }
                if ui.button("📂 Im Explorer zeigen").clicked() {
                    actions.push(LibAction::Reveal(path.clone()));
                    ui.close_menu();
                }
            });
            return;
        }
        let resp = resp.on_hover_text(format!("{}\nKlick: vorhören · Doppelklick: in Channel Rack · Ziehen: in Playlist/Rack", path.display()));
        resp.context_menu(|ui| {
            if ui.button("▶ Vorhören").clicked() {
                actions.push(LibAction::Preview(path.clone()));
                ui.close_menu();
            }
            if ui.button("➕ Als Sampler-Kanal").clicked() {
                actions.push(LibAction::AddToRack(path.clone()));
                ui.close_menu();
            }
            if ui.button("➕ In Playlist (am Playhead)").clicked() {
                actions.push(LibAction::AddToPlaylist(path.clone()));
                ui.close_menu();
            }
            if ui.button("✂ Im Sample-Cutter öffnen").clicked() {
                actions.push(LibAction::OpenTrim(path.clone()));
                ui.close_menu();
            }
            let fav = favorites.contains(&path);
            if ui.button(if fav { "☆ Favorit entfernen" } else { "★ Als Favorit" }).clicked() {
                actions.push(LibAction::ToggleFav(path.clone()));
                ui.close_menu();
            }
            if ui.button("📂 Im Explorer zeigen").clicked() {
                actions.push(LibAction::Reveal(path.clone()));
                ui.close_menu();
            }
        });
    }

    fn folder(&mut self, ui: &mut Ui, folder: &Folder, id: &str, depth: usize, favorites: &BTreeSet<PathBuf>, actions: &mut Vec<LibAction>) {
        for (name, child) in &folder.children {
            let cid = format!("{id}/{name}");
            let count = count_files(child);
            egui::CollapsingHeader::new(RichText::new(format!("📁 {name}  ")).size(12.5))
                .id_salt(&cid)
                .default_open(false)
                .show(ui, |ui| self.folder(ui, child, &cid, depth + 1, favorites, actions))
                .header_response
                .on_hover_text(format!("{count} Dateien"));
        }
        for &i in &folder.files {
            self.row(ui, i, 2.0, favorites, actions);
        }
    }
}

fn count_files(f: &Folder) -> usize {
    f.files.len() + f.children.values().map(count_files).sum::<usize>()
}

fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.heading("Library");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("⟳").on_hover_text("Neu einlesen").clicked() {
                app.library.rescan(&app.settings);
            }
            if ui.button("+ Ordner").on_hover_text("Sample-Ordner hinzufügen").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    if !app.settings.library_roots.contains(&dir) {
                        app.settings.library_roots.push(dir);
                        app.settings.save(&app.paths.settings());
                    }
                }
            }
        });
    });
    ui.add(egui::TextEdit::singleline(&mut app.library.search).hint_text("🔍 Suchen (z.B. kick 808)").desired_width(f32::INFINITY));
    ui.horizontal(|ui| {
        for (f, label) in [(LibFilter::All, "Alle"), (LibFilter::Loops, "Loops"), (LibFilter::OneShots, "One-Shots"), (LibFilter::Favorites, "★")] {
            if ui.selectable_label(app.library.filter == f, label).clicked() {
                app.library.filter = f;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.settings.auto_preview, "Auto-Vorhören");
        ui.checkbox(&mut app.settings.preview_synced, "Tempo-Sync").on_hover_text("Loops beim Vorhören auf das Projekttempo stretchen");
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Vol").small());
        ui.add(egui::Slider::new(&mut app.settings.preview_volume, 0.0..=1.5).show_value(false));
        if ui.small_button("■").on_hover_text("Vorhören stoppen").clicked() {
            app.stop_preview();
        }
        if app.library.is_busy() {
            ui.spinner();
        }
    });
    ui.separator();

    let mut actions = Vec::new();
    let favorites = app.settings.favorites.clone();
    let roots = app.settings.library_roots.clone();
    egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("lib_scroll").show(ui, |ui| {
        if roots.is_empty() {
            ui.add_space(20.0);
            ui.label("Noch kein Sample-Ordner.");
            ui.weak("Füge deinen Sounds-Ordner hinzu, um loszulegen.");
            if ui.button("📁 Ordner wählen…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    app.settings.library_roots.push(dir);
                    app.settings.save(&app.paths.settings());
                }
            }
            return;
        }
        let lib = &mut app.library;
        if !lib.search.trim().is_empty() || lib.filter != LibFilter::All {
            let hits: Vec<usize> = (0..lib.entries.len()).filter(|&i| lib.matches(&lib.entries[i], &favorites)).take(500).collect();
            if hits.is_empty() {
                ui.weak("Keine Treffer");
            }
            for i in hits {
                lib.row(ui, i, 2.0, &favorites, &mut actions);
            }
        } else {
            let tree = std::mem::take(&mut lib.roots_tree);
            for (ri, root) in roots.iter().enumerate() {
                let name = root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| root.display().to_string());
                let resp = egui::CollapsingHeader::new(RichText::new(format!("🗀 {name}")).strong())
                    .id_salt(format!("root{ri}"))
                    .default_open(true)
                    .show(ui, |ui| {
                        if let Some(f) = tree.get(ri) {
                            lib.folder(ui, f, &format!("r{ri}"), 0, &favorites, &mut actions);
                        }
                    });
                resp.header_response.context_menu(|ui| {
                    if ui.button("Ordner aus Library entfernen").clicked() {
                        actions.push(LibAction::RemoveRoot(ri));
                        ui.close_menu();
                    }
                });
            }
            lib.roots_tree = tree;
        }
    });

    for a in actions {
        match a {
            LibAction::Select(p) => app.library.selected = Some(p),
            LibAction::Preview(p) => {
                if app.settings.auto_preview {
                    app.preview_path(&p);
                }
            }
            LibAction::AddToRack(p) => {
                app.add_sampler_channel(&p);
                if !tstudio_library::is_soundfont_file(&p) {
                    app.set_status("Sampler-Kanal hinzugefügt");
                }
            }
            LibAction::AddToPlaylist(p) => {
                let start = crate::ui::widgets::floor_to(app.position() as i64, app.project.ticks_per_bar());
                let track = app.playlist_ui.first_free_track(&app.project, start);
                app.add_audio_clip(&p, track, start);
                app.song_mode = true;
                app.view = View::Playlist;
            }
            LibAction::OpenTrim(p) => {
                app.fetch.open_in_trim(&p);
                app.view = View::Fetch;
            }
            LibAction::ToggleFav(p) => {
                if !app.settings.favorites.remove(&p) {
                    app.settings.favorites.insert(p);
                }
                app.settings.save(&app.paths.settings());
            }
            LibAction::Reveal(p) => reveal(&p),
            LibAction::RemoveRoot(i) => {
                if i < app.settings.library_roots.len() {
                    app.settings.library_roots.remove(i);
                    app.settings.save(&app.paths.settings());
                }
            }
        }
    }
}
