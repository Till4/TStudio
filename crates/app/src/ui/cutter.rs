//! Sample cutter: chop hits/loops out of MP3s & co. at keyboard speed, optionally isolated with
//! zero-phase EQ / mid-side, saved straight into the sample library (Cuts/<Kategorie>).
//!
//! Keys (Fetch view, file loaded): Space play/stop · ←/→ previous/next hit · Shift+←/→ end to
//! previous/next hit · Enter save · 1–9 save as category · L loop · I isolation A/B · Z zoom ·
//! Shift+Z whole file · H save every hit in the selection · Esc stop.
//!
//! Fine nudge: Ctrl+←/→ moves the whole window by one beat (selection width / `beats`, i.e. the
//! loop's own grid; for the whole file one beat of the project tempo) · Ctrl+Shift+←/→ moves it
//! by 1 ms. Markers: M stores the selection as a named marker · Shift+M saves every marker ·
//! `,` / `.` jump to the previous / next marker (list with rename/delete under the waveform).
//!
//! Waveform mouse: drag inside the selection moves the window, drag outside (or Shift+drag) makes
//! a new one, drag an edge adjusts it, click plays from there, double click selects the hit.

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

mod actions;
mod edit;
mod keys;
mod markers;
mod panels;
mod stems;
#[cfg(test)]
mod tests;

pub use actions::cut_dir;
use actions::*;
use edit::*;
use keys::*;
use markers::*;
use panels::*;
use stems::*;

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
    /// Whole window moves; holds the grab offset (samples from the window start to the pointer).
    Move(usize),
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
    /// named selections of the loaded file (M), saved together with Shift+M
    pub markers: Vec<CutMarker>,
    /// marker whose name is being edited in the list
    marker_edit: Option<usize>,
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
            markers: Vec::new(),
            marker_edit: None,
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
        self.markers.clear();
        self.marker_edit = None;
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

enum Act {
    PlaySel,
    PlayFrom(usize),
    Stop,
    Save(Option<usize>),
    SaveHits,
    /// every marker as its own file
    SaveMarkers,
    SaveFull,
    SaveMp3,
    Lossless,
    ToRack,
    ToPlaylist,
    Preview(PathBuf),
    Rack(PathBuf),
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

    // selection: drag inside = move window, drag outside (or Shift) = new, edges = adjust,
    // click = play from here, double click = hit
    let shift = ui.input(|i| i.modifiers.shift);
    if let Some(pt) = resp.interact_pointer_pos() {
        let s = s_of(pt.x) as usize;
        if resp.drag_started() {
            let ds = (pt.x - x_of(t.sel.0 as f64)).abs();
            let de = (pt.x - x_of(t.sel.1 as f64)).abs();
            let whole = t.sel.0 == 0 && t.sel.1 >= len as usize;
            t.drag = Some(if ds < 8.0 && ds <= de {
                TrimDrag::Start
            } else if de < 8.0 {
                TrimDrag::End
            } else if !shift && !whole && s > t.sel.0 && s < t.sel.1 {
                TrimDrag::Move(s - t.sel.0)
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
                Some(TrimDrag::Move(off)) => {
                    // the grab offset keeps the window under the pointer; snap its start edge
                    let start = snap(t, (s_of(pt.x) as usize).saturating_sub(off));
                    t.sel = moved_selection(t.sel, start, len as usize);
                }
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
        if matches!(t.drag, Some(TrimDrag::New(_) | TrimDrag::Move(_))) {
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
    paint_markers(&p, rect, &t.markers, &x_of);
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
    handle_keys(ui, t, data.len(), app.project.bpm, now, &mut acts);

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
    markers_panel(app, ui, &mut acts);
    ui.weak("Tasten: Leertaste spielen · ←/→ Hit für Hit · Shift+←/→ Ende verlängern/kürzen · Strg+←/→ Auswahl um einen Beat schieben (Strg+Shift fein) · M Marker setzen · Shift+M alle Marker speichern · ,/. Marker wechseln · 1–9 als Kategorie speichern · Enter speichern · H alle Hits · L Loop · I Isolieren an/aus · Z Zoom · Doppelklick = Hit wählen · Klick = ab hier hören");

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
