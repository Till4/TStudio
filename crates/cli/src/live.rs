//! Client for the live session of a running TStudio window (server: crates/app/src/live.rs).
//!
//! `live` (or `@`) as project argument means "the project open in TStudio right now",
//! including unsaved changes. Edits are sent back to the window, where each command
//! becomes one undo step. A path that points to the file open in TStudio is routed to the
//! window as well, so the GUI and the CLI never edit different copies.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

type R<T> = Result<T, String>;

pub fn is_keyword(p: &Path) -> bool {
    matches!(p.to_str(), Some("live") | Some("@"))
}

fn session_file() -> Option<PathBuf> {
    directories::ProjectDirs::from("de", "TStudio", "TStudio").map(|d| d.config_dir().join("live.json"))
}

const NOT_RUNNING: &str = "TStudio ist nicht geöffnet – starte die App (oder `tstudio-cli open <datei.tsp>`), dann wirkt `live` auf das offene Projekt";

pub fn request(op: &str, mut body: Value) -> R<Value> {
    let file = session_file().ok_or(NOT_RUNNING)?;
    let s = std::fs::read_to_string(&file).map_err(|_| NOT_RUNNING.to_string())?;
    let info: Value = serde_json::from_str(&s).map_err(|_| NOT_RUNNING.to_string())?;
    let port = info["port"].as_u64().ok_or(NOT_RUNNING)? as u16;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| NOT_RUNNING.to_string())?;
    // `open` may wait for the user to answer a "save changes?" dialog
    let _ = stream.set_read_timeout(Some(Duration::from_secs(if op == "open" || op == "save" { 120 } else { 30 })));
    body["op"] = json!(op);
    body["token"] = info["token"].clone();
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    writeln!(w, "{body}").map_err(|e| format!("Verbindung zu TStudio fehlgeschlagen: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| format!("Keine Antwort von TStudio: {e}"))?;
    let v: Value = serde_json::from_str(&line).map_err(|e| format!("Unverständliche Antwort von TStudio: {e}"))?;
    if v["ok"].as_bool() == Some(true) {
        Ok(v)
    } else {
        Err(v["error"].as_str().unwrap_or("TStudio hat die Anfrage abgelehnt").to_string())
    }
}

/// Status of the running window, fetched once per CLI call (None = no TStudio open).
pub fn status() -> Option<&'static Value> {
    static S: OnceLock<Option<Value>> = OnceLock::new();
    S.get_or_init(|| request("status", json!({})).ok()).as_ref()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

static NO_LIVE: AtomicBool = AtomicBool::new(false);
static ROUTED: AtomicBool = AtomicBool::new(false);

/// `--no-live`: never write into the window by file path.
pub fn set_no_live(v: bool) {
    NO_LIVE.store(v, Ordering::Relaxed);
}

/// True if a file path (not the `live` keyword) was redirected to the window during this call.
pub fn routed_path() -> bool {
    ROUTED.load(Ordering::Relaxed)
}

fn is_open_in_window(p: &Path) -> bool {
    p.exists() && status().and_then(|s| s["path"].as_str()).map(|open| same_file(Path::new(open), p)).unwrap_or(false)
}

/// With `--no-live`, refuses files that are open in TStudio (writing them would be overwritten
/// by the window's next save, and the window would not see the change).
pub fn guard(p: &Path) -> R<()> {
    if NO_LIVE.load(Ordering::Relaxed) && !is_keyword(p) && is_open_in_window(p) {
        return Err(format!(
            "{} ist in TStudio offen – mit --no-live wird sie nicht angefasst. Kopie bearbeiten (anderer Dateiname) oder ohne --no-live ins Fenster schreiben.",
            p.display()
        ));
    }
    Ok(())
}

/// Should this project argument go to the open TStudio window?
pub fn targets_window(p: &Path) -> bool {
    if is_keyword(p) {
        return true;
    }
    if NO_LIVE.load(Ordering::Relaxed) {
        return false;
    }
    let open = is_open_in_window(p);
    if open {
        ROUTED.store(true, Ordering::Relaxed);
    }
    open
}
