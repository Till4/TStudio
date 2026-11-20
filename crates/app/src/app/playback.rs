//! Transport and engine plumbing: command queue, note preview, play modes, engine restart.

use super::*;

impl TStudio {
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

    pub(super) fn flush(&mut self) {
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
                View::Rack | View::PianoRoll | View::Instrument | View::DrumPad => self.song_mode = false,
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

    pub(super) fn try_send_preview(&mut self) {
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

    pub(super) fn desired_mode(&self) -> PlayMode {
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
        // recording can switch the click on for the duration of a take
        let click = self.metronome || (self.rec.active && self.settings.record_click);
        if self.sent_metronome != Some(click) {
            self.sent_metronome = Some(click);
            self.send(Command::SetMetronome(click));
        }
    }

    pub(super) fn publish(&mut self) {
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
}
