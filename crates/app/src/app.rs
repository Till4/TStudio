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

    pub fn send(&mut self, cmd: Command) {
        // coalesce snapshot updates that could not be delivered yet
        if matches!(cmd, Command::SetSnapshot(_)) {
            if let Some(Command::SetSnapshot(_)) = self.pending.back() {
                self.pending.pop_back();
            }
        }
        self.pending.push_back(cmd);
        self.flush();
    }

    fn flush(&mut self) {
        while let Some(cmd) = self.pending.pop_front() {
            if let Err(cmd) = self.engine.send(cmd) {
                self.pending.push_front(cmd);
                break;
            }
        }
    }

    pub fn note_on(&mut self, channel: usize, key: u8, velocity: f32) {
        self.send(Command::NoteOn { channel, key, velocity });
    }

    pub fn note_off(&mut self, channel: usize, key: u8) {
        self.send(Command::NoteOff { channel, key });
    }

    /// Play/stop. Starting in the channel rack, piano roll or instrument view plays the current
    /// pattern; starting in the playlist plays the song. Other views keep the PAT/SONG switch.
    pub fn play_toggle(&mut self) {
        if self.engine.shared.is_playing() {
            self.send(Command::Stop);
        } else {
            match self.view {
                View::Rack | View::PianoRoll | View::Instrument => self.song_mode = false,
                View::Playlist => self.song_mode = true,
                _ => {}
            }
            self.sync_mode();
            self.send(Command::Play);
        }
    }

    /// Plays (or stops) only the current pattern.
    pub fn play_pattern_toggle(&mut self) {
        if self.engine.shared.is_playing() && !self.song_mode {
            self.send(Command::Stop);
        } else {
            self.song_mode = false;
            self.sync_mode();
            self.send(Command::Play);
        }
    }

    pub fn is_playing(&self) -> bool {
        self.engine.shared.is_playing()
    }

    pub fn position(&self) -> f64 {
        self.engine.shared.position()
    }

    /// Previews a library file (optionally tempo-synced to the project).
    pub fn preview_path(&mut self, path: &Path) {
        if tstudio_library::is_soundfont_file(path) {
            return;
        }
        self.pending_preview = Some(path.to_path_buf());
        self.try_send_preview();
    }

    fn try_send_preview(&mut self) {
        let Some(path) = self.pending_preview.clone() else { return };
        let data = if self.settings.preview_synced {
            let bpm = self.library.analysis.get(&path).and_then(|a| a.bpm);
            match bpm {
                Some(b) => {
                    let st = StretchSettings { mode: StretchMode::Stretch, source_bpm: b, ..Default::default() };
                    self.cache.get_prepared(&path, &st, self.project.bpm).map(|l| l.data)
                }
                None => self.cache.get_raw(&path),
            }
        } else {
            self.cache.get_raw(&path)
        };
        if let Some(d) = data {
            self.pending_preview = None;
            let vol = self.settings.preview_volume;
            self.send(Command::PreviewSample(d, vol));
        } else if self.cache.error(&path).is_some() {
            self.pending_preview = None;
        }
    }

    pub fn preview_data(&mut self, data: Arc<tstudio_dsp::SampleData>) {
        self.pending_preview = None;
        self.cache.hold("preview", Arc::clone(&data));
        let vol = self.settings.preview_volume;
        self.send(Command::PreviewSample(data, vol));
    }

    pub fn stop_preview(&mut self) {
        self.pending_preview = None;
        self.send(Command::StopPreview);
    }

    fn desired_mode(&self) -> PlayMode {
        if self.song_mode {
            PlayMode::Song
        } else {
            PlayMode::Pattern(self.current_pattern_index().unwrap_or(0))
        }
    }

    pub fn sync_mode(&mut self) {
        let m = self.desired_mode();
        if self.sent_mode != Some(m) {
            self.sent_mode = Some(m);
            self.send(Command::SetMode(m));
        }
        if self.sent_metronome != Some(self.metronome) {
            self.sent_metronome = Some(self.metronome);
            self.send(Command::SetMetronome(self.metronome));
        }
    }

    fn publish(&mut self) {
        let gen = self.cache.generation;
        if self.published.as_ref() == Some(&self.project) && gen == self.published_gen {
            return;
        }
        let snap = Arc::new(compile(&self.project, &mut self.cache, false));
        let sig = snap.signature();
        if sig != self.rack_sig {
            let rack = Box::new(Rack::build(&snap, self.engine.sample_rate as f32, tstudio_engine::MAX_CHUNK));
            self.rack_sig = sig;
            self.send(Command::SetRack(rack, snap));
        } else {
            self.send(Command::SetSnapshot(snap));
        }
        self.published = Some(self.project.clone());
        // compile may have started loads; their completion bumps the generation again
        self.published_gen = gen;
    }

    pub fn restart_engine(&mut self) {
        // the microphone is tied to the old engine's clock and monitor queue
        ui::record::close_input(self);
        let cfg = AudioConfig { device: self.settings.audio_device.clone(), buffer_size: self.settings.buffer_size };
        // drop the old stream first so the device is free
        let old = std::mem::replace(&mut self.engine, EngineHandle::start_null(48000));
        drop(old);
        self.engine = EngineHandle::start(&cfg);
        *self.midi.engine_tx.lock().unwrap() = self.engine.take_midi_sender();
        if self.engine.sample_rate != self.cache.sample_rate() {
            self.cache.reset(self.engine.sample_rate);
        }
        self.pending.clear();
        self.published = None;
        self.rack_sig = 0;
        self.sent_mode = None;
        self.sent_metronome = None;
        match self.engine.status.clone() {
            Some(e) => self.set_error(e),
            None => self.set_status(format!("Audio: {} @ {} Hz", self.engine.device_name, self.engine.sample_rate)),
        }
    }

    // ------------------------------------------------------------ helpers

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

    /// Creates a sampler channel for `path` and selects it.
    /// New SoundFont channel with the file's first preset (drum kits: bank 128).
    pub fn add_soundfont_channel(&mut self, path: &Path) -> Option<usize> {
        match tstudio_engine::soundfont_presets(path) {
            Ok(presets) if !presets.is_empty() => {
                let p = &presets[0];
                let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "SoundFont".into());
                let name = if presets.len() == 1 && !p.name.is_empty() { p.name.clone() } else { stem };
                let inst = SoundFontInstrument { path: path.to_path_buf(), bank: p.bank, program: p.program, preset_name: p.name.clone(), gain: 1.0 };
                let idx = self.project.add_channel(&name, Instrument::SoundFont(inst));
                self.selected_channel = idx;
                self.set_status(&format!("SoundFont-Kanal: {} Presets – Auswahl im Instrument-Fenster (F8)", presets.len()));
                Some(idx)
            }
            Ok(_) => {
                self.set_status("SoundFont enthält keine Presets");
                None
            }
            Err(e) => {
                self.set_status(&e);
                None
            }
        }
    }

    pub fn add_sampler_channel(&mut self, path: &Path) -> usize {
        if tstudio_library::is_soundfont_file(path) {
            return self.add_soundfont_channel(path).unwrap_or(self.selected_channel);
        }
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sampler".into());
        let mut inst = SamplerInstrument { sample: Some(path.to_path_buf()), ..Default::default() };
        if let Some(a) = self.library.analysis.get(path) {
            if a.is_loop() {
                inst.stretch.source_bpm = a.bpm.unwrap_or(120.0);
            }
        }
        let idx = self.project.add_channel(&name, Instrument::Sampler(inst));
        self.selected_channel = idx;
        self.cache.get_raw(path);
        idx
    }

    /// Length in ticks and stretch settings for a new audio clip.
    pub fn audio_clip_for(&mut self, path: &Path) -> (i64, AudioClip) {
        let analysis = self.library.analysis.get(path).cloned();
        let dur = analysis
            .as_ref()
            .map(|a| a.duration as f64)
            .or_else(|| tstudio_library::probe_duration(path))
            .or_else(|| self.cache.get_raw(path).map(|d| d.duration_secs()))
            .unwrap_or(4.0);
        let mut clip = AudioClip { path: path.to_path_buf(), ..Default::default() };
        let len = match analysis.as_ref().and_then(|a| if a.is_loop() { a.bpm } else { None }) {
            Some(bpm) => {
                clip.stretch = StretchSettings { mode: StretchMode::Stretch, source_bpm: bpm, ..Default::default() };
                (dur * bpm / 60.0 * PPQ as f64).round() as i64
            }
            None => self.project.secs_to_ticks(dur).round() as i64,
        };
        self.cache.get_raw(path);
        (len.max(1), clip)
    }

    pub fn add_audio_clip(&mut self, path: &Path, track: usize, start: i64) -> Id {
        let (len, mut clip) = self.audio_clip_for(path);
        // every sample gets its own mixer insert, so it can take effects and automation
        clip.mixer_track = self.project.sample_insert(path);
        self.project.add_clip(track, start, len, ClipKind::Audio(clip))
    }

    // ------------------------------------------------------------ files

    fn confirm_discard(&mut self) -> bool {
        if !self.is_dirty() {
            return true;
        }
        let r = rfd::MessageDialog::new()
            .set_title("TStudio")
            .set_description("Ungespeicherte Änderungen speichern?")
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match r {
            rfd::MessageDialogResult::Yes => self.save(),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    pub fn load_project(&mut self, p: Project, path: Option<PathBuf>) {
        self.send(Command::Stop);
        self.project = p;
        self.history.reset(&self.project);
        self.pattern_lens.clear();
        self.saved = self.project.clone();
        self.disk_stamp = path.as_deref().and_then(Self::file_stamp);
        self.project_path = path;
        self.current_pattern = self.project.patterns.first().map(|p| p.id).unwrap_or(0);
        self.selected_channel = 0;
        self.piano.clear_selection();
        self.playlist_ui.selected.clear();
        self.ensure_valid_selection();
    }

    pub fn new_project(&mut self) {
        if self.confirm_discard() {
            self.load_project(Project::new_default(), None);
            self.set_status("Neues Projekt");
        }
    }

    pub fn open_dialog(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let mut d = rfd::FileDialog::new().add_filter("TStudio Projekt", &["tsp"]);
        if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        }
        if let Some(path) = d.pick_file() {
            self.open_path(&path);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(p) => {
                self.load_project(p, Some(path.to_path_buf()));
                self.settings.push_recent(path);
                self.settings.save(&self.paths.settings());
                let missing = self.project.referenced_samples().into_iter().filter(|p| !p.exists()).count();
                if missing > 0 {
                    self.set_error(format!("Projekt geladen – {missing} Sample-Datei(en) fehlen"));
                } else {
                    self.set_status(format!("Geöffnet: {}", path.display()));
                }
            }
            Err(e) => self.set_error(format!("Konnte Projekt nicht laden: {e}")),
        }
    }

    pub fn save(&mut self) -> bool {
        match self.project_path.clone() {
            Some(p) => self.save_to(&p),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) -> bool {
        let name = format!("{}.tsp", self.project.name);
        let mut d = rfd::FileDialog::new().add_filter("TStudio Projekt", &["tsp"]).set_file_name(name);
        if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        }
        match d.save_file() {
            Some(mut p) => {
                if p.extension().is_none() {
                    p.set_extension("tsp");
                }
                if let Some(stem) = p.file_stem() {
                    self.project.name = stem.to_string_lossy().to_string();
                }
                self.save_to(&p)
            }
            None => false,
        }
    }

    fn save_to(&mut self, path: &Path) -> bool {
        self.history.commit(&self.project);
        match self.project.save(path) {
            Ok(()) => {
                self.saved = self.project.clone();
                self.disk_stamp = Self::file_stamp(path);
                self.project_path = Some(path.to_path_buf());
                self.settings.push_recent(path);
                self.settings.save(&self.paths.settings());
                self.set_status(format!("Gespeichert: {}", path.display()));
                true
            }
            Err(e) => {
                self.set_error(format!("Speichern fehlgeschlagen: {e}"));
                false
            }
        }
    }

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.project) {
            self.piano.clear_selection();
            self.set_status("Rückgängig");
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.project) {
            self.piano.clear_selection();
            self.set_status("Wiederhergestellt");
        }
    }

    // ------------------------------------------------------------ frame

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
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

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
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

    fn handle_midi(&mut self) {
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

    fn transport_bar(&mut self, ui: &mut egui::Ui) {
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
            let bar = self.project.ticks_per_bar() as f64;
            let b = (pos / bar).floor() as i64 + 1;
            let beat = ((pos % bar) / PPQ as f64).floor() as i64 + 1;
            let step = ((pos % PPQ as f64) / (PPQ as f64 / 4.0)).floor() as i64 + 1;
            ui.label(RichText::new(format!("{b:>3}:{beat}:{step}")).monospace().size(15.0).color(ACCENT));
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

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            widgets::logo(ui, 18.0);
            ui.menu_button("Datei", |ui| {
                if ui.button("Neu  (Strg+N)").clicked() {
                    ui.close_menu();
                    self.new_project();
                }
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
                ui.separator();
                if ui.button("Exportieren (WAV/MP3)…").clicked() {
                    ui.close_menu();
                    self.export.open = true;
                }
                if ui.button("Autosave wiederherstellen").clicked() {
                    ui.close_menu();
                    let p = self.paths.autosave();
                    if p.exists() && self.confirm_discard() {
                        match Project::load(&p) {
                            Ok(proj) => {
                                self.load_project(proj, None);
                                self.set_status("Autosave wiederhergestellt");
                            }
                            Err(e) => self.set_error(e),
                        }
                    }
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
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

    fn update_meters(&mut self) {
        let n = self.project.mixer.len();
        self.meters.resize(n, (0.0, 0.0));
        for (i, m) in self.meters.iter_mut().enumerate() {
            let (l, r) = self.engine.shared.take_meter(i);
            m.0 = l.max(m.0 * 0.85);
            m.1 = r.max(m.1 * 0.85);
        }
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
