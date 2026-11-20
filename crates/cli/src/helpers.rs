//! Shared helpers: project load/save, selector parsing, JSON path get/set, `info`.

use crate::*;

pub(crate) type R<T> = Result<T, String>;

/// `dj_name` from TStudio's settings.ron (empty if not set).
/// TStudio's user template folder (same place the app uses).
pub(crate) fn templates_dir() -> PathBuf {
    directories::ProjectDirs::from("de", "TStudio", "TStudio").map(|d| d.config_dir().join("templates")).unwrap_or_else(|| std::env::temp_dir().join("TStudio").join("templates"))
}

pub(crate) fn settings_dj_name() -> String {
    let Some(dir) = directories::ProjectDirs::from("de", "TStudio", "TStudio") else { return String::new() };
    let Ok(s) = std::fs::read_to_string(dir.config_dir().join("settings.ron")) else { return String::new() };
    let Some(i) = s.find("dj_name: \"") else { return String::new() };
    let mut out = String::new();
    let mut esc = false;
    for c in s[i + 10..].chars() {
        match (esc, c) {
            (false, '\\') => esc = true,
            (false, '"') => break,
            (_, c) => {
                out.push(c);
                esc = false;
            }
        }
    }
    out
}

pub(crate) fn load(p: &Path) -> R<Project> {
    live::guard(p)?;
    if live::targets_window(p) {
        let v = live::request("get", json!({}))?;
        return Project::from_ron(v["project_ron"].as_str().unwrap_or_default()).map_err(|e| format!("Projekt aus TStudio unlesbar: {e}"));
    }
    if !p.exists() {
        return Err(format!("{} gibt es nicht (für das in TStudio offene Projekt: `live`)", p.display()));
    }
    Project::load(p).map_err(|e| format!("{}: {e}", p.display()))
}

pub(crate) fn save(p: &Path, proj: &mut Project) -> R<()> {
    live::guard(p)?;
    proj.sanitize();
    if live::targets_window(p) {
        let ron = proj.to_ron()?;
        // status-bar label: the command words before the project argument, e.g. "notes add"
        let arg = p.to_string_lossy();
        let label: Vec<String> = std::env::args().skip(1).take_while(|a| *a != arg).take(3).collect();
        live::request("put", json!({"project_ron": ron, "label": label.join(" ")}))?;
        return Ok(());
    }
    proj.save(p).map_err(|e| format!("Speichern fehlgeschlagen ({}): {e}", p.display()))
}

pub(crate) fn abs(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

pub(crate) fn channel_idx(p: &Project, sel: &str) -> R<usize> {
    if let Ok(i) = sel.parse::<usize>() {
        if i < p.channels.len() {
            return Ok(i);
        }
    }
    p.channels
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(sel))
        .ok_or_else(|| format!("Kanal '{sel}' nicht gefunden (vorhanden: {})", p.channels.iter().enumerate().map(|(i, c)| format!("{i}={}", c.name)).collect::<Vec<_>>().join(", ")))
}

pub(crate) fn pattern_id(p: &Project, sel: &str) -> R<Id> {
    if let Ok(i) = sel.parse::<usize>() {
        if let Some(pat) = p.patterns.get(i) {
            return Ok(pat.id);
        }
    }
    p.patterns
        .iter()
        .find(|x| x.name.eq_ignore_ascii_case(sel))
        .map(|x| x.id)
        .ok_or_else(|| format!("Pattern '{sel}' nicht gefunden (vorhanden: {})", p.patterns.iter().enumerate().map(|(i, x)| format!("{i}={}", x.name)).collect::<Vec<_>>().join(", ")))
}

pub(crate) fn stretch_mode(s: &str) -> R<StretchMode> {
    match s.to_lowercase().as_str() {
        "stretch" | "time" => Ok(StretchMode::Stretch),
        "resample" | "vinyl" => Ok(StretchMode::Resample),
        "off" | "none" => Ok(StretchMode::Off),
        _ => Err(format!("Stretch-Modus '{s}' unbekannt (stretch, resample, off)")),
    }
}

pub(crate) fn synth_preset(name: &str) -> R<SynthParams> {
    let all = SynthParams::presets();
    all.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, p)| p.clone())
        .ok_or_else(|| format!("Preset '{name}' unbekannt (vorhanden: {})", all.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")))
}

/// Picks a SoundFont preset by name / part of the name / `BANK:PROGRAM`, or explicit numbers.
pub(crate) fn pick_sf_preset(presets: &[Sf2Preset], sel: Option<&str>, program: Option<u8>, bank: Option<u16>) -> R<Sf2Preset> {
    if presets.is_empty() {
        return Err("SoundFont enthält keine Presets".into());
    }
    if let Some(s) = sel {
        if let Some((b, p)) = s.split_once(':') {
            if let (Ok(b), Ok(p)) = (b.trim().parse::<u16>(), p.trim().parse::<u8>()) {
                return presets.iter().find(|x| x.bank == b && x.program == p).cloned().ok_or_else(|| format!("kein Preset {b}:{p}"));
            }
        }
        let l = s.to_lowercase();
        return presets
            .iter()
            .find(|x| x.name.to_lowercase() == l)
            .or_else(|| presets.iter().find(|x| x.name.to_lowercase().contains(&l)))
            .cloned()
            .ok_or_else(|| format!("kein Preset '{s}' (tstudio-cli soundfont DATEI listet alle)"));
    }
    if program.is_some() || bank.is_some() {
        let (b, p) = (bank.unwrap_or(0), program.unwrap_or(0));
        return presets.iter().find(|x| x.bank == b && x.program == p).cloned().ok_or_else(|| format!("kein Preset {b}:{p}"));
    }
    Ok(presets[0].clone())
}

pub(crate) fn effect_kind(s: &str) -> R<EffectKind> {
    EffectKind::ALL
        .into_iter()
        .find(|k| k.name().eq_ignore_ascii_case(s) || (s.eq_ignore_ascii_case("comp") && *k == EffectKind::Compressor))
        .ok_or_else(|| format!("Effekt '{s}' unbekannt (vorhanden: {})", EffectKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", ")))
}

/// Stretch settings for a sample: explicit BPM, else from name/detection.
pub(crate) fn stretch_for(path: &Path, args: &StretchArgs, data: Option<&SampleData>) -> R<(StretchSettings, Option<f64>)> {
    let detected = data.map(|d| tstudio_library::analyze_data(d, &path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())).and_then(|a| a.bpm);
    let mode = match &args.stretch {
        Some(m) => stretch_mode(m)?,
        None => StretchMode::Off,
    };
    let src = args.sample_bpm.or(detected);
    if mode != StretchMode::Off && src.is_none() {
        return Err("Sample-BPM unbekannt – bitte --sample-bpm angeben".into());
    }
    Ok((StretchSettings { mode, source_bpm: src.unwrap_or(120.0), target_bpm: None, pitch: args.pitch.unwrap_or(0.0), formant: args.formant }, detected))
}

pub(crate) fn ticks_to_beats(t: i64) -> f64 {
    (t as f64 / PPQ as f64 * 1000.0).round() / 1000.0
}

pub(crate) fn info(p: &Project) -> Value {
    let bar = p.ticks_per_bar();
    json!({
        "name": p.name,
        "bpm": p.bpm,
        "beats_per_bar": p.beats_per_bar,
        "tempo_changes": p.tempo.len(),
        "song_length_s": (p.tick_to_time(p.song_length() as f64) * 1000.0).round() / 1000.0,
        "swing": p.swing,
        "master_volume": p.mixer[0].volume,
        "song_length_bars": p.tick_to_bar_number(p.song_length()) - 1.0,
        "meters": p.meters.iter().map(|m| json!({"bar": m.bar + 1, "meter": m.label()})).collect::<Vec<_>>(),
        "channels": p.channels.iter().enumerate().map(|(i, c)| {
            let mut v = json!({"index": i, "name": c.name, "type": c.instrument.kind_name(), "mixer_track": c.mixer_track, "volume": c.volume, "pan": c.pan, "muted": c.muted});
            if let Instrument::Sampler(s) = &c.instrument {
                v["sample"] = json!(s.sample);
                v["root"] = json!(note_name(s.params.root_note));
                v["stretch"] = json!(s.stretch.mode.name());
                if !s.params.slices.is_empty() { v["slices"] = json!(s.params.slices.len()); }
            }
            if let Instrument::SoundFont(s) = &c.instrument {
                v["soundfont"] = json!(s.path);
                v["preset"] = json!(format!("{}:{} {}", s.bank, s.program, s.preset_name));
            }
            v
        }).collect::<Vec<_>>(),
        "patterns": p.patterns.iter().enumerate().map(|(i, pat)| {
            let mut per: Vec<Value> = Vec::new();
            for (ci, c) in p.channels.iter().enumerate() {
                let n = pat.notes.iter().filter(|n| n.channel == c.id).count();
                if n > 0 { per.push(json!({"channel": ci, "notes": n})); }
            }
            json!({"index": i, "name": pat.name, "length_bars": p.pattern_length(pat.id) as f64 / bar as f64, "notes": pat.notes.len(), "by_channel": per})
        }).collect::<Vec<_>>(),
        "playlist_tracks": p.playlist.tracks.len(),
        "tracks": (0..p.playlist.tracks.len()).filter(|t| {
            let tr = &p.playlist.tracks[*t];
            tr.muted || tr.solo || !tr.name.starts_with("Track ") || p.playlist.clips.iter().any(|c| c.track == *t)
        }).map(|t| track_json(p, t)).collect::<Vec<_>>(),
        "clips": p.playlist.clips.iter().map(|c| {
            let what = match &c.kind {
                ClipKind::Pattern { pattern } => json!({"pattern": p.patterns.iter().position(|x| x.id == *pattern), "pattern_name": p.pattern(*pattern).map(|x| x.name.clone())}),
                ClipKind::Audio(a) => json!({"audio": a.path, "stretch": a.stretch.mode.name(), "gain": a.gain, "mixer_track": a.mixer_track}),
                ClipKind::Automation(a) => json!({"automation": a.target, "points": a.points.len(),
                    "from": a.points.first().map(|q| q.v), "to": a.points.last().map(|q| q.v)}),
            };
            json!({"id": c.id, "track": c.track + 1, "bar": p.tick_to_bar_number(c.start), "bars": p.tick_to_bar_number(c.end()) - p.tick_to_bar_number(c.start), "muted": c.muted, "content": what})
        }).collect::<Vec<_>>(),
        "mixer": p.mixer.iter().enumerate().filter(|(i, t)| *i == 0 || !t.fx.is_empty() || p.channels.iter().any(|c| c.mixer_track == *i) || (t.volume - 0.8).abs() > 1e-4 || t.muted || t.solo
                || t.output != 0 || !t.sends.is_empty() || p.mixer.iter().any(|o| o.output == *i || o.sends.iter().any(|s| s.target == *i)))
            .map(|(i, t)| {
                let mut v = json!({"track": i, "name": t.name, "volume": t.volume, "pan": t.pan, "muted": t.muted, "solo": t.solo,
                    "fx": t.fx.iter().enumerate().map(|(s, f)| {
                        let mut fx = json!({"slot": s, "kind": fx_kind_name(&f.params), "enabled": f.enabled});
                        if let Some(src) = f.params.sidechain_source() { fx["sidechain_source"] = json!(src); }
                        fx
                    }).collect::<Vec<_>>()});
                if t.output != 0 { v["output"] = json!(t.output); }
                if !t.sends.is_empty() { v["sends"] = json!(t.sends.iter().map(|s| json!({"to": s.target, "amount": s.amount})).collect::<Vec<_>>()); }
                v
            })
            .collect::<Vec<_>>(),
        "hint": "Indizes sind 0-basiert (Kanäle, Patterns, Mixer: 0 = Master); Clip-Spuren/Takte 1-basiert wie in der GUI.",
    })
}

pub(crate) fn fx_kind_name_p(s: &FxSlot) -> String {
    s.params.kind().name().to_string()
}

pub(crate) fn fx_kind_name(p: &tstudio_dsp::fx::EffectParams) -> String {
    serde_json::to_value(p).ok().and_then(|v| v.as_object().and_then(|o| o.keys().next().cloned())).unwrap_or_default()
}

// --- generic JSON-path access

pub(crate) fn path_segments(path: &str) -> Vec<String> {
    path.split(['.', '/']).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
}

pub(crate) fn get_path<'a>(mut v: &'a Value, path: &str) -> R<&'a Value> {
    for seg in path_segments(path) {
        v = match v {
            Value::Object(o) => o.get(&seg).ok_or_else(|| format!("'{seg}' nicht gefunden (möglich: {})", o.keys().cloned().collect::<Vec<_>>().join(", ")))?,
            Value::Array(a) => {
                let i: usize = seg.parse().map_err(|_| format!("'{seg}' ist kein Index"))?;
                a.get(i).ok_or_else(|| format!("Index {i} außerhalb (Länge {})", a.len()))?
            }
            _ => return Err(format!("'{seg}': Wert hat keine Unterfelder")),
        };
    }
    Ok(v)
}

pub(crate) fn set_path(root: &mut Value, path: &str, new: Value) -> R<()> {
    let segs = path_segments(path);
    let (last, parents) = segs.split_last().ok_or("leerer Pfad")?;
    let mut v = root;
    for seg in parents {
        v = match v {
            Value::Object(o) => o.get_mut(seg).ok_or_else(|| format!("'{seg}' nicht gefunden"))?,
            Value::Array(a) => {
                let n = a.len();
                let i: usize = seg.parse().map_err(|_| format!("'{seg}' ist kein Index"))?;
                a.get_mut(i).ok_or_else(|| format!("Index {i} außerhalb (Länge {n})"))?
            }
            _ => return Err(format!("'{seg}': Wert hat keine Unterfelder")),
        };
    }
    match v {
        Value::Object(o) => {
            if !o.contains_key(last.as_str()) && !o.is_empty() {
                return Err(format!("Feld '{last}' existiert nicht (möglich: {})", o.keys().cloned().collect::<Vec<_>>().join(", ")));
            }
            o.insert(last.clone(), new);
        }
        Value::Array(a) => {
            let n = a.len();
            let i: usize = last.parse().map_err(|_| format!("'{last}' ist kein Index"))?;
            *a.get_mut(i).ok_or_else(|| format!("Index {i} außerhalb (Länge {n})"))? = new;
        }
        _ => return Err("Ziel ist kein Objekt/Array".into()),
    }
    Ok(())
}
