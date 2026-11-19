//! Project → engine snapshot, with samples from the async UI cache.

use crate::samples::SampleCache;
use tstudio_engine::{compile_project, Snapshot};
use tstudio_project::Project;

pub fn compile(p: &Project, cache: &mut SampleCache, for_export: bool) -> Snapshot {
    compile_project(p, &mut |path, st, bpm| cache.get_prepared(path, st, bpm).map(|l| l.data), for_export)
}
