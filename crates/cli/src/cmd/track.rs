//! playlist track commands

use crate::*;

pub(crate) fn track_json(p: &Project, t: usize) -> Value {
    let tr = &p.playlist.tracks[t];
    json!({"track": t + 1, "name": tr.name, "muted": tr.muted, "solo": tr.solo, "volume_percent": (tr.volume * 100.0).round(), "audible": p.playlist_track_audible(t),
        "clips": p.playlist.clips.iter().filter(|c| c.track == t).count()})
}

/// Playlist track number from the CLI (1-based) → index.
pub(crate) fn track_idx(p: &Project, track: usize) -> Result<usize, String> {
    if track == 0 || track > p.playlist.tracks.len() {
        return Err(format!("Playlist-Spur {track} gibt es nicht (1..{})", p.playlist.tracks.len()));
    }
    Ok(track - 1)
}

pub(crate) fn track_cmd(cmd: TrackCmd) -> Result<Value, String> {
    match cmd {
        TrackCmd::List { project } => {
            let p = load(&project)?;
            Ok(json!({"ok": true, "tracks": (0..p.playlist.tracks.len()).map(|t| track_json(&p, t)).collect::<Vec<_>>()}))
        }
        TrackCmd::Add { project, name, at } => {
            let mut p = load(&project)?;
            let t = match at {
                Some(a) => p.insert_playlist_track(a.max(1) - 1, name),
                None => p.add_playlist_track(name),
            };
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "added": v, "tracks": p.playlist.tracks.len()}))
        }
        TrackCmd::Delete { project, track } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            let name = p.playlist.tracks[t].name.clone();
            let clips = p.delete_playlist_track(t)?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "deleted": track, "name": name, "clips_deleted": clips, "tracks": p.playlist.tracks.len()}))
        }
        TrackCmd::Move { project, from, to } => {
            let mut p = load(&project)?;
            let (f, t) = (track_idx(&p, from)?, track_idx(&p, to)?);
            p.move_playlist_track(f, t)?;
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "moved": v}))
        }
        TrackCmd::Duplicate { project, track } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            let n = p.duplicate_playlist_track(t)?;
            let v = track_json(&p, n);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "copy": v}))
        }
        TrackCmd::Rename { project, track, name } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            p.playlist.tracks[t].name = name;
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": v}))
        }
        TrackCmd::Volume { project, track, percent } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            p.playlist.tracks[t].volume = (percent / 100.0).clamp(0.0, 1.25);
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": v}))
        }
        TrackCmd::Mute { project, track, off } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            p.playlist.tracks[t].muted = !off;
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": v}))
        }
        TrackCmd::Solo { project, track, off, only, clear } => {
            let mut p = load(&project)?;
            if clear || only {
                for t in p.playlist.tracks.iter_mut() {
                    t.solo = false;
                }
            }
            if !clear {
                let t = track_idx(&p, track)?;
                p.playlist.tracks[t].solo = !off;
            }
            let solos: Vec<usize> = p.playlist.tracks.iter().enumerate().filter(|(_, t)| t.solo).map(|(i, _)| i + 1).collect();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "solo": solos}))
        }
    }
}
