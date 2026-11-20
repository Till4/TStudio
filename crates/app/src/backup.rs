//! Auto-backup of unsaved work: file naming, rotation, listing and crash detection.
//!
//! Backups are `<config>/backups/<Projekt> YYYYMMDD-HHMMSS.tsp` (UTC time), each with an optional
//! `<backup>.path` sidecar holding the project file it belongs to. A `session.lock` file exists
//! while TStudio runs; finding it at start means the last session did not end cleanly.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How often unsaved changes are backed up.
pub const INTERVAL: Duration = Duration::from_secs(120);
/// How many backups are kept (oldest are deleted).
pub const KEEP: usize = 10;
const EXT: &str = "tsp";
const SIDECAR: &str = "path";

/// `YYYYMMDD-HHMMSS` (UTC) for seconds since the Unix epoch.
pub fn timestamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

/// Human readable form of a [`timestamp`]: `19.11.2026 14:30:05`.
pub fn pretty_stamp(stamp: &str) -> String {
    let c: Vec<char> = stamp.chars().collect();
    if c.len() != 15 {
        return stamp.to_string();
    }
    let s = |a: usize, b: usize| c[a..b].iter().collect::<String>();
    format!("{}.{}.{} {}:{}:{}", s(6, 8), s(4, 6), s(0, 4), s(9, 11), s(11, 13), s(13, 15))
}

fn clean_name(name: &str) -> String {
    let n: String = name.trim().chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let n = n.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if n.is_empty() {
        "Untitled".into()
    } else {
        n
    }
}

/// `<name> <timestamp>.tsp`
pub fn file_name(project_name: &str, unix_secs: u64) -> String {
    format!("{} {}.{EXT}", clean_name(project_name), timestamp(unix_secs))
}

/// Splits a backup file stem into (project name, timestamp).
fn parse_stem(stem: &str) -> Option<(String, String)> {
    let (name, stamp) = stem.rsplit_once(' ')?;
    let ok = stamp.len() == 15 && stamp.as_bytes()[8] == b'-' && stamp.chars().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit());
    ok.then(|| (name.to_string(), stamp.to_string()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    pub path: PathBuf,
    pub name: String,
    pub stamp: String,
    /// project file the backup belongs to (None = never saved)
    pub project_path: Option<PathBuf>,
}

/// All backups in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Backup> {
    let mut v: Vec<Backup> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == EXT).unwrap_or(false))
                .filter_map(|path| {
                    let (name, stamp) = parse_stem(&path.file_stem()?.to_string_lossy())?;
                    let project_path = std::fs::read_to_string(sidecar(&path)).ok().map(|s| PathBuf::from(s.trim())).filter(|p| !p.as_os_str().is_empty());
                    Some(Backup { path, name, stamp, project_path })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.stamp.cmp(&a.stamp).then_with(|| a.name.cmp(&b.name)));
    v
}

fn sidecar(backup: &Path) -> PathBuf {
    let mut s = backup.as_os_str().to_owned();
    s.push(".");
    s.push(SIDECAR);
    PathBuf::from(s)
}

/// Deletes all but the newest `keep` backups; returns how many were removed.
pub fn rotate(dir: &Path, keep: usize) -> usize {
    let mut n = 0;
    for b in list(dir).into_iter().skip(keep) {
        if std::fs::remove_file(&b.path).is_ok() {
            n += 1;
        }
        let _ = std::fs::remove_file(sidecar(&b.path));
    }
    n
}

/// Writes a backup (project as RON text) and rotates; returns the new file.
pub fn write(dir: &Path, project_name: &str, ron: &str, project_path: Option<&Path>, unix_secs: u64, keep: usize) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(file_name(project_name, unix_secs));
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, ron).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    match project_path {
        Some(p) => std::fs::write(sidecar(&path), p.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?,
        None => {
            let _ = std::fs::remove_file(sidecar(&path));
        }
    }
    rotate(dir, keep);
    Ok(path)
}

/// A backup is due when the project has unsaved changes that differ from the last backup and
/// the interval has passed.
pub fn due(dirty: bool, changed_since_backup: bool, elapsed: Duration, interval: Duration) -> bool {
    dirty && changed_since_backup && elapsed >= interval
}

/// Marks a running session; returns true if the previous one did not end cleanly.
pub fn start_session(lock: &Path) -> bool {
    let crashed = lock.exists();
    let _ = std::fs::write(lock, std::process::id().to_string());
    crashed
}

/// Clean exit: removes the session marker.
pub fn end_session(lock: &Path) {
    let _ = std::fs::remove_file(lock);
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tstudio_backup_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(0), "19700101-000000");
        // 2026-11-19 14:30:05 UTC
        assert_eq!(timestamp(1_795_098_605), "20261119-143005");
        // leap day
        assert_eq!(timestamp(951_782_400), "20000229-000000");
        assert_eq!(pretty_stamp("20261119-143005"), "19.11.2026 14:30:05");
        assert_eq!(file_name("My: Song", 0), "My_ Song 19700101-000000.tsp");
        assert_eq!(file_name("  ", 0), "Untitled 19700101-000000.tsp");
        assert_eq!(parse_stem("My Song 20261119-143005"), Some(("My Song".into(), "20261119-143005".into())));
        assert_eq!(parse_stem("My Song"), None);
        assert_eq!(parse_stem("x 2026111a-143005"), None);
    }

    #[test]
    fn write_list_rotate() {
        let dir = tmp("rot");
        for i in 0..5u64 {
            let orig = (i % 2 == 0).then(|| PathBuf::from(format!("C:/songs/s{i}.tsp")));
            write(&dir, &format!("Song{i}"), "()", orig.as_deref(), 1_000_000 + i * 60, 3).unwrap();
        }
        let l = list(&dir);
        assert_eq!(l.len(), 3, "rotation keeps 3");
        assert_eq!(l.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), vec!["Song4", "Song3", "Song2"]);
        assert_eq!(l[0].project_path, Some(PathBuf::from("C:/songs/s4.tsp")));
        assert_eq!(l[1].project_path, None);
        // sidecars of rotated backups are gone too
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(files, 3 + 2);
        assert_eq!(rotate(&dir, 1), 2);
        assert_eq!(list(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn due_and_session_lock() {
        let s = Duration::from_secs;
        assert!(due(true, true, s(120), INTERVAL));
        assert!(!due(true, true, s(60), INTERVAL));
        assert!(!due(false, true, s(500), INTERVAL));
        assert!(!due(true, false, s(500), INTERVAL));
        let dir = tmp("lock");
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join("session.lock");
        assert!(!start_session(&lock), "first start is clean");
        assert!(start_session(&lock), "lock left behind = crash");
        end_session(&lock);
        assert!(!start_session(&lock));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
