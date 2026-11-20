//! Command-line definition (clap): `Cli`, `Cmd` and all sub-command enums/structs.

use crate::*;

mod midi;
mod timeline;
pub(crate) use midi::*;
pub(crate) use timeline::*;

#[derive(Parser)]
#[command(
    name = "tstudio-cli",
    version,
    about = "TStudio ohne GUI: Projekte bearbeiten, rendern, Audio analysieren. Ausgabe immer JSON.",
    after_help = "PROJECT: Pfad zu einer .tsp-Datei – oder `live` für das Projekt, das gerade in TStudio offen ist
(inkl. ungespeicherter Änderungen; jede Änderung ist dort mit Strg+Z rückgängig zu machen)."
)]
pub(crate) struct Cli {
    /// Nie ins offene TStudio-Fenster schreiben: ist die Datei dort offen, bricht der Befehl ab
    #[arg(long, global = true)]
    pub(crate) no_live: bool,
    /// JSON eingerückt ausgeben (Standard: kompakt, eine Zeile – spart Platz/Token)
    #[arg(long, global = true)]
    pub(crate) pretty: bool,
    #[command(subcommand)]
    pub(crate) cmd: Cmd,
}

#[derive(Subcommand)]
pub(crate) enum Cmd {
    /// Neues Projekt anlegen (mit TSynth-Kanal und Pattern 1)
    New {
        project: PathBuf,
        /// Tempo (Standard 140, mit --template das der Vorlage)
        #[arg(long)]
        bpm: Option<f64>,
        #[arg(long)]
        name: Option<String>,
        /// ohne Standard-Kanal
        #[arg(long)]
        empty: bool,
        /// aus Vorlage: eigene (TStudio-Vorlagenordner) oder mitgelieferte, z.B. "Schranz 155"
        /// (Tempo der Vorlage, außer --bpm ist angegeben)
        #[arg(long, conflicts_with = "empty")]
        template: Option<String>,
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
    /// MIDI-Dateien: Pattern/Song exportieren, .mid als Pattern importieren
    #[command(subcommand)]
    Midi(MidiCmd),
    /// Song-Abschnitte: Takte kopieren, leeren, einfügen, löschen (Takt oder Markername)
    #[command(subcommand)]
    Section(SectionCmd),
    /// Tempo-Spur: Tempowechsel ab einem Takt (springend oder als Rampe)
    #[command(subcommand)]
    Tempo(TempoCmd),
    /// Taktartwechsel (4/4 → 7/8 → 4/4 …); --bar bleibt überall 1-basiert und folgt den Wechseln
    #[command(subcommand)]
    Meter(MeterCmd),
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
pub(crate) enum ChannelCmd {
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
pub(crate) enum MixerCmd {
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
pub(crate) enum TrackCmd {
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
pub(crate) enum AutoCmd {
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
pub(crate) struct IsoArgs {
    /// Preset: "Kick / Sub", "Bass", "Snare / Clap", "Hi-Hats / Becken", "Vocals (Mitte)", "Synths / Mitten", "Vocal entfernen", "Telefon"
    #[arg(long)]
    pub(crate) preset: Option<String>,
    /// Hochpass (Low Cut) in Hz
    #[arg(long)]
    pub(crate) hp: Option<f32>,
    /// Tiefpass (High Cut) in Hz
    #[arg(long)]
    pub(crate) lp: Option<f32>,
    /// Steilheit in dB/Okt: 24, 48 oder 96
    #[arg(long)]
    pub(crate) slope: Option<u32>,
    /// Band-EQ "FREQ:DB[:Q]", z.B. "2500:4:1.2"
    #[arg(long)]
    pub(crate) band: Option<String>,
    /// stereo | mid | side | left | right
    #[arg(long)]
    pub(crate) stereo: Option<String>,
    #[arg(long)]
    pub(crate) gain_db: Option<f32>,
}

impl IsoArgs {
    pub(crate) fn params(&self) -> Result<Option<tstudio_dsp::isolate::IsolateParams>, String> {
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
pub(crate) struct StretchArgs {
    /// Tempo anpassen: stretch | resample | off
    #[arg(long)]
    pub(crate) stretch: Option<String>,
    /// Original-BPM des Samples (Default: aus Dateiname/Erkennung)
    #[arg(long)]
    pub(crate) sample_bpm: Option<f64>,
    /// Tonhöhe in Halbtönen (jeder Modus, Länge bleibt)
    #[arg(long)]
    pub(crate) pitch: Option<f32>,
    /// Formanten halten beim Pitchen
    #[arg(long)]
    pub(crate) formant: bool,
}

#[derive(Subcommand)]
pub(crate) enum PatternCmd {
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
pub(crate) struct Target {
    pub(crate) project: PathBuf,
    /// Pattern (Index oder Name)
    #[arg(long, short)]
    pub(crate) pattern: String,
    /// Kanal (Index oder Name)
    #[arg(long, short)]
    pub(crate) channel: String,
}

#[derive(Subcommand)]
pub(crate) enum NotesCmd {
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
pub(crate) enum ClipCmd {
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
pub(crate) enum FxCmd {
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
pub(crate) enum MarkerCmd {
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
pub(crate) struct Range {
    pub(crate) project: PathBuf,
    /// Starttakt oder Markername
    #[arg(long)]
    pub(crate) from: String,
    /// Länge in Takten (Standard: bis zum nächsten Marker)
    #[arg(long)]
    pub(crate) bars: Option<f64>,
    /// nur diese Playlist-Spuren (1-basiert, z.B. 1,3,4)
    #[arg(long)]
    pub(crate) tracks: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum SectionCmd {
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
