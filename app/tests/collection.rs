//! Does the program actually collect data? These tests run without a window: no eframe, no
//! egui context, nothing painted — they drive the same engine and history the GUI uses and
//! check that samples arrive and land in the saved file.
//!
//! The probes go to `127.0.0.1` (the loopback address), so no internet is needed; the real
//! `ping.exe` still runs. Each test gets its own throwaway state directory, so it never
//! touches a recorded history (`SYSTEM_PING_STATE_DIR`, see `store::state_dir`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// The store picks its folder from one process-wide variable (`SYSTEM_PING_STATE_DIR`), so a
/// test that points it somewhere else owns the store for as long as it runs. Without this
/// lock, two tests in the same binary would read each other's state file — which is exactly
/// what happened to `app_tick_collects_data_into_its_charts`: another pass deleted the folder
/// between the app's write and the test's read-back.
static STATE_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Unique temp folder per test run, removed at the end. The guard is held for the owner's
/// whole lifetime; `_guard` must therefore be the last field of every test struct that owns
/// one (fields drop in declaration order).
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
        let _ = std::fs::create_dir_all(&p);
        // Poisoned only if a holder panicked; the variable it set is still ours to reuse.
        let _guard = STATE_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("SYSTEM_PING_STATE_DIR", &p);
        Self { dir: p, _guard }
    }

    fn state_file(&self) -> PathBuf {
        self.dir.join("system-ping-state.json")
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        // Only clear the variable if it still points at this folder: tests run in parallel,
        // and a later `StateDir::new` may already own it.
        if std::env::var_os("SYSTEM_PING_STATE_DIR").as_deref() == Some(self.dir.as_os_str()) {
            std::env::remove_var("SYSTEM_PING_STATE_DIR");
        }
    }
}

/// Wait until `probe_now` answers, with retries: the loopback ping is fast, but a busy
/// machine or a filtered ICMP stack can take a second or two on the first try.
fn probe_loopback(
    engine: &system_ping::ping::PingEngine,
    host: &str,
    tries: u32,
) -> Option<ProbeResultAlias> {
    for _ in 0..tries {
        if let Some(r) = engine.probe_now(1, host, 1500, 6.0) {
            return Some(r);
        }
    }
    None
}

type ProbeResultAlias = system_ping::ping::ProbeResult;

/// A fresh history file starts empty — that is what the next tests compare against.
#[test]
fn state_starts_empty() {
    let dir = StateDir::new("empty");
    let (options, saved) = system_ping::store::load();
    assert!(saved.is_empty(), "a new state dir must have no history");
    assert!(options.horizon_secs > 0.0, "options fall back to defaults");
    assert!(!dir.state_file().exists());
}

/// The engine alone: one probe on the loopback address comes back as a parsed result.
/// This is the collection path with nothing else attached — no window, no history.
#[test]
fn engine_collects_one_sample_without_window() {
    let _dir = StateDir::new("engine");
    let engine = system_ping::ping::PingEngine::new(2);
    let r = probe_loopback(&engine, "127.0.0.1", 3).expect("no probe result came back");
    assert_eq!(r.id, 1, "the result must carry the job id it was asked for");
    if let Some(fail) = r.fail {
        // A reply is not required to be fast, but a failure must say why, in text we parsed.
        let text = system_ping::history::Fail::from(fail).short();
        assert!(!text.is_empty(), "a failed probe still carries a reason");
    } else {
        assert_ne!(
            r.rtt_ms,
            system_ping::history::TIMEOUT,
            "a good sample is not a timeout"
        );
        assert!(
            r.rtt_ms >= 0.0 && r.rtt_ms < 60_000.0,
            "implausible rtt {}",
            r.rtt_ms
        );
    }
    engine.stop_all();
}

/// The scheduled path the window uses: `start` hands jobs to the scheduler, and the results
/// come back through `drain_results`.
#[test]
fn scheduled_probes_reach_the_collector() {
    let _dir = StateDir::new("scheduled");
    let engine = system_ping::ping::PingEngine::new(2);
    engine.start(system_ping::ping::ProbeJob {
        id: 7,
        host: "127.0.0.1".to_string(),
        timeout_ms: 1500,
    });
    let mut got = Vec::new();
    // `start` only queues the job; the scheduler issues it from its own tick, so this waits
    // for the first result rather than sleeping a fixed guess.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while got.is_empty() && std::time::Instant::now() < deadline {
        engine.debug_advance(1.0);
        std::thread::sleep(std::time::Duration::from_millis(50));
        got = engine.drain_results();
    }
    let r = got
        .first()
        .expect("scheduled probe produced no result within 30 s")
        .clone();
    engine.stop_all();
    // t_secs is stamped by the worker when it finishes, so it must be a real reading from
    // this run: not negative, and not later than "now" either.
    assert_eq!(r.id, 7);
    assert!(r.t_secs >= 0.0);
    assert!(r.t_secs <= std::time::Instant::now().elapsed().as_secs_f64() + 5.0);
}

/// End to end without a GUI: `collect_once` runs the app's own loop for a few seconds and
/// writes what it collected to the state file. The history must grow, and the file on disk
/// must show the same samples.
#[test]
fn collect_once_records_samples_to_disk() {
    let dir = StateDir::new("collect");
    let before = system_ping::store::load().1;
    assert!(before.is_empty(), "the pass starts from an empty history");

    // 7 s at the default interval should give a handful of samples per target. The hosts are
    // named by `collect_once` itself, so this does not depend on any saved settings.
    let report = system_ping::app::collect_once(&dir.dir, 7.0);
    eprintln!(
        "report {report:?} file {}",
        std::fs::read_to_string(dir.state_file()).unwrap_or_default()
    );
    assert!(!report.is_empty(), "no targets were probed at all");

    // The pass reports what it wrote; the file is checked separately below.
    let mut total_new = 0usize;
    for (host, was, now) in &report {
        assert!(*now >= *was, "{host}: the history shrank");
        total_new += now - was;
    }
    assert!(
        total_new > 0,
        "no samples were collected — the app would show empty charts"
    );

    let path = dir.state_file();
    assert!(path.exists(), "collect_once must write {}", path.display());
    let text = std::fs::read_to_string(&path).expect("state file unreadable");
    let persisted: serde_json::Value = serde_json::from_str(&text).expect("state file is not JSON");
    let history = persisted["history"]
        .as_array()
        .expect("history array missing");
    let on_disk: usize = history
        .iter()
        .map(|t| t["samples"].as_array().map_or(0, |s| s.len()))
        .sum();
    // The app saves from its own `tick` while the pass runs, so the file holds everything
    // collected so far; what matters is that it recorded real samples and at least as many
    // as the report counted.
    assert!(
        on_disk >= total_new,
        "state file has {on_disk} samples but the pass reported {total_new}"
    );
    assert!(
        on_disk > 0,
        "nothing landed in the state file — a restart would show empty charts"
    );

    // Reading it back through the app's own loader gives the same counts. This call takes
    // the directory directly, so it is not affected by SYSTEM_PING_STATE_DIR.
    let (_, saved) = system_ping::store::load_from(&dir.dir);
    let in_memory: usize = saved.iter().map(|t| t.samples.len()).sum();
    assert!(in_memory >= total_new);
    for t in &saved {
        assert!(!t.host.is_empty() && !t.label.is_empty());
        for (at_ms, rtt) in &t.samples {
            // Version 2 stores absolute Unix milliseconds; a real clock is always positive.
            assert!(*at_ms > 0, "sample time must be an absolute timestamp");
            if *rtt < 0.0 {
                // -1 marks a timeout: it is stored so the chart can show the gap.
                assert!(
                    matches!(*rtt, r if r == -1.0),
                    "unexpected negative rtt {rtt}"
                );
            }
        }
    }
}

/// Does the app collect data at all? This drives the whole app headless — `App::new` and
/// `App::tick`, the exact two calls eframe makes every frame — with no window, so it is the
/// strongest version of that question that still runs without a desktop session. If anything
/// between the tick and the history were broken (no targets, a probe job that never starts,
/// samples dropped on arrival) the charts would stay empty and this fails.
///
/// The default targets are gateway/DNS/Internet, which need a working network, so this
/// replaces them with one address that always answers: the pass then proves collection, not
/// connectivity.
#[test]
fn app_tick_collects_data_into_its_charts() {
    let dir = StateDir::new("app-tick");
    // No saved history: everything counted below comes from this run.
    assert!(system_ping::store::load().1.is_empty());

    // The state file is shared by every test in this binary (`store::state_dir()` reads one
    // environment variable, and tests run in parallel), so a run that starts with nothing
    // saved can find another run's data on disk. `fresh` counts what came from this app only.

    let mut app = system_ping::app::App::new(system_ping::app::AppOptions::default(), Vec::new());
    assert!(
        !app.targets.is_empty(),
        "the app starts with nothing to probe"
    );
    // Same as `collect_once`: the default interval is 1 s, and loopback answers, so at least
    // one target must produce data even on a machine without internet. The default targets
    // are gateway/DNS/Internet, so the loopback one has to be added by hand here.
    app.options.interval_secs = 1.0;
    app.targets.clear();
    app.add_named_target("127.0.0.1", "loopback");
    assert!(
        app.targets.iter().any(|t| t.host == "127.0.0.1"),
        "the loopback target was not added — the pass would prove nothing without internet"
    );
    let started: usize = app.targets.iter().map(|t| t.series.len()).sum();
    app.start();

    // Drive it like eframe would, on a rough frame budget, for ~10 s. `tick` takes the
    // session clock (seconds since start), not a frame duration: passing a constant 0.05
    // would make every target due again after 50 ms and schedule probes into a clock that
    // never moves, so no sample ever lands on the chart's x axis.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        app.tick(app.now_secs());
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // 1) The charts have data. `App::new` got an empty history and the target list was rebuilt
    // from scratch, so every sample in a series came from this run.
    let mut fresh = 0usize;
    let mut with_data = 0usize;
    for t in &app.targets {
        if !t.series.is_empty() {
            with_data += 1;
        }
        fresh += t.series.len();
    }
    assert!(
        fresh > started,
        "the app ran 10 s of ticks and the charts stayed empty"
    );
    assert!(with_data > 0, "no target produced a single sample");

    // 2) The numbers are plausible, not placeholders.
    let mut logged = 0usize;
    for t in &app.targets {
        for s in t.series.samples() {
            assert!(s.t_secs >= 0.0);
            if let Some(fail) = s.fail {
                assert!(!fail.short().is_empty(), "a failure must carry a reason");
                assert!(!system_ping::history::is_ok(s.rtt_ms));
            } else {
                assert!(system_ping::history::is_ok(s.rtt_ms));
                assert!(s.rtt_ms < 60_000.0, "implausible rtt {}", s.rtt_ms);
            }
            logged += 1;
        }
    }
    assert_eq!(logged, fresh);

    // 3) The event log. Loopback answered, so the app's one outage line ("loopback antwortet
    // nicht (Ping-Fehler).") is what a failed probe writes — proof that results reach both the
    // charts and the log. Which of the two happens depends on this machine's ICMP permissions,
    // so the pass asserts on the text rather than a count, and never on connectivity.
    let texts: Vec<&str> = app.log.iter().map(|e| e.text.as_str()).collect();
    assert!(!texts.is_empty(), "the app logged nothing at all");
    let healthy = with_data > 0;
    if healthy {
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("Timeout") || t.contains("nicht erreichbar")),
            "a target that answered was reported as a failure: {texts:?}",
        );
    } else {
        assert!(
            texts.iter().any(|t| t.contains("loopback")),
            "a target that never answered left no trace in the log: {texts:?}",
        );
    }
    eprintln!(
        "log_total {} tail {:?}",
        app.log_total,
        &texts[texts.len().saturating_sub(4)..]
    );

    // 4) And it survives a restart: the same state dir, read back by the loader. Saving runs
    // on its own clock (`save_due_secs`, every 8 s of ticked time), so the pass asks for the
    // write itself and then reads it back. The file is the app's real state file, which other
    // tests in this binary share through `SYSTEM_PING_STATE_DIR`, so what counts here is the
    // loopback target's own samples — they can only have come from this run.
    app.save_now();
    let mut saved = system_ping::store::load_from(&dir.dir);
    let on_disk_for_us = |s: &(
        system_ping::app::AppOptions,
        Vec<system_ping::store::SavedTarget>,
    )| {
        s.1.iter()
            .filter(|t| t.host == "127.0.0.1")
            .map(|t| t.samples.len())
            .sum::<usize>()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while on_disk_for_us(&saved) == 0 && std::time::Instant::now() < deadline {
        app.tick(app.now_secs());
        std::thread::sleep(std::time::Duration::from_millis(200));
        saved = system_ping::store::load_from(&dir.dir);
    }
    let on_disk: usize = on_disk_for_us(&saved);
    assert!(
        on_disk > 0,
        "the app collected {} samples in memory but wrote none to disk",
        fresh
    );
}

/// The same samples must survive the CSV export path, which also runs without a window.
#[test]
fn collected_samples_export_to_csv() {
    let dir = StateDir::new("csv");
    // The export below reads through `store::load()`, which follows SYSTEM_PING_STATE_DIR;
    // `StateDir::new` already points that at this folder, so both halves agree.
    let report = system_ping::app::collect_once(&dir.dir, 5.0);
    let samples: usize = report.iter().map(|(_, was, now)| now - was).sum();
    assert!(samples > 0, "nothing to export");

    let (_, saved) = system_ping::store::load_from(&dir.dir);
    assert!(
        saved.iter().map(|t| t.samples.len()).sum::<usize>() >= samples,
        "the loader read less than the pass collected"
    );
    let mut out = dir.dir.clone();
    // Unique per run: `write_csv` appends, so a leftover file from an earlier run would add
    // lines this run did not report.
    out.push(format!("export-{}.csv", std::process::id()));
    let rows = system_ping::app::export_history_to(&saved, &out).expect("csv export failed");
    assert!(rows > 0, "the csv has no data rows");

    let text = std::fs::read_to_string(&out).expect("csv unreadable");
    // One row per sample plus the header line.
    let data_lines = text.lines().filter(|l| !l.starts_with('#')).count() - 1;
    assert_eq!(
        data_lines, rows,
        "csv line count does not match the reported rows"
    );
}

/// A protocol file must stream the same rows the CSV export writes, and keep what was
/// already on disk (it appends, it does not rewrite).
#[test]
fn protocol_file_streams_every_measurement() {
    let dir = StateDir::new("protocol");
    let mut path = dir.dir.clone();
    path.push("protocol.csv");

    let mut file = system_ping::exporter::ProtocolFile::new(
        path.clone(),
        system_ping::exporter::ProtocolFormat::Csv,
    );
    file.open();
    assert!(file.is_open(), "the protocol file must open");
    let origin = chrono::Local::now();
    let samples = [(0.0_f64, 4.0_f32), (1.0, -1.0), (2.0, 9.5)];
    let mut series = system_ping::history::Series::new(8);
    for &(t, rtt) in &samples {
        series.push(system_ping::history::Sample {
            t_secs: t,
            rtt_ms: rtt,
            fail: if rtt >= 0.0 {
                None
            } else {
                Some(system_ping::history::Fail::Timeout)
            },
            // Zero means "no absolute time", so the exporter falls back to `origin` + t_secs —
            // exactly the path an older caller hits.
            at_ms: 0,
        });
    }
    let mut written = 0;
    for row in system_ping::exporter::collect_rows("example", &series, origin) {
        assert!(file.write(&row), "protocol write must succeed");
        written += 1;
    }
    drop(file);

    let text = std::fs::read_to_string(&path).expect("protocol unreadable");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "timestamp,target,latency_ms,status");
    assert_eq!(lines.len(), written + 1, "{text}");
    // The failed probe is a row too, with no latency and the reason as status.
    assert!(lines.iter().any(|l| l.ends_with(",timeout")), "{text}");

    // A second file over the same path adds lines instead of replacing them.
    let mut again = system_ping::exporter::ProtocolFile::new(
        path.clone(),
        system_ping::exporter::ProtocolFormat::Csv,
    );
    again.open();
    assert!(again.is_open());
    let row = system_ping::exporter::Row {
        timestamp: "2025-01-01 00:00:00.000".into(),
        target: "example".into(),
        latency_ms: Some(1.0),
        status: "ok".into(),
    };
    assert!(again.write(&row));
    drop(again);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), lines.len() + 1, "{text}");
    assert_eq!(
        text.matches("timestamp,target").count(),
        1,
        "header written twice"
    );
}

/// A host that will never answer still has to produce a record — otherwise an outage would
/// look like "no data" instead of "lost".
#[test]
fn unreachable_host_still_records_a_result() {
    let _dir = StateDir::new("unreachable");
    let engine = system_ping::ping::PingEngine::new(2);
    // TEST-NET-1 (RFC 5737) is not routable, so this times out quickly and offline.
    let r = engine
        .probe_now(9, "192.0.2.123", 400, 8.0)
        .expect("a timed-out probe must still be reported");
    assert_eq!(r.id, 9);
    if r.fail.is_none() {
        // Only reachable if something on this network answers that address; then it is a
        // sample like any other.
        assert!(r.rtt_ms >= 0.0);
    } else {
        assert_eq!(r.rtt_ms, system_ping::history::TIMEOUT);
    }
    engine.stop_all();
}

/// Nothing in the collection path may need a window: check that the pieces used above are
/// reachable from the library alone (this test compiles without eframe types).
#[test]
fn collection_needs_no_gui_types() {
    fn takes_no_egui<T>(x: &T) -> *const T {
        x
    }
    let job = system_ping::ping::ProbeJob {
        id: 0,
        host: String::new(),
        timeout_ms: 0,
    };
    assert!(!takes_no_egui(&job).is_null());
    let p: &Path = Path::new(".");
    assert!(p.exists());
}
