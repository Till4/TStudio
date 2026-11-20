//! Parameter panels (knobs, EQ curve) for the built-in mixer effects.

use super::*;

/// Visual EQ: response curve with three draggable bands.
/// Drag a point = frequency + gain, mouse wheel on a point = width (Q, mid band), double click = 0 dB.
fn eq_graph(ui: &mut Ui, e: &mut tstudio_dsp::fx::EqParams) {
    use egui::{pos2, Align2, FontId, Pos2, Rect};
    const DB: f32 = 18.0;
    let w = ui.available_width().clamp(260.0, 640.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 150.0), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, BG_DARK);
    let body = rect.shrink2(vec2(6.0, 8.0));
    let (lf0, lf1) = (20f32.ln(), 20000f32.ln());
    let x_of = |f: f32| body.left() + (f.max(20.0).ln() - lf0) / (lf1 - lf0) * body.width();
    let f_of = |x: f32| ((x - body.left()) / body.width() * (lf1 - lf0) + lf0).exp();
    let y_of = |db: f32| body.center().y - db.clamp(-DB - 3.0, DB + 3.0) / DB * body.height() * 0.5;
    let db_of = |y: f32| ((body.center().y - y) / (body.height() * 0.5) * DB).clamp(-DB, DB);
    // grid
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        let x = x_of(f);
        p.line_segment([pos2(x, body.top()), pos2(x, body.bottom())], Stroke::new(1.0, Color32::from_gray(45)));
        let t = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        p.text(pos2(x + 2.0, body.bottom()), Align2::LEFT_BOTTOM, t, FontId::proportional(9.0), Color32::from_gray(100));
    }
    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let y = y_of(db);
        p.line_segment([pos2(body.left(), y), pos2(body.right(), y)], Stroke::new(1.0, if db == 0.0 { Color32::from_gray(80) } else { Color32::from_gray(45) }));
        p.text(pos2(body.left() + 2.0, y), Align2::LEFT_BOTTOM, format!("{db:+.0}"), FontId::proportional(9.0), Color32::from_gray(100));
    }
    // bands: (freq, gain, freq range, colour)
    let cols = [Color32::from_rgb(240, 150, 80), Color32::from_rgb(120, 220, 140), Color32::from_rgb(110, 170, 250)];
    let ranges = [(30.0, 500.0), (150.0, 8000.0), (2000.0, 16000.0)];
    let bands = |e: &tstudio_dsp::fx::EqParams| [(e.low_freq, e.low_gain), (e.mid_freq, e.mid_gain), (e.high_freq, e.high_gain)];
    let drag_id = resp.id.with("eq_drag");
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id)).flatten();
    let near = |pt: Pos2, e: &tstudio_dsp::fx::EqParams| {
        bands(e).iter().enumerate().map(|(i, (f, g))| (i, pos2(x_of(*f), y_of(*g)).distance(pt))).filter(|(_, d)| *d < 14.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    };
    if resp.drag_started() {
        dragging = resp.interact_pointer_pos().and_then(|pt| near(pt, e));
    }
    if resp.dragged() {
        if let (Some(i), Some(pt)) = (dragging, resp.interact_pointer_pos()) {
            let f = f_of(pt.x).clamp(ranges[i].0, ranges[i].1);
            let g = db_of(pt.y);
            let g = if g.abs() < 0.4 { 0.0 } else { g };
            match i {
                0 => (e.low_freq, e.low_gain) = (f, g),
                1 => (e.mid_freq, e.mid_gain) = (f, g),
                _ => (e.high_freq, e.high_gain) = (f, g),
            }
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        dragging = None;
    }
    let hover_band = resp.hover_pos().and_then(|pt| near(pt, e));
    if resp.double_clicked() {
        match hover_band {
            Some(0) => e.low_gain = 0.0,
            Some(1) => e.mid_gain = 0.0,
            Some(_) => e.high_gain = 0.0,
            None => {}
        }
    }
    if hover_band.is_some() || dragging.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 && hover_band == Some(1) {
            e.mid_q = (e.mid_q * (1.0 + scroll * 0.004)).clamp(0.2, 8.0);
        }
    }
    ui.data_mut(|d| d.insert_temp(drag_id, dragging));
    // per-band fill + total curve
    let n = 180;
    let pts: Vec<Pos2> = (0..=n).map(|k| {
        let x = body.left() + body.width() * k as f32 / n as f32;
        pos2(x, y_of(e.response_db(f_of(x), 48000.0)))
    }).collect();
    let mut fill = pts.clone();
    fill.push(pos2(body.right(), y_of(0.0)));
    fill.push(pos2(body.left(), y_of(0.0)));
    for w in pts.windows(2) {
        p.line_segment([pos2(w[0].x, y_of(0.0)), w[0]], Stroke::new(1.0, ACCENT.gamma_multiply(0.12)));
    }
    p.add(egui::Shape::line(pts, Stroke::new(2.0, ACCENT)));
    for (i, (f, g)) in bands(e).iter().enumerate() {
        let c = pos2(x_of(*f), y_of(*g));
        let active = dragging == Some(i) || hover_band == Some(i);
        p.circle_filled(c, if active { 7.0 } else { 5.5 }, cols[i]);
        p.circle_stroke(c, if active { 7.0 } else { 5.5 }, Stroke::new(1.0, Color32::BLACK));
    }
    let info = match dragging.or(hover_band) {
        Some(i) => {
            let (f, g) = bands(e)[i];
            let name = ["Low", "Mid", "High"][i];
            let q = if i == 1 { format!(" · Q {:.2} (Mausrad)", e.mid_q) } else { String::new() };
            format!("{name} {} {g:+.1} dB{q}", widgets::fmt_hz(f))
        }
        None => "Punkte ziehen · Mausrad auf Mid = Breite · Doppelklick = 0 dB".into(),
    };
    p.text(Rect::from_min_max(body.min, body.max).right_top() + vec2(-4.0, 0.0), Align2::RIGHT_TOP, info, FontId::proportional(11.0), Color32::WHITE);
    let _ = fill;
}

/// Knob panels for each built-in effect.
pub fn fx_params(ui: &mut Ui, p: &mut EffectParams, track_names: &[String]) {
    use widgets::{fmt_db, fmt_hz, fmt_ms, fmt_pct, fmt_x};
    if let EffectParams::Eq(e) = p {
        eq_graph(ui, e);
    }
    ui.horizontal_wrapped(|ui| match p {
        EffectParams::Eq(e) => {
            knob(ui, &mut e.low_gain, -18.0, 18.0, 0.0, "Low", false, &fmt_db);
            knob(ui, &mut e.low_freq, 30.0, 500.0, 120.0, "Low Hz", true, &fmt_hz);
            knob(ui, &mut e.mid_gain, -18.0, 18.0, 0.0, "Mid", false, &fmt_db);
            knob(ui, &mut e.mid_freq, 150.0, 8000.0, 1000.0, "Mid Hz", true, &fmt_hz);
            knob(ui, &mut e.mid_q, 0.2, 8.0, 0.8, "Q", true, &fmt_x);
            knob(ui, &mut e.high_gain, -18.0, 18.0, 0.0, "High", false, &fmt_db);
            knob(ui, &mut e.high_freq, 2000.0, 16000.0, 8000.0, "High Hz", true, &fmt_hz);
        }
        EffectParams::Compressor(c) => {
            knob(ui, &mut c.threshold, -60.0, 0.0, -18.0, "Thresh", false, &fmt_db);
            knob(ui, &mut c.ratio, 1.0, 20.0, 4.0, "Ratio", true, &|v| format!("{v:.1}:1"));
            knob(ui, &mut c.attack_ms, 0.1, 200.0, 10.0, "Attack", true, &fmt_ms);
            knob(ui, &mut c.release_ms, 10.0, 1500.0, 120.0, "Release", true, &fmt_ms);
            knob(ui, &mut c.knee, 0.0, 18.0, 6.0, "Knee", false, &fmt_db);
            knob(ui, &mut c.makeup, -12.0, 24.0, 0.0, "Makeup", false, &fmt_db);
            knob(ui, &mut c.mix, 0.0, 1.0, 1.0, "Mix", false, &fmt_pct);
        }
        EffectParams::Reverb(r) => {
            knob(ui, &mut r.size, 0.0, 1.0, 0.6, "Größe", false, &fmt_pct);
            knob(ui, &mut r.damping, 0.0, 1.0, 0.5, "Dämpfung", false, &fmt_pct);
            knob(ui, &mut r.predelay_ms, 0.0, 200.0, 10.0, "Predelay", false, &fmt_ms);
            knob(ui, &mut r.width, 0.0, 1.0, 1.0, "Breite", false, &fmt_pct);
            knob(ui, &mut r.mix, 0.0, 1.0, 0.25, "Mix", false, &fmt_pct);
        }
        EffectParams::Delay(d) => {
            ui.vertical(|ui| {
                ui.label(RichText::new("Zeit").small());
                egui::ComboBox::from_id_salt(ui.id().with("dly_time")).selected_text(beats_name(d.time_beats)).width(90.0).show_ui(ui, |ui| {
                    for (b, n) in BEAT_VALUES {
                        ui.selectable_value(&mut d.time_beats, b, n);
                    }
                });
                ui.checkbox(&mut d.ping_pong, "Ping-Pong");
            });
            knob(ui, &mut d.feedback, 0.0, 0.95, 0.4, "Feedback", false, &fmt_pct);
            knob(ui, &mut d.tone, 500.0, 18000.0, 6000.0, "Ton", true, &fmt_hz);
            knob(ui, &mut d.mix, 0.0, 1.0, 0.3, "Mix", false, &fmt_pct);
        }
        EffectParams::Distortion(d) => {
            ui.vertical(|ui| {
                ui.label(RichText::new("Modus").small());
                egui::ComboBox::from_id_salt(ui.id().with("dist_mode")).selected_text(d.mode.name()).width(90.0).show_ui(ui, |ui| {
                    for m in DistMode::ALL {
                        ui.selectable_value(&mut d.mode, m, m.name());
                    }
                });
            });
            knob(ui, &mut d.drive_db, 0.0, 48.0, 12.0, "Drive", false, &fmt_db);
            knob(ui, &mut d.tone, 500.0, 20000.0, 9000.0, "Ton", true, &fmt_hz);
            if d.mode == DistMode::Crush {
                knob(ui, &mut d.bits, 1.0, 16.0, 8.0, "Bits", false, &|v| format!("{v:.0} bit"));
                knob(ui, &mut d.downsample, 1.0, 32.0, 1.0, "Down", true, &|v| format!("÷{v:.1}"));
            }
            knob(ui, &mut d.output_db, -24.0, 12.0, -6.0, "Out", false, &fmt_db);
            knob(ui, &mut d.mix, 0.0, 1.0, 1.0, "Mix", false, &fmt_pct);
        }
        EffectParams::Chorus(c) => {
            knob(ui, &mut c.rate, 0.05, 8.0, 0.8, "Rate", true, &|v| format!("{v:.2} Hz"));
            knob(ui, &mut c.depth_ms, 0.0, 10.0, 3.0, "Tiefe", false, &fmt_ms);
            knob(ui, &mut c.delay_ms, 2.0, 40.0, 12.0, "Delay", false, &fmt_ms);
            knob(ui, &mut c.mix, 0.0, 1.0, 0.5, "Mix", false, &fmt_pct);
        }
        EffectParams::Filter(f) => {
            ui.vertical(|ui| {
                ui.label(RichText::new("Typ").small());
                filter_mode_combo(ui, &mut f.mode);
            });
            knob(ui, &mut f.cutoff, 20.0, 20000.0, 2000.0, "Cutoff", true, &fmt_hz);
            knob(ui, &mut f.resonance, 0.0, 1.0, 0.2, "Reso", false, &fmt_pct);
            knob(ui, &mut f.mix, 0.0, 1.0, 1.0, "Mix", false, &fmt_pct);
        }
        EffectParams::Limiter(l) => {
            knob(ui, &mut l.gain_db, -12.0, 24.0, 0.0, "Gain", false, &fmt_db);
            knob(ui, &mut l.ceiling_db, -12.0, 0.0, -0.3, "Ceiling", false, &fmt_db);
            knob(ui, &mut l.release_ms, 5.0, 1000.0, 80.0, "Release", true, &fmt_ms);
        }
        EffectParams::Sidechain(s) => {
            ui.vertical(|ui| {
                ui.label(RichText::new("Auslöser").small());
                let cur = track_names.get(s.source).cloned().unwrap_or_else(|| format!("{}", s.source));
                egui::ComboBox::from_id_salt(ui.id().with("sc_src")).selected_text(cur).width(120.0).show_ui(ui, |ui| {
                    for (i, n) in track_names.iter().enumerate() {
                        ui.selectable_value(&mut s.source, i, n);
                    }
                })
                .response
                .on_hover_text("Mixer-Spur, deren Signal diese Spur leiser macht (meist die Kick)");
            });
            knob(ui, &mut s.depth, 0.0, 1.0, 0.85, "Tiefe", false, &fmt_pct);
            knob(ui, &mut s.threshold_db, -48.0, 0.0, -18.0, "Schwelle", false, &fmt_db);
            knob(ui, &mut s.attack_ms, 0.1, 50.0, 1.0, "Attack", true, &fmt_ms);
            knob(ui, &mut s.release_ms, 10.0, 1000.0, 160.0, "Release", true, &fmt_ms);
        }
        EffectParams::Autotune(a) => {
            use tstudio_dsp::autotune::Scale;
            ui.vertical(|ui| {
                ui.label(RichText::new("Tonart").small());
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(ui.id().with("at_key")).selected_text(a.key_name()).width(46.0).show_ui(ui, |ui| {
                        for (k, n) in tstudio_dsp::NOTE_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut a.key, k as u8, *n);
                        }
                    });
                    egui::ComboBox::from_id_salt(ui.id().with("at_scale")).selected_text(a.scale.name()).width(110.0).show_ui(ui, |ui| {
                        for s in Scale::ALL {
                            ui.selectable_value(&mut a.scale, s, s.name());
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if ui.small_button("Hart").on_hover_text("Roboter-/T-Pain-Effekt: sofort auf den Ton").clicked() {
                        a.retune_ms = 0.0;
                        a.amount = 1.0;
                    }
                    if ui.small_button("Natürlich").on_hover_text("sanfte Korrektur, Vibrato bleibt").clicked() {
                        a.retune_ms = 80.0;
                        a.amount = 0.8;
                    }
                });
            });
            knob(ui, &mut a.retune_ms, 0.0, 400.0, 20.0, "Retune", false, &|v| if v < 0.5 { "hart".into() } else { format!("{v:.0} ms") })
                .on_hover_text("Wie schnell der Ton auf die Skala gezogen wird: 0 = hart/robotisch, 50–150 ms = natürlich");
            knob(ui, &mut a.amount, 0.0, 1.0, 1.0, "Stärke", false, &fmt_pct);
            knob(ui, &mut a.transpose, -12.0, 12.0, 0.0, "Transp.", false, &|v| format!("{v:+.0} HT"));
            knob(ui, &mut a.mix, 0.0, 1.0, 1.0, "Mix", false, &fmt_pct);
        }
        EffectParams::Clipper(c) => {
            knob(ui, &mut c.drive_db, 0.0, 24.0, 3.0, "Drive", false, &fmt_db).on_hover_text("Wie stark in den Clipper gefahren wird – macht Kick und Master lauter ohne Pumpen");
            knob(ui, &mut c.ceiling_db, -12.0, 0.0, -0.5, "Ceiling", false, &fmt_db);
            knob(ui, &mut c.softness, 0.0, 1.0, 0.25, "Weich", false, &fmt_pct).on_hover_text("0 = harter Clip (aggressiv), 100 % = weiches Knie (wärmer)");
            knob(ui, &mut c.output_db, -24.0, 12.0, 0.0, "Out", false, &fmt_db);
            knob(ui, &mut c.mix, 0.0, 1.0, 1.0, "Mix", false, &fmt_pct);
        }
        EffectParams::Multiband(m) => {
            knob(ui, &mut m.low_freq, 40.0, 600.0, 150.0, "Tief/Mitte", true, &fmt_hz);
            knob(ui, &mut m.high_freq, 800.0, 12000.0, 2500.0, "Mitte/Hoch", true, &fmt_hz);
            knob(ui, &mut m.low_drive_db, 0.0, 36.0, 6.0, "Drive T", false, &fmt_db);
            knob(ui, &mut m.mid_drive_db, 0.0, 36.0, 9.0, "Drive M", false, &fmt_db);
            knob(ui, &mut m.high_drive_db, 0.0, 36.0, 4.0, "Drive H", false, &fmt_db);
            knob(ui, &mut m.low_gain_db, -24.0, 12.0, 0.0, "Gain T", false, &fmt_db);
            knob(ui, &mut m.mid_gain_db, -24.0, 12.0, 0.0, "Gain M", false, &fmt_db);
            knob(ui, &mut m.high_gain_db, -24.0, 12.0, 0.0, "Gain H", false, &fmt_db);
            knob(ui, &mut m.mix, 0.0, 1.0, 1.0, "Mix", false, &fmt_pct);
        }
        EffectParams::Ott(o) => {
            knob(ui, &mut o.depth, 0.0, 1.0, 0.4, "Depth", false, &fmt_pct).on_hover_text("Gesamtstärke – 20–40 % reichen meist");
            knob(ui, &mut o.time, 0.1, 4.0, 1.0, "Zeit", true, &fmt_x);
            knob(ui, &mut o.upward, 0.0, 1.0, 1.0, "Upward", false, &fmt_pct).on_hover_text("Leise Details hochziehen");
            knob(ui, &mut o.downward, 0.0, 1.0, 1.0, "Downward", false, &fmt_pct).on_hover_text("Spitzen zähmen");
            knob(ui, &mut o.threshold_db, -48.0, 0.0, -24.0, "Schwelle", false, &fmt_db);
            knob(ui, &mut o.in_gain_db, -24.0, 24.0, 0.0, "In", false, &fmt_db);
            knob(ui, &mut o.out_gain_db, -24.0, 24.0, 0.0, "Out", false, &fmt_db);
            knob(ui, &mut o.low_gain_db, -24.0, 12.0, 0.0, "Tief", false, &fmt_db);
            knob(ui, &mut o.mid_gain_db, -24.0, 12.0, 0.0, "Mitte", false, &fmt_db);
            knob(ui, &mut o.high_gain_db, -24.0, 12.0, 0.0, "Hoch", false, &fmt_db);
        }
        EffectParams::Width(w) => {
            knob(ui, &mut w.width, 0.0, 2.0, 1.0, "Breite", false, &fmt_pct).on_hover_text("0 % = Mono, 100 % = unverändert, 200 % = doppelt so breit (Mono-Summe bleibt gleich)");
            knob(ui, &mut w.mid_db, -24.0, 12.0, 0.0, "Mitte", false, &fmt_db).on_hover_text("Pegel des Mittensignals (L+R)");
            knob(ui, &mut w.side_db, -24.0, 12.0, 0.0, "Seite", false, &fmt_db).on_hover_text("Pegel des Seitensignals (L−R)");
            let mut on = w.bass_mono_hz >= tstudio_dsp::fx_width::BASS_MONO_MIN_HZ;
            ui.vertical(|ui| {
                ui.label(RichText::new("Bass mono").small());
                if ui.checkbox(&mut on, "").on_hover_text("Alles unterhalb der Frequenz mono machen – hält Kick und Bass club-tauglich").changed() {
                    w.bass_mono_hz = if on { 120.0 } else { 0.0 };
                }
            });
            if on {
                knob(ui, &mut w.bass_mono_hz, 20.0, 500.0, 120.0, "Mono bis", true, &fmt_hz);
            }
        }
    });
}

pub fn filter_mode_combo(ui: &mut Ui, m: &mut FilterMode) {
    egui::ComboBox::from_id_salt(ui.id().with("fmode")).selected_text(m.name()).width(90.0).show_ui(ui, |ui| {
        for f in FilterMode::ALL {
            ui.selectable_value(m, f, f.name());
        }
    });
}

const BEAT_VALUES: [(f32, &str); 9] = [
    (0.125, "1/32"),
    (0.25, "1/16"),
    (1.0 / 3.0, "1/8 T"),
    (0.5, "1/8"),
    (0.75, "1/8 punktiert"),
    (1.0, "1/4"),
    (1.5, "1/4 punktiert"),
    (2.0, "1/2"),
    (4.0, "1 Takt"),
];

fn beats_name(b: f32) -> String {
    BEAT_VALUES.iter().find(|(v, _)| (v - b).abs() < 1e-4).map(|(_, n)| n.to_string()).unwrap_or_else(|| format!("{b:.2} Beats"))
}
