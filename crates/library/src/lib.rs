//! Sample library: decoding (symphonia), folder scanning, background analysis
//! (BPM / key / waveform) with an on-disk cache, and WAV writing.

use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tstudio_dsp::{analysis, SampleData};

pub mod decode;
pub use decode::{decode_file, probe_duration};

pub const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "ogg", "aif", "aiff", "m4a", "aac", "mp4", "wave"];

pub fn is_audio_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| AUDIO_EXTS.contains(&e.to_lowercase().as_str())).unwrap_or(false)
}

pub fn is_soundfont_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("sf2")).unwrap_or(false)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    pub path: PathBuf,
    /// index into the roots list
    pub root: usize,
    /// folder relative to the root, with forward slashes ("" = root itself)
    pub rel_dir: String,
    pub name: String,
    pub ext: String,
    pub size: u64,
    pub modified: u64,
}

fn file_meta(p: &Path) -> (u64, u64) {
    match std::fs::metadata(p) {
        Ok(m) => (m.len(), m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)),
        Err(_) => (0, 0),
    }
}

/// Recursively lists audio and SoundFont (.sf2) files under each root.
pub fn scan(roots: &[PathBuf]) -> Vec<LibraryEntry> {
    let mut out = Vec::new();
    for (ri, root) in roots.iter().enumerate() {
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if depth < 24 {
                        stack.push((p, depth + 1));
                    }
                } else if is_audio_file(&p) || is_soundfont_file(&p) {
                    let rel_dir = p
                        .parent()
                        .and_then(|d| d.strip_prefix(root).ok())
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    let (size, modified) = file_meta(&p);
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name);
                    out.push(LibraryEntry { path: p, root: ri, rel_dir, name: stem, ext, size, modified });
                }
            }
        }
    }
    out.sort_by(|a, b| (a.root, a.rel_dir.to_lowercase(), a.name.to_lowercase()).cmp(&(b.root, b.rel_dir.to_lowercase(), b.name.to_lowercase())));
    out
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Analysis {
    pub duration: f32,
    pub sample_rate: u32,
    pub bpm: Option<f64>,
    /// true if the tempo came from the file name rather than detection
    pub bpm_from_name: bool,
    pub key: Option<String>,
    /// 128 min/max pairs for thumbnails
    pub peaks: Vec<(f32, f32)>,
    pub peak_level: f32,
}

impl Analysis {
    /// Heuristic: long enough and rhythmic.
    pub fn is_loop(&self) -> bool {
        self.bpm.is_some() && self.duration >= 1.0
    }
}

pub fn analyze_data(data: &SampleData, name: &str) -> Analysis {
    let mono = data.mono();
    let sr = data.sample_rate;
    let dur = data.duration_secs() as f32;
    let from_name = analysis::bpm_from_name(name);
    let bpm = from_name.or_else(|| if dur >= 1.0 { analysis::detect_bpm(&mono, sr) } else { None });
    let key = if dur >= 0.4 { analysis::detect_key(&mono, sr) } else { None };
    Analysis {
        duration: dur,
        sample_rate: sr,
        bpm,
        bpm_from_name: from_name.is_some(),
        key,
        peaks: data.peaks(128),
        peak_level: data.peak(),
    }
}

pub fn analyze_file(path: &Path) -> Result<Analysis, String> {
    let data = decode_file(path)?;
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Ok(analyze_data(&data, &name))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cached {
    size: u64,
    modified: u64,
    analysis: Analysis,
}

/// Analysis results keyed by path, invalidated by size/mtime.
#[derive(Default, Serialize, Deserialize)]
pub struct AnalysisCache {
    entries: HashMap<String, Cached>,
    #[serde(skip)]
    dirty: bool,
}

impl AnalysisCache {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string(self) {
            if std::fs::write(path, s).is_ok() {
                self.dirty = false;
            }
        }
    }

    pub fn get(&self, path: &Path) -> Option<&Analysis> {
        let (size, modified) = file_meta(path);
        self.entries
            .get(&path.to_string_lossy().to_string())
            .filter(|c| c.size == size && c.modified == modified)
            .map(|c| &c.analysis)
    }

    pub fn insert(&mut self, path: &Path, analysis: Analysis) {
        let (size, modified) = file_meta(path);
        self.entries.insert(path.to_string_lossy().to_string(), Cached { size, modified, analysis });
        self.dirty = true;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Background analysis worker pool.
pub struct Analyzer {
    tx: Sender<PathBuf>,
    pub results: Receiver<(PathBuf, Result<Analysis, String>)>,
}

impl Analyzer {
    pub fn new(threads: usize) -> Self {
        let (tx, rx) = unbounded::<PathBuf>();
        let (rtx, rrx) = unbounded();
        for i in 0..threads.max(1) {
            let rx = rx.clone();
            let rtx = rtx.clone();
            std::thread::Builder::new()
                .name(format!("tstudio-analyze-{i}"))
                .spawn(move || {
                    while let Ok(p) = rx.recv() {
                        let r = analyze_file(&p);
                        if rtx.send((p, r)).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawn analyzer");
        }
        Self { tx, results: rrx }
    }

    pub fn request(&self, path: PathBuf) {
        let _ = self.tx.send(path);
    }

    pub fn pending(&self) -> usize {
        self.tx.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    Pcm16,
    Pcm24,
    Float32,
}

pub fn write_wav(path: &Path, data: &SampleData, fmt: WavFormat) -> Result<(), String> {
    let (bits, sample_format) = match fmt {
        WavFormat::Pcm16 => (16, hound::SampleFormat::Int),
        WavFormat::Pcm24 => (24, hound::SampleFormat::Int),
        WavFormat::Float32 => (32, hound::SampleFormat::Float),
    };
    let spec = hound::WavSpec { channels: 2, sample_rate: data.sample_rate, bits_per_sample: bits, sample_format };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for (l, r) in data.left.iter().zip(&data.right) {
        for v in [*l, *r] {
            let v = v.clamp(-1.0, 1.0);
            match fmt {
                WavFormat::Pcm16 => w.write_sample((v * 32767.0) as i16).map_err(|e| e.to_string())?,
                WavFormat::Pcm24 => w.write_sample((v * 8_388_607.0) as i32).map_err(|e| e.to_string())?,
                WavFormat::Float32 => w.write_sample(v).map_err(|e| e.to_string())?,
            }
        }
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Picks a free file name: `name.ext`, `name (2).ext`, ...
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let clean: String = stem.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    let clean = if clean.is_empty() { "sample".to_string() } else { clean };
    let mut p = dir.join(format!("{clean}.{ext}"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{clean} ({i}).{ext}"));
        i += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_roundtrip_and_scan() {
        let dir = std::env::temp_dir().join(format!("tstudio-lib-test-{}", std::process::id()));
        let sub = dir.join("Drums");
        std::fs::create_dir_all(&sub).unwrap();
        let v: Vec<f32> = (0..4410).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        let data = SampleData::new(v.clone(), v, 44100);
        let p = sub.join("kick 01.wav");
        write_wav(&p, &data, WavFormat::Pcm24).unwrap();
        let back = decode_file(&p).unwrap();
        assert_eq!(back.len(), data.len());
        assert_eq!(back.sample_rate, 44100);
        assert!((back.left[100] - data.left[100]).abs() < 1e-4);

        let entries = scan(&[dir.clone()]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rel_dir, "Drums");
        assert_eq!(entries[0].name, "kick 01");

        let a = analyze_file(&p).unwrap();
        assert!((a.duration - 0.1).abs() < 0.01);
        let mut cache = AnalysisCache::default();
        cache.insert(&p, a.clone());
        assert_eq!(cache.get(&p), Some(&a));
        assert_eq!(unique_path(&sub, "kick 01", "wav"), sub.join("kick 01 (2).wav"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
