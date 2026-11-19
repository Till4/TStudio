//! Audio input (microphone / line-in) for recording and live monitoring.
//!
//! The cpal input callback pushes stereo frames into two lock-free rings: one is drained by a
//! small collector thread into the current take, the other feeds the engine for monitoring
//! (through a mixer insert, so e.g. Autotune is heard live). Every take carries an *anchor*
//! (frame index ↔ capture time) so the app can place it exactly where it was sung, using the
//! engine's playback clock ([`crate::SharedState::play_start`]).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

/// Lock-free state shared with the input callback.
pub struct InputShared {
    epoch: Instant,
    level: [AtomicU32; 2],
    frames: AtomicU64,
    recording: AtomicBool,
    anchor_ready: AtomicBool,
    anchor_frame: AtomicU64,
    anchor_ns: AtomicU64,
    dropped: AtomicU64,
    pub error: AtomicBool,
}

impl InputShared {
    /// Peak since the last call (linear), per side.
    pub fn take_level(&self) -> (f32, f32) {
        (f32::from_bits(self.level[0].swap(0, Ordering::Relaxed)), f32::from_bits(self.level[1].swap(0, Ordering::Relaxed)))
    }
}

/// A finished recording: interleaved stereo at `sample_rate`.
#[derive(Default)]
pub struct Take {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// input frame index of `samples[0]`
    pub first_frame: u64,
    /// (frame index, capture time in ns since the engine epoch)
    pub anchor: Option<(u64, u64)>,
    /// frames lost because the collector fell behind (should be 0)
    pub dropped: u64,
}

impl Take {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Capture time (ns since epoch, may be negative) of the first frame.
    pub fn first_frame_ns(&self) -> Option<i128> {
        let (af, ans) = self.anchor?;
        let df = self.first_frame as i128 - af as i128;
        Some(ans as i128 + df * 1_000_000_000 / self.sample_rate.max(1) as i128)
    }
}

struct Collected {
    take: Take,
    started: bool,
}

pub struct InputHandle {
    _stream: cpal::Stream,
    pub sample_rate: u32,
    pub device_name: String,
    pub shared: Arc<InputShared>,
    collected: Arc<Mutex<Collected>>,
    stop: Arc<AtomicBool>,
}

impl InputHandle {
    /// Opens the input device (None = system default), preferring `prefer_sr` (the engine rate)
    /// so monitoring works without resampling. `monitor` receives stereo frames for the engine.
    pub fn start(device: Option<&str>, prefer_sr: u32, epoch: Instant, monitor: Option<rtrb::Producer<f32>>) -> Result<InputHandle, String> {
        let host = cpal::default_host();
        let dev = match device {
            Some(name) => host.input_devices().map_err(|e| e.to_string())?.find(|d| d.name().map(|n| n == name).unwrap_or(false)).or_else(|| host.default_input_device()),
            None => host.default_input_device(),
        }
        .ok_or_else(|| "kein Aufnahmegerät (Mikrofon) gefunden".to_string())?;
        let device_name = dev.name().unwrap_or_else(|_| "Mikrofon".into());
        let default = dev.default_input_config().map_err(|e| format!("Mikrofon nicht verfügbar: {e}"))?;
        // same rate as the engine if the device supports it
        let supported = dev
            .supported_input_configs()
            .ok()
            .and_then(|mut it| it.find(|c| c.sample_format() == default.sample_format() && c.min_sample_rate().0 <= prefer_sr && c.max_sample_rate().0 >= prefer_sr && c.channels() == default.channels()))
            .map(|c| c.with_sample_rate(cpal::SampleRate(prefer_sr)))
            .unwrap_or(default);
        let sr = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();

        let shared = Arc::new(InputShared {
            epoch,
            level: [AtomicU32::new(0), AtomicU32::new(0)],
            frames: AtomicU64::new(0),
            recording: AtomicBool::new(false),
            anchor_ready: AtomicBool::new(false),
            anchor_frame: AtomicU64::new(0),
            anchor_ns: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            error: AtomicBool::new(false),
        });
        // ~10 s buffer between callback and collector
        let (rec_tx, mut rec_rx) = rtrb::RingBuffer::<f32>::new(sr as usize * 2 * 10);
        let monitor = if sr == prefer_sr { monitor } else { None };
        let cb = Callback { shared: Arc::clone(&shared), rec: rec_tx, monitor, channels: channels.max(1) };
        let err_shared = Arc::clone(&shared);
        let err_fn = move |e: cpal::StreamError| {
            eprintln!("input stream error: {e}");
            err_shared.error.store(true, Ordering::Relaxed);
        };
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&dev, &config, cb, err_fn),
            cpal::SampleFormat::I16 => build::<i16>(&dev, &config, cb, err_fn),
            cpal::SampleFormat::U16 => build::<u16>(&dev, &config, cb, err_fn),
            cpal::SampleFormat::I32 => build::<i32>(&dev, &config, cb, err_fn),
            cpal::SampleFormat::F64 => build::<f64>(&dev, &config, cb, err_fn),
            other => Err(format!("Sampleformat {other:?} nicht unterstützt")),
        }?;
        stream.play().map_err(|e| e.to_string())?;

        let collected = Arc::new(Mutex::new(Collected { take: Take { sample_rate: sr, ..Default::default() }, started: false }));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let collected = Arc::clone(&collected);
            let stop = Arc::clone(&stop);
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("tstudio-input-collect".into())
                .spawn(move || {
                    let mut popped: u64 = 0; // samples (not frames)
                    let mut buf: Vec<f32> = Vec::with_capacity(8192);
                    while !stop.load(Ordering::Relaxed) {
                        buf.clear();
                        while let Ok(v) = rec_rx.pop() {
                            buf.push(v);
                        }
                        if !buf.is_empty() {
                            let recording = shared.recording.load(Ordering::Acquire);
                            if let Ok(mut c) = collected.lock() {
                                if recording {
                                    if !c.started {
                                        c.started = true;
                                        c.take.first_frame = popped / 2;
                                    }
                                    c.take.samples.extend_from_slice(&buf);
                                }
                            }
                            popped += buf.len() as u64;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(InputHandle { _stream: stream, sample_rate: sr, device_name, shared, collected, stop })
    }

    pub fn is_recording(&self) -> bool {
        self.shared.recording.load(Ordering::Relaxed)
    }

    /// Starts a new take (discarding any unfinished one).
    pub fn start_take(&self) {
        if let Ok(mut c) = self.collected.lock() {
            c.take = Take { sample_rate: self.sample_rate, ..Default::default() };
            c.started = false;
        }
        self.shared.dropped.store(0, Ordering::Relaxed);
        self.shared.anchor_ready.store(false, Ordering::Release);
        self.shared.recording.store(true, Ordering::Release);
    }

    /// Ends the take and returns it.
    pub fn stop_take(&self) -> Take {
        // let the collector pick up what is still in the ring
        std::thread::sleep(Duration::from_millis(30));
        self.shared.recording.store(false, Ordering::Release);
        let mut take = match self.collected.lock() {
            Ok(mut c) => {
                c.started = false;
                std::mem::take(&mut c.take)
            }
            Err(_) => Take::default(),
        };
        take.sample_rate = self.sample_rate;
        if self.shared.anchor_ready.load(Ordering::Acquire) {
            take.anchor = Some((self.shared.anchor_frame.load(Ordering::Relaxed), self.shared.anchor_ns.load(Ordering::Relaxed)));
        }
        take.dropped = self.shared.dropped.load(Ordering::Relaxed);
        take
    }
}

impl Drop for InputHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Callback {
    shared: Arc<InputShared>,
    rec: rtrb::Producer<f32>,
    monitor: Option<rtrb::Producer<f32>>,
    channels: usize,
}

impl Callback {
    fn process(&mut self, data: &[f32], info: &cpal::InputCallbackInfo) {
        let s = &self.shared;
        let frames = data.len() / self.channels;
        let before = s.frames.load(Ordering::Relaxed);
        if s.recording.load(Ordering::Acquire) && !s.anchor_ready.load(Ordering::Relaxed) {
            let ts = info.timestamp();
            let lat = ts.callback.duration_since(&ts.capture).unwrap_or_default();
            let now = s.epoch.elapsed();
            let capture = now.saturating_sub(lat);
            s.anchor_frame.store(before, Ordering::Relaxed);
            s.anchor_ns.store(capture.as_nanos() as u64, Ordering::Relaxed);
            s.anchor_ready.store(true, Ordering::Release);
        }
        let (mut pl, mut pr) = (0.0f32, 0.0f32);
        for f in 0..frames {
            let l = data[f * self.channels];
            let r = if self.channels > 1 { data[f * self.channels + 1] } else { l };
            pl = pl.max(l.abs());
            pr = pr.max(r.abs());
            if self.rec.slots() >= 2 {
                let _ = self.rec.push(l);
                let _ = self.rec.push(r);
            } else {
                s.dropped.fetch_add(1, Ordering::Relaxed);
            }
            if let Some(m) = &mut self.monitor {
                if m.slots() >= 2 {
                    let _ = m.push(l);
                    let _ = m.push(r);
                }
            }
        }
        s.level[0].fetch_max(pl.to_bits(), Ordering::Relaxed);
        s.level[1].fetch_max(pr.to_bits(), Ordering::Relaxed);
        s.frames.store(before + frames as u64, Ordering::Relaxed);
    }
}

fn build<T>(dev: &cpal::Device, config: &cpal::StreamConfig, mut cb: Callback, err_fn: impl FnMut(cpal::StreamError) + Send + 'static) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let mut conv: Vec<f32> = Vec::with_capacity(1 << 16);
    dev.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            conv.clear();
            // capacity is reserved up front; very large blocks are processed in pieces
            let step = (conv.capacity() / cb.channels).max(1) * cb.channels;
            for chunk in data.chunks(step) {
                conv.clear();
                conv.extend(chunk.iter().map(|x| x.to_sample::<f32>()));
                cb.process(&conv, info);
            }
        },
        err_fn,
        None,
    )
    .map_err(|e| format!("Mikrofon konnte nicht geöffnet werden: {e}"))
}
