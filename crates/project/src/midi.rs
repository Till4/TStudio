//! Standard MIDI File import/export (via `midly`).
//!
//! Export: a pattern or the whole playlist (pattern clips, repeats included) as format-1 file:
//! track 0 holds name, tempo and time signature, then one track per channel with notes.
//! A song export writes the whole tempo track (ramps as one tempo step per beat) and every
//! time-signature change; a pattern export only the start tempo and meter.
//! SoundFont drum kits (bank 128) go to MIDI channel 10, the others count up skipping it.
//!
//! Import: every MIDI track with notes becomes a new TSynth channel (format-0 files are split by
//! MIDI channel instead) and all notes land in one new pattern; the first tempo event sets the
//! project tempo if wanted.

use crate::{Id, Instrument, Note, Pattern, Project, PPQ};
use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use std::collections::{BTreeMap, HashMap};

/// Notes of a pattern as (channel index, note).
pub fn pattern_notes(p: &Project, pattern: Id) -> Vec<(usize, Note)> {
    let Some(pat) = p.pattern(pattern) else { return Vec::new() };
    pat.notes.iter().filter_map(|n| Some((p.channel_index(n.channel)?, n.clone()))).collect()
}

/// Notes the playlist plays (unmuted pattern clips on audible tracks; a clip longer than its
/// pattern repeats it, notes are cut at the clip end), sorted by time.
pub fn song_notes(p: &Project) -> Vec<(usize, Note)> {
    let mut out = Vec::new();
    for c in &p.playlist.clips {
        let crate::ClipKind::Pattern { pattern } = c.kind else { continue };
        if c.muted || !p.playlist_track_audible(c.track) {
            continue;
        }
        let plen = p.pattern_length(pattern).max(1);
        let notes = pattern_notes(p, pattern);
        let end = c.end();
        let mut base = c.start;
        while base < end {
            for (ci, n) in &notes {
                let start = base + n.start;
                if n.start < plen && start < end {
                    out.push((*ci, Note { start, len: n.len.min(end - start).max(1), ..n.clone() }));
                }
            }
            base += plen;
        }
    }
    out.sort_by_key(|(ci, n)| (n.start, *ci, n.key));
    out
}

fn tempo_us(bpm: f64) -> u32 {
    (60_000_000.0 / bpm.max(1.0)).round().clamp(1.0, 16_777_215.0) as u32
}

/// MIDI channel (0-based) for each exported project channel, in order of appearance.
fn midi_channels(p: &Project, used: &[usize]) -> HashMap<usize, u8> {
    let mut next = 0u8;
    let mut map = HashMap::new();
    for ci in used {
        let drums = matches!(&p.channels[*ci].instrument, Instrument::SoundFont(s) if s.bank >= 128);
        let ch = if drums {
            9
        } else {
            let c = next;
            next = (next + 1) % 16;
            if next == 9 {
                next = 10;
            }
            c
        };
        map.insert(*ci, ch);
    }
    map
}

/// Conductor events (tick, event) for the song: tempo changes (ramps stepped per beat) and
/// time-signature changes. Without `song` only the start tempo and meter.
fn conductor(p: &Project, song: bool) -> Vec<(i64, MetaMessage<'static>)> {
    let sig = |beats: u32, unit: u32| MetaMessage::TimeSignature(beats.clamp(1, 255) as u8, unit.max(1).trailing_zeros() as u8, 24, 8);
    let mut out = vec![(0, MetaMessage::Tempo(u24::new(tempo_us(p.bpm)))), (0, sig(p.beats_per_bar, 4))];
    if !song {
        return out;
    }
    for s in p.bar_map().segments().iter().filter(|s| s.tick > 0) {
        out.push((s.tick, sig(s.beats, s.unit)));
    }
    let map = p.tempo_map();
    let mut pts = p.tempo.clone();
    pts.sort_by_key(|t| t.tick);
    let mut prev = 0i64;
    for t in pts.iter().filter(|t| t.tick > 0) {
        if t.ramp {
            let mut x = prev + PPQ;
            while x < t.tick {
                out.push((x, MetaMessage::Tempo(u24::new(tempo_us(map.bpm_at(x as f64))))));
                x += PPQ;
            }
        }
        out.push((t.tick, MetaMessage::Tempo(u24::new(tempo_us(t.bpm)))));
        prev = t.tick;
    }
    out.sort_by_key(|e| e.0);
    out
}

/// Writes notes (ticks in project PPQ) as a format-1 Standard MIDI File (start tempo/meter only).
pub fn write_smf(p: &Project, notes: &[(usize, Note)]) -> Result<Vec<u8>, String> {
    write_smf_with(p, notes, false)
}

fn write_smf_with(p: &Project, notes: &[(usize, Note)], song: bool) -> Result<Vec<u8>, String> {
    if notes.is_empty() {
        return Err("keine Noten zum Exportieren".into());
    }
    let mut by_channel: BTreeMap<usize, Vec<&Note>> = BTreeMap::new();
    for (ci, n) in notes {
        by_channel.entry(*ci).or_default().push(n);
    }
    let used: Vec<usize> = by_channel.keys().copied().collect();
    let chans = midi_channels(p, &used);
    let names: Vec<String> = std::iter::once(p.name.clone()).chain(used.iter().map(|ci| p.channels[*ci].name.clone())).collect();

    let mut smf = Smf::new(Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ as u16))));
    let mut track0 = vec![TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(names[0].as_bytes())) }];
    let mut last = 0i64;
    for (t, m) in conductor(p, song) {
        track0.push(TrackEvent { delta: u28::new((t - last) as u32), kind: TrackEventKind::Meta(m) });
        last = t;
    }
    track0.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
    smf.tracks.push(track0);
    for (k, ci) in used.iter().enumerate() {
        let channel = u4::new(chans[ci]);
        // (tick, on?, key, velocity); note-offs sort before note-ons at the same tick
        let mut ev: Vec<(i64, bool, u8, u8)> = Vec::new();
        for n in &by_channel[ci] {
            let vel = (n.velocity.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8;
            let key = n.key.min(127);
            ev.push((n.start.max(0), true, key, vel));
            ev.push((n.end().max(n.start.max(0) + 1), false, key, 0));
        }
        ev.sort_by_key(|e| (e.0, e.1, e.2));
        let mut track = vec![TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(names[k + 1].as_bytes())) }];
        let mut last = 0i64;
        for (t, on, key, vel) in ev {
            let message = if on { MidiMessage::NoteOn { key: u7::new(key), vel: u7::new(vel) } } else { MidiMessage::NoteOff { key: u7::new(key), vel: u7::new(0) } };
            track.push(TrackEvent { delta: u28::new((t - last) as u32), kind: TrackEventKind::Midi { channel, message } });
            last = t;
        }
        track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
        smf.tracks.push(track);
    }
    let mut out = Vec::new();
    smf.write_std(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// A pattern as Standard MIDI File.
pub fn export_pattern(p: &Project, pattern: Id) -> Result<Vec<u8>, String> {
    write_smf(p, &pattern_notes(p, pattern))
}

/// The whole playlist as Standard MIDI File.
pub fn export_song(p: &Project) -> Result<Vec<u8>, String> {
    write_smf_with(p, &song_notes(p), true)
}

/// Result of [`import`].
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub pattern: Id,
    /// indices of the new channels
    pub channels: Vec<usize>,
    pub notes: usize,
    /// tempo found in the file
    pub bpm: Option<f64>,
}

/// Imports a .mid into a new pattern called `name` with one new channel per MIDI track.
pub fn import(p: &mut Project, bytes: &[u8], name: &str, set_tempo: bool) -> Result<Imported, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("keine gültige MIDI-Datei: {e}"))?;
    let ppq = match smf.header.timing {
        Timing::Metrical(t) => t.as_int().max(1) as i64,
        Timing::Timecode(..) => return Err("MIDI-Dateien mit SMPTE-Zeitbasis werden nicht unterstützt".into()),
    };
    let split_channels = smf.header.format == Format::SingleTrack;
    let scale = |t: i64| (t * PPQ + ppq / 2) / ppq;
    let mut bpm = None;
    // group key (track, midi channel or 0) -> (name, notes as (start, len, key, vel), all on ch 10?)
    let mut groups: BTreeMap<(usize, u8), (Option<String>, Vec<(i64, i64, u8, f32)>, bool)> = BTreeMap::new();
    for (ti, track) in smf.tracks.iter().enumerate() {
        let mut t = 0i64;
        let mut track_name: Option<String> = None;
        let mut open: HashMap<(u8, u8), Vec<(i64, f32)>> = HashMap::new();
        let mut done: Vec<(u8, i64, i64, u8, f32)> = Vec::new();
        for ev in track {
            t += ev.delta.as_int() as i64;
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(us)) if bpm.is_none() => bpm = Some((6_000_000_000.0 / us.as_int().max(1) as f64).round() / 100.0),
                TrackEventKind::Meta(MetaMessage::TrackName(n)) if track_name.is_none() => {
                    let s = String::from_utf8_lossy(n).trim().to_string();
                    if !s.is_empty() {
                        track_name = Some(s);
                    }
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => open.entry((ch, key.as_int())).or_default().push((t, vel.as_int() as f32 / 127.0)),
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some(v) = open.get_mut(&(ch, key.as_int())) {
                                if !v.is_empty() {
                                    let (s, vel) = v.remove(0);
                                    done.push((ch, s, t, key.as_int(), vel));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // notes still held at the end of the track end there
        for ((ch, key), v) in open {
            for (s, vel) in v {
                done.push((ch, s, t.max(s + 1), key, vel));
            }
        }
        for (ch, s, e, key, vel) in done {
            let g = groups.entry((ti, if split_channels { ch } else { 0 })).or_insert_with(|| (track_name.clone(), Vec::new(), true));
            g.1.push((scale(s), (scale(e) - scale(s)).max(1), key, vel));
            g.2 &= ch == 9;
        }
    }
    if groups.is_empty() {
        return Err("die MIDI-Datei enthält keine Noten".into());
    }
    if set_tempo {
        if let Some(b) = bpm {
            p.bpm = b;
        }
    }
    let pattern = p.alloc_id();
    let mut notes = Vec::new();
    let mut channels = Vec::new();
    for (k, ((_, ch), (tname, list, drums))) in groups.into_iter().enumerate() {
        let mut cname = tname.unwrap_or_else(|| format!("{name} {}", k + 1));
        if split_channels {
            cname = format!("{cname} Ch{}", ch + 1);
        }
        if drums {
            cname.push_str(" (Drums)");
        }
        let ci = p.add_channel(&cname, Instrument::Synth(Default::default()));
        let cid = p.channels[ci].id;
        channels.push(ci);
        notes.extend(list.into_iter().map(|(start, len, key, velocity)| Note { channel: cid, key, start, len, velocity }));
    }
    notes.sort_by_key(|n| (n.start, n.key));
    let count = notes.len();
    let color = crate::palette_color(p.patterns.len() + 3);
    p.patterns.push(Pattern { id: pattern, name: name.to_string(), color, notes, length: None });
    Ok(Imported { pattern, channels, notes: count, bpm })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipKind, SoundFontInstrument};

    fn demo() -> (Project, Id) {
        let mut p = Project::new_default();
        p.bpm = 150.0;
        let pat = p.patterns[0].id;
        let bass = p.add_channel("Bass", Instrument::Synth(Default::default()));
        let (c0, c1) = (p.channels[0].id, p.channels[bass].id);
        let n = |channel, key, start, len| Note { channel, key, start, len, velocity: 0.5 };
        p.pattern_mut(pat).unwrap().notes = vec![n(c0, 36, 0, 24), n(c0, 36, 96, 24), n(c1, 45, 48, 48), n(c1, 48, 48, 48)];
        (p, pat)
    }

    #[test]
    fn pattern_roundtrip() {
        let (p, pat) = demo();
        let bytes = export_pattern(&p, pat).unwrap();
        assert_eq!(&bytes[..4], b"MThd");
        let mut q = Project::new_default();
        let r = import(&mut q, &bytes, "Import", true).unwrap();
        assert_eq!(r.bpm, Some(150.0));
        assert_eq!(q.bpm, 150.0);
        assert_eq!(r.channels.len(), 2);
        assert_eq!(r.notes, 4);
        assert_eq!(q.channels[r.channels[0]].name, "TSynth");
        assert_eq!(q.channels[r.channels[1]].name, "Bass");
        let got = q.pattern(r.pattern).unwrap();
        let bass_id = q.channels[r.channels[1]].id;
        let mut bass: Vec<(u8, i64, i64)> = got.notes.iter().filter(|n| n.channel == bass_id).map(|n| (n.key, n.start, n.len)).collect();
        bass.sort();
        assert_eq!(bass, vec![(45, 48, 48), (48, 48, 48)]);
        assert!(got.notes.iter().all(|n| (n.velocity - 0.5).abs() < 0.01));
        // tempo left alone when not wanted
        let mut z = Project::new_default();
        import(&mut z, &bytes, "x", false).unwrap();
        assert_eq!(z.bpm, 140.0);
    }

    #[test]
    fn song_repeats_patterns_and_skips_muted() {
        let (mut p, pat) = demo();
        let bar = p.ticks_per_bar();
        p.add_clip(0, 0, bar * 2, ClipKind::Pattern { pattern: pat });
        let muted = p.add_clip(1, bar * 4, bar, ClipKind::Pattern { pattern: pat });
        p.playlist.clips.iter_mut().find(|c| c.id == muted).unwrap().muted = true;
        let notes = song_notes(&p);
        assert_eq!(notes.len(), 8, "two repeats of four notes");
        assert_eq!(notes.last().unwrap().1.start, bar + 96);
        let bytes = export_song(&p).unwrap();
        let mut q = Project::new_default();
        let r = import(&mut q, &bytes, "Song", true).unwrap();
        assert_eq!(r.notes, 8);
        assert!(write_smf(&p, &[]).is_err());
    }

    #[test]
    fn song_export_writes_tempo_track_and_meter_changes() {
        let (mut p, pat) = demo();
        let bar = p.ticks_per_bar();
        p.add_clip(0, 0, bar * 4, ClipKind::Pattern { pattern: pat });
        p.set_tempo_point(bar, 160.0, false);
        p.set_tempo_point(bar * 2, 170.0, true);
        p.set_meter(2, 7, 8);
        let bytes = export_song(&p).unwrap();
        let smf = Smf::parse(&bytes).unwrap();
        let (mut t, mut tempos, mut sigs) = (0i64, Vec::new(), Vec::new());
        for ev in &smf.tracks[0] {
            t += ev.delta.as_int() as i64;
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(us)) => tempos.push((t, (60_000_000.0 / us.as_int() as f64).round() as i64)),
                TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, ..)) => sigs.push((t, n, d)),
                _ => {}
            }
        }
        assert_eq!(tempos[0], (0, 150));
        assert!(tempos.contains(&(bar, 160)));
        assert_eq!(*tempos.last().unwrap(), (bar * 2, 170));
        // ramp from bar 1 to bar 2 is stepped once per beat
        assert_eq!(tempos.iter().filter(|(x, _)| *x > bar && *x < bar * 2).count(), 3);
        assert_eq!(sigs, vec![(0, 4, 2), (bar * 2, 7, 3)]);
        // a pattern export keeps only the start tempo
        let pb = Smf::parse(&export_pattern(&p, pat).unwrap()).unwrap().tracks[0].iter().filter(|e| matches!(e.kind, TrackEventKind::Meta(MetaMessage::Tempo(_)))).count();
        assert_eq!(pb, 1);
    }

    #[test]
    fn drum_kits_use_channel_ten_and_import_scales_ppq() {
        let (mut p, _) = demo();
        let sf = p.add_channel("Kit", Instrument::SoundFont(SoundFontInstrument { bank: 128, ..Default::default() }));
        let map = midi_channels(&p, &[0, sf, 1]);
        assert_eq!((map[&0], map[&sf], map[&1]), (0, 9, 1));

        // hand-made format-0 file, 480 PPQ, two MIDI channels, running note-on vel 0 as off
        let mut smf = Smf::new(Header::new(Format::SingleTrack, Timing::Metrical(u15::new(480))));
        let on = |d: u32, ch: u8, key: u8, vel: u8| TrackEvent { delta: u28::new(d), kind: TrackEventKind::Midi { channel: u4::new(ch), message: MidiMessage::NoteOn { key: u7::new(key), vel: u7::new(vel) } } };
        smf.tracks.push(vec![
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500_000))) },
            on(0, 9, 36, 100),
            on(0, 0, 60, 64),
            on(240, 9, 36, 0),
            on(720, 0, 60, 0),
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
        ]);
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        let mut q = Project::new_default();
        let r = import(&mut q, &bytes, "Clip", true).unwrap();
        assert_eq!(r.bpm, Some(120.0));
        assert_eq!(r.channels.len(), 2);
        let names: Vec<&str> = r.channels.iter().map(|c| q.channels[*c].name.as_str()).collect();
        assert_eq!(names, vec!["Clip 1 Ch1", "Clip 2 Ch10 (Drums)"]);
        let notes = &q.pattern(r.pattern).unwrap().notes;
        let lens: Vec<(u8, i64)> = notes.iter().map(|n| (n.key, n.len)).collect();
        // 240/480 beat = PPQ/2, 960/480 = 2 beats
        assert!(lens.contains(&(36, PPQ / 2)) && lens.contains(&(60, PPQ * 2)));
        assert!(import(&mut q, b"not midi", "x", true).is_err());
    }
}
