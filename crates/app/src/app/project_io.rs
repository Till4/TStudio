//! Project lifecycle: new/open/save/undo/redo and adding channels or audio clips.

use super::*;

impl TStudio {
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

    pub(super) fn confirm_discard(&mut self) -> bool {
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

    pub(super) fn save_to(&mut self, path: &Path) -> bool {
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
}
