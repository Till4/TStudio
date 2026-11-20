//! Executes the cutter's queued actions: playback, saving, sending to rack/playlist.

use super::*;

pub fn cut_dir(app: &TStudio) -> PathBuf {
    app.settings.output_dir().join("Cuts")
}

/// Makes sure saved cuts show up in the library.
pub(super) fn ensure_in_library(app: &mut TStudio) {
    let dir = cut_dir(app);
    if !app.settings.library_roots.iter().any(|r| dir.starts_with(r)) {
        let base = app.settings.output_dir();
        app.settings.library_roots.push(base);
        app.settings.save(&app.paths.settings());
    }
    app.library.rescan(&app.settings);
}

/// Folder and file stem for a cut: `<base>/<Kategorie>/<name> [<label>] <Kategorie>`.
pub(super) fn save_target(base: &Path, name: &str, cat: Option<usize>, label: Option<&str>) -> (PathBuf, String) {
    let name = match label {
        Some(l) => format!("{name} {l}"),
        None => name.to_string(),
    };
    match cat {
        Some(c) => (base.join(CATEGORIES[c]), format!("{name} {}", CATEGORIES[c])),
        None => (base.to_path_buf(), name),
    }
}

pub(super) fn save_range(app: &TStudio, a: usize, b: usize, cat: Option<usize>) -> Result<PathBuf, String> {
    save_range_as(app, a, b, cat, None)
}

/// Like [`save_range`]; `label` (e.g. a marker name) becomes part of the file name.
pub(super) fn save_range_as(app: &TStudio, a: usize, b: usize, cat: Option<usize>, label: Option<&str>) -> Result<PathBuf, String> {
    let t = &app.fetch.trim;
    let d = render_range(t, a, b).ok_or("Leere Auswahl")?;
    let (dir, stem) = save_target(&cut_dir(app), &t.base_name(), cat, label);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = numbered_path(&dir, &stem, "wav");
    write_wav(&path, &d, WavFormat::Pcm24)?;
    Ok(path)
}

pub(super) fn run_actions(app: &mut TStudio, acts: Vec<Act>, now: f64) {
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
            Act::SaveMarkers => {
                let t = &app.fetch.trim;
                let cat = Some(t.category);
                let jobs: Vec<(String, (usize, usize))> = t.markers.iter().map(|m| (marker_label(&m.name), m.sel)).collect();
                if jobs.is_empty() {
                    app.set_status("Keine Marker gesetzt (M merkt die Auswahl)");
                    continue;
                }
                let mut n = 0;
                for (label, (a, b)) in jobs {
                    match save_range_as(app, a, b, cat, Some(&label)) {
                        Ok(p) => {
                            app.fetch.trim.saved.insert(0, p);
                            n += 1;
                        }
                        Err(e) => {
                            app.set_error(format!("Speichern fehlgeschlagen ({label}): {e}"));
                            break;
                        }
                    }
                }
                if n > 0 {
                    app.set_status(format!("✂ {n} Marker als {} gespeichert", CATEGORIES[app.fetch.trim.category]));
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
