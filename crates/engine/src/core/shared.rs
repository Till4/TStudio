//! Types shared between UI and audio thread: play mode, commands, garbage queue, `SharedState` (atomics for position, meters, CPU load).

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    /// Loop one pattern (index into `Snapshot::patterns`).
    Pattern(usize),
    /// Play the playlist.
    Song,
}

pub enum Command {
    SetSnapshot(Arc<Snapshot>),
    SetRack(Box<Rack>, Arc<Snapshot>),
    Play,
    Pause,
    Stop,
    SetPosition(f64),
    SetMode(PlayMode),
    SetMetronome(bool),
    NoteOn { channel: usize, key: u8, velocity: f32 },
    NoteOff { channel: usize, key: u8 },
    PreviewSample(Arc<SampleData>, f32),
    StopPreview,
    /// Loop the current preview sample (sample cutter)
    PreviewLoop(bool),
    Panic,
    /// Live input for monitoring (replaces the previous one)
    SetMonitor(Option<Box<MonitorIn>>),
    /// Mixer insert the live input is played through (None = not audible)
    SetMonitorTrack(Option<usize>),
}

/// Live input frames (interleaved stereo) from the audio input callback.
pub struct MonitorIn {
    pub rx: rtrb::Consumer<f32>,
    pub(super) primed: bool,
}

impl MonitorIn {
    pub fn new(rx: rtrb::Consumer<f32>) -> Self {
        Self { rx, primed: false }
    }
}

/// Objects handed back to the UI thread for deallocation.
pub enum Garbage {
    Rack(Box<Rack>),
    Snapshot(Arc<Snapshot>),
    Sample(Arc<SampleData>),
    Monitor(Box<MonitorIn>),
}

/// (time, position) written by the audio thread, read consistently by the UI (seqlock).
pub(super) struct Stamp {
    pub(super) seq: AtomicU32,
    pub(super) ns: AtomicU64,
    pub(super) pos: AtomicU64,
}

impl Stamp {
    pub(super) fn new() -> Self {
        Self { seq: AtomicU32::new(0), ns: AtomicU64::new(0), pos: AtomicU64::new(0) }
    }
    pub(super) fn write(&self, ns: u64, pos: f64) {
        self.seq.fetch_add(1, Ordering::AcqRel);
        self.ns.store(ns, Ordering::Relaxed);
        self.pos.store(pos.to_bits(), Ordering::Relaxed);
        self.seq.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn read(&self) -> Option<(u64, f64)> {
        for _ in 0..100 {
            let a = self.seq.load(Ordering::Acquire);
            if a % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let ns = self.ns.load(Ordering::Relaxed);
            let pos = f64::from_bits(self.pos.load(Ordering::Relaxed));
            if self.seq.load(Ordering::Acquire) == a {
                return if a == 0 { None } else { Some((ns, pos)) };
            }
        }
        None
    }
}

/// Lock-free state the UI reads every frame.
pub struct SharedState {
    pub(super) position: AtomicU64,
    pub(super) playing: AtomicBool,
    pub(super) meters: Vec<[AtomicU32; 2]>,
    pub(super) cpu: AtomicU32,
    pub sample_rate: AtomicU32,
    pub stream_error: AtomicBool,
    /// time base shared with the audio input (ns since `epoch`)
    pub epoch: std::time::Instant,
    /// when (audible at the speakers) and where the last playback started
    pub(super) play_start: Stamp,
    /// output latency of the device in ns
    pub(super) out_latency_ns: AtomicU64,
}

impl SharedState {
    pub fn new(sr: u32) -> Self {
        Self {
            position: AtomicU64::new(0f64.to_bits()),
            playing: AtomicBool::new(false),
            meters: (0..MAX_TRACKS).map(|_| [AtomicU32::new(0), AtomicU32::new(0)]).collect(),
            cpu: AtomicU32::new(0),
            sample_rate: AtomicU32::new(sr),
            stream_error: AtomicBool::new(false),
            epoch: std::time::Instant::now(),
            play_start: Stamp::new(),
            out_latency_ns: AtomicU64::new(0),
        }
    }

    /// (ns since epoch when it reached the speakers, start position in ticks) of the last playback start.
    pub fn play_start(&self) -> Option<(u64, f64)> {
        self.play_start.read()
    }

    pub fn output_latency_ms(&self) -> f32 {
        self.out_latency_ns.load(Ordering::Relaxed) as f32 / 1e6
    }

    /// Called by the output callback at the start of every block.
    pub fn note_block(&self, started_playing: bool, pos: f64, latency: std::time::Duration) {
        self.out_latency_ns.store(latency.as_nanos() as u64, Ordering::Relaxed);
        if started_playing {
            let ns = (self.epoch.elapsed() + latency).as_nanos() as u64;
            self.play_start.write(ns, pos);
        }
    }

    /// Playhead in ticks.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Peak since the last call (linear), per side.
    pub fn take_meter(&self, track: usize) -> (f32, f32) {
        match self.meters.get(track) {
            Some(m) => (f32::from_bits(m[0].swap(0, Ordering::Relaxed)), f32::from_bits(m[1].swap(0, Ordering::Relaxed))),
            None => (0.0, 0.0),
        }
    }

    /// Fraction of the real-time budget used by the last callback.
    pub fn cpu_load(&self) -> f32 {
        f32::from_bits(self.cpu.load(Ordering::Relaxed))
    }

    pub fn set_cpu_load(&self, v: f32) {
        self.cpu.store(v.to_bits(), Ordering::Relaxed);
    }

    pub(super) fn set_meter(&self, track: usize, l: f32, r: f32) {
        if let Some(m) = self.meters.get(track) {
            // positive floats order like their bit patterns
            m[0].fetch_max(l.abs().to_bits(), Ordering::Relaxed);
            m[1].fetch_max(r.abs().to_bits(), Ordering::Relaxed);
        }
    }
}
