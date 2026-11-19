//! pattern commands

use crate::*;

pub(crate) fn pattern_cmd(c: PatternCmd) -> R<Value> {
    match c {
        PatternCmd::Add { project, name, bars } => {
            let mut p = load(&project)?;
            let id = p.add_pattern();
            let bar = p.ticks_per_bar();
            let pat = p.pattern_mut(id).unwrap();
            if let Some(n) = name {
                pat.name = n;
            }
            pat.length = bars.map(|b| (b * bar as f64).round() as i64);
            let idx = p.patterns.len() - 1;
            let name = p.patterns[idx].name.clone();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "pattern": idx, "name": name}))
        }
        PatternCmd::Remove { project, pattern } => {
            let mut p = load(&project)?;
            let id = pattern_id(&p, &pattern)?;
            if p.patterns.len() <= 1 {
                return Err("das letzte Pattern kann nicht gelöscht werden".into());
            }
            p.remove_pattern(id);
            save(&project, &mut p)?;
            Ok(json!({"ok": true}))
        }
    }
}
