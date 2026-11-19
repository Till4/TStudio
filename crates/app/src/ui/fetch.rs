use crate::app::TStudio;
use crate::ui::cutter::{self, Loaded, Trim};
use crate::settings::Settings;
use crate::ui::widgets::TEXT_DIM;
use crossbeam_channel::Receiver;
use egui::{Color32, RichText, Ui};
use std::path::{Path, PathBuf};
use tstudio_fetch::{AudioFormat, DownloadHandle, FetchEvent, Tools};

pub struct FetchState {
    pub url: String,
    pub format: AudioFormat,
    pub tools: Tools,
    versions: Option<(Option<String>, Option<String>)>,
    download: Option<DownloadHandle>,
    pub progress: f32,
    pub log: Vec<String>,
    pub last_error: Option<String>,
    pub downloads: Vec<PathBuf>,
    pub trim: Trim,
    trim_load: Option<(PathBuf, Receiver<Result<Loaded, String>>)>,
    /// running stem separation, its progress (0..1), last log line and start time
    pub stems: Option<tstudio_fetch::stems::StemHandle>,
    pub stem_progress: f32,
    pub stem_log: String,
    pub stem_started: Option<std::time::Instant>,
    /// folder with the stems of the last run, or its error
    pub stem_result: Option<Result<PathBuf, String>>,
    /// background "is Demucs installed?" check
    pub stem_check: Option<Receiver<Result<String, String>>>,
    pub stem_status: Option<Result<String, String>>,
}

impl FetchState {
    pub fn new(settings: &Settings) -> Self {
        Self {
            url: String::new(),
            format: AudioFormat::Mp3,
            tools: Tools::detect(settings.ffmpeg_path.as_deref(), settings.ytdlp_path.as_deref()),
            versions: None,
            download: None,
            progress: 0.0,
            log: Vec::new(),
            last_error: None,
            downloads: Vec::new(),
            trim: Trim::default(),
            trim_load: None,
            stems: None,
            stem_progress: 0.0,
            stem_log: String::new(),
            stem_started: None,
            stem_result: None,
            stem_check: None,
            stem_status: None,
        }
    }

    pub fn redetect(&mut self, settings: &Settings) {
        self.tools = Tools::detect(settings.ffmpeg_path.as_deref(), settings.ytdlp_path.as_deref());
        self.versions = None;
    }

    pub fn is_busy(&self) -> bool {
        self.download.is_some() || self.trim_load.is_some()
    }

    /// Decodes a file (at its original sample rate) into the trim editor.
    pub fn open_in_trim(&mut self, path: &Path) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let p = path.to_path_buf();
        let sens = self.trim.sensitivity;
        std::thread::spawn(move || {
            let _ = tx.send(cutter::load(&p, sens));
        });
        self.trim_load = Some((path.to_path_buf(), rx));
    }
}

pub fn poll(app: &mut TStudio) {
    let mut finished = None;
    let mut failed = None;
    if let Some(dl) = &app.fetch.download {
        while let Ok(ev) = dl.events.try_recv() {
            match ev {
                FetchEvent::Progress(p) => app.fetch.progress = p,
                FetchEvent::Log(l) => {
                    app.fetch.log.push(l);
                    if app.fetch.log.len() > 200 {
                        app.fetch.log.remove(0);
                    }
                }
                FetchEvent::Finished(p) => finished = Some(p),
                FetchEvent::Failed(e) => failed = Some(e),
            }
        }
    }
    if let Some(p) = finished {
        app.fetch.download = None;
        app.fetch.downloads.insert(0, p.clone());
        app.fetch.open_in_trim(&p);
        app.library.rescan(&app.settings);
        app.set_status(format!("Download fertig: {}", p.display()));
    }
    if let Some(e) = failed {
        app.fetch.download = None;
        app.fetch.last_error = Some(e.clone());
        app.set_error(format!("Download fehlgeschlagen: {e}"));
    }
    // stem separation
    let mut stem_done: Option<Result<PathBuf, String>> = None;
    if let Some(h) = &app.fetch.stems {
        while let Ok(ev) = h.events.try_recv() {
            match ev {
                FetchEvent::Progress(p) => app.fetch.stem_progress = p,
                FetchEvent::Log(l) => app.fetch.stem_log = l,
                FetchEvent::Finished(p) => stem_done = Some(Ok(p)),
                FetchEvent::Failed(e) => stem_done = Some(Err(e)),
            }
        }
    }
    if let Some(r) = stem_done {
        app.fetch.stems = None;
        match &r {
            Ok(dir) => {
                let secs = app.fetch.stem_started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                if !app.settings.library_roots.iter().any(|root| dir.starts_with(root)) {
                    app.settings.library_roots.push(app.settings.output_dir());
                    app.settings.save(&app.paths.settings());
                }
                app.library.rescan(&app.settings);
                app.set_status(format!("Stems fertig ({secs} s): {}", dir.display()));
            }
            Err(e) => app.set_error(format!("Stem-Trennung fehlgeschlagen: {e}")),
        }
        app.fetch.stem_result = Some(r);
    }
    if let Some(rx) = &app.fetch.stem_check {
        if let Ok(r) = rx.try_recv() {
            app.fetch.stem_status = Some(r);
            app.fetch.stem_check = None;
        }
    }
    if let Some((path, rx)) = &app.fetch.trim_load {
        if let Ok(res) = rx.try_recv() {
            let path = path.clone();
            app.fetch.trim_load = None;
            match res {
                Ok(l) => app.fetch.trim.set_loaded(path, l),
                Err(e) => app.set_error(format!("Konnte Datei nicht laden: {e}")),
            }
        }
    }
}

fn tool_row(ui: &mut Ui, name: &str, path: &Option<PathBuf>, version: Option<&String>, winget: &str) -> bool {
    let mut pick = false;
    ui.horizontal(|ui| {
        match path {
            Some(p) => {
                ui.colored_label(Color32::from_rgb(120, 230, 140), "✔");
                ui.label(RichText::new(name).strong());
                ui.weak(version.cloned().unwrap_or_else(|| p.display().to_string()));
            }
            None => {
                ui.colored_label(Color32::from_rgb(240, 110, 90), "✖");
                ui.label(RichText::new(name).strong());
                ui.label("nicht gefunden – installieren mit:");
                ui.code(winget);
                if ui.small_button("📋").on_hover_text("Befehl kopieren").clicked() {
                    ui.ctx().copy_text(winget.to_string());
                }
            }
        }
        if ui.small_button("Pfad…").clicked() {
            pick = true;
        }
    });
    pick
}

pub fn show(app: &mut TStudio, ui: &mut Ui) {
    egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("fetch_scroll").show(ui, |ui| {
        let loading = app.fetch.trim_load.is_some();
        cutter::show(app, ui, loading);
        ui.add_space(10.0);
        let open = app.fetch.trim.data.is_none() || app.fetch.download.is_some();
        egui::CollapsingHeader::new(RichText::new("⬇ Sample-Download (YouTube, SoundCloud, Bandcamp …)").strong())
            .id_salt("dl_section")
            .open(if app.fetch.download.is_some() { Some(true) } else { None })
            .default_open(open)
            .show(ui, |ui| download_ui(app, ui));
    });
}

fn download_ui(app: &mut TStudio, ui: &mut Ui) {
    if app.fetch.versions.is_none() {
        let f = app.fetch.tools.ffmpeg.as_deref().and_then(tstudio_fetch::tool_version);
        let y = app.fetch.tools.ytdlp.as_deref().and_then(tstudio_fetch::tool_version);
        app.fetch.versions = Some((f, y));
    }
    let (fv, yv) = app.fetch.versions.clone().unwrap_or_default();
    let mut redetect = false;
    if tool_row(ui, "yt-dlp", &app.fetch.tools.ytdlp, yv.as_ref(), "winget install yt-dlp.yt-dlp") {
        if let Some(p) = rfd::FileDialog::new().add_filter("yt-dlp", &["exe"]).pick_file() {
            app.settings.ytdlp_path = Some(p);
            redetect = true;
        }
    }
    if tool_row(ui, "ffmpeg", &app.fetch.tools.ffmpeg, fv.as_ref(), "winget install Gyan.FFmpeg") {
        if let Some(p) = rfd::FileDialog::new().add_filter("ffmpeg", &["exe"]).pick_file() {
            app.settings.ffmpeg_path = Some(p);
            redetect = true;
        }
    }
    if ui.small_button("⟳ Tools neu suchen").on_hover_text("Nach einer Installation hier klicken (ggf. App neu starten, damit PATH aktualisiert ist)").clicked() {
        redetect = true;
    }
    if redetect {
        app.settings.save(&app.paths.settings());
        app.fetch.redetect(&app.settings);
    }
    ui.add_space(4.0);
    let out_dir = app.settings.output_dir().join("Downloads");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.fetch.url).hint_text("URL einfügen (YouTube, SoundCloud, Bandcamp …)").desired_width(420.0));
        ui.selectable_value(&mut app.fetch.format, AudioFormat::Mp3, "MP3");
        ui.selectable_value(&mut app.fetch.format, AudioFormat::Wav, "WAV");
        if app.fetch.download.is_some() {
            if ui.button("Abbrechen").clicked() {
                if let Some(d) = &app.fetch.download {
                    d.cancel();
                }
            }
        } else {
            let ready = app.fetch.tools.ytdlp.is_some() && app.fetch.tools.ffmpeg.is_some() && !app.fetch.url.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new(RichText::new("⬇ Download").strong())).clicked() {
                app.fetch.log.clear();
                app.fetch.progress = 0.0;
                app.fetch.last_error = None;
                match tstudio_fetch::start_download(&app.fetch.tools, &app.fetch.url, &out_dir, app.fetch.format) {
                    Ok(h) => app.fetch.download = Some(h),
                    Err(e) => app.fetch.last_error = Some(e),
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.weak(format!("Ziel: {}", out_dir.display()));
        if ui.small_button("Ändern…").clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                app.settings.sample_out_dir = Some(d);
                app.settings.save(&app.paths.settings());
            }
        }
    });
    if app.fetch.download.is_some() {
        ui.add(egui::ProgressBar::new(app.fetch.progress).show_percentage().animate(true));
    }
    if let Some(e) = &app.fetch.last_error {
        ui.colored_label(Color32::from_rgb(240, 110, 90), e);
    }
    if !app.fetch.log.is_empty() {
        egui::CollapsingHeader::new("Log").id_salt("dl_log").show(ui, |ui| {
            for l in app.fetch.log.iter().rev().take(12) {
                ui.label(RichText::new(l).monospace().small());
            }
        });
    }
    if !app.fetch.downloads.is_empty() {
        ui.label(RichText::new("Letzte Downloads").small().color(TEXT_DIM));
        let list = app.fetch.downloads.clone();
        for p in list.iter().take(8) {
            if ui.link(p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()).on_hover_text("Im Sample-Cutter öffnen").clicked() {
                app.fetch.open_in_trim(p);
            }
        }
    }
    ui.weak("Hinweis: Lade nur Material, das du verwenden darfst.");
}

