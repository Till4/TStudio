//! External tool integration: yt-dlp (URL downloads) and ffmpeg (MP3 encode, lossless cuts).
//! Tools are never downloaded automatically; the user installs them (e.g. via winget).

pub mod stems;
pub mod video;

use crossbeam_channel::{unbounded, Receiver, Sender};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tstudio_dsp::SampleData;
use tstudio_library::{write_wav, WavFormat};

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

pub(crate) fn command(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    no_window(&mut c);
    c
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tools {
    pub ffmpeg: Option<PathBuf>,
    pub ytdlp: Option<PathBuf>,
}

impl Tools {
    /// Uses the configured paths if they exist, otherwise searches PATH and common install folders.
    pub fn detect(ffmpeg_override: Option<&Path>, ytdlp_override: Option<&Path>) -> Tools {
        let pick = |ov: Option<&Path>, name: &str| ov.filter(|p| p.is_file()).map(Path::to_path_buf).or_else(|| find_executable(name));
        Tools { ffmpeg: pick(ffmpeg_override, "ffmpeg"), ytdlp: pick(ytdlp_override, "yt-dlp") }
    }
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Searches PATH plus the usual winget / scoop / chocolatey locations.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let file = exe_name(name);
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        dirs.push(local.join("Microsoft").join("WinGet").join("Links"));
        dirs.push(local.join("Programs").join(name));
        // winget package folders, e.g. Gyan.FFmpeg_.../ffmpeg-7.1-full_build/bin
        if let Ok(rd) = std::fs::read_dir(local.join("Microsoft").join("WinGet").join("Packages")) {
            for pkg in rd.flatten() {
                let p = pkg.path();
                dirs.push(p.clone());
                if let Ok(inner) = std::fs::read_dir(&p) {
                    for i in inner.flatten() {
                        dirs.push(i.path().join("bin"));
                        dirs.push(i.path());
                    }
                }
            }
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(home).join("scoop").join("shims"));
    }
    dirs.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin"));
    if let Some(p) = dirs.into_iter().map(|d| d.join(&file)).find(|p| p.is_file()) {
        return Some(p);
    }
    drive_tool_folders(name).into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// Unpacked release zips on any drive, e.g. `E:\ffmpeg\ffmpeg-master-latest-win64-gpl\…\bin`
/// or `D:\Tools\yt-dlp`: searches `X:\NAME`, `X:\Tools\NAME`, `X:\Program Files\NAME` up to 3 levels deep.
fn drive_tool_folders(name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !cfg!(windows) {
        return out;
    }
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        out.push(dir.to_path_buf());
        out.push(dir.join("bin"));
        if depth == 0 {
            return;
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten().take(64) {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    walk(&e.path(), depth - 1, out);
                }
            }
        }
    }
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        for base in [root.join(name), root.join("Tools").join(name), root.join("Program Files").join(name)] {
            if base.is_dir() {
                walk(&base, 3, &mut out);
            }
        }
    }
    out
}

/// First line of `--version` / `-version`.
pub fn tool_version(exe: &Path) -> Option<String> {
    let is_ffmpeg = exe.file_stem().map(|s| s.to_string_lossy().to_lowercase().contains("ffmpeg")).unwrap_or(false);
    let arg = if is_ffmpeg { "-version" } else { "--version" };
    let out = command(exe).arg(arg).stdin(Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().next().map(|l| l.trim().chars().take(80).collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Mp3,
}

impl AudioFormat {
    pub fn ext(self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
        }
    }
}

#[derive(Clone, Debug)]
pub enum FetchEvent {
    Progress(f32),
    Log(String),
    Finished(PathBuf),
    Failed(String),
}

pub struct DownloadHandle {
    pub events: Receiver<FetchEvent>,
    child: Arc<Mutex<Option<Child>>>,
}

impl DownloadHandle {
    pub fn cancel(&self) {
        if let Ok(mut c) = self.child.lock() {
            if let Some(ch) = c.as_mut() {
                let _ = ch.kill();
            }
        }
    }
}

pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // skip CSI sequence
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Extracts "42.1" from lines like `[download]  42.1% of 3.2MiB at ...`.
pub fn parse_percent(line: &str) -> Option<f32> {
    let line = strip_ansi(line);
    if !line.contains('%') {
        return None;
    }
    let idx = line.find('%')?;
    let before = &line[..idx];
    let start = before.rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map(|i| i + 1).unwrap_or(0);
    before[start..].parse::<f32>().ok().filter(|v| (0.0..=100.0).contains(v))
}

fn newest_file_since(dir: &Path, since: SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let t = m.modified().ok()?;
            (m.is_file() && t >= since && tstudio_library::is_audio_file(&e.path())).then_some((t, e.path()))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// Downloads the audio of `url` into `out_dir` via yt-dlp (+ ffmpeg for conversion).
pub fn start_download(tools: &Tools, url: &str, out_dir: &Path, format: AudioFormat) -> Result<DownloadHandle, String> {
    let ytdlp = tools.ytdlp.clone().ok_or("yt-dlp wurde nicht gefunden")?;
    let ffmpeg = tools.ffmpeg.clone().ok_or("ffmpeg wurde nicht gefunden (yt-dlp braucht es zum Konvertieren)")?;
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Bitte eine gültige URL (http/https) einfügen".into());
    }
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let template = out_dir.join("%(title).120B.%(ext)s");

    let mut cmd = command(&ytdlp);
    cmd.args(["--no-playlist", "--extract-audio", "--audio-format", format.ext(), "--audio-quality", "0", "--newline", "--progress", "--encoding", "utf-8"])
        .arg("--ffmpeg-location")
        .arg(&ffmpeg)
        .arg("-o")
        .arg(&template)
        .args(["--print", "after_move:filepath"])
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = SystemTime::now();
    let mut child = cmd.spawn().map_err(|e| format!("yt-dlp konnte nicht gestartet werden: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));
    let (tx, rx) = unbounded();

    let final_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    fn pump<R: Read + Send + 'static>(r: R, tx: Sender<FetchEvent>, final_path: Arc<Mutex<Option<PathBuf>>>, errors: Arc<Mutex<Vec<String>>>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(r);
            let mut buf = Vec::new();
            while let Ok(n) = reader.read_until(b'\n', &mut buf) {
                if n == 0 {
                    break;
                }
                let line = strip_ansi(String::from_utf8_lossy(&buf).trim_end());
                buf.clear();
                if line.is_empty() {
                    continue;
                }
                if let Some(p) = parse_percent(&line) {
                    let _ = tx.send(FetchEvent::Progress(p / 100.0));
                    continue;
                }
                let candidate = PathBuf::from(line.trim());
                if candidate.is_absolute() && candidate.is_file() {
                    *final_path.lock().unwrap() = Some(candidate);
                    continue;
                }
                if line.contains("ERROR") {
                    errors.lock().unwrap().push(line.clone());
                }
                let _ = tx.send(FetchEvent::Log(line));
            }
        })
    }

    let t1 = stdout.map(|s| pump(s, tx.clone(), final_path.clone(), errors.clone()));
    let t2 = stderr.map(|s| pump(s, tx.clone(), final_path.clone(), errors.clone()));
    let child2 = Arc::clone(&child);
    let out_dir = out_dir.to_path_buf();
    std::thread::spawn(move || {
        if let Some(t) = t1 {
            let _ = t.join();
        }
        if let Some(t) = t2 {
            let _ = t.join();
        }
        let status = child2.lock().unwrap().take().and_then(|mut c| c.wait().ok());
        let ok = status.map(|s| s.success()).unwrap_or(false);
        let path = final_path.lock().unwrap().clone().or_else(|| if ok { newest_file_since(&out_dir, started) } else { None });
        match (ok, path) {
            (true, Some(p)) => {
                let _ = tx.send(FetchEvent::Progress(1.0));
                let _ = tx.send(FetchEvent::Finished(p));
            }
            (true, None) => {
                let _ = tx.send(FetchEvent::Failed("Download fertig, aber die Datei wurde nicht gefunden".into()));
            }
            (false, _) => {
                let errs = errors.lock().unwrap();
                let msg = errs.last().cloned().unwrap_or_else(|| "yt-dlp ist fehlgeschlagen (oder wurde abgebrochen)".into());
                let _ = tx.send(FetchEvent::Failed(msg));
            }
        }
    });
    Ok(DownloadHandle { events: rx, child })
}

fn run_ffmpeg(ffmpeg: &Path, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let out = command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg konnte nicht gestartet werden: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ffmpeg-Fehler").to_string())
    }
}

/// Encodes audio to MP3 (via a temporary WAV) with the given bitrate.
pub fn export_mp3(ffmpeg: &Path, data: &SampleData, out: &Path, kbps: u32) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!("tstudio-export-{}-{}.wav", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
    write_wav(&tmp, data, WavFormat::Float32)?;
    let br = format!("{kbps}k");
    let r = run_ffmpeg(ffmpeg, &["-i".as_ref(), tmp.as_os_str(), "-codec:a".as_ref(), "libmp3lame".as_ref(), "-b:a".as_ref(), br.as_ref(), out.as_os_str()]);
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Cuts without re-encoding (fast, frame-accurate for MP3 — about ±26 ms).
pub fn cut_lossless(ffmpeg: &Path, input: &Path, out: &Path, start_secs: f64, end_secs: f64) -> Result<(), String> {
    let ss = format!("{start_secs:.3}");
    let to = format!("{end_secs:.3}");
    run_ffmpeg(ffmpeg, &["-ss".as_ref(), ss.as_ref(), "-to".as_ref(), to.as_ref(), "-i".as_ref(), input.as_os_str(), "-c".as_ref(), "copy".as_ref(), out.as_os_str()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_percent("[download]  42.1% of 3.20MiB at 1.2MiB/s ETA 00:02"), Some(42.1));
        assert_eq!(parse_percent("\u{1b}[0;94m[download]\u{1b}[0m 100% of 3MiB"), Some(100.0));
        assert_eq!(parse_percent("[ExtractAudio] Destination: x.mp3"), None);
    }

    #[test]
    fn rejects_bad_urls() {
        let t = Tools { ffmpeg: Some("ffmpeg".into()), ytdlp: Some("yt-dlp".into()) };
        assert!(start_download(&t, "not a url", &std::env::temp_dir(), AudioFormat::Mp3).is_err());
        let none = Tools::default();
        assert!(start_download(&none, "https://example.com", &std::env::temp_dir(), AudioFormat::Mp3).is_err());
    }
}
