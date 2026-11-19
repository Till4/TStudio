//! tstudio-cli – headless control of TStudio projects (.tsp).
//!
//! Every command prints JSON on stdout (errors: `{"ok":false,"error":…}` + exit code 1),
//! so scripts and AI assistants can drive the DAW. Project argument `live` (or a path to the
//! file open in TStudio) edits the project open in the running TStudio window directly.

mod audio;
mod live;
mod spec;
mod tools;

use clap::{Args, Parser, Subcommand};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tstudio_dsp::fx::EffectKind;
use tstudio_dsp::stretch::StretchMode;
use tstudio_dsp::synth::{ModDest, ModSlot, ModSource, SynthParams};
use tstudio_dsp::{note_name, SampleData};
use tstudio_engine::{soundfont_presets, PlayMode, Sf2Preset};
use tstudio_project::automation::field_range;
use tstudio_project::{
    AudioClip, AutoPoint, AutomationClip, ClipKind, FxSlot, Id, Instrument, MixerSend, Note, Project, SamplerInstrument, SoundFontInstrument, StretchSettings, MAX_FX_SLOTS, PPQ,
};

#[derive(Parser)]
#[command(
    name = "tstudio-cli",
    version,
    about = "TStudio ohne GUI: Projekte bearbeiten, rendern, Audio analysieren. Ausgabe immer JSON.",
    after_help = "PROJECT: Pfad zu einer .tsp-Datei – oder `live` für das Projekt, das gerade in TStudio offen ist
(inkl. ungespeicherter Änderungen; jede Änderung ist dort mit Strg+Z rückgängig zu machen)."
)]
struct Cli {
    /// Nie ins offene TStudio-Fenster schreiben: ist die Datei dort offen, bricht der Befehl ab
    #[arg(long, global = true)]
    no_live: bool,
    /// JSON eingerückt ausgeben (Standard: kompakt, eine Zeile – spart Platz/Token)
    #[arg(long, global = true)]
    pretty: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Neues Projekt anlegen (mit TSynth-Kanal und Pattern 1)
    New {
        project: PathBuf,
        #[arg(long, default_value_t = 140.0)]
        bpm: f64,
        #[arg(long)]
        name: Option<String>,
        /// ohne Standard-Kanal
        #[arg(long)]
        empty: bool,
        /// vorhandene Datei überschreiben
        #[arg(long)]
        force: bool,
    },
    /// Übersicht: Kanäle, Patterns, Clips, Mixer (Indizes für andere Befehle)
    Info { project: PathBuf },
    /// Beliebigen Wert als JSON lesen, z.B. `channels.0.instrument` (ohne Pfad: alles)
    Get { project: PathBuf, path: Option<String> },
    /// Beliebige Werte setzen: `pfad=json` …, z.B. `bpm=128` `channels.0.instrument.Synth.cutoff=900`
    Set {
        project: PathBuf,
        #[arg(required = true)]
        assignments: Vec<String>,
    },
    /// Kanäle (Instrumente) verwalten
    #[command(subcommand)]
    Channel(ChannelCmd),
    /// Patterns verwalten
    #[command(subcommand)]
    Pattern(PatternCmd),
    /// Noten in Patterns schreiben/lesen
    #[command(subcommand)]
    Notes(NotesCmd),
    /// Playlist-Clips
    #[command(subcommand)]
    Clip(ClipCmd),
    /// Playlist-Spuren: auflisten, anlegen, einfügen, löschen, verschieben, umbenennen, Mute/Solo (1-basiert)
    #[command(subcommand)]
    Track(TrackCmd),
    /// Mixer: Inserts anlegen/löschen/verschieben/umbenennen, Mute/Solo, Sends und Bus-Ausgang
    #[command(subcommand)]
    Mixer(MixerCmd),
    /// Automation-Clips (Filter-Sweeps, Lautstärke-Fahrten, … im Song-Modus)
    #[command(subcommand)]
    Auto(AutoCmd),
    /// Presets einer SoundFont (.sf2) auflisten
    Soundfont { file: PathBuf },
    /// Effekte im Mixer
    #[command(subcommand)]
    Fx(FxCmd),
    /// Song (oder ein Pattern) offline als WAV/MP3 rendern + Analyse ausgeben
    Render {
        project: PathBuf,
        out: PathBuf,
        /// nur dieses Pattern rendern (Index oder Name) statt der Playlist
        #[arg(long, short)]
        pattern: Option<String>,
        /// Pattern so oft wiederholen
        #[arg(long, default_value_t = 1)]
        loops: u32,
        #[arg(long, default_value_t = 48000)]
        sr: u32,
        /// WAV-Bittiefe: 16, 24 oder 32 (float)
        #[arg(long, default_value_t = 24)]
        bits: u32,
        #[arg(long, default_value_t = 320)]
        kbps: u32,
        /// Ausklingzeit in Sekunden
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
        #[arg(long)]
        normalize: bool,
        /// Video (.mp4): DJ-/Künstlername im Visualizer (Standard: dj_name aus den TStudio-Einstellungen)
        #[arg(long)]
        artist: Option<String>,
        /// Video: Tracktitel (Standard: Projektname)
        #[arg(long)]
        title: Option<String>,
        /// Video: Akzentfarbe (orange, rot, pink, lila, cyan, grün oder #RRGGBB)
        #[arg(long)]
        accent: Option<String>,
        /// Video: Größe (1080p, 720p, vertical = 1080×1920, square)
        #[arg(long)]
        size: Option<String>,
        #[arg(long, default_value_t = 30)]
        fps: u32,
        /// Video: kein „made with TStudio“-Logo
        #[arg(long)]
        no_watermark: bool,
        /// Statt Video nur ein Standbild bei dieser Sekunde als PNG (out = .png)
        #[arg(long)]
        still: Option<f32>,
        /// zusätzlich jede Mixer-Spur einzeln als WAV in diesen Ordner (ohne Master-Effekte)
        #[arg(long)]
        stems: Option<PathBuf>,
    },
    /// Audiodatei analysieren: Länge, Pegel, Clipping, BPM, Tonart, Onsets
    Analyze {
        file: PathBuf,
        /// bekanntes/ungefähres Tempo (Lautheit pro Takt; Beat-Grid sucht ±4 % darum)
        #[arg(long)]
        bpm: Option<f64>,
        /// genaues Beat-Grid: Tempo auf 0,01 BPM, Beat-Phase, Downbeat („die 1“), Takt-Startzeiten
        #[arg(long)]
        grid: bool,
        /// Akkord-Zeitleiste (Grundton/Dur/Moll je Abschnitt, ab dem Downbeat)
        #[arg(long)]
        chords: bool,
        /// Beats pro Akkord-Abschnitt (Standard: 1 Takt)
        #[arg(long)]
        chord_beats: Option<f64>,
        /// Mix-Report: LUFS, True Peak, Frequenzbänder, Stereo/Mono-Bass, Warnungen
        #[arg(long)]
        mix: bool,
        #[arg(long, default_value_t = 4)]
        beats_per_bar: u32,
    },
    /// Eigenen Mixdown mit einem Referenz-Track vergleichen (Lautheit, Bänder, Dichte, Breite)
    Compare { mine: PathBuf, reference: PathBuf },
    /// Mix-Report eines Projekts: Master (LUFS, Bänder, Warnungen) + jede Mixer-Spur einzeln + Maskierung
    Mix {
        project: PathBuf,
        /// zusätzlich mit diesem Referenz-Track vergleichen
        #[arg(long)]
        reference: Option<PathBuf>,
        /// nur den Master analysieren (schneller, ohne Einzelspuren)
        #[arg(long)]
        master_only: bool,
        #[arg(long, default_value_t = 48000)]
        sr: u32,
    },
    /// Stems trennen (Demucs auf der CPU, niedrige Priorität): Drums, Bass, Vocals, Rest
    Stems {
        input: PathBuf,
        /// Zielordner (Standard: neben der Datei, Unterordner „<Name> Stems“)
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        threads: u32,
    },
    /// One-Shot (Kick, 808) auf einen Ton stimmen: Grundton erkennen und verschieben
    Tune {
        input: PathBuf,
        output: PathBuf,
        /// Zielton, z.B. F, F#, G1 (ohne Oktave: nächstgelegene)
        #[arg(long)]
        to: String,
        /// resample (wie Platte, Länge ändert sich minimal – klingt bei Kicks am natürlichsten) oder pitch
        #[arg(long, default_value = "resample")]
        mode: String,
    },
    /// Song-Marker (Intro, Break, Drop …) – überall statt Taktnummern nutzbar
    #[command(subcommand)]
    Marker(MarkerCmd),
    /// Song-Abschnitte: Takte kopieren, leeren, einfügen, löschen (Takt oder Markername)
    #[command(subcommand)]
    Section(SectionCmd),
    /// Tempo einer Datei ändern (Stretch = Tonhöhe bleibt, Resample = wie Platte)
    Stretch {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        from: f64,
        #[arg(long)]
        to: f64,
        #[arg(long, default_value = "stretch")]
        mode: String,
        /// zusätzliche Tonhöhe in Halbtönen (jeder Modus)
        #[arg(long, default_value_t = 0.0)]
        pitch: f32,
        /// Formanten halten (Stimmen klingen beim Pitchen natürlich)
        #[arg(long)]
        formant: bool,
    },
    /// Ausschnitt speichern (Sekunden), optional mit Fades/Normalisieren
    Trim {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 0.0)]
        start: f64,
        #[arg(long)]
        end: Option<f64>,
        #[arg(long, default_value_t = 3.0)]
        fade_in_ms: f32,
        #[arg(long, default_value_t = 10.0)]
        fade_out_ms: f32,
        #[arg(long)]
        normalize: bool,
        /// Stille (unter -60 dB) am Ende abschneiden
        #[arg(long)]
        trim_tail: bool,
        #[arg(long, default_value_t = 24)]
        bits: u32,
        #[arg(long, default_value_t = 320)]
        kbps: u32,
        #[command(flatten)]
        iso: IsoArgs,
        /// Schnitt im Beat-Raster: so viele Beats (statt --start/--end)
        #[arg(long)]
        beats: Option<f64>,
        /// Schnitt im Beat-Raster: so viele Takte
        #[arg(long)]
        bars: Option<f64>,
        /// ab diesem Beat (0 = erster Downbeat; negativ = davor)
        #[arg(long)]
        from_beat: Option<f64>,
        /// ab diesem Takt (1 = erster Downbeat)
        #[arg(long)]
        from_bar: Option<f64>,
        /// Tempo der Quelle (sonst erkannt; mit --downbeat wird nichts erkannt)
        #[arg(long)]
        bpm: Option<f64>,
        /// Zeit (s) eines Downbeats der Quelle (sonst erkannt)
        #[arg(long)]
        downbeat: Option<f64>,
        #[arg(long, default_value_t = 4)]
        beats_per_bar: u32,
        /// Ergebnis direkt auf dieses Tempo bringen
        #[arg(long)]
        to_bpm: Option<f64>,
        /// Tempo-Modus für --to-bpm: stretch | resample
        #[arg(long, default_value = "stretch")]
        mode: String,
        /// Tonhöhe in Halbtönen
        #[arg(long, default_value_t = 0.0)]
        pitch: f32,
        /// Formanten halten beim Pitchen
        #[arg(long)]
        formant: bool,
    },
    /// Slice-Punkte finden (Transienten oder gleichmäßig), optional als Dateien schreiben
    Chop {
        input: PathBuf,
        #[arg(long, default_value_t = 0.5)]
        sensitivity: f32,
        /// statt Transienten in N gleiche Teile
        #[arg(long)]
        even: Option<usize>,
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Audiodateien in Ordnern auflisten
    Scan {
        #[arg(required = true)]
        dirs: Vec<PathBuf>,
        /// nur Namen, die diesen Text enthalten
        #[arg(long)]
        filter: Option<String>,
        /// BPM/Tonart/Länge analysieren (langsamer)
        #[arg(long)]
        analyze: bool,
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Audio einer URL herunterladen (yt-dlp + ffmpeg)
    Fetch {
        url: String,
        #[arg(long, default_value = "wav")]
        format: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Status von ffmpeg / yt-dlp
    Tools,
    /// Synth-Presets, Effekt-Typen (mit Standard-Parametern) und Stretch-Modi
    Presets,
    /// Projekt in TStudio öffnen (im laufenden Fenster, sonst neues Fenster)
    Open { project: PathBuf },
    /// Läuft TStudio? Welches Projekt ist offen, gibt es ungespeicherte Änderungen?
    Status,
    /// Das in TStudio offene Projekt speichern (wie Strg+S)
    Save {
        /// unter neuem Namen speichern (nötig, wenn das Projekt noch keine Datei hat)
        #[arg(long = "as")]
        as_path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ChannelCmd {
    /// Kanal hinzufügen: `--synth [--preset Bass]`, `--sample datei.wav` oder `--soundfont datei.sf2 [--preset NAME | --program N --bank B]`
    Add {
        project: PathBuf,
        #[arg(long, conflicts_with_all = ["sample", "soundfont"])]
        synth: bool,
        /// Synth-Preset bzw. SoundFont-Preset (Name, Teil davon oder BANK:PROGRAMM)
        #[arg(long)]
        preset: Option<String>,
        #[arg(long, conflicts_with = "soundfont")]
        sample: Option<PathBuf>,
        #[arg(long)]
        soundfont: Option<PathBuf>,
        /// SoundFont: Programm 0..127
        #[arg(long)]
        program: Option<u8>,
        /// SoundFont: Bank (128 = Drums)
        #[arg(long)]
        bank: Option<u16>,
        #[arg(long)]
        name: Option<String>,
        /// Grundton des Samplers (Default C4)
        #[arg(long)]
        root: Option<String>,
        #[command(flatten)]
        stretch: StretchArgs,
    },
    /// Kanal entfernen (Index oder Name)
    Remove { project: PathBuf, channel: String },
    /// Preset laden: TSynth-Preset bzw. SoundFont-Preset (Name, Teil davon oder BANK:PROGRAMM)
    Preset { project: PathBuf, channel: String, preset: String },
    /// TSynth-Mod-Matrix: Routing hinzufügen (`--source Lfo1 --dest Cutoff1 --amount 0.4`), `--clear` leert sie; ohne Optionen nur anzeigen
    Mod {
        project: PathBuf,
        channel: String,
        /// Quelle, z.B. AmpEnv, FilterEnv, ModEnv, Lfo1..Lfo4, Velocity, Note, Random, Macro1..Macro4, KickSweep
        #[arg(long)]
        source: Option<String>,
        /// Ziel, z.B. Pitch, Osc1Pos, Osc2Warp, Cutoff1, Reso1, Cutoff2, Amp, Pan, KickPitch, KickDrive, FxDistDrive …
        #[arg(long)]
        dest: Option<String>,
        /// -1..1
        #[arg(long, default_value_t = 0.5, allow_hyphen_values = true)]
        amount: f32,
        /// vorher alle Routings löschen
        #[arg(long)]
        clear: bool,
    },
    /// TSynth-Kick-Engine einschalten und den Grundton stimmen (`--tune F1`, Zahl = MIDI-Note)
    Kick {
        project: PathBuf,
        channel: String,
        #[arg(long)]
        tune: Option<String>,
        /// Kick-Engine ausschalten
        #[arg(long)]
        off: bool,
    },
}

#[derive(Subcommand)]
enum MixerCmd {
    /// Rumble-Bus anlegen: neue Spur, Send von der Kick, Kette Reverb → Verzerrung → Tiefpass → Sidechain
    Rumble {
        project: PathBuf,
        /// Mixer-Spur der Kick
        #[arg(long)]
        kick: usize,
        /// Send-Menge 0..1
        #[arg(long, default_value_t = 0.7)]
        amount: f32,
        #[arg(long, default_value = "Rumble")]
        name: String,
    },
    /// Post-Fader-Send von Spur FROM auf eine höhere Spur TO (amount 0 entfernt den Send)
    Send {
        project: PathBuf,
        #[arg(long)]
        from: usize,
        #[arg(long)]
        to: usize,
        #[arg(long, default_value_t = 0.5)]
        amount: f32,
    },
    /// Ausgang einer Spur auf einen Bus (höhere Spur) legen; --to 0 = Master
    Route {
        project: PathBuf,
        #[arg(long)]
        from: usize,
        #[arg(long)]
        to: usize,
    },
    /// Neuen Insert anlegen (am Ende oder mit --at an Position N; höhere Inserts rücken nach, Verweise bleiben gültig)
    Add {
        project: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        at: Option<usize>,
    },
    /// Insert löschen: Kanäle/Audio-Clips darauf gehen auf den Master, Sends dorthin und Automation darauf werden entfernt
    Delete { project: PathBuf, track: usize },
    /// Insert an eine andere Position verschieben (Kanal-Routing, Sends, Sidechain, Automation wandern mit)
    Move {
        project: PathBuf,
        #[arg(long)]
        from: usize,
        #[arg(long)]
        to: usize,
    },
    /// Insert umbenennen
    Rename { project: PathBuf, track: usize, name: String },
    /// Insert stumm schalten (--off = wieder an)
    Mute {
        project: PathBuf,
        track: usize,
        #[arg(long)]
        off: bool,
    },
    /// Insert solo (--off = aus, --only = alle anderen Solos löschen, --clear = alle Solos aus; track dann egal)
    Solo {
        project: PathBuf,
        #[arg(default_value_t = 0)]
        track: usize,
        #[arg(long)]
        off: bool,
        #[arg(long)]
        only: bool,
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Subcommand)]
enum TrackCmd {
    /// Alle Playlist-Spuren mit Mute/Solo und Clip-Anzahl
    List { project: PathBuf },
    /// Leere Spur anlegen (am Ende oder mit --at vor Spur N einfügen; Clips darunter rutschen mit)
    Add {
        project: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        at: Option<usize>,
    },
    /// Spur samt ihren Clips löschen
    Delete { project: PathBuf, track: usize },
    /// Spur (mit Clips) an Position TO verschieben
    Move {
        project: PathBuf,
        #[arg(long)]
        from: usize,
        #[arg(long)]
        to: usize,
    },
    /// Spur samt Clips direkt darunter kopieren
    Duplicate { project: PathBuf, track: usize },
    /// Spur umbenennen
    Rename { project: PathBuf, track: usize, name: String },
    /// Spur stumm schalten (--off = wieder an)
    Mute {
        project: PathBuf,
        track: usize,
        #[arg(long)]
        off: bool,
    },
    /// Spur-Lautstärke in Prozent (100 = unverändert, max 125); wirkt auf Samples und Pattern-Noten der Spur
    Volume { project: PathBuf, track: usize, percent: f32 },
    /// Spur solo (--off = aus, --only = alle anderen Solos löschen, --clear = alle Solos aus)
    Solo {
        project: PathBuf,
        #[arg(default_value_t = 0)]
        track: usize,
        #[arg(long)]
        off: bool,
        #[arg(long)]
        only: bool,
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Subcommand)]
enum AutoCmd {
    /// Automation-Clip anlegen. Ziel z.B. `mixer.3.fx.0.cutoff`, `mixer.0.volume`, `channel.2.synth.cutoff`.
    /// Kurve: `--from A --to B` (Rampe über die Länge) oder `--points "0:200,4:8000,8:300"` (Beat:Wert)
    Add {
        project: PathBuf,
        #[arg(long)]
        target: String,
        /// Playlist-Spur (1-basiert)
        #[arg(long, default_value_t = 1)]
        track: usize,
        #[arg(long, default_value_t = 1.0)]
        bar: f64,
        #[arg(long, default_value_t = 4.0)]
        bars: f64,
        #[arg(long, allow_hyphen_values = true)]
        from: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        to: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        points: Option<String>,
        #[arg(long, default_value_t = 1)]
        repeat: u32,
    },
    /// Alle automatisierbaren Ziele des Projekts (mit Wertebereich)
    Targets {
        project: PathBuf,
        /// nur Ziele, die diesen Text enthalten
        #[arg(long)]
        filter: Option<String>,
    },
}

/// Isolieren per Zero-Phase-EQ und Mitte/Seite (wie im Sample-Cutter der GUI)
#[derive(Args, Clone, Default)]
struct IsoArgs {
    /// Preset: "Kick / Sub", "Bass", "Snare / Clap", "Hi-Hats / Becken", "Vocals (Mitte)", "Synths / Mitten", "Vocal entfernen", "Telefon"
    #[arg(long)]
    preset: Option<String>,
    /// Hochpass (Low Cut) in Hz
    #[arg(long)]
    hp: Option<f32>,
    /// Tiefpass (High Cut) in Hz
    #[arg(long)]
    lp: Option<f32>,
    /// Steilheit in dB/Okt: 24, 48 oder 96
    #[arg(long)]
    slope: Option<u32>,
    /// Band-EQ "FREQ:DB[:Q]", z.B. "2500:4:1.2"
    #[arg(long)]
    band: Option<String>,
    /// stereo | mid | side | left | right
    #[arg(long)]
    stereo: Option<String>,
    #[arg(long)]
    gain_db: Option<f32>,
}

impl IsoArgs {
    fn params(&self) -> Result<Option<tstudio_dsp::isolate::IsolateParams>, String> {
        use tstudio_dsp::isolate::{IsolateParams, StereoMode};
        let mut p = match &self.preset {
            Some(n) => {
                let all = IsolateParams::presets();
                let key = n.to_lowercase();
                all.iter()
                    .find(|(name, _)| name.to_lowercase() == key)
                    .or_else(|| all.iter().find(|(name, _)| name.to_lowercase().contains(&key)))
                    .map(|(_, p)| *p)
                    .ok_or_else(|| format!("Isolier-Preset '{n}' unbekannt (verfügbar: {})", all.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")))?
            }
            None => IsolateParams::default(),
        };
        if let Some(v) = self.hp {
            p.hp_hz = v.max(0.0);
        }
        if let Some(v) = self.lp {
            p.lp_hz = v.max(0.0);
        }
        if let Some(v) = self.slope {
            p.slope = match v {
                24 => 1,
                48 => 2,
                96 => 4,
                _ => return Err("--slope: 24, 48 oder 96".into()),
            };
        }
        if let Some(b) = &self.band {
            let parts: Vec<f32> = b.split(':').map(|x| x.trim().parse::<f32>().map_err(|_| format!("--band '{b}': FREQ:DB[:Q]"))).collect::<Result<_, _>>()?;
            if parts.len() < 2 {
                return Err(format!("--band '{b}': FREQ:DB[:Q]"));
            }
            p.peak_hz = parts[0];
            p.peak_db = parts[1].clamp(-24.0, 24.0);
            if let Some(q) = parts.get(2) {
                p.peak_q = q.clamp(0.1, 20.0);
            }
        }
        if let Some(m) = &self.stereo {
            p.stereo = StereoMode::parse(m).ok_or_else(|| format!("--stereo '{m}': stereo | mid | side | left | right"))?;
        }
        if let Some(g) = self.gain_db {
            p.gain_db = g;
        }
        Ok(if p.is_neutral() { None } else { Some(p) })
    }
}

#[derive(Args)]
struct StretchArgs {
    /// Tempo anpassen: stretch | resample | off
    #[arg(long)]
    stretch: Option<String>,
    /// Original-BPM des Samples (Default: aus Dateiname/Erkennung)
    #[arg(long)]
    sample_bpm: Option<f64>,
    /// Tonhöhe in Halbtönen (jeder Modus, Länge bleibt)
    #[arg(long)]
    pitch: Option<f32>,
    /// Formanten halten beim Pitchen
    #[arg(long)]
    formant: bool,
}

#[derive(Subcommand)]
enum PatternCmd {
    /// Pattern anlegen
    Add {
        project: PathBuf,
        #[arg(long)]
        name: Option<String>,
        /// feste Länge in Takten (sonst automatisch)
        #[arg(long)]
        bars: Option<f64>,
    },
    /// Pattern entfernen (und seine Clips)
    Remove { project: PathBuf, pattern: String },
}

#[derive(Args)]
struct Target {
    project: PathBuf,
    /// Pattern (Index oder Name)
    #[arg(long, short)]
    pattern: String,
    /// Kanal (Index oder Name)
    #[arg(long, short)]
    channel: String,
}

#[derive(Subcommand)]
enum NotesCmd {
    /// Noten `KEY[|KEY]@START[+LEN][*VEL]` (Beats), z.B. `C4@0+1` `C4|E4|G4@2+2*0.6`
    Add {
        #[command(flatten)]
        t: Target,
        #[arg(required = true)]
        notes: Vec<String>,
        /// Standardlänge in Beats
        #[arg(long, default_value = "1/4")]
        len: String,
        #[arg(long, default_value_t = 0.8)]
        vel: f32,
        /// Versatz in Beats, der auf alle Starts addiert wird
        #[arg(long, default_value = "0")]
        offset: String,
        /// vorher alle Noten dieses Kanals im Pattern löschen
        #[arg(long)]
        replace: bool,
    },
    /// Step-Reihe (16tel): `x...x...x...x...` (x=0.8 X=1.0 o=0.45 .=Pause)
    Steps {
        #[command(flatten)]
        t: Target,
        steps: String,
        /// Tonhöhe (Default: Grundton des Samplers bzw. C4)
        #[arg(long)]
        key: Option<String>,
        /// Notenlänge in 16teln
        #[arg(long, default_value_t = 1)]
        len: i64,
        /// erst ab diesem Beat
        #[arg(long, default_value = "0")]
        offset: String,
        /// vorhandene Noten des Kanals behalten (sonst ersetzt)
        #[arg(long)]
        keep: bool,
    },
    /// Noten aus einer Akkordfolge erzeugen, z.B. "Cm Ab Eb Bb" (Länge je Akkord: Cm:2)
    /// Stile: block/pad (gehalten), stab (Offbeat-Akkorde), arp, bass (Viertel), offbeat (Tekk-Bass), rolling
    Chords {
        #[command(flatten)]
        t: Target,
        progression: String,
        #[arg(long, default_value_t = 1.0)]
        bars_per_chord: f64,
        #[arg(long, default_value = "block")]
        style: String,
        /// Oktave des Grundtons (Standard: 2 für Bass-Stile, sonst 4)
        #[arg(long)]
        octave: Option<i32>,
        #[arg(long, default_value_t = 0.8)]
        vel: f32,
        /// eigener 16tel-Rhythmus je Takt: x=Schlag X=betont o=leise .=Pause -=halten
        #[arg(long)]
        rhythm: Option<String>,
        #[arg(long, default_value = "0")]
        offset: String,
        /// vorher alle Noten dieses Kanals im Pattern löschen
        #[arg(long)]
        replace: bool,
        /// keine Umkehrungen (Akkorde immer in Grundstellung)
        #[arg(long)]
        no_voicing: bool,
    },
    /// Noten löschen (ganzes Pattern oder ein Kanal)
    Clear {
        project: PathBuf,
        #[arg(long, short)]
        pattern: String,
        #[arg(long, short)]
        channel: Option<String>,
    },
    /// Noten auflisten
    List {
        project: PathBuf,
        #[arg(long, short)]
        pattern: String,
        #[arg(long, short)]
        channel: Option<String>,
    },
}

#[derive(Subcommand)]
enum ClipCmd {
    /// Clip einfügen: `--pattern P` oder `--audio datei`; Position `--bar` (1 = Songanfang)
    Add {
        project: PathBuf,
        #[arg(long, default_value_t = 1)]
        track: usize,
        #[arg(long, default_value_t = 1.0)]
        bar: f64,
        #[arg(long, conflicts_with = "audio")]
        pattern: Option<String>,
        #[arg(long)]
        audio: Option<PathBuf>,
        /// Länge in Takten (Default: Pattern-/Sample-Länge)
        #[arg(long)]
        bars: Option<f64>,
        /// so oft hintereinander einfügen
        #[arg(long, default_value_t = 1)]
        repeat: u32,
        #[arg(long, default_value_t = 1.0)]
        gain: f32,
        #[command(flatten)]
        stretch: StretchArgs,
    },
    /// Clips entfernen (IDs aus `info`) oder `--all`
    Remove {
        project: PathBuf,
        ids: Vec<Id>,
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum FxCmd {
    /// Effekt auf Mixer-Spur legen (0 = Master): eq, compressor, reverb, delay, distortion, chorus, filter, limiter, sidechain, autotune
    /// (sidechain: `--source N` = Spur, deren Signal duckt, z.B. die Kick)
    Add {
        project: PathBuf,
        #[arg(long)]
        track: usize,
        kind: String,
        /// nur für sidechain: Mixer-Spur des Auslösers (Kick)
        #[arg(long)]
        source: Option<usize>,
    },
    /// Effekt-Slot entfernen
    Remove {
        project: PathBuf,
        #[arg(long)]
        track: usize,
        slot: usize,
    },
    /// Fertige Effektkette auf eine Spur: schranz-kick, rumble, master, ott, vocal, bass
    Chain {
        project: PathBuf,
        #[arg(long)]
        track: usize,
        name: String,
        /// Sidechain-Auslöser (Kick-Spur) für rumble/bass
        #[arg(long)]
        source: Option<usize>,
        /// vorhandene Effekte der Spur vorher entfernen
        #[arg(long)]
        replace: bool,
    },
}

#[derive(Subcommand)]
enum MarkerCmd {
    /// Marker setzen (--bar 1-basiert), ein gleichnamiger wird verschoben
    Add {
        project: PathBuf,
        #[arg(long)]
        bar: f64,
        name: String,
    },
    List { project: PathBuf },
    Remove { project: PathBuf, name: String },
    Clear { project: PathBuf },
}

#[derive(Args)]
struct Range {
    project: PathBuf,
    /// Starttakt oder Markername
    #[arg(long)]
    from: String,
    /// Länge in Takten (Standard: bis zum nächsten Marker)
    #[arg(long)]
    bars: Option<f64>,
    /// nur diese Playlist-Spuren (1-basiert, z.B. 1,3,4)
    #[arg(long)]
    tracks: Option<String>,
}

#[derive(Subcommand)]
enum SectionCmd {
    /// Clips eines Bereichs an eine andere Stelle kopieren (--to Takt oder Marker)
    Copy {
        #[command(flatten)]
        r: Range,
        #[arg(long)]
        to: String,
    },
    /// Clips eines Bereichs entfernen (Lücke bleibt)
    Clear {
        #[command(flatten)]
        r: Range,
    },
    /// Leere Takte einfügen: alles ab --at rückt nach hinten
    Insert {
        project: PathBuf,
        #[arg(long)]
        at: String,
        #[arg(long)]
        bars: f64,
    },
    /// Bereich herausschneiden: Clips weg, alles danach rückt nach vorn
    Delete {
        #[command(flatten)]
        r: Range,
    },
}

// ------------------------------------------------------------------ helpers

type R<T> = Result<T, String>;

/// `dj_name` from TStudio's settings.ron (empty if not set).
fn settings_dj_name() -> String {
    let Some(dir) = directories::ProjectDirs::from("de", "TStudio", "TStudio") else { return String::new() };
    let Ok(s) = std::fs::read_to_string(dir.config_dir().join("settings.ron")) else { return String::new() };
    let Some(i) = s.find("dj_name: \"") else { return String::new() };
    let mut out = String::new();
    let mut esc = false;
    for c in s[i + 10..].chars() {
        match (esc, c) {
            (false, '\\') => esc = true,
            (false, '"') => break,
            (_, c) => {
                out.push(c);
                esc = false;
            }
        }
    }
    out
}

fn load(p: &Path) -> R<Project> {
    live::guard(p)?;
    if live::targets_window(p) {
        let v = live::request("get", json!({}))?;
        return Project::from_ron(v["project_ron"].as_str().unwrap_or_default()).map_err(|e| format!("Projekt aus TStudio unlesbar: {e}"));
    }
    if !p.exists() {
        return Err(format!("{} gibt es nicht (für das in TStudio offene Projekt: `live`)", p.display()));
    }
    Project::load(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn save(p: &Path, proj: &mut Project) -> R<()> {
    live::guard(p)?;
    proj.sanitize();
    if live::targets_window(p) {
        let ron = proj.to_ron()?;
        // status-bar label: the command words before the project argument, e.g. "notes add"
        let arg = p.to_string_lossy();
        let label: Vec<String> = std::env::args().skip(1).take_while(|a| *a != arg).take(3).collect();
        live::request("put", json!({"project_ron": ron, "label": label.join(" ")}))?;
        return Ok(());
    }
    proj.save(p).map_err(|e| format!("Speichern fehlgeschlagen ({}): {e}", p.display()))
}

fn abs(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn channel_idx(p: &Project, sel: &str) -> R<usize> {
    if let Ok(i) = sel.parse::<usize>() {
        if i < p.channels.len() {
            return Ok(i);
        }
    }
    p.channels
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(sel))
        .ok_or_else(|| format!("Kanal '{sel}' nicht gefunden (vorhanden: {})", p.channels.iter().enumerate().map(|(i, c)| format!("{i}={}", c.name)).collect::<Vec<_>>().join(", ")))
}

fn pattern_id(p: &Project, sel: &str) -> R<Id> {
    if let Ok(i) = sel.parse::<usize>() {
        if let Some(pat) = p.patterns.get(i) {
            return Ok(pat.id);
        }
    }
    p.patterns
        .iter()
        .find(|x| x.name.eq_ignore_ascii_case(sel))
        .map(|x| x.id)
        .ok_or_else(|| format!("Pattern '{sel}' nicht gefunden (vorhanden: {})", p.patterns.iter().enumerate().map(|(i, x)| format!("{i}={}", x.name)).collect::<Vec<_>>().join(", ")))
}

fn stretch_mode(s: &str) -> R<StretchMode> {
    match s.to_lowercase().as_str() {
        "stretch" | "time" => Ok(StretchMode::Stretch),
        "resample" | "vinyl" => Ok(StretchMode::Resample),
        "off" | "none" => Ok(StretchMode::Off),
        _ => Err(format!("Stretch-Modus '{s}' unbekannt (stretch, resample, off)")),
    }
}

fn synth_preset(name: &str) -> R<SynthParams> {
    let all = SynthParams::presets();
    all.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, p)| p.clone())
        .ok_or_else(|| format!("Preset '{name}' unbekannt (vorhanden: {})", all.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")))
}

/// Picks a SoundFont preset by name / part of the name / `BANK:PROGRAM`, or explicit numbers.
fn pick_sf_preset(presets: &[Sf2Preset], sel: Option<&str>, program: Option<u8>, bank: Option<u16>) -> R<Sf2Preset> {
    if presets.is_empty() {
        return Err("SoundFont enthält keine Presets".into());
    }
    if let Some(s) = sel {
        if let Some((b, p)) = s.split_once(':') {
            if let (Ok(b), Ok(p)) = (b.trim().parse::<u16>(), p.trim().parse::<u8>()) {
                return presets.iter().find(|x| x.bank == b && x.program == p).cloned().ok_or_else(|| format!("kein Preset {b}:{p}"));
            }
        }
        let l = s.to_lowercase();
        return presets
            .iter()
            .find(|x| x.name.to_lowercase() == l)
            .or_else(|| presets.iter().find(|x| x.name.to_lowercase().contains(&l)))
            .cloned()
            .ok_or_else(|| format!("kein Preset '{s}' (tstudio-cli soundfont DATEI listet alle)"));
    }
    if program.is_some() || bank.is_some() {
        let (b, p) = (bank.unwrap_or(0), program.unwrap_or(0));
        return presets.iter().find(|x| x.bank == b && x.program == p).cloned().ok_or_else(|| format!("kein Preset {b}:{p}"));
    }
    Ok(presets[0].clone())
}

fn effect_kind(s: &str) -> R<EffectKind> {
    EffectKind::ALL
        .into_iter()
        .find(|k| k.name().eq_ignore_ascii_case(s) || (s.eq_ignore_ascii_case("comp") && *k == EffectKind::Compressor))
        .ok_or_else(|| format!("Effekt '{s}' unbekannt (vorhanden: {})", EffectKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", ")))
}

/// Stretch settings for a sample: explicit BPM, else from name/detection.
fn stretch_for(path: &Path, args: &StretchArgs, data: Option<&SampleData>) -> R<(StretchSettings, Option<f64>)> {
    let detected = data.map(|d| tstudio_library::analyze_data(d, &path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())).and_then(|a| a.bpm);
    let mode = match &args.stretch {
        Some(m) => stretch_mode(m)?,
        None => StretchMode::Off,
    };
    let src = args.sample_bpm.or(detected);
    if mode != StretchMode::Off && src.is_none() {
        return Err("Sample-BPM unbekannt – bitte --sample-bpm angeben".into());
    }
    Ok((StretchSettings { mode, source_bpm: src.unwrap_or(120.0), target_bpm: None, pitch: args.pitch.unwrap_or(0.0), formant: args.formant }, detected))
}

fn ticks_to_beats(t: i64) -> f64 {
    (t as f64 / PPQ as f64 * 1000.0).round() / 1000.0
}

fn info(p: &Project) -> Value {
    let bar = p.ticks_per_bar();
    json!({
        "name": p.name,
        "bpm": p.bpm,
        "beats_per_bar": p.beats_per_bar,
        "swing": p.swing,
        "master_volume": p.mixer[0].volume,
        "song_length_bars": p.song_length() as f64 / bar as f64,
        "channels": p.channels.iter().enumerate().map(|(i, c)| {
            let mut v = json!({"index": i, "name": c.name, "type": c.instrument.kind_name(), "mixer_track": c.mixer_track, "volume": c.volume, "pan": c.pan, "muted": c.muted});
            if let Instrument::Sampler(s) = &c.instrument {
                v["sample"] = json!(s.sample);
                v["root"] = json!(note_name(s.params.root_note));
                v["stretch"] = json!(s.stretch.mode.name());
                if !s.params.slices.is_empty() { v["slices"] = json!(s.params.slices.len()); }
            }
            if let Instrument::SoundFont(s) = &c.instrument {
                v["soundfont"] = json!(s.path);
                v["preset"] = json!(format!("{}:{} {}", s.bank, s.program, s.preset_name));
            }
            v
        }).collect::<Vec<_>>(),
        "patterns": p.patterns.iter().enumerate().map(|(i, pat)| {
            let mut per: Vec<Value> = Vec::new();
            for (ci, c) in p.channels.iter().enumerate() {
                let n = pat.notes.iter().filter(|n| n.channel == c.id).count();
                if n > 0 { per.push(json!({"channel": ci, "notes": n})); }
            }
            json!({"index": i, "name": pat.name, "length_bars": p.pattern_length(pat.id) as f64 / bar as f64, "notes": pat.notes.len(), "by_channel": per})
        }).collect::<Vec<_>>(),
        "playlist_tracks": p.playlist.tracks.len(),
        "tracks": (0..p.playlist.tracks.len()).filter(|t| {
            let tr = &p.playlist.tracks[*t];
            tr.muted || tr.solo || !tr.name.starts_with("Track ") || p.playlist.clips.iter().any(|c| c.track == *t)
        }).map(|t| track_json(p, t)).collect::<Vec<_>>(),
        "clips": p.playlist.clips.iter().map(|c| {
            let what = match &c.kind {
                ClipKind::Pattern { pattern } => json!({"pattern": p.patterns.iter().position(|x| x.id == *pattern), "pattern_name": p.pattern(*pattern).map(|x| x.name.clone())}),
                ClipKind::Audio(a) => json!({"audio": a.path, "stretch": a.stretch.mode.name(), "gain": a.gain, "mixer_track": a.mixer_track}),
                ClipKind::Automation(a) => json!({"automation": a.target, "points": a.points.len(),
                    "from": a.points.first().map(|q| q.v), "to": a.points.last().map(|q| q.v)}),
            };
            json!({"id": c.id, "track": c.track + 1, "bar": c.start as f64 / bar as f64 + 1.0, "bars": c.len as f64 / bar as f64, "muted": c.muted, "content": what})
        }).collect::<Vec<_>>(),
        "mixer": p.mixer.iter().enumerate().filter(|(i, t)| *i == 0 || !t.fx.is_empty() || p.channels.iter().any(|c| c.mixer_track == *i) || (t.volume - 0.8).abs() > 1e-4 || t.muted || t.solo
                || t.output != 0 || !t.sends.is_empty() || p.mixer.iter().any(|o| o.output == *i || o.sends.iter().any(|s| s.target == *i)))
            .map(|(i, t)| {
                let mut v = json!({"track": i, "name": t.name, "volume": t.volume, "pan": t.pan, "muted": t.muted, "solo": t.solo,
                    "fx": t.fx.iter().enumerate().map(|(s, f)| {
                        let mut fx = json!({"slot": s, "kind": fx_kind_name(&f.params), "enabled": f.enabled});
                        if let Some(src) = f.params.sidechain_source() { fx["sidechain_source"] = json!(src); }
                        fx
                    }).collect::<Vec<_>>()});
                if t.output != 0 { v["output"] = json!(t.output); }
                if !t.sends.is_empty() { v["sends"] = json!(t.sends.iter().map(|s| json!({"to": s.target, "amount": s.amount})).collect::<Vec<_>>()); }
                v
            })
            .collect::<Vec<_>>(),
        "hint": "Indizes sind 0-basiert (Kanäle, Patterns, Mixer: 0 = Master); Clip-Spuren/Takte 1-basiert wie in der GUI.",
    })
}

fn fx_kind_name_p(s: &FxSlot) -> String {
    s.params.kind().name().to_string()
}

fn fx_kind_name(p: &tstudio_dsp::fx::EffectParams) -> String {
    serde_json::to_value(p).ok().and_then(|v| v.as_object().and_then(|o| o.keys().next().cloned())).unwrap_or_default()
}

// --- generic JSON-path access

fn path_segments(path: &str) -> Vec<String> {
    path.split(['.', '/']).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
}

fn get_path<'a>(mut v: &'a Value, path: &str) -> R<&'a Value> {
    for seg in path_segments(path) {
        v = match v {
            Value::Object(o) => o.get(&seg).ok_or_else(|| format!("'{seg}' nicht gefunden (möglich: {})", o.keys().cloned().collect::<Vec<_>>().join(", ")))?,
            Value::Array(a) => {
                let i: usize = seg.parse().map_err(|_| format!("'{seg}' ist kein Index"))?;
                a.get(i).ok_or_else(|| format!("Index {i} außerhalb (Länge {})", a.len()))?
            }
            _ => return Err(format!("'{seg}': Wert hat keine Unterfelder")),
        };
    }
    Ok(v)
}

fn set_path(root: &mut Value, path: &str, new: Value) -> R<()> {
    let segs = path_segments(path);
    let (last, parents) = segs.split_last().ok_or("leerer Pfad")?;
    let mut v = root;
    for seg in parents {
        v = match v {
            Value::Object(o) => o.get_mut(seg).ok_or_else(|| format!("'{seg}' nicht gefunden"))?,
            Value::Array(a) => {
                let n = a.len();
                let i: usize = seg.parse().map_err(|_| format!("'{seg}' ist kein Index"))?;
                a.get_mut(i).ok_or_else(|| format!("Index {i} außerhalb (Länge {n})"))?
            }
            _ => return Err(format!("'{seg}': Wert hat keine Unterfelder")),
        };
    }
    match v {
        Value::Object(o) => {
            if !o.contains_key(last.as_str()) && !o.is_empty() {
                return Err(format!("Feld '{last}' existiert nicht (möglich: {})", o.keys().cloned().collect::<Vec<_>>().join(", ")));
            }
            o.insert(last.clone(), new);
        }
        Value::Array(a) => {
            let n = a.len();
            let i: usize = last.parse().map_err(|_| format!("'{last}' ist kein Index"))?;
            *a.get_mut(i).ok_or_else(|| format!("Index {i} außerhalb (Länge {n})"))? = new;
        }
        _ => return Err("Ziel ist kein Objekt/Array".into()),
    }
    Ok(())
}

// ------------------------------------------------------------------ commands

fn run(cmd: Cmd) -> R<Value> {
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

fn default_download_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.audio_dir().map(|d| d.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
        .join("TStudio Samples")
        .join("Downloads")
}

fn channel_cmd(c: ChannelCmd) -> R<Value> {
    match c {
        ChannelCmd::Add { project, synth, preset, sample, soundfont, program, bank, name, root, stretch } => {
            let mut p = load(&project)?;
            let mut extra = json!({});
            let (inst, default_name) = match (&sample, synth || preset.is_some()) {
                _ if soundfont.is_some() => {
                    let path = abs(soundfont.as_ref().unwrap());
                    let presets = soundfont_presets(&path)?;
                    let pr = pick_sf_preset(&presets, preset.as_deref(), program, bank)?;
                    extra["preset"] = json!(format!("{}:{} {}", pr.bank, pr.program, pr.name));
                    extra["presets_in_file"] = json!(presets.len());
                    let name = if pr.name.is_empty() { path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("SoundFont".into()) } else { pr.name.clone() };
                    (Instrument::SoundFont(SoundFontInstrument { path, bank: pr.bank, program: pr.program, preset_name: pr.name, gain: 1.0 }), name)
                }
                (Some(path), _) => {
                    let path = abs(path);
                    let data = audio::load(&path)?;
                    let (st, detected) = stretch_for(&path, &stretch, Some(&data))?;
                    extra["detected_bpm"] = json!(detected);
                    let mut s = SamplerInstrument { sample: Some(path.clone()), stretch: st, ..Default::default() };
                    if let Some(r) = &root {
                        s.params.root_note = spec::parse_key(r)?;
                    }
                    (Instrument::Sampler(s), path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("Sampler".into()))
                }
                (None, true) => {
                    let params = match &preset {
                        Some(n) => synth_preset(n)?,
                        None => SynthParams::default(),
                    };
                    (Instrument::Synth(params), preset.clone().unwrap_or("TSynth".into()))
                }
                (None, false) => return Err("--synth [--preset NAME], --sample DATEI oder --soundfont DATEI.sf2 angeben".into()),
            };
            let idx = p.add_channel(&name.unwrap_or(default_name), inst);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "channel": idx, "name": p.channels[idx].name, "mixer_track": p.channels[idx].mixer_track, "extra": extra}))
        }
        ChannelCmd::Remove { project, channel } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let name = p.channels[i].name.clone();
            p.remove_channel(i);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "removed": name}))
        }
        ChannelCmd::Mod { project, channel, source, dest, amount, clear } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let Instrument::Synth(s) = &mut p.channels[i].instrument else { return Err(format!("Kanal {i} ist kein TSynth")) };
            if clear {
                s.mod_matrix = [ModSlot::default(); tstudio_dsp::synth::NUM_MOD_SLOTS];
            }
            match (&source, &dest) {
                (Some(src), Some(dst)) => {
                    let src = ModSource::ALL
                        .into_iter()
                        .find(|m| format!("{m:?}").eq_ignore_ascii_case(src) || m.name().eq_ignore_ascii_case(src))
                        .ok_or_else(|| format!("Quelle '{src}' unbekannt (möglich: {})", ModSource::ALL.iter().skip(1).map(|m| format!("{m:?}")).collect::<Vec<_>>().join(", ")))?;
                    let dst = ModDest::ALL
                        .into_iter()
                        .find(|m| format!("{m:?}").eq_ignore_ascii_case(dst) || m.name().eq_ignore_ascii_case(dst))
                        .ok_or_else(|| format!("Ziel '{dst}' unbekannt (möglich: {})", ModDest::ALL.iter().skip(1).map(|m| format!("{m:?}")).collect::<Vec<_>>().join(", ")))?;
                    if dst.is_global() && !src.is_macro() {
                        return Err(format!("{} ist ein FX-Ziel und nimmt nur Makros als Quelle", dst.name()));
                    }
                    if !s.add_mod(src, dst, amount.clamp(-1.0, 1.0)) {
                        return Err("Mod-Matrix ist voll (16 Slots) – `--clear` oder per `set` überschreiben".into());
                    }
                }
                (None, None) => {}
                _ => return Err("--source und --dest gehören zusammen".into()),
            }
            let matrix: Vec<Value> = s.mod_matrix.iter().enumerate().filter(|(_, m)| m.active()).map(|(k, m)| json!({"slot": k, "source": m.source, "dest": m.dest, "amount": m.amount})).collect();
            let macros = s.macros();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "channel": i, "mod_matrix": matrix, "macros": macros, "set_macro_with": format!("channels.{i}.instrument.Synth.macro1=0.7")}))
        }
        ChannelCmd::Kick { project, channel, tune, off } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let Instrument::Synth(s) = &mut p.channels[i].instrument else { return Err(format!("Kanal {i} ist kein TSynth")) };
            s.kick.enabled = !off;
            if let Some(t) = &tune {
                s.kick.tune = match t.trim().parse::<f32>() {
                    Ok(n) => n,
                    Err(_) => spec::parse_key(t)? as f32,
                };
            }
            let k = s.kick.clone();
            save(&project, &mut p)?;
            Ok(json!({
                "ok": true,
                "channel": i,
                "kick": k,
                "fundamental_hz": tstudio_dsp::midi_to_freq(k.tune),
                "note": tstudio_dsp::note_name(k.tune.round().clamp(0.0, 127.0) as u8),
            }))
        }
        ChannelCmd::Preset { project, channel, preset } => {
            let mut p = load(&project)?;
            let i = channel_idx(&p, &channel)?;
            let chosen = match &mut p.channels[i].instrument {
                Instrument::Synth(s) => {
                    *s = synth_preset(&preset)?;
                    preset.clone()
                }
                Instrument::SoundFont(s) => {
                    let pr = pick_sf_preset(&soundfont_presets(&s.path)?, Some(&preset), None, None)?;
                    s.bank = pr.bank;
                    s.program = pr.program;
                    s.preset_name = pr.name.clone();
                    format!("{}:{} {}", pr.bank, pr.program, pr.name)
                }
                Instrument::Sampler(_) => return Err(format!("Kanal {i} ist ein Sampler – Presets gibt es für TSynth und SoundFont")),
            };
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "channel": i, "preset": chosen}))
        }
    }
}

fn pattern_cmd(c: PatternCmd) -> R<Value> {
    match c {
        PatternCmd::Add { project, name, bars } => {
            let mut p = load(&project)?;
            let id = p.add_pattern();
            let bar = p.ticks_per_bar();
            let pat = p.pattern_mut(id).unwrap();
            if let Some(n) = name {
                pat.name = n;
            }
            pat.length = bars.map(|b| (b * bar as f64).round() as i64);
            let idx = p.patterns.len() - 1;
            let name = p.patterns[idx].name.clone();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "pattern": idx, "name": name}))
        }
        PatternCmd::Remove { project, pattern } => {
            let mut p = load(&project)?;
            let id = pattern_id(&p, &pattern)?;
            if p.patterns.len() <= 1 {
                return Err("das letzte Pattern kann nicht gelöscht werden".into());
            }
            p.remove_pattern(id);
            save(&project, &mut p)?;
            Ok(json!({"ok": true}))
        }
    }
}

fn notes_cmd(c: NotesCmd) -> R<Value> {
    match c {
        NotesCmd::Add { t, notes, len, vel, offset, replace } => {
            let mut p = load(&t.project)?;
            let ci = channel_idx(&p, &t.channel)?;
            let cid = p.channels[ci].id;
            let pid = pattern_id(&p, &t.pattern)?;
            let default_len = spec::parse_beats(&len)?;
            let off = spec::parse_beats(&offset)?;
            let mut new = Vec::new();
            for tok in notes.iter().flat_map(|s| s.split([' ', ',', ';'])).filter(|s| !s.trim().is_empty()) {
                let n = spec::parse_note(tok.trim(), default_len, vel)?;
                for k in n.keys {
                    new.push(Note { channel: cid, key: k, start: n.start + off, len: n.len, velocity: n.velocity });
                }
            }
            let pat = p.pattern_mut(pid).unwrap();
            if replace {
                pat.notes.retain(|n| n.channel != cid);
            }
            // identical key+start replaces the old note
            pat.notes.retain(|o| !(o.channel == cid && new.iter().any(|n| n.key == o.key && n.start == o.start)));
            let added = new.len();
            pat.notes.extend(new);
            pat.notes.sort_by_key(|n| (n.start, n.key));
            let total = pat.notes.iter().filter(|n| n.channel == cid).count();
            let len_bars = p.pattern_length(pid) as f64 / p.ticks_per_bar() as f64;
            save(&t.project, &mut p)?;
            Ok(json!({"ok": true, "added": added, "channel_notes": total, "pattern_length_bars": len_bars}))
        }
        NotesCmd::Steps { t, steps, key, len, offset, keep } => {
            let mut p = load(&t.project)?;
            let ci = channel_idx(&p, &t.channel)?;
            let cid = p.channels[ci].id;
            let pid = pattern_id(&p, &t.pattern)?;
            let key = match key {
                Some(k) => spec::parse_key(&k)?,
                None => match &p.channels[ci].instrument {
                    Instrument::Sampler(s) => s.params.root_note,
                    Instrument::Synth(_) => 60,
                    Instrument::SoundFont(s) => if s.bank >= 128 { 36 } else { 60 },
                },
            };
            let off = spec::parse_beats(&offset)?;
            let parsed = spec::parse_steps(&steps)?;
            let step = PPQ / 4;
            let pat = p.pattern_mut(pid).unwrap();
            if !keep {
                pat.notes.retain(|n| n.channel != cid);
            }
            for (i, v) in &parsed {
                let start = off + *i as i64 * step;
                pat.notes.retain(|n| !(n.channel == cid && n.start == start && n.key == key));
                pat.notes.push(Note { channel: cid, key, start, len: step * len.max(1), velocity: *v });
            }
            pat.notes.sort_by_key(|n| (n.start, n.key));
            save(&t.project, &mut p)?;
            Ok(json!({"ok": true, "hits": parsed.len(), "key": note_name(key)}))
        }
        NotesCmd::Chords { t, progression, bars_per_chord, style, octave, vel, rhythm, offset, replace, no_voicing } => {
            let mut p = load(&t.project)?;
            let ci = channel_idx(&p, &t.channel)?;
            let cid = p.channels[ci].id;
            let pid = pattern_id(&p, &t.pattern)?;
            let off = spec::parse_beats(&offset)?;
            let bass_like = matches!(style.to_lowercase().as_str(), "bass" | "offbeat" | "rolling");
            let oct = octave.unwrap_or(if bass_like { 2 } else { 4 });
            let (gen, names) = tools::chord_notes(&progression, bars_per_chord, &style, oct, vel, rhythm.as_deref(), p.ticks_per_bar(), !no_voicing)?;
            let pat = p.pattern_mut(pid).unwrap();
            if replace {
                pat.notes.retain(|n| n.channel != cid);
            }
            let added = gen.len();
            for g in gen {
                let start = g.start + off;
                pat.notes.retain(|n| !(n.channel == cid && n.start == start && n.key == g.key));
                pat.notes.push(Note { channel: cid, key: g.key, start, len: g.len, velocity: g.vel });
            }
            pat.notes.sort_by_key(|n| (n.start, n.key));
            let len_bars = p.pattern_length(pid) as f64 / p.ticks_per_bar() as f64;
            save(&t.project, &mut p)?;
            Ok(json!({"ok": true, "chords": names, "notes_added": added, "octave": oct, "pattern_length_bars": len_bars}))
        }
        NotesCmd::Clear { project, pattern, channel } => {
            let mut p = load(&project)?;
            let pid = pattern_id(&p, &pattern)?;
            let cid = match &channel {
                Some(c) => Some(p.channels[channel_idx(&p, c)?].id),
                None => None,
            };
            let pat = p.pattern_mut(pid).unwrap();
            let before = pat.notes.len();
            pat.notes.retain(|n| cid.map(|c| n.channel != c).unwrap_or(false));
            let removed = before - pat.notes.len();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "removed": removed}))
        }
        NotesCmd::List { project, pattern, channel } => {
            let p = load(&project)?;
            let pid = pattern_id(&p, &pattern)?;
            let cid = match &channel {
                Some(c) => Some(p.channels[channel_idx(&p, c)?].id),
                None => None,
            };
            let pat = p.pattern(pid).unwrap();
            let notes: Vec<Value> = pat
                .notes
                .iter()
                .filter(|n| cid.map(|c| n.channel == c).unwrap_or(true))
                .map(|n| {
                    json!({
                        "channel": p.channel_index(n.channel),
                        "key": note_name(n.key),
                        "midi": n.key,
                        "start_beats": ticks_to_beats(n.start),
                        "len_beats": ticks_to_beats(n.len),
                        "velocity": (n.velocity * 100.0).round() / 100.0,
                    })
                })
                .collect();
            Ok(json!({"pattern": pat.name, "length_bars": p.pattern_length(pid) as f64 / p.ticks_per_bar() as f64, "notes": notes}))
        }
    }
}

fn clip_cmd(c: ClipCmd) -> R<Value> {
    match c {
        ClipCmd::Add { project, track, bar, pattern, audio, bars, repeat, gain, stretch } => {
            let mut p = load(&project)?;
            if track == 0 {
                return Err("--track ist 1-basiert (1 = erste Spur)".into());
            }
            let bar_t = p.ticks_per_bar();
            let start = ((bar - 1.0).max(0.0) * bar_t as f64).round() as i64;
            let mut ids = Vec::new();
            let mut extra = json!({});
            match (pattern, audio) {
                (Some(sel), None) => {
                    let id = pattern_id(&p, &sel)?;
                    let len = bars.map(|b| (b * bar_t as f64).round() as i64).unwrap_or_else(|| p.pattern_length(id));
                    for i in 0..repeat.max(1) as i64 {
                        ids.push(p.add_clip(track - 1, start + i * len, len, ClipKind::Pattern { pattern: id }));
                    }
                }
                (None, Some(path)) => {
                    let path = abs(&path);
                    let data = audio::load(&path)?;
                    let (st, detected) = stretch_for(&path, &stretch, Some(&data))?;
                    extra["detected_bpm"] = json!(detected);
                    // length of the (tempo-fitted) audio in ticks
                    let secs = match st.mode {
                        StretchMode::Off => data.duration_secs(),
                        _ => data.duration_secs() * st.source_bpm / p.bpm,
                    };
                    let natural = p.secs_to_ticks(secs).round() as i64;
                    let len = bars.map(|b| (b * bar_t as f64).round() as i64).unwrap_or(natural).max(1);
                    let clip = AudioClip { path, gain, stretch: st, ..Default::default() };
                    for i in 0..repeat.max(1) as i64 {
                        ids.push(p.add_clip(track - 1, start + i * len, len, ClipKind::Audio(clip.clone())));
                    }
                    extra["length_bars"] = json!(len as f64 / bar_t as f64);
                }
                _ => return Err("genau eins von --pattern oder --audio angeben".into()),
            }
            let song = p.song_length() as f64 / bar_t as f64;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "clip_ids": ids, "song_length_bars": song, "extra": extra}))
        }
        ClipCmd::Remove { project, ids, all } => {
            let mut p = load(&project)?;
            let before = p.playlist.clips.len();
            if all {
                p.playlist.clips.clear();
            } else {
                if ids.is_empty() {
                    return Err("Clip-IDs angeben (siehe `info`) oder --all".into());
                }
                p.playlist.clips.retain(|c| !ids.contains(&c.id));
            }
            let removed = before - p.playlist.clips.len();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "removed": removed}))
        }
    }
}

fn fx_cmd(c: FxCmd) -> R<Value> {
    match c {
        FxCmd::Add { project, track, kind, source } => {
            let mut p = load(&project)?;
            let k = effect_kind(&kind)?;
            let n_tracks = p.mixer.len();
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} existiert nicht"))?;
            if t.fx.len() >= MAX_FX_SLOTS {
                return Err(format!("Spur {track} hat schon {MAX_FX_SLOTS} Effekte"));
            }
            let mut slot = FxSlot::new(k);
            if let tstudio_dsp::fx::EffectParams::Sidechain(sc) = &mut slot.params {
                let src = source.ok_or("sidechain braucht --source SPUR (Mixer-Spur der Kick)")?;
                if src >= n_tracks || src == track {
                    return Err(format!("--source {src} ungültig"));
                }
                sc.source = src;
            }
            t.fx.push(slot);
            let slot = t.fx.len() - 1;
            let params = serde_json::to_value(&t.fx[slot].params).unwrap_or_default();
            save(&project, &mut p)?;
            Ok(json!({
                "ok": true, "track": track, "slot": slot, "params": params,
                "set_with": format!("mixer.{track}.fx.{slot}.params.{}.<feld>=<wert>", k.name()),
            }))
        }
        FxCmd::Chain { project, track, name, source, replace } => {
            let mut p = load(&project)?;
            if track >= p.mixer.len() {
                return Err(format!("Mixer-Spur {track} existiert nicht"));
            }
            if let Some(s) = source {
                if s == 0 || s >= p.mixer.len() || s == track {
                    return Err(format!("--source {s} ungültig"));
                }
            }
            let slots = tools::chain(&name, source)?;
            let t = &mut p.mixer[track];
            if replace {
                t.fx.clear();
            }
            if t.fx.len() + slots.len() > MAX_FX_SLOTS {
                return Err(format!("Spur {track} hat {} Effekte, die Kette braucht {} Plätze (max {MAX_FX_SLOTS}) – --replace nutzen", t.fx.len(), slots.len()));
            }
            let first = t.fx.len();
            let kinds: Vec<String> = slots.iter().map(fx_kind_name_p).collect();
            t.fx.extend(slots);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": track, "chain": name, "slots_from": first, "effects": kinds}))
        }
        FxCmd::Remove { project, track, slot } => {
            let mut p = load(&project)?;
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} existiert nicht"))?;
            if slot >= t.fx.len() {
                return Err(format!("Slot {slot} existiert nicht"));
            }
            t.fx.remove(slot);
            save(&project, &mut p)?;
            Ok(json!({"ok": true}))
        }
    }
}

fn mixer_cmd(c: MixerCmd) -> R<Value> {
    match c {
        MixerCmd::Rumble { project, kick, amount, name } => {
            let mut p = load(&project)?;
            if kick == 0 || kick >= p.mixer.len() {
                return Err(format!("--kick {kick}: Mixer-Spur der Kick (1..{})", p.mixer.len() - 1));
            }
            let t = p.add_mixer_track(Some(name.clone()));
            if t <= kick {
                return Err("Rumble-Bus muss hinter der Kick-Spur liegen".into());
            }
            p.mixer[kick].sends.retain(|s| s.target != t);
            p.mixer[kick].sends.push(MixerSend { target: t, amount: amount.clamp(0.0, 1.0) });
            p.mixer[t].fx = tools::chain("rumble", Some(kick))?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "rumble_track": t, "name": name, "send_from": kick, "amount": amount,
                "hint": format!("Lautstärke: set PROJECT mixer.{t}.volume=0.6 · Tiefpass: mixer.{t}.fx.2.params.Filter.cutoff")}))
        }
        MixerCmd::Send { project, from, to, amount } => {
            let mut p = load(&project)?;
            if from == 0 || to <= from || to >= p.mixer.len() {
                return Err(format!("Sends gehen nur von einer Insert-Spur auf eine höhere (1..{}), z.B. --from 2 --to 10", p.mixer.len() - 1));
            }
            let t = &mut p.mixer[from];
            t.sends.retain(|s| s.target != to);
            if amount > 0.0 {
                t.sends.push(MixerSend { target: to, amount });
            }
            let sends = t.sends.iter().map(|s| json!({"to": s.target, "amount": s.amount})).collect::<Vec<_>>();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": from, "sends": sends}))
        }
        MixerCmd::Route { project, from, to } => {
            let mut p = load(&project)?;
            if from == 0 || from >= p.mixer.len() || (to != 0 && (to <= from || to >= p.mixer.len())) {
                return Err("Route: --from Insert-Spur, --to 0 (Master) oder eine höhere Spur".into());
            }
            p.mixer[from].output = to;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": from, "output": to}))
        }
        MixerCmd::Add { project, name, at } => {
            let mut p = load(&project)?;
            let t = match at {
                Some(a) => p.insert_mixer_track(a, name),
                None => p.add_mixer_track(name),
            };
            let name = p.mixer[t].name.clone();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": t, "name": name, "mixer_tracks": p.mixer.len()}))
        }
        MixerCmd::Delete { project, track } => {
            let mut p = load(&project)?;
            let name = p.mixer.get(track).map(|t| t.name.clone());
            p.delete_mixer_track(track)?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "deleted": track, "name": name, "mixer_tracks": p.mixer.len()}))
        }
        MixerCmd::Move { project, from, to } => {
            let mut p = load(&project)?;
            p.move_mixer_track(from, to)?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "moved": from, "to": to, "name": p.mixer[to].name}))
        }
        MixerCmd::Rename { project, track, name } => {
            let mut p = load(&project)?;
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} gibt es nicht"))?;
            t.name = name.clone();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": track, "name": name}))
        }
        MixerCmd::Mute { project, track, off } => {
            let mut p = load(&project)?;
            let t = p.mixer.get_mut(track).ok_or_else(|| format!("Mixer-Spur {track} gibt es nicht"))?;
            t.muted = !off;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": track, "muted": !off}))
        }
        MixerCmd::Solo { project, track, off, only, clear } => {
            let mut p = load(&project)?;
            if clear || only {
                for t in p.mixer.iter_mut() {
                    t.solo = false;
                }
            }
            if !clear {
                if track == 0 || track >= p.mixer.len() {
                    return Err(format!("Solo gibt es für Insert 1..{}", p.mixer.len() - 1));
                }
                p.mixer[track].solo = !off;
            }
            let solos: Vec<usize> = p.mixer.iter().enumerate().filter(|(_, t)| t.solo).map(|(i, _)| i).collect();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "solo": solos}))
        }
    }
}

fn track_json(p: &Project, t: usize) -> Value {
    let tr = &p.playlist.tracks[t];
    json!({"track": t + 1, "name": tr.name, "muted": tr.muted, "solo": tr.solo, "volume_percent": (tr.volume * 100.0).round(), "audible": p.playlist_track_audible(t),
        "clips": p.playlist.clips.iter().filter(|c| c.track == t).count()})
}

/// Playlist track number from the CLI (1-based) → index.
fn track_idx(p: &Project, track: usize) -> Result<usize, String> {
    if track == 0 || track > p.playlist.tracks.len() {
        return Err(format!("Playlist-Spur {track} gibt es nicht (1..{})", p.playlist.tracks.len()));
    }
    Ok(track - 1)
}

fn track_cmd(cmd: TrackCmd) -> Result<Value, String> {
    match cmd {
        TrackCmd::List { project } => {
            let p = load(&project)?;
            Ok(json!({"ok": true, "tracks": (0..p.playlist.tracks.len()).map(|t| track_json(&p, t)).collect::<Vec<_>>()}))
        }
        TrackCmd::Add { project, name, at } => {
            let mut p = load(&project)?;
            let t = match at {
                Some(a) => p.insert_playlist_track(a.max(1) - 1, name),
                None => p.add_playlist_track(name),
            };
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "added": v, "tracks": p.playlist.tracks.len()}))
        }
        TrackCmd::Delete { project, track } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            let name = p.playlist.tracks[t].name.clone();
            let clips = p.delete_playlist_track(t)?;
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "deleted": track, "name": name, "clips_deleted": clips, "tracks": p.playlist.tracks.len()}))
        }
        TrackCmd::Move { project, from, to } => {
            let mut p = load(&project)?;
            let (f, t) = (track_idx(&p, from)?, track_idx(&p, to)?);
            p.move_playlist_track(f, t)?;
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "moved": v}))
        }
        TrackCmd::Duplicate { project, track } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            let n = p.duplicate_playlist_track(t)?;
            let v = track_json(&p, n);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "copy": v}))
        }
        TrackCmd::Rename { project, track, name } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            p.playlist.tracks[t].name = name;
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": v}))
        }
        TrackCmd::Volume { project, track, percent } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            p.playlist.tracks[t].volume = (percent / 100.0).clamp(0.0, 1.25);
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": v}))
        }
        TrackCmd::Mute { project, track, off } => {
            let mut p = load(&project)?;
            let t = track_idx(&p, track)?;
            p.playlist.tracks[t].muted = !off;
            let v = track_json(&p, t);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "track": v}))
        }
        TrackCmd::Solo { project, track, off, only, clear } => {
            let mut p = load(&project)?;
            if clear || only {
                for t in p.playlist.tracks.iter_mut() {
                    t.solo = false;
                }
            }
            if !clear {
                let t = track_idx(&p, track)?;
                p.playlist.tracks[t].solo = !off;
            }
            let solos: Vec<usize> = p.playlist.tracks.iter().enumerate().filter(|(_, t)| t.solo).map(|(i, _)| i + 1).collect();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "solo": solos}))
        }
    }
}

/// `"0:200,4:8000"` (beat:value) → points in ticks.
fn parse_points(s: &str) -> R<Vec<AutoPoint>> {
    let mut v = Vec::new();
    for part in s.split([',', ' ']).filter(|x| !x.trim().is_empty()) {
        let (b, val) = part.split_once(':').ok_or_else(|| format!("Punkt '{part}' muss BEAT:WERT sein"))?;
        let ticks = spec::parse_beats(b.trim())?;
        let val: f32 = val.trim().parse().map_err(|_| format!("Wert '{val}' ist keine Zahl"))?;
        v.push(AutoPoint { t: ticks, v: val });
    }
    if v.is_empty() {
        return Err("--points ist leer".into());
    }
    v.sort_by_key(|p| p.t);
    Ok(v)
}

fn auto_cmd(c: AutoCmd) -> R<Value> {
    match c {
        AutoCmd::Add { project, target, track, bar, bars, from, to, points, repeat } => {
            let mut p = load(&project)?;
            if track == 0 {
                return Err("--track ist 1-basiert".into());
            }
            let t = p.parse_auto_target(&target)?;
            let bar_t = p.ticks_per_bar();
            let len = (bars * bar_t as f64).round().max(1.0) as i64;
            let start = ((bar - 1.0).max(0.0) * bar_t as f64).round() as i64;
            let (lo, hi, _) = field_range(Project::auto_field_name(&target));
            let base = p.auto_base_value(t);
            let mut pts = match (points, from, to) {
                (Some(s), _, _) => parse_points(&s)?,
                (None, a, b) => vec![AutoPoint { t: 0, v: a.unwrap_or(base) }, AutoPoint { t: len, v: b.unwrap_or(base) }],
            };
            let mut clamped = false;
            for q in pts.iter_mut() {
                let c = q.v.clamp(lo, hi);
                clamped |= c != q.v;
                q.v = c;
                q.t = q.t.clamp(0, len);
            }
            let clip = AutomationClip { target: target.clone(), points: pts };
            let mut ids = Vec::new();
            for i in 0..repeat.max(1) as i64 {
                ids.push(p.add_clip(track - 1, start + i * len, len, ClipKind::Automation(clip.clone())));
            }
            save(&project, &mut p)?;
            let mut out = json!({"ok": true, "clip_ids": ids, "target": target, "range": [lo, hi], "base_value": base,
                "note": "Automation wirkt im Song-Modus; nach dem Clip bleibt der letzte Wert stehen."});
            if clamped {
                out["warning"] = json!(format!("Werte auf den Bereich {lo}..{hi} begrenzt"));
            }
            Ok(out)
        }
        AutoCmd::Targets { project, filter } => {
            let p = load(&project)?;
            let f = filter.map(|s| s.to_lowercase());
            let list: Vec<Value> = p
                .automation_targets()
                .into_iter()
                .filter(|(t, l)| f.as_ref().map(|f| t.to_lowercase().contains(f) || l.to_lowercase().contains(f)).unwrap_or(true))
                .map(|(t, l)| {
                    let (lo, hi, log) = field_range(Project::auto_field_name(&t));
                    json!({"target": t, "label": l, "min": lo, "max": hi, "log": log})
                })
                .collect();
            Ok(json!({"ok": true, "count": list.len(), "targets": list}))
        }
    }
}

/// f32 → JSON turns 0.8 into 0.800000011920929; round floats to 5 decimals for readability.
fn marker_cmd(c: MarkerCmd) -> R<Value> {
    let list = |p: &Project| -> Vec<Value> { p.playlist.markers.iter().map(|m| json!({"name": m.name, "bar": tools::tick_bar(p, m.tick)})).collect() };
    match c {
        MarkerCmd::Add { project, bar, name } => {
            let mut p = load(&project)?;
            if bar < 1.0 {
                return Err("--bar beginnt bei 1".into());
            }
            let tick = ((bar - 1.0) * p.ticks_per_bar() as f64).round() as i64;
            p.playlist.markers.retain(|m| !m.name.eq_ignore_ascii_case(name.trim()));
            p.playlist.markers.push(tstudio_project::Marker { tick, name: name.trim().to_string() });
            p.playlist.sort_markers();
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "markers": list(&p)}))
        }
        MarkerCmd::List { project } => {
            let p = load(&project)?;
            Ok(json!({"markers": list(&p)}))
        }
        MarkerCmd::Remove { project, name } => {
            let mut p = load(&project)?;
            let before = p.playlist.markers.len();
            p.playlist.markers.retain(|m| !m.name.eq_ignore_ascii_case(name.trim()));
            if before == p.playlist.markers.len() {
                return Err(format!("Marker '{name}' gibt es nicht"));
            }
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "markers": list(&p)}))
        }
        MarkerCmd::Clear { project } => {
            let mut p = load(&project)?;
            p.playlist.markers.clear();
            save(&project, &mut p)?;
            Ok(json!({"ok": true}))
        }
    }
}

fn parse_tracks(s: &Option<String>, p: &Project) -> R<Option<std::collections::BTreeSet<usize>>> {
    let Some(s) = s else { return Ok(None) };
    let mut set = std::collections::BTreeSet::new();
    for t in s.split([',', ' ']).filter(|x| !x.trim().is_empty()) {
        let n: usize = t.trim().parse().map_err(|_| format!("Spur '{t}' unverständlich"))?;
        set.insert(track_idx(p, n)?);
    }
    Ok(Some(set))
}

/// (start tick, length ticks) of a range; without --bars up to the next marker.
fn range_ticks(p: &Project, r: &Range) -> R<(i64, i64)> {
    let from = tools::bar_tick(p, &r.from)?;
    let len = match r.bars {
        Some(b) if b > 0.0 => (b * p.ticks_per_bar() as f64).round() as i64,
        Some(_) => return Err("--bars muss > 0 sein".into()),
        None => {
            let next = p.playlist.markers.iter().map(|m| m.tick).filter(|t| *t > from).min();
            match next {
                Some(n) => n - from,
                None => return Err("--bars fehlt (und es gibt keinen Marker danach)".into()),
            }
        }
    };
    Ok((from, len))
}

fn section_cmd(c: SectionCmd) -> R<Value> {
    match c {
        SectionCmd::Copy { r, to } => {
            let mut p = load(&r.project)?;
            let (from, len) = range_ticks(&p, &r)?;
            let to_t = tools::bar_tick(&p, &to)?;
            let tracks = parse_tracks(&r.tracks, &p)?;
            let n = tools::section_copy(&mut p, from, len, to_t, tracks.as_ref());
            save(&r.project, &mut p)?;
            Ok(json!({"ok": true, "copied_clips": n, "from_bar": tools::tick_bar(&p, from), "bars": len as f64 / p.ticks_per_bar() as f64, "to_bar": tools::tick_bar(&p, to_t)}))
        }
        SectionCmd::Clear { r } => {
            let mut p = load(&r.project)?;
            let (from, len) = range_ticks(&p, &r)?;
            let tracks = parse_tracks(&r.tracks, &p)?;
            let n = tools::section_clear(&mut p, from, len, tracks.as_ref());
            save(&r.project, &mut p)?;
            Ok(json!({"ok": true, "removed_clips": n}))
        }
        SectionCmd::Insert { project, at, bars } => {
            let mut p = load(&project)?;
            let at_t = tools::bar_tick(&p, &at)?;
            let len = (bars * p.ticks_per_bar() as f64).round() as i64;
            if len <= 0 {
                return Err("--bars muss > 0 sein".into());
            }
            // clips that span the insert point are split by shortening (the rest keeps its place)
            for c in p.playlist.clips.iter_mut() {
                if c.start < at_t && c.end() > at_t {
                    c.len = at_t - c.start;
                }
            }
            tools::shift_after(&mut p, at_t, len);
            save(&project, &mut p)?;
            Ok(json!({"ok": true, "inserted_bars": bars, "at_bar": tools::tick_bar(&p, at_t)}))
        }
        SectionCmd::Delete { r } => {
            let mut p = load(&r.project)?;
            if r.tracks.is_some() {
                return Err("delete wirkt auf alle Spuren (für einzelne Spuren: section clear)".into());
            }
            let (from, len) = range_ticks(&p, &r)?;
            let n = tools::section_clear(&mut p, from, len, None);
            p.playlist.markers.retain(|m| m.tick < from || m.tick >= from + len);
            tools::shift_after(&mut p, from + len, -len);
            save(&r.project, &mut p)?;
            Ok(json!({"ok": true, "removed_clips": n, "removed_bars": len as f64 / p.ticks_per_bar() as f64}))
        }
    }
}

fn tidy(v: &mut Value) {
    match v {
        Value::Number(n) if n.is_f64() => {
            if let Some(f) = n.as_f64() {
                let r = (f * 1e5).round() / 1e5;
                if let Some(x) = serde_json::Number::from_f64(r) {
                    *n = x;
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(tidy),
        Value::Object(o) => o.values_mut().for_each(tidy),
        _ => {}
    }
}

fn main() {
    let cli = Cli::parse();
    live::set_no_live(cli.no_live);
    let pretty = cli.pretty;
    match run(cli.cmd) {
        Ok(mut v) => {
            tidy(&mut v);
            if live::routed_path() {
                if let Value::Object(o) = &mut v {
                    o.insert("routed_to_window".into(), json!(true));
                    o.insert("note".into(), json!("Datei ist in TStudio offen – die Änderung ging ins Fenster (Strg+Z dort rückgängig). --no-live verhindert das."));
                }
            }
            let out = if pretty { serde_json::to_string_pretty(&v) } else { serde_json::to_string(&v) };
            println!("{}", out.unwrap_or_default())
        }
        Err(e) => {
            println!("{}", json!({"ok": false, "error": e}));
            std::process::exit(1);
        }
    }
}
