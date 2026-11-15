// Prüft den Inline-Editor für Ziel-Einträge: Einzelklick öffnet ihn, Speichern übernimmt
// Name, Adresse, Kategorie und die Mess-Schaltfläche, und nichts kann die Liste verschieben,
// während ein Eintrag offen ist.

use system_ping::app::{App, AppOptions, TargetKind};

fn app_with(targets: Vec<(&str, &str)>) -> App {
    let mut app = App::with_theme_override(AppOptions::default(), Vec::new(), None);
    // The default list comes from the machine's gateway; start from a known set instead.
    app.targets.clear();
    for (label, host) in targets {
        app.new_label = label.to_string();
        app.new_host = host.to_string();
        app.add_target();
    }
    app.status_msg.clear();
    app
}

/// Ein Klick auf eine Zeile markiert sie und öffnet sogleich den Editor.
#[test]
fn click_selects_and_opens_the_editor() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);
    assert_eq!(app.edit_idx, None, "initial ist nichts in Bearbeitung");

    app.begin_edit(1);
    assert_eq!(app.selected, 1, "die Zeile ist markiert");
    assert_eq!(app.edit_idx, Some(1));
    assert_eq!(app.edit_host, "127.0.0.1");
    assert_eq!(app.edit_label, "B");
    assert!(app.edit_enabled, "ein neues Ziel wird gemessen");

    // Der Editor zeigt die echten Werte des Ziels, nicht irgendwelche Reste.
    app.begin_edit(0);
    assert_eq!(app.edit_idx, Some(0));
    assert_eq!(app.edit_label, "A");
}

/// Ein leerer Name ist erlaubt: `commit_edit` behält dann den alten bei.
#[test]
fn renaming_a_target_keeps_its_history() {
    let mut app = app_with(vec![("alt", "127.0.0.1")]);
    let id = app.targets[0].id;
    app.targets[0]
        .series
        .record(1.0, 1_700_000_000_000, 4.0, None);

    app.begin_edit(0);
    app.edit_label = "neu".to_string();
    app.commit_edit();

    assert_eq!(app.targets[0].label, "neu");
    assert_eq!(app.targets[0].id, id, "das Ziel bleibt dasselbe");
    assert_eq!(
        app.targets[0].series.len(),
        1,
        "der Verlauf bleibt erhalten"
    );
    assert_eq!(app.edit_idx, None);
    assert!(app.status_msg.contains("neu"), "{}", app.status_msg);
}

/// Adresswechsel müssen die laufenden Messungen stoppen und den Status zurücksetzen.
#[test]
fn changing_the_host_resets_that_target_only() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);
    for t in app.targets.iter_mut() {
        t.series.record(1.0, 1_700_000_000_000, 5.0, None);
        t.last_rtt = 5.0;
    }

    app.begin_edit(0);
    app.edit_host = "192.0.2.7".to_string();
    app.commit_edit();

    assert_eq!(app.targets[0].host, "192.0.2.7");
    assert!(
        app.targets[0].last_rtt.is_nan(),
        "der alte Messwert darf nicht für die neue Adresse stehen"
    );
    // Das andere Ziel bleibt unangetastet.
    assert_eq!(app.targets[1].host, "127.0.0.1");
    assert_eq!(app.targets[1].last_rtt, 5.0);
}

/// Eine leere Adresse darf das Ziel nicht zerstören.
#[test]
fn an_empty_host_is_refused() {
    let mut app = app_with(vec![("A", "127.0.0.1")]);
    app.begin_edit(0);
    app.edit_host = "   ".to_string();
    app.edit_label = "kaputt".to_string();
    app.commit_edit();

    assert_eq!(app.targets[0].host, "127.0.0.1");
    assert_eq!(
        app.targets[0].label, "A",
        "auch der Name bleibt unverändert"
    );
    assert!(app.status_msg.contains("verworfen"), "{}", app.status_msg);
    assert_eq!(app.edit_idx, None);
}

/// Der Kategorie-Picker überschreibt die automatische Einordnung, solange die Adresse gleich bleibt.
#[test]
fn the_category_can_be_set_by_hand() {
    let mut app = app_with(vec![("Drucker", "192.168.178.1")]);
    assert_eq!(app.targets[0].kind, TargetKind::Gateway);

    app.begin_edit(0);
    assert_eq!(app.edit_kind, TargetKind::Gateway, "vorbelegt");
    app.edit_kind = TargetKind::Lan;
    app.commit_edit();

    assert_eq!(app.targets[0].kind, TargetKind::Lan);

    // Eine neue Adresse wird dagegen wieder automatisch eingeordnet.
    app.begin_edit(0);
    app.edit_kind = TargetKind::Custom;
    app.edit_host = "8.8.8.8".to_string();
    app.commit_edit();
    assert_eq!(
        app.targets[0].kind,
        TargetKind::Dns,
        "bei Adresswechsel entscheidet wieder classify()"
    );
}

/// „misst aus“ pausiert das Ziel, ohne den Verlauf zu löschen.
#[test]
fn measurement_can_be_switched_off_and_on() {
    let mut app = app_with(vec![("A", "127.0.0.1")]);
    app.targets[0]
        .series
        .record(1.0, 1_700_000_000_000, 6.0, None);

    app.begin_edit(0);
    app.edit_enabled = false;
    app.commit_edit();

    assert!(!app.targets[0].enabled);
    assert_eq!(app.targets[0].series.len(), 1, "Verlauf bleibt erhalten");
    assert!(app.status_msg.contains("pausiert"), "{}", app.status_msg);
    assert_eq!(app.status_text(&app.targets[0]), "pausiert");

    app.begin_edit(0);
    assert!(!app.edit_enabled, "der Editor zeigt den aktuellen Zustand");
    app.edit_enabled = true;
    app.commit_edit();
    assert!(app.targets[0].enabled);
}

/// Ein Abbruch verändert nichts.
#[test]
fn cancel_leaves_everything_alone() {
    let mut app = app_with(vec![("A", "127.0.0.1")]);
    app.begin_edit(0);
    app.edit_host = "198.51.100.9".to_string();
    app.edit_label = "weg".to_string();
    app.cancel_edit();

    assert_eq!(app.edit_idx, None);
    assert_eq!(app.targets[0].host, "127.0.0.1");
    assert_eq!(app.targets[0].label, "A");
}

/// Der Wechsel auf eine andere Zeile verliert keine Eingaben.
#[test]
fn switching_rows_commits_the_previous_edit() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);

    app.begin_edit(0);
    app.edit_label = "A umbenannt".to_string();
    app.begin_edit(1);

    assert_eq!(app.targets[0].label, "A umbenannt");
    assert_eq!(app.edit_idx, Some(1));
    assert_eq!(app.edit_label, "B", "der zweite Editor ist frisch gefüllt");
}

/// Solange ein Eintrag offen ist, darf die Liste nicht umsortiert werden.
#[test]
fn the_list_cannot_change_while_editing() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);
    app.begin_edit(1);

    app.new_host = "10.0.0.1".to_string();
    app.new_label = "C".to_string();
    app.add_target();
    assert_eq!(
        app.targets.len(),
        2,
        "kein neues Ziel während der Bearbeitung"
    );

    app.remove_selected();
    assert_eq!(app.targets.len(), 2, "kein Ziel entfernt");

    // Der Index zeigt weiterhin auf dasselbe Ziel.
    assert_eq!(app.edit_idx, Some(1));
    assert_eq!(app.targets[1].label, "B");
    assert!(app.status_msg.contains("Bearbeitung"), "{}", app.status_msg);

    // Nach dem Abschließen geht beides wieder.
    app.commit_edit();
    app.new_host = "10.0.0.1".to_string();
    app.add_target();
    assert_eq!(app.targets.len(), 3);
}

/// Das Entfernen sitzt jetzt im Editor des offenenen Ziels und nicht mehr oben rechts, wo es das
/// zuletzt markierte Ziel traf. Der Knopf selbst ist UI; hier wird derselbe Weg geprüft, den die
/// Kopfzeile des Editors geht: Editor schliessen, Index setzen, entfernen.
#[test]
fn removing_from_the_editor_targets_that_very_target() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);

    // Oben rechts war das markierte Ziel gemeint - hier absichtlich ein anderes.
    app.selected = 0;
    // Der Weg des Editor-Knopfes: er zeigt auf die Zeile, die gerade offen ist.
    app.begin_edit(1);
    app.cancel_edit();
    app.selected = 1;
    app.remove_selected();

    assert_eq!(app.targets.len(), 1);
    assert_eq!(app.targets[0].label, "A", "B ist weg, A bleibt");
}

/// Der Editor-Knopf entfernt auch dann das richtige Ziel, wenn die Liste vorher offen war: der
/// Editor wird zuerst geschlossen, damit kein Index ins Leere zeigt.
#[test]
fn the_editor_closes_before_its_target_is_removed() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);
    app.begin_edit(1);

    // Genau die Reihenfolge, die `ui::left_body` nach der Zeilenschleife ausführt.
    app.cancel_edit();
    app.selected = 1;
    app.remove_selected();

    assert_eq!(
        app.edit_idx, None,
        "kein Editor zeigt mehr auf eine entfernte Zeile"
    );
    assert_eq!(app.targets.len(), 1);
}
