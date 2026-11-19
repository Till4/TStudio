//! Live session: lets `tstudio-cli` edit the project that is open in this window.
//!
//! The GUI listens on 127.0.0.1 (random port) and writes `{port, token, pid}` to
//! `<config>/live.json`. Each connection carries one JSON line request and gets one JSON
//! line reply. Requests: `status`, `get` (current project as RON, including unsaved
//! changes), `put` (replace the project; applied as one undo step), `open`, `save`.
//!
//! The server thread never touches the UI state directly: it keeps the latest project
//! snapshot for `get`, and queues actions that the UI thread applies in `TStudio::live_sync`.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tstudio_project::Project;

pub type Reply = mpsc::Sender<Result<Value, String>>;

pub enum Action {
    /// Replace the open project (one undo step); `label` names the command for the status bar.
    Put { project: Project, label: String },
    Open { path: PathBuf, reply: Reply },
    Save { path: Option<PathBuf>, reply: Reply },
}

#[derive(Default)]
pub struct Shared {
    /// What the CLI sees: the GUI project, or a put that the UI has not applied yet.
    pub project_ron: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub name: String,
    pub actions: VecDeque<Action>,
}

pub struct LiveServer {
    pub shared: Arc<Mutex<Shared>>,
    session_file: PathBuf,
    /// Last project published to `shared`, to skip re-serializing unchanged state.
    pub last_published: Option<Project>,
    pub last_path: Option<PathBuf>,
    info: String,
    last_check: std::time::Instant,
}

fn random_token() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut a = std::collections::hash_map::RandomState::new().build_hasher();
    a.write_u128(nanos);
    a.write_u32(std::process::id());
    let b = std::collections::hash_map::RandomState::new().build_hasher();
    format!("{:016x}{:016x}", a.finish(), b.finish())
}

impl LiveServer {
    pub fn start(config_dir: &std::path::Path, ctx: egui::Context) -> Option<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        let token = random_token();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let session_file = config_dir.join("live.json");
        let info = json!({"port": port, "token": token, "pid": std::process::id()});
        let info = info.to_string();
        std::fs::write(&session_file, &info).ok()?;

        let sh = shared.clone();
        std::thread::Builder::new()
            .name("tstudio-live".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (sh, ctx, token) = (sh.clone(), ctx.clone(), token.clone());
                    std::thread::spawn(move || handle(stream, &sh, &ctx, &token));
                }
            })
            .ok()?;
        Some(Self { shared, session_file, last_published: None, last_path: None, info, last_check: std::time::Instant::now() })
    }

    /// Re-registers this window if the session file vanished (e.g. a second TStudio
    /// window took it over and was closed again).
    pub fn ensure_session_file(&mut self) {
        if self.last_check.elapsed() > Duration::from_secs(2) {
            self.last_check = std::time::Instant::now();
            if !self.session_file.exists() {
                let _ = std::fs::write(&self.session_file, &self.info);
            }
        }
    }

    /// Removes the session file if it still belongs to this process.
    pub fn shutdown(&self) {
        if let Ok(s) = std::fs::read_to_string(&self.session_file) {
            let mine = serde_json::from_str::<Value>(&s).ok().and_then(|v| v["pid"].as_u64()) == Some(std::process::id() as u64);
            if mine {
                let _ = std::fs::remove_file(&self.session_file);
            }
        }
    }
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn handle(stream: TcpStream, shared: &Arc<Mutex<Shared>>, ctx: &egui::Context, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Value>(&line) {
        Ok(req) if req["token"].as_str() == Some(token) => respond(&req, shared, ctx),
        Ok(_) => json!({"ok": false, "error": "ungültiges Token"}),
        Err(e) => json!({"ok": false, "error": format!("ungültige Anfrage: {e}")}),
    };
    let _ = writeln!(writer, "{reply}");
}

/// Waits for the UI thread to finish an action; a hidden/minimized window may not
/// run its update loop, so after a while the action just stays queued.
fn wait(rx: mpsc::Receiver<Result<Value, String>>) -> Value {
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => json!({"ok": false, "error": e}),
        Err(_) => json!({"ok": true, "queued": true, "note": "TStudio führt das aus, sobald das Fenster aktiv ist"}),
    }
}

fn respond(req: &Value, shared: &Arc<Mutex<Shared>>, ctx: &egui::Context) -> Value {
    let op = req["op"].as_str().unwrap_or("");
    match op {
        "status" | "get" => {
            let s = shared.lock().unwrap();
            let mut v = json!({
                "ok": true,
                "pid": std::process::id(),
                "path": s.path,
                "name": s.name,
                "dirty": s.dirty,
                "pending": s.actions.len(),
            });
            if op == "get" {
                v["project_ron"] = json!(s.project_ron);
            }
            v
        }
        "put" => {
            let Some(ron) = req["project_ron"].as_str() else {
                return json!({"ok": false, "error": "project_ron fehlt"});
            };
            let project = match Project::from_ron(ron) {
                Ok(p) => p,
                Err(e) => return json!({"ok": false, "error": format!("Projekt ungültig: {e}")}),
            };
            let label = req["label"].as_str().unwrap_or("CLI").to_string();
            {
                let mut s = shared.lock().unwrap();
                s.project_ron = project.to_ron().unwrap_or_else(|_| ron.to_string());
                s.name = project.name.clone();
                s.dirty = true;
                s.actions.push_back(Action::Put { project, label });
            }
            ctx.request_repaint();
            json!({"ok": true})
        }
        "open" | "save" => {
            let path = req["path"].as_str().map(PathBuf::from);
            if op == "open" && path.is_none() {
                return json!({"ok": false, "error": "path fehlt"});
            }
            let (tx, rx) = mpsc::channel();
            {
                let mut s = shared.lock().unwrap();
                s.actions.push_back(match (op, path) {
                    ("open", Some(path)) => Action::Open { path, reply: tx },
                    (_, path) => Action::Save { path, reply: tx },
                });
            }
            ctx.request_repaint();
            wait(rx)
        }
        _ => json!({"ok": false, "error": format!("unbekannte Operation '{op}'")}),
    }
}
