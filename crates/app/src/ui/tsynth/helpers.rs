//! Layout helpers and small widgets of the TSynth editor (cards, knobs, curves, displays).

use super::*;

/// Lays out `n` cards in rows. Columns = as many as fit with at least `min_w` each (max `max_cols`).
/// Cards of one row get the same width and (from the previous frame) the same height.
pub(super) fn card_grid(ui: &mut Ui, id: &str, min_w: f32, max_cols: usize, n: usize, mut add: impl FnMut(&mut Ui, usize)) {
    let avail = ui.available_width();
    let cols = (((avail + GAP) / (min_w + GAP)).floor() as usize).clamp(1, max_cols.min(n).max(1));
    let w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
    let mut start = 0;
    while start < n {
        let end = (start + cols).min(n);
        let hid = ui.id().with(("card_row", id, start, cols));
        let target: f32 = ui.data(|d| d.get_temp(hid)).unwrap_or(0.0);
        let mut max_h = 0.0f32;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for i in start..end {
                ui.vertical(|ui| {
                    ui.set_width(w);
                    egui::Frame::default().fill(BG_DARK).stroke(Stroke::new(1.0f32, CARD_EDGE)).rounding(6.0).inner_margin(egui::Margin::same(PAD)).show(ui, |ui| {
                        ui.set_width(w - 2.0 * PAD - 2.0);
                        let top = ui.min_rect().top();
                        add(ui, i);
                        let h = ui.min_rect().bottom() - top;
                        max_h = max_h.max(h);
                        if target > h {
                            ui.add_space(target - h);
                        }
                    });
                });
            }
        });
        if (max_h - target).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(hid, max_h));
            ui.ctx().request_repaint();
        }
        ui.add_space(GAP);
        start = end;
    }
}

/// Full-width card.
pub(super) fn wide_card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    egui::Frame::default().fill(BG_DARK).stroke(Stroke::new(1.0f32, CARD_EDGE)).rounding(6.0).inner_margin(egui::Margin::same(PAD)).show(ui, |ui| {
        ui.set_width(w - 2.0 * PAD - 2.0);
        add(ui);
    });
    ui.add_space(GAP);
}

/// Card title: colour bar, optional on/off checkbox, title, extra widgets right-aligned
/// (added right to left). Returns whether the card's content is active.
pub(super) fn head(ui: &mut Ui, on: Option<&mut bool>, title: &str, col: Color32, extra: impl FnOnce(&mut Ui)) -> bool {
    let mut active = true;
    ui.horizontal(|ui| {
        let lit = on.as_ref().map_or(true, |b| **b);
        let (r, _) = ui.allocate_exact_size(vec2(3.0, 16.0), Sense::hover());
        ui.painter().rect_filled(r, 1.5, if lit { col } else { BG_LIGHT });
        let text = RichText::new(title).strong().color(if lit { TITLE } else { TEXT_DIM });
        match on {
            Some(b) => {
                ui.checkbox(b, text);
                active = *b;
            }
            None => {
                ui.label(text);
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), extra);
    });
    ui.add_space(4.0);
    active
}

pub(super) fn knobs(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        add(ui)
    });
}

/// Framed chip inside a `horizontal_wrapped` row; frames do not wrap by themselves, so the row
/// is broken manually when the chip would not fit.
pub(super) fn chip(ui: &mut Ui, text: &str, fill: Color32, color: Color32) {
    let w = ui.painter().layout_no_wrap(text.to_string(), egui::TextStyle::Small.resolve(ui.style()), color).size().x + 16.0;
    if ui.max_rect().right() - ui.cursor().left() < w {
        ui.end_row();
    }
    egui::Frame::default().fill(fill).stroke(Stroke::new(1.0f32, CARD_EDGE)).rounding(8.0).inner_margin(egui::Margin::symmetric(7.0, 2.0)).show(ui, |ui| {
        ui.label(RichText::new(text).small().color(color));
    });
}

pub(super) fn small_label(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).small().color(TEXT_DIM));
}

/// Draws a polyline of `ys` (-1..1 when `bipolar`, else 0..1) across the full width.
pub(super) fn curve(ui: &mut Ui, height: f32, ys: &[f32], bipolar: bool, color: Color32) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, CURVE_BG);
    let inner = rect.shrink2(vec2(4.0, 5.0));
    for k in 1..4 {
        let x = inner.left() + inner.width() * k as f32 / 4.0;
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0f32, Color32::from_rgb(30, 33, 38)));
    }
    if bipolar {
        p.line_segment([pos2(inner.left(), inner.center().y), pos2(inner.right(), inner.center().y)], Stroke::new(1.0f32, BG_LIGHT));
    }
    let n = ys.len().max(2);
    let pts: Vec<egui::Pos2> = ys
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let x = inner.left() + inner.width() * i as f32 / (n - 1) as f32;
            let v = if bipolar { 0.5 - 0.5 * y.clamp(-1.2, 1.2) } else { 1.0 - y.clamp(0.0, 1.0) };
            pos2(x, inner.top() + inner.height() * v)
        })
        .collect();
    // soft fill under the curve
    let base = if bipolar { inner.center().y } else { inner.bottom() };
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28);
    for w in pts.windows(2) {
        p.add(egui::Shape::convex_polygon(vec![w[0], w[1], pos2(w[1].x, base), pos2(w[0].x, base)], fill, Stroke::NONE));
    }
    p.add(egui::Shape::line(pts, Stroke::new(1.8f32, color)));
    (rect, resp)
}

pub(super) fn env_curve(e: &AdsrParams, n: usize) -> Vec<f32> {
    let total = (e.attack + e.hold + e.decay * 0.8 + e.release * 0.8 + 0.15).max(0.02);
    let sr = n as f32 / total;
    let mut a = Adsr::new(sr);
    a.set_params(e);
    a.trigger();
    let gate = ((e.attack + e.hold + e.decay * 0.6 + 0.1) * sr) as usize;
    (0..n)
        .map(|i| {
            if i == gate {
                a.release();
            }
            a.next()
        })
        .collect()
}

/// Magnitude response (display units 0..1 = -36..+18 dB) of a filter slot over 20 Hz..20 kHz,
/// measured from its impulse response.
pub(super) fn filter_response(kind: FilterType, mode: tstudio_dsp::filter::FilterMode, cutoff: f32, res: f32, n_pts: usize) -> Vec<f32> {
    const SR: f32 = 48000.0;
    const N: usize = 4096; // long enough for the bass region
    let mut f = tstudio_dsp::filter::SynthFilter::new(4096);
    f.set(kind, cutoff, res, SR);
    let amp = 0.02; // small impulse keeps the saturating ladder linear
    let h: Vec<f32> = (0..N).map(|i| f.process(if i == 0 { amp } else { 0.0 }, kind, mode) / amp).collect();
    (0..n_pts)
        .map(|k| {
            let hz = 20.0 * 1000f32.powf(k as f32 / (n_pts - 1) as f32);
            let w = std::f32::consts::TAU * hz / SR;
            let (sw, cw) = w.sin_cos();
            let (mut c, mut s, mut re, mut im) = (1.0f32, 0.0f32, 0.0f32, 0.0f32);
            for &x in &h {
                re += x * c;
                im -= x * s;
                let c2 = c * cw - s * sw;
                s = s * cw + c * sw;
                c = c2;
            }
            let db = 10.0 * (re * re + im * im).max(1e-12).log10();
            ((db + 36.0) / 54.0).clamp(0.0, 1.0)
        })
        .collect()
}

/// x position (0..1) of a frequency on the 20 Hz..20 kHz log axis
pub(super) fn freq_x(hz: f32) -> f32 {
    ((hz.max(20.0) / 20.0).log10() / 3.0).clamp(0.0, 1.0)
}

pub(super) fn filter_display(ui: &mut Ui, kind: FilterType, mode: tstudio_dsp::filter::FilterMode, cutoff: f32, res: f32, color: Color32) {
    let ys = filter_response(kind, mode, cutoff, res, 110);
    let (rect, _) = curve(ui, 70.0, &ys, false, color);
    let p = ui.painter_at(rect);
    let inner = rect.shrink2(vec2(4.0, 5.0));
    // 0 dB line and decade labels
    let y0 = inner.top() + inner.height() * (1.0 - 36.0 / 54.0);
    p.line_segment([pos2(inner.left(), y0), pos2(inner.right(), y0)], Stroke::new(1.0f32, Color32::from_rgb(40, 44, 50)));
    for (hz, l) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        let x = inner.left() + inner.width() * freq_x(hz);
        p.text(pos2(x + 2.0, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, l, FontId::proportional(9.0), Color32::from_rgb(90, 96, 106));
    }
    let x = inner.left() + inner.width() * freq_x(cutoff);
    p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0f32, color.gamma_multiply(0.5)));
    p.text(pos2(rect.right() - 4.0, rect.top() + 3.0), Align2::RIGHT_TOP, widgets::fmt_hz(cutoff), FontId::proportional(10.0), TEXT_DIM);
}

/// Saturation transfer curve (input -1..1 -> output).
pub(super) fn drive_display(ui: &mut Ui, mode: DriveMode, drive_db: f32, color: Color32) {
    let ys: Vec<f32> = (0..120).map(|i| mode.transfer(-1.0 + 2.0 * i as f32 / 119.0, drive_db)).collect();
    let (rect, _) = curve(ui, 56.0, &ys, true, color);
    ui.painter_at(rect).text(rect.right_top() + vec2(-4.0, 3.0), Align2::RIGHT_TOP, format!("{} · {}", mode.name(), widgets::fmt_db(drive_db)), FontId::proportional(10.0), TEXT_DIM);
}

pub(super) fn env_card(ui: &mut Ui, title: &str, e: &mut AdsrParams, color: Color32) {
    head(ui, None, title, color, |_| {});
    curve(ui, 64.0, &env_curve(e, 220), false, color);
    ui.add_space(4.0);
    knobs(ui, |ui| {
        knob(ui, &mut e.attack, 0.0005, 10.0, 0.002, "Attack", true, &widgets::fmt_s);
        knob(ui, &mut e.hold, 0.0, 2.0, 0.0, "Hold", false, &widgets::fmt_s);
        knob(ui, &mut e.decay, 0.001, 10.0, 0.2, "Decay", true, &widgets::fmt_s);
        knob(ui, &mut e.sustain, 0.0, 1.0, 1.0, "Sustain", false, &widgets::fmt_pct);
        knob(ui, &mut e.release, 0.001, 10.0, 0.05, "Release", true, &widgets::fmt_s);
        knob(ui, &mut e.attack_curve, -1.0, 1.0, 0.0, "A-Kurve", false, &|v| format!("{v:+.2}")).on_hover_text("> 0: schneller Anstieg, < 0: langsam");
        knob(ui, &mut e.decay_curve, -1.0, 1.0, 0.0, "D/R-Kurve", false, &|v| format!("{v:+.2}")).on_hover_text("> 0: knackiger (Kicks), < 0: runder");
    });
}

pub(super) fn combo<T: PartialEq + Copy>(ui: &mut Ui, id: impl std::hash::Hash, v: &mut T, all: &[T], name: fn(T) -> &'static str, width: f32) {
    egui::ComboBox::from_id_salt(id).selected_text(name(*v)).width(width).show_ui(ui, |ui| {
        for x in all {
            ui.selectable_value(v, *x, name(*x));
        }
    });
}

pub(super) fn note_drag(ui: &mut Ui, v: &mut f32, lo: f32, hi: f32) -> egui::Response {
    ui.add(egui::DragValue::new(v).speed(0.05).range(lo..=hi).custom_formatter(|n, _| {
        let r = n.round();
        let cents = ((n - r) * 100.0).round();
        let name = tstudio_dsp::note_name(r.clamp(0.0, 127.0) as u8);
        if cents.abs() >= 1.0 {
            format!("{name} {cents:+.0}ct")
        } else {
            name
        }
    }))
}
