//! time-signature commands

use crate::*;

/// "7/8" → (7, 8); the denominator must be 1, 2, 4, 8, 16 or 32.
pub(crate) fn parse_meter(s: &str) -> R<(u32, u32)> {
    let (n, d) = s.trim().split_once('/').ok_or_else(|| format!("Taktart '{s}' muss ZÄHLER/NENNER sein (z.B. 7/8)"))?;
    let n: u32 = n.trim().parse().map_err(|_| format!("Zähler '{n}' ist keine Zahl"))?;
    let d: u32 = d.trim().parse().map_err(|_| format!("Nenner '{d}' ist keine Zahl"))?;
    if !(1..=64).contains(&n) {
        return Err(format!("Zähler {n} außerhalb 1..64"));
    }
    if ![1, 2, 4, 8, 16, 32].contains(&d) {
        return Err(format!("Nenner {d} muss 1, 2, 4, 8, 16 oder 32 sein"));
    }
    Ok((n, d))
}

pub(crate) fn meter_json(p: &Project) -> Value {
    let m = p.bar_map();
    json!({
        "base": format!("{}/4", p.beats_per_bar),
        "changes": p.meters.iter().map(|c| json!({
            "bar": c.bar + 1,
            "meter": c.label(),
            "beat": ticks_to_beats(m.bar_to_tick(c.bar as f64).round() as i64),
        })).collect::<Vec<_>>(),
        "song_length_bars": p.tick_to_bar_number(p.song_length()) - 1.0,
    })
}

pub(crate) fn meter_cmd(c: MeterCmd) -> R<Value> {
    match c {
        MeterCmd::Add { project, bar, meter } => {
            let mut p = load(&project)?;
            if bar == 0 {
                return Err("--bar beginnt bei 1".into());
            }
            let (n, d) = parse_meter(&meter)?;
            p.set_meter(bar - 1, n, d);
            save(&project, &mut p)?;
            let mut out = meter_json(&p);
            out["ok"] = json!(true);
            out["note"] = json!("Clips bleiben an ihrer Beat-Position; Takte danach verschieben sich entsprechend.");
            Ok(out)
        }
        MeterCmd::List { project } => Ok(meter_json(&load(&project)?)),
        MeterCmd::Remove { project, bar, all } => {
            let mut p = load(&project)?;
            if all {
                p.meters.clear();
            } else {
                let b = bar.unwrap_or(0).saturating_sub(1);
                let before = p.meters.len();
                p.meters.retain(|m| m.bar != b);
                if before == p.meters.len() {
                    let have: Vec<String> = p.meters.iter().map(|m| format!("{}", m.bar + 1)).collect();
                    return Err(format!("kein Taktartwechsel bei Takt {} (vorhanden: {})", b + 1, if have.is_empty() { "keine".into() } else { have.join(", ") }));
                }
            }
            save(&project, &mut p)?;
            let mut out = meter_json(&p);
            out["ok"] = json!(true);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meters() {
        assert_eq!(parse_meter("7/8"), Ok((7, 8)));
        assert_eq!(parse_meter(" 3 / 4 "), Ok((3, 4)));
        assert!(parse_meter("7/9").is_err());
        assert!(parse_meter("7").is_err());
    }

    #[test]
    fn bar_selectors_follow_meter_changes() {
        let mut p = Project::default();
        p.set_meter(2, 7, 8); // bar 3 = 7/8
        p.set_meter(3, 4, 4); // bar 4 = 4/4 again
        assert_eq!(tools::bar_tick(&p, "3").unwrap(), 768);
        assert_eq!(tools::bar_tick(&p, "4").unwrap(), 768 + 336);
        assert_eq!(tools::tick_bar(&p, 768 + 336), 4.0);
        // inserting a bar before the 7/8 bar moves it to bar 4
        tools::shift_after(&mut p, 384, 384);
        assert_eq!(p.meters.iter().map(|m| (m.bar, m.beats_per_bar)).collect::<Vec<_>>(), vec![(3, 7), (4, 4)]);
    }
}
