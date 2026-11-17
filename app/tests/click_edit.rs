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
    // The closure runs several times per pass, so it is built in place here rather than moved in.
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

/// Where a row is right now, asked by the id of the entry it shows rather than by counting rows.
///
/// The panel reports each rect together with the entry it was drawn for (see [`App::row_rect_of`]), and a test that
/// says "row 1" means "the row showing the second entry", so that is how it asks. Pairing the rects with the list by
/// position instead would be wrong here as soon as something has reordered the list between the pass that drew them
/// and the one that reads them — which is exactly what a drag does.
fn row_rect(app: &App, row: usize) -> Rect {
    let id = app.targets[row].id;
    app.row_rect_of(id)
        .unwrap_or_else(|| panic!("row for target {id} was never reported as a rect"))
}

/// A full click at `spot`: move the pointer there, press, and let go a little way off.
///
/// The two points are deliberately not the same one. egui calls a finished press a click as long as the pointer stayed inside
/// `InputOptions::max_click_dist` of where it began — six points — and a hand that goes back to the exact pixel it pressed on is
/// something no real hand does. Releasing at the press point therefore tested a case the app never sees: it hid a version of the
/// gesture in which a pull answered "no click" for every row, so nothing recognised the drag and the editor opened on whatever row
/// the pointer happened to finish over. Two points three apart are still well inside the slop, so this remains a click, but now with
/// the wobble a click really has — and if the app ever stops trusting egui's own verdict and starts measuring positions again, this
/// is what fails.
fn click_at(ctx: &Context, app: &mut App, spot: Pos2, t0: f64) {
    pass(ctx, app, t0, vec![Event::PointerMoved(spot)]);
    pass(ctx, app, t0 + 0.02, vec![press(spot)]);
    let lifted = spot + Vec2::new(3.0, 1.0);
    pass(ctx, app, t0 + 0.03, vec![Event::PointerMoved(lifted)]);
    pass(ctx, app, t0 + 0.04, vec![release(lifted)]);
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
fn a_click_on_a_row_opens_its_editor() {
    let mut app = new_app("ClickOpens");
    assert!(app.targets.len() >= 3, "the test needs three rows");
    assert_eq!(app.edit_idx, None, "nothing is being edited at the start");

    let ctx = test_context();

    // The first pass builds the layout so egui knows where the rows are.
    pass(&ctx, &mut app, 0.5, vec![]);

    // Click row 1 (DNS).
    let spot = row_rect(&app, 1).center();
    click_at(&ctx, &mut app, spot, 1.0);
    assert_eq!(app.edit_idx, Some(1), "a click on row 1 opens its editor");
    assert_eq!(app.selected, 1, "the same click selects the row");

    // The editor is much taller than the row it replaces: everything below it moves down. A click
    // has to hang around the editor — and at the place where the row lies NOW, not where it pointed
    // before opening. The idle pass lets the list settle at its new size.
    let label0 = app.targets[0].label.clone();
    pass(&ctx, &mut app, 1.5, vec![]);
    let spot2 = row_rect(&app, 2).center();
    click_at(&ctx, &mut app, spot2, 2.0);
    assert_eq!(
        app.edit_idx,
        Some(2),
        "a click on row 2 switches the editor"
    );
    assert_eq!(
        app.targets[0].label, label0,
        "nothing changed on the first target"
    );

    // And back to row 0.
    pass(&ctx, &mut app, 2.5, vec![]);
    let spot0 = row_rect(&app, 0).center();
    click_at(&ctx, &mut app, spot0, 3.0);
    assert_eq!(app.edit_idx, Some(0), "a click on row 0 switches back");

    // The editor shows the right target.
    assert_eq!(app.edit_label, "Gateway");
    assert_eq!(app.edit_host, "192.168.178.1");
}

#[test]
fn a_click_on_the_open_row_leaves_the_editor_alone() {
    let mut app = new_app("ClickStands");
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);

    // Open row 0 and type something there.
    let spot = row_rect(&app, 0).center();
    click_at(&ctx, &mut app, spot, 1.0);
    assert_eq!(app.edit_idx, Some(0));
    app.edit_label = "draft".to_string();

    // Clicking the same row again: the editor stays open together with what was typed. A slip of
    // the mouse cannot close anything this way — cancel, save, Esc and clicking another row are
    // there for that.
    let open = row_rect(&app, 0).center();
    click_at(&ctx, &mut app, open, 2.0);
    assert_eq!(
        app.edit_idx,
        Some(0),
        "clicking the open row again leaves it open"
    );
    assert_eq!(app.edit_label, "draft", "the input is kept");
    assert_eq!(app.targets[0].label, "Gateway", "nothing is stored");

    // A click outside the list does the same thing: nothing. The point sits to the right of the
    // list and therefore outside every row in any case — a fixed point below the rows used to be
    // enough for that, but the editor is taller than a normal row and can cover it by now.
    let bottom = app
        .row_rects
        .iter()
        .flatten()
        .map(|r| r.bottom())
        .fold(f32::NEG_INFINITY, f32::max);
    let beside = Pos2::new(
        app.row_rects
            .iter()
            .flatten()
            .map(|r| r.right())
            .fold(0.0_f32, f32::max)
            + 40.0,
        bottom + 60.0,
    );
    click_at(&ctx, &mut app, beside, 3.0);
    assert_eq!(
        app.edit_idx,
        Some(0),
        "a click outside the list leaves the editor alone"
    );
}

#[test]
fn escape_closes_the_editor() {
    let mut app = new_app("EscCloses");
    let ctx = test_context();

    pass(&ctx, &mut app, 0.5, vec![]);
    let spot = row_rect(&app, 0).center();
    click_at(&ctx, &mut app, spot, 1.0);
    assert_eq!(app.edit_idx, Some(0));

    let label = app.targets[0].label.clone();
    app.edit_label = "do not save".to_string();

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
    assert_eq!(app.edit_idx, None, "Esc closes the editor");
    assert_eq!(app.targets[0].label, label, "and saves nothing");
}
