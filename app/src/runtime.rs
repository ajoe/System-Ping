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

/// How much older than the process the session is treated as being.
static BACKDATE: OnceLock<f64> = OnceLock::new();

/// Makes the session look `secs` older than the process has been running. Only the picture
/// command uses this: it plants an example history spanning the whole chart window, and a process
/// of two seconds has no window of that age of its own - every tick left of its start would be
/// time before the app began, and the axis would say so.
///
/// Only the first call counts. Every timestamp of a session is measured from one origin, so the
/// offset must not move underneath the history that was stamped with it.
pub fn backdate(secs: f64) {
    let _ = BACKDATE.set(secs.max(0.0));
}

/// Seconds since the start of the session, for the chart's "now".
pub fn secs_since_start() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64() + BACKDATE.get().copied().unwrap_or(0.0)
}
