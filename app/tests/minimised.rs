//! Does the app keep measuring while its window is minimised?
//!
//! Reported from the running program: start, minimise the window for half a minute, maximise it
//! again — and the chart has a hole for exactly those thirty seconds. `eframe` runs no egui pass
//! for a window that is minimised (or hidden, the same rule): `App::ui` is not called, only
//! `App::logic` (about ten times a second). So the engine step has to live in `logic`, where a
//! pass without a window still reaches it.
//!
//! The tests below drive the app the way the integration drives such a window: `logic` in a
//! loop, `ui` never. Probes go to `127.0.0.1`, so no network is involved. As in the other test
//! files, `SYSTEM_PING_STATE_DIR` points the app at a throwaway folder — the app saves from its
//! own `tick`, and a test must never be able to write over a recorded history.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use system_ping::app::{App, AppOptions};

/// The store folder comes from one process-wide variable, so whoever sets it owns the store
/// until it is done (see the same lock in `collection.rs`).
static STATE_DIR_LOCK: Mutex<()> = Mutex::new(());

struct StateDir {
    dir: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl StateDir {
    fn new(name: &str) -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "system-ping-test-{}-{}",
            name,
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&p);
        let _ = std::fs::create_dir_all(&p);
        let _guard = STATE_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("SYSTEM_PING_STATE_DIR", &p);
        Self { dir: p, _guard }
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        if std::env::var_os("SYSTEM_PING_STATE_DIR").as_deref() == Some(self.dir.as_os_str()) {
            std::env::remove_var("SYSTEM_PING_STATE_DIR");
        }
    }
}

/// A measuring app with one loopback target and a short probe interval.
fn app_measuring(name: &str) -> (StateDir, App) {
    let dir = StateDir::new(name);
    let mut app = App::with_theme_override(
        AppOptions {
            interval_secs: 0.25,
            timeout_ms: 150,
            horizon_secs: 300.0,
            ..AppOptions::default()
        },
        Vec::new(),
        Some("dark"),
    );
    app.set_test_targets(&[("loop", "127.0.0.1")]);
    app.start_quiet(true);
    (dir, app)
}

/// The reported bug, as a test: a window that shows nothing for a few seconds must fill its
/// chart the whole time.
#[test]
fn a_minimised_window_keeps_measuring() {
    let (_dir, mut app) = app_measuring("minimised");

    // What eframe hands the app on such a pass: the context, and a `Frame` with nothing in it.
    // `ui` is the call that does not come — the whole point of the test.
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();

    let from = app.now_secs();
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_millis(3200) {
        <App as eframe::App>::logic(&mut app, &ctx, &mut frame);
        // The shortest interval eframe uses for a window nobody looks at (its
        // `INVISIBLE_WINDOW_REPAINT_INTERVAL`).
        std::thread::sleep(Duration::from_millis(100));
    }
    let away = app.now_secs() - from;

    let samples = app.targets[0].series.samples();
    assert!(
        samples.len() >= 4,
        "{} s of passes that paint nothing, at a 0,25 s interval, collected {} samples — the \
         window measured nothing while it was hidden",
        away.round(),
        samples.len()
    );

    // The complaint was not about the number of samples but about the hole: the chart showed
    // nothing for the time the window was away. So the samples have to cover that time — counted
    // from the first sample, because when *that* one can arrive is the machine's business and not
    // the app's: the system's own `ping` decides how long a probe takes, and a machine that does
    // not answer its own loopback at all spends its whole timeout on every probe — there the
    // first sample is more than a second late while the app measured the entire time. Two probe
    // intervals of slack: the last probe may still be on its way when the loop stops, and one
    // that left after the final pass has nobody left to collect it.
    let first = samples[0].t_secs - from;
    let span = samples.last().unwrap().t_secs - samples[0].t_secs;
    let after_first = away - first;
    assert!(
        span + 0.9 >= after_first,
        "the samples cover {span:.2} s of the {after_first:.2} s after the first measurement — \
         the chart would show a hole"
    );
    for w in samples.windows(2) {
        assert!(
            w[1].t_secs - w[0].t_secs < 1.5,
            "a pause of {:.2} s between two measurements at a 0,25 s interval",
            w[1].t_secs - w[0].t_secs
        );
    }
    assert!(
        app.now_secs() - samples.last().unwrap().t_secs < 1.5,
        "the newest measurement is older than one interval — the last moment before the window \
         came back would be missing"
    );
}

/// `eframe` calls `logic` and then `ui` inside one pass of a visible window, so with the engine
/// step in both places the step runs twice per painted frame. That is only acceptable while the
/// second call for the same moment gets nothing left to do: probes are armed for a moment in the
/// future, and the results queue is drained empty the first time. Should `tick` ever grow a
/// side effect that does not look at the clock, this is where it shows up.
#[test]
fn a_second_step_at_the_same_moment_arms_nothing() {
    let (_dir, mut app) = app_measuring("minimised-twice");
    let id = app.targets[0].id;

    let at = app.now_secs();
    app.tick(at);
    let due = app.next_probe_secs(id);
    let collected = app.targets[0].series.len();
    assert!(
        due > at,
        "the first step was supposed to arm the next probe for a later moment"
    );

    app.tick(at);
    assert_eq!(
        app.next_probe_secs(id),
        due,
        "a second step for the same moment scheduled a new probe"
    );
    assert!(
        app.targets[0].series.len() <= collected + 1,
        "a second step for the same moment collected more than one measurement"
    );

    // And the step must never be the reason for a measurement that was never probed: no two
    // samples may share the moment they claim to come from.
    let samples = app.targets[0].series.samples();
    for w in samples.windows(2) {
        assert_ne!(
            w[0].t_secs, w[1].t_secs,
            "the same moment appears twice in the chart's data"
        );
    }
}
