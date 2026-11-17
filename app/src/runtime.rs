//! One shared monotone clock for the whole program.
//!
//! Every timestamp in a session comes from here: the probe engine stamps its results
//! with [`secs_since_start`], and the chart asks egui for the frame time, which eframe
//! fills from the same origin ([`secs_since_start`] at startup). Keeping both on one
//! axis is what makes "now" in the view line up with "now" in the history - a second
//! clock (a plain `Instant` inside the engine) would drift away from the frame time as
//! soon as the process spent time anywhere else than in the loop.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Seconds since the process started, for the chart's "now".
pub fn secs_since_start() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}
