//! The TSynth editor pages: sound, kick, filter, LFO, mod matrix, FX.

use super::*;

pub(super) fn osc_card(ui: &mut Ui, i: usize, o: &mut OscParams) {
    let col = OSC_COL[i];
    let on = head(ui, Some(&mut o.enabled), &format!("Osc {}", i + 1), col, |ui| {
        if o.table == WtKind::Basic {
            combo(ui, ("wave", i), &mut o.wave, &Waveform::ALL, Waveform::name, 64.0);
        }
        combo(ui, ("wt", i), &mut o.table, &WtKind::ALL, WtKind::name, 84.0);
    });
    ui.add_enabled_ui(on, |ui| {
        let (rect, resp) = curve(ui, 70.0, &o.preview(200), true, col);
        if o.table != WtKind::Basic {
            // drag horizontally over the display to scan the wavetable
            if resp.dragged() {
                o.position = (o.position + resp.drag_delta().x / rect.width()).clamp(0.0, 1.0);
            }
            let x = rect.left() + 4.0 + (rect.width() - 8.0) * o.position;
            ui.painter().line_segment([pos2(x, rect.bottom() - 7.0), pos2(x, rect.bottom())], Stroke::new(3.0f32, col));
            ui.painter().text(rect.left_top() + vec2(5.0, 3.0), Align2::LEFT_TOP, format!("Pos {:.0} %", o.position * 100.0), FontId::proportional(10.0), TEXT_DIM);
            resp.on_hover_text("Ziehen: Wavetable-Position");
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            small_label(ui, "Warp");
            combo(ui, ("warp", i), &mut o.warp, &WarpMode::ALL, WarpMode::name, 84.0);
            ui.add(egui::DragValue::new(&mut o.octave).range(-4..=4).prefix("Okt "));
            ui.add(egui::DragValue::new(&mut o.semi).range(-12..=12).prefix("St "));
        });
        knobs(ui, |ui| {
            if o.table != WtKind::Basic {
                knob(ui, &mut o.position, 0.0, 1.0, 0.0, "Position", false, &widgets::fmt_pct);
            }
            knob(ui, &mut o.warp_amount, 0.0, 1.0, 0.0, "Warp", false, &widgets::fmt_pct);
            knob(ui, &mut o.fine, -100.0, 100.0, 0.0, "Fine", false, &|v| format!("{v:+.0} ct"));
            knob(ui, &mut o.level, 0.0, 1.0, 0.7, "Level", false, &widgets::fmt_pct);
            knob(ui, &mut o.pan, -1.0, 1.0, 0.0, "Pan", false, &widgets::fmt_pan);
        });
        ui.add_space(2.0);
        small_label(ui, "Unison");
        knobs(ui, |ui| {
            let mut uni = o.unison as f32;
            knob(ui, &mut uni, 1.0, MAX_UNISON as f32, 1.0, "Stimmen", false, &|v| format!("{:.0}", v.round()));
            o.unison = uni.round() as u32;
            knob(ui, &mut o.detune, 0.0, 100.0, 20.0, "Detune", false, &|v| format!("{v:.0} ct"));
            knob(ui, &mut o.blend, 0.0, 1.0, 1.0, "Blend", false, &widgets::fmt_pct).on_hover_text("Lautstärke der äußeren Unison-Stimmen");
            knob(ui, &mut o.spread, 0.0, 1.0, 0.6, "Stereo", false, &widgets::fmt_pct);
            knob(ui, &mut o.rand_phase, 0.0, 1.0, 1.0, "Rnd Phase", false, &widgets::fmt_pct).on_hover_text("0 = jede Note startet gleich (Bässe, Kicks)");
        });
    });
}

pub(super) fn sub_noise_card(ui: &mut Ui, p: &mut SynthParams) {
    head(ui, None, "Sub", OSC_COL[0], |ui| {
        ui.add(egui::DragValue::new(&mut p.sub_octave).range(-3..=0).prefix("Okt "));
        combo(ui, "subw", &mut p.sub_wave, &Waveform::ALL, Waveform::name, 64.0);
    });
    knobs(ui, |ui| {
        knob(ui, &mut p.sub_level, 0.0, 1.0, 0.0, "Sub", false, &widgets::fmt_pct);
    });
    ui.add_space(6.0);
    head(ui, None, "Noise", TEXT_DIM, |ui| {
        combo(ui, "noise", &mut p.noise_type, &NoiseType::ALL, NoiseType::name, 72.0);
    });
    knobs(ui, |ui| {
        knob(ui, &mut p.noise_level, 0.0, 1.0, 0.0, "Noise", false, &widgets::fmt_pct);
        knob(ui, &mut p.noise_tone, 200.0, 20000.0, 20000.0, "Tone", true, &widgets::fmt_hz);
    });
}

pub(super) fn sound_page(ui: &mut Ui, p: &mut SynthParams) {
    card_grid(ui, "osc", 300.0, 4, NUM_OSC + 1, |ui, i| {
        if i < NUM_OSC {
            osc_card(ui, i, p.osc_mut(i));
        } else {
            sub_noise_card(ui, p);
        }
    });
}

pub(super) fn kick_page(ui: &mut Ui, p: &mut SynthParams) {
    if !p.kick.enabled {
        wide_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut p.kick.enabled, RichText::new("Kick-Engine einschalten").strong().color(ACCENT));
                small_label(ui, "Pitch-Sweep, Click, Drive und Rumble für Schranz-/Hardtekk-Kicks. Oder ein Preset aus der Kategorie „Kick“ laden.");
            });
        });
    }
    card_grid(ui, "kick", 290.0, 3, 6, |ui, i| {
        let on = p.kick.enabled;
        let k = &mut p.kick;
        match i {
            0 => {
                head(ui, Some(&mut k.enabled), "Kick-Engine · Pitch-Sweep", ACCENT, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    // pitch sweep display (log frequency, 0..300 ms)
                    let ys: Vec<f32> = (0..200)
                        .map(|i| {
                            let f = k.freq_at(60, i as f32 / 200.0 * 0.3);
                            ((f / 20.0).log2() / (5000f32 / 20.0).log2()).clamp(0.0, 1.0)
                        })
                        .collect();
                    let (rect, _) = curve(ui, 70.0, &ys, false, ACCENT);
                    let f0 = tstudio_dsp::midi_to_freq(k.tune);
                    ui.painter().text(rect.right_bottom() - vec2(5.0, 4.0), Align2::RIGHT_BOTTOM, format!("{f0:.1} Hz · 300 ms"), FontId::proportional(10.0), TEXT_DIM);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        small_label(ui, "Grundton");
                        note_drag(ui, &mut k.tune, 12.0, 60.0).on_hover_text("Tonhöhe des Kick-Körpers nach dem Pitch-Sweep");
                        ui.checkbox(&mut k.key_follow, "Taste transponiert").on_hover_text("Aus: jede Taste spielt die gestimmte Kick");
                    });
                    knobs(ui, |ui| {
                        knob(ui, &mut k.pitch_start, 0.0, 72.0, 40.0, "Start", false, &|v| format!("+{v:.0} st")).on_hover_text("Halbtöne über dem Grundton beim Anschlag");
                        knob(ui, &mut k.pitch_time, 0.005, 0.5, 0.07, "Zeit", true, &widgets::fmt_s);
                        knob(ui, &mut k.pitch_curve, -1.0, 1.0, 0.6, "Kurve", false, &|v| format!("{v:+.2}")).on_hover_text("> 0: schnell fallen, dann landen (Punch)");
                    });
                });
            }
            1 => {
                head(ui, None, "Click", CLICK_COL, |ui| {
                    ui.add_enabled_ui(on, |ui| combo(ui, "click", &mut k.click_type, &ClickType::ALL, ClickType::name, 64.0));
                });
                ui.add_enabled_ui(on, |ui| {
                    // click envelope over the first 50 ms
                    let ys: Vec<f32> = (0..120).map(|i| k.click_level.min(1.0) * (-(i as f32 / 119.0 * 0.05) / k.click_decay.max(1e-4)).exp()).collect();
                    let (rect, _) = curve(ui, 56.0, &ys, false, CLICK_COL);
                    ui.painter().text(rect.right_top() + vec2(-4.0, 3.0), Align2::RIGHT_TOP, "50 ms", FontId::proportional(10.0), TEXT_DIM);
                    ui.add_space(4.0);
                    knobs(ui, |ui| {
                        knob(ui, &mut k.click_level, 0.0, 1.0, 0.3, "Level", false, &widgets::fmt_pct);
                        knob(ui, &mut k.click_tone, 300.0, 12000.0, 2800.0, "Tone", true, &widgets::fmt_hz);
                        knob(ui, &mut k.click_decay, 0.0005, 0.05, 0.006, "Decay", true, &widgets::fmt_s);
                    });
                });
            }
            2 => {
                head(ui, None, "Body / Drive", FX_COL, |ui| {
                    ui.add_enabled_ui(on, |ui| combo(ui, "kdrive", &mut k.drive_mode, &DriveMode::ALL, DriveMode::name, 60.0));
                });
                ui.add_enabled_ui(on, |ui| {
                    drive_display(ui, k.drive_mode, k.drive_db, FX_COL);
                    ui.add_space(4.0);
                    knobs(ui, |ui| {
                        knob(ui, &mut k.drive_db, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db).on_hover_text("Schranz: 15–24 dB Hard, Hardtekk: Asym");
                        knob(ui, &mut k.body_hold, 0.0, 0.3, 0.0, "Hold", false, &widgets::fmt_s);
                        knob(ui, &mut k.body_decay, 0.0, 2.0, 0.0, "Decay", false, &|v| if v <= 0.0 { "Amp-Env".into() } else { widgets::fmt_s(v) }).on_hover_text("0 = Amp-Hüllkurve wirkt VOR dem Drive: Drive hält den Bass lang und fett (Standard).
> 0: eigene Körper-Hüllkurve, Amp-Hüllkurve danach.");
                    });
                });
            }
            3 => env_card(ui, "Env 1 · Amp (Kick-Länge)", &mut p.amp_env, OSC_COL[0]),
            4 => {
                head(ui, None, "Rumble", OSC_COL[1], |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut k.rumble_level, 0.0, 1.0, 0.0, "Level", false, &widgets::fmt_pct);
                        knob(ui, &mut k.rumble_delay, 0.0, 0.3, 0.06, "Delay", false, &widgets::fmt_s);
                        knob(ui, &mut k.rumble_decay, 0.05, 3.0, 0.4, "Decay", true, &widgets::fmt_s);
                        knob(ui, &mut k.rumble_tone, 40.0, 600.0, 160.0, "Tone", true, &widgets::fmt_hz);
                    });
                    small_label(ui, "Tiefpassgefilterte Hallfahne des Körpers. Für langen Rumble die Amp-Hüllkurve lang lassen.");
                });
            }
            _ => {
                head(ui, None, "Pitch-Hüllkurve (klassisch)", TEXT_DIM, |_| {});
                knobs(ui, |ui| {
                    knob(ui, &mut p.pitch_env_amount, -24.0, 72.0, 0.0, "Pitch", false, &|v| format!("{v:+.0} st")).on_hover_text("Tonhöhe beim Anschlag, fällt auf 0 ab – Zaps, Toms");
                    knob(ui, &mut p.pitch_env_decay, 0.002, 0.5, 0.04, "Decay", true, &widgets::fmt_s);
                });
                small_label(ui, "Wirkt auf alle Oszillatoren, auch ohne Kick-Engine.");
            }
        }
    });
}

pub(super) fn filter_slot(ui: &mut Ui, id: &str, kind: &mut FilterType, mode: &mut tstudio_dsp::filter::FilterMode) {
    // called inside a right-to-left layout: mode first so it ends up on the right
    if kind.uses_mode() {
        ui.push_id(id, |ui| filter_mode_combo(ui, mode));
    }
    combo(ui, (id, "type"), kind, &FilterType::ALL, FilterType::name, 70.0);
}

pub(super) fn filter_page(ui: &mut Ui, p: &mut SynthParams) {
    card_grid(ui, "filter", 330.0, 2, 4, |ui, i| match i {
        0 => {
            let mut on = p.filter_enabled;
            let active = head(ui, Some(&mut on), "Filter 1", OSC_COL[1], |ui| {
                ui.add_enabled_ui(p.filter_enabled, |ui| filter_slot(ui, "f1", &mut p.filter_type, &mut p.filter_mode));
            });
            p.filter_enabled = on;
            ui.add_enabled_ui(active, |ui| {
                filter_display(ui, p.filter_type, p.filter_mode, p.cutoff, p.resonance, OSC_COL[1]);
                ui.add_space(4.0);
                knobs(ui, |ui| {
                    let label = if p.filter_type == FilterType::Formant { "Vokal" } else { "Cutoff" };
                    knob(ui, &mut p.cutoff, 20.0, 20000.0, 4000.0, label, true, &widgets::fmt_hz);
                    knob(ui, &mut p.resonance, 0.0, 1.0, 0.15, "Reso", false, &widgets::fmt_pct);
                    knob(ui, &mut p.filter_drive, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db);
                    knob(ui, &mut p.filter_env_amount, -8.0, 8.0, 1.5, "Env 2", false, &|v| format!("{v:+.1} oct"));
                    knob(ui, &mut p.key_track, 0.0, 1.0, 0.3, "Keytrack", false, &widgets::fmt_pct);
                });
            });
        }
        1 => {
            let f2 = &mut p.filter2;
            let mut on = f2.enabled;
            let active = head(ui, Some(&mut on), "Filter 2", OSC_COL[2], |ui| {
                ui.add_enabled_ui(f2.enabled, |ui| filter_slot(ui, "f2", &mut f2.kind, &mut f2.mode));
            });
            f2.enabled = on;
            ui.add_enabled_ui(active, |ui| {
                filter_display(ui, f2.kind, f2.mode, f2.cutoff, f2.resonance, OSC_COL[2]);
                ui.add_space(4.0);
                knobs(ui, |ui| {
                    knob(ui, &mut f2.cutoff, 20.0, 20000.0, 120.0, "Cutoff", true, &widgets::fmt_hz);
                    knob(ui, &mut f2.resonance, 0.0, 1.0, 0.1, "Reso", false, &widgets::fmt_pct);
                    knob(ui, &mut f2.drive_db, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db);
                    knob(ui, &mut f2.env_amount, -8.0, 8.0, 0.0, "Env 2", false, &|v| format!("{v:+.1} oct"));
                    knob(ui, &mut f2.key_track, 0.0, 1.0, 0.0, "Keytrack", false, &widgets::fmt_pct);
                });
                ui.horizontal(|ui| {
                    small_label(ui, "Routing");
                    ui.selectable_value(&mut p.filter_routing, FilterRouting::Serial, "Seriell (1 → 2)");
                    ui.selectable_value(&mut p.filter_routing, FilterRouting::Parallel, "Parallel");
                });
            });
        }
        2 => env_card(ui, "Env 2 · Filter", &mut p.filter_env, OSC_COL[1]),
        _ => {
            head(ui, None, "Sättigung (nach Filter)", FX_COL, |ui| {
                combo(ui, "pdrive", &mut p.drive_mode, &DriveMode::ALL, DriveMode::name, 60.0);
            });
            drive_display(ui, p.drive_mode, p.drive_db, FX_COL);
            ui.add_space(4.0);
            knobs(ui, |ui| {
                knob(ui, &mut p.drive_db, 0.0, 36.0, 0.0, "Drive", false, &widgets::fmt_db);
            });
        }
    });
}

pub(super) fn lfo_card(ui: &mut Ui, i: usize, l: &mut LfoParams) {
    head(ui, None, &format!("LFO {}", i + 1), MOD_COL, |ui| {
        if l.shape == LfoShape::Wave {
            combo(ui, ("lwave", i), &mut l.wave, &Waveform::ALL, Waveform::name, 64.0);
        }
        combo(ui, ("lshape", i), &mut l.shape, &LfoShape::ALL, LfoShape::name, 64.0);
    });
    let ys: Vec<f32> = (0..160).map(|k| l.value_at(k as f32 / 160.0)).collect();
    let (rect, resp) = curve(ui, 56.0, &ys, true, MOD_COL);
    if l.shape == LfoShape::Custom {
        // draw the 16 steps: click / drag sets a step
        if let Some(pt) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.clicked() {
                let s = (((pt.x - rect.left()) / rect.width()) * 16.0).clamp(0.0, 15.0) as usize;
                l.custom[s] = (1.0 - 2.0 * (pt.y - rect.top()) / rect.height()).clamp(-1.0, 1.0);
            }
        }
        resp.on_hover_text("Klicken/ziehen: Stufen zeichnen");
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut l.sync, "Sync");
        ui.checkbox(&mut l.retrig, "Retrig");
        if l.shape == LfoShape::Custom {
            ui.checkbox(&mut l.smooth, "Glatt");
        }
    });
    ui.horizontal(|ui| {
        small_label(ui, "Direkt →");
        combo(ui, ("ltarget", i), &mut l.target, &LfoTarget::ALL, LfoTarget::name, 64.0);
        if l.sync {
            const DIV: [(f32, &str); 9] = [(0.125, "1/32"), (0.25, "1/16"), (0.5, "1/8"), (1.0, "1/4"), (2.0, "1/2"), (4.0, "1 Takt"), (8.0, "2 Takte"), (16.0, "4 Takte"), (32.0, "8 Takte")];
            let cur = DIV.iter().find(|(b, _)| (b - l.beats).abs() < 1e-4).map(|(_, n)| *n).unwrap_or("frei");
            egui::ComboBox::from_id_salt(("ldiv", i)).selected_text(cur).width(64.0).show_ui(ui, |ui| {
                for (b, n) in DIV {
                    ui.selectable_value(&mut l.beats, b, n);
                }
            });
        }
    });
    knobs(ui, |ui| {
        if !l.sync {
            knob(ui, &mut l.rate, 0.02, 30.0, 5.0, "Rate", true, &|v| format!("{v:.2} Hz"));
        }
        knob(ui, &mut l.amount, 0.0, 1.0, 0.0, "Direkt", false, &widgets::fmt_pct).on_hover_text("Menge für das direkte Ziel (Matrix-Mengen stellt man in der Matrix ein)");
        knob(ui, &mut l.phase, 0.0, 1.0, 0.0, "Phase", false, &widgets::fmt_pct);
    });
}

pub(super) fn mod_page(ui: &mut Ui, p: &mut SynthParams) {
    card_grid(ui, "envs", 400.0, 3, 3, |ui, i| match i {
        0 => env_card(ui, "Env 1 · Amp", &mut p.amp_env, OSC_COL[0]),
        1 => env_card(ui, "Env 2 · Filter", &mut p.filter_env, OSC_COL[1]),
        _ => env_card(ui, "Env 3 · Mod", &mut p.mod_env, MOD_COL),
    });
    card_grid(ui, "lfos", 250.0, 4, NUM_LFO, |ui, i| lfo_card(ui, i, p.lfo_mut(i)));
    wide_card(ui, |ui| mod_matrix(ui, p));
}

pub(super) fn mod_matrix(ui: &mut Ui, p: &mut SynthParams) {
    let used = p.mod_matrix.iter().filter(|s| s.active()).count();
    head(ui, None, "Mod-Matrix", MOD_COL, |ui| {
        small_label(ui, &format!("{used} / {NUM_MOD_SLOTS} aktiv"));
    });
    // drag sources
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        small_label(ui, "Quelle auf eine Zeile ziehen:");
        for s in ModSource::ALL.iter().skip(1) {
            let id = egui::Id::new(("modsrc", *s as u8));
            let w = ui.painter().layout_no_wrap(s.name().to_string(), egui::TextStyle::Small.resolve(ui.style()), TITLE).size().x + 16.0;
            if ui.max_rect().right() - ui.cursor().left() < w {
                ui.end_row();
            }
            ui.dnd_drag_source(id, *s, |ui| chip(ui, s.name(), BG_PANEL, if s.is_macro() { MOD_COL } else { TITLE }));
        }
    });
    ui.add_space(6.0);
    let last_used = p.mod_matrix.iter().rposition(|s| s.source != ModSource::Off || s.dest != ModDest::Off).map(|i| i + 1).unwrap_or(0);
    let show = (last_used + 1).min(NUM_MOD_SLOTS);
    let avail = ui.available_width();
    let narrow = avail < 720.0;
    let (src_w, dst_w) = if narrow { (92.0, 118.0) } else { (110.0, 150.0) };
    let slider_w = (avail - if narrow { 420.0 } else { 520.0 }).clamp(50.0, 360.0);
    egui::Grid::new("modmatrix").num_columns(5).spacing([10.0, 4.0]).striped(true).min_col_width(0.0).show(ui, |ui| {
        small_label(ui, "#");
        small_label(ui, "Quelle");
        small_label(ui, "Ziel");
        small_label(ui, "Menge");
        ui.label("");
        ui.end_row();
        for (i, slot) in p.mod_matrix.iter_mut().enumerate().take(show) {
            ui.label(RichText::new(format!("{}", i + 1)).color(TEXT_DIM));
            let r = ui.push_id(("src", i), |ui| combo(ui, ("msrc", i), &mut slot.source, &ModSource::ALL, ModSource::name, src_w)).response;
            if let Some(src) = r.dnd_release_payload::<ModSource>() {
                slot.source = *src;
                if slot.amount == 0.0 {
                    slot.amount = 0.5;
                }
            }
            ui.push_id(("dst", i), |ui| combo(ui, ("mdst", i), &mut slot.dest, &ModDest::ALL, ModDest::name, dst_w));
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = slider_w;
                ui.add(egui::Slider::new(&mut slot.amount, -1.0..=1.0).show_value(false));
                ui.add_sized(vec2(48.0, 18.0), egui::Label::new(RichText::new(format!("{:+.0} %", slot.amount * 100.0)).monospace()));
            });
            ui.horizontal(|ui| {
                if slot.dest.is_global() && slot.source != ModSource::Off && !slot.source.is_macro() {
                    ui.colored_label(FX_COL, "⚠ nur Makros").on_hover_text("FX-Ziele gelten für alle Stimmen und nehmen nur Makros");
                }
                if (slot.source != ModSource::Off || slot.dest != ModDest::Off) && ui.small_button("✕").on_hover_text("Slot leeren").clicked() {
                    *slot = ModSlot::default();
                }
            });
            ui.end_row();
        }
    });
}

pub(super) fn fx_page(ui: &mut Ui, p: &mut SynthParams) {
    // signal chain overview
    wide_card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            small_label(ui, "Kette:");
            let fx = &p.fx;
            let chain = [("Dist", fx.dist_on), ("Comp", fx.comp_on), ("OTT", fx.ott_on), ("Chorus", fx.chorus_on), ("Delay", fx.delay_on), ("Reverb", fx.reverb_on), ("Clipper", fx.clip_on)];
            for (k, (name, on)) in chain.iter().enumerate() {
                if k > 0 {
                    small_label(ui, "→");
                }
                chip(ui, name, if *on { FX_COL.gamma_multiply(0.35) } else { BG_PANEL }, if *on { TITLE } else { TEXT_DIM });
            }
        });
    });
    card_grid(ui, "fx", 300.0, 4, 8, |ui, i| {
        let fx = &mut p.fx;
        match i {
            0 => {
                let on = head(ui, Some(&mut fx.dist_on), "Distortion", FX_COL, |ui| {
                    combo(ui, "fxdist", &mut fx.dist.mode, &DistMode::ALL, DistMode::name, 80.0);
                });
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.dist.drive_db, 0.0, 40.0, 12.0, "Drive", false, &widgets::fmt_db);
                        knob(ui, &mut fx.dist.tone, 500.0, 20000.0, 9000.0, "Tone", true, &widgets::fmt_hz);
                        knob(ui, &mut fx.dist.output_db, -24.0, 6.0, -3.0, "Out", false, &widgets::fmt_db);
                        knob(ui, &mut fx.dist.mix, 0.0, 1.0, 1.0, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            1 => {
                let on = head(ui, Some(&mut fx.comp_on), "Kompressor", FX_COL, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.comp.threshold, -60.0, 0.0, -18.0, "Thresh", false, &widgets::fmt_db);
                        knob(ui, &mut fx.comp.ratio, 1.0, 20.0, 4.0, "Ratio", false, &|v| format!("{v:.1}:1"));
                        knob(ui, &mut fx.comp.attack_ms, 0.1, 200.0, 10.0, "Attack", true, &widgets::fmt_ms);
                        knob(ui, &mut fx.comp.release_ms, 5.0, 2000.0, 120.0, "Release", true, &widgets::fmt_ms);
                        knob(ui, &mut fx.comp.makeup, -12.0, 24.0, 0.0, "Makeup", false, &widgets::fmt_db);
                    });
                });
            }
            2 => {
                let on = head(ui, Some(&mut fx.ott_on), "OTT", FX_COL, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.ott.depth, 0.0, 1.0, 0.4, "Depth", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.ott.time, 0.2, 3.0, 1.0, "Time", true, &widgets::fmt_x);
                        knob(ui, &mut fx.ott.upward, 0.0, 1.0, 1.0, "Up", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.ott.downward, 0.0, 1.0, 1.0, "Down", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.ott.out_gain_db, -24.0, 12.0, 0.0, "Out", false, &widgets::fmt_db);
                    });
                });
            }
            3 => {
                let on = head(ui, Some(&mut fx.chorus_on), "Chorus", OSC_COL[1], |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.chorus.rate, 0.05, 5.0, 0.8, "Rate", true, &|v| format!("{v:.2} Hz"));
                        knob(ui, &mut fx.chorus.depth_ms, 0.0, 10.0, 3.0, "Depth", false, &widgets::fmt_ms);
                        knob(ui, &mut fx.chorus.mix, 0.0, 1.0, 0.35, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            4 => {
                let on = head(ui, Some(&mut fx.delay_on), "Delay", OSC_COL[1], |ui| {
                    ui.checkbox(&mut fx.delay.ping_pong, "Ping-Pong");
                });
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.delay.time_beats, 0.0625, 2.0, 0.75, "Zeit", false, &|v| format!("{v:.3} Beats"));
                        knob(ui, &mut fx.delay.feedback, 0.0, 0.95, 0.4, "Feedback", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.delay.tone, 500.0, 20000.0, 6000.0, "Tone", true, &widgets::fmt_hz);
                        knob(ui, &mut fx.delay.mix, 0.0, 1.0, 0.2, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            5 => {
                let on = head(ui, Some(&mut fx.reverb_on), "Reverb", OSC_COL[1], |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.reverb.size, 0.0, 1.0, 0.6, "Größe", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.reverb.damping, 0.0, 1.0, 0.5, "Dämpf.", false, &widgets::fmt_pct);
                        knob(ui, &mut fx.reverb.predelay_ms, 0.0, 200.0, 10.0, "Pre", false, &widgets::fmt_ms);
                        knob(ui, &mut fx.reverb.mix, 0.0, 1.0, 0.2, "Mix", false, &widgets::fmt_pct);
                    });
                });
            }
            6 => {
                let on = head(ui, Some(&mut fx.clip_on), "Clipper (Ende)", FX_COL, |_| {});
                ui.add_enabled_ui(on, |ui| {
                    knobs(ui, |ui| {
                        knob(ui, &mut fx.clip.drive_db, 0.0, 24.0, 0.0, "Drive", false, &widgets::fmt_db);
                        knob(ui, &mut fx.clip.ceiling_db, -12.0, 0.0, -0.8, "Ceiling", false, &widgets::fmt_db);
                        knob(ui, &mut fx.clip.softness, 0.0, 1.0, 0.25, "Soft", false, &widgets::fmt_pct);
                    });
                });
            }
            _ => {
                head(ui, None, "Voicing / Ausgang", ACCENT, |_| {});
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.mono, "Mono");
                    ui.add_enabled(p.mono, egui::Checkbox::new(&mut p.legato, "Legato")).on_hover_text("An: gebundene Noten gleiten ohne Neuanschlag · Aus: jede Note triggert neu (Kicks, Stabs)");
                    ui.add_enabled(p.mono && p.glide > 0.0, egui::Checkbox::new(&mut p.slide_only, "303-Slide")).on_hover_text("Glide nur zwischen gebundenen (überlappenden) Noten – wie bei der TB-303");
                });
                knobs(ui, |ui| {
                    let mut poly = p.polyphony as f32;
                    ui.add_enabled_ui(!p.mono, |ui| {
                        knob(ui, &mut poly, 1.0, 16.0, 16.0, "Stimmen", false, &|v| format!("{:.0}", v.round()));
                    });
                    p.polyphony = poly.round().clamp(1.0, 16.0) as u32;
                    knob(ui, &mut p.glide, 0.0, 1.0, 0.0, "Glide", false, &widgets::fmt_s);
                    knob(ui, &mut p.velocity_sens, 0.0, 1.0, 0.6, "Velocity", false, &widgets::fmt_pct);
                    knob(ui, &mut p.gain, 0.0, 1.5, 0.5, "Gain", false, &widgets::fmt_gain);
                });
            }
        }
    });
}
