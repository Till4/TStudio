use crate::compile::compile;
use crate::live::{Action, LiveServer};
use crate::midi::{Midi, MidiMsg};
use crate::samples::SampleCache;
use crate::settings::{Paths, Settings};
use crate::ui;
use crate::ui::widgets::{self, SampleDrag, ACCENT, TEXT_DIM};
use egui::{Color32, Key, RichText};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tstudio_dsp::stretch::StretchMode;
use tstudio_engine::{AudioConfig, Command, EngineHandle, PlayMode, Rack, PPQ};
use tstudio_project::{AudioClip, ClipKind, History, Id, Instrument, Note, Project, SamplerInstrument, SoundFontInstrument, StretchSettings};

mod playback;
mod project_io;
mod chrome;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Playlist,
    Rack,
    PianoRoll,
    Mixer,
    Instrument,
    Fetch,
}

impl View {
    pub const ALL: [(View, &'static str, &'static str); 6] = [
        (View::Playlist, "Playlist", "F5"),
        (View::Rack, "Channel Rack", "F6"),
        (View::PianoRoll, "Piano Roll", "F7"),
        (View::Mixer, "Mixer", "F9"),
        (View::Instrument, "Instrument", "F8"),
        (View::Fetch, "Sample-Cutter", "F10"),
    ];
}

pub struct TStudio {
    pub paths: Paths,
    pub settings: Settings,
    pub project: Project,
    pub history: History,
    pub project_path: Option<PathBuf>,
    pub saved: Project,
    pub engine: EngineHandle,
    pub cache: SampleCache,
    pub library: ui::library::LibraryState,
    pub midi: Midi,
    pub view: View,
    pub show_library: bool,
    pub song_mode: bool,
    pub metronome: bool,
    pub recording: bool,
    /// vocal/audio recording
    pub rec: ui::record::RecordState,
    pub current_pattern: Id,
    pub selected_channel: usize,
    /// last known pattern lengths (clips follow them until resized by hand)
    pub pattern_lens: std::collections::HashMap<tstudio_project::Id, i64>,
    pub playlist_ui: ui::playlist::PlaylistState,
    pub piano: ui::piano_roll::PianoState,
    pub mixer_ui: ui::mixer::MixerState,
    pub instrument_ui: ui::instrument::InstrumentState,
    pub fetch: ui::fetch::FetchState,
    pub export: ui::export::ExportState,
    pub show_settings: bool,
    pub settings_ui: ui::settings::SettingsUiState,
    pub status: Option<(String, Instant, bool)>,
    pub meters: Vec<(f32, f32)>,
    published: Option<Project>,
    published_gen: u64,
    rack_sig: u64,
    sent_mode: Option<PlayMode>,
    sent_metronome: Option<bool>,
    pending: VecDeque<Command>,
    pending_preview: Option<PathBuf>,
    last_autosave: Instant,
    rec_notes: HashMap<u8, (f64, f32)>,
    allow_close: bool,
    last_title: String,
    /// mtime of the project file as last loaded/saved (detects CLI edits)
    disk_stamp: Option<std::time::SystemTime>,
    last_disk_check: Instant,
    /// lets `tstudio-cli live …` edit the project open in this window
    live: Option<LiveServer>,
}

impl TStudio {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        widgets::apply_theme(&cc.egui_ctx);
        let paths = Paths::new();
        let settings = Settings::load(&paths.settings());
        let mut engine = EngineHandle::start(&AudioConfig { device: settings.audio_device.clone(), buffer_size: settings.buffer_size });
        let cache = SampleCache::new(engine.sample_rate);
        let library = ui::library::LibraryState::new(&paths, &settings);
        let mut midi = Midi::new();
        *midi.engine_tx.lock().unwrap() = engine.take_midi_sender();
        if let Some(port) = settings.midi_port.clone() {
            midi.connect(&port, cc.egui_ctx.clone());
        }
        let project = Project::new_default();
        let current_pattern = project.patterns[0].id;
        let fetch = ui::fetch::FetchState::new(&settings);
        let status = engine.status.clone().map(|s| (s, Instant::now(), true));
        let live = LiveServer::start(&paths.config_dir, cc.egui_ctx.clone());
        let mut app = Self {
            paths,
            history: History::new(&project),
            saved: project.clone(),
            project,
            project_path: None,
            engine,
            cache,
            library,
            midi,
            view: View::Rack,
            show_library: true,
            song_mode: false,
            metronome: false,
            recording: false,
            rec: Default::default(),
            current_pattern,
            selected_channel: 0,
            pattern_lens: Default::default(),
            playlist_ui: Default::default(),
            piano: Default::default(),
            mixer_ui: Default::default(),
            instrument_ui: Default::default(),
            fetch,
            export: Default::default(),
            show_settings: false,
            settings_ui: Default::default(),
            status,
            meters: Vec::new(),
            published: None,
            published_gen: 0,
            rack_sig: 0,
            sent_mode: None,
            sent_metronome: None,
            pending: VecDeque::new(),
            pending_preview: None,
            last_autosave: Instant::now(),
            rec_notes: HashMap::new(),
            allow_close: false,
            last_title: String::new(),
            disk_stamp: None,
            last_disk_check: Instant::now(),
            live,
            settings,
        };
        if let Some(p) = open {
            app.open_path(&p);
        }
        app
    }

    fn file_stamp(path: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    /// Picks up edits made by `tstudio-cli` (or anything else) to the open project file.
    /// Clean projects reload in place (playback keeps running, Ctrl+Z reverts the change);
    /// with unsaved edits the user is only warned.
    fn check_external_change(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_millis(700));
        if self.last_disk_check.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_disk_check = Instant::now();
        let Some(path) = self.project_path.clone() else { return };
        let stamp = Self::file_stamp(&path);
        if stamp.is_none() || stamp == self.disk_stamp {
            return;
        }
        self.disk_stamp = stamp;
        if self.is_dirty() {
            self.set_error("Projektdatei wurde extern geändert – du hast ungespeicherte Änderungen (Strg+S überschreibt, Datei > Öffnen lädt neu)");
            return;
        }
        match Project::load(&path) {
            Ok(p) => {
                if p == self.project {
                    return;
                }
                self.project = p;
                self.history.commit(&self.project);
                self.saved = self.project.clone();
                self.ensure_valid_selection();
                self.set_status("Projekt extern geändert – neu geladen (Strg+Z macht es rückgängig)");
            }
            // a writer may be mid-way; the next stamp change retries
            Err(_) => self.disk_stamp = None,
        }
    }

    /// Applies edits queued by `tstudio-cli` (each one undoable) and publishes the
    /// current project so the CLI always sees what is on screen, unsaved changes included.
    fn live_sync(&mut self, ctx: &egui::Context) {
        let Some(shared) = self.live.as_ref().map(|l| l.shared.clone()) else { return };
        if !ctx.input(|i| i.pointer.any_down()) {
            loop {
                let action = shared.lock().unwrap().actions.pop_front();
                let Some(action) = action else { break };
                match action {
                    Action::Put { project, label } => {
                        self.history.commit(&self.project);
                        self.project = project;
                        self.history.commit(&self.project);
                        self.ensure_valid_selection();
                        self.set_status(format!("Claude/CLI: {label} – Strg+Z macht es rückgängig"));
                    }
                    Action::Open { path, reply } => {
                        let r = if !self.confirm_discard() {
                            Err("Abgebrochen – der User hat ungespeicherte Änderungen behalten".to_string())
                        } else {
                            self.open_path(&path);
                            if self.project_path.as_deref() == Some(path.as_path()) {
                                Ok(json!({"ok": true, "opened": path}))
                            } else {
                                Err(format!("Konnte {} nicht öffnen", path.display()))
                            }
                        };
                        let _ = reply.send(r);
                    }
                    Action::Save { path, reply } => {
                        let r = match path.clone().or_else(|| self.project_path.clone()) {
                            None => Err("Projekt hat noch keinen Dateinamen – mit --as <datei.tsp> speichern".to_string()),
                            Some(p) => {
                                if path.is_some() {
                                    if let Some(stem) = p.file_stem() {
                                        self.project.name = stem.to_string_lossy().to_string();
                                    }
                                }
                                if self.save_to(&p) {
                                    Ok(json!({"ok": true, "saved": p}))
                                } else {
                                    Err(format!("Speichern nach {} fehlgeschlagen", p.display()))
                                }
                            }
                        };
                        let _ = reply.send(r);
                    }
                }
            }
        }
        let dirty = self.is_dirty();
        let Some(live) = self.live.as_mut() else { return };
        live.ensure_session_file();
        let changed = live.last_published.as_ref() != Some(&self.project) || live.last_path != self.project_path;
        let mut s = shared.lock().unwrap();
        // while CLI edits are queued, the snapshot already holds the newer state
        if s.actions.is_empty() {
            if changed {
                s.project_ron = self.project.to_ron().unwrap_or_default();
                s.name = self.project.name.clone();
                s.path = self.project_path.clone();
                live.last_published = Some(self.project.clone());
                live.last_path = self.project_path.clone();
            }
            s.dirty = dirty;
        }
    }

    // ------------------------------------------------------------ engine i/o

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), false));
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), true));
    }

    pub fn current_pattern_index(&self) -> Option<usize> {
        self.project.patterns.iter().position(|p| p.id == self.current_pattern)
    }

    pub fn ensure_valid_selection(&mut self) {
        if self.project.patterns.is_empty() {
            self.project.add_pattern();
        }
        if self.current_pattern_index().is_none() {
            self.current_pattern = self.project.patterns[0].id;
        }
        if self.selected_channel >= self.project.channels.len() {
            self.selected_channel = self.project.channels.len().saturating_sub(1);
        }
        self.midi.target_channel.store(self.selected_channel, Ordering::Relaxed);
    }

    pub fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    fn autosave(&mut self) {
        if self.last_autosave.elapsed() > Duration::from_secs(120) {
            self.last_autosave = Instant::now();
            if self.is_dirty() {
                let _ = self.project.save(&self.paths.autosave());
            }
            self.library.save_cache();
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let t = format!("TStudio — {}{}", self.project.name, if self.is_dirty() { " •" } else { "" });
        if t != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.last_title = t;
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.is_dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if self.confirm_discard() {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                self.allow_close = true;
            }
            if self.allow_close {
                if let Some(l) = &self.live {
                    l.shutdown();
                }
                self.library.save_cache();
                self.settings.save(&self.paths.settings());
            }
        }
    }
}

impl eframe::App for TStudio {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_close(ctx);
        self.ensure_valid_selection();
        self.cache.poll();
        let lib_changed = self.library.poll(&self.settings);
        if lib_changed {
            ctx.request_repaint();
        }
        self.try_send_preview();
        self.handle_dropped_files(ctx);
        self.handle_shortcuts(ctx);
        self.handle_midi();
        ui::fetch::poll(self);
        ui::export::poll(self);
        ui::record::poll(self, ctx);
        self.update_meters();

        egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu_bar(ui));
        egui::TopBottomPanel::top("transport").frame(egui::Frame::default().fill(widgets::BG_DARK).inner_margin(egui::Margin::symmetric(8.0, 5.0))).show(ctx, |ui| self.transport_bar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        if self.show_library {
            egui::SidePanel::left("library").resizable(true).default_width(290.0).width_range(200.0..=600.0).show(ctx, |ui| ui::library::show(self, ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::default().fill(widgets::BG_PANEL).inner_margin(egui::Margin::same(6.0))).show(ctx, |ui| match self.view {
            View::Playlist => ui::playlist::show(self, ui),
            View::Rack => ui::rack::show(self, ui),
            View::PianoRoll => ui::piano_roll::show(self, ui),
            View::Mixer => ui::mixer::show(self, ui),
            View::Instrument => ui::instrument::show(self, ui),
            View::Fetch => ui::fetch::show(self, ui),
        });
        ui::settings::window(self, ctx);
        ui::export::window(self, ctx);
        ui::record::window(self, ctx);
        widgets::draw_drag_ghost(ctx);
        if !ctx.input(|i| i.pointer.any_down()) {
            egui::DragAndDrop::clear_payload(ctx);
        }

        self.project.follow_pattern_lengths(&mut self.pattern_lens);
        self.ensure_valid_selection();
        self.publish();
        self.sync_mode();
        self.flush();
        self.engine.collect_garbage();
        let keep = self.project.referenced_samples();
        self.cache.gc(&keep);
        if !ctx.input(|i| i.pointer.any_down()) && !ctx.wants_keyboard_input() {
            self.history.commit(&self.project);
        }
        self.autosave();
        self.check_external_change(ctx);
        self.live_sync(ctx);
        self.update_title(ctx);

        let busy = self.is_playing() || self.fetch.is_busy() || self.export.is_busy() || self.library.is_busy() || self.pending_preview.is_some() || !self.pending.is_empty();
        if busy {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// Re-export for UI modules that build drag payloads.
pub fn sample_drag(path: &Path) -> SampleDrag {
    SampleDrag { path: path.to_path_buf() }
}
