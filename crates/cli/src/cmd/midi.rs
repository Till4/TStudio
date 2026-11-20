//! `midi export|import` – Standard MIDI Files.

use crate::*;
use tstudio_project::midi;

pub(crate) fn midi_cmd(c: MidiCmd) -> R<Value> {
    match c {
        MidiCmd::Export { project, out, pattern } => {
            let p = load(&project)?;
            let bytes = match &pattern {
                Some(sel) => midi::export_pattern(&p, pattern_id(&p, sel)?)?,
                None => midi::export_song(&p)?,
            };
            std::fs::write(&out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
            Ok(json!({"ok": true, "file": abs(&out), "bytes": bytes.len(), "source": pattern.unwrap_or_else(|| "song".into())}))
        }
        MidiCmd::Import { project, file, name, keep_tempo } => {
            let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let mut p = load(&project)?;
            let name = name.unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "MIDI".into()));
            let r = midi::import(&mut p, &bytes, &name, !keep_tempo)?;
            let pattern = p.patterns.iter().position(|x| x.id == r.pattern);
            let channels: Vec<Value> = r.channels.iter().map(|i| json!({"index": i, "name": p.channels[*i].name})).collect();
            let bpm = p.bpm;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "pattern": pattern, "name": name, "channels": channels, "notes": r.notes, "file_bpm": r.bpm, "bpm": bpm}))
        }
    }
}
