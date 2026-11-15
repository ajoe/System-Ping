//! Per-target trace colours: automatic assignment, manual override, persistence.

use system_ping::app::{App, AppOptions, AUTO_COLOR};
use system_ping::theme;

/// An app with the default targets and a fixed look, using its own state directory so nothing is
/// read from or written to the user's real settings.
fn new_app(tag: &str) -> App {
    let dir = std::env::temp_dir().join(format!("sp-colors-{tag}"));
    // Left over from an earlier run of this test? Then the app would start with those targets
    // instead of the default list, and the assertions below would compare against nothing.
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);
    App::with_theme_override(AppOptions::default(), Vec::new(), Some("dark"))
}

#[test]
fn jedes_ziel_kriegt_eigene_farbe() {
    let app = new_app("auto");
    assert!(
        app.targets.len() >= 3,
        "der Test braucht mehrere Ziele, um trennen zu können"
    );
    // Automatic means: nothing chosen, and the palette decides by position.
    for t in &app.targets {
        assert_eq!(t.color_idx, AUTO_COLOR);
    }
    let mut used = Vec::new();
    for t in &app.targets {
        let idx = app.color_idx_of(t);
        assert!(
            idx < theme::PALETTE_LEN,
            "farbe {idx} ist nicht in der Palette"
        );
        assert!(!used.contains(&idx), "Ziel {} teilt sich eine Farbe", t.id);
        used.push(idx);
        // What the chart paints is exactly that palette entry, in the shade of the current look.
        assert_eq!(
            app.trace_color(t),
            theme::swatch(idx).color(theme::Mode::Dark)
        );
    }
}

#[test]
fn manuelle_farbe_verdraengt_die_automatik() {
    let mut app = new_app("manuell");
    // Ziel 0 gets a fixed colour; the automatic ones have to route around it.
    app.targets[0].color_idx = 1;
    let mut used = vec![app.color_idx_of(&app.targets[0])];
    assert_eq!(used[0], 1, "eine gewählte Farbe gilt unverändert");
    for t in app.targets.iter().skip(1) {
        let idx = app.color_idx_of(t);
        assert!(!used.contains(&idx), "Ziel {} teilt sich eine Farbe", t.id);
        used.push(idx);
    }
}

#[test]
fn mehr_ziele_als_farben_wiederverwenden_die_palette() {
    let mut app = new_app("ueberlauf");
    // Beyond the palette size a colour has to be used twice; that must still be a valid entry and
    // must never leave a target without a line colour.
    while app.targets.len() <= theme::PALETTE_LEN + 2 {
        let n = app.targets.len();
        app.add_named_target(&format!("10.0.0.{n}"), &format!("Ziel {n}"));
    }
    for t in &app.targets {
        let idx = app.color_idx_of(t);
        assert!(idx < theme::PALETTE_LEN);
        assert_eq!(
            app.trace_color(t),
            theme::swatch(idx).color(theme::Mode::Dark)
        );
    }
}

#[test]
fn gewaehlte_farbe_ueberlebt_einen_neustart() {
    let mut app = new_app("persist");
    app.targets[1].color_idx = 5;
    app.save_now();

    // What the state file holds is the palette number, not a colour.
    let (options, saved) = system_ping::store::load();
    assert_eq!(saved[1].color_idx, 5);
    assert_eq!(saved[0].color_idx, AUTO_COLOR);

    let again = App::with_theme_override(options, saved, Some("dark"));
    assert_eq!(again.targets[1].color_idx, 5);
    assert_eq!(
        again.trace_color(&again.targets[1]),
        theme::swatch(5).color(theme::Mode::Dark)
    );
}

#[test]
fn ein_altes_profil_ohne_farbe_bekommt_automatik() {
    // Ein Profil, dem das neue Feld fehlt, muss weiterladen: serde nimmt dann den Feld-Standard,
    // und der ist die automatische Farbe. Der Weg führt durch dieselbe Datei und denselben Loader,
    // den die App beim Start benutzt — ein ausgedachtes JSON würde an jeder Kleinigkeit scheitern,
    // die nichts mit der Farbe zu tun hat.
    let dir = std::env::temp_dir().join("sp-colors-alt");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);

    // First a real state file, then the same file with the colour field taken out again.
    let mut app = App::with_theme_override(AppOptions::default(), Vec::new(), Some("dark"));
    app.targets.truncate(1);
    app.save_now_to(&dir).unwrap();
    let path = system_ping::store::state_path_in(&dir);
    let mut written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let removed = written["history"][0]
        .as_object_mut()
        .unwrap()
        .remove("color_idx");
    assert!(removed.is_some(), "ein aktuelles Profil enthält die Farbe");
    std::fs::write(&path, serde_json::to_string(&written).unwrap()).unwrap();

    let (options, saved) = system_ping::store::load_from(&dir);
    assert_eq!(saved.len(), 1, "das alte Profil muss sich laden lassen");
    assert_eq!(saved[0].color_idx, AUTO_COLOR);

    // And the loaded profile is a normal starting point for the app.
    let app = App::with_theme_override(options, saved, Some("dark"));
    assert_eq!(app.targets.len(), 1);
    assert_eq!(app.targets[0].color_idx, AUTO_COLOR);
    assert!(app.color_idx_of(&app.targets[0]) < theme::PALETTE_LEN);
}
