//! Sample cutter: chop hits/loops out of MP3s & co. at keyboard speed, optionally isolated with
//! zero-phase EQ / mid-side, saved straight into the sample library (Cuts/<Kategorie>).
//!
//! Keys (Fetch view, file loaded): Space play/stop · ←/→ previous/next hit · Shift+←/→ end to
//! previous/next hit · Enter save · 1–9 save as category · L loop · I isolation A/B · Z zoom ·
//! Shift+Z whole file · H save every hit in the selection · Esc stop.

use crate::app::{TStudio, View};
use crate::samples::Peaks;
use crate::ui::widgets::{self, SampleDrag, ACCENT, BG_DARK, TEXT_DIM};
use crossbeam_channel::Receiver;
use egui::{pos2, vec2, Align2, Color32, FontId, Key, Rect, RichText, Sense, Stroke, Ui};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tstudio_dsp::isolate::{log_freqs, spectrum_db, IsolateParams, StereoMode, SLOPES};
use tstudio_dsp::SampleData;
use tstudio_library::{unique_path, write_wav, WavFormat};

pub const CATEGORIES: [&str; 9] = ["Kick", "Snare", "Clap", "Hat", "Perc", "Bass", "Vocal", "Loop", "FX"];
const WAVE_COLOR: Color32 = Color32::from_rgb(110, 180, 240);
const ISO_COLOR: Color32 = Color32::from_rgb(250, 190, 90);
const HIT_COLOR: Color32 = Color32::from_rgb(240, 140, 70);

/// Result of decoding a file for the cutter (done off the UI thread).
pub struct Loaded {
    pub data: SampleData,
    pub peaks: Peaks,
    pub hits: Vec<usize>,
}

pub fn load(path: &Path, sensitivity: f32) -> Result<Loaded, String> {
    let data = tstudio_library::decode_file(path)?;
    let peaks = Peaks::of(&data);
    let hits = tstudio_dsp::analysis::transients(&data.mono(), data.sample_rate, sensitivity, 30.0);
    Ok(Loaded { data, peaks, hits })
}

enum TrimDrag {
    Start,
    End,
    New(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum EqHandle {
    Hp,
    Lp,
    Peak,
}

struct Play {
    t0: f64,
    from: usize,
    to: usize,
    looped: bool,
}

pub struct Trim {
    pub path: Option<PathBuf>,
    pub data: Option<Arc<SampleData>>,
    peaks: Option<Arc<Peaks>>,
    pub sel: (usize, usize),
    view: (f64, f64),
    pub fade_in_ms: f32,
    pub fade_out_ms: f32,
    pub normalize: bool,
    pub trim_tail: bool,
    pub name: String,
    pub mp3_kbps: u32,
    beats: u32,
    drag: Option<TrimDrag>,
    // hits
    pub hits: Vec<usize>,
    pub sensitivity: f32,
    pub snap_hits: bool,
    // isolation
    pub iso: IsolateParams,
    pub iso_on: bool,
    processed: Option<(IsolateParams, Arc<SampleData>, Arc<Peaks>)>,
    iso_job: Option<(IsolateParams, Receiver<(SampleData, Peaks)>)>,
    iso_changed: Option<f64>,
    eq_drag: Option<EqHandle>,
    spectrum: Option<((usize, usize, usize), Vec<f32>)>,
    // saving / playback
    pub category: usize,
    pub saved: Vec<PathBuf>,
    pub loop_play: bool,
    playing: Option<Play>,
}

impl Default for Trim {
    fn default() -> Self {
        Self {
            path: None,
            data: None,
            peaks: None,
            sel: (0, 0),
            view: (0.0, 1.0),
            fade_in_ms: 1.0,
            fade_out_ms: 10.0,
            normalize: true,
            trim_tail: true,
            name: String::new(),
            mp3_kbps: 320,
            beats: 4,
            drag: None,
            hits: Vec::new(),
            sensitivity: 0.5,
            snap_hits: true,
            iso: IsolateParams::default(),
            iso_on: false,
            processed: None,
            iso_job: None,
            iso_changed: None,
            eq_drag: None,
            spectrum: None,
            category: 0,
            saved: Vec::new(),
            loop_play: false,
            playing: None,
        }
    }
}

impl Trim {
    pub fn set_loaded(&mut self, path: PathBuf, l: Loaded) {
        let len = l.data.len();
        self.peaks = Some(Arc::new(l.peaks));
        self.hits = l.hits;
        self.sel = (0, len);
        self.view = (0.0, len as f64);
        self.name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        self.data = Some(Arc::new(l.data));
        self.path = Some(path);
        self.processed = None;
        self.iso_job = None;
        self.spectrum = None;
        self.playing = None;
        self.drag = None;
    }

    fn sr(&self) -> f64 {
        self.data.as_ref().map(|d| d.sample_rate).unwrap_or(44100).max(1) as f64
    }

    fn secs(&self, samples: usize) -> f64 {
        samples as f64 / self.sr()
    }

    fn base_name(&self) -> String {
        let n = self.name.trim();
        if n.is_empty() {
            "cut".into()
        } else {
            n.to_string()
        }
    }

    fn iso_active(&self) -> bool {
        self.iso_on && !self.iso.is_neutral()
    }

    /// Fully processed file, if it matches the current isolation settings.
    fn processed_now(&self) -> Option<&(IsolateParams, Arc<SampleData>, Arc<Peaks>)> {
        self.processed.as_ref().filter(|p| self.iso_active() && p.0 == self.iso)
    }

    /// Hit segment [a, b) around sample `s`.
    fn hit_around(&self, s: usize) -> (usize, usize) {
        let len = self.data.as_ref().map(|d| d.len()).unwrap_or(0);
        let a = self.hits.iter().rev().find(|h| **h <= s).copied().unwrap_or(0);
        let b = self.hits.iter().find(|h| **h > s).copied().unwrap_or(len);
        (a, b.max(a + 1))
    }

    fn next_hit_after(&self, s: usize) -> Option<usize> {
        self.hits.iter().find(|h| **h > s + 16).copied()
    }

    fn prev_hit_before(&self, s: usize) -> Option<usize> {
        self.hits.iter().rev().find(|h| **h + 16 < s).copied()
    }

    /// Keeps the selection visible.
    fn reveal_sel(&mut self) {
        let (v0, v1) = self.view;
        let span = v1 - v0;
        let (a, b) = (self.sel.0 as f64, self.sel.1 as f64);
        if a < v0 || b > v1 {
            let len = self.data.as_ref().map(|d| d.len()).unwrap_or(0) as f64;
            let span = if b - a > span * 0.9 { ((b - a) * 1.3).min(len.max(1.0)) } else { span };
            let nv0 = (a - span * 0.2).clamp(0.0, (len - span).max(0.0));
            self.view = (nv0, nv0 + span);
        }
    }

    fn recompute_hits(&mut self) {
        if let Some(d) = &self.data {
            self.hits = tstudio_dsp::analysis::transients(&d.mono(), d.sample_rate, self.sensitivity, 30.0);
        }
    }

    fn poll_job(&mut self) {
        if let Some((p, rx)) = &self.iso_job {
            if let Ok((d, peaks)) = rx.try_recv() {
                self.processed = Some((*p, Arc::new(d), Arc::new(peaks)));
                self.iso_job = None;
            }
        }
    }
}

/// Nearest zero crossing of the mono sum within `radius` samples.
fn zero_near(d: &SampleData, i: usize, radius: usize) -> usize {
    let m = |k: usize| d.left[k] + d.right[k];
    if d.len() < 2 || i == 0 || i >= d.len() {
        return i.min(d.len());
    }
    for r in 0..radius {
        for k in [i.saturating_sub(r), i + r] {
            if k >= 1 && k < d.len() && (m(k - 1) <= 0.0) != (m(k) <= 0.0) {
                return k;
            }
        }
    }
    i
}

/// Audio of [a, b) with isolation, tail trim, fades and normalisation applied.
fn render_range(t: &Trim, a: usize, b: usize) -> Option<SampleData> {
    let d = t.data.as_ref()?;
    let (mut a, mut b) = (a.min(b), a.max(b).min(d.len()));
    let zr = (d.sample_rate / 1000) as usize;
    a = zero_near(d, a, zr);
    b = zero_near(d, b, zr).max(a + 1).min(d.len());
    if b <= a + 1 {
        return None;
    }
    let mut out = match t.processed_now() {
        Some((_, full, _)) => full.slice(a, b),
        None if t.iso_active() => t.iso.process_range(d, a, b),
        None => d.slice(a, b),
    };
    if t.trim_tail {
        // drop trailing audio below -60 dBFS (keep 5 ms)
        let thr = 0.001f32;
        let keep = (d.sample_rate / 200) as usize;
        if let Some(last) = (0..out.len()).rev().find(|i| out.left[*i].abs() > thr || out.right[*i].abs() > thr) {
            let end = (last + keep).min(out.len()).max(1);
            out.left.truncate(end);
            out.right.truncate(end);
        }
    }
    let sr = d.sample_rate as f32;
    out.fade((t.fade_in_ms * 0.001 * sr) as usize, (t.fade_out_ms * 0.001 * sr) as usize);
    if t.normalize {
        out.normalize(0.98);
    }
    Some(out)
}

/// `dir/stem NN.ext` with the first free number.
fn numbered_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let clean: String = stem.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().to_string();
    for n in 1..10000 {
        let p = dir.join(format!("{clean} {n:02}.{ext}"));
        if !p.exists() {
            return p;
        }
    }
    unique_path(dir, &clean, ext)
}

pub fn cut_dir(app: &TStudio) -> PathBuf {
    app.settings.output_dir().join("Cuts")
}

/// Makes sure saved cuts show up in the library.
fn ensure_in_library(app: &mut TStudio) {
    let dir = cut_dir(app);
    if !app.settings.library_roots.iter().any(|r| dir.starts_with(r)) {
        let base = app.settings.output_dir();
        app.settings.library_roots.push(base);
        app.settings.save(&app.paths.settings());
    }
    app.library.rescan(&app.settings);
}

fn save_range(app: &TStudio, a: usize, b: usize, cat: Option<usize>) -> Result<PathBuf, String> {
    let t = &app.fetch.trim;
    let d = render_range(t, a, b).ok_or("Leere Auswahl")?;
    let base = cut_dir(app);
    let (dir, stem) = match cat {
        Some(c) => (base.join(CATEGORIES[c]), format!("{} {}", t.base_name(), CATEGORIES[c])),
        None => (base, t.base_name()),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = numbered_path(&dir, &stem, "wav");
    write_wav(&path, &d, WavFormat::Pcm24)?;
    Ok(path)
}

enum Act {
    PlaySel,
    PlayFrom(usize),
    Stop,
    Save(Option<usize>),
    SaveHits,
    SaveFull,
    SaveMp3,
    Lossless,
    ToRack,
    ToPlaylist,
    Preview(PathBuf),
    Rack(PathBuf),
}

/// Free folder <output>/Stems/<name> (adds " 2", " 3" … if it exists).
fn stem_dir(app: &TStudio, name: &str) -> PathBuf {
    let base = app.settings.output_dir().join("Stems");
    let first = base.join(name);
    if !first.exists() {
        return first;
    }
    (2..).map(|i| base.join(format!("{name} {i}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Starts Demucs on the loaded file (CPU only, low priority) or explains how to install it.
fn start_stem_job(app: &mut TStudio) {
    use tstudio_fetch::stems;
    let Some(data) = app.fetch.trim.data.clone() else { return };
    let Some(python) = stems::find_python(app.settings.python_path.as_deref()) else {
        app.fetch.stem_result = Some(Err("Python wurde nicht gefunden".into()));
        return;
    };
    let name = app.fetch.trim.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Song".into());
    let dest = stem_dir(app, &name);
    let threads = stems::thread_count(app.settings.stem_threads);
    match stems::start_stems(&python, &data, &dest, &name, threads) {
        Ok(h) => {
            app.fetch.stems = Some(h);
            app.fetch.stem_progress = 0.0;
            app.fetch.stem_log = format!("startet auf {threads} CPU-Kern(en)…");
            app.fetch.stem_started = Some(std::time::Instant::now());
            app.fetch.stem_result = None;
        }
        Err(e) => app.fetch.stem_result = Some(Err(e)),
    }
}

/// "🎚 Stems trennen" row: progress while Demucs runs, buttons for the finished stems.
fn stems_bar(app: &mut TStudio, ui: &mut Ui) {
    use tstudio_fetch::stems;
    if app.fetch.trim.data.is_none() && app.fetch.stems.is_none() && app.fetch.stem_result.is_none() {
        return;
    }
    let mut open: Option<PathBuf> = None;
    let mut start = false;
    ui.horizontal_wrapped(|ui| {
        if let Some(h) = &app.fetch.stems {
            ui.spinner();
            ui.add(egui::ProgressBar::new(app.fetch.stem_progress).desired_width(220.0).show_percentage());
            let secs = app.fetch.stem_started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            ui.weak(format!("Stems werden getrennt (nur CPU) · {}:{:02}", secs / 60, secs % 60));
            if ui.small_button("Abbrechen").clicked() {
                h.cancel();
            }
            let log: String = app.fetch.stem_log.chars().take(90).collect();
            if !log.is_empty() {
                ui.label(RichText::new(log).small().color(TEXT_DIM));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        if ui
            .add_enabled(app.fetch.trim.data.is_some(), egui::Button::new("🎚 Stems trennen"))
            .on_hover_text("KI-Trennung (Demucs) in Drums, Bass, Vocals und Rest – läuft nur auf der CPU mit niedriger Priorität.\nDauert je nach Song 1–4 Minuten; TStudio bleibt bedienbar.")
            .clicked()
        {
            start = true;
        }
        match &app.fetch.stem_result {
            Some(Ok(dir)) => {
                ui.label(RichText::new("Stems:").strong());
                let files: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
                for (_, label) in stems::STEMS {
                    let suffix = format!(" - {label}");
                    if let Some(f) = files.iter().find(|p| p.file_stem().map(|s| s.to_string_lossy().ends_with(&suffix)).unwrap_or(false)) {
                        if ui.button(label).on_hover_text(format!("{} im Sample-Cutter öffnen", f.display())).clicked() {
                            open = Some(f.clone());
                        }
                    }
                }
                if ui.small_button("📂").on_hover_text(dir.display().to_string()).clicked() {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
                ui.weak("Tipp: Drums öffnen → Preset „Kick/Sub“ → Taste 1 speichert den Kick");
            }
            Some(Err(e)) => {
                ui.colored_label(Color32::from_rgb(240, 110, 90), format!("⚠ {e}"));
            }
            None => {}
        }
    });
    if let Some(Err(_)) = &app.fetch.stem_result {
        ui.horizontal_wrapped(|ui| {
            ui.weak("Einmalig installieren (Eingabeaufforderung, nacheinander):");
            if ui.small_button("📋 Befehle kopieren").clicked() {
                ui.ctx().copy_text(stems::INSTALL_HINT.to_string());
            }
        });
        ui.label(RichText::new(stems::INSTALL_HINT).monospace().small());
    }
    if start {
        start_stem_job(app);
    }
    if let Some(f) = open {
        app.fetch.open_in_trim(&f);
    }
}

pub fn show(app: &mut TStudio, ui: &mut Ui, loading: bool) {
    app.fetch.trim.poll_job();
    let mut acts: Vec<Act> = Vec::new();

    ui.horizontal(|ui| {
        ui.heading("Sample-Cutter");
        if ui.button("Datei öffnen…").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("Audio", tstudio_library::AUDIO_EXTS).pick_file() {
                app.fetch.open_in_trim(&p);
            }
        }
        if loading {
            ui.spinner();
            ui.weak("lädt…");
        }
        if let (Some(p), Some(d)) = (&app.fetch.trim.path, &app.fetch.trim.data) {
            ui.weak(format!(
                "{} · {:.2} s · {} Hz · {} Hits",
                p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                d.duration_secs(),
                d.sample_rate,
                app.fetch.trim.hits.len()
            ));
        }
    });

    stems_bar(app, ui);

    let now = ui.input(|i| i.time);
    let pointer_down = ui.input(|i| i.pointer.any_down());

    // ------------------------------------------------------------ overview + waveform
    let (ov_rect, ov_resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 200.0), Sense::click_and_drag());
    let p = ui.painter_at(rect.union(ov_rect));
    p.rect_filled(ov_rect, 3.0, Color32::from_rgb(18, 20, 24));
    p.rect_filled(rect, 4.0, Color32::from_rgb(22, 24, 28));
    for r in [&resp, &ov_resp] {
        if let Some(d) = r.dnd_release_payload::<SampleDrag>() {
            app.fetch.open_in_trim(&d.path);
        }
    }
    let t = &mut app.fetch.trim;
    let (Some(data), Some(peaks)) = (t.data.clone(), t.peaks.clone()) else {
        p.text(rect.center(), Align2::CENTER_CENTER, "MP3/WAV öffnen, aus der Library hierher ziehen oder oben herunterladen", FontId::proportional(13.0), TEXT_DIM);
        return;
    };
    let len = data.len() as f64;
    let sr = data.sample_rate as f64;

    // overview: click/drag moves the view
    if let Some(pt) = ov_resp.interact_pointer_pos() {
        if ov_resp.dragged() || ov_resp.clicked() {
            let c = ((pt.x - ov_rect.left()) / ov_rect.width()) as f64 * len;
            let span = t.view.1 - t.view.0;
            let nv0 = (c - span / 2.0).clamp(0.0, (len - span).max(0.0));
            t.view = (nv0, nv0 + span);
        }
    }

    // zoom / scroll
    {
        let (v0, v1) = t.view;
        let s_of = |x: f32| (v0 + (x - rect.left()) as f64 / rect.width() as f64 * (v1 - v0)).clamp(0.0, len);
        if ui.rect_contains_pointer(rect) {
            let (scroll, zoom, mods, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers, i.pointer.hover_pos()));
            let wheel = if zoom != 1.0 { zoom } else if !mods.shift && scroll.y != 0.0 { (scroll.y * 0.004).exp() } else { 1.0 };
            if wheel != 1.0 {
                let center = hover.map(|h| s_of(h.x)).unwrap_or((v0 + v1) / 2.0);
                let span = ((v1 - v0) / wheel as f64).clamp(64.0, len.max(64.0));
                let frac = (center - v0) / (v1 - v0);
                let nv0 = (center - span * frac).clamp(0.0, (len - span).max(0.0));
                t.view = (nv0, nv0 + span);
            } else if mods.shift && (scroll.y != 0.0 || scroll.x != 0.0) {
                let span = v1 - v0;
                let d = -(scroll.y + scroll.x) as f64 / rect.width() as f64 * span;
                let nv0 = (v0 + d).clamp(0.0, (len - span).max(0.0));
                t.view = (nv0, nv0 + span);
            }
        }
    }
    let (v0, v1) = t.view;
    let x_of = |s: f64| rect.left() + ((s - v0) / (v1 - v0)) as f32 * rect.width();
    let s_of = |x: f32| (v0 + (x - rect.left()) as f64 / rect.width() as f64 * (v1 - v0)).clamp(0.0, len);
    let alt = ui.input(|i| i.modifiers.alt);
    let snap = |t: &Trim, s: usize| -> usize {
        if !t.snap_hits || alt {
            return s;
        }
        let px = (v1 - v0) / rect.width() as f64 * 10.0;
        t.hits.iter().copied().filter(|h| (*h as f64 - s as f64).abs() < px).min_by_key(|h| (*h as i64 - s as i64).abs()).unwrap_or(s)
    };

    // selection: drag = select, edges = adjust, click = play from here, double click = hit
    if let Some(pt) = resp.interact_pointer_pos() {
        let s = s_of(pt.x) as usize;
        if resp.drag_started() {
            let ds = (pt.x - x_of(t.sel.0 as f64)).abs();
            let de = (pt.x - x_of(t.sel.1 as f64)).abs();
            t.drag = Some(if ds < 8.0 && ds <= de {
                TrimDrag::Start
            } else if de < 8.0 {
                TrimDrag::End
            } else {
                TrimDrag::New(snap(t, s))
            });
        }
        if resp.dragged() {
            let s = snap(t, s);
            match t.drag {
                Some(TrimDrag::Start) => t.sel.0 = s.min(t.sel.1.saturating_sub(1)),
                Some(TrimDrag::End) => t.sel.1 = s.max(t.sel.0 + 1),
                Some(TrimDrag::New(a)) => t.sel = (a.min(s), a.max(s).max(a.min(s) + 1)),
                None => {}
            }
        }
        if resp.double_clicked() {
            t.sel = t.hit_around(s);
            acts.push(Act::PlaySel);
        } else if resp.clicked() {
            acts.push(Act::PlayFrom(s));
        }
    }
    if resp.drag_stopped() {
        if matches!(t.drag, Some(TrimDrag::New(_))) {
            acts.push(Act::PlaySel);
        }
        t.drag = None;
    }
    if let Some(h) = resp.hover_pos() {
        let ds = (h.x - x_of(t.sel.0 as f64)).abs();
        let de = (h.x - x_of(t.sel.1 as f64)).abs();
        if ds < 8.0 || de < 8.0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }

    // painting
    let iso_peaks = t.processed_now().map(|p| p.2.clone());
    let wave_rect = rect.shrink2(vec2(0.0, 14.0));
    match &iso_peaks {
        Some(ip) => {
            widgets::draw_waveform(&p, wave_rect, &peaks, v0, v1, Color32::from_gray(70));
            widgets::draw_waveform(&p, wave_rect, ip, v0, v1, ISO_COLOR);
        }
        None => widgets::draw_waveform(&p, wave_rect, &peaks, v0, v1, WAVE_COLOR),
    }
    for h in t.hits.iter().filter(|h| (**h as f64) >= v0 && (**h as f64) <= v1) {
        let x = x_of(*h as f64);
        p.line_segment([pos2(x, rect.bottom() - 9.0), pos2(x, rect.bottom())], Stroke::new(1.5, HIT_COLOR));
    }
    let (a, b) = (x_of(t.sel.0 as f64), x_of(t.sel.1 as f64));
    p.rect_filled(Rect::from_min_max(rect.min, pos2(a.clamp(rect.left(), rect.right()), rect.bottom())), 0.0, Color32::from_black_alpha(150));
    p.rect_filled(Rect::from_min_max(pos2(b.clamp(rect.left(), rect.right()), rect.top()), rect.max), 0.0, Color32::from_black_alpha(150));
    for x in [a, b] {
        if x >= rect.left() - 1.0 && x <= rect.right() + 1.0 {
            p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, ACCENT));
        }
    }
    // time ruler
    let span_s = (v1 - v0) / sr;
    let step = [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0].into_iter().find(|s| span_s / s < 12.0).unwrap_or(120.0);
    let mut ts = (v0 / sr / step).ceil() * step;
    while ts * sr < v1 {
        let x = x_of(ts * sr);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.top() + 8.0)], Stroke::new(1.0, TEXT_DIM));
        p.text(pos2(x + 2.0, rect.top() + 1.0), Align2::LEFT_TOP, format!("{ts:.2}s"), FontId::proportional(9.0), TEXT_DIM);
        ts += step;
    }
    // overview
    {
        let ox = |s: f64| ov_rect.left() + (s / len.max(1.0)) as f32 * ov_rect.width();
        widgets::draw_waveform(&p, ov_rect.shrink(2.0), &peaks, 0.0, len, Color32::from_gray(110));
        p.rect_filled(Rect::from_min_max(pos2(ox(t.sel.0 as f64), ov_rect.top()), pos2(ox(t.sel.1 as f64).max(ox(t.sel.0 as f64) + 1.0), ov_rect.bottom())), 0.0, ACCENT.gamma_multiply(0.25));
        p.rect_stroke(Rect::from_min_max(pos2(ox(v0), ov_rect.top()), pos2(ox(v1), ov_rect.bottom())), 2.0, Stroke::new(1.0, Color32::WHITE));
    }
    // playhead (estimated from the time playback started)
    if let Some(pl) = &t.playing {
        let span = pl.to.saturating_sub(pl.from).max(1) as f64;
        let mut el = (now - pl.t0) * sr;
        if pl.looped {
            el %= span;
        }
        if el > span {
            t.playing = None;
        } else {
            let x = x_of(pl.from as f64 + el);
            if x >= rect.left() && x <= rect.right() {
                p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, Color32::WHITE));
            }
            ui.ctx().request_repaint();
        }
    }

    // ------------------------------------------------------------ keyboard
    if !ui.ctx().wants_keyboard_input() {
        let (keys, shift, ctrl) = ui.input(|i| {
            let k = [
                Key::Space, Key::Enter, Key::ArrowLeft, Key::ArrowRight, Key::L, Key::I, Key::Z, Key::H, Key::Escape, Key::A,
            ]
            .map(|k| i.key_pressed(k));
            (k, i.modifiers.shift, i.modifiers.command)
        });
        let nums = ui.input(|i| [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9].map(|k| i.key_pressed(k)));
        let [space, enter, left, right, l, iso, z, h, esc, a] = keys;
        if space {
            acts.push(if t.playing.is_some() { Act::Stop } else { Act::PlaySel });
        }
        if esc {
            acts.push(Act::Stop);
        }
        if enter && !ctrl {
            acts.push(Act::Save(Some(t.category)));
        }
        for (k, pressed) in nums.iter().enumerate() {
            if *pressed && !ctrl {
                t.category = k;
                acts.push(Act::Save(Some(k)));
            }
        }
        if right && !shift {
            if let Some(n) = t.next_hit_after(t.sel.0) {
                t.sel = (n, t.next_hit_after(n).unwrap_or(data.len()));
                t.reveal_sel();
                acts.push(Act::PlaySel);
            }
        }
        if left && !shift {
            if let Some(n) = t.prev_hit_before(t.sel.0) {
                t.sel = (n, t.next_hit_after(n).unwrap_or(data.len()));
                t.reveal_sel();
                acts.push(Act::PlaySel);
            }
        }
        if right && shift {
            t.sel.1 = t.next_hit_after(t.sel.1).unwrap_or(data.len());
            t.reveal_sel();
            acts.push(Act::PlaySel);
        }
        if left && shift {
            if let Some(n) = t.prev_hit_before(t.sel.1).filter(|n| *n > t.sel.0) {
                t.sel.1 = n;
                acts.push(Act::PlaySel);
            }
        }
        if l {
            t.loop_play = !t.loop_play;
            if t.playing.is_some() {
                acts.push(Act::PlaySel);
            }
        }
        if iso {
            t.iso_on = !t.iso_on;
            t.iso_changed = Some(now);
            if t.playing.is_some() {
                acts.push(Act::PlaySel);
            }
        }
        if z && !shift {
            let pad = ((t.sel.1 - t.sel.0) as f64 * 0.08).max(32.0);
            t.view = ((t.sel.0 as f64 - pad).max(0.0), (t.sel.1 as f64 + pad).min(len));
        }
        if z && shift {
            t.view = (0.0, len);
        }
        if h {
            acts.push(Act::SaveHits);
        }
        if a && ctrl {
            t.sel = (0, data.len());
        }
    }

    // ------------------------------------------------------------ transport row
    let sel_len = t.secs(t.sel.1.saturating_sub(t.sel.0));
    let bpm = app.project.bpm;
    ui.horizontal_wrapped(|ui| {
        let t = &mut app.fetch.trim;
        if ui.button("▶ Auswahl").on_hover_text("Leertaste").clicked() {
            acts.push(Act::PlaySel);
        }
        if ui.button("■").on_hover_text("Esc").clicked() {
            acts.push(Act::Stop);
        }
        if ui.toggle_value(&mut t.loop_play, "⟳ Loop").on_hover_text("L").changed() && t.playing.is_some() {
            acts.push(Act::PlaySel);
        }
        if ui.button("◀ Hit").on_hover_text("←").clicked() {
            if let Some(n) = t.prev_hit_before(t.sel.0) {
                t.sel = (n, t.next_hit_after(n).unwrap_or(data.len()));
                t.reveal_sel();
                acts.push(Act::PlaySel);
            }
        }
        if ui.button("Hit ▶").on_hover_text("→").clicked() {
            if let Some(n) = t.next_hit_after(t.sel.0) {
                t.sel = (n, t.next_hit_after(n).unwrap_or(data.len()));
                t.reveal_sel();
                acts.push(Act::PlaySel);
            }
        }
        if ui.button("Zoom").on_hover_text("Z (Shift+Z = ganze Datei)").clicked() {
            let pad = ((t.sel.1 - t.sel.0) as f64 * 0.08).max(32.0);
            t.view = ((t.sel.0 as f64 - pad).max(0.0), (t.sel.1 as f64 + pad).min(len));
        }
        if ui.button("Alles").clicked() {
            t.view = (0.0, len);
        }
        ui.separator();
        ui.label("Hits");
        let r = ui.add(egui::Slider::new(&mut t.sensitivity, 0.0..=1.0).show_value(false)).on_hover_text("Empfindlichkeit der Hit-Erkennung (mehr = mehr Schnittpunkte)");
        if r.drag_stopped() || (r.changed() && !r.dragged()) {
            t.recompute_hits();
        }
        ui.checkbox(&mut t.snap_hits, "Einrasten").on_hover_text("Auswahlkanten rasten an Hits ein (Alt gedrückt = frei)");
        ui.separator();
        let mut s0 = t.secs(t.sel.0);
        let mut s1 = t.secs(t.sel.1);
        if ui.add(egui::DragValue::new(&mut s0).speed(0.001).range(0.0..=len / sr).fixed_decimals(3).prefix("Start ").suffix(" s")).changed() {
            t.sel.0 = ((s0 * sr) as usize).min(t.sel.1.saturating_sub(1));
        }
        if ui.add(egui::DragValue::new(&mut s1).speed(0.001).range(0.0..=len / sr).fixed_decimals(3).prefix("Ende ").suffix(" s")).changed() {
            t.sel.1 = ((s1 * sr) as usize).clamp(t.sel.0 + 1, data.len());
        }
        ui.label(RichText::new(format!("{sel_len:.3} s")).monospace());
        ui.separator();
        let beat = 60.0 / bpm * sr;
        let n_beats = ((t.sel.1 - t.sel.0) as f64 / beat).round().max(1.0);
        if ui
            .button(format!("⇥ {} Beats @ {:.0}", n_beats, bpm))
            .on_hover_text("Ende so setzen, dass die Auswahl genau ganze Beats im Projekttempo lang ist (für Loops)")
            .clicked()
        {
            t.sel.1 = ((t.sel.0 as f64 + n_beats * beat) as usize).min(data.len());
            acts.push(Act::PlaySel);
        }
        ui.add(egui::DragValue::new(&mut t.beats).range(1..=256).suffix(" Beats"));
        if sel_len > 0.0 {
            ui.label(RichText::new(format!("= {:.2} BPM", t.beats as f64 * 60.0 / sel_len)).color(ACCENT)).on_hover_text("Tempo, wenn die Auswahl genau so viele Beats lang ist");
        }
    });
    ui.add_space(4.0);

    // ------------------------------------------------------------ save | isolate
    ui.columns(2, |cols| {
        save_panel(app, &mut cols[0], &mut acts);
        iso_panel(app, &mut cols[1], &mut acts, now, pointer_down);
    });
    ui.weak("Tasten: Leertaste spielen · ←/→ Hit für Hit · Shift+←/→ Ende verlängern/kürzen · 1–9 als Kategorie speichern · Enter speichern · H alle Hits · L Loop · I Isolieren an/aus · Z Zoom · Doppelklick = Hit wählen · Klick = ab hier hören");

    // background isolation of the whole file (for the waveform and instant saves)
    let t = &mut app.fetch.trim;
    if t.iso_active() {
        let ready = t.processed.as_ref().map(|p| p.0 == t.iso).unwrap_or(false);
        let running = t.iso_job.as_ref().map(|j| j.0 == t.iso).unwrap_or(false);
        let settled = t.iso_changed.map(|c| now - c > 0.15).unwrap_or(true);
        if !ready && !running && settled && !pointer_down {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (params, d) = (t.iso, data.clone());
            std::thread::spawn(move || {
                let out = params.process(&d);
                let pk = Peaks::of(&out);
                let _ = tx.send((out, pk));
            });
            t.iso_job = Some((params, rx));
        }
        if !ready {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        }
    }

    run_actions(app, acts, now);
}

fn save_panel(app: &mut TStudio, ui: &mut Ui, acts: &mut Vec<Act>) {
    ui.label(RichText::new("Speichern").strong());
    ui.horizontal_wrapped(|ui| {
        let t = &mut app.fetch.trim;
        for (i, c) in CATEGORIES.iter().enumerate() {
            let sel = t.category == i;
            let b = egui::Button::new(RichText::new(format!("{} {c}", i + 1)).color(if sel { Color32::BLACK } else { Color32::WHITE })).fill(if sel { ACCENT } else { BG_DARK });
            if ui.add(b).on_hover_text(format!("Taste {} = sofort als {c} speichern", i + 1)).clicked() {
                t.category = i;
            }
        }
    });
    ui.horizontal(|ui| {
        let t = &mut app.fetch.trim;
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut t.name).desired_width(170.0));
        if ui.button(RichText::new(format!("💾 {} speichern", CATEGORIES[t.category])).strong()).on_hover_text("Enter").clicked() {
            acts.push(Act::Save(Some(t.category)));
        }
        if ui.button("✂ Alle Hits").on_hover_text("H – jeden Hit in der Auswahl als eigene Datei speichern").clicked() {
            acts.push(Act::SaveHits);
        }
    });
    ui.horizontal_wrapped(|ui| {
        let t = &mut app.fetch.trim;
        ui.add(egui::DragValue::new(&mut t.fade_in_ms).speed(0.2).range(0.0..=5000.0).prefix("Fade In ").suffix(" ms"));
        ui.add(egui::DragValue::new(&mut t.fade_out_ms).speed(0.5).range(0.0..=5000.0).prefix("Fade Out ").suffix(" ms"));
        ui.checkbox(&mut t.normalize, "Normalisieren");
        ui.checkbox(&mut t.trim_tail, "Stille am Ende weg").on_hover_text("Alles unter -60 dB am Ende abschneiden");
    });
    let ffmpeg = app.fetch.tools.ffmpeg.is_some();
    let is_mp3 = app.fetch.trim.path.as_ref().and_then(|p| p.extension()).map(|e| e.eq_ignore_ascii_case("mp3")).unwrap_or(false);
    ui.horizontal_wrapped(|ui| {
        if ui.button("→ Channel Rack").on_hover_text("Speichern und als Sampler-Kanal einfügen").clicked() {
            acts.push(Act::ToRack);
        }
        if ui.button("→ Playlist").on_hover_text("Speichern und am Playhead einfügen").clicked() {
            acts.push(Act::ToPlaylist);
        }
        if ui.add_enabled(ffmpeg, egui::Button::new("MP3")).on_hover_text("Auswahl als MP3 speichern").clicked() {
            acts.push(Act::SaveMp3);
        }
        let t = &mut app.fetch.trim;
        if ui
            .add_enabled(ffmpeg && is_mp3 && !t.iso_active(), egui::Button::new("⚡ MP3 verlustfrei"))
            .on_hover_text("Ohne Neu-Kodierung (±26 ms genau, ohne Fades/Isolieren)")
            .clicked()
        {
            acts.push(Act::Lossless);
        }
        egui::ComboBox::from_id_salt("mp3_kbps").selected_text(format!("{} kbps", t.mp3_kbps)).width(80.0).show_ui(ui, |ui| {
            for k in [128, 192, 256, 320] {
                ui.selectable_value(&mut t.mp3_kbps, k, format!("{k} kbps"));
            }
        });
    });
    let dir = cut_dir(app);
    ui.horizontal(|ui| {
        ui.weak(format!("Ziel: {}\\<Kategorie>", dir.display()));
        if ui.small_button("📂").on_hover_text("Ordner öffnen").clicked() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
        if ui.small_button("Ändern…").clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                app.settings.sample_out_dir = Some(d);
                app.settings.save(&app.paths.settings());
            }
        }
    });
    if !app.fetch.trim.saved.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new("Gerade geschnitten – ziehen in Playlist/Rack, ▶ hören").small().color(TEXT_DIM));
        egui::ScrollArea::vertical().id_salt("cuts_list").max_height(150.0).show(ui, |ui| {
            for path in app.fetch.trim.saved.iter().take(40) {
                ui.horizontal(|ui| {
                    if ui.small_button("▶").clicked() {
                        acts.push(Act::Preview(path.clone()));
                    }
                    if ui.small_button("+Rack").clicked() {
                        acts.push(Act::Rack(path.clone()));
                    }
                    let folder = path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    let r = ui.add(egui::Label::new(RichText::new(format!("{folder}/{name}")).small()).sense(Sense::click_and_drag()));
                    if r.drag_started() {
                        egui::DragAndDrop::set_payload(ui.ctx(), SampleDrag { path: path.clone() });
                    }
                    if r.clicked() {
                        acts.push(Act::Preview(path.clone()));
                    }
                });
            }
        });
    }
}

fn iso_panel(app: &mut TStudio, ui: &mut Ui, acts: &mut Vec<Act>, now: f64, pointer_down: bool) {
    let t = &mut app.fetch.trim;
    let before = (t.iso, t.iso_on);
    ui.horizontal(|ui| {
        ui.toggle_value(&mut t.iso_on, RichText::new("🎚 Isolieren").strong()).on_hover_text("I – an/aus zum A/B-Vergleich");
        let current = IsolateParams::presets().into_iter().find(|(_, p)| *p == t.iso).map(|(n, _)| n).unwrap_or("Eigene");
        egui::ComboBox::from_id_salt("iso_preset").selected_text(current).width(130.0).show_ui(ui, |ui| {
            for (name, p) in IsolateParams::presets() {
                if ui.selectable_label(t.iso == p, name).clicked() {
                    t.iso = p;
                    t.iso_on = !p.is_neutral();
                }
            }
        });
        if t.iso_job.is_some() {
            ui.spinner();
        }
        ui.weak(t.iso.describe());
    });

    // spectrum with draggable HP / LP / peak handles
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_rgb(20, 22, 26));
    let Some(data) = t.data.clone() else { return };
    let srate = data.sample_rate;
    let (fl, fh) = (20.0f32, 20000.0f32.min(srate as f32 * 0.49));
    let fx = |f: f32| rect.left() + ((f.max(fl) / fl).ln() / (fh / fl).ln()) * rect.width();
    let f_of = |x: f32| fl * (fh / fl).powf(((x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    let ry = |db: f32| rect.top() + (12.0 - db.clamp(-48.0, 12.0)) / 60.0 * rect.height();
    let r_of = |y: f32| 12.0 - (y - rect.top()) / rect.height() * 60.0;
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        let x = fx(f);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, Color32::from_gray(38)));
        let l = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        p.text(pos2(x + 2.0, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, l, FontId::proportional(9.0), TEXT_DIM);
    }
    p.line_segment([pos2(rect.left(), ry(0.0)), pos2(rect.right(), ry(0.0))], Stroke::new(1.0, Color32::from_gray(45)));

    // spectrum of the selection (cached; recomputed when the selection settles)
    let key = (t.sel.0, t.sel.1, data.len());
    let freqs = log_freqs(140, fl, fh);
    if t.spectrum.as_ref().map(|s| s.0) != Some(key) && !pointer_down {
        let (a, b) = (t.sel.0.min(data.len()), t.sel.1.min(data.len()));
        let mono: Vec<f32> = (a..b).map(|i| 0.5 * (data.left[i] + data.right[i])).collect();
        t.spectrum = Some((key, spectrum_db(&mono, srate, &freqs)));
    }
    if let Some((_, spec)) = &t.spectrum {
        let top = spec.iter().cloned().fold(-120.0f32, f32::max) + 3.0;
        let sy = |db: f32| rect.top() + ((top - db) / 72.0).clamp(0.0, 1.0) * rect.height();
        let orig: Vec<_> = freqs.iter().zip(spec).map(|(f, d)| pos2(fx(*f), sy(*d))).collect();
        p.add(egui::Shape::line(orig, Stroke::new(1.0, Color32::from_gray(110))));
        if t.iso_on && !t.iso.is_neutral() {
            let after: Vec<_> = freqs.iter().zip(spec).map(|(f, d)| pos2(fx(*f), sy(d + t.iso.response_db(*f, srate)))).collect();
            p.add(egui::Shape::line(after, Stroke::new(1.5, ISO_COLOR)));
        }
    }
    // response curve
    let on = t.iso_on;
    let curve_col = if on { ACCENT } else { Color32::from_gray(90) };
    let curve: Vec<_> = freqs.iter().map(|f| pos2(fx(*f), ry(t.iso.response_db(*f, srate)))).collect();
    p.add(egui::Shape::line(curve, Stroke::new(1.5, curve_col)));
    let hp_x = fx(if t.iso.hp_hz > 0.0 { t.iso.hp_hz } else { fl });
    let lp_x = fx(if t.iso.lp_hz > 0.0 { t.iso.lp_hz } else { fh });
    if t.iso.hp_hz > 0.0 {
        p.rect_filled(Rect::from_min_max(rect.min, pos2(hp_x, rect.bottom())), 0.0, Color32::from_black_alpha(if on { 120 } else { 50 }));
    }
    if t.iso.lp_hz > 0.0 {
        p.rect_filled(Rect::from_min_max(pos2(lp_x, rect.top()), rect.max), 0.0, Color32::from_black_alpha(if on { 120 } else { 50 }));
    }
    let hz = |f: f32| if f >= 1000.0 { format!("{:.1}k", f / 1000.0) } else { format!("{f:.0}") };
    for (x, label, active) in [(hp_x, format!("HP {}", if t.iso.hp_hz > 0.0 { hz(t.iso.hp_hz) } else { "aus".into() }), t.eq_drag == Some(EqHandle::Hp)), (lp_x, format!("LP {}", if t.iso.lp_hz > 0.0 { hz(t.iso.lp_hz) } else { "aus".into() }), t.eq_drag == Some(EqHandle::Lp))] {
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(if active { 2.5 } else { 1.5 }, curve_col));
        let anchor = if x > rect.center().x { Align2::RIGHT_TOP } else { Align2::LEFT_TOP };
        let tx = if x > rect.center().x { x - 4.0 } else { x + 4.0 };
        p.text(pos2(tx, rect.top() + 3.0), anchor, label, FontId::proportional(10.0), Color32::WHITE);
    }
    let peak_pos = pos2(fx(t.iso.peak_hz), ry(t.iso.peak_db));
    p.circle_filled(peak_pos, if t.eq_drag == Some(EqHandle::Peak) { 7.0 } else { 5.5 }, curve_col);
    p.circle_stroke(peak_pos, 7.5, Stroke::new(1.0, Color32::WHITE.gamma_multiply(0.5)));

    // interaction
    let hit_handle = |pt: egui::Pos2| -> Option<EqHandle> {
        if pt.distance(peak_pos) < 12.0 {
            Some(EqHandle::Peak)
        } else if (pt.x - hp_x).abs() < 9.0 {
            Some(EqHandle::Hp)
        } else if (pt.x - lp_x).abs() < 9.0 {
            Some(EqHandle::Lp)
        } else {
            None
        }
    };
    if let Some(h) = resp.hover_pos() {
        match hit_handle(h) {
            Some(EqHandle::Peak) => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                let s = ui.input(|i| i.smooth_scroll_delta.y);
                if s != 0.0 {
                    t.iso.peak_q = (t.iso.peak_q * (s * 0.004).exp()).clamp(0.1, 20.0);
                    t.iso_on = true;
                }
            }
            Some(_) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            None => {}
        }
    }
    if resp.drag_started() {
        t.eq_drag = resp.interact_pointer_pos().and_then(hit_handle);
    }
    if let (true, Some(pt), Some(h)) = (resp.dragged(), resp.interact_pointer_pos(), t.eq_drag) {
        let f = f_of(pt.x);
        t.iso_on = true;
        match h {
            EqHandle::Hp => {
                t.iso.hp_hz = if f <= fl * 1.05 { 0.0 } else { (f.round()).min(if t.iso.lp_hz > 0.0 { t.iso.lp_hz * 0.9 } else { fh }) };
            }
            EqHandle::Lp => {
                t.iso.lp_hz = if f >= fh * 0.97 { 0.0 } else { (f.round()).max(t.iso.hp_hz * 1.1) };
            }
            EqHandle::Peak => {
                t.iso.peak_hz = f.round();
                t.iso.peak_db = (r_of(pt.y) * 2.0).round() / 2.0;
                t.iso.peak_db = t.iso.peak_db.clamp(-24.0, 24.0);
            }
        }
    }
    if resp.drag_stopped() {
        t.eq_drag = None;
        if t.playing.is_some() {
            acts.push(Act::PlaySel);
        }
    }
    if resp.double_clicked() {
        match resp.interact_pointer_pos().and_then(hit_handle) {
            Some(EqHandle::Hp) => t.iso.hp_hz = 0.0,
            Some(EqHandle::Lp) => t.iso.lp_hz = 0.0,
            Some(EqHandle::Peak) => t.iso.peak_db = 0.0,
            None => {}
        }
    }
    resp.on_hover_text("Griffe ziehen: HP (links), LP (rechts), Band (Punkt; Mausrad = Breite). Doppelklick auf einen Griff = aus.");

    // numeric controls
    ui.horizontal_wrapped(|ui| {
        let mut hp = t.iso.hp_hz;
        if ui.add(egui::DragValue::new(&mut hp).speed(2.0).range(0.0..=20000.0).prefix("HP ").suffix(" Hz")).changed() {
            t.iso.hp_hz = hp;
            t.iso_on = true;
        }
        let mut lp = t.iso.lp_hz;
        if ui.add(egui::DragValue::new(&mut lp).speed(20.0).range(0.0..=22000.0).prefix("LP ").suffix(" Hz")).on_hover_text("0 = aus").changed() {
            t.iso.lp_hz = lp;
            t.iso_on = true;
        }
        egui::ComboBox::from_id_salt("iso_slope").selected_text(SLOPES.iter().find(|s| s.0 == t.iso.slope).map(|s| s.1).unwrap_or("?")).width(85.0).show_ui(ui, |ui| {
            for (n, l) in SLOPES {
                ui.selectable_value(&mut t.iso.slope, n, l);
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::DragValue::new(&mut t.iso.peak_hz).speed(5.0).range(20.0..=20000.0).prefix("Band ").suffix(" Hz"));
        ui.add(egui::DragValue::new(&mut t.iso.peak_db).speed(0.2).range(-24.0..=24.0).suffix(" dB"));
        ui.add(egui::DragValue::new(&mut t.iso.peak_q).speed(0.02).range(0.1..=20.0).prefix("Q "));
        ui.add(egui::DragValue::new(&mut t.iso.gain_db).speed(0.2).range(-24.0..=24.0).prefix("Gain ").suffix(" dB"));
    });
    ui.horizontal_wrapped(|ui| {
        for m in StereoMode::ALL {
            if ui.selectable_label(t.iso.stereo == m, m.label()).on_hover_text(match m {
                StereoMode::Mid => "Nur die Mitte: Vocals, Kick, Bass, Snare",
                StereoMode::Side => "Mitte entfernen (mono): Vocals/Kick/Bass raus, Pads/Hall/Stereo-Synths bleiben",
                _ => "",
            }).clicked() {
                t.iso.stereo = m;
                t.iso_on = true;
            }
        }
        ui.separator();
        if ui.add_enabled(t.iso_active(), egui::Button::new("💾 Ganze Datei isoliert")).on_hover_text("Den kompletten Song mit dieser Isolierung speichern").clicked() {
            acts.push(Act::SaveFull);
        }
    });
    if (t.iso, t.iso_on) != before {
        t.iso_changed = Some(now);
    }
}

fn run_actions(app: &mut TStudio, acts: Vec<Act>, now: f64) {
    let mut saved_any = false;
    for a in acts {
        match a {
            Act::Stop => {
                app.stop_preview();
                app.fetch.trim.playing = None;
            }
            Act::PlaySel | Act::PlayFrom(_) => {
                let t = &app.fetch.trim;
                let Some(d) = t.data.clone() else { continue };
                let (from, to, looped) = match a {
                    Act::PlayFrom(s) => (s.min(d.len()), d.len(), false),
                    _ => (t.sel.0, t.sel.1, t.loop_play),
                };
                // preview exactly what would be saved (isolation, fades, …); from-here plays the raw/isolated file
                let audio = match a {
                    Act::PlayFrom(_) => match t.processed_now() {
                        Some((_, full, _)) => Some(full.slice(from, to)),
                        None if t.iso_active() => Some(t.iso.process_range(&d, from, (from + d.sample_rate as usize * 20).min(to))),
                        None => Some(d.slice(from, to)),
                    },
                    _ => render_range(t, from, to),
                };
                let Some(audio) = audio else { continue };
                let target = app.engine.sample_rate;
                let audio = if audio.sample_rate != target { audio.resample_to(target) } else { audio };
                let to = from + (audio.len() as f64 * d.sample_rate as f64 / target as f64) as usize;
                app.send(tstudio_engine::Command::PreviewLoop(looped));
                app.preview_data(Arc::new(audio));
                app.fetch.trim.playing = Some(Play { t0: now, from, to, looped });
            }
            Act::Save(cat) => {
                let (a, b) = app.fetch.trim.sel;
                match save_range(app, a, b, cat) {
                    Ok(p) => {
                        app.set_status(format!("✂ Gespeichert: {}", p.display()));
                        app.fetch.trim.saved.insert(0, p);
                        saved_any = true;
                    }
                    Err(e) => app.set_error(format!("Speichern fehlgeschlagen: {e}")),
                }
            }
            Act::SaveHits => {
                let t = &app.fetch.trim;
                let (a, b) = t.sel;
                let mut cuts: Vec<usize> = std::iter::once(a).chain(t.hits.iter().copied().filter(|h| *h > a + 16 && *h < b)).collect();
                cuts.push(b);
                let min_len = (t.sr() * 0.02) as usize;
                let cat = Some(t.category);
                let mut n = 0;
                for w in cuts.windows(2) {
                    if w[1] - w[0] < min_len {
                        continue;
                    }
                    match save_range(app, w[0], w[1], cat) {
                        Ok(p) => {
                            app.fetch.trim.saved.insert(0, p);
                            n += 1;
                        }
                        Err(e) => {
                            app.set_error(format!("Speichern fehlgeschlagen: {e}"));
                            break;
                        }
                    }
                }
                if n > 0 {
                    app.set_status(format!("✂ {n} Hits als {} gespeichert", CATEGORIES[app.fetch.trim.category]));
                    saved_any = true;
                }
            }
            Act::SaveFull => {
                let t = &app.fetch.trim;
                let Some(d) = t.data.clone() else { continue };
                let out = match t.processed_now() {
                    Some((_, full, _)) => (**full).clone(),
                    None => t.iso.process(&d),
                };
                let preset = IsolateParams::presets().into_iter().find(|(_, p)| *p == t.iso).map(|(n, _)| n.replace(" / ", "-")).unwrap_or_else(|| "isoliert".into());
                let dir = cut_dir(app).join("Isoliert");
                let stem = format!("{} ({preset})", t.base_name());
                let res = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| {
                    let p = numbered_path(&dir, &stem, "wav");
                    write_wav(&p, &out, WavFormat::Pcm24).map(|_| p)
                });
                match res {
                    Ok(p) => {
                        app.set_status(format!("Gespeichert: {}", p.display()));
                        app.fetch.trim.saved.insert(0, p);
                        saved_any = true;
                    }
                    Err(e) => app.set_error(format!("Speichern fehlgeschlagen: {e}")),
                }
            }
            Act::SaveMp3 | Act::Lossless => {
                let t = &app.fetch.trim;
                let dir = cut_dir(app);
                let _ = std::fs::create_dir_all(&dir);
                let path = numbered_path(&dir, &t.base_name(), "mp3");
                let ffmpeg = app.fetch.tools.ffmpeg.clone();
                let res = match (a, &ffmpeg) {
                    (_, None) => Err("ffmpeg fehlt".to_string()),
                    (Act::SaveMp3, Some(f)) => render_range(t, t.sel.0, t.sel.1).ok_or("Leere Auswahl".to_string()).and_then(|d| tstudio_fetch::export_mp3(f, &d, &path, t.mp3_kbps)),
                    (_, Some(f)) => match &t.path {
                        Some(src) => tstudio_fetch::cut_lossless(f, src, &path, t.secs(t.sel.0), t.secs(t.sel.1)),
                        None => Err("keine Quelldatei".into()),
                    },
                };
                match res {
                    Ok(()) => {
                        app.set_status(format!("Gespeichert: {}", path.display()));
                        app.fetch.trim.saved.insert(0, path);
                        saved_any = true;
                    }
                    Err(e) => app.set_error(format!("Speichern fehlgeschlagen: {e}")),
                }
            }
            Act::ToRack | Act::ToPlaylist => {
                let (a0, b0) = app.fetch.trim.sel;
                let cat = Some(app.fetch.trim.category);
                match save_range(app, a0, b0, cat) {
                    Ok(path) => {
                        app.fetch.trim.saved.insert(0, path.clone());
                        saved_any = true;
                        if matches!(a, Act::ToRack) {
                            app.add_sampler_channel(&path);
                            app.view = View::Rack;
                        } else {
                            let start = widgets::floor_to(app.position() as i64, app.project.ticks_per_bar());
                            let track = app.playlist_ui.first_free_track(&app.project, start);
                            app.add_audio_clip(&path, track, start);
                            app.song_mode = true;
                            app.view = View::Playlist;
                        }
                    }
                    Err(e) => app.set_error(format!("Speichern fehlgeschlagen: {e}")),
                }
            }
            Act::Preview(p) => {
                app.send(tstudio_engine::Command::PreviewLoop(false));
                app.preview_path(&p);
            }
            Act::Rack(p) => {
                app.add_sampler_channel(&p);
                app.set_status("Sampler-Kanal hinzugefügt");
            }
        }
    }
    app.fetch.trim.saved.truncate(200);
    if saved_any {
        ensure_in_library(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trim_with(d: SampleData) -> Trim {
        let mut t = Trim::default();
        let len = d.len();
        t.set_loaded(PathBuf::from("song.mp3"), Loaded { peaks: Peaks::of(&d), hits: vec![0, len / 2], data: d });
        t
    }

    #[test]
    fn render_range_trims_tail_normalizes_and_isolates() {
        let sr = 44100;
        // 0.2 s of 100 Hz + 8 kHz, then 0.3 s silence
        let mut l: Vec<f32> = (0..sr / 5).map(|i| {
            let x = i as f32 / sr as f32;
            0.2 * (2.0 * std::f32::consts::PI * 100.0 * x).sin() + 0.2 * (2.0 * std::f32::consts::PI * 8000.0 * x).sin()
        }).collect();
        l.extend(std::iter::repeat(0.0).take(sr * 3 / 10));
        let mut t = trim_with(SampleData::new(l.clone(), l, sr as u32));
        let len = t.data.as_ref().unwrap().len();
        let out = render_range(&t, 0, len).unwrap();
        assert!(out.len() < sr / 5 + sr / 100, "tail removed: {}", out.len());
        assert!((out.peak() - 0.98).abs() < 0.01, "normalized");
        // isolate the bass: the 8 kHz part must be gone
        t.iso = IsolateParams { lp_hz: 300.0, ..Default::default() };
        t.iso_on = true;
        t.normalize = false;
        let lo = render_range(&t, 0, len).unwrap();
        let hf: f32 = lo.left.windows(2).skip(2000).take(4000).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / 4000.0;
        assert!(hf < 0.01, "high frequencies left: {hf}");
    }

    #[test]
    fn hits_and_numbered_paths() {
        let d = SampleData::new(vec![0.1; 1000], vec![0.1; 1000], 44100);
        let t = trim_with(d);
        assert_eq!(t.hit_around(100), (0, 500));
        assert_eq!(t.hit_around(700), (500, 1000));
        let dir = std::env::temp_dir().join(format!("tstudio_cut_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p1 = numbered_path(&dir, "song Kick", "wav");
        assert!(p1.ends_with("song Kick 01.wav"));
        std::fs::write(&p1, b"x").unwrap();
        assert!(numbered_path(&dir, "song Kick", "wav").ends_with("song Kick 02.wav"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
