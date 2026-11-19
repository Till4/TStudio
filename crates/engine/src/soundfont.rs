//! SoundFont (.sf2) playback via rustysynth.

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub use rustysynth::SoundFont as Sf2;

fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<SoundFont>>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, Arc<SoundFont>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Loads (and caches) an .sf2 file.
pub fn load_soundfont(path: &Path) -> Result<Arc<SoundFont>, String> {
    if let Some(sf) = cache().lock().unwrap().get(path) {
        return Ok(Arc::clone(sf));
    }
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let sf = Arc::new(SoundFont::new(&mut f).map_err(|e| format!("{}: kein gültiges SF2 ({e:?})", path.display()))?);
    cache().lock().unwrap().insert(path.to_path_buf(), Arc::clone(&sf));
    Ok(sf)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sf2Preset {
    pub name: String,
    pub bank: u16,
    pub program: u8,
}

/// Presets sorted by bank, then program.
pub fn soundfont_presets(path: &Path) -> Result<Vec<Sf2Preset>, String> {
    let sf = load_soundfont(path)?;
    let mut v: Vec<Sf2Preset> = sf
        .get_presets()
        .iter()
        .map(|p| Sf2Preset { name: p.get_name().trim().to_string(), bank: p.get_bank_number().clamp(0, 65535) as u16, program: p.get_patch_number().clamp(0, 127) as u8 })
        .collect();
    v.sort_by_key(|p| (p.bank, p.program));
    Ok(v)
}

const MAX_PENDING: usize = 128;

/// One channel's SoundFont synth. Notes with a duration are released by an internal timer.
pub struct SfProc {
    synth: Synthesizer,
    /// (key, frames left) of sounding notes with a known length
    pending: [(u8, u64); MAX_PENDING],
    n_pending: usize,
    bank: i32,
    program: i32,
    /// frames of silence rendered since the last note (stops rendering once long enough)
    silent_frames: u64,
    active: bool,
    sr: f32,
}

impl SfProc {
    pub fn new(sf: &Arc<SoundFont>, sr: f32) -> Option<SfProc> {
        let mut settings = SynthesizerSettings::new(sr.round() as i32);
        settings.enable_reverb_and_chorus = false;
        settings.maximum_polyphony = 64;
        let mut synth = Synthesizer::new(sf, &settings).ok()?;
        // rustysynth defaults to 0.5, which leaves most SoundFonts ~6 dB quieter than the other instruments
        synth.set_master_volume(1.0);
        Some(SfProc { synth, pending: [(0, 0); MAX_PENDING], n_pending: 0, bank: -1, program: -1, silent_frames: 0, active: false, sr })
    }

    /// Drum kits (bank 128) play on the GM percussion channel.
    fn midi_channel(bank: u16) -> i32 {
        if bank >= 128 {
            9
        } else {
            0
        }
    }

    fn select(&mut self, bank: u16, program: u8) {
        let (b, p) = (bank as i32, program as i32);
        if b != self.bank || p != self.program {
            let ch = Self::midi_channel(bank);
            let bank_cc = if bank >= 128 { b - 128 } else { b };
            self.synth.process_midi_message(ch, 0xB0, 0x00, bank_cc.clamp(0, 127));
            self.synth.process_midi_message(ch, 0xC0, p, 0);
            self.bank = b;
            self.program = p;
        }
    }

    pub fn note_on(&mut self, bank: u16, program: u8, key: u8, vel: f32, dur: Option<u64>) {
        self.select(bank, program);
        let ch = Self::midi_channel(bank);
        let v = (vel.clamp(0.0, 1.0) * 127.0).round().max(1.0) as i32;
        // retrigger: release a still-held identical key first
        self.synth.note_off(ch, key as i32);
        self.drop_pending(key);
        self.synth.note_on(ch, key as i32, v);
        if let Some(d) = dur {
            if self.n_pending < MAX_PENDING {
                self.pending[self.n_pending] = (key, d.max(1));
                self.n_pending += 1;
            }
        }
        self.active = true;
        self.silent_frames = 0;
    }

    fn drop_pending(&mut self, key: u8) {
        let mut i = 0;
        while i < self.n_pending {
            if self.pending[i].0 == key {
                self.n_pending -= 1;
                self.pending[i] = self.pending[self.n_pending];
            } else {
                i += 1;
            }
        }
    }

    pub fn note_off(&mut self, key: u8) {
        self.drop_pending(key);
        self.synth.note_off(Self::midi_channel(self.bank.max(0) as u16), key as i32);
    }

    pub fn release_all(&mut self) {
        self.n_pending = 0;
        self.synth.note_off_all(false);
    }

    pub fn kill_all(&mut self) {
        self.n_pending = 0;
        self.synth.note_off_all(true);
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Adds `gain`-scaled output to `l`/`r` (which are expected to be zeroed scratch).
    pub fn render(&mut self, gain: f32, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let mut done = 0;
        while done < n {
            // render up to the next scheduled note-off
            let mut step = (n - done) as u64;
            for i in 0..self.n_pending {
                step = step.min(self.pending[i].1);
            }
            let step = step.max(1) as usize;
            self.synth.render(&mut l[done..done + step], &mut r[done..done + step]);
            done += step;
            let ch = Self::midi_channel(self.bank.max(0) as u16);
            let mut i = 0;
            while i < self.n_pending {
                let left = self.pending[i].1.saturating_sub(step as u64);
                if left == 0 {
                    let key = self.pending[i].0;
                    self.synth.note_off(ch, key as i32);
                    self.n_pending -= 1;
                    self.pending[i] = self.pending[self.n_pending];
                } else {
                    self.pending[i].1 = left;
                    i += 1;
                }
            }
        }
        let mut peak = 0.0f32;
        for k in 0..n {
            l[k] *= gain;
            r[k] *= gain;
            peak = peak.max(l[k].abs()).max(r[k].abs());
        }
        if self.n_pending == 0 && peak < 1e-5 {
            self.silent_frames += n as u64;
            if self.silent_frames as f32 > self.sr * 0.5 {
                self.active = false;
            }
        } else {
            self.silent_frames = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_sf2() -> Option<PathBuf> {
        let docs = std::env::var_os("USERPROFILE").map(PathBuf::from)?.join("Documents");
        [docs.join("lmms/samples/soundfonts/SuperMetroid.sf2"), docs.join("Image-Line/Downloads/FL Studio Mobile Factory Data/SoundFonts/STR Ensemble.sf2")]
            .into_iter()
            .find(|p| p.exists())
    }

    #[test]
    fn plays_a_real_soundfont_if_present() {
        let Some(path) = find_sf2() else { return };
        let presets = soundfont_presets(&path).unwrap();
        assert!(!presets.is_empty());
        let p = &presets[0];
        let sf = load_soundfont(&path).unwrap();
        let mut proc = SfProc::new(&sf, 48000.0).unwrap();
        proc.note_on(p.bank, p.program, 60, 0.9, Some(4800));
        let mut l = vec![0.0f32; 9600];
        let mut r = vec![0.0f32; 9600];
        for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            proc.render(1.0, a, b);
        }
        let peak = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        eprintln!("sf2 {} preset {} peak {peak}", path.display(), p.name);
        assert!(peak > 0.001, "{} preset {} silent", path.display(), p.name);
        assert!(l.iter().all(|v| v.is_finite()));
        assert_eq!(proc.n_pending, 0, "note released after its duration");
    }
}
