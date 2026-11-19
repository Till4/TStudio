//! MIDI keyboard input: notes go straight to the engine (low latency) and to the UI (recording).

use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tstudio_engine::Command;

#[derive(Clone, Copy, Debug)]
pub enum MidiMsg {
    NoteOn(u8, f32),
    NoteOff(u8),
}

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
                if msg.len() < 3 {
                    return;
                }
                let status = msg[0] & 0xF0;
                let key = msg[1].min(127);
                let vel = msg[2];
                let parsed = match status {
                    0x90 if vel > 0 => Some(MidiMsg::NoteOn(key, vel as f32 / 127.0)),
                    0x80 | 0x90 => Some(MidiMsg::NoteOff(key)),
                    _ => None,
                };
                let Some(m) = parsed else { return };
                let ch = target.load(Ordering::Relaxed);
                if let Ok(mut guard) = engine_tx.lock() {
                    if let Some(p) = guard.as_mut() {
                        let cmd = match m {
                            MidiMsg::NoteOn(k, v) => Command::NoteOn { channel: ch, key: k, velocity: v },
                            MidiMsg::NoteOff(k) => Command::NoteOff { channel: ch, key: k },
                        };
                        let _ = p.push(cmd);
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
