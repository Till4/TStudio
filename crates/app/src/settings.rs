use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub library_roots: Vec<PathBuf>,
    /// Where downloads and trimmed samples are saved.
    pub sample_out_dir: Option<PathBuf>,
    pub ffmpeg_path: Option<PathBuf>,
    pub ytdlp_path: Option<PathBuf>,
    /// Python with Demucs for stem separation (None = search automatically)
    pub python_path: Option<PathBuf>,
    /// CPU threads for stem separation (0 = half of the cores)
    pub stem_threads: u32,
    pub audio_device: Option<String>,
    /// microphone for vocal recording (None = Windows default)
    pub input_device: Option<String>,
    /// hear the microphone through the Vocals insert while recording
    pub monitor_input: bool,
    /// manual timing correction for recordings
    pub record_offset_ms: f32,
    pub buffer_size: Option<u32>,
    pub midi_port: Option<String>,
    pub favorites: BTreeSet<PathBuf>,
    pub recent_projects: Vec<PathBuf>,
    pub preview_volume: f32,
    pub auto_preview: bool,
    pub preview_synced: bool,
    pub follow_playhead: bool,
    /// default sample folders were added once (removing them later is respected)
    pub roots_seeded: bool,
    /// DJ / artist name shown in the MP4 visualizer
    pub dj_name: String,
    pub video_accent: [u8; 3],
    /// "1080p", "720p", "vertical", "square"
    pub video_size: String,
    pub video_watermark: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            library_roots: Vec::new(),
            sample_out_dir: None,
            ffmpeg_path: None,
            ytdlp_path: None,
            python_path: None,
            stem_threads: 0,
            audio_device: None,
            input_device: None,
            monitor_input: false,
            record_offset_ms: 0.0,
            buffer_size: None,
            midi_port: None,
            favorites: BTreeSet::new(),
            recent_projects: Vec::new(),
            roots_seeded: false,
            preview_volume: 0.8,
            auto_preview: true,
            preview_synced: false,
            follow_playhead: true,
            dj_name: String::new(),
            video_accent: tstudio_fetch::video::ACCENTS[0].1,
            video_size: "1080p".into(),
            video_watermark: true,
        }
    }
}

pub struct Paths {
    pub config_dir: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        let config_dir = directories::ProjectDirs::from("de", "TStudio", "TStudio")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir().join("TStudio"));
        let _ = std::fs::create_dir_all(&config_dir);
        let _ = std::fs::create_dir_all(config_dir.join("presets"));
        Self { config_dir }
    }
    pub fn settings(&self) -> PathBuf {
        self.config_dir.join("settings.ron")
    }
    pub fn analysis_cache(&self) -> PathBuf {
        self.config_dir.join("analysis-cache.json")
    }
    pub fn autosave(&self) -> PathBuf {
        self.config_dir.join("autosave.tsp")
    }
    pub fn presets_dir(&self) -> PathBuf {
        self.config_dir.join("presets")
    }
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        let mut s: Settings = std::fs::read_to_string(path).ok().and_then(|s| ron::from_str(&s).ok()).unwrap_or_default();
        if !s.roots_seeded {
            s.roots_seeded = true;
            if s.library_roots.is_empty() {
                s.library_roots = default_sample_folders();
                if !s.library_roots.is_empty() && s.sample_out_dir.is_none() {
                    // keep downloads out of other programs' folders
                    s.sample_out_dir = Some(Settings::default().output_dir());
                }
            }
        }
        s
    }

    pub fn save(&self, path: &Path) {
        if let Ok(s) = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()) {
            let _ = std::fs::write(path, s);
        }
    }

    /// Folder for downloads / trimmed samples (inside the first library root if possible).
    pub fn output_dir(&self) -> PathBuf {
        if let Some(d) = &self.sample_out_dir {
            return d.clone();
        }
        if let Some(root) = self.library_roots.first() {
            return root.join("_TStudio");
        }
        directories::UserDirs::new()
            .and_then(|u| u.audio_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("TStudio Samples")
    }

    pub fn push_recent(&mut self, p: &Path) {
        self.recent_projects.retain(|x| x != p);
        self.recent_projects.insert(0, p.to_path_buf());
        self.recent_projects.truncate(10);
    }
}

/// Sample / SoundFont folders of other music software found in Documents and Music.
pub fn default_sample_folders() -> Vec<PathBuf> {
    let Some(u) = directories::UserDirs::new() else { return Vec::new() };
    let mut cands = Vec::new();
    if let Some(d) = u.document_dir() {
        cands.push(d.join("lmms").join("samples"));
        cands.push(d.join("Image-Line").join("Downloads").join("FL Studio Mobile Factory Data"));
        cands.push(d.join("Image-Line").join("FL Studio").join("Audio"));
        cands.push(d.join("Ableton").join("User Library").join("Samples"));
        cands.push(d.join("SoundFonts"));
        cands.push(d.join("Samples"));
    }
    if let Some(m) = u.audio_dir() {
        cands.push(m.join("TStudio Samples"));
        cands.push(m.join("Samples"));
    }
    cands.into_iter().filter(|p| p.is_dir()).collect()
}
