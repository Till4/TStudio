---
name: tstudio
description: Bedient die TStudio-DAW des Users über tstudio-cli – direkt im gerade offenen Projekt –Beats, Basslines, Akkorde und Melodien in Patterns schreiben, Synth-Presets/-Parameter, SoundFonts (.sf2), Mixer-Effekte, Sidechain, Sends/Busse und Automation einstellen, Samples einbinden, tempo-anpassen, schneiden und chopen, Songs als WAV/MP3 oder MP4-Video mit Visualizer rendern und das Ergebnis per Audio-Analyse prüfen. Verwenden, wenn der User Musik in TStudio bauen/ändern will, ein .tsp-Projekt erwähnt oder Samples analysieren/zuschneiden/herunterladen möchte.
---

# TStudio über die CLI bedienen

TStudio ist die eigene DAW des Users (Rust, FL-Studio-artiger Workflow: Channel Rack → Patterns → Playlist → Mixer).
Du bedienst sie **ausschließlich über `tstudio-cli`** – niemals per Bildschirmsteuerung/Screenshots.

## Programm finden

```bash
CLI=E:/TStudio/target/release/tstudio-cli.exe
```

Falls die Datei fehlt oder veraltet ist: `cargo build -p tstudio-cli -p tstudio --release` in `E:/TStudio`.
Jeder Befehl gibt JSON aus; Fehler: `{"ok":false,"error":"…"}` mit Exit-Code 1 – die Meldung nennt gültige Alternativen.
`$CLI <befehl> --help` zeigt alle Optionen.

## Standard: das offene Projekt bearbeiten (`live`)

Der User arbeitet meist mit offenem TStudio. **Starte immer mit `$CLI status`:**

- `"running": true` → nimm **`live`** als Projekt-Argument für alles, was der User „mein Projekt / den Track / das Offene“ nennt
  (oder wenn er gar nichts angibt): `$CLI info live`, `$CLI notes add live -p 0 -c Bass …`, `$CLI render live out.wav`.
  - `live` liest den aktuellen Stand aus dem Fenster, **inkl. ungespeicherter Änderungen**, und schreibt direkt dorthin zurück.
    Der User sieht/hört es sofort; jeder Befehl ist in TStudio **ein Strg+Z-Schritt**.
  - Die Datei auf der Platte bleibt unverändert, das Projekt ist danach „ungespeichert“ (•). **Nicht selbst speichern**, außer der
    User will es: `$CLI save` (wie Strg+S) bzw. `$CLI save --as E:/…/name.tsp`, wenn das Projekt noch keine Datei hat.
  - Ein Pfad zur Datei, die gerade offen ist, wird automatisch auch ans Fenster geleitet – es gibt nie zwei Versionen.
- `"running": false` → mit Dateipfaden arbeiten; zum Anhören `$CLI open song.tsp` (startet TStudio).
- Neues Projekt für den User: `$CLI new E:/…/song.tsp --bpm 124` und dann `$CLI open E:/…/song.tsp` – lädt es ins laufende Fenster
  (bei ungespeicherten Änderungen fragt TStudio den User selbst). `new live` gibt es nicht.
- **Niemals `tstudio.exe` beenden** (kein `taskkill`): der User hat dort evtl. ungespeicherte Arbeit. Muss die GUI neu gebaut werden,
  den User bitten, zu speichern und TStudio zu schließen. `tstudio-cli` neu bauen geht jederzeit.

## Konventionen (wichtig)

- **Token sparen:** Ausgabe ist kompaktes JSON (eine Zeile; `--pretty` nur zum Lesen für Menschen). Viele Schritte in ein
  Skript bündeln statt einzeln aufrufen; große Ausgaben (`get`, `mix`, `analyze --chords`) mit `| grep`/`jq` auf die nötigen
  Felder kürzen. Beim Bauen nur Fehler zeigen: `cargo build --release 2>&1 | grep -E "^error" -A5`. Große Quelldateien
  nicht ganz lesen, sondern per `grep -n` die Stelle suchen und nur den Bereich lesen.

- Kanäle, Patterns, Mixer-Spuren: **0-basiert** (Mixer 0 = Master). Kanäle/Patterns auch per Name (`-c Bass`, `-p "Pattern 1"`).
- Playlist: `--track` und `--bar` sind **1-basiert** wie in der GUI (Takt 1 = Songanfang).
- Notenpositionen in **Beats** (Viertel), 0 = Pattern-Anfang; Brüche erlaubt (`1/4`, `3/2`). C4 = MIDI 60.
- Steps = 16tel. Ein Pattern ist automatisch so viele ganze Takte lang wie seine Noten (oder fest via `pattern add --bars`).

## Workflow

1. **Ansehen**: `$CLI status`, dann `$CLI info live` (bzw. `info song.tsp`) (Kanäle, Patterns, Clips, Mixer mit Indizes).
2. **Ändern** mit den Befehlen unten.
3. **Prüfen**: `$CLI render live out.wav` (oder `-p 0 --loops 2` für ein Pattern) und die `stats` lesen:
   `peak_db` (> -0.1 oder `clipped_samples` > 0 = übersteuert), `rms_db`, `rms_db_per_bar` (stille Takte ≈ < -60),
   `detected_bpm`, `key`, `onsets`. So verifizierst du, ohne zuzuhören. Renders in ein Temp-/Scratch-Verzeichnis legen,
   nicht in den Projektordner, außer der User will den Export.
4. Dem User kurz sagen, was sich geändert hat – er kann es sofort in TStudio anhören (und mit Strg+Z zurücknehmen).

## Befehle

`song.tsp` steht für das Projekt-Argument: `live` (offenes Projekt) oder ein Dateipfad.

```bash
$CLI status                                                  # läuft TStudio? welches Projekt, ungespeichert?
$CLI save [--as E:/…/name.tsp]                               # offenes Projekt speichern (nur auf Wunsch des Users)
$CLI new song.tsp --bpm 140 [--empty] [--name X] [--force]   # neu (Standard: 1 TSynth + Pattern 1)
$CLI new song.tsp --template "Schranz 155" [--bpm 150]      # aus Vorlage: mitgeliefert (House 124, Techno 140, Schranz 155) oder eigene
#   (eigene = in der App „Datei → Als Vorlage speichern…“); Tempo der Vorlage, außer --bpm ist angegeben
$CLI info song.tsp
$CLI presets                                                 # Synth-Presets, Effekt-Typen + Default-Parameter
$CLI set song.tsp swing=0.25 mixer.0.volume=0.9              # Swing 0..1 (jede 2. 16tel später), Master-Lautstärke

# Kanäle
$CLI channel add song.tsp --synth --preset Bass --name Bass  # TSynth-Presets nach Kategorie: `presets` → synth_preset_categories
#   Kick: "Hard Kick", "Schranz Kick", "Hardtekk Kick", "Fat Kick", "Tekk Bass Kick" (Taste = Ton), "Rumble Kick", "Industrial Kick", "Punch Kick"
#   Bass: Bass, "808 Sub", "Offbeat Bass", "Hardtekk Bass", "Reese Bass" · Lead: Lead, "Hoover Lead", "Rave Lead"
#   Stab: Pluck, "Hard Stab", "Schranz Stab", "Industrial Stab", "Chord Stab" · Pad: Pad, "Dark Pad"
#   Acid: Acid, "Acid Ladder", "Acid Squelch" · FX: Riser, "Laser Zap", Siren · Init
$CLI channel kick song.tsp Kick --tune F#1                    # TSynth-Kick-Engine an + Grundton stimmen (Notenname oder MIDI-Zahl), --off = aus
$CLI channel mod song.tsp Lead --source Lfo1 --dest Cutoff1 --amount 0.4   # Mod-Matrix-Routing (16 Slots); --clear leert; ohne Optionen: anzeigen
$CLI soundfont datei.sf2                                     # Presets einer SoundFont (bank:program name)
$CLI channel add song.tsp --soundfont datei.sf2 [--preset "Strings" | --program 48 --bank 0]   # Bank 128 = Drum-Kit
$CLI channel preset song.tsp Strings "Choir"                  # SoundFont-Kanal: anderes Preset (Name, Teil, oder 0:52)
$CLI channel add song.tsp --sample E:/Samples/kick.wav --name Kick [--root C4]
$CLI channel add song.tsp --sample loop.wav --stretch stretch [--sample-bpm 92]   # Loop ans Projekttempo anpassen
$CLI channel preset song.tsp Bass Pluck
$CLI channel remove song.tsp 2

# Patterns + Noten
$CLI pattern add song.tsp --name Drop [--bars 4]
$CLI notes steps song.tsp -p 0 -c Kick  "x...x...x...x..."       # x=0.8 X=1.0 o=0.45 .=Pause; |/Leerzeichen ignoriert
$CLI notes steps song.tsp -p 0 -c Hat   "..x...x...x...x." --keep --offset 4   # --offset in Beats, --keep = nicht ersetzen
$CLI notes add  song.tsp -p 0 -c Bass  "A1@0+0.5 A1@0.75+0.25 C2@2+1*0.9"   # KEY@START[+LEN][*VEL]
$CLI notes add  song.tsp -p 0 -c Pad   "A3|C4|E4@0+4 F3|A3|C4@4+4" --replace # Akkorde mit |
$CLI notes chords song.tsp -p 0 -c Bass "G#:1 A#:1 Cm:2" --style offbeat   # Akkordfolge → Noten (Länge je Akkord in Takten)
#   Stile: block|pad (gehalten, mit Umkehrungen), stab, arp, bass (Viertel), offbeat (Tekk-Bass), rolling; --octave, --rhythm "x.x-..x." (16tel/Takt)
#   Progression aus einem Sample übernehmen: `analyze sample.wav --chords` → Feld `progression` direkt einsetzen
$CLI notes list song.tsp -p 0 [-c Bass]
$CLI notes clear song.tsp -p 0 [-c Bass]

# Playlist
$CLI clip add song.tsp --pattern 0 --track 1 --bar 1 --repeat 8
$CLI clip add song.tsp --pattern 0 --track 1 --bar 1 --bars 16     # EIN Clip, länger als das Pattern → Pattern wiederholt sich darin
$CLI clip add song.tsp --audio vocal.wav --track 3 --bar 5 [--stretch stretch|resample --sample-bpm 120] [--bars 4]
$CLI clip add song.tsp --audio vox.wav --bar 9 --pitch 3 --formant   # Pitch in jedem Tempo-Modus, --formant = Stimme bleibt natürlich
$CLI clip remove song.tsp 12 13   |  $CLI clip remove song.tsp --all

# Song-Struktur: Marker statt Taktnummern (überall bei --from/--to/--at nutzbar; erscheinen im GUI im Playlist-Lineal)
$CLI marker add song.tsp --bar 1 Intro | marker add song.tsp --bar 33 Drop | marker list song.tsp | marker remove song.tsp Drop
$CLI section copy song.tsp --from Drop --bars 16 --to 65 [--tracks 1,2]   # Clips kopieren (ohne --bars: bis zum nächsten Marker)
$CLI section insert song.tsp --at Break --bars 8     # leere Takte einfügen, alles danach (inkl. Marker) rückt nach hinten
$CLI section delete song.tsp --from 49 --bars 8      # Takte herausschneiden, Rest rückt nach vorn
$CLI section clear song.tsp --from Drop --bars 4 [--tracks 3]

# Tempo-Spur (im GUI: Spur „Tempo“ unter dem Lineal) – Takt 1 = Starttempo (`bpm`)
$CLI tempo add song.tsp --bar 33 --bpm 150 [--ramp]   # ab Takt 33 150 BPM; --ramp = vom vorigen Punkt aus gleitend
$CLI tempo list song.tsp   |  $CLI tempo remove song.tsp --bar 33  |  $CLI tempo remove song.tsp --all
# Audio-Clips mit --stretch folgen dem Tempo am Clip-Anfang; ändert sich das Tempo innerhalb des Clips,
# läuft er per Varispeed synchron mit (Tonhöhe ändert sich dabei leicht). Clips ohne Stretch spielen in Echtzeit.

# Taktartwechsel (GUI: Rechtsklick in die Tempo-Spur → „Taktart ab Takt N“); alle --bar-Angaben folgen den Wechseln
$CLI meter add song.tsp --bar 9 7/8 | meter add song.tsp --bar 10 4/4 | meter list song.tsp | meter remove song.tsp --bar 9
# Einzählen bei Aufnahmen und Metronom folgen Taktart- und Tempo-Spur (1 Takt 7/8 = 7 Achtel-Klicks)

# MIDI-Dateien (Standard MIDI File)
$CLI midi export song.tsp out.mid [--pattern 2]      # ohne --pattern: ganzer Song inkl. Tempo- und Taktartwechseln (Rampen in Viertel-Schritten)
$CLI midi import song.tsp in.mid [--name X] [--keep-tempo]   # neues Pattern, ein TSynth-Kanal pro MIDI-Spur (Kanal 10 = Drums), Tempo aus der Datei

# Mixer / Effekte (Kanal → mixer_track aus `info`)
$CLI fx add song.tsp --track 2 reverb      # eq compressor reverb delay distortion chorus filter limiter sidechain autotune clipper multiband ott width
$CLI fx add song.tsp --track 4 width       # Stereobreite (Alias stereo/imager): Width {width 0–2 (1 = unverändert), mid_db, side_db, bass_mono_hz (0 = aus)}
$CLI fx chain song.tsp --track 1 schranz-kick [--replace]     # fertige Ketten: schranz-kick, master, ott, vocal, bass (--source Kick), rumble (--source Kick), wide (140 %, Bass <150 Hz mono), mono-bass (<120 Hz mono)
$CLI fx chain song.tsp --track 0 master --replace            # Master: EQ → Multiband → Clipper → Limiter (-1 dBTP)
$CLI mixer rumble song.tsp --kick 1 [--amount 0.7]           # Rumble-Bus komplett: neue Spur + Send + Reverb/Verzerrung/Tiefpass/Sidechain
$CLI fx add song.tsp --track 3 sidechain --source 1   # Spur 3 duckt, wenn Spur 1 (Kick) spielt
# Autotune: key 0=C … 11=B, scale Chromatic|Major|Minor|HarmonicMinor|Phrygian|MinorPentatonic|MajorPentatonic,
# retune_ms 0 = hart (T-Pain), 50–150 = natürlich; z.B. set song.tsp 'mixer.5.fx.0.params.Autotune.key=3' 'mixer.5.fx.0.params.Autotune.retune_ms=0'
# Vocal-Aufnahmen (🎤 im GUI) landen auf dem Insert „Vocals“ und in <Sample-Ordner>/Recordings/
$CLI fx remove song.tsp --track 2 0
$CLI mixer send song.tsp --from 1 --to 10 --amount 0.5   # Post-Fader-Send auf höhere Spur (amount 0 = entfernen)
$CLI mixer route song.tsp --from 2 --to 12               # Ausgang auf Bus 12 statt Master (--to 0 = Master)
$CLI mixer add song.tsp [--name Bus] [--at 5]           # neuer Insert (mit --at eingefügt, Verweise rücken mit)
$CLI mixer delete song.tsp 7                          # Insert löschen (Kanäle darauf → Master)
$CLI mixer move song.tsp --from 7 --to 3 | mixer rename song.tsp 3 "Drums"
$CLI mixer mute song.tsp 3 [--off] | mixer solo song.tsp 3 [--off|--only] | mixer solo song.tsp --clear
$CLI track list song.tsp                              # Playlist-Spuren (1-basiert) mit Mute/Solo/Clip-Anzahl
$CLI track add song.tsp [--name FX] [--at 3]          # leere Spur (mit --at davor eingefügt)
$CLI track delete song.tsp 4                          # Spur samt Clips löschen
$CLI track move song.tsp --from 5 --to 1 | track duplicate song.tsp 2 | track rename song.tsp 1 "Kick"
$CLI track volume song.tsp 2 70                       # Spur-Lautstärke in % (Samples + Pattern-Noten der Spur)
$CLI track mute song.tsp 2 [--off] | track solo song.tsp 1 [--off|--only] | track solo song.tsp --clear

# Automation (nur Song-Modus; Wert bleibt nach dem Clip stehen)
# Audio-Clips, die länger als ihr Sample sind, loopen das Sample. Zum Automatisieren eines Samples:
# Clip auf einen eigenen Mixer-Insert legen (mixer_track ≠ 0), dann mixer.N.volume / mixer.N.fx.S.cutoff automatisieren.
$CLI auto targets song.tsp [--filter cutoff]             # alle Ziele mit Bereich
$CLI auto add song.tsp --target mixer.3.fx.0.cutoff --track 6 --bar 9 --bars 8 --from 200 --to 12000
$CLI auto add song.tsp --target channel.2.synth.drive_db --points "0:0,4:18,8:6" --bar 17 --bars 2   # BEAT:WERT

# Jeder Parameter: get/set über JSON-Pfade
$CLI get song.tsp channels.0.instrument
$CLI set song.tsp bpm=128 "channels.0.instrument.Synth.cutoff=650" "mixer.2.fx.0.params.Reverb.mix=0.35"

# Rendern / Audio
$CLI render song.tsp out.wav [-p 0 --loops 4] [--bits 16|24|32] [--normalize] [--tail 2]   # .mp3 braucht ffmpeg
$CLI render live out.mp4 [--artist NAME] [--title T] [--accent orange|rot|pink|lila|cyan|grün|#RRGGBB] [--size 1080p|720p|vertical|square] [--fps 30] [--no-watermark]   # Video mit Visualizer (ffmpeg, nur CPU, ~Songlänge)
$CLI render live vorschau.png --still 60 [--artist …]   # ein Visualizer-Standbild ansehen (Read-Tool), bevor das ganze Video gerendert wird
$CLI analyze file.wav [--bpm 128]          # Tonart (+ key_confidence); kurze One-Shots: `fundamental` (Grundton der Kick)
$CLI analyze song.wav --grid [--bpm 144]   # Beat-Grid: bpm (0,001 genau), first_beat_s, downbeat_s („die 1“), bar_starts_s, onset_deviation_ms
$CLI analyze song.wav --chords [--chord-beats 4]   # Akkord-Zeitleiste ab dem Downbeat + `progression` für `notes chords`
$CLI analyze master.wav --mix              # LUFS, True Peak, PLR, Frequenzbänder, Mono-Bass, Warnungen
$CLI mix song.tsp [--reference ref.wav] [--master-only]   # Mix-Report: Master + jede Mixer-Spur einzeln + Maskierung (wer verdeckt wen) + Warnungen
$CLI compare mein.wav referenz.wav         # Lautheit/Bänder/Dichte/Breite gegen einen echten Release, mit konkreten Vorschlägen
$CLI render song.tsp out.wav --stems stems/   # zusätzlich jede Mixer-Spur einzeln (ohne Master-Effekte, Busse inklusive)
#   Stems folgen Modus/Tail des Renders und der Tempo-Spur (gleiche Länge wie der Mix); Dateien "NN Name.wav"; in der App: Export → „Stems exportieren“
$CLI stretch in.wav out.wav --from 92 --to 140 [--mode stretch|resample|off] [--pitch -2] [--formant]
$CLI trim song.wav hook.wav --from-bar 25 --bars 8 [--bpm 144] [--downbeat 0.067] [--to-bpm 181.43 --mode resample] [--pitch 2 --formant]
#   Schnitt im Beat-Raster (Grid wird erkannt, sonst --bpm/--downbeat aus `analyze --grid`); --from-beat/--beats für Beats statt Takte
$CLI tune kick.wav kick_F.wav --to F [--mode resample|pitch]   # Kick/808 auf die Tonart stimmen (Grundton vorher/nachher im Ergebnis)
$CLI stems song.mp3 [--out dir] [--threads 2]   # Demucs: Drums/Bass/Vocals/Rest (nur CPU, niedrige Priorität)
$CLI trim in.mp3 out.wav --start 12.5 --end 16.25 [--normalize]
$CLI trim in.mp3 kick.wav --start 30.1 --end 30.4 --preset "Kick / Sub" --normalize --trim-tail   # isoliert schneiden
$CLI trim song.mp3 hats.wav --hp 6000 --slope 96          # oder frei: --lp HZ --band "FREQ:DB[:Q]" --stereo mid|side --gain-db
$CLI trim song.mp3 instrumental.wav --stereo side          # ganze Datei: Mitte (Vocals) entfernen; Presets: `presets` → isolate_presets
$CLI chop loop.wav [--even 8 | --sensitivity 0.6] [--out-dir slices/]
$CLI scan E:/Samples [--filter kick] [--analyze]
$CLI fetch "<url>" [--format wav|mp3] [--out dir]           # yt-dlp + ffmpeg nötig; nur Material, das der User nutzen darf
$CLI tools
```

**Sicherheit:** Ein Dateipfad, der in TStudio gerade offen ist, wird automatisch ins Fenster umgeleitet (Antwort enthält dann
`routed_to_window: true`). Für Experimente an einer *Kopie* die Datei vorher kopieren oder `--no-live` anhängen (bricht ab, statt
das offene Projekt zu ändern). `new` verweigert Dateien, die in TStudio offen sind.

## Datenmodell für `set` (Auszug – vollständig: `get` / `presets`)

- Projekt: `bpm`, `beats_per_bar`, `name`
- Kanal `channels.N`: `name`, `volume` (0–1.25, Standard 0.8), `pan` (-1..1), `muted`, `mixer_track`
- TSynth `channels.N.instrument.Synth.…` (alle neuen Felder sind standardmäßig neutral; alte Projekte klingen unverändert):
  - Oszillatoren `osc1`/`osc2`/`osc3` {`enabled`, `table` ("Basic"=klassisch über `wave`|"Morph"|"Pulse"|"Harmonics"|"Vowel"|"SyncSweep"|"Fold"|"Digital"|"Fm"|"DriveSine"|"Hollow"),
    `wave` ("Sine"|"Saw"|"Square"|"Triangle"), `position` (0–1 im Wavetable), `warp` ("Off"|"BendPlus"|"BendMinus"|"Sync"|"Pwm"|"Quantize"|"FmOsc1"|"FmOsc2"|"FmOsc3"|"FmSub"),
    `warp_amount` (0–1), `octave`, `semi`, `fine` (Cent), `level`, `pan`, `unison` (1–9), `detune` (Cent), `blend` (Seitenstimmen 0–1), `spread` (Stereo 0–1),
    `phase`, `rand_phase` (0 = jede Note gleich)}. FM-Warp: Modulator-Osc mit `level` 0 einschalten.
  - `sub_level`, `sub_wave`, `sub_octave` (rel. zu Osc 1, Standard -1), `noise_level`, `noise_type` ("White"|"Crackle"|"Digital"), `noise_tone` (Hz)
  - **Kick-Engine** `kick` {`enabled`, `tune` (MIDI-Note des Körpers, 29 = F1), `key_follow` (Taste transponiert rel. C4), `pitch_start` (Halbtöne über Grundton),
    `pitch_time` (s bis zum Grundton), `pitch_curve` (-1..1, >0 = Punch), `click_level`, `click_type` ("Noise"|"Tick"|"Snap"), `click_tone` (Hz), `click_decay` (s),
    `body_hold`/`body_decay` (s, 0 = nur Amp-Env), `drive_db`, `drive_mode` ("Soft"|"Hard"|"Fold"|"Asym"), `rumble_level`, `rumble_delay`, `rumble_decay`, `rumble_tone` (Hz)}.
    Mit Amp-Sustain 0 ist die Kick ein One-Shot (Notenlänge egal), jede Taste spielt den gestimmten Grundton.
    **Fette Kick mit Bass:** Bei `body_decay` 0 formt die Amp-Hüllkurve den Körper VOR dem Drive – `drive_db` (14–22 dB) hält den
    Bass dann lang oben, `amp_env.hold`/`decay` bestimmen die Länge (Hold 0.04–0.1 s, Decay 0.6–0.8 s bei 150–180 BPM).
    OTT und Hochpass um 30 Hz kosten Bass – bei Kicks sparsam; am Ende `fx.clip_on`.
  - Filter 1: `filter_enabled`, `filter_type` ("Svf12"|"Svf24"|"Ladder"|"Comb"|"Formant"), `filter_mode` ("LowPass"|"HighPass"|"BandPass"|"Notch"), `cutoff` (Hz; Formant: Vokal),
    `resonance` (0–1), `filter_drive` (dB), `filter_env_amount` (Oktaven), `key_track`; Filter 2: `filter2` {`enabled`, `kind`, `mode`, `cutoff`, `resonance`, `drive_db`,
    `env_amount`, `key_track`}, `filter_routing` ("Serial"|"Parallel"); danach `drive_db` + `drive_mode`
  - Hüllkurven `amp_env` (Env 1), `filter_env` (Env 2), `mod_env` (Env 3) {`attack`,`hold`,`decay`,`sustain`,`release` (s), `attack_curve`, `decay_curve` (-1..1)},
    klassisch `pitch_env_amount`/`pitch_env_decay`
  - LFOs `lfo1`…`lfo4` {`shape` ("Wave"|"RampDown"|"SampleHold"|"Custom"), `wave`, `rate` (Hz), `sync` + `beats` (0.25 = 1/16), `retrig`, `phase`,
    `custom` [16 Werte -1..1], `smooth`, direktes Ziel `target` ("Off"|"Pitch"|"Cutoff"|"Amp"|"Pan") + `amount`}
  - Mod-Matrix `mod_matrix.0`…`.15` = {"source": …, "dest": …, "amount": -1..1} (lieber `channel mod`). Quellen: AmpEnv, FilterEnv, ModEnv, Lfo1–4, Velocity,
    Note, Random, Macro1–4, KickSweep. Ziele: Pitch, Osc1–3Pitch/Pos/Warp/Level, Detune, SubLevel, NoiseLevel, Cutoff1/2, Reso1/2, FilterDrive, Amp, Pan,
    Lfo1–4Rate, KickPitch, KickDrive, ClickLevel; nur mit Makro-Quelle: FxDistDrive, FxReverbMix, FxDelayMix, FxChorusMix
  - Makros `macro1`…`macro4` (0–1, automatisierbar), Voicing `mono`, `legato` (aus = jede Note neu), `glide`, `slide_only` (Glide nur bei überlappenden Noten = 303-Slide; Acid-Noten mit Länge > Abstand binden), `polyphony` (1–16), `gain`, `velocity_sens`
  - Synth-FX `fx` {`dist_on`/`dist`, `comp_on`/`comp`, `ott_on`/`ott`, `chorus_on`/`chorus`, `delay_on`/`delay` (tempo-synchron), `reverb_on`/`reverb`,
    `clip_on`/`clip`} – Reihenfolge fest: Dist → Comp → OTT → Chorus → Delay → Reverb → Clipper; Parameter wie die Mixer-Effekte
  - Automatisierbar (`auto add --target channel.N.synth.FELD`): siehe `presets` → `synth_automatable` (u.a. cutoff, macro1–4, osc1_position, osc1_warp,
    filter2_cutoff, kick_tune, kick_pitch_start, kick_drive_db, kick_rumble, fx_dist_drive_db, fx_reverb_mix)
- Sampler `channels.N.instrument.Sampler.…`: `sample` (Pfad), `params` {`root_note` (MIDI), `start`/`end` (0–1), `reverse`, `looped`, `loop_start`/`loop_end` (0–1, Loop-Bereich zum Halten), `loop_xfade` (0–0,5), `loop_tail` (nach Loslassen ausklingen),
  `one_shot`, `pitch` (Halbtöne), `gain`, `amp_env`, `mode` ("Normal"|"Chop"), `slices` (Bruchteile 0–1, ab `slice_root` je Taste), `mono`},
  `stretch` {`mode` ("Off"|"Stretch"|"Resample"), `source_bpm`, `target_bpm` (null = Projekttempo), `pitch` (jeder Modus), `formant`}
- Audio-Clip `playlist.clips.N.kind.Audio.…`: `path`, `gain`, `mixer_track`, `stretch` {wie oben}, `fade_in`, `fade_out`
- Marker `playlist.markers` [{`tick`, `name`}] (lieber per `marker add`)
- SoundFont `channels.N.instrument.SoundFont.…`: `path`, `bank`, `program`, `preset_name`, `gain`
- Mixer `mixer.N`: `volume`, `pan`, `muted`, `solo`, `output`, `sends`, `fx.S.enabled`, `fx.S.params.<Typ>.<feld>`
  (Felder je Typ: siehe `presets`, z.B. Reverb {size, damping, mix, predelay_ms, width}, Delay {time_beats, feedback, mix, ping_pong, tone},
  Compressor {threshold, ratio, attack_ms, release_ms, knee, makeup, mix}, Filter {mode, cutoff, resonance, mix},
  Clipper {drive_db, ceiling_db, softness, output_db, mix}, Multiband {low_freq, high_freq, low/mid/high_drive_db, low/mid/high_gain_db, mix},
  OTT {depth, time, upward, downward, threshold_db, in_gain_db, out_gain_db, low/mid/high_gain_db})

## Tipps

- Drums: einen Sampler-Kanal pro Sound (Kick, Snare, Hat …), Patterns per `notes steps`. Ohne Samples: "808 Sub"-Preset als Kick-Ersatz
  (kurze tiefe Noten, z.B. C1/C2).
- Chop: `chop loop.wav --even 8` → `fractions` als `channels.N.instrument.Sampler.params.slices=[…]` setzen und `params.mode="Chop"`;
  Slice i liegt dann auf Taste `slice_root + i` (Standard C4 = 60).
  Kürzer: `channel slice song.tsp Break --even 16 --root C2` bzw. `--sensitivity 0.5` (Transienten) setzt Chop-Modus,
  `slices` und `slice_root` direkt; danach `notes steps … --key C2` (Slice 1), `C#2` (Slice 2) … In der App: Drum Pads (F11).
- **Schranz / Hard Techno** (150–160 BPM): Kick = TSynth-Preset "Schranz Kick"/"Hardtekk Kick"/"Industrial Kick" (beliebige Taste; per
  `channel kick … --tune <Grundton der Tonart>` stimmen, prüfen mit `render` + `analyze` → `fundamental`) oder Kick-Sample;
  "Rumble Kick" bringt einen eigenen Rumble mit, alternativ:
  Rumble = Kick-Spur per `mixer send` auf einen Bus mit reverb (mix 1.0) → distortion → filter (LowPass ~200 Hz) → sidechain (`--source` Kick-Spur);
  Offbeat-Bass = "Offbeat Bass"/"Hardtekk Bass" (Noten um F2–A2, Presets liegen eine Oktave tiefer); Acid-Line = "Acid Ladder"
  (Velocity = Accent, Makro 1 = Cutoff, Makro 2 = Distortion) mit `auto add … --target channel.N.synth.macro1`; Stabs = "Schranz Stab"/"Hard Stab"; Hats/Percussion offbeat; Swing 0–10 %.
  Filter-Fahrten (Breaks/Build-ups) mit `auto add … --target mixer.N.fx.S.cutoff`.
- SoundFonts des Users: `$CLI scan "$USERPROFILE/Documents" --filter sf2` findet .sf2-Dateien (z.B. in Documents/lmms/samples/soundfonts).
- Bei Pegelproblemen erst Kanal-`volume` senken, dann Kompressor/Limiter.
- **Sample-Workflow** (besser als nach Gefühl): `analyze song.wav --grid --chords` → Downbeat + Akkorde → `trim … --from-bar N --bars 8
  --to-bpm <Projekt> --mode resample` (exakt auf dem Raster) → `notes chords … "<progression>" --style offbeat` für einen Bass, der
  den Akkorden folgt → Kick per `analyze kick.wav` (fundamental) und `tune --to <Grundton der Tonart>` stimmen.
- **Mix prüfen** statt raten: `mix live` nach jeder größeren Änderung; Warnungen zu Maskierung (z.B. Kick ↔ Bass), Mono-Bass,
  LUFS und Bändern abarbeiten. Mit `--reference` gegen einen echten Hardtekk/Schranz-Release vergleichen (die Band-Richtwerte
  ohne Referenz sind grob). Ziel Hardtekk/Schranz: etwa -8 bis -5 LUFS, True Peak ≤ -1 dBTP.
- Für Samples aus der Library des Users: `scan <Ordner> --filter …` und absolute Pfade verwenden.
