//! Timeline sub-commands: tempo track.

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
