//! `midi` sub-commands: Standard MIDI File export/import.

use super::*;

#[derive(Subcommand)]
pub(crate) enum MidiCmd {
    /// Pattern oder ganzen Song (Playlist) als .mid exportieren (Song: mit Tempo- und Taktartwechseln)
    Export {
        project: PathBuf,
        /// Ziel-Datei (.mid)
        out: PathBuf,
        /// nur dieses Pattern (Index oder Name); ohne = ganze Playlist
        #[arg(long, short)]
        pattern: Option<String>,
    },
    /// .mid als neues Pattern importieren (ein Kanal pro MIDI-Spur, Tempo aus der Datei)
    Import {
        project: PathBuf,
        file: PathBuf,
        /// Name des neuen Patterns (Standard: Dateiname)
        #[arg(long)]
        name: Option<String>,
        /// Projekttempo nicht aus der Datei übernehmen
        #[arg(long)]
        keep_tempo: bool,
    },
}
