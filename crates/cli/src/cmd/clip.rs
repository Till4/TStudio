//! playlist clip commands

use crate::*;

pub(crate) fn clip_cmd(c: ClipCmd) -> R<Value> {
    match c {
        ClipCmd::Add { project, track, bar, pattern, audio, bars, repeat, gain, stretch } => {
            let mut p = load(&project)?;
            if track == 0 {
                return Err("--track ist 1-basiert (1 = erste Spur)".into());
            }
            let start = p.bar_number_to_tick(bar.max(1.0));
            let mut ids = Vec::new();
            let mut extra = json!({});
            match (pattern, audio) {
                (Some(sel), None) => {
                    let id = pattern_id(&p, &sel)?;
                    let len = bars.map(|b| p.bars_to_ticks_from(start, b)).unwrap_or_else(|| p.pattern_length(id));
                    for i in 0..repeat.max(1) as i64 {
                        ids.push(p.add_clip(track - 1, start + i * len, len, ClipKind::Pattern { pattern: id }));
                    }
                }
                (None, Some(path)) => {
                    let path = abs(&path);
                    let data = audio::load(&path)?;
                    let (st, detected) = stretch_for(&path, &stretch, Some(&data))?;
                    extra["detected_bpm"] = json!(detected);
                    // length of the (tempo-fitted) audio in ticks
                    let secs = match st.mode {
                        StretchMode::Off => data.duration_secs(),
                        _ => data.duration_secs() * st.source_bpm / p.bpm_at(start),
                    };
                    // clips are fitted to the tempo at their start (tempo track)
                    let natural = (secs * p.bpm_at(start) / 60.0 * PPQ as f64).round() as i64;
                    let len = bars.map(|b| p.bars_to_ticks_from(start, b)).unwrap_or(natural).max(1);
                    let clip = AudioClip { path, gain, stretch: st, ..Default::default() };
                    for i in 0..repeat.max(1) as i64 {
                        ids.push(p.add_clip(track - 1, start + i * len, len, ClipKind::Audio(clip.clone())));
                    }
                    extra["length_bars"] = json!(tools::tick_bar(&p, start + len) - tools::tick_bar(&p, start));
                }
                _ => return Err("genau eins von --pattern oder --audio angeben".into()),
            }
            let song = tools::tick_bar(&p, p.song_length()) - 1.0;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "clip_ids": ids, "song_length_bars": song, "extra": extra}))
        }
        ClipCmd::Remove { project, ids, all } => {
            let mut p = load(&project)?;
            let before = p.playlist.clips.len();
            if all {
                p.playlist.clips.clear();
            } else {
                if ids.is_empty() {
                    return Err("Clip-IDs angeben (siehe `info`) oder --all".into());
                }
                p.playlist.clips.retain(|c| !ids.contains(&c.id));
            }
            let removed = before - p.playlist.clips.len();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "removed": removed}))
        }
    }
}
