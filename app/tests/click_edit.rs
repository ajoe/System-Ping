//! Clicking a target row in the left list must open that row's editor.
//!
//! These tests drive real egui passes with synthetic pointer events, because the click handling
//! of a row cannot be reached through `Response::clicked()`: the row is a `Frame`, and egui 0.31
//! registers a frame with `Sense::hover()` only. See `frame_click.rs` for that isolated fact.
//!
//! What the tests have to respect: hit-testing uses the rects of the *previous* pass, so a click
//! can only aim at what was drawn before it. Opening an editor makes the row much taller, which
//! moves every row below it — a later click has to be aimed at the new position.

use std::num::NonZeroUsize;

use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use system_ping::app::{App, AppOptions};

/// Viewport size for the synthetic input; only wide/tall enough to hold the panel.
fn viewport() -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(250.0, 600.0))
}

/// One egui pass with the given events. `t` keeps egui's clock strictly increasing: egui stores a
/// pointer history and asserts that time never runs backwards.
///
/// `Context::run` repeats its internal pass while the layout still changes, and every repeat
/// consumes the input — so a press would be forgotten before the release arrives. The option set
/// in the tests below pins it to exactly one pass per call, which is what makes a press in one
/// call and a release in the next behave like a real mouse click.
fn pass(ctx: &Context, app: &mut App, t: f64, events: Vec<Event>) {
    // Der Closure laeuft pro Durchlauf mehrmals, also wird hier reingebaut statt hineingezogen.
    let mut raw = RawInput {
        events,
        time: Some(t),
        predicted_dt: 0.016,
        ..Default::default()
    };
    raw.screen_rect = Some(viewport());
    // The `FullOutput` is not interesting here; the app state after the pass is.
    let _ = ctx.run(raw, |c| {
        // Draw the real app: this is what fills `app.row_rects` for the next pass.
        app.frame(c);
    });
}

/// A press of the primary button at `at`.
fn press(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }
}

/// The matching release.
fn release(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }
}

/// Where a row is right now, as the last pass drew it.
fn row_rect(app: &App, row: usize) -> Rect {
    app.row_rects()
        .get(row)
        .copied()
        .unwrap_or_else(|| panic!("Zeile {row} wurde nicht als Rect gemeldet"))
}

/// A full click at `spot`: move the pointer there, press, release. The pointer rests inside the
/// row the whole time, otherwise egui reports no click.
fn klick_auf(ctx: &Context, app: &mut App, spot: Pos2, t0: f64) {
    pass(ctx, app, t0, vec![Event::PointerMoved(spot)]);
    pass(ctx, app, t0 + 0.02, vec![press(spot)]);
    pass(ctx, app, t0 + 0.04, vec![release(spot)]);
}

/// A fresh app with the built-in targets (Gateway, DNS, Internet) in a throwaway state dir.
fn new_app(tag: &str) -> App {
    let dir = std::env::temp_dir().join(format!("sp-click-edit-{tag}"));
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);
    App::with_theme_override(AppOptions::default(), Vec::new(), None)
}

/// A context configured the way every click test needs it.
fn test_context() -> Context {
    let ctx = Context::default();
    ctx.set_pixels_per_point(1.0);
    ctx.options_mut(|o| o.max_passes = NonZeroUsize::new(1).unwrap());
    ctx
}

#[test]
fn ein_klick_auf_eine_zeile_oeffnet_den_editor() {
    let mut app = new_app("KlickOeffnet");
    assert!(app.targets.len() >= 3, "der Test braucht drei Zeilen");
    assert_eq!(app.edit_idx, None, "initial ist nichts in Bearbeitung");

    let ctx = test_context();

    // Erster Pass baut das Layout auf, damit egui weiss, wo die Zeilen liegen.
    pass(&ctx, &mut app, 0.5, vec![]);

    // Zeile 1 (DNS) anklicken.
    let spot = row_rect(&app, 1).center();
    klick_auf(&ctx, &mut app, spot, 1.0);
    assert_eq!(
        app.edit_idx,
        Some(1),
        "Klick auf Zeile 1 oeffnet deren Editor"
    );
    assert_eq!(app.selected, 1, "derselbe Klick markiert die Zeile");

    // Der Editor ist viel hoeher als die Zeile, die er ersetzt: alles unter ihm rueckt nach unten.
    // Ein Klick muss den Editor umhaengen - und zwar dorthin, wo die Zeile JETZT liegt, nicht wohin
    // sie vor dem Oeffnen zeigte. Der Ruhe-Pass laesst die Liste auf die neue Groesse kommen.
    let label0 = app.targets[0].label.clone();
    pass(&ctx, &mut app, 1.5, vec![]);
    let spot2 = row_rect(&app, 2).center();
    klick_auf(&ctx, &mut app, spot2, 2.0);
    assert_eq!(
        app.edit_idx,
        Some(2),
        "Klick auf Zeile 2 wechselt den Editor"
    );
    assert_eq!(
        app.targets[0].label, label0,
        "nichts am ersten Ziel veraendert"
    );

    // Und zurueck auf Zeile 0.
    pass(&ctx, &mut app, 2.5, vec![]);
    let spot0 = row_rect(&app, 0).center();
    klick_auf(&ctx, &mut app, spot0, 3.0);
    assert_eq!(app.edit_idx, Some(0), "Klick auf Zeile 0 wechselt zurueck");

    // Der Editor zeigt das richtige Ziel.
    assert_eq!(app.edit_label, "Gateway");
    assert_eq!(app.edit_host, "192.168.178.1");
}

#[test]
fn ein_klick_auf_die_offene_zeile_laesst_den_editor_stehen() {
    let mut app = new_app("KlickSteht");
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);

    // Zeile 0 oeffnen und dort etwas tippen.
    let spot = row_rect(&app, 0).center();
    klick_auf(&ctx, &mut app, spot, 1.0);
    assert_eq!(app.edit_idx, Some(0));
    app.edit_label = "entwurf".to_string();

    // Nochmal auf dieselbe Zeile klicken: der Editor bleibt samt Eingabe stehen. Ein Versehen kann
    // so nichts schliessen - dafuer sind Abbrechen, Ubernehmen, Esc und der Klick auf eine andere
    // Zeile da.
    let offen = row_rect(&app, 0).center();
    klick_auf(&ctx, &mut app, offen, 2.0);
    assert_eq!(
        app.edit_idx,
        Some(0),
        "erneuter Klick auf die offene Zeile laesst sie offen"
    );
    assert_eq!(app.edit_label, "entwurf", "die Eingabe bleibt erhalten");
    assert_eq!(app.targets[0].label, "Gateway", "nichts ist gespeichert");

    // Ein Klick ausserhalb der Liste (unter den Zeilen) tut dasselbe: nichts.
    let daneben = Pos2::new(125.0, 400.0);
    klick_auf(&ctx, &mut app, daneben, 3.0);
    assert_eq!(
        app.edit_idx,
        Some(0),
        "ein Klick ausserhalb der Liste laesst den Editor in Ruhe"
    );
}

#[test]
fn escape_closes_the_editor() {
    let mut app = new_app("esc");
    let ctx = test_context();

    pass(&ctx, &mut app, 0.5, vec![]);
    let spot = row_rect(&app, 0).center();
    klick_auf(&ctx, &mut app, spot, 1.0);
    assert_eq!(app.edit_idx, Some(0));

    let label = app.targets[0].label.clone();
    app.edit_label = "nicht speichern".to_string();

    pass(
        &ctx,
        &mut app,
        2.0,
        vec![Event::Key {
            key: Key::Escape,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
            physical_key: None,
        }],
    );
    assert_eq!(app.edit_idx, None, "Esc schließt den Editor");
    assert_eq!(app.targets[0].label, label, "und speichert nichts");
}
