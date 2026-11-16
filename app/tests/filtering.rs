//! Checks the filters that decide what the chart, legend, event list and statistics show: the
//! categories as switches in the header, single targets that disappear when their name in the
//! legend is clicked, and the text field. All three meet at one place
//! (`App::shown_target_ids`), and that is exactly what is checked here — not the way there through
//! chips or legend entries, because the GUI takes care of that part.

use system_ping::app::{App, AppOptions, TargetKind};

/// An app instance with known targets, independent of whatever the network happens to offer.
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

/// The built-in targets, but set up so that every category appears exactly once.
fn one_of_each() -> App {
    let mut app = app_with(vec![
        ("Gateway", "192.168.178.1"),
        ("NAS", "192.168.178.42"),
        ("DNS", "8.8.8.8"),
    ]);
    // `add_target` guesses the category from the address; here it should be set explicitly.
    for (kind, host) in [
        (TargetKind::Gateway, "192.168.178.1"),
        (TargetKind::Lan, "192.168.178.42"),
        (TargetKind::Internet, "8.8.8.8"),
    ] {
        let t = app
            .targets
            .iter_mut()
            .find(|t| t.host == host)
            .expect("target present");
        t.kind = kind;
    }
    app
}

/// Whether a target is in the chart, the legend and the statistics — i.e. it passed every filter.
fn shown(app: &App, host: &str) -> bool {
    let id = app
        .targets
        .iter()
        .find(|t| t.host == host)
        .expect("target present")
        .id;
    app.shown_target_ids().contains(&id)
}

/// Hiding a category takes all of its targets out of the chart, the legend and the statistics.
/// Switching happens through `App::toggle_kind`; the chips in the header call exactly that.
#[test]
fn eine_kategorie_aus_versteckt_alle_ihre_ziele() {
    let mut app = one_of_each();
    assert_eq!(
        app.shown_target_ids().len(),
        3,
        "at first everything is visible"
    );

    app.toggle_kind(TargetKind::Lan);
    assert!(!shown(&app, "192.168.178.42"), "the NAS is gone");
    assert!(shown(&app, "192.168.178.1"), "the gateway stays");
    assert!(shown(&app, "8.8.8.8"), "the DNS server stays");

    // The switch also works for a category no target belongs to: doing nothing is fine.
    app.toggle_kind(TargetKind::Custom);
    assert_eq!(app.shown_target_ids().len(), 2);

    // "All" switches everything back on.
    app.show_all_kinds();
    assert_eq!(
        app.shown_target_ids().len(),
        3,
        "after the all switch everything is back"
    );
}

/// Clicking the name in the legend hides only this one target — the legend itself
/// still lists it, otherwise there would be no way to bring it back.
#[test]
fn ein_ausgeblendetes_ziel_bleibt_in_der_legende() {
    let mut app = one_of_each();
    let nas = app
        .targets
        .iter()
        .find(|t| t.host == "192.168.178.42")
        .unwrap()
        .id;

    assert!(app.toggle_hidden_by_id(nas), "the target is hidden now");
    assert!(
        !shown(&app, "192.168.178.42"),
        "it is missing from chart and statistics"
    );
    assert!(shown(&app, "192.168.178.1"), "the neighbours stay");

    // The legend has to keep naming it, otherwise there is no way back.
    let legend = app.visible_targets();
    assert!(legend.contains(&nas), "the legend still shows it");

    assert!(
        !app.toggle_hidden_by_id(nas),
        "a second click brings it back"
    );
    assert!(shown(&app, "192.168.178.42"));
}

/// The text filter also searches the category, otherwise "LAN device" would not be found although the
/// column shows exactly that.
#[test]
fn der_textfilter_findet_auch_die_kategorie() {
    let mut app = one_of_each();
    app.filter = String::from("LAN");
    assert!(shown(&app, "192.168.178.42"), "the category matches");
    assert!(!shown(&app, "8.8.8.8"), "the internet target does not");

    app.filter = String::from("dns");
    assert!(
        shown(&app, "8.8.8.8"),
        "the name matches, without caring about upper or lower case"
    );
    assert!(!shown(&app, "192.168.178.42"));

    app.filter = String::new();
    assert_eq!(
        app.shown_target_ids().len(),
        3,
        "an empty filter hides nothing"
    );
}

/// The filters together: a target is only visible when it passes all of them.
#[test]
fn text_und_kategoriefilter_gelten_gleichzeitig() {
    let mut app = one_of_each();
    app.filter = String::from("LAN");
    assert_eq!(app.shown_target_ids().len(), 1);

    // Now switch off the category the filter points at: nothing is left.
    app.toggle_kind(TargetKind::Lan);
    assert!(app.shown_target_ids().is_empty());

    app.show_all_kinds();
    app.filter = String::new();
    assert_eq!(app.shown_target_ids().len(), 3);
}
