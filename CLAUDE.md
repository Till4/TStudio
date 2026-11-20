# TStudio – Code-Landkarte

Rust-Workspace (DAW mit egui + cpal). Zum Ändern zuerst die passende Datei hier suchen und **nur diese** lesen –
Dateien sind bewusst klein gehalten (Richtwert: < 600 Zeilen, danach in `<name>/`-Untermodule teilen).

## Crates

| Crate | Aufgabe |
|---|---|
| `crates/dsp` | Reine Audio-DSP (kein I/O): Synth, Effekte, Filter, Stretch, Analyse |
| `crates/project` | Projektmodell (`.tsp`), Tracks, Automation, Undo-History |
| `crates/engine` | Audio-Thread: Snapshot → Sequencer → Mixer, SoundFont, Offline-Render |
| `crates/library` | Sample-Bibliothek, Decoder, WAV schreiben |
| `crates/fetch` | Download (yt-dlp), Stems, Video-Render (ffmpeg) |
| `crates/app` | GUI (egui) |
| `crates/cli` | `tstudio-cli` – headless Bearbeiten/Rendern/Analysieren, JSON-Ausgabe |

## Wo liegt was

**dsp**
- `synth/` – TSynth: `params.rs` (SynthParams), `osc_params.rs`, `kick.rs`, `lfo.rs`, `modmatrix.rs`, `voice.rs` (Voice-Rendering), `mod.rs` (`Synth`), `tests.rs`
- `fx/` – `params.rs` (EffectKind/-Params), `processors.rs` (Effect-DSP), `tests.rs`; weitere Effekte in `fx_extra.rs`
- `synth_presets.rs` (Presets), `filter.rs`, `osc.rs`, `wavetable.rs`, `stretch.rs`, `meter.rs`, `isolate.rs`, `autotune.rs`

**project**
- `lib.rs` (Project, Kanäle, Clips), `automation.rs`, `tracks.rs`, `history.rs`, `tempo.rs` (Tempo-Spur: TempoPoint, TempoMap tick↔Sekunden), `meter.rs` (Taktartwechsel: MeterChange, BarMap Takt↔Tick, Metronom-Beats)

**engine**
- `core.rs` – `EngineCore` (process/process_chunk); `core/shared.rs` (Command, SharedState, PlayMode),
  `core/sequencer.rs` (Events, Clips, Swing, Latenz, Tempo-Map), `core/tests.rs`, `core/tempo_tests.rs`
- `snapshot.rs`, `compile.rs` (Projekt → Snapshot), `audio.rs` (cpal), `render.rs` (Offline), `soundfont.rs`, `input.rs`

**app** (`crates/app/src`)
- `app.rs` – `TStudio`-Struct, `View`, `update`-Schleife; `app/playback.rs` (Transport/Engine-Kommandos),
  `app/project_io.rs` (Neu/Öffnen/Speichern/Undo, Kanäle & Clips hinzufügen), `app/chrome.rs` (Shortcuts, Menü, Transportleiste, Statusleiste)
- `live.rs` – Fenster-Steuerung durch `tstudio-cli live`
- `ui/` – ein Modul pro Ansicht:
  `playlist.rs` (+ `playlist/menus|automation|properties|tempo.rs`), `cutter.rs` (+ `cutter/stems|panels|tests.rs`),
  `tsynth.rs` (+ `tsynth/helpers|pages|layout_tests.rs`), `piano_roll.rs`, `mixer.rs`, `rack.rs`, `instrument.rs`,
  `library.rs`, `export.rs`, `fetch.rs`, `record.rs`, `settings.rs`, `widgets.rs` (gemeinsame Widgets/Farben)

**cli** (`crates/cli/src`)
- `main.rs` (Einstieg, JSON-Ausgabe), `args.rs` (alle clap-Definitionen; `args/timeline.rs`: tempo/meter), `helpers.rs` (Laden/Speichern, Selektoren, JSON-Pfade, `info`)
- `cmd/mod.rs` – `run`-Dispatch; pro Bereich `cmd/channel|pattern|notes|clip|fx|mixer|track|auto|marker|section|tempo|meter.rs`
- `audio.rs` (Analyse/Render), `tools.rs` (Akkorde, Sections, Hilfen), `spec.rs`, `live.rs`

## Konventionen beim Splitten

- Untermodule: `use super::*;` oben; vom Parent gebrauchte Items als `pub(super)`; bei `pub`-API im Parent `pub use child::*;`.
- Inline-Tests liegen in `<modul>/tests.rs` (`#[cfg(test)] mod tests;`).
- Öffentliche Pfade (z. B. `tstudio_dsp::synth::SynthParams`, `tstudio_dsp::fx::*`) dürfen sich beim Splitten nicht ändern.

## Prüfen

```bash
cargo check --workspace --all-targets
```

```bash
cargo test --workspace
```

Neue Warnungen vermeiden (Stand: 62 Alt-Warnungen in `crates/app`, v. a. `f32`-Fallback und toter Code).
Laufendes `tstudio.exe` nicht beenden; zum Testen `cargo check`/`cargo test` und `tstudio-cli` nutzen.
