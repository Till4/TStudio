//! Timeline sub-commands: tempo track and time-signature changes.

use super::*;

#[derive(Subcommand)]
pub(crate) enum TempoCmd {
    /// Tempowechsel setzen: ab --bar gilt --bpm (mit --ramp gleitend vom vorigen Punkt aus).
    /// Takt 1 ändert das Starttempo. Ein Punkt am gleichen Takt wird ersetzt.
    Add {
        project: PathBuf,
        /// Takt (1-basiert, Kommazahl erlaubt, z.B. 9 oder 16.5) oder Markername
        #[arg(long)]
        bar: String,
        #[arg(long)]
        bpm: f64,
        /// linear vom vorigen Tempo zu diesem hin gleiten (statt springen)
        #[arg(long)]
        ramp: bool,
    },
    /// Starttempo und alle Tempowechsel mit Takt und Zeit
    List { project: PathBuf },
    /// Tempowechsel am Takt entfernen (oder --all für alle)
    Remove {
        project: PathBuf,
        /// Takt (1-basiert) oder Markername
        #[arg(long, required_unless_present = "all")]
        bar: Option<String>,
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum MeterCmd {
    /// Taktart ab einem Takt setzen, z.B. `meter add song.tsp --bar 9 7/8` (gilt bis zum nächsten Wechsel).
    /// Takt 1 mit x/4 ändert die Grundtaktart (`beats_per_bar`). Ein Wechsel am gleichen Takt wird ersetzt.
    Add {
        project: PathBuf,
        /// ganzer Takt (1-basiert)
        #[arg(long)]
        bar: u32,
        /// Taktart ZÄHLER/NENNER, z.B. 4/4, 7/8, 3/4
        meter: String,
    },
    /// Grundtaktart und alle Wechsel (mit Start-Beat)
    List { project: PathBuf },
    /// Taktartwechsel am Takt entfernen (oder --all)
    Remove {
        project: PathBuf,
        #[arg(long, required_unless_present = "all")]
        bar: Option<u32>,
        #[arg(long)]
        all: bool,
    },
}
