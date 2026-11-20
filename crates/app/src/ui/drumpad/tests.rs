use super::*;
use tstudio_project::SamplerInstrument;

fn sampler(p: &mut Project, name: &str) -> usize {
    p.add_channel(name, Instrument::Sampler(SamplerInstrument { sample: Some(format!("{name}.wav").into()), ..Default::default() }))
}

#[test]
fn one_pad_per_sampler_channel() {
    let mut p = Project::new_default(); // channel 0 is a TSynth → no pad
    let k = sampler(&mut p, "Kick");
    let s = sampler(&mut p, "Snare");
    let pads = pad_map(&p, PadSet::Auto, 0);
    assert_eq!(pads.len(), 2);
    assert_eq!((pads[0].channel, pads[0].key, pads[0].label.as_str()), (k, 60, "Kick"));
    assert_eq!(pads[1].channel, s);
    for i in 0..20 {
        sampler(&mut p, &format!("S{i}"));
    }
    assert_eq!(pad_map(&p, PadSet::Channels, 0).len(), PADS);
}

#[test]
fn chopped_sampler_maps_slices_to_pads() {
    let mut p = Project::new_default();
    let c = sampler(&mut p, "Break");
    let other = sampler(&mut p, "Hat");
    if let Instrument::Sampler(s) = &mut p.channels[c].instrument {
        slicer::apply_slices(&mut s.params, slicer::even_slices(0.0, 1.0, 8), Some(36));
    }
    let pads = pad_map(&p, PadSet::Auto, c);
    assert_eq!(pads.len(), 8);
    assert!(pads.iter().all(|x| x.channel == c));
    assert_eq!(pads.iter().map(|x| x.key).collect::<Vec<_>>(), (36..44).collect::<Vec<u8>>());
    // Auto with an unchopped channel selected → channel pads again
    assert_eq!(pad_map(&p, PadSet::Auto, other).len(), 2);
    // forced slice view of a channel without slices: no pads
    assert!(pad_map(&p, PadSet::Chop, other).is_empty());
    // more than 16 slices: the first 16 go on the pads
    if let Instrument::Sampler(s) = &mut p.channels[c].instrument {
        slicer::apply_slices(&mut s.params, slicer::even_slices(0.0, 1.0, 32), Some(36));
    }
    assert_eq!(pad_map(&p, PadSet::Chop, c).len(), PADS);
}
