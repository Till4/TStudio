use super::*;

#[test]
fn files_without_the_newer_fields_load() {
    // projects from before tempo track, time-signature changes and take lanes
    let mut p = Project::new_default();
    p.playlist.tracks.push(PlaylistTrack { name: "Vocals".into(), ..Default::default() });
    let s = p.to_ron().unwrap();
    let old: String = s
        .lines()
        .filter(|l| !["tempo:", "meters:", "take_group:"].iter().any(|k| l.trim_start().starts_with(k)))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!old.contains("take_group") && !old.contains("meters"));
    let q = Project::from_ron(&old).unwrap();
    assert!(q.tempo.is_empty() && q.meters.is_empty());
    assert!(q.playlist.tracks.iter().all(|t| t.take_group.is_none()));
    assert_eq!(q.playlist.tracks.len(), p.playlist.tracks.len());
}
use tstudio_dsp::fx::EffectKind;

#[test]
fn roundtrip_ron() {
    let mut p = Project::new_default();
    while p.mixer.len() < 17 {
        p.add_mixer_track(None);
    }
    let ch = p.add_channel("Kick", Instrument::Sampler(SamplerInstrument { sample: Some("C:/s/kick.wav".into()), ..Default::default() }));
    let cid = p.channels[ch].id;
    let pat = p.patterns[0].id;
    p.pattern_mut(pat).unwrap().notes.push(Note { channel: cid, key: 60, start: 0, len: 24, velocity: 0.8 });
    p.add_clip(0, 0, p.ticks_per_bar(), ClipKind::Pattern { pattern: pat });
    p.add_clip(
        1,
        384,
        768,
        ClipKind::Audio(AudioClip {
            path: "C:/s/loop.wav".into(),
            stretch: StretchSettings { mode: StretchMode::Stretch, source_bpm: 92.0, ..Default::default() },
            ..Default::default()
        }),
    );
    p.mixer[1].fx.push(FxSlot::new(EffectKind::Reverb));
    p.mixer[1].sends.push(MixerSend { target: 5, amount: 0.4 });
    p.swing = 0.3;
    p.add_channel("Strings", Instrument::SoundFont(SoundFontInstrument { path: "C:/sf/x.sf2".into(), program: 48, ..Default::default() }));
    p.add_clip(
        2,
        0,
        768,
        ClipKind::Automation(AutomationClip { target: "mixer.1.fx.0.mix".into(), points: vec![AutoPoint { t: 0, v: 0.0 }, AutoPoint { t: 768, v: 1.0 }] }),
    );
    let s = p.to_ron().unwrap();
    let q = Project::from_ron(&s).unwrap();
    assert_eq!(p, q);
}

#[test]
fn pattern_length_rounds_to_bars() {
    let mut p = Project::new_default();
    let pat = p.patterns[0].id;
    let cid = p.channels[0].id;
    assert_eq!(p.pattern_length(pat), 384);
    p.pattern_mut(pat).unwrap().notes.push(Note { channel: cid, key: 60, start: 380, len: 30, velocity: 1.0 });
    assert_eq!(p.pattern_length(pat), 768);
}

#[test]
fn removing_channel_removes_its_notes() {
    let mut p = Project::new_default();
    let pat = p.patterns[0].id;
    let cid = p.channels[0].id;
    p.pattern_mut(pat).unwrap().notes.push(Note { channel: cid, key: 60, start: 0, len: 24, velocity: 1.0 });
    p.remove_channel(0);
    assert!(p.pattern(pat).unwrap().notes.is_empty());
}
