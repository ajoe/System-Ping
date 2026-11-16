// Checks the inline editor for target rows: a single click opens it, save takes over name,
// address, category and the measuring switch, and nothing may move the list while a row is open.

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

/// A click on a row selects it and opens the editor right away.
#[test]
fn click_selects_and_opens_the_editor() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);
    assert_eq!(app.edit_idx, None, "nothing is being edited at the start");

    app.begin_edit(1);
    assert_eq!(app.selected, 1, "the row is selected");
    assert_eq!(app.edit_idx, Some(1));
    assert_eq!(app.edit_host, "127.0.0.1");
    assert_eq!(app.edit_label, "B");
    assert!(app.edit_enabled, "a new target is measured");

    // The editor shows the target's real values, not some leftovers.
    app.begin_edit(0);
    assert_eq!(app.edit_idx, Some(0));
    assert_eq!(app.edit_label, "A");
}

/// An empty name is allowed: `commit_edit` then keeps the old one.
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
    assert_eq!(app.targets[0].id, id, "the target stays the same");
    assert_eq!(app.targets[0].series.len(), 1, "the series is kept");
    assert_eq!(app.edit_idx, None);
    assert!(app.status_msg.contains("neu"), "{}", app.status_msg);
}

/// Changing the address has to stop the running probes and reset the status.
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
        "the old measurement must not stand for the new address"
    );
    // The other target stays untouched.
    assert_eq!(app.targets[1].host, "127.0.0.1");
    assert_eq!(app.targets[1].last_rtt, 5.0);
}

/// An empty address must not destroy the target.
#[test]
fn an_empty_host_is_refused() {
    let mut app = app_with(vec![("A", "127.0.0.1")]);
    app.begin_edit(0);
    app.edit_host = "   ".to_string();
    app.edit_label = "kaputt".to_string();
    app.commit_edit();

    assert_eq!(app.targets[0].host, "127.0.0.1");
    assert_eq!(app.targets[0].label, "A", "the name stays unchanged too");
    assert!(app.status_msg.contains("verworfen"), "{}", app.status_msg);
    assert_eq!(app.edit_idx, None);
}

/// The category picker overrides the automatic classification as long as the address stays the same.
#[test]
fn the_category_can_be_set_by_hand() {
    let mut app = app_with(vec![("Drucker", "192.168.178.1")]);
    assert_eq!(app.targets[0].kind, TargetKind::Gateway);

    app.begin_edit(0);
    assert_eq!(app.edit_kind, TargetKind::Gateway, "vorbelegt");
    app.edit_kind = TargetKind::Lan;
    app.commit_edit();

    assert_eq!(app.targets[0].kind, TargetKind::Lan);

    // A new address, on the other hand, is classified automatically again.
    app.begin_edit(0);
    app.edit_kind = TargetKind::Custom;
    app.edit_host = "8.8.8.8".to_string();
    app.commit_edit();
    assert_eq!(
        app.targets[0].kind,
        TargetKind::Dns,
        "on an address change, classify() decides again"
    );
}

/// "measuring off" pauses the target without deleting its history.
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
    assert_eq!(app.targets[0].series.len(), 1, "the series is kept");
    // The status line and the list row speak the language of the app, so the test asks for their
    // texts instead of hard-coding German words.
    let lang = app.lang();
    assert!(
        app.status_msg.contains(&lang.paused().to_lowercase()),
        "{}",
        app.status_msg
    );
    assert_eq!(app.status_text(&app.targets[0]), lang.paused());

    app.begin_edit(0);
    assert!(!app.edit_enabled, "the editor shows the current state");
    app.edit_enabled = true;
    app.commit_edit();
    assert!(app.targets[0].enabled);
}

/// Cancelling changes nothing.
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

/// Switching to another row loses no input.
#[test]
fn switching_rows_commits_the_previous_edit() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);

    app.begin_edit(0);
    app.edit_label = "A umbenannt".to_string();
    app.begin_edit(1);

    assert_eq!(app.targets[0].label, "A umbenannt");
    assert_eq!(app.edit_idx, Some(1));
    assert_eq!(app.edit_label, "B", "the second editor is freshly filled");
}

/// While an entry is open, the list must not be re-sorted.
#[test]
fn the_list_cannot_change_while_editing() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);
    app.begin_edit(1);

    app.new_host = "10.0.0.1".to_string();
    app.new_label = "C".to_string();
    app.add_target();
    assert_eq!(app.targets.len(), 2, "no new target while editing");

    app.remove_selected();
    assert_eq!(app.targets.len(), 2, "no target removed");

    // The index still points at the same target.
    assert_eq!(app.edit_idx, Some(1));
    assert_eq!(app.targets[1].label, "B");
    assert!(app.status_msg.contains("Bearbeitung"), "{}", app.status_msg);

    // After closing, both work again.
    app.commit_edit();
    app.new_host = "10.0.0.1".to_string();
    app.add_target();
    assert_eq!(app.targets.len(), 3);
}

/// Removal now sits in the editor of the open target and no longer in the top right, where it hit
/// hit whichever target was selected last. The button itself is UI; what is checked here is the same
/// the editor's header goes: close the editor, set the index, remove.
#[test]
fn removing_from_the_editor_targets_that_very_target() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);

    // In the top right the selected target was meant — deliberately another one here.
    app.selected = 0;
    // The editor button's path: it points at the row that is open right now.
    app.begin_edit(1);
    app.cancel_edit();
    app.selected = 1;
    app.remove_selected();

    assert_eq!(app.targets.len(), 1);
    assert_eq!(app.targets[0].label, "A", "B is gone, A stays");
}

/// The editor button removes the right target even when a row was open beforehand: the editor is
/// closed first so no index points into nowhere.
#[test]
fn the_editor_closes_before_its_target_is_removed() {
    let mut app = app_with(vec![("A", "127.0.0.1"), ("B", "127.0.0.1")]);
    app.begin_edit(1);

    // Exactly the order `ui::left_body` runs after its row loop.
    app.cancel_edit();
    app.selected = 1;
    app.remove_selected();

    assert_eq!(
        app.edit_idx, None,
        "no editor points at a removed row anymore"
    );
    assert_eq!(app.targets.len(), 1);
}
