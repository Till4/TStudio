//! tstudio-cli – headless control of TStudio projects (.tsp).
//!
//! Every command prints JSON on stdout (errors: `{"ok":false,"error":…}` + exit code 1),
//! so scripts and AI assistants can drive the DAW. Project argument `live` (or a path to the
//! file open in TStudio) edits the project open in the running TStudio window directly.

mod args;
mod audio;
mod cmd;
mod helpers;
mod live;
mod spec;
mod tools;

use clap::{Args, Parser, Subcommand};
#[allow(unused_imports)]
use args::*;
#[allow(unused_imports)]
use cmd::*;
#[allow(unused_imports)]
use helpers::*;
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
