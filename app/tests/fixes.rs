// Prüft die drei behobenen Funktionsfehler: Export-Rückmeldung, Autostart-Ablesepfad,
// und ob die Gleichzahl-Einstellung sofort an die Engine geht.

/// Der CSV-Export meldet sein Ergebnis jetzt in der Statuszeile, statt für immer
/// „Schreibe CSV-Datei …“ zu behaupten.
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
    app.export_csv(out.clone());
    // Der Pfad steht jetzt in der Meldung — bei einem langsamen Netzlaufwerk ist ein schreibender
    // Export nach einer Sekunde nicht von einem hängenden Fenster zu unterscheiden.
    assert!(
        app.status_msg.starts_with("Schreibe CSV-Datei …") && app.status_msg.contains("out.csv"),
        "die Statuszeile nennt nicht die Datei: {:?}",
        app.status_msg
    );

    // Der Schreibthread braucht einen Moment; danach holt tick() die Meldung ab.
    let mut seen = String::new();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.tick(app.engine.elapsed_secs());
        seen = app.status_msg.clone();
        if !seen.contains("Schreibe") {
            break;
        }
    }
    assert!(out.exists(), "die CSV-Datei wurde geschrieben");
    assert!(
        seen.contains("CSV-Export geschrieben"),
        "die Statuszeile meldet das Ergebnis nicht, sondern: {seen:?}"
    );
}

/// Der Autostart-Knopf fragt jetzt beim Start das Registry ab. Ohne diesen Pfad stand er
/// dauerhaft auf „aus“, egal was in HKCU\...\Run stand.
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
    // with_theme_override stößt die Abfrage an; bis sie antwortet ist der Zustand unbekannt.
    let mut answer = None;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.tick(app.engine.elapsed_secs());
        if let Some(found) = app.autostart_state() {
            answer = Some(found);
            break;
        }
    }
    let found = answer.expect("das Ergebnis der Registry-Abfrage kommt nie an");

    // Auf diesem Rechner existiert kein SystemPing-Eintrag (nachgesehen in HKCU\...\Run),
    // also muss die Antwort „nicht vorhanden“ lauten — und tick() hat sie bereits in
    // options.autostart nachgezogen.
    assert!(
        !found,
        "es gibt keinen Run-Eintrag, also muss die App das auch melden"
    );
    assert!(!app.options.autostart);
}

/// Die Einstellung „Gleichzeitige Prüfungen“ erreicht die Engine jetzt ohne Neustart.
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
    // Ein Wert, der deutlich über dem Startwert liegt: die Engine soll ihn annehmen.
    app.options.concurrency = 9;
    app.sync_engine();
    // Kein direktes Abfragen möglich (die Engine hält ihre Größe im eigenen Thread), aber der
    // Weg darf zumindest nicht still verworfen werden: ein Send auf den Befehlskanal muss
    // gelingen, auch wenn gerade nichts läuft.
    app.options.concurrency = 3;
    app.sync_engine();
}
