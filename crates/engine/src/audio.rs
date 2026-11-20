//! cpal output stream (WASAPI on Windows) wrapping an [`EngineCore`].
//! Falls back to a silent "null" clock thread when no device is available,
//! so the app stays usable (and export still works).

use crate::core::{Command, EngineCore, Garbage, SharedState, MAX_CHUNK};
use crate::snapshot::{Rack, Snapshot};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioConfig {
    /// Output device name; None = system default.
    pub device: Option<String>,
    /// Requested buffer size in frames; None = driver default.
    pub buffer_size: Option<u32>,
}

pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

struct RtProcessor {
    core: EngineCore,
    rx: rtrb::Consumer<Command>,
    midi_rx: rtrb::Consumer<Command>,
    gc: rtrb::Producer<Garbage>,
    l: Vec<f32>,
    r: Vec<f32>,
    shared: Arc<SharedState>,
}

impl RtProcessor {
    fn drain(&mut self) {
        let gc = &mut self.gc;
        let mut sink = |g: Garbage| {
            if let Err(rtrb::PushError::Full(g)) = gc.push(g) {
                // never free on the audio thread; leaking is the lesser evil
                std::mem::forget(g);
            }
        };
        while let Ok(cmd) = self.rx.pop() {
            self.core.handle(cmd, &mut sink);
        }
        while let Ok(cmd) = self.midi_rx.pop() {
            self.core.handle(cmd, &mut sink);
        }
    }

    fn fill<T: cpal::SizedSample + cpal::FromSample<f32>>(&mut self, data: &mut [T], channels: usize, latency: Duration) {
        let started = Instant::now();
        let block_at = self.shared.epoch.elapsed();
        let was_playing = self.core.is_playing();
        self.drain();
        let now_playing = self.core.is_playing();
        self.shared.note_block(now_playing && !was_playing, self.core.position(), latency);
        let channels = channels.max(1);
        let frames = data.len() / channels;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.l.len());
            self.core.process(&mut self.l[..n], &mut self.r[..n]);
            if let Some((f, pos)) = self.core.take_started_at() {
                // count-in finished inside this block: stamp when that frame is heard
                let offset = Duration::from_secs_f64((done + f) as f64 / self.core.sample_rate().max(1.0) as f64);
                self.shared.note_start_at(block_at + latency + offset, pos);
            }
            for i in 0..n {
                let base = (done + i) * channels;
                data[base] = T::from_sample(self.l[i]);
                if channels > 1 {
                    data[base + 1] = T::from_sample(self.r[i]);
                }
                for c in 2..channels {
                    data[base + c] = T::from_sample(0.0);
                }
            }
            done += n;
        }
        if frames > 0 {
            let budget = frames as f32 / self.core.sample_rate();
            self.shared.set_cpu_load(started.elapsed().as_secs_f32() / budget);
        }
    }
}

pub struct EngineHandle {
    tx: rtrb::Producer<Command>,
    gc_rx: rtrb::Consumer<Garbage>,
    midi_tx: Option<rtrb::Producer<Command>>,
    pub shared: Arc<SharedState>,
    pub sample_rate: u32,
    pub device_name: String,
    /// Human-readable problem with the audio device, if any.
    pub status: Option<String>,
    _stream: Option<cpal::Stream>,
    null_stop: Option<Arc<AtomicBool>>,
}

impl EngineHandle {
    pub fn start(cfg: &AudioConfig) -> EngineHandle {
        match Self::try_start(cfg) {
            Ok(h) => h,
            Err(e) => {
                let mut h = Self::start_null(48000);
                h.status = Some(format!("Kein Audiogerät: {e}"));
                h
            }
        }
    }

    fn make_processor(sr: u32) -> (RtProcessor, rtrb::Producer<Command>, rtrb::Consumer<Garbage>, rtrb::Producer<Command>, Arc<SharedState>) {
        let (tx, rx) = rtrb::RingBuffer::new(1024);
        let (midi_tx, midi_rx) = rtrb::RingBuffer::new(512);
        let (gc_tx, gc_rx) = rtrb::RingBuffer::new(1024);
        let shared = Arc::new(SharedState::new(sr));
        let snap = Arc::new(Snapshot::empty());
        let rack = Box::new(Rack::build(&snap, sr as f32, MAX_CHUNK));
        let core = EngineCore::new(sr as f32, snap, rack, Arc::clone(&shared));
        let proc = RtProcessor { core, rx, midi_rx, gc: gc_tx, l: vec![0.0; 8192], r: vec![0.0; 8192], shared: Arc::clone(&shared) };
        (proc, tx, gc_rx, midi_tx, shared)
    }

    fn try_start(cfg: &AudioConfig) -> Result<EngineHandle, String> {
        let host = cpal::default_host();
        let device = match &cfg.device {
            Some(name) => host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| &n == name).unwrap_or(false))
                .or_else(|| host.default_output_device()),
            None => host.default_output_device(),
        }
        .ok_or_else(|| "kein Ausgabegerät gefunden".to_string())?;
        let device_name = device.name().unwrap_or_else(|_| "Unbekannt".into());
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let sr = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let format = supported.sample_format();
        let base_config: cpal::StreamConfig = supported.config();

        let attempts: Vec<cpal::StreamConfig> = match cfg.buffer_size {
            Some(b) => vec![cpal::StreamConfig { buffer_size: cpal::BufferSize::Fixed(b), ..base_config.clone() }, base_config.clone()],
            None => vec![base_config.clone()],
        };

        let mut last_err = String::new();
        for config in attempts {
            let (proc, tx, gc_rx, midi_tx, shared) = Self::make_processor(sr);
            let err_shared = Arc::clone(&shared);
            let err_fn = move |e: cpal::StreamError| {
                eprintln!("audio stream error: {e}");
                err_shared.stream_error.store(true, Ordering::Relaxed);
            };
            let stream = match format {
                cpal::SampleFormat::F32 => build::<f32>(&device, &config, proc, channels, err_fn),
                cpal::SampleFormat::I16 => build::<i16>(&device, &config, proc, channels, err_fn),
                cpal::SampleFormat::U16 => build::<u16>(&device, &config, proc, channels, err_fn),
                cpal::SampleFormat::I32 => build::<i32>(&device, &config, proc, channels, err_fn),
                cpal::SampleFormat::F64 => build::<f64>(&device, &config, proc, channels, err_fn),
                other => Err(format!("Sampleformat {other:?} nicht unterstützt")),
            };
            match stream {
                Ok(stream) => {
                    stream.play().map_err(|e| e.to_string())?;
                    return Ok(EngineHandle {
                        tx,
                        gc_rx,
                        midi_tx: Some(midi_tx),
                        shared,
                        sample_rate: sr,
                        device_name,
                        status: None,
                        _stream: Some(stream),
                        null_stop: None,
                    });
                }
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Silent clock thread so sequencing and UI keep working without a device.
    /// Engine driven by a silent clock thread (no audio device).
    pub fn start_null(sr: u32) -> EngineHandle {
        let (mut proc, tx, gc_rx, midi_tx, shared) = Self::make_processor(sr);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("tstudio-null-audio".into())
            .spawn(move || {
                let frames = 512usize;
                let mut buf = vec![0.0f32; frames * 2];
                let period = Duration::from_secs_f64(frames as f64 / sr as f64);
                let mut next = Instant::now();
                while !stop2.load(Ordering::Relaxed) {
                    proc.fill(&mut buf, 2, Duration::ZERO);
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
            })
            .expect("spawn null audio thread");
        EngineHandle {
            tx,
            gc_rx,
            midi_tx: Some(midi_tx),
            shared,
            sample_rate: sr,
            device_name: "Kein Gerät (stumm)".into(),
            status: None,
            _stream: None,
            null_stop: Some(stop),
        }
    }

    /// Sends a command; gives it back if the queue is full.
    pub fn send(&mut self, cmd: Command) -> Result<(), Command> {
        self.tx.push(cmd).map_err(|rtrb::PushError::Full(c)| c)
    }

    /// Drops objects the audio thread handed back. Call regularly from the UI thread.
    pub fn collect_garbage(&mut self) -> usize {
        let mut n = 0;
        while let Ok(g) = self.gc_rx.pop() {
            drop(g);
            n += 1;
        }
        n
    }

    /// Second producer for low-latency MIDI input from another thread.
    pub fn take_midi_sender(&mut self) -> Option<rtrb::Producer<Command>> {
        self.midi_tx.take()
    }

    pub fn is_null(&self) -> bool {
        self._stream.is_none()
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        if let Some(s) = &self.null_stop {
            s.store(true, Ordering::Relaxed);
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut proc: RtProcessor,
    channels: usize,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    device
        .build_output_stream::<T, _, _>(config, move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                let latency = ts.playback.duration_since(&ts.callback).unwrap_or_default();
                proc.fill(data, channels, latency)
            }, err_fn, None)
        .map_err(|e| e.to_string())
}
