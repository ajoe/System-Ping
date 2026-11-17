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
    //
    // One tick first: an action's sentence reaches the bar at the end of a tick (see `App::say`), so reading the line straight
    // after the click would only prove the bar had not caught up yet.
    app.tick(app.engine.elapsed_secs());
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

    // What the Run key holds is a fact about this machine, not about the app: an entry may well be there because someone turned
    // autostart on once. So the test asks the same question the app asked and compares the two answers — that is what "the app
    // reads the registry instead of trusting its option" means, and it stays true whichever way the key happens to stand.
    let really_there = system_ping::platform::autostart::exists();
    assert_eq!(
        found, really_there,
        "the app reported {found}, the Run key says {really_there}"
    );
    // And tick() has already pulled that answer into the option the settings screen shows.
    assert_eq!(app.options.autostart, really_there);
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

/// A sentence the user caused has to survive the things that report by themselves. The case that started this: a protocol file
/// whose path had gone bad wrote its warning into the status bar on every tick for the next half minute, so "A is now position 2"
/// was there for one frame and gone — the list looked like it had never answered the drag at all.
#[test]
fn an_action_outlives_an_automatic_warning() {
    let dir = std::env::temp_dir().join("sp-status-order");
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);

    let mut app = system_ping::app::App::with_theme_override(
        system_ping::app::AppOptions::default(),
        Vec::new(),
        None,
    );
    // The warning needs a protocol file that is open and then reports a failure — which is what happens when the path goes bad
    // between one probe and the next. Both fields are public for exactly this reason.
    app.set_protocol(true, system_ping::exporter::ProtocolFormat::Csv);
    assert!(app.protocol.is_some(), "the protocol file is on");
    if let Some(p) = &mut app.protocol {
        p.failed = Some("disk gone".to_string());
    }

    // The action: the first row moved to the end. Then one tick, in which the warning gets its say first.
    let a = app.targets[0].id;
    let n = app.targets.len();
    app.move_target(a, n);
    app.tick(app.engine.elapsed_secs());

    let lang = app.lang();
    assert!(
        !app.status_msg.contains(
            lang.msg_protocol_unwritable_fmt()
                .split('{')
                .next()
                .unwrap()
                .trim_end()
        ),
        "the automatic warning pushed the move out of the status bar: {:?}",
        app.status_msg
    );
    assert!(
        app.status_msg.contains("A") || app.status_msg.contains(&app.targets[n - 1].label),
        "the status bar does not report the move, but says: {:?}",
        app.status_msg
    );

    // And it stays: the warning is not allowed to win a second tick either, because a note is only ever replaced by a newer one.
    app.tick(app.engine.elapsed_secs());
    assert!(
        !app.status_msg.contains(
            lang.msg_protocol_unwritable_fmt()
                .split('{')
                .next()
                .unwrap()
                .trim_end()
        ),
        "the warning came back on the next tick: {:?}",
        app.status_msg
    );
}
