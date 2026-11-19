//! marker commands

use crate::*;

/// f32 → JSON turns 0.8 into 0.800000011920929; round floats to 5 decimals for readability.
pub(crate) fn marker_cmd(c: MarkerCmd) -> R<Value> {
    let list = |p: &Project| -> Vec<Value> { p.playlist.markers.iter().map(|m| json!({"name": m.name, "bar": tools::tick_bar(p, m.tick)})).collect() };
    match c {
        MarkerCmd::Add { project, bar, name } => {
            let mut p = load(&project)?;
            if bar < 1.0 {
                return Err("--bar beginnt bei 1".into());
            }
            let tick = ((bar - 1.0) * p.ticks_per_bar() as f64).round() as i64;
            p.playlist.markers.retain(|m| !m.name.eq_ignore_ascii_case(name.trim()));
            p.playlist.markers.push(tstudio_project::Marker { tick, name: name.trim().to_string() });
            p.playlist.sort_markers();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "markers": list(&p)}))
        }
        MarkerCmd::List { project } => {
            let p = load(&project)?;
            Ok(json!({"markers": list(&p)}))
        }
        MarkerCmd::Remove { project, name } => {
            let mut p = load(&project)?;
            let before = p.playlist.markers.len();
            p.playlist.markers.retain(|m| !m.name.eq_ignore_ascii_case(name.trim()));
            if before == p.playlist.markers.len() {
                return Err(format!("Marker '{name}' gibt es nicht"));
            }
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "markers": list(&p)}))
        }
        MarkerCmd::Clear { project } => {
            let mut p = load(&project)?;
            p.playlist.markers.clear();
            save(&project, &mut p)?;
            Ok(json!({"ok": true}))
        }
    }
}
