//! Audio recording (vocals): 🎤 in the transport bar records the microphone while the song
//! plays and drops the take as an audio clip on a "Vocals" track, routed to the "Vocals" mixer
//! insert (with Autotune). Takes are placed by the engine's playback clock, so the output and
//! input latency of the sound card are compensated.

use crate::app::TStudio;
use crate::ui::widgets::{self, ACCENT, TEXT_DIM};
use egui::{Color32, RichText, Ui};
use std::path::PathBuf;
use tstudio_dsp::fx::{EffectKind, EffectParams};
use tstudio_engine::{Command, InputHandle, MonitorIn};
use tstudio_project::{AudioClip, ClipKind, FxSlot, PPQ};

pub const VOCAL_INSERT: &str = "Vocals";

#[derive(Default)]
pub struct RecordState {
    pub input: Option<InputHandle>,
    pub error: Option<String>,
    pub show_window: bool,
    /// a take is running
    pub active: bool,
    /// playback was started by the recording (→ use the engine's play-start stamp)
    started_playback: bool,
    /// play-start stamp before pressing record (to recognise the new one)
    old_stamp: Option<(u64, f64)>,
    /// (ns, ticks) reference when recording started during playback
    reference: Option<(u64, f64)>,
    pub level: (f32, f32),
    pub last_take: Option<PathBuf>,
}

/// Opens the microphone (and the monitoring path) if it is not open yet.
pub fn ensure_input(app: &mut TStudio) -> bool {
    if app.rec.input.is_some() {
        return true;
    }
    let sr = app.engine.sample_rate;
    let (tx, rx) = rtrb::RingBuffer::<f32>::new(sr as usize); // 0.5 s stereo
    match InputHandle::start(app.settings.input_device.as_deref(), sr, app.engine.shared.epoch, Some(tx)) {
        Ok(h) => {
            if h.sample_rate == sr {
                app.send(Command::SetMonitor(Some(Box::new(MonitorIn::new(rx)))));
            }
            app.rec.input = Some(h);
            app.rec.error = None;
            sync_monitor(app);
            true
        }
        Err(e) => {
            app.rec.error = Some(e.clone());
            app.set_error(format!("Aufnahme: {e}"));
            false
        }
    }
}

/// Closes the microphone (e.g. after the audio device changed).
pub fn close_input(app: &mut TStudio) {
    if app.rec.active {
        finish_take(app);
    }
    app.rec.input = None;
    app.send(Command::SetMonitor(None));
}

/// Index of the "Vocals" insert (created with Autotune if missing).
pub fn vocal_insert(app: &mut TStudio) -> usize {
    if let Some(i) = app.project.mixer.iter().position(|t| t.name == VOCAL_INSERT) {
        return i;
    }
    let i = app.project.add_mixer_track(Some(VOCAL_INSERT.into()));
    app.project.mixer[i].fx.push(FxSlot::new(EffectKind::Autotune));
    i
}

/// Routes the live input through the vocal insert when monitoring is on.
pub fn sync_monitor(app: &mut TStudio) {
    let track = if app.settings.monitor_input && app.rec.input.is_some() {
        Some(app.project.mixer.iter().position(|t| t.name == VOCAL_INSERT).unwrap_or(0))
    } else {
        None
    };
    app.send(Command::SetMonitorTrack(track));
}

/// 🎤 pressed: start or finish a take.
pub fn toggle(app: &mut TStudio) {
    if app.rec.active {
        finish_take(app);
        return;
    }
    if !ensure_input(app) {
        return;
    }
    vocal_insert(app);
    sync_monitor(app);
    let Some(input) = &app.rec.input else { return };
    input.start_take();
    app.rec.active = true;
    app.rec.reference = None;
    if app.is_playing() && app.song_mode {
        let shared = &app.engine.shared;
        let ns = shared.epoch.elapsed().as_nanos() as u64 + (shared.output_latency_ms() * 1e6) as u64;
        app.rec.reference = Some((ns, app.position()));
        app.rec.started_playback = false;
    } else {
        if app.is_playing() {
            app.send(Command::Stop);
        }
        app.rec.old_stamp = app.engine.shared.play_start();
        app.song_mode = true;
        app.sync_mode();
        app.send(Command::Play);
        app.rec.started_playback = true;
    }
    app.set_status("● Aufnahme läuft – 🎤 oder Stop beendet sie");
}

/// Called every frame: levels, and stopping the take when playback stops.
pub fn poll(app: &mut TStudio, ctx: &egui::Context) {
    let Some((l, r)) = app.rec.input.as_ref().map(|i| i.shared.take_level()) else { return };
    app.rec.level = ((app.rec.level.0 * 0.85).max(l), (app.rec.level.1 * 0.85).max(r));
    if app.rec.active {
        // the reference for a take started from a stopped transport is the new play-start stamp
        if app.rec.reference.is_none() && app.rec.started_playback {
            let stamp = app.engine.shared.play_start();
            if stamp.is_some() && stamp != app.rec.old_stamp {
                app.rec.reference = stamp;
            }
        }
        if !app.is_playing() && app.rec.reference.is_some() {
            finish_take(app);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    } else if app.rec.show_window {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

fn free_take_path(app: &TStudio) -> PathBuf {
    let dir = app.settings.output_dir().join("Recordings");
    let _ = std::fs::create_dir_all(&dir);
    let base = app.project_path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
    (1..).map(|i| dir.join(format!("{base} Vocal {i:02}.wav"))).find(|p| !p.exists()).unwrap()
}

/// Where a take goes: (frames to cut from its start, start position in ticks).
/// `ref_ns`/`ref_pos`: the song was at `ref_pos` ticks when it was heard at `ref_ns`;
/// `first_ns`: capture time of the take's first frame. Nothing is placed before `ref_pos`.
pub fn place_take(ref_ns: u64, ref_pos: f64, first_ns: i128, tps: f64, sr: f64, extra_s: f64) -> (usize, f64) {
    let first_pos = ref_pos + ((first_ns - ref_ns as i128) as f64 / 1e9 - extra_s) * tps;
    if first_pos < ref_pos {
        (((ref_pos - first_pos) / tps * sr).round() as usize, ref_pos)
    } else {
        (0, first_pos)
    }
}

/// Ends the take: trims the part before the song started, saves the WAV and adds the clip.
pub fn finish_take(app: &mut TStudio) {
    app.rec.active = false;
    let Some(input) = &app.rec.input else { return };
    let take = input.stop_take();
    if app.rec.started_playback && app.is_playing() {
        app.send(Command::Stop);
    }
    let sr = take.sample_rate.max(1) as f64;
    let Some((ref_ns, ref_pos)) = app.rec.reference.take() else {
        app.set_error("Aufnahme verworfen: Wiedergabe ist nicht gestartet");
        return;
    };
    if take.frames() < (sr * 0.2) as usize {
        app.set_status("Aufnahme zu kurz – verworfen");
        return;
    }
    let tps = app.project.bpm / 60.0 * PPQ as f64; // ticks per second
    let extra = app.settings.record_offset_ms as f64 / 1000.0;
    let first_ns = take.first_frame_ns().unwrap_or(ref_ns as i128);
    let (skip, start) = place_take(ref_ns, ref_pos, first_ns, tps, sr, extra);
    let frames = take.frames();
    if skip >= frames {
        app.set_status("Aufnahme zu kurz – verworfen");
        return;
    }
    let (mut left, mut right) = (Vec::with_capacity(frames - skip), Vec::with_capacity(frames - skip));
    for f in skip..frames {
        left.push(take.samples[2 * f]);
        right.push(take.samples[2 * f + 1]);
    }
    // mono microphones arrive as identical channels → keep them identical (centred vocal)
    let data = tstudio_dsp::SampleData::new(left, right, take.sample_rate);
    let path = free_take_path(app);
    if let Err(e) = tstudio_library::write_wav(&path, &data, tstudio_library::WavFormat::Pcm24) {
        app.set_error(format!("Aufnahme konnte nicht gespeichert werden: {e}"));
        return;
    }
    let start = start.round().max(0.0) as i64;
    let len = ((data.left.len() as f64 / sr) * tps).round().max(1.0) as i64;
    let insert = vocal_insert(app);
    let track = vocal_track(app, start, start + len);
    app.project.add_clip(track, start, len, ClipKind::Audio(AudioClip { path: path.clone(), mixer_track: insert, ..Default::default() }));
    app.cache.get_raw(&path);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let lost = if take.dropped > 0 { format!(" ⚠ {} Frames verloren", take.dropped) } else { String::new() };
    app.set_status(format!("✔ {name} aufgenommen ({:.1} s) → Spur {}{lost}", data.left.len() as f64 / sr, track + 1));
    app.rec.last_take = Some(path);
}

/// A "Vocals…" playlist track that is free between `a` and `b` (new one otherwise).
fn vocal_track(app: &mut TStudio, a: i64, b: i64) -> usize {
    let p = &app.project;
    let free = |t: usize| !p.playlist.clips.iter().any(|c| c.track == t && c.start < b && c.start + c.len > a);
    if let Some(t) = (0..p.playlist.tracks.len()).find(|&t| p.playlist.tracks[t].name.starts_with("Vocal") && free(t)) {
        return t;
    }
    // first completely empty track below the last used one
    let last_used = p.playlist.clips.iter().map(|c| c.track + 1).max().unwrap_or(0);
    let t = match (last_used..p.playlist.tracks.len()).next() {
        Some(t) => t,
        None => app.project.add_playlist_track(None),
    };
    let n = app.project.playlist.tracks.iter().filter(|t| t.name.starts_with("Vocal")).count();
    app.project.playlist.tracks[t].name = if n == 0 { "Vocals".into() } else { format!("Vocals {}", n + 1) };
    t
}

/// 🎤 button + input meter for the transport bar.
pub fn transport_button(app: &mut TStudio, ui: &mut Ui) {
    let col = if app.rec.active { Color32::from_rgb(240, 70, 60) } else if app.rec.input.is_some() { ACCENT } else { TEXT_DIM };
    let txt = if app.rec.active { "🎤 ●" } else { "🎤" };
    let resp = ui
        .add(egui::Button::new(RichText::new(txt).size(15.0).color(col)).min_size(egui::vec2(34.0, 26.0)))
        .on_hover_text("Vocals aufnehmen (Strg+Shift+R): spielt den Song ab der Playhead-Position und nimmt das Mikrofon auf.\nRechtsklick: Aufnahme-Einstellungen (Mikrofon, Mithören, Autotune)");
    if resp.clicked() {
        toggle(app);
    }
    if resp.secondary_clicked() {
        app.rec.show_window = !app.rec.show_window;
    }
    if app.rec.input.is_some() {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 24.0), egui::Sense::hover());
        widgets::draw_meter(ui.painter(), rect, app.rec.level.0, app.rec.level.1);
    }
}

/// "Aufnahme" settings window.
pub fn window(app: &mut TStudio, ctx: &egui::Context) {
    if !app.rec.show_window {
        return;
    }
    let mut open = true;
    egui::Window::new("🎤 Vocal-Aufnahme").open(&mut open).resizable(false).default_width(380.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Mikrofon");
            let cur = app.settings.input_device.clone().unwrap_or_else(|| "Standard".into());
            let mut changed = false;
            egui::ComboBox::from_id_salt("rec_dev").selected_text(cur).width(240.0).show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut app.settings.input_device, None, "Standard (Windows)").changed();
                for d in tstudio_engine::list_input_devices() {
                    changed |= ui.selectable_value(&mut app.settings.input_device, Some(d.clone()), d).changed();
                }
            });
            if changed {
                close_input(app);
                ensure_input(app);
            }
        });
        ui.horizontal(|ui| {
            if app.rec.input.is_none() {
                if ui.button("Mikrofon einschalten").clicked() {
                    ensure_input(app);
                }
            } else {
                let (l, r) = app.rec.level;
                let peak = l.max(r);
                let db = tstudio_dsp::gain_to_db(peak).max(-60.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(220.0, 12.0), egui::Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, 2.0, Color32::from_gray(35));
                let frac = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
                let col = if db > -3.0 { Color32::from_rgb(240, 70, 60) } else if db > -12.0 { Color32::from_rgb(240, 200, 70) } else { Color32::from_rgb(110, 210, 120) };
                p.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, rect.height())), 2.0, col);
                ui.label(RichText::new(format!("{db:.0} dB")).monospace().small());
                if ui.small_button("Aus").on_hover_text("Mikrofon schließen").clicked() {
                    close_input(app);
                }
            }
        });
        if let Some(e) = &app.rec.error {
            ui.colored_label(Color32::from_rgb(240, 110, 90), format!("⚠ {e}"));
            ui.weak("Windows-Einstellungen → Datenschutz → Mikrofon: Zugriff für Desktop-Apps erlauben.");
        }
        if let Some(h) = &app.rec.input {
            ui.weak(format!("{} · {} Hz · Ausgabe-Latenz {:.0} ms", h.device_name, h.sample_rate, app.engine.shared.output_latency_ms()));
            if h.sample_rate != app.engine.sample_rate {
                ui.colored_label(Color32::from_rgb(240, 200, 70), "Mikrofon läuft mit anderer Samplerate – Mithören ist aus, Aufnahme funktioniert.");
            }
        }
        ui.separator();
        if ui
            .checkbox(&mut app.settings.monitor_input, "Mithören über den Vocals-Insert (mit Autotune)")
            .on_hover_text("Nur mit Kopfhörern – sonst gibt es Rückkopplung. Etwas verzögert (Puffer + Autotune).")
            .changed()
        {
            sync_monitor(app);
        }
        ui.horizontal(|ui| {
            ui.label("Versatz");
            ui.add(egui::Slider::new(&mut app.settings.record_offset_ms, -100.0..=100.0).suffix(" ms"))
                .on_hover_text("Feinkorrektur, falls Aufnahmen zu früh (+) oder zu spät (−) liegen. Normalerweise 0.");
        });
        ui.separator();
        ui.horizontal(|ui| {
            let vi = app.project.mixer.iter().position(|t| t.name == VOCAL_INSERT);
            match vi {
                Some(i) => {
                    let has_at = app.project.mixer[i].fx.iter().any(|f| matches!(f.params, EffectParams::Autotune(_)));
                    ui.label(format!("Insert {i} „{VOCAL_INSERT}“ {}", if has_at { "· Autotune ✔" } else { "" }));
                    if !has_at && ui.button("+ Autotune").clicked() {
                        app.project.mixer[i].fx.insert(0, FxSlot::new(EffectKind::Autotune));
                    }
                    if ui.button("🎚 Öffnen").on_hover_text("Vocals-Insert im Mixer öffnen (Tonart/Skala einstellen)").clicked() {
                        app.mixer_ui.selected = i;
                        app.view = crate::app::View::Mixer;
                    }
                }
                None => {
                    if ui.button("Vocals-Insert mit Autotune anlegen").clicked() {
                        vocal_insert(app);
                        sync_monitor(app);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("📂 Aufnahmen-Ordner").clicked() {
                let dir = app.settings.output_dir().join("Recordings");
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
            }
            let txt = if app.rec.active { "■ Aufnahme beenden" } else { "● Aufnahme starten" };
            if ui.button(RichText::new(txt).color(Color32::from_rgb(240, 90, 80))).clicked() {
                toggle(app);
            }
        });
        ui.weak("Tipp: Playhead an die Stelle setzen, 🎤 drücken, singen, Stop. Jeder Take landet auf einer freien Vocals-Spur.");
    });
    if !open {
        app.rec.show_window = false;
    }
}

#[cfg(test)]
mod tests {
    use super::place_take;

    #[test]
    fn take_is_trimmed_to_the_song_start() {
        // 140 bpm → 224 ticks/s; song heard from tick 768 at t = 10 s; mic started 0.25 s earlier
        let tps = 140.0 / 60.0 * 96.0;
        let (skip, start) = place_take(10_000_000_000, 768.0, 9_750_000_000, tps, 48000.0, 0.0);
        assert_eq!(skip, 12000);
        assert_eq!(start, 768.0);
        // recording started mid-song half a second after the reference
        let (skip, start) = place_take(10_000_000_000, 768.0, 10_500_000_000, tps, 48000.0, 0.0);
        assert_eq!(skip, 0);
        assert!((start - (768.0 + 0.5 * tps)).abs() < 1e-6);
        // manual +20 ms offset moves it earlier
        let (_, s2) = place_take(10_000_000_000, 768.0, 10_500_000_000, tps, 48000.0, 0.02);
        assert!((start - s2 - 0.02 * tps).abs() < 1e-6);
    }
}
