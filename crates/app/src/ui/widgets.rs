use crate::samples::Peaks;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Rect, Response, Sense, Stroke, Ui};
use std::f32::consts::PI;

pub const ACCENT: Color32 = Color32::from_rgb(255, 150, 60);
pub const BG_DARK: Color32 = Color32::from_rgb(28, 31, 36);
pub const BG_PANEL: Color32 = Color32::from_rgb(36, 40, 46);
pub const BG_LIGHT: Color32 = Color32::from_rgb(48, 53, 61);
pub const GRID_BAR: Color32 = Color32::from_rgb(90, 98, 110);
pub const GRID_BEAT: Color32 = Color32::from_rgb(62, 68, 78);
pub const GRID_SUB: Color32 = Color32::from_rgb(44, 49, 56);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 156, 166);
pub const PLAYHEAD: Color32 = Color32::from_rgb(120, 230, 140);

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// egui's bundled fonts miss many symbols (⋯ ↶ 🎹 📁 …); Windows ships fonts that have them.
fn install_symbol_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    for (name, file) in [("segoe-symbol", "seguisym.ttf"), ("segoe-emoji", "seguiemj.ttf")] {
        if let Ok(bytes) = std::fs::read(dir.join(file)) {
            fonts.font_data.insert(name.to_owned(), egui::FontData::from_owned(bytes));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push(name.to_owned());
            }
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &egui::Context) {
    install_symbol_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_PANEL;
    v.window_fill = BG_PANEL;
    v.extreme_bg_color = BG_DARK;
    v.faint_bg_color = BG_LIGHT;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    v.widgets.active.bg_fill = ACCENT.gamma_multiply(0.6);
    ctx.set_visuals(v);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = vec2(6.0, 4.0);
    style.spacing.button_padding = vec2(6.0, 2.0);
    ctx.set_style(style);
}

/// Rotary knob. Drag vertically (Shift = fine), double-click resets.
#[allow(clippy::too_many_arguments)]
pub fn knob(ui: &mut Ui, value: &mut f32, min: f32, max: f32, default: f32, label: &str, log: bool, fmt: &dyn Fn(f32) -> String) -> Response {
    let size = vec2(46.0, 54.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let to_norm = |v: f32| -> f32 {
        if log {
            ((v.max(min) / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
        } else {
            ((v - min) / (max - min)).clamp(0.0, 1.0)
        }
    };
    let from_norm = |n: f32| -> f32 {
        if log {
            min * (max / min).powf(n)
        } else {
            min + n * (max - min)
        }
    };
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let speed = if fine { 0.0012 } else { 0.006 };
        let n = (to_norm(*value) - resp.drag_delta().y * speed + resp.drag_delta().x * speed * 0.3).clamp(0.0, 1.0);
        *value = from_norm(n);
        resp.mark_changed();
    }
    if resp.double_clicked() {
        *value = default;
        resp.mark_changed();
    }
    let p = ui.painter();
    let c = pos2(rect.center().x, rect.top() + 19.0);
    let r = 15.0;
    p.circle_filled(c, r, BG_DARK);
    let start = 0.75 * PI;
    let sweep = 1.5 * PI;
    let arc = |from: f32, to: f32, color: Color32, w: f32| {
        let steps = 24;
        let pts: Vec<egui::Pos2> = (0..=steps)
            .map(|i| {
                let a = from + (to - from) * i as f32 / steps as f32;
                c + vec2(a.cos(), a.sin()) * (r - 2.0)
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(w, color)));
    };
    arc(start, start + sweep, BG_LIGHT, 3.0);
    let n = to_norm(*value);
    // bipolar knobs (range spans zero) draw from the centre
    let zero = if min < 0.0 && max > 0.0 && !log { to_norm(0.0) } else { 0.0 };
    let (a0, a1) = if n >= zero { (start + sweep * zero, start + sweep * n) } else { (start + sweep * n, start + sweep * zero) };
    let col = if resp.hovered() || resp.dragged() { ACCENT } else { ACCENT.gamma_multiply(0.85) };
    if (a1 - a0).abs() > 0.01 {
        arc(a0, a1, col, 3.0);
    }
    let a = start + sweep * n;
    p.line_segment([c, c + vec2(a.cos(), a.sin()) * (r - 4.0)], Stroke::new(2.0, Color32::WHITE));
    let text = if resp.dragged() || resp.hovered() { fmt(*value) } else { label.to_string() };
    p.text(pos2(rect.center().x, rect.bottom() - 7.0), Align2::CENTER_CENTER, text, FontId::proportional(10.0), TEXT_DIM);
    resp.on_hover_text(format!("{label}: {}\nZiehen = ändern, Shift = fein, Doppelklick = Reset", fmt(*value)))
}

pub fn fmt_hz(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.2} kHz", v / 1000.0)
    } else {
        format!("{v:.0} Hz")
    }
}
pub fn fmt_db(v: f32) -> String {
    format!("{v:+.1} dB")
}
pub fn fmt_pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}
pub fn fmt_ms(v: f32) -> String {
    format!("{v:.0} ms")
}
pub fn fmt_s(v: f32) -> String {
    if v < 1.0 {
        format!("{:.0} ms", v * 1000.0)
    } else {
        format!("{v:.2} s")
    }
}
pub fn fmt_pan(v: f32) -> String {
    if v.abs() < 0.01 {
        "C".into()
    } else if v < 0.0 {
        format!("L{:.0}", -v * 100.0)
    } else {
        format!("R{:.0}", v * 100.0)
    }
}
pub fn fmt_semi(v: f32) -> String {
    format!("{v:+.2} st")
}
pub fn fmt_x(v: f32) -> String {
    format!("{v:.2}")
}
pub fn fmt_gain(v: f32) -> String {
    if v <= 0.0001 {
        "-inf dB".into()
    } else {
        format!("{:+.1} dB", tstudio_dsp::gain_to_db(v))
    }
}

/// Draws samples [s0, s1) of a waveform into `rect`.
pub fn draw_waveform(p: &Painter, rect: Rect, peaks: &Peaks, s0: f64, s1: f64, color: Color32) {
    let w = rect.width().max(1.0) as usize;
    let mid = rect.center().y;
    let h = rect.height() * 0.48;
    let span = (s1 - s0).max(1.0);
    for x in 0..w {
        let a = s0 + span * x as f64 / w as f64;
        let b = s0 + span * (x + 1) as f64 / w as f64;
        let (mn, mx) = peaks.range(a, b);
        if mx - mn < 0.002 {
            continue;
        }
        let xx = rect.left() + x as f32 + 0.5;
        p.line_segment([pos2(xx, mid - mx.min(1.0) * h), pos2(xx, mid - mn.max(-1.0) * h)], Stroke::new(1.0, color));
    }
}

/// Small 16-px square toggle button (mute/solo style).
pub fn small_toggle(ui: &mut Ui, on: &mut bool, text: &str, on_color: Color32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 16.0), Sense::click());
    let fill = if *on { on_color } else { BG_LIGHT };
    ui.painter().rect_filled(rect, 3.0, fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(10.0), if *on { Color32::BLACK } else { TEXT_DIM });
    let resp = resp.on_hover_text(tip);
    if resp.clicked() {
        *on = !*on;
    }
    resp
}

/// Vertical meter (two bars) for linear peaks.
pub fn draw_meter(p: &Painter, rect: Rect, l: f32, r: f32) {
    p.rect_filled(rect, 1.0, BG_DARK);
    let half = rect.width() / 2.0;
    for (i, v) in [l, r].into_iter().enumerate() {
        let db = tstudio_dsp::gain_to_db(v).clamp(-60.0, 6.0);
        let n = (db + 60.0) / 66.0;
        let bar = Rect::from_min_max(pos2(rect.left() + i as f32 * half + 1.0, rect.bottom() - rect.height() * n), pos2(rect.left() + (i + 1) as f32 * half - 1.0, rect.bottom()));
        let col = if db > -0.5 {
            Color32::from_rgb(240, 70, 60)
        } else if db > -9.0 {
            Color32::from_rgb(240, 200, 70)
        } else {
            Color32::from_rgb(90, 210, 120)
        };
        p.rect_filled(bar, 0.0, col);
    }
    // 0 dB mark
    let y0 = rect.bottom() - rect.height() * (60.0 / 66.0);
    p.line_segment([pos2(rect.left(), y0), pos2(rect.right(), y0)], Stroke::new(1.0, Color32::from_gray(140)));
}

/// Grid snap values in ticks (PPQ = 96).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Bar,
    Beat,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    TripletEighth,
    TripletSixteenth,
    None,
}

impl Snap {
    pub const ALL: [Snap; 9] = [Snap::Bar, Snap::Beat, Snap::Half, Snap::Quarter, Snap::Eighth, Snap::Sixteenth, Snap::TripletEighth, Snap::TripletSixteenth, Snap::None];
    pub fn ticks(self, bar: i64) -> i64 {
        match self {
            Snap::Bar => bar,
            Snap::Beat => 96,
            Snap::Half => 48,
            Snap::Quarter => 24,
            Snap::Eighth => 12,
            Snap::Sixteenth => 6,
            Snap::TripletEighth => 32,
            Snap::TripletSixteenth => 16,
            Snap::None => 1,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Snap::Bar => "Takt",
            Snap::Beat => "Beat",
            Snap::Half => "1/2 Beat",
            Snap::Quarter => "1/4 Beat (16tel)",
            Snap::Eighth => "1/8 Beat (32tel)",
            Snap::Sixteenth => "1/16 Beat",
            Snap::TripletEighth => "1/3 Beat (Triole)",
            Snap::TripletSixteenth => "1/6 Beat (Triole)",
            Snap::None => "Aus",
        }
    }
}

pub fn snap_combo(ui: &mut Ui, id: &str, snap: &mut Snap) {
    egui::ComboBox::from_id_salt(id).selected_text(format!("Raster: {}", snap.name())).width(150.0).show_ui(ui, |ui| {
        for s in Snap::ALL {
            ui.selectable_value(snap, s, s.name());
        }
    });
}

pub fn floor_to(v: i64, step: i64) -> i64 {
    if step <= 1 {
        v
    } else {
        v.div_euclid(step) * step
    }
}

pub fn round_to(v: i64, step: i64) -> i64 {
    if step <= 1 {
        v
    } else {
        ((v as f64 / step as f64).round() as i64) * step
    }
}

/// Library drag & drop payload.
#[derive(Clone, Debug)]
pub struct SampleDrag {
    pub path: std::path::PathBuf,
}

pub fn draw_drag_ghost(ctx: &egui::Context) {
    if let Some(p) = egui::DragAndDrop::payload::<SampleDrag>(ctx) {
        if let Some(pos) = ctx.pointer_hover_pos() {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dnd_ghost")));
            let name = p.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let galley = painter.layout_no_wrap(format!("♪ {name}"), FontId::proportional(12.0), Color32::WHITE);
            let r = Rect::from_min_size(pos + vec2(12.0, 8.0), galley.size() + vec2(10.0, 6.0));
            painter.rect_filled(r, 4.0, ACCENT.gamma_multiply(0.8));
            painter.galley(r.min + vec2(5.0, 3.0), galley, Color32::WHITE);
        }
    }
}

/// Ctrl+C / Ctrl+X / Ctrl+V for in-app clipboards. egui turns these keys into
/// `Event::Copy` / `Cut` / `Paste` (paste only when the system clipboard has text), so look at both.
pub fn edit_keys(ui: &egui::Ui) -> (bool, bool, bool) {
    ui.input(|i| {
        let (mut c, mut x, mut v) = (false, false, false);
        for e in &i.events {
            match e {
                egui::Event::Copy => c = true,
                egui::Event::Cut => x = true,
                egui::Event::Paste(_) => v = true,
                egui::Event::Key { key, pressed: true, modifiers, .. } if modifiers.command => match key {
                    egui::Key::C => c = true,
                    egui::Key::X => x = true,
                    egui::Key::V => v = true,
                    _ => {}
                },
                _ => {}
            }
        }
        (c, x, v)
    })
}

/// Puts a marker into the system clipboard after an in-app copy, so the next Ctrl+V reaches egui as a paste event.
pub fn mark_copied(ui: &egui::Ui, what: &str) {
    ui.ctx().copy_text(format!("TStudio: {what}"));
}

/// Small TStudio logo (same geometry as the app icon).
pub fn logo(ui: &mut Ui, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let k = size / 256.0;
    let p = ui.painter();
    let c = |v: [u8; 3]| Color32::from_rgb(v[0], v[1], v[2]);
    p.rect_filled(rect, crate::logo::BG_RADIUS * k, c(crate::logo::BG_TOP));
    for [x0, y0, x1, y1, r] in crate::logo::shapes() {
        let r2 = egui::Rect::from_min_max(rect.min + vec2(x0 * k, y0 * k), rect.min + vec2(x1 * k, y1 * k));
        p.rect_filled(r2, r * k, ACCENT);
    }
    resp.on_hover_text("TStudio")
}
