//! Rendering and audio analysis (all synchronous – no audio device needed).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tstudio_dsp::stretch::{change_tempo_ex, StretchMode};
use tstudio_dsp::{gain_to_db, SampleData};
use tstudio_engine::{compile_project, render_offline, PlayMode};
use tstudio_project::Project;

/// Decodes + resamples + tempo-fits samples on demand, caching every variant.
pub struct SyncSamples {
    sr: u32,
    raw: HashMap<PathBuf, Option<Arc<SampleData>>>,
    prepared: HashMap<(PathBuf, StretchMode, i64, i64, i32, bool), Arc<SampleData>>,
    pub missing: Vec<String>,
}

impl SyncSamples {
    pub fn new(sr: u32) -> Self {
        Self { sr, raw: HashMap::new(), prepared: HashMap::new(), missing: Vec::new() }
    }

    fn raw(&mut self, path: &Path) -> Option<Arc<SampleData>> {
        if let Some(v) = self.raw.get(path) {
            return v.clone();
        }
        let v = match tstudio_library::decode_file(path) {
            Ok(d) => Some(Arc::new(if d.sample_rate != self.sr { d.resample_to(self.sr) } else { d })),
            Err(e) => {
                self.missing.push(format!("{}: {e}", path.display()));
                None
            }
        };
        self.raw.insert(path.to_path_buf(), v.clone());
        v
    }

    pub fn get(&mut self, path: &Path, st: &tstudio_project::StretchSettings, bpm: f64) -> Option<Arc<SampleData>> {
        let raw = self.raw(path)?;
        let (mode, src, dst) = st.tempo_job(bpm);
        let pitch = st.pitch;
        let key = (path.to_path_buf(), mode, (src * 1000.0).round() as i64, (dst * 1000.0).round() as i64, (pitch * 100.0).round() as i32, st.formant);
        if (mode == StretchMode::Off || key.2 == key.3) && key.4 == 0 {
            return Some(raw);
        }
        if let Some(d) = self.prepared.get(&key) {
            return Some(d.clone());
        }
        let d = Arc::new(change_tempo_ex(&raw, mode, src, dst, pitch, key.5));
        self.prepared.insert(key, d.clone());
        Some(d)
    }
}

pub struct Rendered {
    pub data: SampleData,
    pub missing: Vec<String>,
}

pub fn render(p: &Project, sr: u32, mode: PlayMode, tail: f64) -> Result<Rendered, String> {
    let mut samples = SyncSamples::new(sr);
    let snap = compile_project(p, &mut |path, st, bpm| samples.get(path, st, bpm), true);
    if matches!(mode, PlayMode::Song) && snap.song_end <= 0 {
        return Err("Playlist ist leer – nichts zu rendern (Clips mit `clip add` hinzufügen oder --pattern nutzen)".into());
    }
    let data = render_offline(Arc::new(snap), sr, mode, tail, |_| true).ok_or("Rendern abgebrochen")?;
    Ok(Rendered { data, missing: samples.missing })
}

fn db(v: f32) -> Value {
    if v <= 1e-9 {
        json!(null)
    } else {
        json!((gain_to_db(v) * 10.0).round() / 10.0)
    }
}

fn rms(xs: impl Iterator<Item = f32>) -> f32 {
    let (mut s, mut n) = (0.0f64, 0usize);
    for x in xs {
        s += (x as f64) * (x as f64);
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        (s / n as f64).sqrt() as f32
    }
}

/// Level / timing / tempo facts about a piece of audio, as JSON.
/// `bpm` (if given) adds a per-bar loudness map so silent bars are easy to spot.
pub fn stats(d: &SampleData, name: &str, bpm_bars: Option<(f64, u32)>) -> Value {
    let sr = d.sample_rate as usize;
    let mono = d.mono();
    let peak = d.peak();
    let clipped = d.left.iter().chain(&d.right).filter(|x| x.abs() >= 0.999).count();
    let first = mono.iter().position(|x| x.abs() > 1e-3);
    let last = mono.iter().rposition(|x| x.abs() > 1e-3);
    let onsets = tstudio_dsp::analysis::transients(&mono, d.sample_rate, 0.5, 60.0);
    let a = tstudio_library::analyze_data(d, name);
    let corr = {
        let (mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64);
        for (l, r) in d.left.iter().zip(&d.right) {
            lr += (*l * *r) as f64;
            ll += (*l * *l) as f64;
            rr += (*r * *r) as f64;
        }
        if ll > 0.0 && rr > 0.0 {
            (lr / (ll * rr).sqrt() * 100.0).round() / 100.0
        } else {
            1.0
        }
    };
    let mut v = json!({
        "duration_s": (d.duration_secs() * 1000.0).round() / 1000.0,
        "sample_rate": d.sample_rate,
        "peak_db": db(peak),
        "rms_db": db(rms(mono.iter().copied())),
        "clipped_samples": clipped,
        "silent": first.is_none(),
        "sound_start_s": first.map(|f| (f as f64 / sr as f64 * 1000.0).round() / 1000.0),
        "sound_end_s": last.map(|f| (f as f64 / sr as f64 * 1000.0).round() / 1000.0),
        "stereo_correlation": corr,
        "onsets": onsets.len(),
        "onset_times_s": onsets.iter().take(64).map(|o| (*o as f64 / sr as f64 * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
        "detected_bpm": a.bpm.map(|b| (b * 10.0).round() / 10.0),
        "bpm_from_name": a.bpm_from_name,
        "key": a.key,
    });
    if let Some((bpm, bpb)) = bpm_bars {
        let bar_len = (sr as f64 * 60.0 / bpm * bpb.max(1) as f64) as usize;
        if bar_len > 0 {
            let bars: Vec<Value> = mono.chunks(bar_len).take(256).map(|c| db(rms(c.iter().copied()))).collect();
            v["rms_db_per_bar"] = json!(bars);
        }
    }
    v
}

pub fn save_audio(d: &SampleData, out: &Path, bits: u32, kbps: u32) -> Result<(), String> {
    if let Some(dir) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "mp3" => {
            let tools = tstudio_fetch::Tools::detect(None, None);
            let ff = tools.ffmpeg.ok_or("MP3 braucht ffmpeg (winget install Gyan.FFmpeg) – oder .wav verwenden")?;
            tstudio_fetch::export_mp3(&ff, d, out, kbps)
        }
        "wav" | "" => {
            let fmt = match bits {
                16 => tstudio_library::WavFormat::Pcm16,
                32 => tstudio_library::WavFormat::Float32,
                _ => tstudio_library::WavFormat::Pcm24,
            };
            tstudio_library::write_wav(out, d, fmt)
        }
        other => Err(format!("Unbekanntes Ausgabeformat .{other} (wav oder mp3)")),
    }
}

pub fn load(path: &Path) -> Result<SampleData, String> {
    tstudio_library::decode_file(path).map_err(|e| format!("{}: {e}", path.display()))
}
