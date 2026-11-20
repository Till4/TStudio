//! arrangement section commands (copy/insert/delete bars …)

use crate::*;

pub(crate) fn parse_tracks(s: &Option<String>, p: &Project) -> R<Option<std::collections::BTreeSet<usize>>> {
    let Some(s) = s else { return Ok(None) };
    let mut set = std::collections::BTreeSet::new();
    for t in s.split([',', ' ']).filter(|x| !x.trim().is_empty()) {
        let n: usize = t.trim().parse().map_err(|_| format!("Spur '{t}' unverständlich"))?;
        set.insert(track_idx(p, n)?);
    }
    Ok(Some(set))
}

/// (start tick, length ticks) of a range; without --bars up to the next marker.
pub(crate) fn range_ticks(p: &Project, r: &Range) -> R<(i64, i64)> {
    let from = tools::bar_tick(p, &r.from)?;
    let len = match r.bars {
        Some(b) if b > 0.0 => p.bars_to_ticks_from(from, b),
        Some(_) => return Err("--bars muss > 0 sein".into()),
        None => {
            let next = p.playlist.markers.iter().map(|m| m.tick).filter(|t| *t > from).min();
            match next {
                Some(n) => n - from,
                None => return Err("--bars fehlt (und es gibt keinen Marker danach)".into()),
            }
        }
    };
    Ok((from, len))
}

pub(crate) fn section_cmd(c: SectionCmd) -> R<Value> {
    match c {
        SectionCmd::Copy { r, to } => {
            let mut p = load(&r.project)?;
            let (from, len) = range_ticks(&p, &r)?;
            let to_t = tools::bar_tick(&p, &to)?;
            let tracks = parse_tracks(&r.tracks, &p)?;
            let n = tools::section_copy(&mut p, from, len, to_t, tracks.as_ref());
            save(&r.project, &mut p)?;
            Ok(json!({"ok": true, "copied_clips": n, "from_bar": tools::tick_bar(&p, from), "bars": tools::tick_bar(&p, from + len) - tools::tick_bar(&p, from), "to_bar": tools::tick_bar(&p, to_t)}))
        }
        SectionCmd::Clear { r } => {
            let mut p = load(&r.project)?;
            let (from, len) = range_ticks(&p, &r)?;
            let tracks = parse_tracks(&r.tracks, &p)?;
            let n = tools::section_clear(&mut p, from, len, tracks.as_ref());
            save(&r.project, &mut p)?;
            Ok(json!({"ok": true, "removed_clips": n}))
        }
        SectionCmd::Insert { project, at, bars } => {
            let mut p = load(&project)?;
            let at_t = tools::bar_tick(&p, &at)?;
            let len = p.bars_to_ticks_from(at_t, bars);
            if len <= 0 {
                return Err("--bars muss > 0 sein".into());
            }
            // clips that span the insert point are split by shortening (the rest keeps its place)
            for c in p.playlist.clips.iter_mut() {
                if c.start < at_t && c.end() > at_t {
                    c.len = at_t - c.start;
                }
            }
            tools::shift_after(&mut p, at_t, len);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "inserted_bars": bars, "at_bar": tools::tick_bar(&p, at_t)}))
        }
        SectionCmd::Delete { r } => {
            let mut p = load(&r.project)?;
            if r.tracks.is_some() {
                return Err("delete wirkt auf alle Spuren (für einzelne Spuren: section clear)".into());
            }
            let (from, len) = range_ticks(&p, &r)?;
            let removed_bars = tools::tick_bar(&p, from + len) - tools::tick_bar(&p, from);
            let n = tools::section_clear(&mut p, from, len, None);
            p.playlist.markers.retain(|m| m.tick < from || m.tick >= from + len);
            tools::shift_after(&mut p, from + len, -len);
            save(&r.project, &mut p)?;
            Ok(json!({"ok": true, "removed_clips": n, "removed_bars": removed_bars}))
        }
    }
}
