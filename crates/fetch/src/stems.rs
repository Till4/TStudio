//! Stem separation (drums / bass / vocals / other) with Demucs, run as an external Python program.
//!
//! Always on the CPU (`-d cpu`), at below-normal process priority and with a limited number of
//! threads, so the machine stays usable and no GPU is ever touched. The user installs Python and
//! Demucs themselves; nothing is downloaded by TStudio (Demucs fetches its model on first use).

use crate::{strip_ansi, FetchEvent};
use crossbeam_channel::{unbounded, Receiver};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use tstudio_dsp::SampleData;
use tstudio_library::{write_wav, WavFormat};

/// Stems in Demucs order with the label used in file names.
pub const STEMS: [(&str, &str); 4] = [("drums", "Drums"), ("bass", "Bass"), ("vocals", "Vocals"), ("other", "Rest")];

/// pip commands shown to the user (CPU-only PyTorch: small, never uses the GPU).
pub const INSTALL_HINT: &str = "winget install Python.Python.3.12\n\
python -m pip install torch==2.5.1 torchaudio==2.5.1 --index-url https://download.pytorch.org/whl/cpu\n\
python -m pip install demucs soundfile";

fn python_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut versioned = |dir: PathBuf| {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase().starts_with("python3")).unwrap_or(false))
                .map(|p| p.join("python.exe"))
                .collect();
            v.sort();
            v.reverse(); // newest first
            out.extend(v);
        }
    };
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        versioned(PathBuf::from(local).join("Programs").join("Python"));
    }
    versioned(PathBuf::from(r"C:\Program Files"));
    versioned(PathBuf::from(r"C:\"));
    out
}

/// The Microsoft-Store stub (`…\WindowsApps\python.exe`) only opens the store: never use it.
fn is_store_stub(p: &Path) -> bool {
    p.to_string_lossy().to_lowercase().contains("windowsapps")
}

/// Python to run Demucs with: the configured path, else PATH, else the usual install folders.
pub fn find_python(override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = override_path.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    let from_path = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).map(|d| d.join(if cfg!(windows) { "python.exe" } else { "python3" })).find(|p| p.is_file() && !is_store_stub(p))
    });
    from_path.or_else(|| python_candidates().into_iter().find(|p| p.is_file()))
}

/// Threads Demucs may use: `requested` (0 = half of the cores, at least 1).
pub fn thread_count(requested: u32) -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    if requested == 0 {
        (cores / 2).max(1)
    } else {
        (requested as usize).clamp(1, cores)
    }
}

/// Checks that Demucs can be imported. Ok(version) or a German error with the install hint.
pub fn check(python: &Path) -> Result<String, String> {
    let out = crate::command(python)
        .args(["-c", "import demucs, soundfile, torch; print(demucs.__version__, torch.__version__)"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Python konnte nicht gestartet werden: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().last().unwrap_or("").trim().to_string();
        Err(format!("Demucs ist nicht installiert ({last})"))
    }
}

pub struct StemHandle {
    pub events: Receiver<FetchEvent>,
    child: Arc<Mutex<Option<Child>>>,
}

impl StemHandle {
    pub fn cancel(&self) {
        if let Ok(mut c) = self.child.lock() {
            if let Some(ch) = c.as_mut() {
                let _ = ch.kill();
            }
        }
    }
}

/// Splits on both '\n' and '\r' (tqdm redraws its bar with '\r').
fn read_lines<R: Read>(mut r: R, mut f: impl FnMut(String)) {
    let mut buf = [0u8; 4096];
    let mut line = Vec::new();
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !line.is_empty() {
                            f(strip_ansi(String::from_utf8_lossy(&line).trim()));
                            line.clear();
                        }
                    } else {
                        line.push(b);
                    }
                }
            }
        }
    }
    if !line.is_empty() {
        f(strip_ansi(String::from_utf8_lossy(&line).trim()));
    }
}

/// tqdm progress (`" 45%|████▌ | 26.0/58.5 [...]"`) as 0..1.
pub fn parse_progress(line: &str) -> Option<f32> {
    let idx = line.find("%|")?;
    let before = line[..idx].trim_end();
    let start = before.rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map(|i| i + 1).unwrap_or(0);
    before[start..].parse::<f32>().ok().filter(|v| (0.0..=100.0).contains(v)).map(|v| v / 100.0)
}

/// Moves Demucs' `<tmp>/htdemucs/<stem>.wav` files to `<dest>/<name> - <Label>.wav`.
pub fn collect_stems(model_dir: &Path, dest: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (stem, label) in STEMS {
        let src = model_dir.join(format!("{stem}.wav"));
        if !src.is_file() {
            continue;
        }
        let dst = dest.join(format!("{name} - {label}.wav"));
        if std::fs::rename(&src, &dst).is_err() {
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&src);
        }
        out.push(dst);
    }
    if out.is_empty() {
        return Err("Demucs hat keine Stems geschrieben".into());
    }
    Ok(out)
}

/// Separates `data` into stems saved under `dest` (`<name> - Drums.wav`, …).
/// Finished(dest) when done. `threads` = CPU threads (see [`thread_count`]).
pub fn start_stems(python: &Path, data: &SampleData, dest: &Path, name: &str, threads: usize) -> Result<StemHandle, String> {
    let work = std::env::temp_dir().join(format!("tstudio-stems-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let input = work.join("input.wav");
    write_wav(&input, data, WavFormat::Pcm24)?;
    let mut cmd = crate::command(python);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    let t = threads.max(1).to_string();
    cmd.args(["-u", "-m", "demucs", "-d", "cpu", "-n", "htdemucs", "-j", "1", "--filename", "{stem}.{ext}", "-o"])
        .arg(&work)
        .arg(&input)
        .env("OMP_NUM_THREADS", &t)
        .env("MKL_NUM_THREADS", &t)
        .env("OPENBLAS_NUM_THREADS", &t)
        .env("CUDA_VISIBLE_DEVICES", "")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Python konnte nicht gestartet werden: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));
    let (tx, rx) = unbounded();
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let pump = |r: Box<dyn Read + Send>, tx: crossbeam_channel::Sender<FetchEvent>, errors: Arc<Mutex<Vec<String>>>| {
        std::thread::spawn(move || {
            read_lines(r, |line| {
                if let Some(p) = parse_progress(&line) {
                    let _ = tx.send(FetchEvent::Progress(p));
                } else {
                    if line.contains("Error") || line.contains("error") {
                        errors.lock().unwrap().push(line.clone());
                    }
                    let _ = tx.send(FetchEvent::Log(line));
                }
            })
        })
    };
    let t1 = stdout.map(|s| pump(Box::new(s), tx.clone(), errors.clone()));
    let t2 = stderr.map(|s| pump(Box::new(s), tx.clone(), errors.clone()));
    let child2 = Arc::clone(&child);
    let (dest, name) = (dest.to_path_buf(), name.to_string());
    std::thread::spawn(move || {
        for t in [t1, t2].into_iter().flatten() {
            let _ = t.join();
        }
        let status = child2.lock().unwrap().take().and_then(|mut c| c.wait().ok());
        let ok = status.map(|s| s.success()).unwrap_or(false);
        let result = if ok {
            collect_stems(&work.join("htdemucs"), &dest, &name)
        } else {
            let errs = errors.lock().unwrap();
            Err(errs.last().cloned().unwrap_or_else(|| "Demucs ist fehlgeschlagen (oder wurde abgebrochen)".into()))
        };
        let _ = std::fs::remove_dir_all(&work);
        let _ = match result {
            Ok(_) => tx.send(FetchEvent::Finished(dest)),
            Err(e) => tx.send(FetchEvent::Failed(e)),
        };
    });
    Ok(StemHandle { events: rx, child })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tqdm_progress() {
        assert_eq!(parse_progress(" 45%|████▌     | 26.0/58.5 [00:20<00:25,  1.27seconds/s]"), Some(0.45));
        assert_eq!(parse_progress("100%|██████████| 58.5/58.5"), Some(1.0));
        assert_eq!(parse_progress("Separated tracks will be stored in x"), None);
    }

    #[test]
    fn splits_carriage_returns() {
        let mut lines = Vec::new();
        read_lines(&b"a\r 10%|x\r 20%|y\nlast"[..], |l| lines.push(l));
        assert_eq!(lines, vec!["a", "10%|x", "20%|y", "last"]);
    }

    #[test]
    fn collects_and_renames_stems() {
        let base = std::env::temp_dir().join(format!("tstudio-stem-test-{}", std::process::id()));
        let model = base.join("htdemucs");
        std::fs::create_dir_all(&model).unwrap();
        for s in ["drums", "bass", "vocals", "other"] {
            std::fs::write(model.join(format!("{s}.wav")), b"x").unwrap();
        }
        let dest = base.join("out");
        let files = collect_stems(&model, &dest, "Song").unwrap();
        let names: Vec<String> = files.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["Song - Drums.wav", "Song - Bass.wav", "Song - Vocals.wav", "Song - Rest.wav"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn half_the_cores_by_default() {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        assert_eq!(thread_count(0), (cores / 2).max(1));
        assert_eq!(thread_count(1), 1);
        assert!(!is_store_stub(Path::new(r"C:\Python312\python.exe")));
        assert!(is_store_stub(Path::new(r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\python.exe")));
    }
}
