//! The whole interface: top bar, target list, the four views, event log, status bar.

use crate::app::{self, App, LogKind, TargetKind, View};
use crate::history;
use crate::i18n::{self, Lang};
use crate::store;
use crate::theme::{self, Mode};
use egui::{
    pos2, Color32, CursorIcon, FontId, Id, Label, Pos2, Rect, RichText, ScrollArea, Stroke, Ui,
    Vec2,
};
use std::path::PathBuf;

/// Draws one frame of the whole window.
pub fn draw(ui: &mut egui::Ui, app: &mut App) {
    top(ui, app);
    bottom(ui, app);
    left(ui, app);
    center(ui, app);
}

/// Text for a selectable control (the four view buttons, the category chips, "outages only", the
/// colour scheme). `.weak()` would be the natural way to make these look like quiet controls, and it
/// used to be written on every one of them — but egui resolves `weak` *before* painting: the galley
/// already carries the faded colour, and the widget's own text colour (black / white, see
/// [`crate::theme::apply`]) can only fill in where no colour was chosen. So a weak selectable label
/// stayed grey next to a button whose label is properly black or white — which is exactly what this
/// app must not do. Hence: plain text here, and the selection background plus the frame say what is
/// active instead of a faded colour.
///
/// The shortcut is appended with two spaces so it reads as a separate hint; pass an empty string for
/// controls that have no key.
fn selectable_text(label: impl Into<String>, shortcut: &str) -> RichText {
    let text = if shortcut.is_empty() {
        label.into()
    } else {
        format!("{}  {}", label.into(), shortcut)
    };
    RichText::new(text)
}

/// The two new-target fields in the top bar have to be found by id, because "Enter adds a target"
/// should only fire while one of them has the focus. `TextEdit` derives its id from an explicit
/// salt for exactly that reason — without it the id would follow the widget's position and change
/// whenever the bar rewraps.
///
/// The salt is hashed with the panel's own `Ui` id, which is what `TextEdit` does internally
/// (`ui.make_persistent_id(salt)`). So the id has to be built inside the panel, where that `Ui`
/// is known, and handed out through [`App::new_target_ids`] for the focus check afterwards.
const NEW_HOST_ID_SALT: &str = "new_host_field";
const NEW_LABEL_ID_SALT: &str = "new_label_field";

/// The two rows of controls above the panels. They are deliberately kept apart: the first row is
/// "what the app does" (title, new target, start/stop), the second is "what is on screen" (view,
/// filter). Cramped together they wrapped into each other at the minimum window width.
pub fn top(ui: &mut egui::Ui, app: &mut App) {
    let lang = app.lang();
    let m = app.theme_mode;
    // Set when something worth persisting changed; the state is written once at the end of the
    // panel so a click on a category chip cannot be lost until the next autosave.
    let mut save_now = false;
    egui::Panel::top("title_bar")
        .frame(
            egui::Frame::new()
                .fill(ui.style().visuals.window_fill)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ui, |ui| {
            // Row 1: title and horizon on the left, start/stop on the far right. Nothing here
            // takes input, so the row stays one clean line at any window width.
            ui.horizontal(|ui| {
                ui.heading(RichText::new("System-Ping").color(m.accent()).strong());
                ui.label(RichText::new(lang.subtitle()).weak());
                ui.separator();
                // The chart window's preset picker (up to 24 h). It sits with the title because it
                // says how much of the picture is shown, not what is being measured.
                let combo = egui::ComboBox::from_id_salt("horizon_top")
                    .selected_text(horizon_label(lang, app.options.horizon_secs))
                    .width(72.0)
                    .show_ui(ui, |ui| {
                        for c in app::HORIZON_CHOICES {
                            ui.selectable_value(
                                &mut app.options.horizon_secs,
                                c.secs,
                                horizon_choice_label(lang, c),
                            );
                        }
                    });
                let _ = combo.response.on_hover_text(lang.tip_horizon());
                if ui
                    .add(egui::Button::selectable(
                        app.follow_live,
                        if app.follow_live {
                            "● Live"
                        } else {
                            "○ Live"
                        },
                    ))
                    .on_hover_text(lang.tip_follow_live())
                    .clicked()
                {
                    app.follow_live = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The path to the state folder belongs where the file is created: someone who
                    // wants to know what is stored has just been looking at the title.
                    if ui
                        .small_button(lang.data_folder_btn())
                        .on_hover_text(lang.tip_data_folder())
                        .clicked()
                    {
                        app.open_state_folder();
                    }
                    // The autostart button is only visible while nothing is being measured, and only
                    // where the OS has a mechanism we can actually write. `autostart_state()` is the
                    // registry's answer (looked up at startup, then kept current); the raw counter
                    // alone would read "off" until the user presses this.
                    if !app.running
                        && crate::platform::autostart::is_supported()
                        && ui
                            .button(if app.autostart_state() == Some(true) {
                                lang.autostart_on()
                            } else {
                                lang.autostart_off()
                            })
                            .on_hover_text(lang.tip_autostart_top())
                            .clicked()
                    {
                        let on = app.autostart_state() != Some(true);
                        app.set_autostart(on);
                    }
                    if ui
                        .button(if app.running {
                            lang.stop_all()
                        } else {
                            lang.start_all()
                        })
                        .clicked()
                    {
                        if app.running {
                            app.stop();
                        } else {
                            app.start();
                        }
                    }
                });
            });

            // Row 2: everything that takes input — the new-target fields, the interval, the
            // filter and the category switches. Kept apart from row 1 so the two never wrap into
            // each other at the minimum window width, and wrapped within itself: its items are more
            // than a narrow window can hold on one line, and a row that does not wrap simply paints
            // the last of them outside the window.
            ui.horizontal_wrapped(|ui| {
                let mut add = false;
                let host_buf = app.new_host.clone();
                let label_buf = app.new_label.clone();
                ui.add_sized(
                    [190.0, 22.0],
                    egui::TextEdit::singleline(&mut app.new_host)
                        .id_salt(NEW_HOST_ID_SALT)
                        .hint_text(lang.hint_new_host()),
                );
                ui.add_sized(
                    [130.0, 22.0],
                    egui::TextEdit::singleline(&mut app.new_label)
                        .id_salt(NEW_LABEL_ID_SALT)
                        .hint_text(lang.hint_new_label()),
                );
                // Remembered for the Enter check at the end of the panel: these are the ids the
                // two fields were given, which is what `Memory::focused` will report.
                app.new_target_ids = [
                    ui.make_persistent_id(NEW_HOST_ID_SALT),
                    ui.make_persistent_id(NEW_LABEL_ID_SALT),
                ];
                // Enter adds the target only while one of the two fields has the focus; otherwise a
                // key press meant for the chart or a combo box would silently create a row. The
                // press itself is detected after the panel (see below) because `Ui::input` does not
                // see keys that a text field already claimed.
                if ui.button(lang.add_target_short()).clicked() {
                    add = true;
                }
                if add {
                    // Adding a target would shift the index the inline editor points at, so any
                    // open editor is committed first.
                    app.close_edit();
                    app.new_host = host_buf;
                    app.new_label = label_buf;
                    app.add_target();
                    app.new_host.clear();
                    app.new_label.clear();
                }

                ui.separator();
                // Only the interval lives here. The chart window has its preset picker in the row
                // above, and a second control for the same setting — worse still one that stopped
                // at 15 min while the picker offers 24 h — was more confusing than useful.
                //
                // A stepped combo box rather than a slider: the useful distances are six rungs
                // (0,25 … 10 s), and a slider makes 0,7 s look as deliberate as 1 s while it only
                // changes how often `ping.exe` starts. The value is read out next to it so the
                // number stays legible without opening the list.
                let before = app.options.interval_secs;
                egui::ComboBox::from_id_salt("interval_top")
                    .selected_text(app::interval_label(lang, before))
                    .width(72.0)
                    .show_ui(ui, |ui| {
                        for c in app::INTERVAL_CHOICES {
                            ui.selectable_value(
                                &mut app.options.interval_secs,
                                *c,
                                app::interval_label(lang, *c),
                            );
                        }
                    })
                    .response
                    .on_hover_text(app::interval_hint(lang, before));
                if (app.options.interval_secs - before).abs() > f32::EPSILON {
                    save_now = true;
                }
                ui.separator();
                // Views first: they decide what the rest of the window shows. The digit behind a name
                // is not decoration but its shortcut (`View::shortcut`) — and the view now names it
                // itself, so button, keyboard and settings do not maintain three different lists.
                for view in [View::Chart, View::Events, View::Stats, View::Settings] {
                    let sel = app.view == view;
                    if ui
                        .selectable_label(sel, selectable_text(lang.view(view), view.shortcut()))
                        .on_hover_text(lang.view_hint(view))
                        .clicked()
                    {
                        app.view = view;
                    }
                }
                ui.separator();
                ui.add_sized(
                    [160.0, 22.0],
                    egui::TextEdit::singleline(&mut app.filter).hint_text(lang.hint_filter()),
                );
                if !app.filter.is_empty() && ui.button("×").clicked() {
                    app.filter.clear();
                }
                // Categories double as a filter: a chip is on when its category is shown, and
                // clicking it switches that whole group in every view (chart, legend, lists). Only
                // categories that some target actually uses are offered — an empty switch is noise.
                let mut kinds_dirty = false;
                for &k in TargetKind::all().iter() {
                    if !app.targets.iter().any(|t| t.kind == k) {
                        continue;
                    }
                    let on = !app.options.kinds_off.contains(&app::kind_index(k));
                    // The word "off" on the chip: without it a switched-off toggle looks like a
                    // button that does not respond. This way the spot says what is going on.
                    let mut chip = ui.selectable_label(
                        on,
                        selectable_text(
                            if on {
                                lang.kind(k).to_string()
                            } else {
                                i18n::fill(
                                    lang.filter_off_fmt(),
                                    &[("k", lang.kind(k).to_string())],
                                )
                            },
                            "",
                        ),
                    );
                    if !on {
                        chip = chip.on_hover_text(i18n::fill(
                            lang.tip_kind_show_fmt(),
                            &[("k", lang.kind(k).to_string())],
                        ));
                    } else {
                        chip = chip.on_hover_text(i18n::fill(
                            lang.tip_kind_hide_fmt(),
                            &[("k", lang.kind(k).to_string())],
                        ));
                    }
                    if !on {
                        chip = chip.highlight();
                    }
                    if chip.clicked() {
                        app.toggle_kind(k);
                        kinds_dirty = true;
                    }
                }
                if !app.all_kinds_on()
                    && ui
                        .small_button(lang.all())
                        .on_hover_text(lang.tip_all_kinds())
                        .clicked()
                {
                    app.show_all_kinds();
                    kinds_dirty = true;
                }
                if kinds_dirty {
                    save_now = true;
                }
            });

            // The Enter-to-add shortcut has to be judged outside the row: `Ui::input` only sees
            // events that were routed into this panel, and a text field claims its keys for
            // itself. So the context is asked directly, and the focus check decides whether the
            // press belongs to one of the two new-target fields.
            let focused = ui.ctx().memory(|mem| {
                mem.focused()
                    .is_some_and(|id| app.new_target_ids.contains(&id))
            });
            let want_add = ui.input(|i| i.key_pressed(egui::Key::Enter))
                && focused
                && !app.new_host.is_empty();
            if want_add {
                // Adding a target would shift the index the inline editor points at, so any open
                // editor is committed first.
                app.close_edit();
                app.add_target();
                app.new_host.clear();
                app.new_label.clear();
            }
        });
    if save_now {
        app.save_now();
    }
    short_cuts(ui.ctx(), app);
}

/// The keyboard: view numbers, start/stop, pan, escape. Drawn after the top bar so it sees the same
/// frame's input as the panels that follow.
///
/// Two guards keep the keys from doing damage. A focused widget (a text field, a combo box) has to
/// keep its keystrokes — `1` in the address field must not jump to the chart. And every shortcut
/// demands unmodified keys: with Ctrl or Alt held, `1` is somebody else's command.
///
/// The window keys are read from the context rather than a panel's `Ui` because they belong to no
/// widget at all; that also means a key press is acted on once per frame, which is what holding a
/// key should *not* do (see the repeat guard).
fn short_cuts(ctx: &egui::Context, app: &mut App) {
    // Held down by a text field or an open combo box's search? Then the keys are typing.
    let busy = ctx.memory(|mem| mem.focused().is_some());
    if busy {
        return;
    }
    let horizon = app.options.horizon_secs as f64;
    let mut quit_edit = false;
    ctx.input(|i| {
        if !i.modifiers.is_none() {
            return;
        }
        for (key, view) in [
            (egui::Key::Num1, View::Chart),
            (egui::Key::Num2, View::Events),
            (egui::Key::Num3, View::Stats),
            (egui::Key::Num4, View::Settings),
        ] {
            if i.key_pressed(key) {
                app.view = view;
            }
        }
        if i.key_pressed(egui::Key::Space) {
            if app.running {
                app.stop();
            } else {
                app.start();
            }
        }
        // A quarter of the window per press — the same step as the two pan buttons, so the key and
        // the button mean the same distance.
        if i.key_pressed(egui::Key::ArrowLeft) {
            app.follow_live = false;
            app.pan_offset_secs += horizon * 0.25;
        }
        if i.key_pressed(egui::Key::ArrowRight) {
            app.pan_offset_secs = (app.pan_offset_secs - horizon * 0.25).max(0.0);
            if app.pan_offset_secs < 0.5 {
                app.follow_live = true;
            }
        }
        if i.key_pressed(egui::Key::Escape) {
            quit_edit = app.edit_idx.is_some();
        }
    });
    // Esc first cancels an open editor; only without one does it mean "back to the live edge".
    if quit_edit {
        app.cancel_edit();
    } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.follow_live = true;
        app.pan_offset_secs = 0.0;
    }
}

/// The left panel: heading, count, and one row per target.
pub fn left(ui: &mut egui::Ui, app: &mut App) {
    egui::Panel::left("targets")
        .exact_size(250.0)
        .frame(
            egui::Frame::new()
                .fill(ui.style().visuals.panel_fill)
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ui, |ui| {
            left_body(ui, app);
            // A colour chosen in the editor is a per-target setting, so it is not covered by the
            // autosave that watches `AppOptions`. It still has to survive a restart.
            if std::mem::take(&mut app.color_dirty) {
                app.save_now();
            }
        });
}

/// Whether a press ended on one of the rows in this very pass, and which entry it was.
///
/// This is egui's own verdict on the question the whole gesture turns on: did the hand stay put, or did it
/// travel? `InteractionSnapshot::clicked` names the widget a finished press belongs to, and egui only fills
/// it in when the pointer stayed inside its slop (`InputOptions::max_click_dist`, six points by default) —
/// so a pull of more than six points answers `None` here, without anyone having to measure the travel. That
/// is why this reads an answer rather than computing one: an earlier version asked where the pointer was
/// when it let go and tried to tell click from pull by distance, which cannot work — both end with the
/// button released — and what it produced was the editor opening on whatever row the hand happened to be
/// over at the end of a drag.
///
/// It answers for one pass only, the pass whose events hold the release. The press before that has no answer
/// yet, and after it egui has already forgotten the press; that is what [`App::drag_row`] is for.
pub fn clicked_target(ctx: &egui::Context) -> Option<usize> {
    // `interaction_snapshot` is read-only and describes this pass, computed in `begin_pass` from this pass's
    // events — so unlike a widget's own `clicked()`, which reaches us one pass late, it is already true while
    // the rows are being laid out. Anything that can be clicked in this window but is not a row (a button, a
    // slider) answers `None` through [`strip_target`].
    strip_target(ctx.interaction_snapshot(|i| i.clicked)?)
}

/// Whether egui has decided this pass that a held press on one of the rows is a pull, and which entry.
///
/// This is the other half of egui's verdict: `dragged_id` names the widget under a drag that has left the
/// slop, and it is set in the same pass the pointer leaves it — one pass *earlier* than the row's own
/// `drag_started()`, which is only handed out to a widget that already exists. So this is what lets the list
/// start following the hand without a frame of lag, and what knows about the release pass itself (there egui
/// has let go again, so it answers `None` — see [`App::drag_row`]).
pub fn dragged_target(ctx: &egui::Context) -> Option<usize> {
    strip_target(ctx.dragged_id()?)
}

/// Whether the hand that grabbed one of the rows is still holding it, and which entry egui says it is pulling.
///
/// The pair belongs together because a live follow has to answer both questions with one breath, and egui's answers about a drag are true for
/// exactly one pass each. `ctx.dragged_id()` names the widget under a pull that has left the slop — but only until the pointer lets go: on the
/// release pass itself egui has already let go again and answers `None` there, which is why [`App::drag_row`] carries the entry across that one
/// pass. That is enough to know *what* is in flight, and on its own it is not enough to decide whether the list still follows the pointer: a
/// release pass also has no button held, so asking only "is something carried?" makes it look like an ordinary hover — and answering that as a
/// follow moves the entry one place past where the hand let go, undoing the drop that is measured from the very same point (see
/// [`App::drop_moved_target`]). So the button has to be part of the question, which makes "still held" mean "not this pass's release".
fn carried_by_drag(ctx: &egui::Context, app: &App) -> Option<usize> {
    let dragged = ctx.dragged_id().and_then(strip_target);
    // A press that has become a pull is what holds a row, and egui knows it from its own slop test (`is_decidedly_dragging`) rather than from
    // anything here measuring travel. Asked on the same pass as `dragged_id()` because both come out of `begin_pass`, so they cannot disagree
    // about which pass this is — a widget's own `drag_started()` would arrive one pass later and answer for a row that may already have moved.
    let held = ctx.input(|i| i.pointer.is_decidedly_dragging());
    match (dragged, held) {
        // The normal case: egui says which row is pulled and the button says it still is.
        (Some(id), true) => Some(id),
        // The release pass, where egui has let go but our own drag has not — the entry is still in flight, only no longer followed. Named by
        // `App::drag_row`, which is the one record that survives egui forgetting the press (see there).
        (None, false) => app.drag_row,
        // Nothing of ours is being pulled: either no row answers and whatever holds the button is a slider or a splitter somewhere else, or a row
        // answers while the pointer is not on a pull at all — which egui only reports for one pass after a release that its slop test has not
        // caught up with, and by then `App::drag_row` has been cleared by that drop.
        _ => None,
    }
}

/// Where the pointer is while it holds a row: the place every part of the gesture measures against.
///
/// This is deliberately not `interact_pos()`. That one follows whichever widget egui thinks the pointer is *over*, and a live drag has been
/// moving rows under the hand the whole time — so on the pass whose events hold the release it can still be standing on the row that came up to
/// meet the entry, rather than where the hand actually is. Measured there, a drop asks for the place below the one the pointer rests on: every
/// live move of such a drag then names that same slot, and the pass after each move names the entry's own place again — so the last move undoes
/// the one before it and the release confirms where the wobble happened to stop rather than where the hand stopped. `latest_pos()` is the pointer
/// itself, with nothing in between, which is what a place on screen has to be read from.
fn drag_pos(ctx: &egui::Context) -> Option<Pos2> {
    ctx.input(|i| i.pointer.latest_pos())
        .or_else(|| ctx.pointer_interact_pos())
}

/// Whether this pass is the one whose events hold the pointer letting go of a row, or the first one after it.
///
/// Both have to be answered together because egui's release answers span exactly those two passes: `drag_stopped_id()` names the entry that let
/// go on the pass it saw the release event, and on every pass after that it keeps naming the same widget until another drag starts — while
/// [`App::drag_row`] is cleared by the drop on the first of them. So "the release has been handled" is what tells the two apart, and a pass that
/// is either of them must not move the list at all: the follow would shuffle the entries after the drop was measured from them.
fn release_is_at_hand(app: &App, ctx: &egui::Context) -> bool {
    app.drag_row.is_none()
        && ctx
            .drag_stopped_id()
            .is_some_and(|id| strip_target(id).is_some())
}

/// What egui currently says about a press on one of the rows, as one line for a terminal.
///
/// This is [`clicked_target`], [`dragged_target`] and the release question printed side by side, which is how a drag that
/// misbehaves tells you *which* of the three it is failing at: a pull whose `hit` never becomes `drag` never left egui's slop,
/// one whose `stop` never appears was never let go of by egui, and one that lands in the wrong place has all three and a layout
/// problem instead. Written as ids rather than labels because the watcher has no access to the list here, and `strip_target` is
/// what turns an id back into an entry.
pub fn gesture_report(ctx: &egui::Context) -> String {
    let said = |id: Option<Id>| match id.and_then(strip_target) {
        Some(target) => format!("#{target}"),
        None => "-".to_string(),
    };
    let click = ctx.interaction_snapshot(|i| i.clicked);
    let press = ctx.viewport(|v| v.hits.click.map(|w| w.id));
    format!(
        "hit={} drag={} start={} stop={} click={}",
        said(press),
        said(ctx.dragged_id()),
        said(ctx.drag_started_id()),
        said(ctx.drag_stopped_id()),
        said(click),
    )
}

/// Height of a row in the target list, in ui points: two lines of text plus the frame's padding.
/// The click area of a row is this tall, so it always covers exactly what the row shows — and an
/// open editor keeps the same strip for its neighbour clicks (see `App::row_heights`).
const ROW_H: f32 = 39.0;

/// The empty band between two rows, which belongs to neither of them. A drop is aimed at a row, so
/// only the row's own height may be claimed as its click area — the gap would otherwise answer for
/// the row above it and make the boundary where an entry lands sit too low. It is exactly the frame's
/// bottom margin plus the list's item spacing, which is what makes `ROW_H - GAP_Y` the strip height.
const GAP_Y: f32 = 9.0;

/// Width of the grip drawn at the left edge of a hovered row — the only hint that a row can be
/// pulled to another place. Narrow enough that the row's own text keeps its room.
const GRIP_W: f32 = 10.0;

/// The id a row's click strip is registered under, for a given target.
///
/// The gesture needs it in both of its halves. A press has to be recognised as a drag later on even if
/// the entry it grabbed has already been shuffled through the list, and egui can only be asked about a
/// widget by id — an index would name whatever row happens to sit there now, while an id follows the
/// entry.
pub fn strip_id(target_id: usize) -> Id {
    Id::unique(("system_ping", "target_row_strip", target_id))
}

/// Where the pointer let go of a row. `None` on every pass that held no release, which is most of them.
///
/// A drop has to be measured in the pass that reads the release event, and only egui's own drag bookkeeping knows about that
/// event at that moment. A row cannot: `interaction::interact` works the events out in `begin_pass`, before a single widget of
/// the pass exists, so the answer reaches a row as `drag_stopped()` one pass late — by which time `interact_pos` has moved on to
/// wherever the pointer drifted next, and `latest_pos` is no longer a place the hand held. So the question is put here, where
/// egui is asked while its answer is still true.
struct DragRelease {
    /// Which entry was in flight when the pointer let go, or `None` if what egui released was not one of the rows —
    /// a slider or a button somewhere else. The entry is named by id rather than by row: a live drag has been reordering
    /// the list all along, so only an id still means the same thing.
    target: Option<usize>,
    pos: Option<Pos2>,
}

/// Asks egui whether a pointer came to rest this pass, and if so where it did. See [`DragRelease`] for why the question has to
/// be put at the top of a pass rather than where the answer is used.
fn drag_release(ctx: &egui::Context) -> DragRelease {
    // The entry that let go comes from egui's own note about it, and what matters here is that this is *not* the question
    // [`clicked_target`] asks. `drag_stopped_id` names a widget egui had counted as dragged and now released, which is exactly
    // the gesture we want to land — whereas a press that never left the slop answers there as "a click", and egui hands that one
    // to the row's own `clicked()`, where it opens the editor instead. The two answers are mutually exclusive by construction:
    // `interaction::interact` only reports a click when the pointer did *not* travel, so a finished pull can never be read as a
    // click and vice versa. That is what lets one press have exactly one outcome without anyone comparing positions.
    DragRelease {
        target: ctx.drag_stopped_id().and_then(strip_target),
        // The same point the live follow measures by (see [`drag_pos`]) — one press cannot be answered against two different places, and the
        // release is the one answer that has to agree with every move it just made.
        pos: drag_pos(ctx),
    }
}

/// The target a row's click strip stands for, or `None` if this widget is not one of the rows.
///
/// `Id` keeps nothing but the hash of what it was made from, so [`strip_id`] cannot be undone and our own
/// widgets have to be recognised by recomputing their ids. A target id is a small number — handed out from 0
/// upward, one above the highest in use, and never renumbered — so the range that could match is known ahead of
/// time and no bookkeeping of our own is needed.
///
/// This runs for every answer egui gives about a press, which is once per pass at worst, and it stops at the first hit — so the
/// cost is bounded by how many targets there are, not by anything the user can do twice in quick succession. A test that drives
/// the rows from outside the crate cannot recompute an id any other way, which is why this is public; nothing outside a row's own
/// gesture should reach for it.
pub fn strip_target(id: Id) -> Option<usize> {
    // Rows are only ever added between passes, never in the middle of one, and no user keeps thousands of
    // targets — so this bound is generous rather than tight, and a miss would cost at most one gesture.
    const ID_GUESS_LIMIT: usize = 4096;
    (0..ID_GUESS_LIMIT).find(|target| strip_id(*target) == id)
}

/// The rows as the panel laid them out on its last completed pass, each with the entry it showed.
///
/// This is the layout the pointer has been travelling over, and the one the live half of a drag measures against: a pass publishes
/// its rects only after its live move (see `move_now` in [`left_body`]), so between two passes there is exactly one answer to
/// "where does row 3 stand" — which is what lets a slot counted here be handed straight to [`App::move_target`]. A row the panel
/// has never drawn at all — an entry added between passes, or one past the panel's bottom edge — keeps its `None` in here rather
/// than being dropped: every consumer counts rows by position, and a hole that quietly vanished would shift every place below it
/// up by one. So a point that belongs to no row has no answer and means "stay", which is sound for both halves: a live drag may
/// simply stop following the pointer for one pass, and a place nobody ever drew was never aimed at. A row whose entry is being
/// edited is *not* one of those holes — its header strip is recorded, and that is what makes a list with a form in it measurable
/// at all (see the loop in [`left_body`]).
fn rows_prev_pass(app: &App) -> Vec<Option<Rect>> {
    app.row_rects().to_vec()
}

/// A whole layout of the target list: which entry stood in which row, and where that row's click strip was drawn.
///
/// This is what a pass publishes, and the reason it has to be kept as one piece rather than as two lists is the question a drop asks: "which entry
/// did the pointer come to rest on". That is only answerable in a layout — the numbering of rows and the numbering of the list agree or disagree
/// depending on where the holes are, and reading an id from one list and a rect from another is how a drop came to name a row two places away from
/// the pointer. See [`App::drag_layouts`], which keeps the last two of these for exactly that reason.
#[derive(Clone)]
pub struct RowLayout {
    /// One entry per drawn row, in the order the panel drew them, with a `None` for a row it never drew at all.
    pub rects: Vec<Option<Rect>>,
    /// Which entry each of those rows showed, same length and order as [`Self::rects`].
    pub ids: Vec<usize>,
}

/// The layouts this pass can measure a pointer against, oldest first: the one before last, then the one being drawn now.
///
/// A live follow always aims at what the hand can see, which is the layout of the pass before this one — that is [`rows_prev_pass`] and what
/// [`live_slot`] measures by. A finished press is different: it began somewhere in a layout that may have been replaced twice over since, so
/// answering it against only the newest one reads the pointer off rows it never travelled over. Both are needed at once, which is why this hands
/// out a pair rather than picking one.
fn layouts(app: &App) -> Vec<RowLayout> {
    let mut out: Vec<RowLayout> = app
        .drag_layouts
        .iter()
        .map(|l| RowLayout {
            rects: l.rects.clone(),
            ids: l.ids.clone(),
        })
        .collect();
    out.push(RowLayout {
        rects: rows_prev_pass(app),
        ids: app.row_ids().to_vec(),
    });
    out
}

/// The same question asked of one layout, with the entry its rows show rather than only a place among them: `row_at` answers per slot, and whether
/// that place is already where an entry stands is a question about *which* row it was. Looking the id up in this layout's own list is what keeps the
/// two halves of a drag from answering it differently (see [`RowLayout`]).
fn rest_on_row(layout: &RowLayout, pos: Pos2) -> Option<(usize, usize, Rect, Option<Rect>)> {
    let (slot, rect, gap) = row_at(&layout.rects, pos)?;
    let id = *layout.ids.get(slot)?;
    Some((slot, id, rect, gap))
}

/// The row the pointer came to rest on, told as a place in the list *of the layout it was measured in*, together with where the entry being carried
/// stands in that very layout. That pairing is the whole answer a drop needs, and it cannot be taken from two layouts: a slot counted among one
/// generation's rows and an entry's place counted among another's are numbers that do not speak about the same list — which is what let a drop name
/// a row the pointer never passed.
///
/// Measured across every layout the gesture could have been aiming at, newest first. Each layout answers with the row under the point and how far down
/// it is — `row_at` already refuses a point that belongs to no row, so a layout that has moved out from under the pointer simply says nothing. The
/// newest layout wins: while it can still answer, the rows there are the ones the hand has been looking at, and an older one would drag the answer
/// back to a place the user stopped seeing passes ago. Only when none of them answers is the point outside the list, which both halves read as "stay".
///
/// Asking the older layouts at all is what keeps a drop honest across a pass that redraws the whole list taller or shorter — closing an editor being
/// the case that broke it first: the form's rows move back up into their places on the very pass that releases the drag, so the pointer rests in empty
/// space as far as *that* layout is concerned while the layout it travelled over still knows exactly which row it meant.
fn rest_in(layouts: &[RowLayout], carried: usize, pos: Pos2) -> Option<(usize, usize)> {
    layouts.iter().rev().find_map(|l| {
        let (slot, _, rect, gap) = rest_on_row(l, pos)?;
        let from = l
            .ids
            .iter()
            .position(|drawn| *drawn == carried)
            // A layout that never drew the carried entry's row cannot say where it stood, and a slot counted in such a list would be a place in a
            // layout the entry is not part of.
            .filter(|row| l.rects.get(*row).is_some_and(|rect| rect.is_some()))?;
        Some((drop_index_for_row(slot, pos, rect, gap), from))
    })
}

/// How far the target list is scrolled right now, read back from where its rows were last drawn.
///
/// A scroll area does not keep its offset to itself: it hands back what it remembered only while the pointer hovers it and
/// otherwise starts at the top of the content — which a drag finds out the hard way, because a press on a row does not hover the
/// area any more once egui counts it as dragging a widget. The list would then jump up for exactly that pass: the pointer is
/// still where it was and therefore over a different row than before, so the slot asked for is the one below the entry's own,
/// which means "stay", and nothing moves for the rest of the gesture either because the entry never left its place. Naming the
/// offset out loud at the call site is only half the fix; the other half is knowing what to ask for, and the rows know. A recorded
/// strip was measured in the panel's own coordinates *after* the area had scrolled, so its distance from the top of the list —
/// the first row's `min.y` — has the offset folded into it. Undoing that with the `min.y` the panel offers *now* gives the number
/// to ask for, taken from the layout the pointer is actually aiming at rather than a remembered one.
fn list_scroll_offset(app: &App, ui: &Ui) -> Option<f32> {
    let first = app.target_row_rects().into_iter().next()?;
    Some(ui.cursor().min.y - first.min.y)
}

/// Which row of the current layout shows the entry with this id, or `None` when no row does — which can only mean the panel drew
/// none for it, so nothing was ever aimed at it. Counted in `rows`, and therefore in the same numbering as [`row_at`] answers with.
fn row_of(rows: &[Option<Rect>], app: &App, id: usize) -> Option<usize> {
    app.row_ids()
        .iter()
        .position(|drawn| *drawn == id)
        .filter(|row| rows.get(*row).is_some_and(|rect| rect.is_some()))
}

/// The band a point is hit against for a row: the row's own click area plus whatever it takes for the rows of a
/// list to meet without a seam.
///
/// A point that belongs to no widget at all is what egui resolves by picking the *nearest* widget, and a drop
/// position should not be built on that — so the app decides for itself which row an in-between point counts
/// as, by growing each row's band until they touch.
///
/// The gap goes entirely to the row above rather than being split evenly. A point between two rows has to mean
/// "between them", and only a slot counted from the upper row can say that: `App::drop_index_for` answers
/// "after this one" for every point below a row's centre, so handing the lower half of the gap to the row below
/// would answer "before that one" for a point still above the upper row's own bottom edge — and for an entry
/// being carried one row down, which is where a live drag always puts it, those two answers are the same place.
/// The disagreement then shows up as a wobble: the pass says move, the next says stay, the pass after that says
/// move again. With the gap above, every point between two rows means one slot and nothing oscillates.
///
/// Only the last row's band stops short of that. There is no row below it for a point in its lower gap to be confused
/// with, so growing that band would claim empty space under the list as a place to drop into — which reads as "move to
/// the very end" and quietly reorders a drag that was only meant to be a click. A release there therefore belongs to no
/// row at all, and means "stay", like any other point outside the list.
fn band(rect: Rect, last: bool) -> Rect {
    if last {
        rect
    } else {
        Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y + GAP_Y))
    }
}

/// The gap between where a row's click area ends and the next one begins, or `None` for the last row — which has no neighbour to
/// be separated from. See [`band`]: that gap is handed entirely to the row above, so it is the row above that owns it, and both
/// halves of a drop decision have to own it the same way or they disagree about which slot a point in it names.
fn below_gap(rect: Rect, last: bool) -> Option<Rect> {
    (!last).then(|| {
        Rect::from_min_max(
            pos2(rect.min.x, rect.max.y),
            pos2(rect.max.x, rect.max.y + GAP_Y),
        )
    })
}

/// Whether a point belongs to the strip itself or to the gap beneath it — and therefore which of two different rules decides the
/// slot. Inside the strip the row's own middle is what matters; inside the gap there is no "this row" question to answer at all,
/// only "after the upper one", so a point in a gap always names the place below its row whatever else has happened to the layout.
fn in_gap(gap: Option<Rect>, pos: Pos2) -> bool {
    gap.is_some_and(|gap| gap.contains(pos))
}

/// The place a row asks to be dropped at when the pointer rests on it: above its middle "in front of me", below it "after me".
///
/// `strip` is the row's own click area and `gap` the empty line beneath it, which [`band`] hands to this row rather than splitting
/// between neighbours. The two are decided differently, and that split is what makes a live drag stop wobbling. Inside the strip the
/// row's middle is the answer — but a band reaches past the strip so that bands meet without a seam, which puts the point where two
/// of them touch exactly on the middle of the *strip between* them. Decided by that middle, the pass that had just moved an entry up
/// would ask for the place below it again, because the pointer never left the band: the entry lands on a slot that means "stay" and
/// the gesture dies there, one place short of where the hand stopped. So a point in the gap always names the place below its row —
/// there is no "this row or that one" question to answer in an empty line, only "after the upper one", which is what the gap is for.
fn drop_index_for_row(target_row: usize, pos: Pos2, strip: Rect, gap: Option<Rect>) -> usize {
    if in_gap(gap, pos) || pos.y >= strip.center().y {
        target_row + 1
    } else {
        target_row
    }
}

/// The row a point belongs to in the layout this pass is measuring against: which place in the list it was and where that row's
/// click strip stood. `None` when the point belongs to no row at all, which is what makes "let go outside the list" mean "stay".
///
/// A row whose strip was never drawn is skipped here — there is no strip to sit on — but it keeps its slot in the numbering, which is
/// what the first element of the answer is counted in. That is not a detail: a layout is published with one slot per entry and each
/// rect filed under the entry it belongs to (see the end of [`left_body`]), so a row's position in `rows` *is* its entry's position in
/// [`App::targets`], holes and all (see [`rows_prev_pass`]). An earlier version translated the number by adding one per skipped hole,
/// on the theory that a row whose strip was not drawn shifts the ones under it up. It does not, and that adding is what put a drop one
/// place past the pointer whenever an editor was open above the row the pointer rested on.
///
/// The band below each row counts as its lower half: the empty line between two rows belongs to "after the upper one", and
/// without it such a point would answer with whichever row it happened to be nearest to. See [`band`] for why the last row gets
/// no band and what that means for a point under the list.
///
/// The place is counted in the order of [`App::targets`], so an answer here can be handed straight to [`App::move_target`] without
/// saying anything about the layout it came from. That is the whole point of translating here rather than at the callers — both
/// halves of the gesture ask this one function, so they cannot disagree about what a row number means while a form or a
/// scrolled-away row sits somewhere in the list.
fn row_at(rows: &[Option<Rect>], pos: Pos2) -> Option<(usize, Rect, Option<Rect>)> {
    let last = rows.iter().rposition(|rect| rect.is_some())?;
    rows.iter().enumerate().find_map(|(row, rect)| {
        let rect = *rect.as_ref()?;
        let gap = below_gap(rect, row == last);
        band(rect, row == last)
            .contains(pos)
            .then_some((row, rect, gap))
    })
}

/// Whether dragging an entry from `from` to `to` would actually change anything. A drop on the row it
/// sits on, or just below it, means "stay" — and `move_target` says so in its own words, which a drag
/// that went nowhere should not do.
fn moves(from: usize, to: usize) -> bool {
    to != from && to != from + 1
}

/// The slot a held drag asks for, or `None` when it asks for no move.
///
/// Counted from where the entry stands *today*, which is what keeps a live follow from chasing itself: an entry answered at its
/// own current row asks "stay" the moment the pointer rests on it, so nothing wobbles between two passes. Measuring against a
/// layout from further back would look tempting — it never changes — but it has been off the screen for every pass since, so a
/// hand that moves faster than the list follows would be answered somewhere it stopped looking at long ago. The live follow only
/// has to keep up with what the user can see, which is why this takes the newest layout alone rather than the pair a release gets.
fn live_slot(app: &App, rows: &[Option<Rect>], carried: usize, pos: Pos2) -> Option<usize> {
    let from = row_of(rows, app, carried)?;
    let over = row_at(rows, pos)?;
    slot_at(drop_index_for_row(over.0, pos, over.1, over.2), from)
}

/// The slot a release puts the carried entry in, or `None` when it asks for no move — which is what makes "let go outside the
/// list" mean "stay".
///
/// Both numbers come from one and the same layout, found by [`rest_in`] as the newest one that still has a row under the pointer. That pairing is
/// the fix for a drop that lands somewhere the pointer never went: a finished press began in an older layout than the pass releasing it draws, and
/// if the list changed height in between — an editor closing and pulling every row under it back up being the case that broke first — the newest
/// layout has no strip where the hand rests. Asked there, the point belongs to no row and the answer is "stay", so a drag that visibly travelled the
/// whole list leaves the entry at the bottom. Asked in the layout the pointer really crossed, both its own place and the row it came to rest on are
/// numbers about the same list, which is all [`slot_at`] needs. Where the layouts agree, which is every ordinary drop, this is exactly the old
/// answer.
fn release_slot(layouts: &[RowLayout], release: &DragRelease) -> Option<usize> {
    let pos = release.pos?;
    let carried = release.target?;
    // The newest layout first, and only for the entry's own place: a live follow has been moving the list under the hand, so that is where it
    // stands now. `rest_in` then searches backwards for a layout that can name both rows of the question, and while the newest one answers — which
    // it does unless something changed height mid-gesture — its answer is used as is.
    let over = match rest_in(layouts, carried, pos) {
        Some(over) => over,
        None => {
            // Below the last row there is nothing to aim at, and a point in that empty space must not be read as the nearest row's
            // lower half — see [`band`], which refuses to grow the last row for the same reason. But "below the list" *is* a place a
            // hand means: it is the end of the list, and naming it directly is what keeps a drop past the last row from landing one
            // row short of it. No row can have been added or removed while the pointer was over them, so the end of the list is still
            // exactly one place past the last of them. Asked per layout rather than once up front because an older layout can reach
            // that answer while the newest one has nothing there at all — which is the point of asking every layout, not just the last.
            // `rects` has one slot per entry (see [`rows_prev_pass`]), so its length is the place past the last row of *that* list.
            let under = layouts.iter().rev().find(|l| {
                l.rects
                    .iter()
                    .flatten()
                    .last()
                    .is_some_and(|last| pos.y > last.max.y)
            })?;
            (
                under.rects.len(),
                under.ids.iter().position(|drawn| *drawn == carried)?,
            )
        }
    };
    slot_at(over.0, over.1)
}

/// Turns a place the pointer named into a slot for [`App::move_target`], or `None` when it has nothing to say.
///
/// Both numbers are counted in the same list — the one the move happens in — which is what makes this only about
/// "is that already its own place": `to` is the index measured off the rows the pointer was aimed at, with the carried
/// entry still standing in it, and `from` is where the entry stands in that same list. The move takes the entry out of
/// that list first and inserts it at the requested place, so a slot just past the entry's own row already *is* its
/// own place: pulling a row down by less than a whole row means "stay", and `move_target` would say so in its own
/// words, which a drag that went nowhere should not do. That is also why nothing has to be done about a press that
/// never became a drag: egui calls one of those a click, so the gesture is never even started and no row is asked.
fn slot_at(to: usize, from: usize) -> Option<usize> {
    if moves(from, to) {
        Some(to)
    } else {
        None
    }
}

/// The panel's contents on their own. Tests call this directly so they can place the panel
/// somewhere they control and click or drag a row with a synthetic pointer.
pub fn left_body(ui: &mut Ui, app: &mut App) {
    let lang = app.lang();
    // Which entry the current click landed on, decided once before the rows are built. egui answers this (see
    // [`clicked_target`]), so there is nothing to measure here — and in particular nothing that depends on where the
    // pointer happened to be, which is what made an earlier version of this unable to tell a pull from a click.
    let clicked_on = clicked_target(ui.ctx());
    // A click inside one of the editor's own controls (a text field, the combo box, a button) must
    // not be mistaken for "this row was clicked again", so that case is ignored here. The editor
    // only closes through its buttons, Enter/Esc, or a click on another row.
    let clicked_on = match clicked_on {
        Some(id) if app.edit_id() == Some(id) => None,
        other => other,
    };
    // Which entry is in flight, decided once for the whole pass. See the row loop below for why this
    // cannot be asked per row: the move at the end of this pass relocates the entry, and from that moment on
    // `app.drag_row` no longer matches the row that is being drawn — only its id still does.
    let carried_id = carried_by_drag(ui.ctx(), app);
    // Whether the pointer let go of a row this pass or the one before it (see [`release_is_at_hand`]). Read here, next to the carry, because both
    // halves of the gesture have to ask it at the same moment: a follow that fires on such a pass moves the list *after* the drop has been
    // measured from it, which is how a drag ended up one place past the hand.
    let release_at_hand = release_is_at_hand(app, ui.ctx());
    // Whether this pass is the one in which egui picked a row up, and which one. Read here rather than per row: like every
    // other answer in this block it comes from `begin_pass`, so it is true for the whole pass including the rows that have not
    // been laid out yet, and a row that has already moved below us in this same loop cannot change it.
    let grab_now = ui
        .ctx()
        .drag_started_id()
        .and_then(strip_target)
        .filter(|id| app.drag_row != Some(*id));
    // Before a single row of this pass is laid out: see [`drag_release`].
    let release = drag_release(ui.ctx());
    // The layouts a pointer can be measured against, taken here at the very start of the pass, before the live move below relocates any row.
    // The live follow reads these, so it can only ever aim at a layout the hand has actually seen; the release waits for the end of the pass
    // and asks the layouts as published there, which is what a drop on a closing editor needs (see below).
    let prior = layouts(app);
    // The slot a held drag asks for, computed here — before this pass lays out any row. Once the move has happened at the end
    // of the loop, this pass's own rows describe places the pointer never saw, so waiting for the loop to ask is too late.
    let drop_now = match carried_id {
        // Nothing of ours is in flight: nothing follows the pointer.
        None => None,
        // A release is what ends a drag and decides where the entry finally lands, so on the pass that holds one — and on the one right
        // after it, where egui still names the same widget as "just released" (see [`release_is_at_hand`]) — the live follow keeps its hands
        // off: the two halves must never both move the list for one gesture. This is not the same question as "is something carried", and
        // conflating them was what made a drop land a row short of the hand: with the button up, the carry survives only in `App::drag_row`,
        // so a follow that asks nothing else moves the entry to the slot below the one the release is about to name — and because every pass
        // of such a drag does that, each move is undone by the next and the list ends wherever the wobble stopped rather than where the hand
        // let go.
        Some(_) if release_at_hand => None,
        // The pass that only just picked the row up: egui has moved the pointer past the slop, but the hand has not travelled
        // since then, so there is no new place to follow yet. Waiting one pass costs nothing and keeps the entry from being
        // asked to move twice for a single step — once because it was picked up, once because the pointer moved.
        Some(_) if grab_now.is_some() => None,
        // Where the pointer is with the button still held: the entry follows it while it travels. Asked across both layouts rather than only the
        // newest one, for exactly the reason a release is (see [`release_slot`]): the hand has been travelling over what it could see, and when the
        // list changes height — an editor closing under a drag that cannot move while a form is open — the newest layout stops having any row where
        // the pointer rests. Answered "stay" there, the entry never leaves its place for the whole rest of the gesture, and what a drop then reports
        // is not where the hand stopped but wherever the last shuffle happened to leave it. Newest first keeps the ordinary case exactly as it was:
        // while the current rows can answer, they are what the user is looking at and they decide.
        Some(id) => drag_pos(ui.ctx()).and_then(|pos| {
            prior
                .iter()
                .rev()
                .find_map(|l| live_slot(app, &l.rects, id, pos))
        }),
    };
    // One press, one outcome. egui has already decided which of the two this was — a press that travelled answers as a drag and
    // never as a click, one that stayed answers as a click and never as a drag (see [`clicked_target`] and [`dragged_target`]) —
    // but the two answers arrive through different doors: the click from egui's verdict on this pass, the carry from egui's
    // record plus `App::drag_row`, which lags a pass behind. So there is a window in which both are true, and an editor opening
    // in it would also refuse the move, because `App::move_target` waits for a form to be closed. One of them has to lose, and
    // it is the click: the hand did travel, and that is what a pull is.
    let clicked_on = if carried_id.is_some() || grab_now.is_some() {
        None
    } else {
        clicked_on
    };
    let m = app.theme_mode;
    ui.horizontal(|ui| {
        ui.label(RichText::new(lang.targets()).strong());
        let vis = app.visible_targets().len();
        ui.label(RichText::new(format!("{}/{}", vis, app.targets.len())).weak());
    });
    ui.separator();

    // The release this pass has to act on, if any. It is decided after the row loop — see there — so it only needs to survive
    // as a local; nothing about a drop depends on what the rows of this pass happen to be.
    let release = release.target.map(|_| release);

    let now = app.engine.elapsed_secs();
    // Reused scratch buffer for the per-target statistics shown in the editor.
    let mut scratch = std::mem::take(&mut app.stats_scratch);

    // While a row is in flight the list is held at the scroll position its recorded rows were drawn at, and that is done by
    // naming the offset out loud rather than by switching scrolling off. Both halves of the gesture read `App::row_rects`,
    // which describes where the rows stood when they were drawn; anything that moves the viewport therefore has to move those
    // rects with it, and a scroll area does not: it scrolls its content while the recorded strips stay put, so the pointer
    // ends up aiming at a layout nobody is looking at. There is also a plainer failure waiting in egui — an area only restores
    // the offset it remembered while the pointer hovers it, so a pass that is not hovering starts at zero and the whole list
    // jumps to the top for that one frame. `scroll_offset` asks for the offset this call rather than whatever happens to be
    // remembered, which keeps the rows exactly where the pass that recorded them drew them. As soon as the hand opens, the
    // area is free again.
    let mut area = ScrollArea::vertical().auto_shrink([false, false]);
    if carried_id.is_some() {
        if let Some(offset) = list_scroll_offset(app, ui) {
            area = area.vertical_scroll_offset(offset);
        }
    }
    area.show(ui, |ui| {
        // Collect actions first to avoid borrow conflicts. All of them name the *entry* they act on, never a row:
        // this loop draws rows in the order `order` fixed for it, and a live move at its end can leave every index
        // in `app.targets` pointing somewhere else than the row that decided the action.
        let mut edit_action: Option<fn(&mut App)> = None;
        let mut begin_edit_idx: Option<usize> = None;
        // Set by the "entfernen" button inside an open editor: which target to delete once the
        // list is no longer borrowed.
        let mut remove_idx: Option<usize> = None;
        // A colour change has to be written back to disk right away, like every other setting.
        let mut color_changed = false;
        // Filled with every row's click area while the rows are built, then handed to `app.row_rects` below. A
        // test needs those rects to aim a synthetic click, and so does a drag that is still held — see the slot
        // computation above, which reads them one pass behind this loop.
        //
        // One entry per row of `order`, in that order, with a hole left as `None` for an entry this pass drew no strip
        // for at all — one pushed past the panel's bottom edge, or one added since the last pass (see `row_heights`, which
        // keeps the list from shifting under such a row). A row whose entry is being edited is *not* one of those: its
        // header strip is recorded like any other. The holes matter because every consumer numbers the rows by position:
        // `row_of` counts these positions to say which row a pointer is over, so it has to be counting the same rows —
        // including the ones that were not drawn.
        let mut row_rects = Vec::new();
        // The same strips again, but only their heights: a row the panel cannot show keeps the height it had, or the list
        // would shift under the neighbours around it (see `App::row_heights`).
        // Counted in the order of `order`, so index `i` is the height the row of `order[i]` had last pass — which is
        // what makes this the right answer for a row whose entry has since been carried elsewhere.
        let mut row_heights = Vec::new();
        // And which entry each of those rows showed, so the two can be read together by id later on (see `App::row_ids`).
        let mut row_ids = Vec::new();

        // The live move this pass asks for, held back until every row has been drawn. Applying it inside the loop
        // would reorder `app.targets` halfway through the pass: the rows after it would then be laid out in a list
        // that no longer matches `order`, so an entry would be drawn twice while another was skipped and fell out of
        // `row_rects` — which reads as "the pointer is on nothing" for the rest of the drag, and as a target that has
        // vanished. One move per pass, applied once at the end, keeps this pass' geometry whole: the rows it drew are
        // exactly the rows the next pass measures against.
        let move_now = match (grab_now, carried_id, drop_now) {
            // The pass that picked the row up is not also the pass that moves it (see `drop_now`).
            (Some(_), _, _) | (_, Some(_), None) => None,
            (_, Some(id), Some(to)) => Some((id, to)),
            (None, None, _) => None,
        };

        // The list this pass draws, fixed before the first row is built: the ids, in the order they are to be drawn.
        // Walking indices instead would let a live move below reassign the loop mid-flight — `app.targets` is
        // reordered while this runs, so an index would name a different entry than the pass had planned and a row
        // would be drawn twice while another was skipped entirely. The ids are what every decision in the loop is
        // taken against: which row carries the entry, which one is being edited, where the selection sits.
        let order: Vec<usize> = app.targets.iter().map(|t| t.id).collect();

        // Where the panel's own bottom edge is — the height past which a row cannot be seen at all. A scroll area lets
        // its content grow beyond what it shows, so without this a list longer than the panel would keep registering
        // strips in space nobody can look at, and both halves of a drag would then answer for rows that are not on
        // screen: the pointer is over row 2 while an invisible row 5 says "I am the last one" and the whole numbering
        // shifts. Only rows inside this line are recorded, so what the layout knows and what the user can see stay one
        // and the same thing.
        let visible_until = ui.available_rect_before_wrap().max.y;

        for (row, id) in order.iter().copied().enumerate() {
            // Everything the row and the editor need from this target is read out here, so the
            // rest of the body never has to borrow `app.targets` again. The editor below needs
            // `app` mutably (its fields are the form's state), which a live borrow would forbid.
            //
            // Everything that says "this row" is asked by id rather than by position, for the same reason `carried_id`
            // is read once outside the loop: a drop below moves the entry through the list while this loop runs, and
            // from that moment on every index in the list — including this row's own — describes a different order
            // than the one being drawn.
            let Some(idx) = app.targets.iter().position(|t| t.id == id) else {
                continue;
            };
            // Whether this row still fits where the panel can show it, decided before anything is read or drawn: a row
            // past that edge is not part of the layout at all (see `visible_until`), so there is no point measuring what
            // it would cost. The height is the one the row had last pass — an open editor's form is far taller than its
            // row, and the list must not shift under the neighbours around it (see `App::row_heights`).
            let row_h = app.row_heights.get(row).copied().unwrap_or(ROW_H - GAP_Y);
            let strip_rect = if ui.cursor().min.y + row_h <= visible_until {
                Some(Rect::from_min_size(
                    ui.cursor().min,
                    Vec2::new(ui.available_width(), row_h),
                ))
            } else {
                None
            };
            let t = &app.targets[idx];
            let color = app.status_color(t);
            // The line colour of this target, shown next to the status dot so the row already
            // says which trace in the chart belongs to it.
            let trace = app.trace_color(t);
            let txt = app.status_text(t);
            let sel = app.selected == idx;
            // Asked by id, not by `row`: a live move earlier in this very loop has already changed where the edited
            // entry sits, and its form must keep following the entry it belongs to.
            let editing_here = app.edit_id() == Some(id);
            let secs_ago = if t.last_probe_secs > 0.0 {
                now - t.last_probe_secs
            } else {
                -1.0
            };
            let label = t.label.clone();
            let host = t.host.clone();
            let kind_label = lang.kind(t.kind).to_string();
            let enabled = t.enabled;
            let auto_idx = app.color_idx_of(t);
            // Which palette slot this line uses, said plainly: automatic or picked by hand.
            // Shown as a tooltip on the colour dash in the row.
            let trace_tip = if auto_idx == crate::app::AUTO_COLOR {
                lang.tip_color_auto().to_string()
            } else {
                i18n::fill(
                    lang.tip_color_chosen_fmt(),
                    &[("n", (auto_idx + 1).to_string())],
                )
            };
            // Details for this one target: how much history is kept, and what it says about
            // the current window.
            let horizon = app.options.horizon_secs as f64;
            let from = t.series.last().map_or(0.0, |s| s.t_secs - horizon);
            let st = t.series.stats_window(from, f64::MAX, &mut scratch);
            // A row that is being carried is drawn faded and cannot be edited or grabbed again —
            // one drag at a time, and the ghost says which entry is in flight.
            let id = t.id;
            let carried = carried_id == Some(id);
            let strip_rect = match strip_rect {
                Some(rect) => rect,
                None => {
                    // A row the panel cannot show: it is not part of the layout, so no click and no drop can be aimed at
                    // it (see `visible_until`). Nothing is recorded for it and nothing is drawn for it.
                    continue;
                }
            };
            // While a row is being edited, show its settings instead of the label. The editor's *form*
            // is deliberately not part of `row_rects`: it has real controls, and a click on one of them
            // must never be read as "this row was clicked again".
            if editing_here {
                // Keep the row's click area alive while it is being edited, at exactly the height
                // the finished row had — otherwise the list would shift and a click aimed at a
                // neighbour would land somewhere else. The form hangs *below* that strip, so the strip
                // keeps its place and is recorded as the row's rect: that is where the row stands now and
                // where it will stand again once the form closes, which is the only way a drop can be
                // aimed at a list that has a form in it (see [`rows_prev_pass`]). Nothing else follows
                // from it while the form is open — a reorder is refused anyway (see `App::move_target`),
                // and the area senses clicks as nothing at all.
                let h = strip_rect.height();
                let (header, _) = ui
                    .allocate_exact_size(Vec2::new(ui.available_width(), h), egui::Sense::hover());
                row_ids.push(id);
                row_rects.push(Some(header));
                row_heights.push(h);

                // The panel is only 250 px wide, so the editor has to stay tight.
                ui.style_mut().spacing.item_spacing.y = 3.0;

                let frame = egui::Frame::new()
                    .fill(m.row_fill(true))
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(7, 6));
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(11.0), egui::Sense::hover());
                        let dot_rect = dot_rect.shrink(1.5);
                        ui.painter().rect_filled(dot_rect, 5.5, color);
                        // The colour dash: the same colour as the line in the chart, so the list
                        // alone already says which curve belongs to which target.
                        let (line_rect, _) =
                            ui.allocate_exact_size(Vec2::new(16.0, 11.0), egui::Sense::hover());
                        ui.painter().rect_filled(line_rect.shrink(4.5), 1.0, trace);
                        // Getting rid of the target one is working on: replaces the "remove
                        // target" button in the top right, which hit the selected target and so
                        // easily grabbed the wrong one.
                        if ui
                            .small_button(RichText::new(lang.remove_short()).color(m.danger_text()))
                            .on_hover_text(i18n::fill(
                                lang.tip_remove_fmt(),
                                &[("label", label.clone())],
                            ))
                            .clicked()
                        {
                            remove_idx = Some(idx);
                        }
                        ui.label(RichText::new(lang.edit_heading()).strong().size(12.0));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("×")
                                .on_hover_text(lang.tip_discard_edit())
                                .clicked()
                            {
                                edit_action = Some(App::cancel_edit);
                            }
                        });
                    });

                    // Name and address.
                    ui.add_sized(
                        [215.0, 20.0],
                        egui::TextEdit::singleline(&mut app.edit_label)
                            .hint_text(lang.hint_new_label())
                            .align(egui::Align2::LEFT_CENTER),
                    );
                    ui.add_sized(
                        [215.0, 20.0],
                        egui::TextEdit::singleline(&mut app.edit_host)
                            .hint_text(lang.hint_address())
                            .font(FontId::monospace(12.5))
                            .align(egui::Align2::LEFT_CENTER),
                    );

                    // Category and whether to measure it at all.
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("edit_kind")
                            .width(96.0)
                            .selected_text(RichText::new(lang.kind(app.edit_kind)).size(12.0))
                            .show_ui(ui, |ui| {
                                for k in TargetKind::all() {
                                    ui.selectable_value(&mut app.edit_kind, k, lang.kind(k));
                                }
                            });
                        ui.add_enabled_ui(!app.edit_host.trim().is_empty(), |ui| {
                            ui.checkbox(&mut app.edit_enabled, lang.measures_checkbox())
                                .on_hover_text(lang.tip_measures());
                        });
                    });

                    // The line colour. Left alone, the palette hands one out by position in the
                    // list; touching anything here stores a fixed number that yields the same
                    // colour after a restart and across re-sorting. The button doubles as the
                    // preview: it shows exactly the shade the chart paints.
                    let picked = app.edit_color_idx != app::AUTO_COLOR;
                    let shown_idx = if picked { app.edit_color_idx } else { auto_idx };
                    let swatch_name = if picked {
                        theme::swatch(app.edit_color_idx).name(lang)
                    } else {
                        lang.auto_color()
                    };
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("edit_color")
                            .width(96.0)
                            .selected_text(RichText::new(swatch_name).size(12.0))
                            .show_ui(ui, |ui| {
                                for (n, s) in theme::PALETTE.iter().enumerate() {
                                    if ui
                                        .selectable_value(
                                            &mut app.edit_color_idx,
                                            n,
                                            RichText::new(s.name(lang)).size(12.0),
                                        )
                                        .changed()
                                    {
                                        color_changed = true;
                                    }
                                }
                            });
                        ui.painter().rect_filled(
                            ui.available_rect_before_wrap().shrink(4.0),
                            2.0,
                            m.swatch(shown_idx),
                        );
                        ui.allocate_at_least(Vec2::new(18.0, 16.0), egui::Sense::hover());
                        if ui
                            .small_button(lang.auto_color())
                            .on_hover_text(lang.tip_color_auto_btn())
                            .clicked()
                        {
                            app.edit_color_idx = app::AUTO_COLOR;
                            color_changed = true;
                        }
                    });

                    // Details: what the stored history says about this target.
                    let avg = if st.replies > 0 {
                        format!("ø {} ms", crate::app::fmt_ms(st.avg as f32))
                    } else {
                        "ø —".to_string()
                    };
                    let span = if st.replies > 0 {
                        format!(
                            "{} / {} ms",
                            crate::app::fmt_ms(st.min),
                            crate::app::fmt_ms(st.max)
                        )
                    } else {
                        "— / — ms".to_string()
                    };
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            kind_label,
                            if enabled {
                                lang.active()
                            } else {
                                lang.paused()
                            }
                        ))
                        .weak()
                        .size(11.0),
                    );
                    ui.label(
                        RichText::new(i18n::fill(
                            lang.values_span_fmt(),
                            &[
                                ("replies", st.replies.to_string()),
                                ("total", st.total.to_string()),
                                ("avg", avg.clone()),
                            ],
                        ))
                        .weak()
                        .size(11.0)
                        .monospace(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            span,
                            i18n::fill(lang.loss_count_fmt(), &[("n", st.losses.to_string())])
                        ))
                        .weak()
                        .size(11.0)
                        .monospace(),
                    );

                    ui.horizontal(|ui| {
                        if ui.button(lang.save_btn()).clicked() {
                            edit_action = Some(App::commit_edit);
                        }
                        if ui.button(lang.cancel_btn()).clicked() {
                            edit_action = Some(App::cancel_edit);
                        }
                    });

                    // Enter commits, Escape cancels.
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if enter {
                        edit_action = Some(App::commit_edit);
                    } else if escape {
                        edit_action = Some(App::cancel_edit);
                    }
                });
                continue;
            }

            // The row's frame is drawn *exactly* on the strip allocated below, by giving its content the
            // height that is left after its own padding. Left to itself, a frame sizes its content from what
            // goes into it — two lines of text plus the item spacing between them — and that total is not the
            // same number twice running: egui caches a label's galley, so the first pass measures the text and
            // every pass after it gets the cached size. Rows crept a point further down each time they were
            // drawn, which is invisible in a resting list but ruins a drag: the rows are measured against the
            // strips they were drawn with, so after a few passes of a held pull the last row was registered
            // several points below where it appeared and a drop aimed at it landed on the row before.
            //
            // `inner_margin` rather than `outer_margin`, because egui's `Frame::begin` subtracts *both* margins
            // from the rect it is handed (`total_margin`) and lets the content grow out of what is left, so any
            // margin at all makes an unconstrained frame taller than the strip it was given.
            let mut frame = egui::Frame::new()
                .fill(m.row_fill(sel))
                .corner_radius(5.0)
                .inner_margin(egui::Margin::symmetric(7, 5));
            let content_height = (ROW_H - GAP_Y - frame.inner_margin.topf() * 2.0).max(0.0);
            if carried {
                // The row in flight is painted as a ghost of itself: still in place, so the
                // list does not jump around while the pointer looks for a new spot.
                frame.fill = m.row_fill(true);
                frame.fill = frame.fill.linear_multiply(0.45);
            }
            // A row is a button. `Frame::show` only ever registers `Sense::hover()`, so the
            // click area has to be claimed separately: first the strip allocated below, then the
            // frame painted on top of it. Hovering brightens the strip and shows the pointer
            // cursor, which says "a click opens the editor". Labels stay non-selectable so none
            // of them swallows the press that belongs to the row.
            //
            // On top of that a drop needs an answer to "which row is the pointer standing in", and
            // neither of the two obvious ways to get one works:
            //
            // - `hovered()` cannot answer it. While an entry is carried egui marks only the widget
            //   it is dragging as hovered, and a release always goes back to the widget that was
            //   pressed — no other row ever reports anything.
            // - A taller *allocated* area cannot answer it either, because allocation is layout: a
            //   band reserved for row 0 pushes every row below it down, and the rows would stop
            //   being where the pointer expects them. The list has to keep its exact pitch.
            //
            // So the band is not allocated but computed from the strip: it reaches through the empty
            // line below the row up to the next row's top edge, which costs no layout space and makes
            // the rows cover the list without a seam. That matters because egui's answer for a point
            // that belongs to no widget is to pick the *nearest* one — not something a drop position
            // should be built on. `row_rects` keeps the strip alone: that is what a click lands on.
            // The strip is registered under the target's own id, so egui knows it by name instead of by position.
            // That is what lets a drag hold on to its entry across a reordering: an id follows the entry, a position
            // or an index does not. It is also how `strip_target` recognises our own widgets in egui's record of a
            // pass, which is where the release geometry comes from (see [`DragRelease`]).
            //
            // `new_child` is what makes that name exact, and it matters that it is: every other way of
            // getting a widget id in egui mixes in `next_auto_id_salt`, a counter that runs while the rows
            // are being built. A scope salted with `push_id` registers its child as
            // `strip_id(id).with(counter)`, so the name changes the moment a row above appears, disappears
            // or changes height — which is exactly what a live drag does. Holding on to such an id would
            // lose the row on the pass that reordered the list. Here the strip is allocated in a child ui
            // whose own id *is* `strip_id(id)`, and `interact` registers that id itself.
            // The strip is allocated *before* anything else, because everything below is measured against it: the
            // frame painted on it, the grip drawn over it, and above all `row_rects`, which is what a click and both
            // halves of a drag are aimed with.
            let strip_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(strip_id(id))
                    .max_rect(ui.max_rect()),
            );
            // The row's click area is *this* rect, and it has to be measured against this exact place: the live move
            // below can reorder the list while this pass is still drawing, so a point that was over this row when the
            // pointer passed over it is not necessarily over it any more. `strip_rect` therefore goes into `row_rects`
            // (see below), and egui's own answer for the widget is only ever used for what a widget can say about its
            // own press — hover, drag start, click — never for where anything is.
            let strip = strip_ui.interact(strip_rect, strip_id(id), egui::Sense::click_and_drag());
            ui.advance_cursor_after_rect(strip_rect);
            let mut strip = strip.on_hover_cursor(if carried_id == Some(id) {
                CursorIcon::Grabbing
            } else {
                CursorIcon::PointingHand
            });
            if strip.hovered() {
                frame.fill = m.row_fill(true);
            }
            // The same press that opens the editor on a quick click becomes a drag as soon as the
            // pointer travels: holding a row and pulling it up or down reorders the list.
            // `Sense::click_and_drag` is what makes egui tell the two apart, and how far the pointer has
            // to travel before egui calls it a drag is not ours to choose — it is
            // `InputOptions::max_click_dist`, 6 points by default, adjustable in egui's own debug
            // options. That is deliberate: this app must not invent its own threshold and then disagree
            // with the one egui reports the click with, because a row has to answer "click or pull" for
            // one press exactly once. The slop is small enough that a real pull always leaves it, and a
            // hand that stays inside it let go on purpose, which is a click.
            if strip.drag_started_by(egui::PointerButton::Primary) {
                // The entry egui has just decided is a pull goes into flight here, and nowhere else: `App::drag_row` is what
                // keeps the carry alive on the passes after this one, where egui's own answer for a drag has already been read
                // once at the top of the pass (`carried_id`) and cannot be asked again. Read from the row rather than from that
                // top-of-pass answer because a flag is only ever true for the widget it was handed to — which is exactly what
                // says *which* entry egui picked up, and the whole reason `App::drag_row` names an id and not a row.
                app.drag_row = Some(id);
            }
            if carried_id == Some(id) {
                // Carrying a row says so with the tooltip — the grip drawn at the left edge
                // below is only a hint, and the cursor already changed to a grabbing hand.
                strip = strip.on_hover_text(lang.tip_drag_row());
            }
            // The row's content goes into a child ui pinned to that strip, so the frame is
            // painted exactly on the click area and cannot shift the list. `new_child` gives us
            // the child directly; its used rect is handed back to the list by `prepared.end`.
            let mut prepared =
                frame.begin(&mut ui.new_child(egui::UiBuilder::new().max_rect(strip_rect)));
            // The content is pinned to the height above and centred in it, so the painted frame ends up
            // exactly as tall as the click area no matter how tall its text turns out to be.
            prepared.content_ui.set_min_height(content_height);
            {
                let ui = &mut prepared.content_ui;
                ui.horizontal(|ui| {
                    let (dot_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(11.0), egui::Sense::hover());
                    let dot_rect = dot_rect.shrink(1.5);
                    ui.painter().rect_filled(dot_rect, 5.5, color);
                    // A dash in the line colour: reading the list, one immediately sees which
                    // curve in the chart belongs to this target. The dash is as tall as the dot
                    // so the two form one line instead of looking misaligned.
                    let (line_rect, _) =
                        ui.allocate_exact_size(Vec2::new(16.0, 11.0), egui::Sense::hover());
                    ui.painter().rect_filled(line_rect.shrink(4.5), 1.0, trace);
                    // What the dash means: an automatic or a hand-picked line colour. The knob is
                    // the same one as in the chart so both places mean the same thing.
                    let knob = Rect::from_center_size(
                        line_rect.center(),
                        Vec2::splat(crate::theme::LEGEND_KNOB),
                    );
                    ui.put(knob, Label::new("").selectable(false))
                        .on_hover_text(&trace_tip);
                    // The dash belongs to the text: spacing first, then the label.
                    ui.add_space(7.0);
                    // The value on the right belongs to the row just as much as the text does, so the
                    // text column gets what is left of it and no more. Its width is measured rather
                    // than guessed: the same column shows "—", "46 ms", "1234 ms" or a translated
                    // word, and a column that ignores that runs its second line under the number.
                    let value_w = {
                        let mut font = ui.style().text_styles[&egui::TextStyle::Monospace].clone();
                        font.size = 12.0;
                        ui.painter()
                            .layout_no_wrap(txt.clone(), font, color)
                            .rect
                            .width()
                    };
                    let text_w =
                        (ui.available_width() - value_w - ui.spacing().item_spacing.x).max(40.0);
                    ui.vertical(|ui| {
                        // Truncating needs a width to truncate at, and this is it: both lines end with
                        // an ellipsis inside the column instead of reaching into the value.
                        ui.set_max_width(text_w);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(
                            Label::new(RichText::new(label).strong().size(13.0))
                                .selectable(false)
                                .truncate(),
                        );
                        let age = if secs_ago < 0.0 {
                            lang.not_probed_yet().to_string()
                        } else {
                            i18n::fill(
                                lang.ago_fmt(),
                                &[("n", (secs_ago.max(0.0) as i64).to_string())],
                            )
                        };
                        ui.add(
                            Label::new(RichText::new(format!("{host} · {age}")).weak().size(11.0))
                                .selectable(false)
                                .truncate(),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            Label::new(RichText::new(&txt).color(color).monospace().size(12.0))
                                .selectable(false),
                        );
                    });
                });
            }
            // Closes the frame: paints it over the strip and hands its area back to the list.
            prepared.end(ui);
            // The grip: two columns of dots at the left edge, shown on the hovered row and on
            // the one in flight. Painted after `prepared.end`, which puts the frame's fill on
            // top of everything drawn inside the child — so this is the only way to keep the
            // hint visible. It sits in the frame's 7 px padding and senses nothing: a press on
            // it is a press on the row, exactly as before.
            if strip.hovered() || carried {
                let grip = Rect::from_min_size(
                    strip_rect.left_center() + Vec2::new(1.0, -4.5),
                    Vec2::new(GRIP_W - 2.0, 9.0),
                );
                let tint = if carried {
                    m.swatch(app.color_idx_of(&app.targets[idx]))
                } else {
                    m.text_dim()
                };
                for row in 0..3 {
                    for col in 0..2 {
                        let c =
                            grip.min + Vec2::new(1.5 + col as f32 * 5.0, 1.5 + row as f32 * 3.0);
                        ui.painter().circle_filled(c, 1.1, tint);
                    }
                }
            }

            // The strip allocated above is the row's click area, and it is what egui matches a press against. It is recorded
            // so the drag can answer "which row is the pointer standing in" (see [`rows_prev_pass`]) — the click itself no
            // longer needs it, because egui names the widget a press belongs to.
            //
            // Recorded against `order` rather than pushed, because a row the panel could not draw at all still occupies a
            // place in the list: every consumer of `row_rects` numbers the rows by position, so they all have to be counting
            // the same list. See [`rows_prev_pass`], which reads these back on the next pass in exactly this order.
            row_ids.push(id);
            row_rects.push(Some(strip_rect));
            row_heights.push(strip_rect.height());

            // The editor opens only for a row that offers a strip, so this is written after the recording above rather than
            // before it. A row whose entry is being edited records its header and draws the form below it, and that header
            // senses no clicks at all, so `clicked_on` can never name it: a press inside the form's own controls cannot be
            // read as a row asking to be edited again.
            if clicked_on == Some(id) {
                begin_edit_idx = Some(id);
            }
        }

        // The live move, applied only now that every row of this pass has been drawn — see `move_now`.
        //
        // Unless the pointer let go in this very pass, or egui is still calling one of its widgets "just released" (see
        // [`release_is_at_hand`]). A release is answered below and is meant to be the last word of a gesture, so a follow asked for by either of
        // those passes would move the list *after* that answer had been measured — and because egui reports `drag_stopped_id()` on every pass
        // after the one it saw the release in, such a stray move is still there when the drop has already put the entry where the hand left it.
        // The visible result was an entry that jumped one place further than the pointer ever went.
        if let Some((id, to)) = move_now {
            if !release_at_hand {
                app.move_target(id, to);
            }
        }

        // Then the layout is published, and it is published in the order of the list *after* that move — which is what the
        // next pass's rows will be drawn in. The loop above recorded these lists in the order this pass drew them, and once a
        // live move has shuffled the entries those two orders are no longer the same thing: pairing a rect with an entry by
        // position would then hand every row another row's geometry, and a pointer aimed at one row would be read as a place
        // on another. Reordering here is what keeps `App::row_rects[row]` meaning "the rect of `App::targets[row]`", which is
        // the only contract the click test, [`rows_prev_pass`] and the tests can rely on. A row this pass drew no strip for —
        // an open editor's form — travels with its own hole, so a list that has been dragged about still answers "no rect" for
        // exactly the entry it has no rect for.
        let mut placed: Vec<Option<Rect>> = vec![None; app.targets.len()];
        for (id, rect) in row_ids.into_iter().zip(row_rects) {
            if let Some(idx) = app.targets.iter().position(|t| t.id == id) {
                placed[idx] = rect;
            }
        }
        // Rows the list has that this pass never drew at all — one below a scrollbar's edge, or added between passes. They
        // keep their previous rect rather than becoming holes: a row can leave the screen on a scroll and come back, and an
        // entry nobody ever drew is not the same as an editor's form (see `editing_here` in the loop above), so guessing from
        // its neighbour would be a guess about a place the pointer may well mean.
        for (idx, slot) in placed.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = match (app.row_ids.get(idx), app.row_rects.get(idx)) {
                    (Some(id), Some(rect)) if app.targets.get(idx).is_some_and(|t| t.id == *id) => {
                        *rect
                    }
                    _ => None,
                };
            }
        }
        // Published as one piece, and kept for a pass longer than `row_rects`: a drop is measured against whichever of the last two layouts the
        // pointer was really aimed at (see [`release_slot`]), and that question cannot be answered from two lists read separately — pairing an id
        // from one generation with a rect from another is what let a drop name a row the pointer never passed.
        //
        // This pass's own layout, which means the rows as they will stand on the *next* pass: `placed` was reordered to the order after the live
        // move above, so it is exactly what the panel draws next time and nothing has moved since. Pushed here rather than at the top of the pass
        // because a release is decided below, in this very pass, and the layout the hand travelled over has to be among the two that get asked —
        // which it is as soon as this one is the newest. The pass before it is the other.
        app.drag_layouts.push(RowLayout {
            rects: placed.clone(),
            ids: app.targets.iter().map(|t| t.id).collect(),
        });
        if app.drag_layouts.len() > 2 {
            app.drag_layouts.remove(0);
        }
        app.row_ids = app.targets.iter().map(|t| t.id).collect();
        app.row_rects = placed;
        app.row_heights = app
            .row_rects
            .iter()
            .map(|rect| rect.map_or(0.0, |r| r.height()))
            .collect();

        // The release, decided here rather than in the loop above: it names a place on screen and an entry, and neither can be
        // trusted to a row — while the hand travelled, every pass shuffled the list under the pointer, so the row that carries the
        // entry now is not necessarily the one that held it when the button came up. It runs after the live move above so the two
        // halves of one pass cannot both move: `move_now` already refuses to fire on a pass that saw the release, and here the list
        // has had no move applied at all since the layouts were read, which is what keeps [`release_slot`]'s numbering valid.
        //
        // Asked against the layouts as they stand *here*, published again just above: the pass that holds a release is also the pass
        // that draws whatever the screen will show next, and on the frame where that changes — an editor closed by Esc in the same
        // frame as the release being the case — the rows below the form have already moved back up into their places by the time this
        // line runs. Asked from the top of the pass, the hand would rest in the space the form has just left and no layout could name
        // a row there, so the entry would stay where it was although the pointer plainly said otherwise.
        //
        // The older layouts are still asked, newest first: this press began before the pass that published them, so where the current
        // rows cannot answer — a point outside them — the layout the pointer actually travelled over still knows which row it meant.
        //
        // A slot that names no move — because the point is outside the rows or already is the entry's own place — only lets go of
        // the entry, which is what makes "dropped nowhere in particular" mean "stay where you got to".
        if let Some(release) = release {
            let now = layouts(app);
            match release_slot(&now, &release) {
                Some(to) => {
                    app.drop_moved_target(release.target.expect("only set for our rows"), to)
                }
                None => {
                    app.drag_row = None;
                    // The order may well have changed on an earlier pass of the same drag, and this is the only moment that
                    // knows the gesture is over.
                    app.save_now();
                }
            }
        }

        // Now apply the actions after the loop, when `app` is no longer borrowed. Checked before the
        // edit actions so a drop can never be followed by an editor opening on a stale index. Note that this runs *after* the
        // live move and the drop above: a press that became a pull has already been denied its click (`clicked_on`), so there
        // is nothing here left to reconcile — and if there were, the list it would be measured against is now the one the drag
        // produced rather than the one the pointer travelled over.
        if let Some(id) = begin_edit_idx {
            app.begin_edit_by_id(id);
        }
        if let Some(action) = edit_action {
            action(app);
        }
        // Removing a target closes the editor first: it points at an index that disappears.
        if let Some(id) = remove_idx {
            app.cancel_edit();
            app.selected_by_id(id);
            app.remove_target_by_id(id);
        }
        // The colour lives on the target, not in `AppOptions`, so `commit_edit` cannot hand it
        // to the autosave. Flagging it here lets `left` write the state once the panel is done.
        if color_changed {
            app.color_dirty = true;
        }
    });

    app.stats_scratch = scratch;
}

pub fn bottom(ui: &mut egui::Ui, app: &mut App) {
    let lang = app.lang();
    let m = app.theme_mode;
    // Taken out of the app so it can be reused while `app.targets` is borrowed. The old
    // `stats()` window started at the newest sample and had no upper bound.
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    egui::Panel::bottom("status_bar")
        .frame(
            egui::Frame::new()
                .fill(ui.style().visuals.window_fill)
                .inner_margin(egui::Margin::symmetric(10, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let dot = m.run_dot(app.running);
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, dot);
                ui.label(
                    RichText::new(if app.running {
                        lang.running()
                    } else {
                        lang.stopped()
                    })
                    .strong(),
                );

                let mut probed = 0usize;
                let mut lost = 0usize;
                let horizon = app.options.horizon_secs as f64;
                for t in &app.targets {
                    // The counters follow the chart, so they say what is on screen. A target that
                    // is filtered out or hidden is still measured — the events list and the CSV
                    // keep counting it.
                    if !app.shown(t) {
                        continue;
                    }
                    let s = t.series.stats_window(
                        t.series.last().map_or(0.0, |s| s.t_secs - horizon),
                        f64::MAX,
                        &mut scratch,
                    );
                    probed += s.total;
                    lost += s.losses;
                }
                ui.separator();
                ui.label(i18n::fill(lang.probes_fmt(), &[("n", probed.to_string())]));
                ui.label(
                    RichText::new(i18n::fill(lang.losses_fmt(), &[("n", lost.to_string())]))
                        .color(m.loss_count(lost > 0)),
                );
                // Everything is counted, only what belongs in the protocol is kept — the two numbers
                // are rarely equal, and one alone would easily be read as "this much happened".
                if app.log_total > app.log.len() {
                    ui.label(i18n::fill(
                        lang.events_count_fmt(),
                        &[
                            ("n", app.log.len().to_string()),
                            ("all", app.log_total.to_string()),
                        ],
                    ));
                } else {
                    ui.label(i18n::fill(
                        lang.events_count_short_fmt(),
                        &[("n", app.log.len().to_string())],
                    ));
                }
                ui.separator();
                ui.label(RichText::new(&app.status_msg).weak().italics());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // A8: past an hour, "65:00" would be a number one has to convert first.
                    let secs = app.engine.elapsed_secs() as i64;
                    let uptime = if secs >= 3_600 {
                        format!(
                            "{}:{:02}:{:02}",
                            secs / 3_600,
                            (secs % 3_600) / 60,
                            secs % 60
                        )
                    } else {
                        format!("{:02}:{:02}", secs / 60, secs % 60)
                    };
                    ui.label(
                        RichText::new(i18n::fill(lang.uptime_fmt(), &[("t", uptime.clone())]))
                            .monospace()
                            .weak(),
                    );
                    ui.separator();
                    // A2: the state file writes itself every 8 s, so a "save" button would be a third
                    // way to do the same thing. What is missing is the feedback — so this is where
                    // the time of the last save is shown.
                    let age = app.save_age_secs();
                    let saved = if age < 0.0 {
                        lang.not_saved_yet().to_string()
                    } else if age < 20.0 {
                        i18n::fill(
                            lang.saved_secs_ago_fmt(),
                            &[("n", (age as i64).to_string())],
                        )
                    } else {
                        i18n::fill(
                            lang.saved_min_ago_fmt(),
                            &[("n", (age as i64 / 60).to_string())],
                        )
                    };
                    ui.label(RichText::new(saved).weak().size(11.5));
                    if ui
                        .small_button(lang.export_short())
                        .on_hover_text(lang.tip_export_short())
                        .clicked()
                    {
                        app.quick_export();
                    }
                });
            });
        });
    app.stats_scratch = scratch;
}

pub fn center(ui: &mut egui::Ui, app: &mut App) {
    center_ex(ui, app, false);
}

/// `shot_frame` lets the seeded example history through the "only live samples" rule in
/// `chart`, so a still picture can show a full trace without pretending to be pinging.
pub fn center_ex(ui: &mut egui::Ui, app: &mut App, shot_frame: bool) {
    let m = app.theme_mode;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(m.plot_fill())
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ui, |ui| match app.view {
            View::Chart => chart(ui, app, shot_frame),
            View::Events => events_view(ui, app),
            View::Stats => stats_view(ui, app),
            View::Settings => settings(ui, app),
        });
}

/* ------------------------------------------------------------------ chart */

/// The distances the time axis may pick its grid lines from: whole, round numbers of seconds.
const NICE_STEPS: [f64; 19] = [
    1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1_800.0, 3_600.0, 7_200.0,
    10_800.0, 14_400.0, 21_600.0, 43_200.0, 86_400.0,
];

/// Gap between the plot and the axis labels under it, as they are painted.
/// Room one label on the time axis needs next to its neighbour: the widest thing the axis prints
/// (a negative „-00:00 h“ is eight glyphs of its 10 px monospace font) plus the air between two.
const AXIS_LABEL_SLOT: f32 = 56.0;

/// Gap between the plot and the axis labels under it, as they are painted.
const AXIS_LABEL_GAP: f32 = 3.0;

/// Height the row of axis labels under the plot needs: its gap plus one line of the 10 px
/// monospace font it is painted in. Allocated, not just painted into — see [`PLOT_BOTTOM_H`].
const AXIS_LABEL_H: f32 = AXIS_LABEL_GAP + 13.0;

/// Gap between the axis labels and the event log's heading.
const LOG_GAP: f32 = 6.0;

/// Height of the heading row the event log draws above its list (its buttons decide it).
const LOG_HEAD_H: f32 = 24.0;

/// Height of the event log's list in the chart view.
const LOG_H: f32 = 120.0;

/// What the plot does not get, out of the height the centre panel offers: the axis labels, the gap,
/// the event log's heading and its list. The plot is allocated what is left, which is what keeps the
/// labels from being painted on top of the log's buttons — they hang below the plot's own rect, in a
/// band the layout has to be told about.
const PLOT_BOTTOM_H: f32 = AXIS_LABEL_H + LOG_GAP + LOG_HEAD_H + LOG_H;

fn chart(ui: &mut Ui, app: &mut App, shot_frame: bool) {
    let lang = app.lang();
    let m = app.theme_mode;
    // `shot_frame` means "draw the picture that will be captured", which includes the seeded
    // example history. The clock must come from the same place the samples are stamped with
    // (`runtime::secs_since_start`); egui's own frame counter would make the x axis jump back
    // to zero on a hidden window and trip `History::add`'s monotonic assert.
    let now = if shot_frame {
        crate::runtime::secs_since_start() - app.pan_offset_secs
    } else {
        app.engine.elapsed_secs()
    };
    let horizon = app.options.horizon_secs as f64;

    ui.horizontal(|ui| {
        ui.label(RichText::new(lang.chart_heading()).strong());
        // No "Live" switch here: the top bar has it, and two controls for one state only invite
        // confusion about which one is speaking.
        if ui.button(lang.back_btn()).clicked() {
            app.follow_live = false;
            app.pan_offset_secs += horizon * 0.25;
        }
        let back = ui.button(lang.forward_btn());
        if back.clicked() {
            app.pan_offset_secs = (app.pan_offset_secs - horizon * 0.25).max(0.0);
            if app.pan_offset_secs < 0.5 {
                app.follow_live = true;
            }
        }
        ui.separator();
        if ui
            .button(lang.next_btn())
            .on_hover_text(lang.tip_next_outlier())
            .clicked()
        {
            app.jump_to_next_outlier();
        }
        if ui
            .button(lang.earlier_btn())
            .on_hover_text(lang.tip_earlier())
            .clicked()
        {
            app.jump_to_prev_outlier();
        }
    });

    // The legend is a row of its own, and it wraps: with every target in the same row as the
    // buttons, a long list simply grew past the window's right edge — and because an unwrapped
    // row also moves the cursor it advanced, the plot below it was then allocated a width the
    // window does not have, so its axis labels ended up outside the screen.
    //
    // The id whose visibility a click on its name asked to flip; applied after the loop, when
    // `app.targets` is no longer borrowed by it.
    let mut toggle: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        for t in &app.targets {
            // Only targets the text filter and the category chips let through are listed; among
            // those, a hidden one still appears (weak, hollow knob) so its name can bring it back.
            if !app.passes_filter(t) {
                continue;
            }
            let c = app.trace_color(t);
            // A small colour knob in front of the name, so the legend uses the same mark as the
            // target list: square, the same size everywhere. A hidden target shows it as an outline
            // only — so it stays visible which colour it would have once it is back.
            let (knob_rect, _) = ui
                .allocate_exact_size(Vec2::splat(crate::theme::LEGEND_KNOB), egui::Sense::hover());
            if t.hidden {
                ui.painter().rect_stroke(
                    knob_rect.shrink(1.5),
                    2.0,
                    Stroke::new(1.0f32, c),
                    egui::StrokeKind::Inside,
                );
            } else {
                ui.painter().rect_filled(knob_rect.shrink(1.0), 2.0, c);
            }
            // Clicking the name shows or hides exactly this target — measuring continues, only the
            // drawing disappears. The name stays readable but dim so the hidden target can be found.
            let name = RichText::new(&t.label)
                .color(if t.hidden { m.text_weak() } else { c })
                .strong();
            let tip = if t.hidden {
                i18n::fill(lang.tip_legend_hidden_fmt(), &[("host", t.label.clone())])
            } else {
                i18n::fill(
                    lang.tip_legend_shown_fmt(),
                    &[("host", t.host.clone()), ("status", app.status_text(t))],
                )
            };
            if ui
                .add(Label::new(name).selectable(false).truncate())
                .on_hover_text(tip)
                .clicked()
            {
                toggle = Some(t.id);
            }
        }
        if let Some(id) = toggle {
            app.toggle_hidden_by_id(id);
        }
    });

    let (rect, resp) = ui.allocate_at_least(
        Vec2::new(
            ui.available_width(),
            (ui.available_height() - PLOT_BOTTOM_H).max(160.0),
        ),
        egui::Sense::click_and_drag(),
    );

    // Right edge of the visible window, including the pull-back a still picture needs. Computed
    // before the wheel zoom so the anchor maths below and the axis drawing agree on one number.
    let mut t_max = now - app.pan_offset_secs;
    if shot_frame {
        let mut newest = f64::MIN;
        for t in &app.targets {
            if let Some(s) = t.series.samples().last() {
                newest = newest.max(s.t_secs);
            }
        }
        if newest.is_finite() && newest < t_max {
            let slack = (t_max - newest).min(horizon * 0.45);
            if slack > 0.0 {
                t_max -= slack;
            }
        }
    }

    if resp.dragged() {
        app.follow_live = false;
        app.pan_offset_secs += (resp.drag_delta().x as f64) * (horizon / rect.width() as f64);
        if app.pan_offset_secs < 0.0 {
            app.pan_offset_secs = 0.0;
        }
    }

    // Wheel over the plot zooms the time window instead of scrolling a panel that has no scroll
    // bar. The moment under the cursor keeps its place, so zooming in on an event lands where it
    // was pointed at. The horizon is kept on the preset ladder from the top bar: `HORIZON_CHOICES`
    // picks the step above or below the requested size, which keeps the number and the saved value
    // familiar rather than turning them into a continuum of odd fractions.
    // The plot is the central panel, so "the pointer is over the plot" is exactly "the pointer is
    // inside the panel's rect and no other widget has taken it".
    let zoom: f32 = ui.ctx().input(|i| {
        let Some(pos) = i.pointer.hover_pos() else {
            return 0.0;
        };
        if !rect.contains(pos) {
            return 0.0;
        }
        // egui 0.37 no longer keeps the unsmoothed scroll delta: `smooth_scroll_delta` spreads one
        // mouse-wheel notch over several passes, and the ladder below is stepped per notch. So the
        // plot reads this pass's wheel events — the same numbers egui used to add up into
        // `raw_scroll_delta`, in points.
        let per_line = egui::InputOptions::default().line_scroll_speed;
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    egui::MouseWheelUnit::Line => per_line * delta.y,
                    egui::MouseWheelUnit::Point => delta.y,
                    egui::MouseWheelUnit::Page => i.content_rect().height() * delta.y,
                }),
                _ => None,
            })
            .sum::<f32>()
    });
    if zoom.abs() > 0.5 && rect.width() > 1.0 {
        let u = ((resp.hover_pos().unwrap_or(rect.center()).x - rect.left()) as f64
            / rect.width() as f64)
            .clamp(0.0, 1.0);
        // Factor per notch: four notches cover roughly a decade of zoom.
        let factor = f64::from(-zoom * 0.12).exp();
        let want = (horizon * factor).clamp(5.0, 86_400.0);
        let next = pick_horizon(want);
        if (f64::from(next) - horizon).abs() > f64::EPSILON {
            // Anchor the cursor's time between old and new window, then clamp at "now", which is
            // as far right as the plot ever goes.
            let anchor = t_max + u * (f64::from(next) - horizon);
            app.horizon_secs_set(next);
            app.follow_live = false;
            app.pan_offset_secs = (now - anchor).max(0.0);
        }
    }

    // A double click returns to the live window, the same way it does in a map or a timeline.
    if resp.double_clicked() {
        app.follow_live = true;
        app.pan_offset_secs = 0.0;
    }

    let t_min = t_max - horizon;

    // B5: an empty chart is an answer without a question. With no target on screen, the area says
    // *why* it is empty — filter, category or hidden targets are three different knobs to turn.
    let shown_count = app.targets.iter().filter(|t| is_shown(app, t)).count();
    if shown_count == 0 && !app.targets.is_empty() {
        let why = if !app.all_kinds_on() {
            lang.all_kinds_off()
        } else if app.targets.iter().all(|t| t.hidden) {
            lang.why_hidden()
        } else {
            lang.why_filter()
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            why,
            FontId::proportional(13.0),
            m.text_dim(),
        );
    }

    // The p95 line sits at the 95th percentile of the replies inside the visible window, so
    // it follows the trace instead of being pushed off by one long spike. The scan goes into
    // a scratch buffer on the app, which keeps this allocation-free per frame.

    // Vertical grid + time labels.
    let step = step_for_width(horizon, t_max - t_min, rect.width());
    // Wide windows are labelled with the wall clock, so a sample's real stamp is needed to
    // turn an axis position into a time of day. All series share one origin (see `reanchor`),
    // so any target that has measured gives the offset between the two clocks.
    let clock_at_ms = app.targets.iter().find_map(|t| {
        t.series
            .last()
            .map(|s| s.at_ms - (s.t_secs * 1000.0) as i64)
    });
    let mut gt = (t_min / step).ceil() * step;
    while gt <= t_max {
        let x = map_x(gt, t_min, t_max, rect);
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, m.grid()),
        );
        let label = match clock_at_ms {
            Some(origin) => fmt_axis(gt, horizon, Some(origin + (gt * 1000.0) as i64)),
            None => fmt_axis(gt, horizon, None),
        };
        // A tick's label begins at its gridline, so the last one of a series would run past the
        // plot's right edge — and the plot reaches the window's edge, so it would end up outside the
        // window. Its gridline stays, the number is left out: a label that does not fit is not a
        // label, and pulling it back inside would put it on top of its neighbour.
        let galley = ui
            .painter()
            .layout_no_wrap(label, FontId::monospace(10.0), m.axis_text());
        if x + galley.size().x <= rect.right() + 1.0 {
            ui.painter().galley(
                egui::pos2(x, rect.bottom() + AXIS_LABEL_GAP),
                galley,
                m.axis_text(),
            );
        }
        gt += step;
    }

    // Milliseconds scale: fit the visible data, with a floor so 0 is always shown.
    let mut y_p95 = 40.0f32;
    for t in &app.targets {
        if !is_shown(app, t) {
            continue;
        }
        // The scale follows the 95th percentile of what is visible, so a single huge
        // spike cannot flatten the whole trace into a flat line at the bottom. `chart_p95`
        // keeps its capacity between frames and is cleared here instead of reallocated.
        let v = &mut app.chart_p95;
        v.clear();
        v.extend(
            t.series
                .samples()
                .iter()
                .filter(|s| s.t_secs >= t_min && s.t_secs <= t_max && history::is_ok(s.rtt_ms))
                .map(|s| s.rtt_ms),
        );
        if !v.is_empty() {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let i = (((v.len() - 1) as f64) * 0.95).round() as usize;
            y_p95 = y_p95.max(v[i]);
        }
    }
    let scale_top = (y_p95 * 1.15).max(40.0);

    // C4: which measurements count as outliers depends on the target's whole series and only changes
    // when a new value arrives. The curves, the rings and the hover read-out need the same cut — so
    // it is computed once here per frame and target and stored in `cut_scratch` (indexed by target
    // id, so the read-out never has to re-sort). A hidden target gets `f64::MAX`: it has nothing to
    // mark.
    {
        // Index instead of iterator: `is_shown` needs `app` unchanged, so the vector must not be held
        // as a mutable borrow at the same time. The cut goes into a local list and is swapped in
        // afterwards.
        let mut cuts = Vec::with_capacity(app.targets.len());
        for t in &app.targets {
            cuts.push(if is_shown(app, t) {
                history::outlier_cut(t.series.samples()).0
            } else {
                f64::MAX
            });
        }
        app.cut_scratch = cuts;
    }

    // The millisecond lines follow the scale instead of being four fixed numbers. A fixed ladder is
    // fine for a chart that stays under a hundred milliseconds, but on one that runs into the
    // thousands every one of its lines lands within a finger's width of the bottom — and their labels
    // stand on top of one another. So: the smallest round step whose labels still fit under each
    // other, which also keeps every label a number one can read at a glance.
    let step_ms = ms_band_step(scale_top, rect.height());
    let mut b = step_ms;
    while b < scale_top {
        let y = map_y(b, scale_top, rect);
        // The number stands above its line, the scale's own top label stands at the top edge — so a
        // line is left out when its number would end up under that label.
        if y >= rect.top() + 2.0 + 2.0 * AXIS_BAND_H {
            ui.painter().line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                Stroke::new(1.0_f32, m.grid()),
            );
            ui.painter().text(
                egui::pos2(rect.left() + 3.0, y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{}{}", b as i32, lang.ms_suffix()),
                FontId::monospace(10.0),
                m.axis_text(),
            );
        }
        b += step_ms;
    }
    // The top edge of the plot is where the auto scale ends, and that number changes with the
    // data. Said out loud, it stops being a guess — especially when spikes are clamped there.
    ui.painter().text(
        egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        format!("{:.0}{}", scale_top, lang.ms_suffix()),
        FontId::monospace(10.0),
        m.axis_text(),
    );

    // Loss band along the bottom.
    let loss_h = 14.0;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - loss_h),
            rect.right_bottom(),
        ),
        0.0,
        m.loss_band(),
    );

    for t in &app.targets {
        if !is_shown(app, t) {
            continue;
        }
        // A stopped target still has a history, and hiding it would leave the plot empty
        // until the first probe finishes. The legend dot says whether a trace is live or
        // paused; the line itself stays drawn either way.
        let sel = app.targets.get(app.selected).map(|x| x.id) == Some(t.id);
        // Every target keeps one colour for its whole line. Colouring each segment by its latency
        // band was unreadable as soon as several targets shared the plot: two traces in the same
        // band were indistinguishable, and one trace changed colour mid-line. The band is still
        // visible on the y axis and in the status dot; what the line has to say is "which target".
        // A selected trace gets a thin outline in the text colour so it also stands out where two
        // lines of the same family overlap.
        let line = app.trace_color(t);
        let width = if sel { 2.0_f32 } else { 1.5_f32 };
        // Outliers get a ring on the time axis, at the same cut the statistics use.
        let cut = app.cut_scratch.get(t.id).copied().unwrap_or(f64::MAX);
        let mut prev: Option<(f32, f32)> = None;
        // The window is half-open like everywhere else in the chart: a sample that
        // starts a new window belongs to that one, so it must not be drawn twice.
        // The outlier ring must land where the sample is drawn. When a spike sits above
        // `y_max` it gets clamped to the top edge, so the ring goes there too instead of
        // off-canvas (that hid every spike in the example view).
        for s in t.series.samples() {
            if s.t_secs < t_min || s.t_secs >= t_max {
                continue;
            }
            let x = map_x(s.t_secs, t_min, t_max, rect);
            if !history::is_ok(s.rtt_ms) {
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x - 1.5, rect.bottom() - loss_h),
                        egui::pos2(x + 1.5, rect.bottom()),
                    ),
                    0.0,
                    m.loss_fill(),
                );
                prev = None;
                continue;
            }
            let y = map_y(s.rtt_ms.min(scale_top), scale_top, rect);
            if let Some((px, py)) = prev {
                // One colour per target: the line says which trace belongs to which row.
                ui.painter().line_segment(
                    [egui::pos2(px, py), egui::pos2(x, y)],
                    Stroke::new(width, line),
                );
            }
            if (s.rtt_ms as f64) > cut {
                ui.painter().circle_stroke(
                    egui::pos2(x, y),
                    5.0,
                    Stroke::new(1.5_f32, theme::ORANGE),
                );
                // Off-screen spike: a marker at the top of the plot so it is still findable.
                if s.rtt_ms > scale_top {
                    ui.painter()
                        .circle_filled(egui::pos2(x, rect.top() + 4.0), 3.0, theme::ORANGE);
                }
            }
            prev = Some((x, y));
        }

        // Latest value marker: it marks the last measurement even after a stop, so that
        // "when did this target go quiet" stays readable. It takes the target's line colour,
        // like the trace itself.
        if let Some(s) = t.series.samples().last() {
            if s.t_secs >= t_min && s.t_secs < t_max && history::is_ok(s.rtt_ms) {
                let x = map_x(s.t_secs, t_min, t_max, rect);
                let y = map_y(s.rtt_ms.min(scale_top), scale_top, rect);
                ui.painter()
                    .circle_filled(egui::pos2(x, y), if sel { 3.4 } else { 2.4 }, line);
            }
        }
    }

    // Hover readout.
    if let Some(p) = resp.hover_pos() {
        if rect.contains(p) {
            app.hover_x = Some(map_t(p.x, t_min, t_max, rect));
            ui.painter().line_segment(
                [egui::pos2(p.x, rect.top()), egui::pos2(p.x, rect.bottom())],
                Stroke::new(1.0_f32, m.hover_line()),
            );
            let t_at = map_t(p.x, t_min, t_max, rect);
            // One line per target, and each label is painted in the target's own colour so the
            // readout can be matched to the traces without guessing.
            let head = format!("{} s", t_at.round() as i64);
            let mut rows: Vec<(String, Color32)> = Vec::new();
            for t in &app.targets {
                if !is_shown(app, t) {
                    continue;
                }
                if let Some(s) = nearest(t, t_at) {
                    // C4: the outlier threshold is computed exactly once per frame and target (see the
                    // cut before the curves) and is only read here.
                    let cut = app.cut_scratch.get(t.id).copied().unwrap_or(f64::MAX);
                    rows.push((
                        format!(
                            "{}  {}{}",
                            t.label,
                            if history::is_ok(s.rtt_ms) {
                                format!("{:.1}{}", s.rtt_ms, lang.ms_suffix())
                            } else {
                                lang.timeout_word().to_string()
                            },
                            if (s.rtt_ms as f64) > cut {
                                lang.marker_outlier()
                            } else {
                                ""
                            },
                        ),
                        app.trace_color(t),
                    ));
                }
            }
            let mut layout_job = egui::text::LayoutJob::default();
            layout_job.append(
                &head,
                0.0,
                egui::text::TextFormat::simple(FontId::monospace(11.0), m.text()),
            );
            for (line, color) in &rows {
                layout_job.append(
                    &format!("\n{line}"),
                    0.0,
                    egui::text::TextFormat::simple(FontId::monospace(11.0), *color),
                );
            }
            let gal = ui.painter().layout_job(layout_job);
            // The readout follows the cursor's row rather than sticking to the top: with many
            // targets the block gets tall, and at the top it would cover the very traces it is
            // reading values from. Near the bottom edge it flips above the cursor instead.
            let h = gal.size().y + 6.0;
            let below = p.y + 14.0 + h <= rect.bottom();
            let pos = egui::pos2(
                (p.x + 10.0).min(rect.right() - gal.size().x - 6.0),
                if below {
                    (p.y + 14.0).max(rect.top() + 6.0)
                } else {
                    (p.y - 14.0 - h).max(rect.top() + 6.0)
                },
            );
            ui.painter().rect_filled(
                egui::Rect::from_min_size(pos, gal.size() + Vec2::splat(6.0)),
                4.0,
                m.tooltip_fill(),
            );
            ui.painter().galley(pos + Vec2::splat(3.0), gal, m.text());
        }
    }

    // Event log under the chart. The band its labels were painted into is handed to the layout here,
    // so the log's own heading starts below them instead of under them.
    ui.add_space(AXIS_LABEL_H + LOG_GAP);
    let log_h = (ui.available_height() - LOG_HEAD_H).clamp(48.0, LOG_H);
    event_log(ui, app, log_h);
}

/// The outage/recovery log, in both places it appears: as a compact strip under the chart and as
/// the full list in its own view. One function so the two cannot drift apart in what they show.
///
/// `max_height` is the strip's height; pass [`f32::MAX`] to let the list fill whatever is left. That
/// choice also decides the scroll behaviour: a strip sticks to the newest entry (that is what a
/// status line is for), while the full view starts at the oldest and keeps the position of whoever
/// reads it — jumping to the bottom on every new event would make a long night unreadable.
///
/// Every row carries the wall clock rather than "+123 s": the chart counts from program start, so
/// two windows open at different times would disagree about when an outage happened. The same
/// conversion as the CSV export is used, which makes a log line and a CSV line the same fact.
fn event_log(ui: &mut Ui, app: &mut App, max_height: f32) {
    let lang = app.lang();
    let m = app.theme_mode;
    ui.horizontal(|ui| {
        ui.label(RichText::new(lang.events()).strong());
        // Everything is counted, only what belongs in the protocol is kept — otherwise one reads the
        // length of the list as "this much happened".
        if app.log_total > app.log.len() {
            ui.label(
                RichText::new(i18n::fill(
                    lang.log_count_fmt(),
                    &[
                        ("n", app.log.len().to_string()),
                        ("all", app.log_total.to_string()),
                    ],
                ))
                .weak(),
            );
        } else {
            ui.label(
                RichText::new(i18n::fill(
                    lang.log_count_paren_fmt(),
                    &[("n", app.log.len().to_string())],
                ))
                .weak(),
            );
        }
        // A7: after a long night one wants the incidents, not every message. Only the view is
        // filtered — otherwise "clear" would remain the only way to narrow things down.
        let only_bad = app.log_only_outages;
        let filt = ui
            .selectable_label(only_bad, selectable_text(lang.only_problems(), ""))
            .on_hover_text(lang.tip_only_problems())
            .clicked();
        if filt {
            app.log_only_outages = !only_bad;
        }
        if ui
            .small_button(lang.clear_log_btn())
            .on_hover_text(lang.tip_clear_log())
            .clicked()
        {
            app.log.clear();
            app.log_total = 0;
        }
    });
    // Rows that can be jumped to, remembered by their position in the log so the click can be
    // matched again after the list is drawn: `app` cannot be borrowed while it is being iterated.
    let mut jump: Option<f64> = None;
    ScrollArea::vertical()
        .max_height(max_height)
        .stick_to_bottom(max_height.is_finite())
        .show(ui, |ui| {
            for e in app.log.iter() {
                if app.log_only_outages && e.kind == LogKind::Info {
                    continue;
                }
                let color = m.log_kind(e.kind);
                let wall = app.wall_clock(e.secs);
                let row = ui.add(
                    Label::new(
                        RichText::new(format!("[{wall}] {}", e.text))
                            .color(color)
                            .monospace()
                            .size(11.5),
                    )
                    .selectable(false),
                );
                // A7: the message and its history in one place. Double-clicking an outage line pulls
                // the chart window onto that moment — otherwise one sits in the list, reads "outage
                // at 03:12" and has to find where that was first.
                if e.kind != LogKind::Info && row.double_clicked() {
                    jump = Some(e.secs);
                }
            }
        });
    if let Some(secs) = jump {
        app.jump_to_secs(secs);
    }
}

/// Column widths of the statistics table: Target, Host, Status, Probe, Min, Last, P95, Max,
/// Outages, Loss. The first two hold names and addresses, which are what actually gets long;
/// the numbers are fixed-width monospace, so they only need room for the header word.
fn column_widths() -> [f32; 10] {
    [200.0, 150.0, 84.0, 64.0, 64.0, 72.0, 64.0, 64.0, 72.0, 56.0]
}

fn is_shown(app: &App, t: &crate::app::Target) -> bool {
    match app.view {
        // The chart follows the filter row and the legend clicks; a hidden target leaves the plot
        // and with it its own contribution to the millisecond scale.
        View::Chart => app.shown(t),
        _ => app.targets.get(app.selected).map(|x| x.id) == Some(t.id),
    }
}

fn nearest(t: &crate::app::Target, t_secs: f64) -> Option<history::Sample> {
    let s = t.series.samples();
    s.iter()
        .min_by(|a, b| {
            ((a.t_secs - t_secs).abs())
                .partial_cmp(&(b.t_secs - t_secs).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

fn map_x(t: f64, t_min: f64, t_max: f64, r: egui::Rect) -> f32 {
    let u = (t - t_min) / (t_max - t_min).max(1e-6);
    r.left() + u as f32 * r.width()
}

fn map_t(x: f32, t_min: f64, t_max: f64, r: egui::Rect) -> f64 {
    let u = (x - r.left()) / r.width().max(1.0);
    t_min + u as f64 * (t_max - t_min)
}

fn map_y(ms: f32, y_max: f32, r: egui::Rect) -> f32 {
    let u = ms / y_max.max(1.0);
    r.bottom() - 2.0 - u * (r.height() - 4.0)
}

/// Room one label of the millisecond axis needs under its neighbour: its own twelve points plus the
/// two that lift it off its line.
const AXIS_BAND_H: f32 = 14.0;

/// The distances the millisecond axis may draw its lines at, as multiples of a decade: round
/// numbers, and the round ones people read latency in (15, 20, 30, 50, 150 …).
const MS_BAND_STEPS: [f32; 14] = [
    1.0, 2.0, 5.0, 10.0, 15.0, 20.0, 30.0, 50.0, 100.0, 150.0, 200.0, 300.0, 500.0, 1_000.0,
];

/// The spacing of the horizontal grid for a chart scaled to `scale_top` and `height` points tall:
/// the smallest round step of milliseconds that both keeps the labels one under the other and stays
/// few enough lines to count. Both get easier as the step grows, so the first that fits is the one.
/// `pub` for the tests that check a chart of hundreds of milliseconds and a very short one.
pub fn ms_band_step(scale_top: f32, height: f32) -> f32 {
    let scale_top = scale_top.max(1.0);
    let height = height.max(1.0);
    let fits = |s: f32| height * s / scale_top >= AXIS_BAND_H && scale_top / s <= 10.0;
    // The ladder, taken decade by decade: a chart scaled to tens of thousands of milliseconds needs
    // its lines a thousand milliseconds apart, and that number wants to be as round as the small ones.
    let mut decade = 1.0f32;
    for _ in 0..6 {
        for &s in &MS_BAND_STEPS {
            if fits(s * decade) {
                return s * decade;
            }
        }
        decade *= 10.0;
    }
    scale_top.max(1.0)
}

/// Distance between two vertical grid lines for a chart window. `pub` so a test can check that
/// a day does not get the minute spacing of a short window.
pub fn nice_step(horizon: f64) -> f64 {
    // Hours and days are the part that matters for a wide window: without them a day would
    // fall back to the 900 s default and print sixty grid lines on top of each other.
    for s in NICE_STEPS {
        if horizon / s <= 12.0 {
            return s;
        }
    }
    *NICE_STEPS.last().unwrap_or(&86_400.0)
}

/// The step the time axis is drawn with: [`nice_step`]'s, raised along the same ladder until the
/// labels fit the plot. Twelve lines are what a wide window can take, but the same twelve set their
/// labels on top of one another as soon as the plot is narrow, so a narrow window counts in longer
/// strides. `pub` for the test that checks a day in a narrow window.
pub fn step_for_width(horizon: f64, span_secs: f64, width: f32) -> f64 {
    let first = NICE_STEPS
        .iter()
        .position(|s| *s >= nice_step(horizon))
        .unwrap_or(0);
    let mut step = NICE_STEPS[first];
    for &s in &NICE_STEPS[first..] {
        step = s;
        if (span_secs / s) as f32 * AXIS_LABEL_SLOT <= width {
            break;
        }
    }
    step
}

/// A tick label on the time axis. Short windows count in minutes and seconds; from about an
/// hour upwards that would run past 99 minutes, so wider windows show the wall clock instead —
/// which is also what makes a day readable. `clock` says whether a real time of day is known.
/// `pub` for the tests that check what a day's axis prints.
pub fn fmt_axis(secs: f64, horizon: f64, clock: Option<i64>) -> String {
    if horizon < 2.0 * 3_600.0 {
        let (sign, s) = split_secs(secs);
        format!("{sign}{:02}:{:02}", s / 60, s % 60)
    } else if let Some(at_ms) = clock {
        use chrono::{Local, TimeZone};
        match Local.timestamp_millis_opt(at_ms).single() {
            Some(t) => t.format("%H:%M").to_string(),
            None => String::from("?"),
        }
    } else {
        // No absolute stamp to show (a session that has not measured yet): count in hours.
        let (sign, s) = split_secs(secs);
        format!("{sign}{:02}:{:02} h", s / 3_600, (s % 3_600) / 60)
    }
}

/// Whole seconds of an axis position, and the sign in front of them. An axis position is counted
/// from the start of the session, so the left part of a window that is wider than the session has
/// been running lies *before* zero — and clamping that to zero made every one of those ticks read
/// „00:00“, which looked like a clock that had stopped rather than time before the app started.
fn split_secs(secs: f64) -> (&'static str, i64) {
    let s = secs as i64;
    if s < 0 {
        ("-", -s)
    } else {
        ("", s)
    }
}

/* ------------------------------------------------------------------- list */

fn events_view(ui: &mut Ui, app: &mut App) {
    let lang = app.lang();
    let m = app.theme_mode;
    ui.label(RichText::new(lang.status_list()).strong());
    ui.add_space(4.0);
    let now = app.engine.elapsed_secs();
    // Taken out of the app so it can be reused while `app.targets` is borrowed.
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    // B3: only what has already been drawn can be clicked — so remember the id and act after the
    // loop unwinds.
    let mut unhide_id: Option<usize> = None;
    // B5: here too, "no rows" is an answer that wants explaining.
    if app.targets.iter().filter(|t| app.shown(t)).count() == 0 {
        ui.label(RichText::new(weak_note(app)));
    }
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for idx in 0..app.targets.len() {
                // A1/B2: what the chart does not show is not missing here either — it is written more
                // quietly and stays as the proof that it was hidden.
                let shown = app.shown(&app.targets[idx]);
                if !shown && !app.targets[idx].hidden {
                    continue;
                }
                let color = app.status_color(&app.targets[idx]);
                let trace = app.trace_color(&app.targets[idx]);
                let txt = app.status_text(&app.targets[idx]);
                let st = app.stats_for(&app.targets[idx], &mut scratch);
                let t = &app.targets[idx];
                let sel = app.selected == idx;
                let secs_ago = if t.last_probe_secs > 0.0 {
                    now - t.last_probe_secs
                } else {
                    -1.0
                };

                let frame = egui::Frame::new()
                    .fill(m.row_fill(sel))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 8));
                let r = frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // B3: a hidden target can be brought back here just as well as through the
                        // legend — the up-arrow dot next to the name does it.
                        if app.targets[idx].hidden
                            && ui
                                .small_button(RichText::new("◉").color(m.text_dim()))
                                .on_hover_text(lang.tip_show_target())
                                .clicked()
                        {
                            unhide_id = Some(app.targets[idx].id);
                        }
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 6.0, color);
                        // A dash in the line colour, like in the target list on the left.
                        let (line_rect, _) =
                            ui.allocate_exact_size(Vec2::new(18.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(line_rect.shrink(5.0), 2.0, trace);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            // B2: same name, same place — just much quieter than everything being
                            // measured right now. The reason sits next to it, not in a tooltip.
                            let mut name = format!("{}   ({})", t.label, t.host);
                            if !shown {
                                name.push_str(if t.hidden {
                                    lang.hidden_suffix()
                                } else {
                                    lang.filtered_suffix()
                                });
                            }
                            ui.add(
                                Label::new(RichText::new(name).strong().color(if shown {
                                    m.text()
                                } else {
                                    m.text_dim()
                                }))
                                .selectable(false),
                            );
                            let age = if secs_ago < 0.0 {
                                lang.waiting_short().to_string()
                            } else {
                                i18n::fill(
                                    lang.ago_fmt(),
                                    &[("n", secs_ago.max(0.0).round().to_string())],
                                )
                            };
                            ui.add(
                                Label::new(
                                    RichText::new(format!(
                                        "{} · {} · {}",
                                        lang.kind(t.kind),
                                        age,
                                        crate::app::fmt_ms(t.last_rtt)
                                    ))
                                    .weak()
                                    .size(11.5),
                                )
                                .selectable(false),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(
                                Label::new(RichText::new(&txt).color(color).monospace().strong())
                                    .selectable(false),
                            );
                        });
                    });
                    ui.add_space(4.0);
                    let w = ui.available_width();
                    let (bar, _) = ui.allocate_exact_size(Vec2::new(w, 16.0), egui::Sense::hover());
                    draw_sparkline(ui, &t.series, bar, trace);
                    ui.add(
                        Label::new(
                            RichText::new({
                                let mut line = i18n::fill(
                                    lang.stats_line_fmt(),
                                    &[
                                        ("min", crate::app::fmt_ms(st.min)),
                                        ("avg", crate::app::fmt_ms(st.avg as f32)),
                                        ("p95", crate::app::fmt_ms(st.p95)),
                                        ("max", crate::app::fmt_ms(st.max)),
                                        ("loss", format!("{:.1}", st.loss_pct)),
                                    ],
                                );
                                if st.outliers > 0 {
                                    line.push_str(&i18n::fill(
                                        lang.outliers_suffix_fmt(),
                                        &[("n", st.outliers.to_string())],
                                    ));
                                }
                                line
                            })
                            .weak()
                            .size(11.0),
                        )
                        .selectable(false),
                    );
                });
                if r.response.clicked() {
                    app.selected = idx;
                }
            }
        });
    app.stats_scratch = scratch;
    if let Some(id) = unhide_id {
        app.toggle_hidden_by_id(id);
    }
}

/// Why nothing is shown, in one sentence — the same three causes in every view that can go empty.
fn weak_note(app: &App) -> String {
    let lang = app.lang();
    if !app.all_kinds_on() {
        lang.all_kinds_off().to_string()
    } else if app.targets.iter().all(|t| t.hidden) {
        lang.all_targets_hidden().to_string()
    } else {
        lang.filter_no_match().to_string()
    }
}

fn draw_sparkline(ui: &Ui, series: &history::Series, rect: egui::Rect, color: Color32) {
    let m = Mode::from_dark(ui.visuals().dark_mode);
    let samples = series.samples();
    if samples.is_empty() {
        return;
    }
    let t0 = samples[0].t_secs;
    let t1 = samples.last().map(|s| s.t_secs).unwrap_or(t0 + 1.0);
    // C2: a sparkline is 16 px tall and only says "calm" or "is something moving here". A single
    // 900 ms outlier squashes everything else into a flat line — so the axis ends at the series' p95,
    // but never below 40 ms. What lies above it stays visible as a ring.
    let (cut, _n) = history::outlier_cut(samples);
    let mut y_max = 40.0f32;
    for s in samples {
        if history::is_ok(s.rtt_ms) && (s.rtt_ms as f64) <= cut {
            y_max = y_max.max(s.rtt_ms);
        }
    }
    let painter = ui.painter();

    let mut prev: Option<(f32, f32)> = None;
    for s in samples {
        let u = (s.t_secs - t0) / (t1 - t0).max(1e-6);
        let x = rect.left() + u as f32 * rect.width();
        if !history::is_ok(s.rtt_ms) {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x - 1.0, rect.bottom() - 4.0),
                    egui::pos2(x + 1.0, rect.bottom()),
                ),
                0.0,
                m.loss_fill(),
            );
            prev = None;
            continue;
        }
        let y = rect.bottom() - (s.rtt_ms / y_max) * (rect.height() - 4.0);
        if let Some((px, py)) = prev {
            painter.line_segment(
                [egui::pos2(px, py), egui::pos2(x, y)],
                Stroke::new(1.3_f32, color),
            );
        }
        // Mark outliers with a ring so their position on the time axis is visible.
        if (s.rtt_ms as f64) > cut {
            painter.circle_stroke(egui::pos2(x, y), 4.5, Stroke::new(1.5_f32, m.warn()));
        }
        prev = Some((x, y));
    }
}

/* ------------------------------------------------------------------ stats */

/// Which statistics column a header click sorts by. `pub` and named after the table's own columns so
/// the header, the arrow and [`App::sort_targets`] cannot disagree about what "p95" means.
fn sort_column(n: usize) -> Option<app::SortColumn> {
    Some(match n {
        0 => app::SortColumn::Label,
        1 => app::SortColumn::Host,
        3 => app::SortColumn::Min,
        4 => app::SortColumn::Avg,
        5 => app::SortColumn::Median,
        6 => app::SortColumn::P95,
        7 => app::SortColumn::Max,
        8 => app::SortColumn::Loss,
        9 => app::SortColumn::Outliers,
        _ => return None,
    })
}

fn stats_view(ui: &mut Ui, app: &mut App) {
    let lang = app.lang();
    let m = app.theme_mode;
    ui.label(RichText::new(lang.stats_heading()).strong());
    ui.add_space(6.0);

    // The targets to list are looked up once, so the row count and the row lookup agree. Hidden
    // targets are left out here: unlike the legend, a statistics table has no way to switch them
    // back on.
    let ids: Vec<usize> = app.shown_target_ids();
    // Taken out of the app so it can be reused while `app.targets` is borrowed.
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    let mut select: Option<usize> = None;

    // Plain grid instead of egui_extras' table: the look stays the same (header row,
    // one row per target) without pulling in another crate.
    //
    // B4/C6: every column header is a button. The first click sorts descending — "the worst
    // first", because that is what one looks for in a latency table — and the second reverses it.
    // The arrow is painted next to the word rather than replacing it, so an unsorted table still
    // says what each column would do.
    let mut sort_click: Option<(app::SortColumn, bool)> = None;
    let widths = column_widths();
    ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let headers = [
                lang.col_target(),
                lang.col_host(),
                lang.col_status(),
                lang.col_min(),
                lang.col_avg(),
                lang.col_median(),
                lang.col_p95(),
                lang.col_max(),
                lang.col_loss(),
                lang.col_outliers(),
            ];
            for (n, c) in headers.iter().enumerate() {
                let col = sort_column(n);
                let arrow = match (&app.sort, col) {
                    (Some(s), Some(col)) if s.column == col => {
                        if s.descending {
                            " ↓"
                        } else {
                            " ↑"
                        }
                    }
                    _ => "",
                };
                let btn = ui.add_sized(
                    [widths[n], 20.0],
                    Label::new(RichText::new(format!("{c}{arrow}")).strong().size(12.0))
                        .selectable(false),
                );
                if let Some(col) = col {
                    btn.clone()
                        .on_hover_text(i18n::fill(lang.tip_sort_fmt(), &[("c", (*c).to_string())]));
                    if btn.clicked() {
                        let descending = match &app.sort {
                            Some(s) if s.column == col => !s.descending,
                            _ => true,
                        };
                        sort_click = Some((col, descending));
                    }
                }
            }
        });
        ui.separator();
        if ids.is_empty() {
            // B5: a table of nothing says why.
            ui.label(RichText::new(weak_note(app)).size(12.5));
        }
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for &idx in &ids {
                    let t = &app.targets[idx];
                    // C3: the same computation as in the list on the left and in the totals row below.
                    let st = app.stats_for(t, &mut scratch);
                    let color = app.status_color(t);
                    let trace = app.trace_color(t);
                    let txt = app.status_text(t);
                    let sel = app.selected == idx;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        // Colour dash in front of the name: the same colour as the line in the chart.
                        let (line_rect, _) =
                            ui.allocate_exact_size(Vec2::new(14.0, 20.0), egui::Sense::hover());
                        ui.painter().rect_filled(line_rect.shrink(6.0), 1.5, trace);
                        if ui
                            .add_sized(
                                [widths[0], 20.0],
                                Label::new(RichText::new(&t.label).strong()).selectable(sel),
                            )
                            .clicked()
                        {
                            select = Some(idx);
                        }
                        ui.add_sized(
                            [widths[1], 20.0],
                            Label::new(RichText::new(&t.host).monospace().size(12.0))
                                .selectable(false),
                        );
                        ui.add_sized(
                            [widths[2], 20.0],
                            Label::new(RichText::new(&txt).color(color).size(12.0))
                                .selectable(false),
                        );
                        for (i, v) in [st.min, st.avg as f32, st.median, st.p95, st.max]
                            .iter()
                            .enumerate()
                        {
                            ui.add_sized(
                                [widths[3 + i], 20.0],
                                Label::new(
                                    RichText::new(crate::app::fmt_ms(*v)).monospace().size(12.0),
                                )
                                .selectable(false),
                            );
                        }
                        let cl = m.loss_text(st.loss_pct > 0.5);
                        ui.add_sized(
                            [widths[8], 20.0],
                            Label::new(
                                RichText::new(format!("{:.1} %", st.loss_pct))
                                    .color(cl)
                                    .monospace()
                                    .size(12.0),
                            )
                            .selectable(false),
                        );
                        // Outliers are counted over the whole stored series, so this number
                        // does not change when only part of it is on screen.
                        let c = m.warn_count(st.outliers > 0);
                        ui.add_sized(
                            [widths[9], 20.0],
                            Label::new(
                                RichText::new(st.outliers.to_string())
                                    .color(c)
                                    .monospace()
                                    .size(12.0),
                            )
                            .selectable(false),
                        );
                    });
                }
            });
    });
    app.stats_scratch = scratch;

    if let Some(idx) = select {
        app.selected = idx;
    }
    // B4: the sort happens after the table is drawn — `sort_targets` needs `app` mutably and reorders
    // the very list being iterated. The header arrow updates on the next pass, which is what a click
    // on a column reads as anyway.
    if let Some((column, descending)) = sort_click {
        app.sort = Some(app::SortState { column, descending });
        app.sort_targets(column, descending);
    }

    ui.add_space(10.0);
    ui.separator();
    // D4: "Total" is not a sum of the rows but the same computation as per target — just over every
    // reply in the selection. Averaging the median and p95 of two targets would give a latency no
    // target ever measured, so `stats_total` collects all replies into one buffer and draws the
    // percentiles from it.
    let mut all = std::mem::take(&mut app.all_rtt_scratch);
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    let st = app.stats_total(&mut scratch, &mut all);
    app.stats_scratch = scratch;
    app.all_rtt_scratch = all;
    // A column without a measurement shows "—" and not 0: a target that never answered has no
    // latency, and a zero in a latency column would be a claim that is not true.
    let num = |v: f32| {
        if v.is_finite() && st.replies > 0 {
            crate::app::fmt_ms(v)
        } else {
            lang.dash().to_string()
        }
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let widths = column_widths();
        ui.add_sized(
            [widths[0] + widths[1] + widths[2], 20.0],
            Label::new(
                RichText::new(i18n::fill(
                    lang.total_probes_fmt(),
                    &[("n", st.total.to_string())],
                ))
                .strong()
                .size(12.0),
            )
            .selectable(false),
        );
        for (n, v) in [
            num(st.min),
            if st.replies > 0 {
                crate::app::fmt_ms(st.avg as f32)
            } else {
                lang.dash().to_string()
            },
            num(st.median),
            num(st.p95),
            num(st.max),
        ]
        .into_iter()
        .enumerate()
        {
            ui.add_sized(
                [widths[3 + n], 20.0],
                Label::new(RichText::new(v).monospace().size(12.0)).selectable(false),
            );
        }
        ui.add_sized(
            [widths[8], 20.0],
            Label::new(
                RichText::new(format!("{:.1} %", st.loss_pct))
                    .color(m.loss_text(st.loss_pct > 0.5))
                    .monospace()
                    .size(12.0),
            )
            .selectable(false),
        );
        ui.add_sized(
            [widths[9], 20.0],
            Label::new(
                RichText::new(st.outliers.to_string())
                    .color(m.warn_count(st.outliers > 0))
                    .monospace()
                    .size(12.0),
            )
            .selectable(false),
        );
    });
}

/* --------------------------------------------------------------- settings */

/// The preset window closest to a requested size. Used by the chart's wheel zoom: the ladder from
/// [`app::HORIZON_CHOICES`] keeps the number recognisable and matches what the combo box in the
/// top bar can show, instead of inventing windows no control could name.
fn pick_horizon(secs: f64) -> f32 {
    let want = secs as f32;
    app::HORIZON_CHOICES
        .iter()
        .min_by_key(|c| ((c.secs - want).abs() * 100.0) as i64)
        .map(|c| c.secs)
        .unwrap_or(60.0)
}

/// One preset window, translated: the ladder itself stays language-independent because it is stored
/// in seconds and matched by value, only what the control says about it follows `lang`.
fn horizon_choice_label(lang: Lang, c: &app::HorizonChoice) -> String {
    let secs = c.secs;
    if (secs - secs.round()).abs() < f32::EPSILON {
        let n = secs as i64;
        if n % 3_600 == 0 {
            return format!("{} {}", n / 3_600, lang.hours_unit());
        }
        if n >= 60 {
            return format!("{} {}", n / 60, lang.minutes_unit());
        }
        return format!("{n} {}", lang.seconds_unit());
    }
    format!("{secs:.1} {}", lang.seconds_unit())
}

/// The label of the current window length, and a fallback for a value from an older file that
/// is not one of the presets.
fn horizon_label(lang: Lang, secs: f32) -> String {
    app::HORIZON_CHOICES
        .iter()
        .find(|c| c.secs == secs)
        .map(|c| horizon_choice_label(lang, c))
        .unwrap_or_else(|| format!("{secs:.0} {}", lang.seconds_unit()))
}

/// What a window length means in practice: how many values it holds and how much memory that
/// is per target. A day is the first choice where the number is worth showing.
fn horizon_hint(lang: Lang, secs: f32) -> String {
    let samples = app::samples_for_horizon(secs, 1.0);
    if secs < 3_600.0 {
        lang.horizon_hint_chart().to_string()
    } else {
        let mib =
            samples as f64 * std::mem::size_of::<history::Sample>() as f64 / (1024.0 * 1024.0);
        i18n::fill(
            lang.samples_capacity_fmt(),
            &[
                ("n", samples.to_string()),
                ("mib", format!("{mib:.1}")),
                ("interval", app::interval_label(lang, 1.0)),
            ],
        )
    }
}

fn settings(ui: &mut Ui, app: &mut App) {
    let lang = app.lang();
    let mut dirty = false;
    let m = app.theme_mode;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.group(|ui| {
                ui.label(RichText::new(lang.measuring()).strong());
                ui.add_space(4.0);
                // The interval has its slider up in the top bar and is saved from there; a second slider
                // for the same number was just a second chance to wonder where it was changed.
                ui.label(
                    RichText::new(i18n::fill(
                        lang.interval_now_fmt(),
                        &[("n", app::interval_label(lang, app.options.interval_secs))],
                    ))
                    .weak(),
                );
                ui.horizontal(|ui| {
                    ui.label(lang.timeout_label());
                    if ui
                        .add(
                            egui::DragValue::new(&mut app.options.timeout_ms)
                                .range(200..=10_000)
                                .suffix(" ms")
                                .speed(50),
                        )
                        .changed()
                    {
                        dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(lang.concurrency_label());
                    if ui
                        .add(egui::DragValue::new(&mut app.options.concurrency).range(1..=32))
                        .changed()
                    {
                        dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(lang.horizon_label_text());
                    // The presets reach up to 24 h. A slider would be wrong here: between 15 s and a day,
                    // the difference of a few minutes is not visible.
                    egui::ComboBox::from_id_salt("horizon")
                        .selected_text(horizon_label(lang, app.options.horizon_secs))
                        .show_ui(ui, |ui| {
                            for c in app::HORIZON_CHOICES {
                                if ui
                                    .selectable_value(
                                        &mut app.options.horizon_secs,
                                        c.secs,
                                        horizon_choice_label(lang, c),
                                    )
                                    .on_hover_text(horizon_hint(lang, c.secs))
                                    .changed()
                                {
                                    dirty = true;
                                }
                            }
                        });
                    ui.label(
                        RichText::new(horizon_hint(lang, app.options.horizon_secs))
                            .weak()
                            .size(11.5),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(lang.max_samples_label());
                    // A day at one-second spacing is 86,400 values per target. The cap therefore follows
                    // the time window rather than a fixed maximum.
                    let cap = app::effective_max_samples(&app.options);
                    if ui
                        .add(
                            egui::DragValue::new(&mut app.options.max_samples)
                                .range(0..=cap)
                                .speed(256)
                                .custom_formatter(|v, _| {
                                    if v == 0.0 {
                                        String::from("auto")
                                    } else {
                                        format!("{}", v as usize)
                                    }
                                }),
                        )
                        .on_hover_text(i18n::fill(
                            lang.tip_max_samples_fmt(),
                            &[
                                (
                                    "n",
                                    app::samples_for_horizon(
                                        app.options.horizon_secs,
                                        app.options.interval_secs,
                                    )
                                    .to_string(),
                                ),
                                (
                                    "interval",
                                    app::interval_label(lang, app.options.interval_secs),
                                ),
                            ],
                        ))
                        .changed()
                    {
                        dirty = true;
                    }
                });
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(RichText::new(lang.start_group()).strong());
                ui.add_space(4.0);
                // A3: sound, tray icon and starting minimised were shown as buttons although the program
                // does none of those things. The fields stay in the file for whoever implements them; a
                // button that does nothing is worse than no button.
                ui.horizontal(|ui| {
                    let supported = crate::platform::autostart::is_supported();
                    let on = app.autostart_state() == Some(true);
                    let mut v = on;
                    // Where the OS has no autostart mechanism we can actually reach, a box that
                    // cannot be ticked says more than one that lies about having done something.
                    ui.add_enabled_ui(supported, |ui| {
                        if ui.checkbox(&mut v, lang.autostart_checkbox()).changed() {
                            app.set_autostart(v);
                        }
                        if supported && app.autostart_state() == Some(false) {
                            ui.label(
                                RichText::new(lang.autostart_blocked())
                                    .color(app.theme_mode.warn()),
                            );
                        }
                    });
                    if !supported {
                        ui.label(RichText::new(lang.autostart_unsupported()));
                    }
                });
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                // The switch sits in the window itself rather than in a settings file: someone who has
                // opened the program and cannot read it is exactly the person who cannot find an option
                // named after a language they do not speak. Each name is written in its own language, so
                // both stay recognisable whichever one is currently showing.
                ui.label(RichText::new(lang.language()).strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for l in Lang::ALL {
                        if ui
                            .selectable_label(app.options.language == l, l.native_name())
                            .on_hover_text(i18n::fill(
                                lang.tip_language_fmt(),
                                &[("name", l.native_name().to_string())],
                            ))
                            .clicked()
                        {
                            app::set_language(app, l);
                        }
                    }
                });
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(RichText::new(lang.appearance_group()).strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(lang.theme_scheme_label());
                    for (value, label) in [
                        ("system", lang.system_theme()),
                        ("light", lang.light()),
                        ("dark", lang.dark()),
                    ] {
                        let on = app.options.theme == value;
                        if ui.selectable_label(on, label).clicked() {
                            app.options.theme = String::from(value);
                            dirty = true;
                        }
                    }
                });
                ui.label(RichText::new(lang.tip_system_theme()).weak().size(11.5));
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                // There used to be a "Target" group here with "remove target" and "reset probe". Removal
                // now happens at the target itself (the inline editor), and a button that hit whichever
                // target was selected was far too easy to misunderstand.
                ui.label(RichText::new(lang.data_group()).strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(lang.export_csv())
                        .on_hover_text(lang.tip_export_default())
                        .clicked()
                    {
                        let path = default_export_path();
                        app.export_csv(path);
                    }
                    if ui
                        .button(lang.save_as())
                        .on_hover_text(lang.tip_save_as())
                        .clicked()
                    {
                        app.open_export_dialog();
                    }
                    // A2: no "save" button — everything is written on every change anyway, so what such a
                    // button promises would not be true. The status bar says when now.
                    // This affects all targets — which is why the button says so as well. An open editor
                    // is committed first, otherwise the values vanish under the open inputs.
                    // Because the history cannot be undone, the button wants two clicks: the first only
                    // arms it and asks, the second deletes. After a few seconds the asking lapses again,
                    // otherwise a forgotten warning would delete something.
                    let clear_label = if app.clear_armed {
                        lang.clear_measurements_q()
                    } else {
                        lang.clear_measurements()
                    };
                    let clear_btn =
                        ui.button(RichText::new(clear_label).color(if app.clear_armed {
                            m.danger_text()
                        } else {
                            Color32::from_rgb(150, 150, 150)
                        }));
                    // `on_hover_text` consumes the response, so both arms of the choice have to be
                    // written as one expression that hands the (unchanged) response back.
                    let clear_btn = if app.clear_armed {
                        clear_btn.on_hover_text(lang.tip_clear_again())
                    } else {
                        clear_btn.on_hover_text(lang.tip_clear_measurements())
                    };
                    if clear_btn.clicked() {
                        if app.clear_armed {
                            // Second click within the window: do it.
                            app.clear_armed = false;
                            app.clear_at = None;
                            app.close_edit();
                            app.reset_stats();
                        } else {
                            app.clear_armed = true;
                            app.clear_at = Some(std::time::Instant::now());
                        }
                    }
                    ui.label(RichText::new(lang.autosave_note()).weak().size(11.5));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut app.options.protocol_log, lang.protocol_label())
                        .on_hover_text(lang.tip_protocol_log())
                        .changed()
                    {
                        let format =
                            crate::exporter::ProtocolFormat::from_path(&app.options.protocol_path);
                        app.set_protocol(app.options.protocol_log, format);
                        dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(app.options.protocol_log, |ui| {
                        let mut path = app.options.protocol_path.clone();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut path)
                                    .hint_text(lang.hint_protocol_path())
                                    .desired_width(300.0),
                            )
                            .changed()
                        {
                            app.set_protocol(
                                true,
                                crate::exporter::ProtocolFormat::from_path(&path),
                            );
                            app.options.protocol_path = path;
                            dirty = true;
                        }
                    });
                });
                ui.add_space(4.0);
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                // B1/C5: shortcuts that cannot be read anywhere do not exist. What is written here is the
                // same list that sits in the bar under the buttons.
                ui.label(RichText::new(lang.keys_heading()).strong());
                ui.add_space(4.0);
                for (keys, what) in [
                    ("1 – 4", lang.key_views()),
                    (lang.key_space_label(), lang.key_space()),
                    ("Esc", lang.key_esc()),
                    ("← / →", lang.key_arrow()),
                    (lang.key_wheel_label(), lang.key_wheel()),
                    (lang.key_dblclick_chart_label(), lang.key_dblclick()),
                    (lang.key_dblclick_list_label(), lang.key_list_dblclick()),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [180.0, 18.0],
                            Label::new(RichText::new(keys).monospace().size(12.0))
                                .selectable(false),
                        );
                        ui.label(RichText::new(what).size(12.0));
                    });
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(RichText::new(lang.info_group()).strong());
                ui.label(
                    RichText::new(format!(
                        "{} {}\n{}",
                        lang.app_title(),
                        crate::app::about_version(),
                        lang.about_body()
                    ))
                    .weak()
                    .size(11.5),
                );
            });
        });

    if dirty {
        app.sync_caps();
        app.sync_engine();
        // Save on the next tick.
        app.save_now();
    }
}

/// Where "CSV exportieren" writes without asking: the state folder, so the file lands next to the
/// settings instead of wherever the shortcut happened to start the program from.
pub fn default_export_path() -> PathBuf {
    store::state_dir().join(default_export_name())
}

/// The suggested file name — a timestamped name in the root of the target folder. Shared by the
/// automatic export and the pre-filled name of the "save as…" dialog.
pub fn default_export_name() -> String {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("system-ping-{stamp}.csv")
}
