//! tempo track commands

use crate::*;

/// Start tempo, every tempo point and the resulting song length in seconds.
pub(crate) fn tempo_json(p: &Project) -> Value {
    let map = p.tempo_map();
    json!({
        "start_bpm": p.bpm,
        "points": p.tempo.iter().map(|t| json!({
            "bar": tools::tick_bar(p, t.tick),
            "bpm": t.bpm,
            "ramp": t.ramp,
            "time_s": (map.secs_at(t.tick as f64) * 1000.0).round() / 1000.0,
        })).collect::<Vec<_>>(),
        "song_length_s": (map.secs_at(p.song_length() as f64) * 1000.0).round() / 1000.0,
    })
}

pub(crate) fn tempo_cmd(c: TempoCmd) -> R<Value> {
    match c {
        TempoCmd::Add { project, bar, bpm, ramp } => {
            let mut p = load(&project)?;
            if !(tstudio_project::tempo::MIN_BPM..=tstudio_project::tempo::MAX_BPM).contains(&bpm) {
                return Err(format!("--bpm {bpm} außerhalb {}..{}", tstudio_project::tempo::MIN_BPM, tstudio_project::tempo::MAX_BPM));
            }
            let tick = tools::bar_tick(&p, &bar)?;
            if tick == 0 && ramp {
                return Err("Takt 1 ist das Starttempo – eine Rampe braucht einen späteren Takt".into());
            }
            p.set_tempo_point(tick, bpm, ramp);
            save(&project, &mut p)?;
            let mut out = tempo_json(&p);
            out["ok"] = json!(true);
            Ok(out)
        }
        TempoCmd::List { project } => Ok(tempo_json(&load(&project)?)),
        TempoCmd::Remove { project, bar, all } => {
            let mut p = load(&project)?;
            if all {
                p.tempo.clear();
            } else {
                let tick = tools::bar_tick(&p, bar.as_deref().unwrap_or_default())?;
                let before = p.tempo.len();
                p.tempo.retain(|t| t.tick != tick);
                if before == p.tempo.len() {
                    let have: Vec<String> = p.tempo.iter().map(|t| format!("{}", tools::tick_bar(&p, t.tick))).collect();
                    return Err(format!("kein Tempowechsel bei Takt {} (vorhanden: {})", tools::tick_bar(&p, tick), if have.is_empty() { "keine".into() } else { have.join(", ") }));
                }
            }
            save(&project, &mut p)?;
            let mut out = tempo_json(&p);
            out["ok"] = json!(true);
            Ok(out)
        }
    }
}
