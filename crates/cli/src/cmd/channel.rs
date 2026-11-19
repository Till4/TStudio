//! channel add/set/remove …

use crate::*;

pub(crate) fn channel_cmd(c: ChannelCmd) -> R<Value> {
    match c {
        ChannelCmd::Add { project, synth, preset, sample, soundfont, program, bank, name, root, stretch } => {
            let mut p = load(&project)?;
            let mut extra = json!({});
            let (inst, default_name) = match (&sample, synth || preset.is_some()) {
                _ if soundfont.is_some() => {
                    let path = abs(soundfont.as_ref().unwrap());
                    let presets = soundfont_presets(&path)?;
                    let pr = pick_sf_preset(&presets, preset.as_deref(), program, bank)?;
                    extra["preset"] = json!(format!("{}:{} {}", pr.bank, pr.program, pr.name));
                    extra["presets_in_file"] = json!(presets.len());
                    let name = if pr.name.is_empty() { path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("SoundFont".into()) } else { pr.name.clone() };
                    (Instrument::SoundFont(SoundFontInstrument { path, bank: pr.bank, program: pr.program, preset_name: pr.name, gain: 1.0 }), name)
                }
                (Some(path), _) => {
                    let path = abs(path);
                    let data = audio::load(&path)?;
                    let (st, detected) = stretch_for(&path, &stretch, Some(&data))?;
                    extra["detected_bpm"] = json!(detected);
                    let mut s = SamplerInstrument { sample: Some(path.clone()), stretch: st, ..Default::default() };
                    if let Some(r) = &root {
                        s.params.root_note = spec::parse_key(r)?;
                    }
                    (Instrument::Sampler(s), path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("Sampler".into()))
                }
                (None, true) => {
                    let params = match &preset {
                        Some(n) => synth_preset(n)?,
                        None => SynthParams::default(),
                    };
                    (Instrument::Synth(params), preset.clone().unwrap_or("TSynth".into()))
                }
                (None, false) => return Err("--synth [--preset NAME], --sample DATEI oder --soundfont DATEI.sf2 angeben".into()),
            };
            let idx = p.add_channel(&name.unwrap_or(default_name), inst);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "channel": idx, "name": p.channels[idx].name, "mixer_track": p.channels[idx].mixer_track, "extra": extra}))
        }
        ChannelCmd::Remove { project, channel } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let name = p.channels[i].name.clone();
            p.remove_channel(i);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "removed": name}))
        }
        ChannelCmd::Mod { project, channel, source, dest, amount, clear } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let Instrument::Synth(s) = &mut p.channels[i].instrument else { return Err(format!("Kanal {i} ist kein TSynth")) };
            if clear {
                s.mod_matrix = [ModSlot::default(); tstudio_dsp::synth::NUM_MOD_SLOTS];
            }
            match (&source, &dest) {
                (Some(src), Some(dst)) => {
                    let src = ModSource::ALL
                        .into_iter()
                        .find(|m| format!("{m:?}").eq_ignore_ascii_case(src) || m.name().eq_ignore_ascii_case(src))
                        .ok_or_else(|| format!("Quelle '{src}' unbekannt (möglich: {})", ModSource::ALL.iter().skip(1).map(|m| format!("{m:?}")).collect::<Vec<_>>().join(", ")))?;
                    let dst = ModDest::ALL
                        .into_iter()
                        .find(|m| format!("{m:?}").eq_ignore_ascii_case(dst) || m.name().eq_ignore_ascii_case(dst))
                        .ok_or_else(|| format!("Ziel '{dst}' unbekannt (möglich: {})", ModDest::ALL.iter().skip(1).map(|m| format!("{m:?}")).collect::<Vec<_>>().join(", ")))?;
                    if dst.is_global() && !src.is_macro() {
                        return Err(format!("{} ist ein FX-Ziel und nimmt nur Makros als Quelle", dst.name()));
                    }
                    if !s.add_mod(src, dst, amount.clamp(-1.0, 1.0)) {
                        return Err("Mod-Matrix ist voll (16 Slots) – `--clear` oder per `set` überschreiben".into());
                    }
                }
                (None, None) => {}
                _ => return Err("--source und --dest gehören zusammen".into()),
            }
            let matrix: Vec<Value> = s.mod_matrix.iter().enumerate().filter(|(_, m)| m.active()).map(|(k, m)| json!({"slot": k, "source": m.source, "dest": m.dest, "amount": m.amount})).collect();
            let macros = s.macros();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "channel": i, "mod_matrix": matrix, "macros": macros, "set_macro_with": format!("channels.{i}.instrument.Synth.macro1=0.7")}))
        }
        ChannelCmd::Kick { project, channel, tune, off } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let Instrument::Synth(s) = &mut p.channels[i].instrument else { return Err(format!("Kanal {i} ist kein TSynth")) };
            s.kick.enabled = !off;
            if let Some(t) = &tune {
                s.kick.tune = match t.trim().parse::<f32>() {
                    Ok(n) => n,
                    Err(_) => spec::parse_key(t)? as f32,
                };
            }
            let k = s.kick.clone();
            save(&project, &mut p)?;
            Ok(json!({
                "ok": true,
                "channel": i,
                "kick": k,
                "fundamental_hz": tstudio_dsp::midi_to_freq(k.tune),
                "note": tstudio_dsp::note_name(k.tune.round().clamp(0.0, 127.0) as u8),
            }))
        }
        ChannelCmd::Preset { project, channel, preset } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let chosen = match &mut p.channels[i].instrument {
                Instrument::Synth(s) => {
                    *s = synth_preset(&preset)?;
                    preset.clone()
                }
                Instrument::SoundFont(s) => {
                    let pr = pick_sf_preset(&soundfont_presets(&s.path)?, Some(&preset), None, None)?;
                    s.bank = pr.bank;
                    s.program = pr.program;
                    s.preset_name = pr.name.clone();
                    format!("{}:{} {}", pr.bank, pr.program, pr.name)
                }
                Instrument::Sampler(_) => return Err(format!("Kanal {i} ist ein Sampler – Presets gibt es für TSynth und SoundFont")),
            };
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "channel": i, "preset": chosen}))
        }
    }
}
