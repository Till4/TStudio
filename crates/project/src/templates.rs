//! Project templates: a few built-in starting points (built in code from TSynth presets, so
//! they need no sample files) and user templates stored as `.tsp` files in a templates folder.

use crate::{FxSlot, Instrument, Note, Pattern, Project, PPQ};
use std::path::{Path, PathBuf};
use tstudio_dsp::fx::EffectKind;
use tstudio_dsp::synth::SynthParams;

/// Extension of template files (they are ordinary projects).
pub const TEMPLATE_EXT: &str = "tsp";

/// A channel of a built-in template: name, TSynth preset, step pattern (16th steps of one bar),
/// key, and the effects on its mixer insert.
struct ChannelSpec {
    name: &'static str,
    preset: &'static str,
    steps: &'static [u32],
    key: u8,
    fx: &'static [EffectKind],
}

struct Builtin {
    name: &'static str,
    bpm: f64,
    swing: f32,
    channels: &'static [ChannelSpec],
}

const FOUR_ON_FLOOR: &[u32] = &[0, 4, 8, 12];
const OFFBEATS: &[u32] = &[2, 6, 10, 14];

const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "House 124",
        bpm: 124.0,
        swing: 0.1,
        channels: &[
            ChannelSpec { name: "Kick", preset: "Punch Kick", steps: FOUR_ON_FLOOR, key: 36, fx: &[] },
            ChannelSpec { name: "Bass", preset: "Offbeat Bass", steps: OFFBEATS, key: 45, fx: &[EffectKind::Compressor] },
            ChannelSpec { name: "Chords", preset: "Chord Stab", steps: &[3, 10], key: 57, fx: &[EffectKind::Delay, EffectKind::Reverb] },
            ChannelSpec { name: "Pad", preset: "Pad", steps: &[], key: 60, fx: &[EffectKind::Reverb] },
        ],
    },
    Builtin {
        name: "Techno 140",
        bpm: 140.0,
        swing: 0.0,
        channels: &[
            ChannelSpec { name: "Kick", preset: "Hard Kick", steps: FOUR_ON_FLOOR, key: 36, fx: &[] },
            ChannelSpec { name: "Rumble", preset: "Rumble Kick", steps: OFFBEATS, key: 36, fx: &[EffectKind::Reverb, EffectKind::Filter] },
            ChannelSpec { name: "Acid", preset: "Acid", steps: &[0, 3, 6, 8, 11, 14], key: 45, fx: &[EffectKind::Distortion, EffectKind::Delay] },
            ChannelSpec { name: "Stab", preset: "Hard Stab", steps: &[], key: 57, fx: &[EffectKind::Reverb] },
        ],
    },
    Builtin {
        name: "Schranz 155",
        bpm: 155.0,
        swing: 0.0,
        channels: &[
            ChannelSpec { name: "Kick", preset: "Schranz Kick", steps: FOUR_ON_FLOOR, key: 36, fx: &[EffectKind::Clipper] },
            ChannelSpec { name: "Rumble", preset: "Rumble Kick", steps: OFFBEATS, key: 36, fx: &[EffectKind::Distortion, EffectKind::Reverb] },
            ChannelSpec { name: "Stab", preset: "Schranz Stab", steps: &[2, 7, 10, 15], key: 48, fx: &[EffectKind::Distortion] },
            ChannelSpec { name: "Industrial", preset: "Industrial Stab", steps: &[], key: 48, fx: &[EffectKind::Reverb] },
            ChannelSpec { name: "FX", preset: "Riser", steps: &[], key: 60, fx: &[EffectKind::Reverb] },
        ],
    },
];

/// Names of the built-in templates (menu order).
pub fn builtin_names() -> Vec<&'static str> {
    BUILTINS.iter().map(|b| b.name).collect()
}

fn synth_preset(name: &str) -> SynthParams {
    tstudio_dsp::synth_presets::all().into_iter().find(|(_, n, _)| *n == name).map(|(_, _, p)| p).unwrap_or_default()
}

/// Builds the built-in template `name` (case-insensitive).
pub fn builtin(name: &str) -> Option<Project> {
    let b = BUILTINS.iter().find(|b| b.name.eq_ignore_ascii_case(name))?;
    let mut p = Project::default();
    p.name = b.name.to_string();
    p.bpm = b.bpm;
    p.swing = b.swing;
    let pat = p.alloc_id();
    let step = PPQ / 4;
    let mut notes = Vec::new();
    for c in b.channels {
        let i = p.add_channel(c.name, Instrument::Synth(synth_preset(c.preset)));
        let ch = &p.channels[i];
        let (cid, track) = (ch.id, ch.mixer_track);
        p.mixer[track].fx.extend(c.fx.iter().map(|k| FxSlot::new(*k)));
        notes.extend(c.steps.iter().map(|s| Note { channel: cid, key: c.key, start: *s as i64 * step, len: step, velocity: 0.8 }));
    }
    p.patterns.push(Pattern { id: pat, name: "Beat".into(), notes, ..Default::default() });
    Some(p)
}

/// A new, unsaved project made from a template.
pub fn instantiate(mut template: Project) -> Project {
    template.name = "Untitled".into();
    template.sanitize();
    template
}

/// File name for a user template (characters Windows does not allow become `_`).
pub fn template_file_name(name: &str) -> String {
    let clean: String = name.trim().chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim_matches(|c: char| c == '.' || c.is_whitespace());
    format!("{}.{TEMPLATE_EXT}", if clean.is_empty() { "Vorlage" } else { clean })
}

/// User templates in `dir`: (name, path), sorted by name.
pub fn list_user_templates(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case(TEMPLATE_EXT)).unwrap_or(false))
                .filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_string(), p)))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    v
}

/// Saves `project` as user template `name` in `dir` (overwrites one with the same name).
pub fn save_user_template(dir: &Path, name: &str, project: &Project) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(template_file_name(name));
    let mut t = project.clone();
    t.name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    t.save(&path)?;
    Ok(path)
}

/// Looks a template up by name: user templates in `dir` first, then the built-in ones.
pub fn find(dir: &Path, name: &str) -> Option<Project> {
    list_user_templates(dir)
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .and_then(|(_, p)| Project::load(&p).ok())
        .or_else(|| builtin(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_have_channels_tempo_and_beat() {
        assert_eq!(builtin_names(), vec!["House 124", "Techno 140", "Schranz 155"]);
        for name in builtin_names() {
            let p = builtin(name).unwrap();
            assert!(p.channels.len() >= 4, "{name}");
            assert_eq!(p.patterns.len(), 1);
            // the kick plays four on the floor
            let kick = p.channels[0].id;
            assert_eq!(p.patterns[0].notes.iter().filter(|n| n.channel == kick).count(), 4);
            // every channel has its own insert
            let mut tracks: Vec<usize> = p.channels.iter().map(|c| c.mixer_track).collect();
            tracks.dedup();
            assert_eq!(tracks.len(), p.channels.len());
            assert!(tracks.iter().all(|t| *t > 0 && *t < p.mixer.len()));
            // survives a save/load round trip
            assert_eq!(Project::from_ron(&p.to_ron().unwrap()).unwrap(), p);
        }
        assert_eq!(builtin("schranz 155").unwrap().bpm, 155.0);
        assert!(builtin("Polka").is_none());
        // presets are found (not just Init)
        let s = builtin("Schranz 155").unwrap();
        assert!(matches!(&s.channels[0].instrument, Instrument::Synth(p) if p.kick.enabled));
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(template_file_name("My: Set/Up?"), "My_ Set_Up_.tsp");
        assert_eq!(template_file_name("  "), "Vorlage.tsp");
        assert_eq!(template_file_name("..x.."), "x.tsp");
    }

    #[test]
    fn user_templates_save_list_find() {
        let dir = std::env::temp_dir().join(format!("tstudio_tpl_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(list_user_templates(&dir).is_empty());
        let mut p = builtin("House 124").unwrap();
        p.bpm = 128.0;
        let path = save_user_template(&dir, "Mein House", &p).unwrap();
        assert!(path.ends_with("Mein House.tsp"));
        save_user_template(&dir, "Alpha", &Project::new_default()).unwrap();
        let names: Vec<String> = list_user_templates(&dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["Alpha", "Mein House"]);
        let t = find(&dir, "mein house").unwrap();
        assert_eq!(t.bpm, 128.0);
        let fresh = instantiate(t);
        assert_eq!(fresh.name, "Untitled");
        assert_eq!(find(&dir, "Techno 140").unwrap().bpm, 140.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
