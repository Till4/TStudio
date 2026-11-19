//! Song / pattern export (offline render → WAV or MP3).

use crate::app::TStudio;
use crossbeam_channel::Receiver;
use egui::RichText;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tstudio_dsp::SampleData;
use tstudio_engine::{render_offline, PlayMode};
use tstudio_library::{write_wav, WavFormat};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Wav16,
    Wav24,
    Wav32f,
    Mp3,
    Mp4,
}

impl ExportFormat {
    fn name(self) -> &'static str {
        match self {
            Self::Wav16 => "WAV 16 Bit",
            Self::Wav24 => "WAV 24 Bit",
            Self::Wav32f => "WAV 32 Bit Float",
            Self::Mp3 => "MP3",
            Self::Mp4 => "MP4-Video (Visualizer)",
        }
    }
    fn ext(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Mp4 => "mp4",
            _ => "wav",
        }
    }
}

enum Msg {
    Progress(f32),
    Done(Result<PathBuf, String>),
}

enum Phase {
    Idle,
    /// Waiting for samples / stretch jobs to finish before rendering.
    Preparing { path: PathBuf, mode: PlayMode },
    Rendering { rx: Receiver<Msg>, cancel: Arc<AtomicBool> },
}

pub struct ExportState {
    pub open: bool,
    format: ExportFormat,
    mp3_kbps: u32,
    /// video title (empty = project name)
    title: String,
    song: bool,
    tail_secs: f64,
    normalize: bool,
    progress: f32,
    phase: Phase,
    last: Option<Result<PathBuf, String>>,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            open: false,
            format: ExportFormat::Wav24,
            mp3_kbps: 320,
            title: String::new(),
            song: true,
            tail_secs: 2.0,
            normalize: false,
            progress: 0.0,
            phase: Phase::Idle,
            last: None,
        }
    }
}

impl ExportState {
    pub fn is_busy(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }
}

pub fn poll(app: &mut TStudio) {
    match &app.export.phase {
        Phase::Idle => {}
        Phase::Preparing { path, mode } => {
            let (path, mode) = (path.clone(), *mode);
            // compiling requests every sample / stretch job the export needs
            let _ = crate::compile::compile(&app.project, &mut app.cache, true);
            app.cache.poll();
            if app.cache.busy() {
                return;
            }
            let snap = Arc::new(crate::compile::compile(&app.project, &mut app.cache, true));
            let sr = app.engine.sample_rate;
            let (tx, rx) = crossbeam_channel::unbounded();
            let cancel = Arc::new(AtomicBool::new(false));
            let c2 = Arc::clone(&cancel);
            let fmt = app.export.format;
            let kbps = app.export.mp3_kbps;
            let tail = app.export.tail_secs;
            let normalize = app.export.normalize;
            let ffmpeg = app.fetch.tools.ffmpeg.clone();
            let video = video_opts(app);
            // audio is the quick part of a video export
            let audio_share = if fmt == ExportFormat::Mp4 { 0.15 } else { 1.0 };
            std::thread::spawn(move || {
                let tx2 = tx.clone();
                let out: Option<SampleData> = render_offline(snap, sr, mode, tail, |p| {
                    let _ = tx2.send(Msg::Progress(p * audio_share));
                    !c2.load(Ordering::Relaxed)
                });
                let res = match out {
                    None => Err("Abgebrochen".to_string()),
                    Some(mut d) => {
                        if normalize {
                            d.normalize(0.98);
                        }
                        if let Some(dir) = path.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        match fmt {
                            ExportFormat::Wav16 => write_wav(&path, &d, WavFormat::Pcm16),
                            ExportFormat::Wav24 => write_wav(&path, &d, WavFormat::Pcm24),
                            ExportFormat::Wav32f => write_wav(&path, &d, WavFormat::Float32),
                            ExportFormat::Mp3 => match &ffmpeg {
                                Some(f) => tstudio_fetch::export_mp3(f, &d, &path, kbps),
                                None => Err("ffmpeg nicht gefunden (winget install Gyan.FFmpeg)".into()),
                            },
                            ExportFormat::Mp4 => match &ffmpeg {
                                Some(f) => tstudio_fetch::video::export_mp4(f, &d, &path, &video, |p| {
                                    let _ = tx.send(Msg::Progress(audio_share + (1.0 - audio_share) * p));
                                    !c2.load(Ordering::Relaxed)
                                }),
                                None => Err("ffmpeg nicht gefunden (winget install Gyan.FFmpeg)".into()),
                            },
                        }
                        .map(|_| path)
                    }
                };
                let _ = tx.send(Msg::Done(res));
            });
            app.export.progress = 0.0;
            app.export.phase = Phase::Rendering { rx, cancel };
        }
        Phase::Rendering { rx, .. } => {
            let mut done = None;
            while let Ok(m) = rx.try_recv() {
                match m {
                    Msg::Progress(p) => app.export.progress = p,
                    Msg::Done(r) => done = Some(r),
                }
            }
            if let Some(r) = done {
                match &r {
                    Ok(p) => app.set_status(format!("Exportiert: {}", p.display())),
                    Err(e) => app.set_error(format!("Export fehlgeschlagen: {e}")),
                }
                app.export.last = Some(r);
                app.export.phase = Phase::Idle;
            }
        }
    }
}

pub fn window(app: &mut TStudio, ctx: &egui::Context) {
    if !app.export.open {
        return;
    }
    let mut open = app.export.open;
    let mut start: Option<PlayMode> = None;
    let has_ffmpeg = app.fetch.tools.ffmpeg.is_some();
    let pattern_idx = app.current_pattern_index();
    let pattern_name = pattern_idx.map(|i| app.project.patterns[i].name.clone()).unwrap_or_default();
    let project_name = app.project.name.clone();
    egui::Window::new("Exportieren").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
        let e = &mut app.export;
        let settings = &mut app.settings;
        let busy = e.is_busy();
        ui.add_enabled_ui(!busy, |ui| {
            egui::Grid::new("export_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Bereich");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut e.song, true, "Song");
                    ui.add_enabled_ui(pattern_idx.is_some(), |ui| {
                        ui.selectable_value(&mut e.song, false, format!("Pattern: {pattern_name}"));
                    });
                });
                ui.end_row();
                ui.label("Format");
                egui::ComboBox::from_id_salt("exp_fmt").selected_text(e.format.name()).show_ui(ui, |ui| {
                    for f in [ExportFormat::Wav16, ExportFormat::Wav24, ExportFormat::Wav32f, ExportFormat::Mp3, ExportFormat::Mp4] {
                        ui.selectable_value(&mut e.format, f, f.name());
                    }
                });
                ui.end_row();
                if e.format == ExportFormat::Mp3 {
                    ui.label("Bitrate");
                    egui::ComboBox::from_id_salt("exp_kbps").selected_text(format!("{} kbps", e.mp3_kbps)).show_ui(ui, |ui| {
                        for k in [128, 192, 256, 320] {
                            ui.selectable_value(&mut e.mp3_kbps, k, format!("{k} kbps"));
                        }
                    });
                    ui.end_row();
                }
                if e.format == ExportFormat::Mp4 {
                    ui.label("DJ-Name");
                    ui.add(egui::TextEdit::singleline(&mut settings.dj_name).hint_text("dein DJ-Name").desired_width(200.0));
                    ui.end_row();
                    ui.label("Titel");
                    ui.add(egui::TextEdit::singleline(&mut e.title).hint_text(project_name.as_str()).desired_width(200.0));
                    ui.end_row();
                    ui.label("Farbe");
                    ui.horizontal(|ui| {
                        for (name, c) in tstudio_fetch::video::ACCENTS {
                            let col = egui::Color32::from_rgb(c[0], c[1], c[2]);
                            let sel = settings.video_accent == c;
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                            ui.painter().rect_filled(r.shrink(if sel { 0.0 } else { 3.0 }), 4.0, col);
                            if sel {
                                ui.painter().rect_stroke(r, 4.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                            }
                            if resp.on_hover_text(name).clicked() {
                                settings.video_accent = c;
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Größe");
                    egui::ComboBox::from_id_salt("exp_vsize").selected_text(size_label(&settings.video_size)).show_ui(ui, |ui| {
                        for (n, _, _) in tstudio_fetch::video::SIZES {
                            ui.selectable_value(&mut settings.video_size, n.to_string(), size_label(n));
                        }
                    });
                    ui.end_row();
                    ui.label("Logo");
                    ui.checkbox(&mut settings.video_watermark, "„made with TStudio“");
                    ui.end_row();
                }
                ui.label("Ausklingen");
                ui.add(egui::DragValue::new(&mut e.tail_secs).range(0.0..=20.0).speed(0.1).suffix(" s"));
                ui.end_row();
                ui.label("Normalisieren");
                ui.checkbox(&mut e.normalize, "");
                ui.end_row();
            });
            if matches!(e.format, ExportFormat::Mp3 | ExportFormat::Mp4) && !has_ffmpeg {
                ui.colored_label(egui::Color32::from_rgb(240, 110, 90), "MP3/MP4 braucht ffmpeg: winget install Gyan.FFmpeg");
            }
            if e.format == ExportFormat::Mp4 {
                ui.weak("Das Video wird auf der CPU gerendert (keine GPU-Last) und dauert etwa so lange wie der Song.");
            }
        });
        ui.separator();
        match &e.phase {
            Phase::Idle => {
                let ok = !matches!(e.format, ExportFormat::Mp3 | ExportFormat::Mp4) || has_ffmpeg;
                if ui.add_enabled(ok, egui::Button::new(RichText::new("Exportieren…").strong())).clicked() {
                    start = Some(if e.song { PlayMode::Song } else { PlayMode::Pattern(pattern_idx.unwrap_or(0)) });
                }
            }
            Phase::Preparing { .. } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Samples werden vorbereitet…");
                });
            }
            Phase::Rendering { cancel, .. } => {
                ui.add(egui::ProgressBar::new(e.progress).show_percentage());
                if ui.button("Abbrechen").clicked() {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        }
        if let Some(r) = &e.last {
            match r {
                Ok(p) => {
                    ui.horizontal(|ui| {
                        ui.label(format!("✔ {}", p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
                        if ui.small_button("Im Explorer zeigen").clicked() {
                            reveal(p);
                        }
                    });
                }
                Err(err) => {
                    ui.colored_label(egui::Color32::from_rgb(240, 110, 90), err);
                }
            }
        }
    });
    app.export.open = open || app.export.is_busy();

    if let Some(mode) = start {
        let stem = if app.project.name.trim().is_empty() { "Export".to_string() } else { app.project.name.clone() };
        let ext = app.export.format.ext();
        let dir = app.project_path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| app.settings.output_dir());
        if let Some(path) = rfd::FileDialog::new().set_directory(&dir).set_file_name(format!("{stem}.{ext}")).add_filter(ext.to_uppercase(), &[ext]).save_file() {
            let path = if path.extension().is_none() { path.with_extension(ext) } else { path };
            app.export.last = None;
            app.export.phase = Phase::Preparing { path, mode };
            if app.export.format == ExportFormat::Mp4 {
                // the CLI reads the DJ name from the settings file
                app.settings.save(&app.paths.settings());
            }
        }
    }
}

fn size_label(n: &str) -> &'static str {
    match n {
        "720p" => "1280×720 (16:9)",
        "vertical" => "1080×1920 (Hochkant, Reels/TikTok)",
        "square" => "1080×1080 (Quadrat)",
        _ => "1920×1080 (16:9)",
    }
}

fn video_opts(app: &TStudio) -> tstudio_fetch::video::VideoOpts {
    let s = &app.settings;
    let (width, height) = tstudio_fetch::video::size_by_name(&s.video_size).unwrap_or((1920, 1080));
    let title = if app.export.title.trim().is_empty() { app.project.name.clone() } else { app.export.title.clone() };
    tstudio_fetch::video::VideoOpts { width, height, artist: s.dj_name.clone(), title, accent: s.video_accent, watermark: s.video_watermark, ..Default::default() }
}

pub fn reveal(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}
