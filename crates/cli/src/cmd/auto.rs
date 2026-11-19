//! automation commands

use crate::*;

/// `"0:200,4:8000"` (beat:value) → points in ticks.
pub(crate) fn parse_points(s: &str) -> R<Vec<AutoPoint>> {
    let mut v = Vec::new();
    for part in s.split([',', ' ']).filter(|x| !x.trim().is_empty()) {
        let (b, val) = part.split_once(':').ok_or_else(|| format!("Punkt '{part}' muss BEAT:WERT sein"))?;
        let ticks = spec::parse_beats(b.trim())?;
        let val: f32 = val.trim().parse().map_err(|_| format!("Wert '{val}' ist keine Zahl"))?;
        v.push(AutoPoint { t: ticks, v: val });
    }
    if v.is_empty() {
        return Err("--points ist leer".into());
    }
    v.sort_by_key(|p| p.t);
    Ok(v)
}

pub(crate) fn auto_cmd(c: AutoCmd) -> R<Value> {
    match c {
        AutoCmd::Add { project, target, track, bar, bars, from, to, points, repeat } => {
            let mut p = load(&project)?;
            if track == 0 {
                return Err("--track ist 1-basiert".into());
            }
            let t = p.parse_auto_target(&target)?;
            let bar_t = p.ticks_per_bar();
            let len = (bars * bar_t as f64).round().max(1.0) as i64;
            let start = ((bar - 1.0).max(0.0) * bar_t as f64).round() as i64;
            let (lo, hi, _) = field_range(Project::auto_field_name(&target));
            let base = p.auto_base_value(t);
            let mut pts = match (points, from, to) {
                (Some(s), _, _) => parse_points(&s)?,
                (None, a, b) => vec![AutoPoint { t: 0, v: a.unwrap_or(base) }, AutoPoint { t: len, v: b.unwrap_or(base) }],
            };
            let mut clamped = false;
            for q in pts.iter_mut() {
                let c = q.v.clamp(lo, hi);
                clamped |= c != q.v;
                q.v = c;
                q.t = q.t.clamp(0, len);
            }
            let clip = AutomationClip { target: target.clone(), points: pts };
            let mut ids = Vec::new();
            for i in 0..repeat.max(1) as i64 {
                ids.push(p.add_clip(track - 1, start + i * len, len, ClipKind::Automation(clip.clone())));
            }
            save(&project, &mut p)?;
            let mut out = json!({"ok": true, "clip_ids": ids, "target": target, "range": [lo, hi], "base_value": base,
                "note": "Automation wirkt im Song-Modus; nach dem Clip bleibt der letzte Wert stehen."});
            if clamped {
                out["warning"] = json!(format!("Werte auf den Bereich {lo}..{hi} begrenzt"));
            }
            Ok(out)
        }
        AutoCmd::Targets { project, filter } => {
            let p = load(&project)?;
            let f = filter.map(|s| s.to_lowercase());
            let list: Vec<Value> = p
                .automation_targets()
                .into_iter()
                .filter(|(t, l)| f.as_ref().map(|f| t.to_lowercase().contains(f) || l.to_lowercase().contains(f)).unwrap_or(true))
                .map(|(t, l)| {
                    let (lo, hi, log) = field_range(Project::auto_field_name(&t));
                    json!({"target": t, "label": l, "min": lo, "max": hi, "log": log})
                })
                .collect();
            Ok(json!({"ok": true, "count": list.len(), "targets": list}))
        }
    }
}
