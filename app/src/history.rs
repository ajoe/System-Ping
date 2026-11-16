//! Latency history: one sample per probe, kept in a ring buffer per target.

use serde::{Deserialize, Serialize};

/// Why a probe did not answer. Mirrors [`crate::ping::ProbeFail`]; kept as its own
/// serialisable type so saved history never depends on the ping module's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fail {
    /// The host name did not resolve.
    Resolve,
    /// No reply within the timeout.
    Timeout,
    /// The OS ping could not be run or said something unexpected.
    Error,
}

impl Fail {
    pub fn short(self) -> &'static str {
        match self {
            Fail::Resolve => "DNS",
            Fail::Timeout => "timeout",
            Fail::Error => "Fehler",
        }
    }
}

/// One measurement of one target.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Sample {
    /// Milliseconds since the app started (drives the chart's x axis).
    pub t_secs: f64,
    /// Round-trip time in ms, or `TIMEOUT` when the target did not answer.
    pub rtt_ms: f32,
    /// Why it failed, when it did (`rtt_ms == TIMEOUT`). Lets the chart show a
    /// DNS failure differently from a silent host.
    #[serde(default)]
    pub fail: Option<Fail>,
    /// Absolute wall-clock time of the probe as Unix milliseconds. `t_secs` is only valid
    /// inside one run; this is what survives into the saved file and the CSV export, so a
    /// history that spans restarts still reads as real time.
    #[serde(default)]
    pub at_ms: i64,
}

pub const TIMEOUT: f32 = -1.0;

/// Replies above `avg + 3 * stddev`; the chart marks exactly these samples.
pub const OUTLIER_SIGMA: f64 = 3.0;

pub fn is_ok(rtt: f32) -> bool {
    rtt >= 0.0
}

/// The value at fraction `f` of a sorted list (nearest-rank, no interpolation).
pub fn pick(sorted: &[f32], f: f64) -> f32 {
    let idx = ((sorted.len() as f64 - 1.0) * f).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// The outlier cut for a series: mean plus `OUTLIER_SIGMA` standard deviations of
/// the replies, and how many samples sit above it. Losses are not outliers — they
/// have no latency to be an outlier of.
pub fn outlier_cut(samples: &[Sample]) -> (f64, usize) {
    let mut n = 0usize;
    let mut sum = 0.0f64;
    for s in samples {
        if is_ok(s.rtt_ms) {
            n += 1;
            sum += s.rtt_ms as f64;
        }
    }
    if n < 4 {
        return (f64::INFINITY, 0);
    }
    let avg = sum / n as f64;
    let mut var = 0.0f64;
    for s in samples {
        if is_ok(s.rtt_ms) {
            let d = s.rtt_ms as f64 - avg;
            var += d * d;
        }
    }
    var /= n as f64;
    let cut = avg + OUTLIER_SIGMA * var.sqrt();
    let count = samples
        .iter()
        .filter(|s| is_ok(s.rtt_ms) && s.rtt_ms as f64 > cut)
        .count();
    (cut, count)
}

/// Ring buffer of samples for one target, oldest first.
#[derive(Clone, Debug)]
pub struct Series {
    cap: usize,
    data: Vec<Sample>,
}

impl Series {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(16),
            data: Vec::new(),
        }
    }

    pub fn push(&mut self, s: Sample) {
        if self.data.len() >= self.cap {
            // Drop a quarter at a time so the window doesn't shift every frame.
            let cut = self.cap / 4;
            self.data.drain(0..cut);
        }
        self.data.push(s);
    }

    /// Record one probe, keeping the failure reason with the sample. `at_ms` is the absolute
    /// time of the measurement; see [`Sample::at_ms`].
    pub fn record(&mut self, t_secs: f64, at_ms: i64, rtt_ms: f32, fail: Option<Fail>) {
        self.push(Sample {
            t_secs,
            rtt_ms,
            fail,
            at_ms,
        });
    }

    /// Re-anchors the chart axis of samples restored from disk. Their `at_ms` are real, but
    /// `t_secs` belongs to a session that has ended; the series is laid out again backwards
    /// from its newest stamp so the x axis stays continuous and monotone. The caller then
    /// keeps adding live samples on top of it.
    pub fn reanchor(&mut self) {
        let Some(end) = self.data.last().map(|s| s.at_ms) else {
            return;
        };
        for s in self.data.iter_mut() {
            s.t_secs = (end - s.at_ms) as f64 / 1000.0;
        }
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn set_cap(&mut self, cap: usize) {
        let cap = cap.max(16);
        self.cap = cap;
        if self.data.len() > cap {
            let extra = self.data.len() - cap;
            self.data.drain(0..extra);
        }
    }

    pub fn samples(&self) -> &[Sample] {
        &self.data
    }

    /// The newest sample, if there is one.
    pub fn last(&self) -> Option<Sample> {
        self.data.last().copied()
    }

    /// The outlier cut for this series (see [`outlier_cut`]).
    pub fn outlier_cut(&self) -> (f64, usize) {
        outlier_cut(&self.data)
    }

    /// Drop everything older than `horizon_secs` relative to the newest sample.
    pub fn trim_to_window(&mut self, horizon_secs: f64) {
        if let Some(last) = self.data.last() {
            let cutoff = last.t_secs - horizon_secs;
            let keep_from = self
                .data
                .iter()
                .position(|s| s.t_secs >= cutoff)
                .unwrap_or(self.data.len());
            if keep_from > 0 {
                self.data.drain(0..keep_from);
            }
        }
    }

    /// Statistics over the samples whose stamp falls inside `[t_min, t_max]`. This is what
    /// every panel shows, so all views agree on one definition. It runs once per target and
    /// frame, so the sort goes into the caller's scratch buffer instead of a fresh `Vec`.
    pub fn stats_window(&self, t_min: f64, t_max: f64, scratch: &mut Vec<f32>) -> Stats {
        let (cut, n_cut) = outlier_cut(&self.data);
        // `min` starts at MAX so the fold below can lower it; the empty-window case is
        // settled once, at the end, by `finish` — never left as a sentinel for the UI.
        let mut s = Stats {
            min: f32::MAX,
            ..Stats::default()
        };
        scratch.clear();
        for sample in self.data.iter() {
            if sample.t_secs < t_min || sample.t_secs > t_max {
                continue;
            }
            s.total += 1;
            if is_ok(sample.rtt_ms) {
                s.replies += 1;
                s.sum += sample.rtt_ms as f64;
                s.min = s.min.min(sample.rtt_ms);
                s.max = s.max.max(sample.rtt_ms);
                scratch.push(sample.rtt_ms);
            } else {
                s.losses += 1;
                if sample.fail == Some(Fail::Resolve) {
                    s.dns_fails += 1;
                }
            }
        }
        if !scratch.is_empty() {
            // A sort is needed for the median and the percentile anyway, so the average
            // comes from the same pass over the data as everything else.
            scratch.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = scratch.len();
            s.median = pick(scratch, 0.5);
            s.p95 = pick(scratch, 0.95);
            // The average of the same values that produced min/max, taken in sorted order so
            // two runs over identical data add them up the same way.
            s.avg = scratch.iter().fold(0f64, |a, v| a + *v as f64) / n as f64;
        }
        // A window with no reply has no minimum to report. Left alone it would show the
        // sentinel, so both empty cases land on 0 — same as `Stats::default`.
        if s.replies == 0 {
            s.min = 0.0;
            s.max = 0.0;
        }
        // Outliers are counted over the whole stored series, like the rings on the chart:
        // `outlier_cut` already knows how many replies sit above its cut.
        if cut.is_finite() {
            s.outliers = n_cut;
        }
        if s.total > 0 {
            s.loss_pct = 100.0 * s.losses as f64 / s.total as f64;
        }
        s
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Stats {
    pub total: usize,
    pub replies: usize,
    pub losses: usize,
    pub min: f32,
    pub max: f32,
    pub sum: f64,
    pub avg: f64,
    pub median: f32,
    pub p95: f32,
    pub loss_pct: f64,
    /// Replies above `avg + 3 * stddev`; the chart marks exactly these samples.
    pub outliers: usize,
    /// Losses that were DNS failures rather than silent timeouts.
    pub dns_fails: usize,
}
