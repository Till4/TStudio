//! Background sample loading and tempo-fitting.
//!
//! Every `Arc<SampleData>` handed to the engine stays owned by this cache until
//! nothing else references it (strong_count == 1), so the audio thread never
//! frees sample memory.

use crossbeam_channel::{unbounded, Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tstudio_dsp::stretch::{change_tempo_ex, StretchMode};
use tstudio_dsp::SampleData;
use tstudio_project::StretchSettings;

/// Waveform overview: min/max per `per` samples.
pub struct Peaks {
    pub per: usize,
    pub v: Vec<(f32, f32)>,
}

impl Peaks {
    pub fn of(d: &SampleData) -> Peaks {
        let per = 256;
        let n = d.len().div_ceil(per);
        let mut v = Vec::with_capacity(n);
        for b in 0..n {
            let s = b * per;
            let e = ((b + 1) * per).min(d.len());
            let mut mn = 0.0f32;
            let mut mx = 0.0f32;
            for i in s..e {
                let x = 0.5 * (d.left[i] + d.right[i]);
                mn = mn.min(x);
                mx = mx.max(x);
            }
            v.push((mn, mx));
        }
        Peaks { per, v }
    }

    /// Min/max over sample range [s, e).
    pub fn range(&self, s: f64, e: f64) -> (f32, f32) {
        if self.v.is_empty() || e <= 0.0 {
            return (0.0, 0.0);
        }
        let b0 = (s.max(0.0) / self.per as f64) as usize;
        let b1 = ((e / self.per as f64).ceil() as usize).max(b0 + 1).min(self.v.len());
        if b0 >= self.v.len() {
            return (0.0, 0.0);
        }
        let mut mn = 0.0f32;
        let mut mx = 0.0f32;
        for &(a, b) in &self.v[b0..b1] {
            mn = mn.min(a);
            mx = mx.max(b);
        }
        (mn, mx)
    }
}

#[derive(Clone)]
pub struct Loaded {
    pub data: Arc<SampleData>,
    pub peaks: Arc<Peaks>,
}

enum Slot {
    Loading,
    Ready(Loaded, Instant),
    Failed(String),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PrepKey {
    path: PathBuf,
    mode: StretchMode,
    src_milli: i64,
    dst_milli: i64,
    pitch_cents: i32,
    formant: bool,
}

enum Job {
    Load(PathBuf, u32),
    Prepare(PrepKey, Arc<SampleData>),
}

enum JobResult {
    Loaded(PathBuf, Result<SampleData, String>),
    Prepared(PrepKey, SampleData),
}

pub struct SampleCache {
    sr: u32,
    raw: HashMap<PathBuf, Slot>,
    prepared: HashMap<PrepKey, Slot>,
    /// Latest finished preparation per (path, mode): shown while a newer one computes.
    latest: HashMap<(PathBuf, StretchMode), Loaded>,
    preparing: HashSet<(PathBuf, StretchMode)>,
    jobs: Sender<Job>,
    results: Receiver<JobResult>,
    /// Bumped whenever something finished loading.
    pub generation: u64,
    last_gc: Instant,
}

impl SampleCache {
    pub fn new(sr: u32) -> Self {
        let (jtx, jrx) = unbounded::<Job>();
        let (rtx, rrx) = unbounded::<JobResult>();
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(2, 4);
        for i in 0..threads {
            let jrx = jrx.clone();
            let rtx = rtx.clone();
            std::thread::Builder::new()
                .name(format!("tstudio-samples-{i}"))
                .spawn(move || {
                    while let Ok(job) = jrx.recv() {
                        let res = match job {
                            Job::Load(p, sr) => {
                                let r = tstudio_library::decode_file(&p).map(|d| if d.sample_rate != sr { d.resample_to(sr) } else { d });
                                JobResult::Loaded(p, r)
                            }
                            Job::Prepare(k, raw) => {
                                let out = change_tempo_ex(&raw, k.mode, k.src_milli as f64 / 1000.0, k.dst_milli as f64 / 1000.0, k.pitch_cents as f32 / 100.0, k.formant);
                                JobResult::Prepared(k, out)
                            }
                        };
                        if rtx.send(res).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawn sample worker");
        }
        Self {
            sr,
            raw: HashMap::new(),
            prepared: HashMap::new(),
            latest: HashMap::new(),
            preparing: HashSet::new(),
            jobs: jtx,
            results: rrx,
            generation: 0,
            last_gc: Instant::now(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sr
    }

    /// Engine sample rate changed: everything must be reloaded.
    pub fn reset(&mut self, sr: u32) {
        self.sr = sr;
        self.raw.clear();
        self.prepared.clear();
        self.latest.clear();
        self.preparing.clear();
        self.generation += 1;
    }

    /// Decoded sample at engine rate; starts loading if needed.
    pub fn get_loaded(&mut self, path: &Path) -> Option<Loaded> {
        match self.raw.get_mut(path) {
            Some(Slot::Ready(l, used)) => {
                *used = Instant::now();
                Some(l.clone())
            }
            Some(Slot::Loading) | Some(Slot::Failed(_)) => None,
            None => {
                self.raw.insert(path.to_path_buf(), Slot::Loading);
                let _ = self.jobs.send(Job::Load(path.to_path_buf(), self.sr));
                None
            }
        }
    }

    pub fn get_raw(&mut self, path: &Path) -> Option<Arc<SampleData>> {
        self.get_loaded(path).map(|l| l.data)
    }

    pub fn error(&self, path: &Path) -> Option<&str> {
        match self.raw.get(path) {
            Some(Slot::Failed(e)) => Some(e),
            _ => None,
        }
    }

    pub fn is_loading(&self, path: &Path) -> bool {
        matches!(self.raw.get(path), Some(Slot::Loading))
    }

    /// Tempo-fitted version (or the raw sample when stretching is off).
    pub fn get_prepared(&mut self, path: &Path, st: &StretchSettings, project_bpm: f64) -> Option<Loaded> {
        let (mode, src, dst) = st.tempo_job(project_bpm);
        let pitch_cents = (st.pitch * 100.0).round() as i32;
        if mode == StretchMode::Off && pitch_cents == 0 {
            return self.get_loaded(path);
        }
        let raw = self.get_raw(path)?;
        let key = PrepKey { path: path.to_path_buf(), mode, src_milli: (src * 1000.0).round() as i64, dst_milli: (dst * 1000.0).round() as i64, pitch_cents, formant: st.formant && pitch_cents != 0 };
        if key.src_milli == key.dst_milli && key.pitch_cents == 0 {
            return self.get_loaded(path);
        }
        if let Some(Slot::Ready(l, used)) = self.prepared.get_mut(&key) {
            *used = Instant::now();
            return Some(l.clone());
        }
        let group = (key.path.clone(), key.mode);
        if !self.prepared.contains_key(&key) && !self.preparing.contains(&group) {
            self.preparing.insert(group.clone());
            self.prepared.insert(key.clone(), Slot::Loading);
            let _ = self.jobs.send(Job::Prepare(key, raw));
        }
        self.latest.get(&group).cloned()
    }

    /// Handles finished jobs. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(r) = self.results.try_recv() {
            changed = true;
            match r {
                JobResult::Loaded(p, res) => {
                    let slot = match res {
                        Ok(d) => {
                            let peaks = Arc::new(Peaks::of(&d));
                            Slot::Ready(Loaded { data: Arc::new(d), peaks }, Instant::now())
                        }
                        Err(e) => Slot::Failed(e),
                    };
                    self.raw.insert(p, slot);
                }
                JobResult::Prepared(k, d) => {
                    let peaks = Arc::new(Peaks::of(&d));
                    let l = Loaded { data: Arc::new(d), peaks };
                    let group = (k.path.clone(), k.mode);
                    self.preparing.remove(&group);
                    self.latest.insert(group, l.clone());
                    self.prepared.insert(k, Slot::Ready(l, Instant::now()));
                }
            }
        }
        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Frees samples nobody uses anymore (not referenced by engine, voices or `keep`).
    pub fn gc(&mut self, keep: &[PathBuf]) {
        if self.last_gc.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_gc = Instant::now();
        let idle = Duration::from_secs(30);
        let unused = |l: &Loaded, used: &Instant| Arc::strong_count(&l.data) == 1 && used.elapsed() > idle;
        self.raw.retain(|p, s| match s {
            Slot::Ready(l, used) => keep.contains(p) || !unused(l, used),
            Slot::Failed(_) => false,
            Slot::Loading => true,
        });
        self.prepared.retain(|_, s| match s {
            Slot::Ready(l, used) => !unused(l, used),
            Slot::Failed(_) => false,
            Slot::Loading => true,
        });
        // keep `latest` only while it is the active fallback for a kept file
        self.latest.retain(|(p, _), l| keep.contains(p) || Arc::strong_count(&l.data) > 2);
    }

    /// True while any decode or tempo preparation is still running.
    pub fn busy(&self) -> bool {
        self.raw.values().chain(self.prepared.values()).any(|s| matches!(s, Slot::Loading))
    }

    pub fn memory_mb(&self) -> f32 {
        let mut bytes = 0usize;
        for s in self.raw.values().chain(self.prepared.values()) {
            if let Slot::Ready(l, _) = s {
                bytes += l.data.len() * 8;
            }
        }
        bytes as f32 / (1024.0 * 1024.0)
    }

    /// Keeps an externally created sample alive (e.g. trim preview) while the engine plays it.
    pub fn hold(&mut self, tag: &str, data: Arc<SampleData>) {
        let peaks = Arc::new(Peaks { per: 256, v: Vec::new() });
        self.raw.insert(PathBuf::from(format!("<mem>/{tag}")), Slot::Ready(Loaded { data, peaks }, Instant::now()));
    }
}
