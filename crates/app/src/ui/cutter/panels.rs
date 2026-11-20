//! Save panel and isolation (EQ / mid-side) panel of the sample cutter.

use super::*;

pub(super) fn save_panel(app: &mut TStudio, ui: &mut Ui, acts: &mut Vec<Act>) {
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

pub(super) fn iso_panel(app: &mut TStudio, ui: &mut Ui, acts: &mut Vec<Act>, now: f64, pointer_down: bool) {
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
