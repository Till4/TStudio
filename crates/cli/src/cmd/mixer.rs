//! mixer commands (volume, pan, sends, sidechain …)

use crate::*;

pub(crate) fn mixer_cmd(c: MixerCmd) -> R<Value> {
    match c {
        MixerCmd::Rumble { project, kick, amount, name } => {
            let mut p = load(&project)?;
            if kick == 0 || kick >= p.mixer.len() {
                return Err(format!("--kick {kick}: Mixer-Spur der Kick (1..{})", p.mixer.len() - 1));
            }
            let t = p.add_mixer_track(Some(name.clone()));
            if t <= kick {
                return Err("Rumble-Bus muss hinter der Kick-Spur liegen".into());
            }
            p.mixer[kick].sends.retain(|s| s.target != t);
            p.mixer[kick].sends.push(MixerSend { target: t, amount: amount.clamp(0.0, 1.0) });
            p.mixer[t].fx = tools::chain("rumble", Some(kick))?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "rumble_track": t, "name": name, "send_from": kick, "amount": amount,
                "hint": format!("Lautstärke: set PROJECT mixer.{t}.volume=0.6 · Tiefpass: mixer.{t}.fx.2.params.Filter.cutoff")}))
        }
        MixerCmd::Send { project, from, to, amount } => {
            let mut p = load(&project)?;
            if from == 0 || to <= from || to >= p.mixer.len() {
                return Err(format!("Sends gehen nur von einer Insert-Spur auf eine höhere (1..{}), z.B. --from 2 --to 10", p.mixer.len() - 1));
            }
            let t = &mut p.mixer[from];
            t.sends.retain(|s| s.target != to);
            if amount > 0.0 {
                t.sends.push(MixerSend { target: to, amount });
            }
            let sends = t.sends.iter().map(|s| json!({"to": s.target, "amount": s.amount})).collect::<Vec<_>>();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": from, "sends": sends}))
        }
        MixerCmd::Route { project, from, to } => {
            let mut p = load(&project)?;
            if from == 0 || from >= p.mixer.len() || (to != 0 && (to <= from || to >= p.mixer.len())) {
                return Err("Route: --from Insert-Spur, --to 0 (Master) oder eine höhere Spur".into());
            }
            p.mixer[from].output = to;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": from, "output": to}))
        }
        MixerCmd::Add { project, name, at } => {
            let mut p = load(&project)?;
            let t = match at {
                Some(a) => p.insert_mixer_track(a, name),
                None => p.add_mixer_track(name),
            };
            let name = p.mixer[t].name.clone();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": t, "name": name, "mixer_tracks": p.mixer.len()}))
        }
        MixerCmd::Delete { project, track } => {
            let mut p = load(&project)?;
            let name = p.mixer.get(track).map(|t| t.name.clone());
            p.delete_mixer_track(track)?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "deleted": track, "name": name, "mixer_tracks": p.mixer.len()}))
        }
        MixerCmd::Move { project, from, to } => {
            let mut p = load(&project)?;
            p.move_mixer_track(from, to)?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "moved": from, "to": to, "name": p.mixer[to].name}))
        }
        MixerCmd::Rename { project, track, name } => {
            let mut p = load(&project)?;
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} gibt es nicht"))?;
            t.name = name.clone();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": track, "name": name}))
        }
        MixerCmd::Mute { project, track, off } => {
            let mut p = load(&project)?;
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} gibt es nicht"))?;
            t.muted = !off;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": track, "muted": !off}))
        }
        MixerCmd::Solo { project, track, off, only, clear } => {
            let mut p = load(&project)?;
            if clear || only {
                for t in p.mixer.iter_mut() {
                    t.solo = false;
                }
            }
            if !clear {
                if track == 0 || track >= p.mixer.len() {
                    return Err(format!("Solo gibt es für Insert 1..{}", p.mixer.len() - 1));
                }
                p.mixer[track].solo = !off;
            }
            let solos: Vec<usize> = p.mixer.iter().enumerate().filter(|(_, t)| t.solo).map(|(i, _)| i).collect();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "solo": solos}))
        }
    }
}
