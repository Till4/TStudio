//! note editing commands

use crate::*;

pub(crate) fn notes_cmd(c: NotesCmd) -> R<Value> {
    match c {
        NotesCmd::Add { t, notes, len, vel, offset, replace } => {
            let mut p = load(&t.project)?;
            let ci = channel_idx(&p, &t.channel)?;
            let cid = p.channels[ci].id;
            let pid = pattern_id(&p, &t.pattern)?;
            let default_len = spec::parse_beats(&len)?;
            let off = spec::parse_beats(&offset)?;
            let mut new = Vec::new();
            for tok in notes.iter().flat_map(|s| s.split([' ', ',', ';'])).filter(|s| !s.trim().is_empty()) {
                let n = spec::parse_note(tok.trim(), default_len, vel)?;
                for k in n.keys {
                    new.push(Note { channel: cid, key: k, start: n.start + off, len: n.len, velocity: n.velocity });
                }
            }
            let pat = p.pattern_mut(pid).unwrap();
            if replace {
                pat.notes.retain(|n| n.channel != cid);
            }
            // identical key+start replaces the old note
            pat.notes.retain(|o| !(o.channel == cid && new.iter().any(|n| n.key == o.key && n.start == o.start)));
            let added = new.len();
            pat.notes.extend(new);
            pat.notes.sort_by_key(|n| (n.start, n.key));
            let total = pat.notes.iter().filter(|n| n.channel == cid).count();
            let len_bars = p.pattern_length(pid) as f64 / p.ticks_per_bar() as f64;
            save(&t.project, &mut p)?;
            Ok(json!({"ok": true, "added": added, "channel_notes": total, "pattern_length_bars": len_bars}))
        }
        NotesCmd::Steps { t, steps, key, len, offset, keep } => {
            let mut p = load(&t.project)?;
            let ci = channel_idx(&p, &t.channel)?;
            let cid = p.channels[ci].id;
            let pid = pattern_id(&p, &t.pattern)?;
            let key = match key {
                Some(k) => spec::parse_key(&k)?,
                None => match &p.channels[ci].instrument {
                    Instrument::Sampler(s) => s.params.root_note,
                    Instrument::Synth(_) => 60,
                    Instrument::SoundFont(s) => if s.bank >= 128 { 36 } else { 60 },
                },
            };
            let off = spec::parse_beats(&offset)?;
            let parsed = spec::parse_steps(&steps)?;
            let step = PPQ / 4;
            let pat = p.pattern_mut(pid).unwrap();
            if !keep {
                pat.notes.retain(|n| n.channel != cid);
            }
            for (i, v) in &parsed {
                let start = off + *i as i64 * step;
                pat.notes.retain(|n| !(n.channel == cid && n.start == start && n.key == key));
                pat.notes.push(Note { channel: cid, key, start, len: step * len.max(1), velocity: *v });
            }
            pat.notes.sort_by_key(|n| (n.start, n.key));
            save(&t.project, &mut p)?;
            Ok(json!({"ok": true, "hits": parsed.len(), "key": note_name(key)}))
        }
        NotesCmd::Chords { t, progression, bars_per_chord, style, octave, vel, rhythm, offset, replace, no_voicing } => {
            let mut p = load(&t.project)?;
            let ci = channel_idx(&p, &t.channel)?;
            let cid = p.channels[ci].id;
            let pid = pattern_id(&p, &t.pattern)?;
            let off = spec::parse_beats(&offset)?;
            let bass_like = matches!(style.to_lowercase().as_str(), "bass" | "offbeat" | "rolling");
            let oct = octave.unwrap_or(if bass_like { 2 } else { 4 });
            let (gen, names) = tools::chord_notes(&progression, bars_per_chord, &style, oct, vel, rhythm.as_deref(), p.ticks_per_bar(), !no_voicing)?;
            let pat = p.pattern_mut(pid).unwrap();
            if replace {
                pat.notes.retain(|n| n.channel != cid);
            }
            let added = gen.len();
            for g in gen {
                let start = g.start + off;
                pat.notes.retain(|n| !(n.channel == cid && n.start == start && n.key == g.key));
                pat.notes.push(Note { channel: cid, key: g.key, start, len: g.len, velocity: g.vel });
            }
            pat.notes.sort_by_key(|n| (n.start, n.key));
            let len_bars = p.pattern_length(pid) as f64 / p.ticks_per_bar() as f64;
            save(&t.project, &mut p)?;
            Ok(json!({"ok": true, "chords": names, "notes_added": added, "octave": oct, "pattern_length_bars": len_bars}))
        }
        NotesCmd::Clear { project, pattern, channel } => {
            let mut p = load(&project)?;
            let pid = pattern_id(&p, &pattern)?;
            let cid = match &channel {
                Some(c) => Some(p.channels[channel_idx(&p, c)?].id),
                None => None,
            };
            let pat = p.pattern_mut(pid).unwrap();
            let before = pat.notes.len();
            pat.notes.retain(|n| cid.map(|c| n.channel != c).unwrap_or(false));
            let removed = before - pat.notes.len();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "removed": removed}))
        }
        NotesCmd::List { project, pattern, channel } => {
            let p = load(&project)?;
            let pid = pattern_id(&p, &pattern)?;
            let cid = match &channel {
                Some(c) => Some(p.channels[channel_idx(&p, c)?].id),
                None => None,
            };
            let pat = p.pattern(pid).unwrap();
            let notes: Vec<Value> = pat
                .notes
                .iter()
                .filter(|n| cid.map(|c| n.channel == c).unwrap_or(true))
                .map(|n| {
                    json!({
                        "channel": p.channel_index(n.channel),
                        "key": note_name(n.key),
                        "midi": n.key,
                        "start_beats": ticks_to_beats(n.start),
                        "len_beats": ticks_to_beats(n.len),
                        "velocity": (n.velocity * 100.0).round() / 100.0,
                    })
                })
                .collect();
            Ok(json!({"pattern": pat.name, "length_bars": p.pattern_length(pid) as f64 / p.ticks_per_bar() as f64, "notes": notes}))
        }
    }
}
