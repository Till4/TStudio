//! Stem export: one offline render per mixer insert that carries sound (buses included,
//! without master effects). Shared by the GUI export window and `tstudio-cli render --stems`.
//!
//! 1. [`stem_jobs`] plans the project variants (pure, no audio),
//! 2. the caller compiles them with its own sample source ([`compile_stems`]),
//! 3. [`render_stems`] renders them (on any thread) with combined progress.

use crate::core::PlayMode;
use crate::render::render_offline;
use crate::snapshot::Snapshot;
use std::collections::BTreeSet;
use std::sync::Arc;
use tstudio_dsp::SampleData;
use tstudio_project::{ClipKind, Project};

/// What to render for one stem: `with`, minus `without` if set (send-fed buses).
pub struct StemJob<P> {
    pub track: usize,
    pub name: String,
    /// the insert is a bus (fed by other inserts)
    pub bus: bool,
    pub with: P,
    pub without: Option<P>,
}

pub struct Stem {
    pub track: usize,
    pub name: String,
    pub bus: bool,
    pub data: SampleData,
}

/// Mixer inserts that receive audio directly (channels or audio clips).
fn content_tracks(p: &Project) -> BTreeSet<usize> {
    let mut s: BTreeSet<usize> = p.channels.iter().map(|c| c.mixer_track).collect();
    for c in &p.playlist.clips {
        if let ClipKind::Audio(a) = &c.kind {
            s.insert(a.mixer_track);
        }
    }
    s.remove(&0);
    s
}

/// Inserts feeding bus `b`, and whether any of them uses it as its output (not just a send).
fn bus_sources(p: &Project, b: usize) -> (Vec<usize>, bool) {
    let mut src = Vec::new();
    let mut via_output = false;
    for (i, t) in p.mixer.iter().enumerate().skip(1) {
        if t.output == b {
            src.push(i);
            via_output = true;
        } else if t.sends.iter().any(|s| s.target == b && s.amount > 0.0) {
            src.push(i);
        }
    }
    (src, via_output)
}

/// Buses the output of insert `t` passes through on its way to the master.
fn output_chain(p: &Project, mut t: usize) -> BTreeSet<usize> {
    let mut s = BTreeSet::new();
    for _ in 0..p.mixer.len() {
        let o = p.mixer[t].output;
        if o == 0 || o >= p.mixer.len() || !s.insert(o) {
            break;
        }
        t = o;
    }
    s
}

/// The project with only `solo` inserts audible, `mute` inserts muted and master FX bypassed
/// (stems are pre-master).
fn variant(p: &Project, solo: &[usize], mute: &[usize]) -> Project {
    let mut q = p.clone();
    for (i, t) in q.mixer.iter_mut().enumerate() {
        if i == 0 {
            for f in t.fx.iter_mut() {
                f.enabled = false;
            }
            continue;
        }
        t.solo = solo.contains(&i);
        if mute.contains(&i) {
            t.muted = true;
        }
    }
    q
}

/// Plans one stem per unmuted insert that carries sound.
pub fn stem_jobs(p: &Project) -> Vec<StemJob<Project>> {
    let buses: BTreeSet<usize> = (1..p.mixer.len()).filter(|b| !bus_sources(p, *b).0.is_empty()).collect();
    let content = content_tracks(p);
    let mut out = Vec::new();
    for t in 1..p.mixer.len() {
        if p.mixer[t].muted {
            continue;
        }
        let name = p.mixer[t].name.clone();
        if buses.contains(&t) {
            let (src, via_output) = bus_sources(p, t);
            let others: Vec<usize> = buses.iter().copied().filter(|b| *b != t).collect();
            let with = variant(p, &src, &others);
            // a send-only bus (reverb/rumble): its own contribution = mix with it − mix without it
            let without = (!via_output).then(|| {
                let mut mute = others.clone();
                mute.push(t);
                variant(p, &src, &mute)
            });
            out.push(StemJob { track: t, name, bus: true, with, without });
        } else if content.contains(&t) {
            let chain = output_chain(p, t);
            let mute: Vec<usize> = buses.iter().copied().filter(|b| !chain.contains(b)).collect();
            out.push(StemJob { track: t, name, bus: false, with: variant(p, &[t], &mute), without: None });
        }
    }
    out
}

/// Compiles every planned variant with the caller's compiler (sample source).
pub fn compile_stems(jobs: Vec<StemJob<Project>>, mut compile: impl FnMut(&Project) -> Snapshot) -> Vec<StemJob<Arc<Snapshot>>> {
    jobs.into_iter()
        .map(|j| StemJob { track: j.track, name: j.name, bus: j.bus, with: Arc::new(compile(&j.with)), without: j.without.as_ref().map(|w| Arc::new(compile(w))) })
        .collect()
}

/// Renders all stems; `progress` gets 0..1 over the whole set and returns false to cancel.
pub fn render_stems(jobs: Vec<StemJob<Arc<Snapshot>>>, sr: u32, mode: PlayMode, tail_secs: f64, mut progress: impl FnMut(f32) -> bool) -> Option<Vec<Stem>> {
    let total: usize = jobs.iter().map(|j| 1 + j.without.is_some() as usize).sum::<usize>().max(1);
    let mut done = 0usize;
    let mut out = Vec::with_capacity(jobs.len());
    for j in jobs {
        let mut part = |snap: Arc<Snapshot>, done: usize| render_offline(snap, sr, mode, tail_secs, |f| progress((done as f32 + f) / total as f32));
        let with = part(j.with, done)?;
        done += 1;
        let data = match j.without {
            None => with,
            Some(w) => {
                let without = part(w, done)?;
                done += 1;
                let n = with.len().min(without.len());
                SampleData::new((0..n).map(|i| with.left[i] - without.left[i]).collect(), (0..n).map(|i| with.right[i] - without.right[i]).collect(), with.sample_rate)
            }
        };
        out.push(Stem { track: j.track, name: j.name, bus: j.bus, data });
    }
    progress(1.0);
    Some(out)
}

/// File name for a stem: "03 Kick.wav" (characters Windows forbids replaced).
pub fn stem_file_name(track: usize, name: &str, ext: &str) -> String {
    let clean: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim();
    let clean = if clean.is_empty() { "Insert" } else { clean };
    format!("{track:02} {clean}.{ext}")
}

#[cfg(test)]
mod tests;
