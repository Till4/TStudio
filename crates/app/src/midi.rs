//! MIDI keyboard input: notes go straight to the engine (low latency) and to the UI (recording);
//! control changes go only to the UI, which applies learned mappings (`midi_map`).
//! The callback never blocks on the audio thread: the engine queue is a lock-free ring.

use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tstudio_engine::Command;

pub use crate::midi_map::MidiMsg;

pub struct Midi {
    conn: Option<midir::MidiInputConnection<()>>,
    pub port_name: Option<String>,
    pub rx: Receiver<MidiMsg>,
    tx: Sender<MidiMsg>,
    /// Channel that receives live MIDI.
    pub target_channel: Arc<AtomicUsize>,
    /// Engine's MIDI command queue (replaced when the engine restarts).
    pub engine_tx: Arc<Mutex<Option<rtrb::Producer<Command>>>>,
    pub error: Option<String>,
}

impl Midi {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self { conn: None, port_name: None, rx, tx, target_channel: Arc::new(AtomicUsize::new(0)), engine_tx: Arc::new(Mutex::new(None)), error: None }
    }

    pub fn ports() -> Vec<String> {
        let Ok(input) = midir::MidiInput::new("TStudio-list") else { return Vec::new() };
        input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
    }

    pub fn disconnect(&mut self) {
        if let Some(c) = self.conn.take() {
            c.close();
        }
        self.port_name = None;
    }

    pub fn connect(&mut self, name: &str, ctx: egui::Context) {
        self.disconnect();
        self.error = None;
        let input = match midir::MidiInput::new("TStudio") {
            Ok(i) => i,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let Some(port) = input.ports().into_iter().find(|p| input.port_name(p).map(|n| n == name).unwrap_or(false)) else {
            self.error = Some(format!("MIDI-Port '{name}' nicht gefunden"));
            return;
        };
        let tx = self.tx.clone();
        let target = Arc::clone(&self.target_channel);
        let engine_tx = Arc::clone(&self.engine_tx);
        let res = input.connect(
            &port,
            "tstudio-in",
            move |_ts, msg, _| {
                let Some(m) = crate::midi_map::parse(msg) else { return };
                let ch = target.load(Ordering::Relaxed);
                let cmd = match m {
                    MidiMsg::NoteOn(k, v) => Some(Command::NoteOn { channel: ch, key: k, velocity: v }),
                    MidiMsg::NoteOff(k) => Some(Command::NoteOff { channel: ch, key: k }),
                    MidiMsg::Cc(..) => None,
                };
                if let Some(cmd) = cmd {
                    // the UI holds this lock only briefly when the engine restarts
                    if let Ok(mut guard) = engine_tx.lock() {
                        if let Some(p) = guard.as_mut() {
                            let _ = p.push(cmd);
                        }
                    }
                }
                let _ = tx.send(m);
                ctx.request_repaint();
            },
            (),
        );
        match res {
            Ok(c) => {
                self.conn = Some(c);
                self.port_name = Some(name.to_string());
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
}
