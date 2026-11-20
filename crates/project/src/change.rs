//! Describes what changed between two project states, for undo labels and for merging
//! continuous edits (fader nudges, mouse-wheel, typing) into one undo step.

use crate::{MixerTrack, Project};

/// A recognised edit: `label` for the status bar, `key` identifies a continuous control —
/// consecutive commits with the same key are merged by [`crate::History::commit_with`].
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub label: String,
    pub key: Option<String>,
}

impl Change {
    fn step(label: impl Into<String>) -> Self {
        Self { label: label.into(), key: None }
    }
    fn cont(label: impl Into<String>, key: String) -> Self {
        Self { label: label.into(), key: Some(key) }
    }
}

/// Describes a change that only touches the mixer; `None` for anything else (or no change).
pub fn describe_change(before: &Project, after: &Project) -> Option<Change> {
    if before.mixer == after.mixer {
        return None;
    }
    // other parts changed too (e.g. channel routing when an insert is deleted): plain step
    let mut a = before.clone();
    a.mixer = after.mixer.clone();
    if a != *after {
        return Some(Change::step("Mixer"));
    }
    describe_mixer(&before.mixer, &after.mixer)
}

fn track_name(i: usize) -> String {
    if i == 0 {
        "Master".into()
    } else {
        format!("Insert {i}")
    }
}

fn describe_mixer(before: &[MixerTrack], after: &[MixerTrack]) -> Option<Change> {
    if before.len() != after.len() {
        let label = if after.len() > before.len() { "Mixer: Insert hinzugefügt" } else { "Mixer: Insert gelöscht" };
        return Some(Change::step(label));
    }
    let changed: Vec<usize> = (0..before.len()).filter(|&i| before[i] != after[i]).collect();
    match changed.as_slice() {
        [] => None,
        [i] => Some(describe_track(*i, &before[*i], &after[*i])),
        _ if changed.iter().all(|&i| before[i].solo != after[i].solo) => Some(Change::step("Mixer: Solo")),
        _ => Some(Change::step("Mixer: mehrere Spuren")),
    }
}

fn describe_track(i: usize, a: &MixerTrack, b: &MixerTrack) -> Change {
    let n = track_name(i);
    let key = |what: &str| format!("mixer:{i}:{what}");
    if a.fx != b.fx {
        return describe_fx(i, &n, a, b);
    }
    if a.sends != b.sends {
        if a.sends.len() != b.sends.len() {
            let what = if b.sends.len() > a.sends.len() { "Send hinzugefügt" } else { "Send entfernt" };
            return Change::step(format!("{n}: {what}"));
        }
        let j = (0..a.sends.len()).find(|&j| a.sends[j] != b.sends[j]).unwrap_or(0);
        if a.sends[j].target == b.sends[j].target {
            return Change::cont(format!("{n}: Send-Pegel"), key(&format!("send:{j}")));
        }
        return Change::step(format!("{n}: Send-Ziel"));
    }
    let mut parts = Vec::new();
    if a.volume != b.volume {
        parts.push(("Lautstärke", Some("volume")));
    }
    if a.pan != b.pan {
        parts.push(("Panorama", Some("pan")));
    }
    if a.name != b.name {
        parts.push(("Name", Some("name")));
    }
    if a.muted != b.muted {
        parts.push(("Mute", None));
    }
    if a.solo != b.solo {
        parts.push(("Solo", None));
    }
    if a.output != b.output {
        parts.push(("Ausgang", None));
    }
    match parts.as_slice() {
        [(label, Some(k))] => Change::cont(format!("{n}: {label}"), key(k)),
        [(label, None)] => Change::step(format!("{n}: {label}")),
        _ => Change::step(format!("{n}: {}", parts.iter().map(|p| p.0).collect::<Vec<_>>().join(", "))),
    }
}

fn describe_fx(i: usize, n: &str, a: &MixerTrack, b: &MixerTrack) -> Change {
    if a.fx.len() != b.fx.len() {
        return if b.fx.len() > a.fx.len() {
            let added = b.fx.iter().find(|f| !a.fx.contains(f)).or(b.fx.last());
            Change::step(format!("{n}: Effekt hinzugefügt ({})", added.map(|f| f.params.kind().name()).unwrap_or("?")))
        } else {
            let removed = a.fx.iter().find(|f| !b.fx.contains(f));
            Change::step(format!("{n}: Effekt entfernt ({})", removed.map(|f| f.params.kind().name()).unwrap_or("?")))
        };
    }
    let diff: Vec<usize> = (0..a.fx.len()).filter(|&j| a.fx[j] != b.fx[j]).collect();
    if diff.len() != 1 {
        return Change::step(format!("{n}: Effekte verschoben"));
    }
    let j = diff[0];
    let (fa, fb) = (&a.fx[j], &b.fx[j]);
    let kind = fb.params.kind();
    if fa.params.kind() != kind {
        return Change::step(format!("{n}: Effekt ersetzt"));
    }
    if fa.enabled != fb.enabled {
        return Change::step(format!("{n}: {} {}", kind.name(), if fb.enabled { "an" } else { "aus" }));
    }
    let names = fb.params.field_names();
    let fields: Vec<usize> = (0..names.len()).filter(|&f| fa.params.field(f) != fb.params.field(f)).collect();
    match fields.as_slice() {
        [f] if only_field_differs(fa, fb, *f) => {
            Change::cont(format!("{n}: {} {}", kind.name(), names[*f]), format!("mixer:{i}:fx:{j}:{f}"))
        }
        _ => Change::step(format!("{n}: {}-Einstellung", kind.name())),
    }
}

/// True when the two slots differ in field `f` only (not in a non-numeric setting like a mode).
fn only_field_differs(a: &crate::FxSlot, b: &crate::FxSlot, f: usize) -> bool {
    let mut x = a.params.clone();
    match (x.field_mut(f), b.params.field(f)) {
        (Some(v), Some(w)) => *v = w,
        _ => return false,
    }
    x == b.params
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FxSlot, MixerSend};
    use tstudio_dsp::fx::{EffectKind, EffectParams};

    fn pair() -> (Project, Project) {
        let p = Project::new_default();
        (p.clone(), p)
    }

    #[test]
    fn non_mixer_changes_are_not_described() {
        let (a, mut b) = pair();
        assert_eq!(describe_change(&a, &b), None);
        b.bpm = 150.0;
        assert_eq!(describe_change(&a, &b), None);
    }

    #[test]
    fn fader_pan_and_toggles() {
        let (a, mut b) = pair();
        b.mixer[1].volume = 0.3;
        let c = describe_change(&a, &b).unwrap();
        assert_eq!(c.label, "Insert 1: Lautstärke");
        assert_eq!(c.key.as_deref(), Some("mixer:1:volume"));
        let (a, mut b) = pair();
        b.mixer[0].pan = 0.2;
        assert_eq!(describe_change(&a, &b).unwrap().key.as_deref(), Some("mixer:0:pan"));
        let (a, mut b) = pair();
        b.mixer[2].muted = true;
        let c = describe_change(&a, &b).unwrap();
        assert_eq!((c.label.as_str(), c.key), ("Insert 2: Mute", None));
        let (a, mut b) = pair();
        b.mixer[1].solo = true;
        b.mixer[2].solo = true;
        assert_eq!(describe_change(&a, &b).unwrap().label, "Mixer: Solo");
    }

    #[test]
    fn fx_add_remove_and_params() {
        let (a, mut b) = pair();
        b.mixer[1].fx.push(FxSlot::new(EffectKind::Reverb));
        let c = describe_change(&a, &b).unwrap();
        assert_eq!(c.label, "Insert 1: Effekt hinzugefügt (Reverb)");
        assert!(c.key.is_none());
        let before = b.clone();
        if let EffectParams::Reverb(r) = &mut b.mixer[1].fx[0].params {
            r.mix = 0.6;
        }
        let c = describe_change(&before, &b).unwrap();
        assert_eq!(c.label, "Insert 1: Reverb mix");
        assert_eq!(c.key.as_deref(), Some("mixer:1:fx:0:4"));
        let mut off = b.clone();
        off.mixer[1].fx[0].enabled = false;
        assert_eq!(describe_change(&b, &off).unwrap().label, "Insert 1: Reverb aus");
        let mut gone = b.clone();
        gone.mixer[1].fx.clear();
        assert_eq!(describe_change(&b, &gone).unwrap().label, "Insert 1: Effekt entfernt (Reverb)");
        // a non-numeric setting (filter mode) is a discrete step
        let mut f = b.clone();
        f.mixer[1].fx.push(FxSlot::new(EffectKind::Filter));
        let mut g = f.clone();
        if let EffectParams::Filter(p) = &mut g.mixer[1].fx[1].params {
            p.mode = tstudio_dsp::filter::FilterMode::HighPass;
        }
        let c = describe_change(&f, &g).unwrap();
        assert_eq!((c.label.as_str(), c.key), ("Insert 1: Filter-Einstellung", None));
    }

    #[test]
    fn routing_and_sends() {
        let (a, mut b) = pair();
        b.mixer[1].output = 3;
        assert_eq!(describe_change(&a, &b).unwrap().label, "Insert 1: Ausgang");
        let (a, mut b) = pair();
        b.mixer[1].sends.push(MixerSend { target: 2, amount: 0.5 });
        assert_eq!(describe_change(&a, &b).unwrap().label, "Insert 1: Send hinzugefügt");
        let before = b.clone();
        b.mixer[1].sends[0].amount = 0.25;
        let c = describe_change(&before, &b).unwrap();
        assert_eq!(c.key.as_deref(), Some("mixer:1:send:0"));
        let (a, mut b) = pair();
        b.add_mixer_track(None);
        assert_eq!(describe_change(&a, &b).unwrap().label, "Mixer: Insert hinzugefügt");
    }
}
