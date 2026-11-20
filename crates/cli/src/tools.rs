//! Composition / mixing helpers behind the newer CLI commands: chord-based note generation,
//! song sections (markers, copy/insert/delete bars), per-track stem renders, mix report and
//! effect-chain presets.

use crate::audio;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tstudio_dsp::fx::{DistMode, DistParams, EffectParams, EqParams, FilterFxParams, LimiterParams, ReverbParams, SidechainParams};
use tstudio_dsp::fx::{ClipperParams, MultibandParams, OttParams, WidthParams};
use tstudio_dsp::music::pitch_class;
use tstudio_engine::PlayMode;
use tstudio_project::{FxSlot, Project, PPQ};

type R<T> = Result<T, String>;

// ------------------------------------------------------------------ chords

/// Parses "Cm", "F#", "Bb7", "Gsus4", "Am7", "Dmaj7", "E5", "Bdim" → (root pitch class, intervals).
pub fn parse_chord(s: &str) -> R<(u8, Vec<i32>)> {
    let s = s.trim();
    let mut split = 1;
    let b = s.as_bytes();
    if b.len() > 1 && (b[1] == b'#' || b[1] == b'b') {
        // "Bb" = B flat; but "b" alone after the root could also be part of nothing else
        split = 2;
    }
    if s.len() < split {
        return Err(format!("Akkord '{s}' unverständlich"));
    }
    let root = pitch_class(&s[..split]).ok_or_else(|| format!("Grundton in '{s}' unbekannt"))?;
    let q = &s[split..];
    let iv: Vec<i32> = match q {
        "" | "maj" | "M" => vec![0, 4, 7],
        "m" | "min" | "-" => vec![0, 3, 7],
        "7" => vec![0, 4, 7, 10],
        "m7" | "min7" | "-7" => vec![0, 3, 7, 10],
        "maj7" | "M7" => vec![0, 4, 7, 11],
        "dim" | "o" => vec![0, 3, 6],
        "aug" | "+" => vec![0, 4, 8],
        "sus2" => vec![0, 2, 7],
        "sus4" | "sus" => vec![0, 5, 7],
        "5" => vec![0, 7],
        "add9" => vec![0, 4, 7, 14],
        "m9" => vec![0, 3, 7, 10, 14],
        _ => return Err(format!("Akkordtyp '{q}' in '{s}' unbekannt (m, 7, m7, maj7, dim, aug, sus2, sus4, 5, add9, m9)")),
    };
    Ok((root, iv))
}

pub struct GenNote {
    pub key: u8,
    pub start: i64,
    pub len: i64,
    pub vel: f32,
}

/// Default 16th-step rhythms per style ('x' = hit, '.' = rest, '-' = hold previous).
fn style_rhythm(style: &str) -> &'static str {
    match style {
        "bass" => "x...x...x...x...",
        "offbeat" => "..x...x...x...x.",
        "rolling" => ".xxx.xxx.xxx.xxx",
        "stab" => "..x...x...x...x.",
        "arp" => "xxxxxxxxxxxxxxxx",
        _ => "x---------------",
    }
}

/// Notes for a chord progression. `prog`: "Cm Ab Eb Bb" or with lengths in bars "Cm:2 Ab Eb:0.5".
pub fn chord_notes(prog: &str, bars_per_chord: f64, style: &str, octave: i32, vel: f32, rhythm: Option<&str>, ticks_per_bar: i64, inversions: bool) -> R<(Vec<GenNote>, Vec<String>)> {
    let style = style.to_lowercase();
    let known = ["block", "pad", "bass", "offbeat", "rolling", "stab", "arp"];
    if !known.contains(&style.as_str()) {
        return Err(format!("Stil '{style}' unbekannt ({})", known.join(", ")));
    }
    let rh: Vec<char> = rhythm.unwrap_or_else(|| style_rhythm(&style)).chars().filter(|c| !c.is_whitespace() && *c != '|').collect();
    if rh.is_empty() {
        return Err("leerer Rhythmus".into());
    }
    let step = PPQ / 4;
    let mut out = Vec::new();
    let mut names = Vec::new();
    let mut t = 0i64;
    let mut prev_center: Option<f64> = None;
    for tok in prog.split([' ', ',', ';', '|']).filter(|s| !s.trim().is_empty()) {
        let (name, bars) = match tok.split_once(':') {
            Some((n, b)) => (n, b.parse::<f64>().map_err(|_| format!("Länge in '{tok}' unverständlich"))?),
            None => (tok, bars_per_chord),
        };
        let len = (bars * ticks_per_bar as f64).round() as i64;
        if len <= 0 {
            return Err(format!("Länge von '{tok}' ist 0"));
        }
        names.push(name.to_string());
        if name == "N" || name == "-" {
            t += len;
            continue;
        }
        let (root, iv) = parse_chord(name)?;
        let base = 12 * (octave + 1) + root as i32;
        let mut keys: Vec<i32> = iv.iter().map(|i| base + i).collect();
        // voice leading: invert so the chord stays close to the previous one
        if inversions && matches!(style.as_str(), "block" | "pad" | "stab" | "arp") {
            if let Some(pc) = prev_center {
                for _ in 0..3 {
                    let c = keys.iter().sum::<i32>() as f64 / keys.len() as f64;
                    if c - pc > 4.0 {
                        let top = keys.iter().copied().max().unwrap();
                        keys.retain(|k| *k != top);
                        keys.insert(0, top - 12);
                    } else if pc - c > 4.0 {
                        let low = keys.iter().copied().min().unwrap();
                        keys.retain(|k| *k != low);
                        keys.push(low + 12);
                    } else {
                        break;
                    }
                }
            }
            prev_center = Some(keys.iter().sum::<i32>() as f64 / keys.len() as f64);
        }
        let bass_like = matches!(style.as_str(), "bass" | "offbeat" | "rolling");
        if rhythm.is_none() && matches!(style.as_str(), "block" | "pad") {
            // held for the whole chord
            for k in &keys {
                out.push(GenNote { key: (*k).clamp(0, 127) as u8, start: t, len, vel });
            }
            t += len;
            continue;
        }
        let mut arp_i = 0usize;
        let mut s = 0i64;
        while s < len {
            let idx = ((s / step) as usize) % rh.len();
            let c = rh[idx];
            if c == 'x' || c == 'X' || c == 'o' {
                // length: this step plus following '-' holds
                let mut l = step;
                let mut k = 1;
                while rh[(idx + k) % rh.len()] == '-' && s + l < len && k < rh.len() {
                    l += step;
                    k += 1;
                }
                let v = match c {
                    'X' => 1.0,
                    'o' => vel * 0.55,
                    _ => vel,
                };
                let hit: Vec<i32> = if bass_like {
                    vec![base]
                } else if style == "arp" {
                    let mut sorted = keys.clone();
                    sorted.sort();
                    let k = sorted[arp_i % sorted.len()];
                    arp_i += 1;
                    vec![k]
                } else {
                    keys.clone()
                };
                for k in hit {
                    out.push(GenNote { key: k.clamp(0, 127) as u8, start: t + s, len: l.min(len - s), vel: v });
                }
            }
            s += step;
        }
        t += len;
    }
    Ok((out, names))
}

// ------------------------------------------------------------------ sections

/// Bar (1-based, may be fractional) or marker name → tick.
pub fn bar_tick(p: &Project, s: &str) -> R<i64> {
    let s = s.trim();
    if let Ok(b) = s.parse::<f64>() {
        if b < 1.0 {
            return Err(format!("Takt {b}: der Song beginnt bei Takt 1"));
        }
        return Ok(p.bar_number_to_tick(b));
    }
    p.playlist.marker(s).map(|m| m.tick).ok_or_else(|| {
        format!(
            "'{s}' ist weder Taktnummer noch Marker (Marker: {})",
            if p.playlist.markers.is_empty() { "keine".to_string() } else { p.playlist.markers.iter().map(|m| m.name.clone()).collect::<Vec<_>>().join(", ") }
        )
    })
}

pub fn tick_bar(p: &Project, t: i64) -> f64 {
    (p.tick_to_bar_number(t) * 1000.0).round() / 1000.0
}

/// Copies every clip that starts in [from, from+len) to `to` (shifted). Returns the number of copies.
pub fn section_copy(p: &mut Project, from: i64, len: i64, to: i64, tracks: Option<&BTreeSet<usize>>) -> usize {
    let src: Vec<_> = p
        .playlist
        .clips
        .iter()
        .filter(|c| c.start >= from && c.start < from + len && tracks.map(|t| t.contains(&c.track)).unwrap_or(true))
        .cloned()
        .collect();
    let n = src.len();
    for mut c in src {
        c.id = p.alloc_id();
        c.start = c.start - from + to;
        // clip may not reach past the copied range
        c.len = c.len.min(from + len - (c.start - to + from)).max(1);
        p.playlist.clips.push(c);
    }
    n
}

/// Removes clips starting in [from, from+len) (and cuts clips reaching into it).
pub fn section_clear(p: &mut Project, from: i64, len: i64, tracks: Option<&BTreeSet<usize>>) -> usize {
    let before = p.playlist.clips.len();
    let end = from + len;
    let sel = |tr: usize| tracks.map(|t| t.contains(&tr)).unwrap_or(true);
    p.playlist.clips.retain(|c| !(sel(c.track) && c.start >= from && c.start < end));
    for c in p.playlist.clips.iter_mut() {
        if sel(c.track) && c.start < from && c.end() > from {
            c.len = from - c.start;
        }
    }
    before - p.playlist.clips.len()
}

/// Shifts everything at or after `at` by `delta` ticks (negative = pull left).
pub fn shift_after(p: &mut Project, at: i64, delta: i64) {
    for c in p.playlist.clips.iter_mut() {
        if c.start >= at {
            c.start += delta;
        }
    }
    for m in p.playlist.markers.iter_mut() {
        if m.tick >= at {
            m.tick += delta;
        }
    }
    // tempo changes move with the music; those inside a removed range go away
    p.tempo.retain(|t| delta >= 0 || t.tick < at + delta || t.tick >= at);
    for t in p.tempo.iter_mut() {
        if t.tick >= at {
            t.tick += delta;
        }
    }
    p.sort_tempo();
    // time-signature changes too (they are stored by bar: re-derive the bar after moving)
    let old = p.bar_map();
    let moved: Vec<(i64, tstudio_project::MeterChange)> = p
        .meters
        .iter()
        .map(|m| (old.bar_to_tick(m.bar as f64).round() as i64, *m))
        .filter(|(t, _)| delta >= 0 || *t < at + delta || *t >= at)
        .map(|(t, m)| (if t >= at { t + delta } else { t }, m))
        .collect();
    p.meters.clear();
    for (t, m) in moved {
        let bar = p.bar_map().tick_to_bar(t as f64).round().max(1.0) as u32;
        p.meters.push(tstudio_project::MeterChange { bar, ..m });
    }
    p.sort_meters();
    if let Some((a, b)) = p.playlist.loop_region.as_mut() {
        if *a >= at {
            *a += delta;
        }
        if *b >= at {
            *b += delta;
        }
    }
}

// ------------------------------------------------------------------ stems / mix

pub use tstudio_engine::Stem;

/// One render per mixer insert that carries sound (buses included, without master FX) –
/// the same engine code as the GUI stem export.
pub fn render_stems(p: &Project, sr: u32, mode: PlayMode, tail: f64) -> R<Vec<Stem>> {
    let mut samples = audio::SyncSamples::new(sr);
    let jobs = tstudio_engine::compile_stems(tstudio_engine::stem_jobs(p), |q| tstudio_engine::compile_project(q, &mut |path, st, bpm| samples.get(path, st, bpm), true));
    tstudio_engine::render_stems(jobs, sr, mode, tail, |_| true).ok_or_else(|| "Rendern abgebrochen".to_string())
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Per-track levels + masking pairs.
pub fn stems_report(stems: &[Stem]) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut tracks = Vec::new();
    let mut warnings = Vec::new();
    let mut loud = Vec::new();
    for s in stems {
        let l = tstudio_dsp::meter::loudness(&s.data);
        let b = tstudio_dsp::meter::bands(&s.data);
        let st = tstudio_dsp::meter::stereo(&s.data);
        let silent = l.integrated_lufs <= -70.0;
        if !silent && st.bass_correlation < 0.85 && b[0].share_db.max(b[1].share_db) > -12.0 {
            warnings.push(format!("{} (Spur {}): Bass nicht mono (Korrelation {:.2})", s.name, s.track, st.bass_correlation));
        }
        if silent {
            tracks.push(json!({"track": s.track, "name": s.name, "bus": s.bus, "silent": true}));
            continue;
        }
        let main = b.iter().max_by(|a, c| a.share_db.total_cmp(&c.share_db)).map(|x| x.name).unwrap_or("");
        tracks.push(json!({
            "track": s.track, "name": s.name, "bus": s.bus, "silent": false,
            "lufs": l.integrated_lufs, "peak_db": l.sample_peak_db, "short_term_max_lufs": l.short_term_max_lufs,
            "main_band": main,
            "bands": b.iter().map(|x| json!({"band": x.name, "share_db": x.share_db})).collect::<Vec<_>>(),
            "bass_correlation": st.bass_correlation,
        }));
        if !silent {
            loud.push((s, l.integrated_lufs, b));
        }
    }
    // masking: pairs that are both strong in the same low band at the same time
    let mut pairs = Vec::new();
    for i in 0..loud.len() {
        for j in i + 1..loud.len() {
            let (a, la, ba) = &loud[i];
            let (b, lb, bb) = &loud[j];
            let strong = |bands: &Vec<tstudio_dsp::meter::Band>, k: usize| bands[k].share_db > -10.0;
            if !(0..3).any(|k| strong(ba, k) && strong(bb, k)) {
                continue;
            }
            for o in tstudio_dsp::meter::overlap(&a.data, &b.data) {
                let k = tstudio_dsp::meter::BANDS.iter().position(|x| x.0 == o.band).unwrap_or(9);
                if k > 2 || !(strong(ba, k) && strong(bb, k)) || o.simultaneous < 0.35 {
                    continue;
                }
                let level_gap = r1((la - lb).abs());
                pairs.push(json!({"a": a.name, "track_a": a.track, "b": b.name, "track_b": b.track, "band": o.band, "simultaneous": o.simultaneous, "lufs_gap": level_gap}));
                if o.simultaneous >= 0.5 {
                    warnings.push(format!(
                        "{} ↔ {}: beide laut in {} ({:.0} % der Zeit gleichzeitig) – Sidechain tiefer/länger oder einer Spur dort EQ-Platz machen",
                        a.name,
                        b.name,
                        o.band,
                        o.simultaneous * 100.0
                    ));
                }
            }
        }
    }
    (tracks, pairs, warnings)
}

// ------------------------------------------------------------------ fx chain presets

pub const CHAINS: [(&str, &str); 8] = [
    ("schranz-kick", "Verzerrung + Clipper + EQ für eine harte, durchsetzungsfähige Kick"),
    ("rumble", "Reverb 100 % + Verzerrung + Tiefpass + Sidechain – auf einen Rumble-Bus (mit --source = Kick-Spur)"),
    ("master", "EQ, Multiband-Sättigung, Clipper, Limiter (-1 dBTP) – laut wie ein Release"),
    ("ott", "OTT-Kompression (30 %) für dichte Leads/Synths"),
    ("vocal", "EQ (Mulm raus, Präsenz rein), Kompressor, OTT leicht, Reverb"),
    ("bass", "Multiband-Sättigung + Kompressor + Sidechain zur Kick (mit --source)"),
    ("wide", "Stereobreite 140 % für Pads/Leads/Hats, Bass unter 150 Hz bleibt mono"),
    ("mono-bass", "Alles unter 120 Hz mono (Master/Bus: club-tauglicher Bass, Breite oben unverändert)"),
];

pub fn chain(name: &str, source: Option<usize>) -> R<Vec<FxSlot>> {
    let slot = |params: EffectParams| FxSlot { enabled: true, params };
    let sc = |depth: f32, release: f32, thr: f32| -> R<FxSlot> {
        let src = source.ok_or("Diese Kette braucht --source = Mixer-Spur der Kick (Sidechain-Auslöser)")?;
        // threshold close to the key's peak: a distorted kick's long tail would otherwise keep the
        // track ducked the whole beat (tested on "Stahlnacht": offbeat bass came out 20 dB too quiet)
        Ok(slot(EffectParams::Sidechain(SidechainParams { source: src, depth, threshold_db: thr, attack_ms: 0.5, release_ms: release })))
    };
    Ok(match name.to_lowercase().as_str() {
        "schranz-kick" | "kick" => vec![
            slot(EffectParams::Distortion(DistParams { mode: DistMode::Soft, drive_db: 15.0, tone: 7500.0, output_db: -9.0, mix: 1.0, ..Default::default() })),
            slot(EffectParams::Eq(EqParams { low_freq: 70.0, low_gain: 2.5, mid_freq: 380.0, mid_gain: -4.0, mid_q: 1.2, high_freq: 6000.0, high_gain: 1.5 })),
            slot(EffectParams::Clipper(ClipperParams { drive_db: 4.0, ceiling_db: -1.0, softness: 0.15, output_db: 0.0, mix: 1.0 })),
        ],
        "rumble" => vec![
            slot(EffectParams::Reverb(ReverbParams { size: 0.85, damping: 0.65, predelay_ms: 0.0, width: 0.2, mix: 1.0 })),
            slot(EffectParams::Distortion(DistParams { mode: DistMode::Soft, drive_db: 20.0, tone: 2500.0, output_db: -12.0, mix: 1.0, ..Default::default() })),
            slot(EffectParams::Filter(FilterFxParams { cutoff: 220.0, resonance: 0.1, ..Default::default() })),
            slot(EffectParams::Eq(EqParams { low_freq: 40.0, low_gain: -6.0, ..Default::default() })),
            sc(1.0, 190.0, -12.0)?,
        ],
        "master" => vec![
            slot(EffectParams::Eq(EqParams { low_freq: 30.0, low_gain: -1.5, mid_freq: 350.0, mid_gain: -1.0, mid_q: 0.9, high_freq: 10000.0, high_gain: 1.0 })),
            slot(EffectParams::Multiband(MultibandParams { low_drive_db: 1.5, mid_drive_db: 2.5, high_drive_db: 1.5, mix: 0.6, ..Default::default() })),
            slot(EffectParams::Clipper(ClipperParams { drive_db: 2.0, ceiling_db: -0.8, softness: 0.3, output_db: 0.0, mix: 1.0 })),
            slot(EffectParams::Limiter(LimiterParams { gain_db: 3.0, ceiling_db: -1.0, release_ms: 60.0 })),
        ],
        "ott" => vec![slot(EffectParams::Ott(OttParams { depth: 0.3, ..Default::default() }))],
        "vocal" => vec![
            slot(EffectParams::Eq(EqParams { low_freq: 120.0, low_gain: -6.0, mid_freq: 300.0, mid_gain: -3.0, mid_q: 1.0, high_freq: 5000.0, high_gain: 3.0 })),
            slot(EffectParams::Compressor(tstudio_dsp::fx::CompParams { threshold: -20.0, ratio: 4.0, attack_ms: 5.0, release_ms: 80.0, knee: 6.0, makeup: 4.0, mix: 1.0 })),
            slot(EffectParams::Ott(OttParams { depth: 0.15, ..Default::default() })),
            slot(EffectParams::Reverb(ReverbParams { size: 0.55, damping: 0.5, predelay_ms: 25.0, width: 1.0, mix: 0.18 })),
        ],
        "bass" => vec![
            slot(EffectParams::Multiband(MultibandParams { low_drive_db: 3.0, mid_drive_db: 10.0, high_drive_db: 4.0, high_gain_db: -3.0, ..Default::default() })),
            slot(EffectParams::Compressor(tstudio_dsp::fx::CompParams { threshold: -16.0, ratio: 4.0, attack_ms: 3.0, release_ms: 60.0, knee: 6.0, makeup: 2.0, mix: 1.0 })),
            sc(0.45, 90.0, -4.0)?,
        ],
        "wide" => vec![slot(EffectParams::Width(WidthParams { width: 1.4, bass_mono_hz: 150.0, ..Default::default() }))],
        "mono-bass" | "bass-mono" => vec![slot(EffectParams::Width(WidthParams { bass_mono_hz: 120.0, ..Default::default() }))],
        other => return Err(format!("Kette '{other}' unbekannt ({})", CHAINS.iter().map(|c| c.0).collect::<Vec<_>>().join(", "))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chain_preset_builds() {
        for (name, _) in CHAINS {
            let slots = chain(name, Some(1)).unwrap();
            assert!(!slots.is_empty(), "{name}");
        }
        let wide = chain("wide", None).unwrap();
        assert!(matches!(&wide[0].params, EffectParams::Width(w) if w.width > 1.0 && w.bass_mono_hz > 0.0));
    }

    #[test]
    fn chords_parse() {
        assert_eq!(parse_chord("Cm").unwrap(), (0, vec![0, 3, 7]));
        assert_eq!(parse_chord("Bb").unwrap(), (10, vec![0, 4, 7]));
        assert_eq!(parse_chord("F#m7").unwrap(), (6, vec![0, 3, 7, 10]));
        assert!(parse_chord("Xm").is_err());
    }

    #[test]
    fn progression_lengths() {
        let (n, names) = chord_notes("Cm Ab:2 Eb", 1.0, "block", 4, 0.8, None, 384, false).unwrap();
        assert_eq!(names.len(), 3);
        assert_eq!(n.len(), 9);
        assert_eq!(n[3].start, 384);
        assert_eq!(n[3].len, 768);
        let (b, _) = chord_notes("Cm", 1.0, "offbeat", 2, 0.8, None, 384, false).unwrap();
        assert_eq!(b.len(), 4);
        assert_eq!(b[0].key, 36);
        assert_eq!(b[0].start, 2 * PPQ / 4);
    }
}
