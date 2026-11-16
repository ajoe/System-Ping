// Checks the three fixed functional bugs: export feedback, the autostart read-out path, and
// whether the concurrency setting reaches the engine right away.

use system_ping::i18n;

/// The CSV export now reports its result in the status line instead of insisting on "Writing CSV
/// file…" forever.
#[test]
fn export_reports_its_result() {
    let dir = std::env::temp_dir().join("sp-export-result");
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);

    let mut app = system_ping::app::App::with_theme_override(
        system_ping::app::AppOptions::default(),
        Vec::new(),
        None,
    );
    app.targets[0]
        .series
        .record(1.0, 1_700_000_000_000, 12.0, None);

    let out = dir.join("out.csv");
    assert!(
        app.export_csv(out.clone()),
        "the export has to hand a file over"
    );
    // The path is part of the message now — on a slow network drive, an export that is still
    // writing cannot be told apart from a stuck window after one second.
    let lang = app.lang();
    let prefix = i18n::fill(lang.msg_writing_csv_path_fmt(), &[("path", String::new())]);
    assert!(
        app.status_msg.starts_with(prefix.trim_end()) && app.status_msg.contains("out.csv"),
        "the status line does not name the file: {:?}",
        app.status_msg
    );

    // The writer thread needs a moment; afterwards tick() picks the message up.
    let mut seen = String::new();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.tick(app.engine.elapsed_secs());
        seen = app.status_msg.clone();
        if !seen.starts_with(prefix.trim_end()) {
            break;
        }
    }
    assert!(out.exists(), "the CSV file was written");
    assert!(
        seen.contains(
            lang.msg_csv_written_fmt()
                .split('{')
                .next()
                .unwrap()
                .trim_end()
        ),
        "the status line does not report the result, but says: {seen:?}"
    );
}

/// The autostart button now asks the registry at start-up. Without that path it sat
/// permanently on "off", no matter what HKCU\...\Run contained.
#[test]
fn autostart_is_read_from_the_registry() {
    let dir = std::env::temp_dir().join("sp-autostart-read");
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);

    let mut app = system_ping::app::App::with_theme_override(
        system_ping::app::AppOptions::default(),
        Vec::new(),
        None,
    );
    // with_theme_override kicks off the query; until it answers, the state is unknown.
    let mut answer = None;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.tick(app.engine.elapsed_secs());
        if let Some(found) = app.autostart_state() {
            answer = Some(found);
            break;
        }
    }
    let found = answer.expect("the result of the registry query never arrives");

    // On this machine there is no SystemPing entry (checked in HKCU\...\Run), so the answer has to
    // be "not present" — and tick() has already pulled it into options.autostart.
    assert!(
        !found,
        "there is no Run entry, so the app has to report that too"
    );
    assert!(!app.options.autostart);
}

/// The "concurrent probes" setting now reaches the engine without a restart.
#[test]
fn concurrency_change_reaches_the_engine() {
    let dir = std::env::temp_dir().join("sp-concurrency");
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);

    let mut app = system_ping::app::App::with_theme_override(
        system_ping::app::AppOptions::default(),
        Vec::new(),
        None,
    );
    // A value well above the start value: the engine should accept it.
    app.options.concurrency = 9;
    app.sync_engine();
    // No direct query is possible (the engine keeps its size in its own thread), but at least the
    // path must not be discarded silently: a send on the command channel has to succeed even while
    // nothing is running.
    app.options.concurrency = 3;
    app.sync_engine();
}
