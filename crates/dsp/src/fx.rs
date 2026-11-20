//! Built-in mixer effects. Processors are allocated up front (`Effect::new`) and
//! never allocate in `process`.

use crate::autotune::{AutotuneFx, AutotuneParams};
pub use crate::fx_extra::{ClipperFx, ClipperParams, MultibandFx, MultibandParams, OttFx, OttParams};
pub use crate::fx_width::{WidthFx, WidthParams};
use crate::filter::{Biquad, FilterMode, OnePole, Svf};
use crate::{db_to_gain, gain_to_db};
use serde::{Deserialize, Serialize};

mod params;
mod processors;
#[cfg(test)]
mod tests;

pub use params::*;
pub use processors::*;
