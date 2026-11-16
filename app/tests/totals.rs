//! The statistics totals row: median and p95 over all replies together, not as the mean of the
//! medians. Plus the helpers behind it (`App::stats_total`, `history::pick`).

use std::sync::Mutex;

use system_ping::app::{App, AppOptions};

/// The state folder is chosen through a process-wide variable (`SYSTEM_PING_STATE_DIR`) and an app
/// reads the file from it when it is created. Two tests in this binary would therefore get in each
/// other's way — one would see the other's targets. This guard keeps the folder together with its
/// app so it is clear who owns it right now.
static STATE_DIR_LOCK: Mutex<()> = Mutex::new(());

/// An app with its own empty state folder: no real user has anything taken from them, and the test
/// is guaranteed to start without stored targets. The lock is held until the test ends (`_guard`
/// therefore has to be the last field).
struct Fixture {
    app: App,
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::env::var_os("SYSTEM_PING_STATE_DIR").as_deref() == Some(self.dir.as_os_str()) {
            std::env::remove_var("SYSTEM_PING_STATE_DIR");
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An app with its own empty folder. The three built-in targets are handed in, but not as the
/// stored list: otherwise the app would read a file on creation that an earlier test had put into
/// left in the same folder.
fn fixture(tag: &str) -> Fixture {
    let guard = STATE_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("sp-totals-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);
    Fixture {
        app: App::with_theme_override(AppOptions::default(), Vec::new(), Some("dark")),
        dir,
        _guard: guard,
    }
}

/// Two targets with very different latencies, rewritten rather than newly created: `Target::new` is
/// deliberately not public. Without fresh series the content of an earlier
/// Durchlaufs in denselben Objekten.
fn two_targets(app: &mut App) {
    assert!(app.targets.len() >= 2, "the test needs two targets");
    for (i, (label, host)) in [("Schnell", "10.0.0.1"), ("Langsam", "10.0.0.2")]
        .into_iter()
        .enumerate()
    {
        let t = &mut app.targets[i];
        t.label = label.into();
        t.host = host.into();
        t.kind = system_ping::app::TargetKind::Lan;
        t.hidden = false;
        t.enabled = true;
        t.series = system_ping::history::Series::new(1024);
    }
    app.targets.truncate(2);
}

/// `n` replies one second apart, the last of them ending at `t_secs`. The timestamps have to fall
/// inside the time window — with 40 replies and the default 60 s window the oldest would otherwise
/// already be out again, and the test would have counted against a shorter list than it claims.
fn record(app: &mut App, idx: usize, last_t_secs: f64, n: i64, ms: f32) {
    for k in 0..n {
        let t = last_t_secs - (n - 1 - k) as f64;
        app.targets[idx]
            .series
            .record(t, (t * 1000.0) as i64, ms, None);
    }
}

/// The totals row over every target currently in the selection.
fn total(app: &App) -> system_ping::history::Stats {
    let mut s = Vec::new();
    let mut a = Vec::new();
    app.stats_total(&mut s, &mut a)
}

/// The median *of the medians* would be ~255 ms — a number no single measurement has. Correct
/// is ~10 ms, because the fast target contributes far more replies.
#[test]
fn median_und_p95_kommen_aus_allen_antworten() {
    let mut f = fixture("medians");
    two_targets(&mut f.app);
    assert!(
        f.app.shown(&f.app.targets[0]) && f.app.shown(&f.app.targets[1]),
        "both targets have to be in the selection, otherwise the total is wrong"
    );

    // 44 replies at 10 ms and one at 500 ms. Over all replies together, median and p95 (rank 42 of
    // 45) are still with the fast target; the average *of the two* medians would be 255 ms.
    record(&mut f.app, 0, 400.0, 44, 10.0);
    record(&mut f.app, 1, 400.0, 1, 500.0);

    let mut scratch = Vec::new();
    let mut all = Vec::new();
    let st = f.app.stats_total(&mut scratch, &mut all);

    assert_eq!(st.total, 45);
    assert_eq!(st.replies, 45, "all forty-four plus one reply are counted");
    assert!(
        st.median < 20.0,
        "the median comes from all replies, not as the mean of the medians: {}",
        st.median
    );
    assert!(
        st.p95 < 100.0,
        "p95 over all replies: {}, expected still with the fast target (median {}, min {}, max {})",
        st.p95,
        st.median,
        st.min,
        st.max
    );
    assert!(st.min < st.max);
    assert_eq!(st.max, 500.0);

    // A counter-proof so the test does not pass by accident: the same two targets, but this time
    // the slow one is now the majority — so median and p95 have to land there. The series is reset,
    // otherwise the 44 fast replies from above would still be in it.
    f.app.targets[0].series = system_ping::history::Series::new(1024);
    record(&mut f.app, 0, 900.0, 5, 10.0);
    record(&mut f.app, 1, 900.0, 40, 500.0);
    let inv = total(&f.app);
    assert_eq!(inv.replies, 45, "again exactly the two targets: {inv:?}");
    assert!(
        inv.p95 > 400.0,
        "with the slow target as the majority, p95 has to land there: {}",
        inv.p95
    );
    assert!(inv.median > 400.0);
}

/// A hidden target is missing from the totals row too — otherwise it adds up rows that
/// nobody can see. And a target without a reply must not pull the minimum of all targets to 0.
#[test]
fn ausgeblendete_ziele_zaehlen_nicht_mit() {
    let mut f = fixture("hidden-total");
    two_targets(&mut f.app);
    record(&mut f.app, 0, 400.0, 6, 10.0);
    record(&mut f.app, 1, 400.0, 6, 20.0);
    assert_eq!(total(&f.app).total, 12, "counting both targets together");

    f.app.targets[0].hidden = true;
    let after = total(&f.app);
    assert_eq!(after.total, 6, "the hidden target is missing");
    assert_eq!(after.min, 20.0, "and its latencies are missing too");
}

/// An outage has no latency; it must not pull `min` to 0, but it does count towards the loss.
#[test]
fn ein_ausfall_senkt_das_minimum_nicht() {
    let mut f = fixture("loss-min");
    two_targets(&mut f.app);
    record(&mut f.app, 0, 400.0, 4, 30.0);
    for k in 0..4i64 {
        let t = 400.0 - (3 - k) as f64;
        f.app.targets[1].series.record(
            t,
            (t * 1000.0) as i64,
            f32::NAN,
            Some(system_ping::history::Fail::Timeout),
        );
    }
    let st = total(&f.app);
    assert_eq!(st.min, 30.0, "an outage without latency must not lower min");
    assert!(st.loss_pct > 0.0, "but it is not lost from the loss rate");
}

/// "Clear measurements" empties the series but leaves the targets standing. The totals row must not
/// fake a zero then: without any reply there is no minimum, and the interface turns that edge value
/// into "—".
#[test]
fn ohne_messwerte_ist_nichts_gemessen() {
    let mut f = fixture("empty-total");
    two_targets(&mut f.app);
    record(&mut f.app, 0, 400.0, 6, 12.0);
    f.app.reset_stats();
    assert!(!f.app.targets.is_empty(), "the targets themselves are kept");

    let st = total(&f.app);
    assert_eq!(st.total, 0);
    assert_eq!(st.replies, 0);
    assert_eq!(st.median, 0.0);
    assert_eq!(st.p95, 0.0);
    assert_eq!(st.loss_pct, 0.0);
    assert!(
        st.min.is_infinite() && st.min > 0.0,
        "without a reply, min is the boundary value, not 0: {}",
        st.min
    );
}
