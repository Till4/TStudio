use super::*;

#[test]
fn settings_from_before_recording_and_midi_learn_still_load() {
    // a settings.ron written before count-in/take lanes/MIDI mappings existed
    let mut s = Settings::default();
    s.record_offset_ms = 12.5;
    let text = ron::ser::to_string_pretty(&s, ron::ser::PrettyConfig::default()).unwrap();
    let new_fields = ["record_click:", "record_count_in:", "record_take_lanes:", "midi_mappings:"];
    let old: Vec<&str> = text.lines().filter(|l| !new_fields.iter().any(|k| l.trim_start().starts_with(k))).collect();
    let old = old.join(&char::from(10).to_string());
    assert!(!old.contains("midi_mappings"));
    let back: Settings = ron::from_str(&old).expect("old settings must load");
    assert_eq!(back.record_offset_ms, 12.5);
    assert!(back.record_click && back.record_take_lanes && back.record_count_in == 1);
    assert!(back.midi_mappings.is_empty());
}
