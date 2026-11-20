//! Small FFT spectrum display (post-fader) for the selected insert and the master.

use super::*;
use crate::app::View;
use egui::{pos2, Align2, FontId, Rect};
use tstudio_dsp::spectrum::{band_edge_hz, SPECTRUM_BANDS, SPECTRUM_HI_HZ, SPECTRUM_LO_HZ};
use tstudio_engine::SpectrumFrame;

/// Engine analyser slot for the selected insert.
const SLOT_SELECTED: usize = 0;
/// Engine analyser slot for the master.
const SLOT_MASTER: usize = 1;
const TOP_DB: f32 = 0.0;
const BOTTOM_DB: f32 = -78.0;

/// Tells the engine which tracks to analyse; only open spectra in the visible mixer cost CPU.
pub fn sync_watch(app: &TStudio) {
    let visible = app.view == View::Mixer;
    let spec = &app.engine.shared.spectrum;
    let sel = app.mixer_ui.selected.min(app.project.mixer.len().saturating_sub(1));
    spec.watch(SLOT_SELECTED, (visible && app.mixer_ui.spectrum_open && sel > 0).then_some(sel));
    spec.watch(SLOT_MASTER, (visible && app.mixer_ui.master_spectrum_open).then_some(0));
}

/// Collapsible spectra above the effect chain: selected insert (if not the master) + master.
pub(super) fn panels(app: &mut TStudio, ui: &mut Ui, t: usize) {
    let shared = std::sync::Arc::clone(&app.engine.shared);
    if t > 0 {
        let title = format!("Spektrum – Insert {t}");
        collapsible(ui, "spec_sel", &title, &mut app.mixer_ui.spectrum_open, || shared.spectrum.read(SLOT_SELECTED).filter(|f| f.track == t));
    }
    collapsible(ui, "spec_master", "Spektrum – Master", &mut app.mixer_ui.master_spectrum_open, || shared.spectrum.read(SLOT_MASTER).filter(|f| f.track == 0));
}

fn collapsible(ui: &mut Ui, id: &str, title: &str, open: &mut bool, frame: impl FnOnce() -> Option<SpectrumFrame>) {
    let r = egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(id).open(Some(*open)).show(ui, |ui| {
        draw(ui, frame().as_ref());
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
    });
    if r.header_response.clicked() {
        *open = !*open;
    }
}

fn x_of(rect: Rect, f: f32) -> f32 {
    let (a, b) = (SPECTRUM_LO_HZ.ln(), SPECTRUM_HI_HZ.ln());
    rect.left() + (f.clamp(SPECTRUM_LO_HZ, SPECTRUM_HI_HZ).ln() - a) / (b - a) * rect.width()
}

fn y_of(rect: Rect, db: f32) -> f32 {
    let n = ((db - BOTTOM_DB) / (TOP_DB - BOTTOM_DB)).clamp(0.0, 1.0);
    rect.bottom() - n * rect.height()
}

fn draw(ui: &mut Ui, frame: Option<&SpectrumFrame>) {
    let w = ui.available_width().clamp(200.0, 640.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 90.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, BG_DARK);
    let body = rect.shrink2(vec2(4.0, 4.0));
    let grid = Stroke::new(1.0_f32, Color32::from_gray(45));
    let label = Color32::from_gray(100);
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        let x = x_of(body, f);
        p.line_segment([pos2(x, body.top()), pos2(x, body.bottom())], grid);
        let t = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        p.text(pos2(x + 2.0, body.bottom()), Align2::LEFT_BOTTOM, t, FontId::proportional(9.0), label);
    }
    for db in [-12.0, -24.0, -48.0, -72.0] {
        let y = y_of(body, db);
        p.line_segment([pos2(body.left(), y), pos2(body.right(), y)], grid);
        p.text(pos2(body.left() + 2.0, y), Align2::LEFT_BOTTOM, format!("{db:.0}"), FontId::proportional(9.0), label);
    }
    let Some(f) = frame else {
        p.text(body.center(), Align2::CENTER_CENTER, "kein Signal", FontId::proportional(11.0), label);
        return;
    };
    for i in 0..SPECTRUM_BANDS {
        let db = f.bands[i];
        if db <= BOTTOM_DB {
            continue;
        }
        let x0 = x_of(body, band_edge_hz(i));
        let x1 = x_of(body, band_edge_hz(i + 1)) - 1.0;
        let y = y_of(body, db);
        // hot bands (close to 0 dBFS) turn red
        let col = if db > -6.0 { Color32::from_rgb(240, 110, 90) } else { ACCENT.gamma_multiply(0.85) };
        p.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1.max(x0 + 1.0), body.bottom())), 0.0, col);
    }
    if let Some(pos) = resp.hover_pos() {
        let (a, b) = (SPECTRUM_LO_HZ.ln(), SPECTRUM_HI_HZ.ln());
        let hz = ((pos.x - body.left()) / body.width() * (b - a) + a).exp();
        let band = (0..SPECTRUM_BANDS).find(|&i| hz < band_edge_hz(i + 1)).unwrap_or(SPECTRUM_BANDS - 1);
        let text = format!("{} · {:.1} dB", widgets::fmt_hz(hz), f.bands[band]);
        p.text(body.right_top(), Align2::RIGHT_TOP, text, FontId::proportional(11.0), Color32::WHITE);
    }
}
