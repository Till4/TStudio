//! TStudio audio engine.
//!
//! Threading model:
//! * The UI compiles the project into an immutable [`Snapshot`] and, when the
//!   structure changes, a [`Rack`] of stateful processors (allocated on the UI thread).
//! * Both travel to the audio thread through a lock-free `rtrb` queue as [`Command`]s.
//! * Replaced objects travel back through a garbage queue and are dropped on the UI thread,
//!   so the audio thread never allocates or frees.

pub mod audio;
pub mod compile;
pub mod core;
pub mod input;
pub mod render;
pub mod snapshot;
pub mod soundfont;

pub use audio::{list_output_devices, AudioConfig, EngineHandle};
pub use input::{list_input_devices, InputHandle, Take};
pub use core::{Command, EngineCore, Garbage, MonitorIn, PlayMode, SharedState, MAX_AUTO, MAX_CHUNK, MAX_TRACKS};
pub use soundfont::{load_soundfont, soundfont_presets, Sf2Preset};
pub use compile::{compile_project, SampleSource};
pub use render::render_offline;
pub use snapshot::{AutoClipSnap, AutoLane, ChannelSnap, ClipSnap, FxSnap, InstrumentSnap, NoteSnap, PatternSnap, Rack, Snapshot, TrackSnap};
pub use tstudio_project::PPQ;
