//! effect-chain commands

use crate::*;

pub(crate) fn fx_cmd(c: FxCmd) -> R<Value> {
    match c {
        FxCmd::Add { project, track, kind, source } => {
            let mut p = load(&project)?;
            let k = effect_kind(&kind)?;
            let n_tracks = p.mixer.len();
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} existiert nicht"))?;
            if t.fx.len() >= MAX_FX_SLOTS {
                return Err(format!("Spur {track} hat schon {MAX_FX_SLOTS} Effekte"));
            }
            let mut slot = FxSlot::new(k);
            if let tstudio_dsp::fx::EffectParams::Sidechain(sc) = &mut slot.params {
                let src = source.ok_or("sidechain braucht --source SPUR (Mixer-Spur der Kick)")?;
                if src >= n_tracks || src == track {
                    return Err(format!("--source {src} ungültig"));
                }
                sc.source = src;
            }
            t.fx.push(slot);
            let slot = t.fx.len() - 1;
            let params = serde_json::to_value(&t.fx[slot].params).unwrap_or_default();
            save(&project, &mut p)?;
            Ok(json!({
                "ok": true, "track": track, "slot": slot, "params": params,
                "set_with": format!("mixer.{track}.fx.{slot}.params.{}.<feld>=<wert>", k.name()),
            }))
        }
        FxCmd::Chain { project, track, name, source, replace } => {
            let mut p = load(&project)?;
            if track >= p.mixer.len() {
                return Err(format!("Mixer-Spur {track} existiert nicht"));
            }
            if let Some(s) = source {
                if s == 0 || s >= p.mixer.len() || s == track {
                    return Err(format!("--source {s} ungültig"));
                }
            }
            let slots = tools::chain(&name, source)?;
            let t = &mut p.mixer[track];
            if replace {
                t.fx.clear();
            }
            if t.fx.len() + slots.len() > MAX_FX_SLOTS {
                return Err(format!("Spur {track} hat {} Effekte, die Kette braucht {} Plätze (max {MAX_FX_SLOTS}) – --replace nutzen", t.fx.len(), slots.len()));
            }
            let first = t.fx.len();
            let kinds: Vec<String> = slots.iter().map(fx_kind_name_p).collect();
            t.fx.extend(slots);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": track, "chain": name, "slots_from": first, "effects": kinds}))
        }
        FxCmd::Remove { project, track, slot } => {
            let mut p = load(&project)?;
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} existiert nicht"))?;
            if slot >= t.fx.len() {
                return Err(format!("Slot {slot} existiert nicht"));
            }
            t.fx.remove(slot);
            save(&project, &mut p)?;
            Ok(json!({"ok": true}))
        }
    }
}
