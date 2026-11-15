//! Prüft die Filter, die über Diagramm, Legende, Ereignisliste und Statistik entscheiden: die
//! Kategorien als Schalter im Kopfbereich, einzelne Ziele, die per Klick auf ihren Namen in der
//! Legende verschwinden, und das Textfeld. Alle drei laufen an einer Stelle zusammen
//! (`App::shown_target_ids`), und genau das wird hier geprüft — nicht der Weg dorthin über Chips
//! oder Legenden-Einträge, denn den nimmt die GUI dem Test ab.

use system_ping::app::{App, AppOptions, TargetKind};

/// Ein App-Exemplar mit bekannten Zielen, unabhängig davon, was das Netzwerk gerade hergibt.
fn app_with(targets: Vec<(&str, &str)>) -> App {
    let mut app = App::with_theme_override(AppOptions::default(), Vec::new(), None);
    app.targets.clear();
    for (label, host) in targets {
        app.new_label = label.to_string();
        app.new_host = host.to_string();
        app.add_target();
    }
    app.status_msg.clear();
    app
}

/// Die Standardziele, aber so gesetzt, dass jede Kategorie genau einmal vorkommt.
fn one_of_each() -> App {
    let mut app = app_with(vec![
        ("Gateway", "192.168.178.1"),
        ("NAS", "192.168.178.42"),
        ("DNS", "8.8.8.8"),
    ]);
    // `add_target` rät die Kategorie aus der Adresse; hier soll sie ausdrücklich gesetzt sein.
    for (kind, host) in [
        (TargetKind::Gateway, "192.168.178.1"),
        (TargetKind::Lan, "192.168.178.42"),
        (TargetKind::Internet, "8.8.8.8"),
    ] {
        let t = app
            .targets
            .iter_mut()
            .find(|t| t.host == host)
            .expect("Ziel vorhanden");
        t.kind = kind;
    }
    app
}

/// Ob ein Ziel im Diagramm steht, in der Legende und in der Statistik — also alle Filter passiert.
fn shown(app: &App, host: &str) -> bool {
    let id = app
        .targets
        .iter()
        .find(|t| t.host == host)
        .expect("Ziel vorhanden")
        .id;
    app.shown_target_ids().contains(&id)
}

/// Eine Kategorie ausblenden nimmt alle ihre Ziele aus Diagramm, Legende und Statistik.
/// Umgeschaltet wird über `App::toggle_kind`; die Chips im Kopfbereich rufen genau das auf.
#[test]
fn eine_kategorie_aus_versteckt_alle_ihre_ziele() {
    let mut app = one_of_each();
    assert_eq!(
        app.shown_target_ids().len(),
        3,
        "zunächst ist alles sichtbar"
    );

    app.toggle_kind(TargetKind::Lan);
    assert!(!shown(&app, "192.168.178.42"), "die NAS ist weg");
    assert!(shown(&app, "192.168.178.1"), "der Gateway bleibt");
    assert!(shown(&app, "8.8.8.8"), "der DNS bleibt");

    // Der Schalter wirkt auch für Kategorien, die gar kein Ziel hat: nichts zu tun ist in Ordnung.
    app.toggle_kind(TargetKind::Custom);
    assert_eq!(app.shown_target_ids().len(), 2);

    // „Alle“ schaltet alles wieder an.
    app.show_all_kinds();
    assert_eq!(
        app.shown_target_ids().len(),
        3,
        "nach „alle“ ist wieder alles da"
    );
}

/// Ein Klick auf den Namen in der Legende blendet nur dieses eine Ziel aus — die Legende selbst
/// zeigt es weiter an, sonst könnte man es nicht zurückholen.
#[test]
fn ein_ausgeblendetes_ziel_bleibt_in_der_legende() {
    let mut app = one_of_each();
    let nas = app
        .targets
        .iter()
        .find(|t| t.host == "192.168.178.42")
        .unwrap()
        .id;

    assert!(
        app.toggle_hidden_by_id(nas),
        "das Ziel ist jetzt ausgeblendet"
    );
    assert!(
        !shown(&app, "192.168.178.42"),
        "es fehlt in Diagramm und Statistik"
    );
    assert!(shown(&app, "192.168.178.1"), "die Nachbarn bleiben");

    // Die Legende muss es weiterhin nennen, sonst gibt es kein Zurück.
    let legend = app.visible_targets();
    assert!(legend.contains(&nas), "die Legende zeigt es weiter an");

    assert!(
        !app.toggle_hidden_by_id(nas),
        "ein zweiter Klick holt es zurück"
    );
    assert!(shown(&app, "192.168.178.42"));
}

/// Der Textfilter sucht auch in der Kategorie, sonst findet man „LAN-Gerät“ nicht, obwohl die
/// Spalte genau das zeigt.
#[test]
fn der_textfilter_findet_auch_die_kategorie() {
    let mut app = one_of_each();
    app.filter = String::from("LAN");
    assert!(shown(&app, "192.168.178.42"), "die Kategorie passt");
    assert!(!shown(&app, "8.8.8.8"), "das Internet-Ziel nicht");

    app.filter = String::from("dns");
    assert!(
        shown(&app, "8.8.8.8"),
        "der Name passt, und zwar ohne auf die Großschrift zu achten"
    );
    assert!(!shown(&app, "192.168.178.42"));

    app.filter = String::new();
    assert_eq!(
        app.shown_target_ids().len(),
        3,
        "ein leerer Filter blendet nichts aus"
    );
}

/// Die Filter zusammen: ein Ziel ist nur sichtbar, wenn alle durchgehen.
#[test]
fn text_und_kategoriefilter_gelten_gleichzeitig() {
    let mut app = one_of_each();
    app.filter = String::from("LAN");
    assert_eq!(app.shown_target_ids().len(), 1);

    // Jetzt die Kategorie, auf die der Filter zeigt, aus: gar nichts bleibt übrig.
    app.toggle_kind(TargetKind::Lan);
    assert!(app.shown_target_ids().is_empty());

    app.show_all_kinds();
    app.filter = String::new();
    assert_eq!(app.shown_target_ids().len(), 3);
}
