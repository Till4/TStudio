//! Command dispatch: `run` matches the parsed `Cmd` and delegates to the per-area modules.

pub(crate) use crate::*;

mod channel;
mod pattern;
mod notes;
mod clip;
mod fx;
mod mixer;
mod track;
mod auto;
mod marker;
mod section;

pub(crate) use channel::*;
pub(crate) use pattern::*;
pub(crate) use notes::*;
pub(crate) use clip::*;
pub(crate) use fx::*;
pub(crate) use mixer::*;
pub(crate) use track::*;
pub(crate) use auto::*;
pub(crate) use marker::*;
pub(crate) use section::*;

pub(crate) fn run(cmd: Cmd) -> R<Value> {
    match cmd {
        Cmd::New { project, bpm, name, empty, force } => {
            if live::is_keyword(&project) {
                return Err("`new` braucht einen Dateinamen; danach `open <datei>` lädt es in TStudio".into());
            }
            if project.exists() && live::targets_window(&project) {
                return Err(format!("{} ist gerade in TStudio offen – `new` würde das offene Projekt (mit ungespeicherten Änderungen) ersetzen. Anderen Dateinamen wählen.", project.display()));
            }
            if project.exists() && !force {
                return Err(format!("{} existiert schon (--force überschreibt)", project.display()));
            }
            let mut p = if empty {
                let mut p = Project::default();
                let id = p.alloc_id();
                p.patterns.push(tstudio_project::Pattern { id, ..Default::default() });
                p
            } else {
                Project::new_default()
            };
            p.bpm = bpm;
            p.name = name.unwrap_or_else(|| project.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("Untitled".into()));
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "project": abs(&project), "info": info(&p)}))
        }
        Cmd::Info { project } => Ok(info(&load(&project)?)),
        Cmd::Get { project, path } => {
            let v = serde_json::to_value(load(&project)?).map_err(|e| e.to_string())?;
            Ok(match path {
                Some(p) => get_path(&v, &p)?.clone(),
                None => v,
            })
        }
        Cmd::Set { project, assignments } => {
            let p = load(&project)?;
            let mut v = serde_json::to_value(&p).map_err(|e| e.to_string())?;
            let mut done = Vec::new();
            for a in &assignments {
                let (path, val) = a.split_once('=').ok_or_else(|| format!("'{a}': Format pfad=wert"))?;
                let val: Value = serde_json::from_str(val.trim()).unwrap_or_else(|_| Value::String(val.trim().to_string()));
                set_path(&mut v, path.trim(), val.clone())?;
                done.push(json!({"path": path.trim(), "value": val}));
            }
            let mut p: Project = serde_json::from_value(v).map_err(|e| format!("Wert passt nicht zum Datenmodell: {e}"))?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "set": done}))
        }
        Cmd::Channel(c) => channel_cmd(c),
        Cmd::Pattern(c) => pattern_cmd(c),
        Cmd::Notes(c) => notes_cmd(c),
        Cmd::Clip(c) => clip_cmd(c),
        Cmd::Fx(c) => fx_cmd(c),
        Cmd::Mixer(c) => mixer_cmd(c),
        Cmd::Track(c) => track_cmd(c),
        Cmd::Auto(c) => auto_cmd(c),
        Cmd::Soundfont { file } => {
            let path = abs(&file);
            let presets = soundfont_presets(&path)?;
            Ok(json!({
                "ok": true, "file": path, "count": presets.len(),
                "presets": presets.iter().map(|p| json!({"bank": p.bank, "program": p.program, "name": p.name})).collect::<Vec<_>>(),
                "hint": "channel add PROJECT --soundfont DATEI --preset NAME (oder --program N [--bank B]); Bank 128 = Drum-Kits",
            }))
        }
        Cmd::Render { project, out, pattern, loops, sr, bits, kbps, tail, normalize, artist, title, accent, size, fps, no_watermark, still, stems } => {
            let mut p = load(&project)?;
            let mode = match &pattern {
                Some(sel) => {
                    let id = pattern_id(&p, sel)?;
                    let idx = p.patterns.iter().position(|x| x.id == id).unwrap();
                    if loops > 1 {
                        // render the pattern N times by placing it in a scratch playlist
                        let len = p.pattern_length(id);
                        p.playlist.clips.clear();
                        for i in 0..loops as i64 {
                            p.add_clip(0, i * len, len, ClipKind::Pattern { pattern: id });
                        }
                        PlayMode::Song
                    } else {
                        PlayMode::Pattern(idx)
                    }
                }
                None => PlayMode::Song,
            };
            let t0 = std::time::Instant::now();
            let mut r = audio::render(&p, sr, mode, tail)?;
            if normalize {
                r.data.normalize(0.98);
            }
            let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let mut video = Value::Null;
            if ext == "mp4" || ext == "png" || still.is_some() {
                let mut o = tstudio_fetch::video::VideoOpts::default();
                o.artist = artist.unwrap_or_else(settings_dj_name);
                o.title = title.unwrap_or_else(|| if p.name.trim().is_empty() { out.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { p.name.clone() });
                if let Some(a) = &accent {
                    o.accent = tstudio_fetch::video::accent_by_name(a).ok_or(format!("Unbekannte Farbe {a} (orange, rot, pink, lila, cyan, grün oder #RRGGBB)"))?;
                }
                if let Some(sz) = &size {
                    (o.width, o.height) = tstudio_fetch::video::size_by_name(sz).ok_or(format!("Unbekannte Größe {sz} (1080p, 720p, vertical, square)"))?;
                }
                o.fps = fps;
                o.watermark = !no_watermark;
                let ff = tstudio_fetch::Tools::detect(None, None).ffmpeg.ok_or("Video braucht ffmpeg (winget install Gyan.FFmpeg)")?;
                if let Some(dir) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                let t1 = std::time::Instant::now();
                if let Some(secs) = still.or(if ext == "png" { Some(r.data.duration_secs() as f32 * 0.4) } else { None }) {
                    tstudio_fetch::video::export_still(&ff, &r.data, &o, secs, &out)?;
                } else {
                    let mut last = -1i32;
                    tstudio_fetch::video::export_mp4(&ff, &r.data, &out, &o, |f| {
                        let pc = (f * 100.0) as i32;
                        if pc != last {
                            last = pc;
                            eprint!("\rVideo {pc:3}%");
                        }
                        true
                    })?;
                    eprintln!();
                }
                video = json!({"artist": o.artist, "title": o.title, "size": [o.width, o.height], "fps": o.fps, "video_ms": t1.elapsed().as_millis()});
            } else {
                audio::save_audio(&r.data, &out, bits, kbps)?;
            }
            let mut stem_files = Vec::new();
            if let Some(dir) = &stems {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                for s in tools::render_stems(&p, sr)? {
                    let clean: String = s.name.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
                    let f = dir.join(format!("{:02} {}.wav", s.track, clean.trim()));
                    audio::save_audio(&s.data, &f, bits, kbps)?;
                    stem_files.push(json!({"track": s.track, "name": s.name, "bus": s.bus, "file": abs(&f)}));
                }
            }
            Ok(json!({
                "ok": true,
                "out": abs(&out),
                "stems": stem_files,
                "video": video,
                "render_ms": t0.elapsed().as_millis(),
                "missing_samples": r.missing,
                "stats": audio::stats(&r.data, "render", Some((p.bpm, p.beats_per_bar))),
            }))
        }
        Cmd::Analyze { file, bpm, grid, chords, chord_beats, mix, beats_per_bar } => {
            let d = audio::load(&file)?;
            let name = file.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let mut v = audio::stats(&d, &name, bpm.map(|b| (b, beats_per_bar)));
            v["file"] = json!(abs(&file));
            v["channels_in_file"] = json!(if d.left == d.right { 1 } else { 2 });
            let mono = d.mono();
            if d.duration_secs() < 2.5 {
                if let Some(f) = tstudio_dsp::music::fundamental(&mono, d.sample_rate) {
                    v["fundamental"] = json!(f);
                    v["fundamental_hint"] = json!("Grundton eines One-Shots (Kick/808): `tune IN OUT --to <Ton>` stimmt ihn auf die Tonart");
                }
            }
            if grid || chords {
                let hint = bpm.or_else(|| if v["bpm_from_name"].as_bool() == Some(true) { v["detected_bpm"].as_f64() } else { None });
                let g = tstudio_dsp::music::beat_grid(&mono, d.sample_rate, hint, beats_per_bar).ok_or("Kein Beat-Grid gefunden (zu kurz oder kein Rhythmus) – --bpm angeben")?;
                let bars: Vec<f64> = (0..32).map(|b| g.from_downbeat((b * beats_per_bar as i64) as f64)).filter(|t| *t < d.duration_secs()).map(|t| (t * 1000.0).round() / 1000.0).collect();
                let mut gv = json!(g);
                gv["bar_starts_s"] = json!(bars);
                gv["alternatives_bpm"] = json!([g.bpm / 2.0, g.bpm * 2.0]);
                gv["trim_example"] = json!(format!(
                    "tstudio-cli trim \"{}\" out.wav --from-bar 1 --bars 4 --bpm {} --downbeat {:.4}",
                    file.display(),
                    g.bpm,
                    g.downbeat_s
                ));
                if chords {
                    let seg = chord_beats.unwrap_or(beats_per_bar as f64);
                    let tl = tstudio_dsp::music::chord_timeline(&mono, d.sample_rate, &g, seg);
                    let prog: Vec<String> = tl.iter().filter(|c| c.chord != "N").take(16).map(|c| format!("{}:{}", c.chord, ((c.end_s - c.start_s) / g.bar_len_s * 100.0).round() / 100.0)).collect();
                    v["chords"] = json!(tl);
                    v["progression"] = json!(prog.join(" "));
                    v["progression_hint"] = json!("Direkt nutzbar: notes chords PROJECT -p P -c BASS \"<progression>\" --style offbeat");
                }
                v["grid"] = gv;
            }
            if let Some((k, conf)) = tstudio_dsp::music::detect_key(&mono, d.sample_rate) {
                v["key"] = json!(k);
                v["key_confidence"] = json!((conf * 100.0).round() / 100.0);
                if conf < 0.5 {
                    v["key_hint"] = json!("unsicher – in dichten Mixes mit verzerrter Kick/Rumble verfälschen die Drums die Tonart; besser Loops, Samples oder Stems (ohne Drums) analysieren");
                }
            }
            if mix {
                v["mix"] = json!(tstudio_dsp::meter::report(&d, true));
            }
            Ok(v)
        }
        Cmd::Compare { mine, reference } => {
            let a = audio::load(&mine)?;
            let b = audio::load(&reference)?;
            let b = if b.sample_rate != a.sample_rate { b.resample_to(a.sample_rate) } else { b };
            Ok(json!({"ok": true, "mine": abs(&mine), "reference": abs(&reference), "comparison": tstudio_dsp::meter::compare(&a, &b), "mine_report": tstudio_dsp::meter::report(&a, true)}))
        }
        Cmd::Mix { project, reference, master_only, sr } => {
            let p = load(&project)?;
            let t0 = std::time::Instant::now();
            let r = audio::render(&p, sr, PlayMode::Song, 1.0)?;
            let master = tstudio_dsp::meter::report(&r.data, true);
            let mut warnings = master.warnings.clone();
            let mut out = json!({"ok": true, "master": master, "missing_samples": r.missing});
            if !master_only {
                let stems = tools::render_stems(&p, sr)?;
                let (tracks, pairs, w) = tools::stems_report(&stems);
                warnings.extend(w);
                out["tracks"] = json!(tracks);
                out["masking"] = json!(pairs);
            }
            if let Some(rf) = reference {
                let b = audio::load(&rf)?;
                let b = if b.sample_rate != sr { b.resample_to(sr) } else { b };
                let c = tstudio_dsp::meter::compare(&r.data, &b);
                warnings.extend(c.suggestions.iter().map(|s| format!("Referenz: {s}")));
                out["reference"] = json!(c);
            }
            out["warnings"] = json!(warnings);
            out["analysis_ms"] = json!(t0.elapsed().as_millis());
            Ok(out)
        }
        Cmd::Stems { input, out, threads } => {
            let d = audio::load(&input)?;
            let py = tstudio_fetch::stems::find_python(None).ok_or_else(|| format!("Python nicht gefunden.\n{}", tstudio_fetch::stems::INSTALL_HINT))?;
            tstudio_fetch::stems::check(&py).map_err(|e| format!("{e}\n{}", tstudio_fetch::stems::INSTALL_HINT))?;
            let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("Song".into());
            let dest = out.unwrap_or_else(|| input.parent().unwrap_or(Path::new(".")).join(format!("{stem} Stems")));
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            let h = tstudio_fetch::stems::start_stems(&py, &d, &dest, &stem, tstudio_fetch::stems::thread_count(threads))?;
            let mut log = Vec::new();
            loop {
                match h.events.recv() {
                    Ok(tstudio_fetch::FetchEvent::Progress(p)) => eprint!("\rStems {:5.1}%", p * 100.0),
                    Ok(tstudio_fetch::FetchEvent::Log(l)) => log.push(l),
                    Ok(tstudio_fetch::FetchEvent::Finished(_)) => {
                        eprintln!();
                        let files: Vec<Value> = std::fs::read_dir(&dest)
                            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "wav").unwrap_or(false)).map(|p| json!(p)).collect())
                            .unwrap_or_default();
                        return Ok(json!({"ok": true, "dir": abs(&dest), "files": files}));
                    }
                    Ok(tstudio_fetch::FetchEvent::Failed(e)) => {
                        let tail: Vec<String> = log.iter().rev().take(8).rev().cloned().collect();
                        return Err(format!("{e}\n{}", tail.join("\n")));
                    }
                    Err(_) => return Err("Demucs beendet ohne Ergebnis".into()),
                }
            }
        }
        Cmd::Tune { input, output, to, mode } => {
            let d = audio::load(&input)?;
            let f = tstudio_dsp::music::fundamental(&d.mono(), d.sample_rate).ok_or("Kein Grundton gefunden (25–250 Hz)")?;
            let target_midi: f64 = match spec::parse_key(&to) {
                Ok(k) if to.chars().any(|c| c.is_ascii_digit()) => k as f64,
                _ => {
                    let pc = tstudio_dsp::music::pitch_class(&to).ok_or_else(|| format!("Ton '{to}' unbekannt (z.B. F, F#, Bb, G1)"))? as f64;
                    // nearest octave of that pitch class
                    let base = f.midi - (f.midi - pc).rem_euclid(12.0);
                    if f.midi - base > 6.0 { base + 12.0 } else { base }
                }
            };
            let semis = (target_midi - f.midi) as f32;
            let out = match mode.to_lowercase().as_str() {
                "pitch" | "stretch" => tstudio_dsp::stretch::pitch_shift(&d, semis),
                _ => d.resample_speed(2f64.powf(semis as f64 / 12.0)),
            };
            audio::save_audio(&out, &output, 24, 320)?;
            let after = tstudio_dsp::music::fundamental(&out.mono(), out.sample_rate);
            Ok(json!({"ok": true, "out": abs(&output), "before": f, "after": after, "shift_semitones": (semis * 100.0).round() / 100.0, "duration_s": out.duration_secs()}))
        }
        Cmd::Marker(c) => marker_cmd(c),
        Cmd::Section(c) => section_cmd(c),
        Cmd::Stretch { input, output, from, to, mode, pitch, formant } => {
            let d = audio::load(&input)?;
            let m = stretch_mode(&mode)?;
            let out = tstudio_dsp::stretch::change_tempo_ex(&d, m, from, to, pitch, formant);
            audio::save_audio(&out, &output, 24, 320)?;
            Ok(json!({"ok": true, "out": abs(&output), "in_s": d.duration_secs(), "out_s": out.duration_secs()}))
        }
        Cmd::Trim {
            input, output, start, end, fade_in_ms, fade_out_ms, normalize, trim_tail, bits, kbps, iso,
            beats, bars, from_beat, from_bar, bpm, downbeat, beats_per_bar, to_bpm, mode, pitch, formant,
        } => {
            let d = audio::load(&input)?;
            let sr = d.sample_rate as f64;
            let mut grid_info = Value::Null;
            let (start, end) = if beats.is_some() || bars.is_some() || from_beat.is_some() || from_bar.is_some() {
                let bpb = beats_per_bar.max(1) as f64;
                let (g_bpm, g_down) = match (bpm, downbeat) {
                    (Some(b), Some(t)) => (b, t),
                    _ => {
                        let hint = bpm.or_else(|| tstudio_dsp::analysis::bpm_from_name(&input.to_string_lossy()));
                        let g = tstudio_dsp::music::beat_grid(&d.mono(), d.sample_rate, hint, beats_per_bar).ok_or("Kein Beat-Grid gefunden – --bpm und --downbeat angeben")?;
                        grid_info = json!(g);
                        (g.bpm, downbeat.unwrap_or(g.downbeat_s))
                    }
                };
                let beat = 60.0 / g_bpm;
                let from = from_beat.unwrap_or(0.0) + from_bar.map(|b| (b - 1.0) * bpb).unwrap_or(0.0);
                let n = beats.unwrap_or(0.0) + bars.map(|b| b * bpb).unwrap_or(0.0);
                let s0 = g_down + from * beat;
                if s0 < -1e-6 {
                    return Err(format!("Start liegt vor dem Dateianfang ({s0:.3} s)"));
                }
                let e0 = if n > 0.0 { Some(s0 + n * beat) } else { None };
                if grid_info.is_null() {
                    grid_info = json!({"bpm": g_bpm, "downbeat_s": g_down});
                }
                grid_info["beats"] = json!(n);
                grid_info["start_s"] = json!(s0);
                grid_info["end_s"] = json!(e0);
                (s0.max(0.0), e0)
            } else {
                (start, end)
            };
            let a = ((start * sr).round() as usize).min(d.len());
            let b = end.map(|e| ((e * sr).round() as usize).min(d.len())).unwrap_or(d.len());
            if b <= a + 1 {
                return Err("leerer Bereich (start/end prüfen)".into());
            }
            let iso = iso.params()?;
            let mut out = match &iso {
                Some(p) => p.process_range(&d, a, b),
                None => d.slice(a, b),
            };
            if trim_tail {
                if let Some(last) = (0..out.len()).rev().find(|i| out.left[*i].abs() > 0.001 || out.right[*i].abs() > 0.001) {
                    let end = (last + d.sample_rate as usize / 200).min(out.len()).max(1);
                    out.left.truncate(end);
                    out.right.truncate(end);
                }
            }
            out.fade((fade_in_ms * 0.001 * sr as f32) as usize, (fade_out_ms * 0.001 * sr as f32) as usize);
            let src_bpm = grid_info["bpm"].as_f64();
            if to_bpm.is_some() || pitch != 0.0 {
                let m = stretch_mode(&mode)?;
                let from = src_bpm.ok_or("--to-bpm braucht das Tempo der Quelle (--bpm oder Beat-Raster mit --beats/--bars)").or_else(|e| if to_bpm.is_none() { Ok(1.0) } else { Err(e) })?;
                let to = to_bpm.unwrap_or(from);
                let m = if to_bpm.is_none() { StretchMode::Off } else { m };
                out = tstudio_dsp::stretch::change_tempo_ex(&out, m, from, to, pitch, formant);
            }
            if normalize {
                out.normalize(0.98);
            }
            audio::save_audio(&out, &output, bits, kbps)?;
            Ok(json!({"ok": true, "out": abs(&output), "duration_s": out.duration_secs(), "isolate": iso.map(|p| p.describe()), "grid": grid_info, "bpm_out": to_bpm.or(src_bpm)}))
        }
        Cmd::Chop { input, sensitivity, even, out_dir } => {
            let d = audio::load(&input)?;
            let mut points: Vec<usize> = match even {
                Some(n) => (0..n.max(1)).map(|i| i * d.len() / n.max(1)).collect(),
                None => tstudio_dsp::analysis::transients(&d.mono(), d.sample_rate, sensitivity, 40.0),
            };
            points.dedup();
            let sr = d.sample_rate as f64;
            let mut files = Vec::new();
            if let Some(dir) = &out_dir {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("slice".into());
                for (i, s) in points.iter().enumerate() {
                    let e = points.get(i + 1).copied().unwrap_or(d.len());
                    let mut part = d.slice(*s, e);
                    part.fade(0, (0.005 * sr) as usize);
                    let path = dir.join(format!("{stem}_{:02}.wav", i + 1));
                    tstudio_library::write_wav(&path, &part, tstudio_library::WavFormat::Pcm24)?;
                    files.push(abs(&path));
                }
            }
            Ok(json!({
                "ok": true,
                "slices": points.len(),
                "times_s": points.iter().map(|p| (*p as f64 / sr * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
                "fractions": points.iter().map(|p| *p as f64 / d.len().max(1) as f64).collect::<Vec<_>>(),
                "files": files,
                "hint": "fractions können als channels.N.instrument.Sampler.params.slices gesetzt werden (mode=\"Chop\")",
            }))
        }
        Cmd::Scan { dirs, filter, analyze, limit } => {
            let entries = tstudio_library::scan(&dirs);
            let f = filter.map(|s| s.to_lowercase());
            let list: Vec<Value> = entries
                .iter()
                .filter(|e| f.as_ref().map(|f| e.name.to_lowercase().contains(f) || e.rel_dir.to_lowercase().contains(f) || e.ext == *f).unwrap_or(true))
                .take(limit)
                .map(|e| {
                    let mut v = json!({"path": e.path, "name": e.name, "type": e.ext, "folder": e.rel_dir, "size_kb": e.size / 1024});
                    if analyze && e.ext != "sf2" {
                        match tstudio_library::analyze_file(&e.path) {
                            Ok(a) => {
                                v["duration_s"] = json!((a.duration * 100.0).round() / 100.0);
                                v["bpm"] = json!(a.bpm.map(|b| (b * 10.0).round() / 10.0));
                                v["key"] = json!(a.key);
                                v["loop"] = json!(a.is_loop());
                            }
                            Err(err) => v["error"] = json!(err),
                        }
                    }
                    v
                })
                .collect();
            Ok(json!({"total": entries.len(), "shown": list.len(), "files": list}))
        }
        Cmd::Fetch { url, format, out } => {
            let tools = tstudio_fetch::Tools::detect(None, None);
            let fmt = if format.eq_ignore_ascii_case("mp3") { tstudio_fetch::AudioFormat::Mp3 } else { tstudio_fetch::AudioFormat::Wav };
            let out = out.unwrap_or_else(default_download_dir);
            let h = tstudio_fetch::start_download(&tools, &url, &out, fmt)?;
            let mut log = Vec::new();
            loop {
                match h.events.recv() {
                    Ok(tstudio_fetch::FetchEvent::Progress(p)) => eprint!("\r{:5.1}%", p * 100.0),
                    Ok(tstudio_fetch::FetchEvent::Log(l)) => log.push(l),
                    Ok(tstudio_fetch::FetchEvent::Finished(p)) => {
                        eprintln!();
                        return Ok(json!({"ok": true, "file": p}));
                    }
                    Ok(tstudio_fetch::FetchEvent::Failed(e)) => {
                        let tail: Vec<String> = log.iter().rev().take(8).rev().cloned().collect();
                        return Err(format!("{e}\n{}", tail.join("\n")));
                    }
                    Err(_) => return Err("Download-Prozess beendet ohne Ergebnis".into()),
                }
            }
        }
        Cmd::Tools => {
            let t = tstudio_fetch::Tools::detect(None, None);
            Ok(json!({
                "ffmpeg": {"path": t.ffmpeg, "version": t.ffmpeg.as_deref().and_then(tstudio_fetch::tool_version), "install": "winget install Gyan.FFmpeg"},
                "yt-dlp": {"path": t.ytdlp, "version": t.ytdlp.as_deref().and_then(tstudio_fetch::tool_version), "install": "winget install yt-dlp.yt-dlp"},
            }))
        }
        Cmd::Presets => Ok(json!({
            "synth_presets": SynthParams::presets().iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            "synth_preset_categories": tstudio_dsp::synth::PRESET_CATEGORIES.iter().map(|c| json!({"category": c, "presets": SynthParams::presets_by_category().iter().filter(|(k, _, _)| k == c).map(|(_, n, _)| *n).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "synth_wavetables": tstudio_dsp::wavetable::WtKind::ALL.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            "synth_warp_modes": tstudio_dsp::synth::WarpMode::ALL.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            "synth_filter_types": tstudio_dsp::filter::FilterType::ALL.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            "synth_mod_sources": ModSource::ALL.iter().skip(1).map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            "synth_mod_dests": ModDest::ALL.iter().skip(1).map(|k| format!("{k:?}")).collect::<Vec<_>>(),
        "synth_automatable": SynthParams::FIELDS,
            "synth_params_example": SynthParams::default(),
            "effects": EffectKind::ALL.iter().map(|k| json!({"kind": k.name(), "default": k.default_params()})).collect::<Vec<_>>(),
            "stretch_modes": ["stretch", "resample", "off"],
            "fx_chains": tools::CHAINS.iter().map(|(n, d)| json!({"name": n, "description": d})).collect::<Vec<_>>(),
            "chord_styles": ["block", "pad", "stab", "arp", "bass", "offbeat", "rolling"],
            "isolate_presets": tstudio_dsp::isolate::IsolateParams::presets().iter().map(|(n, p)| json!({"name": n, "describe": p.describe(), "params": p})).collect::<Vec<_>>(),
            "sampler_params_example": tstudio_dsp::sampler::SamplerParams::default(),
        })),
        Cmd::Open { project } => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let gui = exe.with_file_name(if cfg!(windows) { "tstudio.exe" } else { "tstudio" });
            if !gui.exists() {
                return Err(format!("GUI nicht gefunden: {} (cargo build -p tstudio --release)", gui.display()));
            }
            let path = abs(&project);
            if !path.exists() {
                return Err(format!("{} gibt es nicht", path.display()));
            }
            // a running window loads it in place (asks the user about unsaved changes)
            if live::status().is_some() {
                let v = live::request("open", json!({"path": path}))?;
                let mut out = json!({"ok": true, "opened": path, "window": "vorhanden"});
                if v["queued"].as_bool() == Some(true) {
                    out["queued"] = json!(true);
                }
                return Ok(out);
            }
            std::process::Command::new(&gui).arg(&path).spawn().map_err(|e| e.to_string())?;
            Ok(json!({"ok": true, "opened": path, "window": "neu gestartet"}))
        }
        Cmd::Status => Ok(match live::status() {
            Some(s) => json!({"ok": true, "running": true, "project_name": s["name"], "path": s["path"], "unsaved_changes": s["dirty"], "pending": s["pending"]}),
            None => json!({"ok": true, "running": false}),
        }),
        Cmd::Save { as_path } => {
            let mut body = json!({});
            if let Some(p) = as_path {
                let mut p = abs(&p);
                if p.extension().is_none() {
                    p.set_extension("tsp");
                }
                body["path"] = json!(p);
            }
            live::request("save", body)
        }
    }
}

pub(crate) fn default_download_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.audio_dir().map(|d| d.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
        .join("TStudio Samples")
        .join("Downloads")
}
