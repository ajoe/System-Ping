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

use egui::{pos2, Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use system_ping::app::App;

/// One egui pass with the given events, drawing the real app so the pass that follows sees the
/// layout this one produced. See `click_edit.rs` for why exactly one internal pass is needed here.
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
    let _ = system_ping::headless::paint(ctx, raw, |root| app.frame_ui(root));
}

/// What egui itself says about the pointer on this pass, in the same words the app reads it in.
///
/// A drag that goes wrong has more than one way to fail, and the order alone does not say which: `hit` names the widget a press belongs to from
/// the moment the button goes down, `drag` only once the pointer has left egui's slop, and `click` is egui's verdict that a press never did. A pull
/// whose lines show `hit=#n` but never `drag=#n` was never picked up — which is exactly how the first version of this gesture failed in a real window
/// while every assertion on the order still looked plausible. Reading egui's answers rather than only our own bookkeeping is what closes that gap,
/// and it is why these tests aim at two different points for one press: an app that measured positions instead of asking egui would answer those
/// lines differently from how a real hand is answered.
///
/// `label` marks the line so the steps of one gesture can be told apart, and `pos` is where that step put the hand — which is what every slot in
/// play was decided from. Only read when a test asks to see it: run with `cargo test --test drag_order -- <name> --nocapture`, and see [`trace`],
/// which every assertion already prints for the app's own side of the same gesture.
fn drag_report(ctx: &Context, app: &App, label: &str, pos: Option<Pos2>) {
    let pos = pos.map_or_else(|| "-".to_string(), |p| format!("({:.1}, {:.1})", p.x, p.y));
    eprintln!(
        "  [{label} @{pos}] egui: {}{}",
        system_ping::ui::gesture_report(ctx),
        trace(app)
    );
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
    // And every finished drag's write is wiped from the folder before this app starts reading it. Tests share one folder (see
    // [`TempDir::shared`], and why there is only one), while each gesture ends in `App::drop_moved_target` saving the order it
    // reached — so without this, an earlier test's list is what the next one loads, and a test that spells out three names finds
    // somebody else's order instead. Only the file goes: an app that finds nothing saved falls back to the built-in list, which is
    // exactly what every drag here assumes. The file name is spelled out rather than taken from the store because there it is a
    // private constant — and a test that silently loads the wrong folder is worse than one that fails.
    let _ = std::fs::remove_file(dir.join("system-ping-state.json"));
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
///
/// A row whose entry is being edited draws a form instead of a strip and therefore registers nothing at all, so this cannot answer for
/// it. That is not a gap in the helper: while a form is open no reordering happens anywhere (see `editing.rs`), so a gesture that wants
/// such a row has to be aimed at where it will stand once the form is closed — which is what [`row_center_when_closed`] does.
fn row_rect(app: &App, row: usize) -> Rect {
    let id = app.targets[row].id;
    app.row_rect_of(id)
        .unwrap_or_else(|| panic!("row {row} did not register a rect"))
}

fn row_center(app: &App, row: usize) -> Pos2 {
    row_rect(app, row).center()
}

/// The panel's empty line between two whole rows, used only as a stand-in for a gap no drawn row could answer for — a list of one entry has
/// none to measure. Spelled out rather than inferred so a change in the panel shows up as a wrong aim rather than as a test that quietly
/// measures nothing.
const GAP_Y: f32 = 9.0;

/// The list laid out with every editor closed, as far as it can be measured from one that is open.
///
/// Two things have to be answered for that, and both come from the rows that *were* drawn rather than from constants: how tall each row's strip
/// is, and how wide the line under it is. An edited row answers both — its header keeps the finished row's place, and the form hangs below it —
/// but the line measured under that header is the form's, not the list's, so it is replaced by what the untouched pairs of the same list measure.
///
/// Rows come out in the order of [`App::targets`] because that is the numbering a drop is answered in; during a drag the same place on screen
/// means a different row, which is why geometry is matched to entries by id here (see [`row_rect`]).
struct ClosedLayout {
    /// A whole row's strip height, taken from any row that has one.
    strip: f32,
    /// The empty line under each entry, in list order.
    gaps: Vec<f32>,
    /// Where the first strip stood when it was drawn, so the answer can be given in real coordinates rather than relative to the top of a list.
    top: Pos2,
}

impl ClosedLayout {
    fn measure(app: &App) -> Self {
        // One rect per entry, holes included, in the order of [`App::targets`] — which is already the order the panel drew them in, because a
        // pass publishes its layout only after its live move (see `ui::left_body`). Sorting by height on screen would be wrong here: during a
        // drag the carried entry has another row's rect while standing in its own place, and it is the places this helper stacks.
        let rows: Vec<Option<Rect>> = app.targets.iter().map(|t| app.row_rect_of(t.id)).collect();
        let strip = rows
            .iter()
            .flatten()
            .map(|rect| rect.height())
            .max_by(|a, b| a.total_cmp(b))
            .expect("the list has been drawn at least once");

        // Every gap that two drawn strips can answer for, in list order. Where no strip was drawn for one of the two — an entry pushed past the
        // panel's bottom edge, or one added since the last pass — nothing honest is known yet, and that one stays `None` and is filled below.
        let mut gaps: Vec<Option<f32>> = vec![None; rows.len()];
        for above in 0..rows.len() {
            if let (Some(a), Some(b)) = (rows[above], rows.get(above + 1).copied().flatten()) {
                gaps[above] = Some(b.min.y - a.max.y);
            }
        }

        // The line under an edited row is the ordinary one: its header keeps the finished row's place, so closing the form only pulls the rows
        // below it up to where the line between any two other rows would have put them. That line is what the untouched pairs of this very list
        // measure, and the smallest of those is it — the distance the form itself leaves is the form's business, not the list's, and guessing it
        // is what made a drop aimed at a row under an open form land a hand's breadth too high. Only a list too short to have an untouched pair
        // has to fall back to the constant.
        if let Some(edited) = app.edit_idx {
            gaps[edited] = (0..rows.len())
                .filter(|above| *above != edited)
                .filter_map(|above| gaps[above])
                .min_by(f32::total_cmp);
        }
        let gaps = gaps.into_iter().map(|gap| gap.unwrap_or(GAP_Y)).collect();
        // The top of the drawn list. Every row measured below hangs from this point, which is what lets the answer be a place in the window
        // rather than a distance — and it has to come from a row that really was drawn, because where the panel's first strip sits depends on
        // the header above it, not on anything a test could know.
        let top = rows
            .iter()
            .flatten()
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .expect("the list has been drawn at least once")
            .min;
        ClosedLayout { strip, gaps, top }
    }

    /// The rect an entry's strip will have once every editor is closed.
    fn rect(&self, row: usize) -> Rect {
        let mut y = self.top.y;
        for above in 0..row {
            y += self.strip + self.gaps[above];
        }
        Rect::from_min_size(pos2(self.top.x, y), Vec2::new(1.0, self.strip))
    }

    /// The centre an entry's strip will have once every editor is closed.
    fn center(&self, row: usize) -> Pos2 {
        self.rect(row).center()
    }
}

/// Where a row's centre will be once the open editor is closed and every row below it has moved back up into its place.
///
/// Everything about that layout is measured off the rows that really were drawn (see [`ClosedLayout`]); this only asks it for one entry and hands
/// back a point to aim at. Which is the whole difficulty of dropping on a list with a form in it: while the form is drawn, every row under it sits
/// somewhere else than where it will stand again, and a pointer aimed by today's geometry asks for the slot *above* the one the hand is on.
fn row_center_when_closed(app: &App, row: usize) -> Pos2 {
    ClosedLayout::measure(app).center(row)
}

fn labels(app: &App) -> Vec<String> {
    app.targets.iter().map(|t| t.label.clone()).collect()
}

/// Presses the centre of a row, drags to `to` in small steps, and releases there.
fn drag(ctx: &Context, app: &mut App, row: usize, to: Pos2, t0: f64) {
    let from = row_center(app, row);
    drag_to(ctx, app, from, to, t0);
}

/// The same gesture aimed by two points instead of by a row number.
///
/// Needed wherever the row being carried and the row being dropped on cannot both be looked up in one layout: while an editor's form is
/// drawn, every row below it sits somewhere else than where it will stand again (see [`row_center_when_closed`]), so a press has to name
/// a real rect and a destination has to name a future one.
///
/// The path is deliberately not straight: walking from one row to another along the column the rows
/// occupy would cross the very slot the drag is meant to aim at on the way out, and egui only reports
/// the release once — so a straight line arrives back at a place it already visited. Going sideways
/// first and then straight down keeps every intermediate point in one column.
fn drag_to(ctx: &Context, app: &mut App, from: Pos2, to: Pos2, t0: f64) {
    drag_report(ctx, app, "start", None);
    pass(ctx, app, t0, vec![]);
    drag_report(ctx, app, "hover", None);
    pass(ctx, app, t0 + 0.02, vec![moved(from)]);
    drag_report(ctx, app, "press-here", None);
    pass(ctx, app, t0 + 0.04, vec![press(from)]);
    // Sideways first: leave the row's own column before moving vertically.
    let out = Pos2::new(to.x, from.y);
    pass(ctx, app, t0 + 0.06, vec![moved(out)]);
    drag_report(ctx, app, "sideways", Some(out));
    let steps = 8;
    for i in 1..=steps {
        let f = i as f32 / steps as f32;
        let spot = Pos2::new(to.x, from.y + (to.y - from.y) * f);
        pass(ctx, app, t0 + 0.08 + 0.02 * f as f64, vec![moved(spot)]);
        drag_report(ctx, app, "travel", Some(spot));
    }
    pass(ctx, app, t0 + 0.34, vec![release(to)]);
    drag_report(ctx, app, "release", Some(to));
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
        .enumerate()
        .map(|(row, r)| {
            // The entry each slot currently holds, since a failing order assertion is only half the story: what a drop asked for is decided by
            // which row the pointer stood in, and that question has an answer per *slot*, not per entry.
            let label = app.targets.get(row).map_or("?", |t| t.label.as_str());
            r.map_or_else(
                || format!("#{row} {label}:none"),
                |r| format!("#{} {label}:({:.0},{:.0})", row, r.min.y, r.max.y),
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "\n  order={:?}\n  edit={:?} drag_row={:?} selected={}\n  rows: {rows}",
        labels(app),
        app.edit_id(),
        app.drag_row,
        app.selected
    )
}

/// The three rows every test here that does not name its own list drags around. Spelled out rather than left to the built-in fallback:
/// an app with nothing saved does install exactly these three, but a previous test's drag writes an order to the shared folder (see
/// [`new_app_in`]), and then "the first row" is somebody else's entry while the names still read as if they were not.
const THREE: [(&str, &str); 3] = [
    ("Gateway", "127.0.0.1"),
    ("DNS (Google)", "127.0.0.1"),
    ("Internet", "127.0.0.1"),
];

#[test]
fn dragging_a_row_down_moves_it() {
    let mut app = new_app("Down");
    app.set_test_targets(&THREE);
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
    app.set_test_targets(&THREE);
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
    app.set_test_targets(&THREE);
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
    app.set_test_targets(&THREE);
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);
    let before = labels(&app);

    // Held and let go a few points away, the way a hand actually works: egui still calls that a click — it only stops doing so
    // past `InputOptions::max_click_dist` — but a test that released on the exact press point would pass against an app that had
    // forgotten how to tell a click from a pull. See `click_edit::click_at`, which is where that once went unnoticed.
    let at = row_center(&app, 1);
    pass(&ctx, &mut app, 1.0, vec![moved(at)]);
    pass(&ctx, &mut app, 1.02, vec![press(at)]);
    let lifted = at + Vec2::new(3.0, 1.0);
    pass(&ctx, &mut app, 1.03, vec![moved(lifted)]);
    pass(&ctx, &mut app, 1.04, vec![release(lifted)]);
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
fn a_drag_released_far_from_where_it_began_lands_and_opens_nothing() {
    let mut app = new_app("FarRelease");
    app.set_test_targets(&THREE);
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);

    // The press point and the release point are a whole row apart. That is the case an earlier version of the click test could not
    // tell apart from a real pull: both end with the button up, and only egui's own verdict on how far the pointer travelled says
    // which one was which. So this asserts both halves of that verdict at once — the entry moved, and no editor opened anywhere.
    let target = row_center(&app, 1);
    drag(&ctx, &mut app, 0, Pos2::new(target.x, target.y + 8.0), 1.0);

    assert_eq!(
        labels(&app),
        ["DNS (Google)", "Gateway", "Internet"],
        "{}",
        trace(&app)
    );
    assert_eq!(
        app.edit_idx,
        None,
        "a pull must not open the editor on the row it ended over: {}",
        trace(&app)
    );
    assert_eq!(app.drag_row, None, "the drag is over: {}", trace(&app));
}

#[test]
fn a_press_that_never_left_the_row_opens_nothing_but_the_editor() {
    let mut app = new_app("Slop");
    app.set_test_targets(&THREE);
    let ctx = test_context();
    pass(&ctx, &mut app, 0.5, vec![]);
    let before = labels(&app);

    // Held a while, moved within the slop, then let go: egui never calls this a drag, so the list must not move and the row's
    // editor must open — even though `App::drag_row` was never set on the way. This is the other half of the same decision, and
    // it is what guards against an app that grabs every held press as a drag and then has nothing to show for a click.
    let at = row_center(&app, 2);
    pass(&ctx, &mut app, 1.0, vec![moved(at)]);
    pass(&ctx, &mut app, 1.02, vec![press(at)]);
    for i in 1..6 {
        let spot = at + Vec2::new(0.0, i as f32 * 0.5);
        pass(&ctx, &mut app, 1.04 + 0.02 * i as f64, vec![moved(spot)]);
    }
    pass(&ctx, &mut app, 1.2, vec![release(at + Vec2::new(0.0, 2.5))]);
    pass(&ctx, &mut app, 1.22, vec![]);

    assert_eq!(labels(&app), before, "{}", trace(&app));
    assert_ne!(
        app.edit_idx,
        None,
        "a held press that stayed put is still a click: {}",
        trace(&app)
    );
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

/// The other half of `an_open_editor_blocks_a_drag`, and the test whose absence let the last bug through twice: a drop *below* an open
/// editor has to land where the pointer is, and not one place too high.
///
/// A row whose entry is being edited draws a form instead of a strip and keeps a `None` in the layout so nothing below it shifts (see
/// `ui::rows_prev_pass`) — which means "row 2 of the layout" and "entry 2 of the list" stop being the same sentence as soon as a form sits
/// above them. Measured by the layout, a drop on the third row moved the entry to second; measured by the list, it goes where the hand is.
/// Every other test here drags through a list of strips only, where both numberings agree and the mistake cannot show itself.
///
/// Two gestures, because one cannot ask both questions: while a form is open `App::move_target` refuses every reordering outright (see
/// `editing.rs::a_moved_row_waits_for_the_editor_to_close`), so a drop there can only prove that nothing moves — and the second gesture then
/// starts on a layout that has already caught up with its list. The pass where the two still disagree is the next test's subject.
#[test]
fn a_drop_below_an_open_editor_lands_where_the_pointer_is() {
    let mut app = new_app("EditorBelow");
    let ctx = test_context();
    // Six rows so there is anything below the form at all — with three rows and a form on the first, a one-place error is the whole
    // list and a failing assertion would not say which way it went.
    app.set_test_targets(&[
        ("Eins", "127.0.0.1"),
        ("Zwei", "127.0.0.1"),
        ("Drei", "127.0.0.1"),
        ("Vier", "127.0.0.1"),
        ("Fünf", "127.0.0.1"),
        ("Sechs", "127.0.0.1"),
    ]);
    pass(&ctx, &mut app, 0.5, vec![]);

    // Open the editor on the first row, exactly as a click there would.
    let at = row_center(&app, 0);
    pass(&ctx, &mut app, 1.0, vec![moved(at)]);
    pass(&ctx, &mut app, 1.02, vec![press(at)]);
    pass(&ctx, &mut app, 1.04, vec![release(at)]);
    pass(&ctx, &mut app, 1.06, vec![]);
    assert_eq!(app.edit_id(), Some(app.targets[0].id), "the form is open");

    // First gesture: drag the last row up to the third one while the form is still open. The press has to be aimed at where that row
    // really stands, which [`row_center`] gives for every entry that drew a strip; only the destination can be aimed at by where it will
    // stand once the form is closed, and that is what [`row_center_when_closed`] is for (see the next test). The column stays the one
    // every other drag in this file uses, since how far in a pointer sits is not part of what a drop asks about.
    let from = row_center(&app, 5);
    let to = Pos2::new(125.0, row_center_when_closed(&app, 3).y + 8.0);
    drag_to(&ctx, &mut app, from, to, 2.0);

    // Nothing moved: the refusal is what a hand that grabs a row under an open form gets, and it has to stay that way — a list that
    // shuffled under an open editor would move the entry the form is editing out from under it.
    assert_eq!(
        labels(&app),
        ["Eins", "Zwei", "Drei", "Vier", "Fünf", "Sechs"],
        "a form that is open refuses the reorder, {}",
        trace(&app)
    );

    // Second gesture, same place, with the form cancelled: cancelling rather than committing because a commit would write an edit nobody
    // made. This is the drop that has to land right, and it does not matter here whether the layout still lags by a pass — [`drag`] starts
    // by drawing one, so by the release both numberings agree again and the only thing left to prove is that a counted-by-list slot lands on
    // the row the pointer rests on rather than above it. Where they disagree is the next test.
    app.cancel_edit();
    pass(&ctx, &mut app, 5.0, vec![]);

    // Aimed at the *fourth* row as it stands with every editor closed — the press is on the last row's strip and the release inside the fourth
    // row's strip, so the hand has travelled two rows up and the entry has to come to rest there. Aimed a little above that row's middle rather
    // than on it or below it, and that choice is the point of the gesture: a point on a row's middle or below it names the place *behind* the row
    // (see `drop_index_for_row`), which for an entry carried up from further down is its own place and so means "stay". Only the row's upper half
    // names the place in front of it, which is the one place in this list an entry pulled up from last place can only reach by being measured where
    // the pointer really is.
    let spot = Pos2::new(125.0, row_center_when_closed(&app, 3).y - 8.0);
    drag(&ctx, &mut app, 5, spot, 6.0);

    // Fourth place, in front of the row the pointer rests on. The live follow had already carried the entry there on the way up, and the release
    // confirms the same slot rather than asking for a different one — which is the property this test is really about: what the hand sees while it
    // travels is what the list keeps when it lets go, so a drop can never land somewhere the pointer never went.
    assert_eq!(
        labels(&app),
        ["Eins", "Zwei", "Drei", "Sechs", "Vier", "Fünf"],
        "{}",
        trace(&app)
    );
}

/// The one pass where the layout and the list disagree: the release lands on the very frame that closes the form, so the panel is still
/// measuring against the tall form of the pass before — a `None` where the next pass will have a strip. A slot counted by the layout then
/// names a row that does not exist yet in the list, and the entry lands one place too high; `row_slot` (see `ui::row_at`) is what keeps the
/// two numberings apart. No other test crosses this pass: they all either drop with a form open, where nothing moves by rule, or after the
/// panel has drawn the list whole again, where both numberings already agree.
#[test]
fn a_drop_on_the_pass_that_closes_an_editor_lands_where_the_pointer_is() {
    let mut app = new_app("EditorSamePass");
    let ctx = test_context();
    app.set_test_targets(&[
        ("Eins", "127.0.0.1"),
        ("Zwei", "127.0.0.1"),
        ("Drei", "127.0.0.1"),
        ("Vier", "127.0.0.1"),
        ("Fünf", "127.0.0.1"),
        ("Sechs", "127.0.0.1"),
    ]);
    pass(&ctx, &mut app, 0.5, vec![]);

    // Open the editor on the first row.
    let at = row_center(&app, 0);
    pass(&ctx, &mut app, 1.0, vec![moved(at)]);
    pass(&ctx, &mut app, 1.02, vec![press(at)]);
    pass(&ctx, &mut app, 1.04, vec![release(at)]);
    pass(&ctx, &mut app, 1.06, vec![]);
    assert_eq!(app.edit_id(), Some(app.targets[0].id), "the form is open");

    // Grab the last row and carry it up, all of it with the form still open — which is what leaves the layout behind: `row_rects` is
    // published once per drawn pass, so every pass of this travel measures against a list whose first row is still a form. The press has to
    // land on a strip that exists, and the last row's does: only the edited row trades its strip for a form.
    let from = row_center(&app, 5);

    // Aimed at a row's *upper half*, and that is not a fudge — it is the only place that asks for what this test needs. A point in a row's gap
    // names the place behind it, and so does a point on its middle or lower half, which `App::move_target` then reads as "stay where you are" for
    // an entry carried up from further down (see there). Only the upper half names the place in front of a row.
    //
    // Which row that has to be follows from how far the pointer travelled: while the form is open no live move can happen at all (see
    // `editing.rs::a_moved_row_waits_for_the_editor_to_close`), so the entry never follows the pointer and stands in its own place the whole time.
    // A release that names the place in front of the fourth row, in a list whose carried entry sits last, is the move this test wants — and only a
    // point in that row's upper half answers with it. So the aim is the fourth entry's strip a little above its middle, measured as it will stand
    // once every editor is closed and every row has moved back up into its place (see [`row_center_when_closed`]). The column is the one every
    // other drag here uses.
    let to = Pos2::new(125.0, row_center_when_closed(&app, 3).y - 8.0);

    pass(&ctx, &mut app, 2.0, vec![]);
    pass(&ctx, &mut app, 2.02, vec![moved(from)]);
    drag_report(&ctx, &app, "form-open press", Some(from));
    pass(&ctx, &mut app, 2.04, vec![press(from)]);
    let out = Pos2::new(to.x, from.y);
    pass(&ctx, &mut app, 2.06, vec![moved(out)]);
    for i in 1..=8 {
        let f = i as f32 / 8.0;
        let spot = Pos2::new(to.x, from.y + (to.y - from.y) * f);
        pass(&ctx, &mut app, 2.08 + 0.02 * f as f64, vec![moved(spot)]);
        drag_report(&ctx, &app, "form-open travel", Some(spot));
    }

    app.cancel_edit();
    pass(&ctx, &mut app, 2.34, vec![release(to)]);
    drag_report(&ctx, &app, "release+cancel", Some(to));
    assert_eq!(app.edit_idx, None, "the form is gone");

    // Two more passes for the release to be seen (see [`drag`]).
    pass(&ctx, &mut app, 2.36, vec![]);
    pass(&ctx, &mut app, 2.38, vec![]);

    // Third place, in front of the row the pointer rests on. The measure had to be taken in a layout whose first row was a form and whose rows
    // stood two hundred points lower than they do now, and it still names the row the hand is on: that is the whole content of this test.
    assert_eq!(
        labels(&app),
        ["Eins", "Zwei", "Drei", "Sechs", "Vier", "Fünf"],
        "{}",
        trace(&app)
    );
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
