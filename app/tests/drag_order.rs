//! Holding a target row and pulling it up or down reorders the list.
//!
//! Like `click_edit.rs`, these tests drive real egui passes with synthetic pointer events, because
//! both halves of the gesture — "was this press a click or a drag?" and "which row is the pointer
//! over now?" — are decided inside egui's input state. The same rules apply: hit-testing uses the
//! rects of the previous pass, and `Context::run` is pinned to one pass per call so a press in one
//! call and a release in the next behave like a real mouse.
//!
//! What must hold for the gesture to be usable at all:
//! - a plain click still only opens the editor (the old behaviour, unchanged),
//! - a drag moves the entry and does *not* open an editor on the way,
//! - releasing over the row's own place leaves the order alone,
//! - and the new order survives a restart, because it is saved.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use system_ping::app::App;

/// One egui pass with the given events, drawing the real app so `app.row_rects` is filled for the
/// next one. See `click_edit.rs` for why exactly one internal pass is needed here.
fn pass(ctx: &Context, app: &mut App, t: f64, events: Vec<Event>) {
    let mut raw = RawInput {
        events,
        time: Some(t),
        predicted_dt: 0.016,
        ..Default::default()
    };
    // Wider than the target panel's 250 points on purpose: a viewport that narrow leaves no room for
    // a central panel, and egui then lays the side panel out differently than in a real window.
    raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 600.0)));
    let _ = ctx.run(raw, |c| app.frame(c));
}

fn press(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }
}

fn release(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }
}

fn moved(at: Pos2) -> Event {
    Event::PointerMoved(at)
}

/// A context configured the way every pointer test needs it.
fn test_context() -> Context {
    let ctx = Context::default();
    ctx.set_pixels_per_point(1.0);
    ctx.options_mut(|o| o.max_passes = NonZeroUsize::new(1).unwrap());
    ctx
}

/// A throwaway state folder that removes itself again. `Drop` is what guarantees the cleanup even
/// when a test fails on the way out — these folders hold nothing worth keeping.
///
/// Held by a guard, not by each test: the store takes its folder from one process-wide variable, so whoever holds the guard
/// owns where *every* test in this binary writes. One guard for the whole binary is what makes "the app wrote my file" mean
/// something — with a folder per test, any other test finishing first could move the variable and the save would land in its
/// folder instead. Tests that care about content therefore read this one path (see [`saved_labels`]) or hand their own
/// directory to `save_now_to`.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sp-drag-order-{tag}-{seq}",
            seq = SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp folder can be created");
        Self(dir)
    }

    /// The state file inside this folder. Every test names this exact path rather than asking the app where it writes, so a
    /// read is never aimed at a folder another test has claimed.
    fn state_file(&self) -> PathBuf {
        system_ping::store::state_path_in(&self.0)
    }

    /// The one folder every app in this binary saves to, pointed at once and left alone until the last test of the binary is
    /// done. A second call keeps the first folder — that is the whole point: nobody may move the variable out from under a
    /// save that has not happened yet.
    fn shared() -> &'static Path {
        static ONCE: OnceLock<TempDir> = OnceLock::new();
        let dir = ONCE.get_or_init(|| {
            let dir = TempDir::new("shared");
            // Set once, never unset: an app built after this point saves here, and so does one built before it.
            std::env::set_var("SYSTEM_PING_STATE_DIR", &dir.0);
            dir
        });
        &dir.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh app with the built-in targets (Gateway, DNS, Internet), saving into the binary's one shared throwaway folder (see
/// [`TempDir::shared`]). That folder is empty when the app starts, which is also what produces the built-in list — an app
/// without saved targets falls back to it, exactly as on a first real start.
fn new_app(tag: &str) -> App {
    // Touching the shared folder is what arms the variable; no test needs the path itself.
    let dir = TempDir::shared();
    // The tag only names the folder this test would have liked, so it still says which run a leftover came from.
    let _tag = TempDir::new(tag);
    new_app_in(dir)
}

/// The same app built against a folder of its own — used directly for the shared list, and by the one test that has to compare
/// a file it wrote itself against a file nothing else touched. `save_now_to` takes such a directory directly, so that test
/// never moves the process-wide variable out from under another one.
fn new_app_in(dir: &Path) -> App {
    // An empty folder gives the built-in list, exactly like a first real start; anything saved in it is read back instead. These
    // three (gateway, DNS, internet) are deliberately what every drag here drags — `dragging_a_row_to_the_front_works` spells
    // their names out, and that only works as long as the list really is theirs. See the note in `editing.rs` for why an app
    // with nothing saved is never empty.
    let (options, saved) = system_ping::store::load_from(dir);
    let mut app = App::with_theme_override(options, saved, None);
    // A window starts measuring on its own, so a test app has to do the same: a row only counts as
    // "new" while it is being carried when the list is actually live. Quietly, because the status bar is
    // what several of these tests read, and a start nobody pressed must not fill it with a sentence.
    app.start_quiet(true);
    // Then it is switched off again for the duration of the drag. Live probes would land in the middle
    // of one and redraw the list, which is fine by itself — but a test that asserts on an order has to
    // know what it is asserting about, and measurements arriving between two passes are not part of the
    // gesture under test. The history they leave behind stays, so the rows still look like real ones.
    app.stop_quiet(true);
    app
}

/// The labels in a state file, top to bottom. Read straight from disk rather than through
/// `store::load()`, whose folder a test in another binary may already own.
fn saved_labels(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).expect("the app wrote its state file");
    let value: serde_json::Value = serde_json::from_str(&text).expect("the state file is json");
    // `history` is what the store calls the list — the same thing the panel shows, and the name the
    // file has always used for it.
    value["history"]
        .as_array()
        .expect("the state file has a history")
        .iter()
        .map(|t| t["label"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The rect a row registered in the last pass, looked up by target rather than by position — during a
/// drag the two are not the same thing.
fn row_rect(app: &App, row: usize) -> Rect {
    let id = app.targets[row].id;
    app.row_rect_of(id)
        .unwrap_or_else(|| panic!("row {row} did not register a rect"))
}

fn row_center(app: &App, row: usize) -> Pos2 {
    row_rect(app, row).center()
}

fn labels(app: &App) -> Vec<String> {
    app.targets.iter().map(|t| t.label.clone()).collect()
}

/// Presses the centre of a row, drags to `to` in small steps, and releases there.
///
/// The path is deliberately not straight: walking from one row to another along the column the rows
/// occupy would cross the very slot the drag is meant to aim at on the way out, and egui only reports
/// the release once — so a straight line arrives back at a place it already visited. Going sideways
/// first and then straight down keeps every intermediate point in one column.
fn drag(ctx: &Context, app: &mut App, row: usize, to: Pos2, t0: f64) {
    let from = row_center(app, row);
    pass(ctx, app, t0, vec![]);
    pass(ctx, app, t0 + 0.02, vec![moved(from)]);
    pass(ctx, app, t0 + 0.04, vec![press(from)]);
    // Sideways first: leave the row's own column before moving vertically.
    let out = Pos2::new(to.x, from.y);
    pass(ctx, app, t0 + 0.06, vec![moved(out)]);
    let steps = 8;
    for i in 1..=steps {
        let f = i as f32 / steps as f32;
        let spot = Pos2::new(to.x, from.y + (to.y - from.y) * f);
        pass(ctx, app, t0 + 0.08 + 0.02 * f as f64, vec![moved(spot)]);
    }
    pass(ctx, app, t0 + 0.34, vec![release(to)]);
    // egui reports the click of a released drag one pass late, and the layout needs one more pass to
    // settle on the new order.
    pass(ctx, app, t0 + 0.36, vec![]);
    pass(ctx, app, t0 + 0.38, vec![]);
}

/// Everything a failing assertion would need to explain itself: the order, what is carried, and where
/// each row was last drawn.
fn trace(app: &App) -> String {
    let rows = app
        .row_rects()
        .iter()
        .map(|r| {
            r.map_or_else(
                || "none".to_string(),
                |r| format!("({:.0},{:.0})", r.min.y, r.max.y),
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "\n  order={:?}\n  drag_row={:?} selected={}\n  rows: {rows}",
        labels(app),
        app.drag_row,
        app.selected
    )
}

#[test]
fn dragging_a_row_down_moves_it() {
    let mut app = new_app("Down");
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);
    let before = labels(&app);
    assert_eq!(before, ["Gateway", "DNS (Google)", "Internet"]);

    // Gateway's row dragged down past DNS' centre.
    let target = row_center(&app, 1);
    drag(&ctx, &mut app, 0, Pos2::new(target.x, target.y + 8.0), 1.0);

    assert_eq!(
        labels(&app),
        ["DNS (Google)", "Gateway", "Internet"],
        "{}",
        trace(&app)
    );
    assert_eq!(app.drag_row, None, "the drag is over: {}", trace(&app));
    assert_eq!(app.edit_idx, None, "and no editor opened: {}", trace(&app));
}

#[test]
fn dragging_a_row_to_the_front_works() {
    let mut app = new_app("Front");
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);

    // The last row pulled up above the first one's centre.
    let target = row_center(&app, 0);
    drag(&ctx, &mut app, 2, Pos2::new(target.x, target.y - 8.0), 1.0);

    assert_eq!(
        labels(&app),
        ["Internet", "Gateway", "DNS (Google)"],
        "{}",
        trace(&app)
    );
}

#[test]
fn dropping_a_row_on_its_own_place_keeps_the_order() {
    let mut app = new_app("Same");
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);
    let before = labels(&app);

    // A small movement inside the row: enough for egui to call it a drag, not enough to leave it.
    let from = row_center(&app, 1);
    drag(&ctx, &mut app, 1, from + Vec2::new(0.0, 5.0), 1.0);

    assert_eq!(labels(&app), before, "{}", trace(&app));
    assert_eq!(app.drag_row, None);
}

#[test]
fn a_plain_click_still_only_opens_the_editor() {
    let mut app = new_app("Click");
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);
    let before = labels(&app);

    let at = row_center(&app, 1);
    pass(&ctx, &mut app, 1.0, vec![moved(at)]);
    pass(&ctx, &mut app, 1.02, vec![press(at)]);
    pass(&ctx, &mut app, 1.04, vec![release(at)]);
    pass(&ctx, &mut app, 1.06, vec![]);

    assert_ne!(
        app.edit_idx,
        None,
        "a click opens the editor: {}",
        trace(&app)
    );
    assert_eq!(labels(&app), before, "and moves nothing");
}

#[test]
fn an_open_editor_blocks_a_drag() {
    let mut app = new_app("Editing");
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);
    let before = labels(&app);

    // Open the editor for row 1 first, exactly as a click would.
    let at = row_center(&app, 1);
    pass(&ctx, &mut app, 1.0, vec![moved(at)]);
    pass(&ctx, &mut app, 1.02, vec![press(at)]);
    pass(&ctx, &mut app, 1.04, vec![release(at)]);
    pass(&ctx, &mut app, 1.06, vec![]);
    assert_ne!(app.edit_idx, None);

    // Then try to drag row 0 across it. The editor is still open, so nothing may move.
    let target = row_center(&app, 1);
    drag(&ctx, &mut app, 0, Pos2::new(target.x, target.y + 8.0), 2.0);

    assert_eq!(labels(&app), before, "{}", trace(&app));
}

#[test]
fn a_moved_row_is_written_to_the_state_file() {
    // A folder of its own, and a save aimed straight at it: the file is then this test's file whatever another test in this
    // binary has done to the process-wide variable. That is what makes the comparison below mean "the drag was written".
    let dir = TempDir::new("Saved");
    let mut app = new_app_in(&dir.0);
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);
    // Nothing has been written to this folder yet, so "the file says the old order" can only mean the drag's own save wrote it.
    let before = labels(&app);

    let target = row_center(&app, 1);
    let to = Pos2::new(target.x, target.y + 8.0);
    drag(&ctx, &mut app, 0, to, 1.0);

    app.save_now_to(&dir.0).expect("this folder can be written");
    let saved = saved_labels(&dir.state_file());
    assert_eq!(saved, labels(&app), "file and app agree");
    assert_ne!(saved, before, "and that is the new order");
}

/// The index a drop would use: above a row's centre the entry goes in front of it, below its centre
/// behind it. Tested on its own because the pointer geometry of a real drag makes the same claim
/// hard to read in a failing assertion.
#[test]
fn the_drop_index_follows_the_row_centre() {
    let rect = Rect::from_min_size(Pos2::new(0.0, 100.0), Vec2::new(200.0, 39.0));
    assert_eq!(App::drop_index_for(2, 105.0, rect), 2);
    assert_eq!(App::drop_index_for(2, 119.4, rect), 2);
    assert_eq!(App::drop_index_for(2, 120.1, rect), 3);
    assert_eq!(App::drop_index_for(2, 138.0, rect), 3);
}
