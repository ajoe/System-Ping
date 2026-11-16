//! The whole interface: top bar, target list, the four views, event log, status bar.

use crate::app::{self, App, LogKind, TargetKind, View};
use crate::history;
use crate::i18n::{self, Lang};
use crate::store;
use crate::theme::{self, Mode};
use egui::{Color32, CursorIcon, FontId, Label, Rect, RichText, ScrollArea, Stroke, Ui, Vec2};
use std::path::PathBuf;

pub fn draw(ctx: &egui::Context, app: &mut App) {
    top(ctx, app);
    left(ctx, app);
    bottom(ctx, app);
    center(ctx, app);
}

/// The same for the two target fields of the settings "Targets" group: their own salts, because the
/// address field would otherwise share its state with the one in the top bar — focus included.
const SETTINGS_HOST_ID_SALT: &str = "settings_host_field";
const SETTINGS_LABEL_ID_SALT: &str = "settings_label_field";

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
pub fn top(ctx: &egui::Context, app: &mut App) {
    let lang = app.lang();
    let m = app.theme_mode;
    // Set when something worth persisting changed; the state is written once at the end of the
    // panel so a click on a category chip cannot be lost until the next autosave.
    let mut save_now = false;
    egui::TopBottomPanel::top("title_bar")
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.window_fill)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ctx, |ui| {
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
                    .add(egui::SelectableLabel::new(
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
            // each other at the minimum window width.
            ui.horizontal(|ui| {
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
                if !app.filter.is_empty() && ui.button("✕").clicked() {
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
    short_cuts(ctx, app);
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
pub fn left(ctx: &egui::Context, app: &mut App) {
    egui::SidePanel::left("targets")
        .exact_width(250.0)
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            left_body(ui, app);
            // A colour chosen in the editor is a per-target setting, so it is not covered by the
            // autosave that watches `AppOptions`. It still has to survive a restart.
            if std::mem::take(&mut app.color_dirty) {
                app.save_now();
            }
        });
}

/// Which of the rows built by [`left_body`] a plain left click belongs to, if any.
///
/// A target row cannot use the ordinary `response.clicked()`: egui's `Frame::show` registers its
/// area with `Sense::hover()` only (see `tests/frame_click.rs`), so a row never reports a click of
/// its own. The decision therefore uses the rects the rows reported in the previous pass together
/// with egui's own pointer state — the same two ingredients `clicked_elsewhere()` is built from,
/// just without the per-area response that a hover-only frame never delivers.
///
/// Both the press and the release have to lie inside the row. Reading only the release position
/// would be wrong: when a click opens an editor, the list relayouts and the very next pass already
/// reports different rects, so the pointer of the click that opened it would be judged against
/// geometry that did not exist when the mouse was pressed.
///
/// `rects` are the row click areas from `App::row_rects`, so this only ever answers after a first
/// layout pass — which is also true of a real click, since egui has to have drawn the rows before
/// anyone can aim at them. A click on a control outside the list — a button, a combo box — lands in
/// no row and returns `None`, which keeps those controls working normally.
pub fn clicked_in_row(ctx: &egui::Context, rects: &[Rect]) -> Option<usize> {
    // Only a finished primary press+release counts; a drag of the panel border must not edit.
    let pointer = ctx.input(|i| {
        if i.pointer.any_click() {
            i.pointer.interact_pos()
        } else {
            None
        }
    })?;
    rects.iter().position(|rect| rect.contains(pointer))
}

/// The panel's contents on their own. Tests call this directly so they can place the panel
/// somewhere they control and click a row with a synthetic pointer.
/// Height of a row in the target list, in ui points: two lines of text plus the frame's padding.
/// The click area of a row is this tall, so it always covers exactly what the row shows — and an
/// open editor keeps the same strip for its neighbour clicks (see `App::row_heights`).
const ROW_H: f32 = 39.0;

pub fn left_body(ui: &mut Ui, app: &mut App) {
    let lang = app.lang();
    // Which row the current click belongs to, decided once before the rows are built. The rects
    // come from the previous pass; a real click always follows a pass that drew the rows.
    let clicked_row = clicked_in_row(ui.ctx(), &app.row_rects);
    // A click inside one of the editor's own controls (a text field, the combo box, a button) must
    // not be mistaken for "this row was clicked again", so that case is ignored here. The editor
    // only closes through its buttons, Enter/Esc, or a click on another row.
    let clicked_row = match clicked_row {
        Some(row) if app.edit_idx == Some(row) => None,
        other => other,
    };
    let m = app.theme_mode;
    ui.horizontal(|ui| {
        ui.label(RichText::new(lang.targets()).strong());
        let vis = app.visible_targets().len();
        ui.label(RichText::new(format!("{}/{}", vis, app.targets.len())).weak());
    });
    ui.separator();

    let now = app.engine.elapsed_secs();
    // Reused scratch buffer for the per-target statistics shown in the editor.
    let mut scratch = std::mem::take(&mut app.stats_scratch);

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Collect actions first to avoid borrow conflicts.
            let mut edit_action: Option<fn(&mut App)> = None;
            let mut begin_edit_idx: Option<usize> = None;
            // Set by the "entfernen" button inside an open editor: which target to delete once the
            // list is no longer borrowed.
            let mut remove_idx: Option<usize> = None;
            // A colour change has to be written back to disk right away, like every other setting.
            let mut color_changed = false;
            // Filled with every row's click area while the rows are built, then handed to
            // `app.row_rects` below. A test needs those rects to aim a synthetic click.
            let mut row_rects = Vec::new();
            let mut row_heights = Vec::new();

            for idx in 0..app.targets.len() {
                // Everything the row and the editor need from this target is read out here, so the
                // rest of the body never has to borrow `app.targets` again. The editor below needs
                // `app` mutably (its fields are the form's state), which a live borrow would forbid.
                let t = &app.targets[idx];
                let color = app.status_color(t);
                // The line colour of this target, shown next to the status dot so the row already
                // says which trace in the chart belongs to it.
                let trace = app.trace_color(t);
                let txt = app.status_text(t);
                let sel = app.selected == idx;
                let editing_here = app.edit_idx == Some(idx);
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

                // While a row is being edited, show its settings instead of the label. The editor
                // is deliberately *not* one of `row_rects`: it is a form with real controls, and a
                // click on one of them must never be read as "this row was clicked again".
                if editing_here {
                    // Keep the row's click area alive while it is being edited. It has to cover
                    // exactly the same spot as before, otherwise the list would shift and a click
                    // aimed at a neighbour would land somewhere else. `clicked_in_row` ignores a
                    // click on the row that is already open, so the editor's own controls are safe.
                    let h = app.row_heights.get(idx).copied().unwrap_or(39.0);
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), h),
                        egui::Sense::hover(),
                    );
                    row_rects.push(rect);

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
                                .small_button(
                                    RichText::new(lang.remove_short()).color(m.danger_text()),
                                )
                                .on_hover_text(i18n::fill(
                                    lang.tip_remove_fmt(),
                                    &[("label", label.clone())],
                                ))
                                .clicked()
                            {
                                remove_idx = Some(idx);
                            }
                            ui.label(RichText::new(lang.edit_heading()).strong().size(12.0));

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("✕")
                                        .on_hover_text(lang.tip_discard_edit())
                                        .clicked()
                                    {
                                        edit_action = Some(App::cancel_edit);
                                    }
                                },
                            );
                        });

                        // Name and address.
                        ui.add_sized(
                            [215.0, 20.0],
                            egui::TextEdit::singleline(&mut app.edit_label)
                                .hint_text(lang.hint_new_label())
                                .vertical_align(egui::Align::Center),
                        );
                        ui.add_sized(
                            [215.0, 20.0],
                            egui::TextEdit::singleline(&mut app.edit_host)
                                .hint_text(lang.hint_address())
                                .font(FontId::monospace(12.5))
                                .vertical_align(egui::Align::Center),
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

                let mut frame = egui::Frame::new()
                    .fill(m.row_fill(sel))
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(7, 5));
                // A row is a button. `Frame::show` only ever registers `Sense::hover()`, so the
                // click area has to be claimed separately: first a strip of fixed height that
                // senses clicks, then the frame painted on top of it. Hovering brightens the strip
                // and shows the pointer cursor, which says "a click opens the editor". Labels stay
                // non-selectable so none of them swallows the press that belongs to the row.
                let (strip_rect, strip) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), ROW_H),
                    egui::Sense::click(),
                );
                let strip = strip.on_hover_cursor(CursorIcon::PointingHand);
                if strip.hovered() {
                    frame.fill = m.row_fill(true);
                }
                // The row's content goes into a child ui pinned to that strip, so the frame is
                // painted exactly on the click area and cannot shift the list. `new_child` gives us
                // the child directly; its used rect is handed back to the list by `prepared.end`.
                let mut prepared =
                    frame.begin(&mut ui.new_child(egui::UiBuilder::new().max_rect(strip_rect)));
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
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(
                                Label::new(RichText::new(label).strong().size(13.0))
                                    .selectable(false),
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
                                Label::new(
                                    RichText::new(format!("{host} · {age}")).weak().size(11.0),
                                )
                                .selectable(false),
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

                // The strip allocated above is the row's click area. It is stored on the app and
                // used by `clicked_in_row` (and by the click test) on the next pass, so a click
                // still lands on the row it was aimed at even though the frame paints over it.
                row_rects.push(strip_rect);
                row_heights.push(strip_rect.height());
                if clicked_row == Some(idx) || strip.clicked() {
                    begin_edit_idx = Some(idx);
                }
            }

            // Now apply the actions after the loop, when `app` is no longer borrowed.
            if let Some(idx) = begin_edit_idx {
                app.begin_edit(idx);
            }
            if let Some(action) = edit_action {
                action(app);
            }
            // Removing a target closes the editor first: it points at an index that disappears.
            if let Some(idx) = remove_idx {
                app.cancel_edit();
                app.selected = idx;
                app.remove_selected();
            }
            // The colour lives on the target, not in `AppOptions`, so `commit_edit` cannot hand it
            // to the autosave. Flagging it here lets `left` write the state once the panel is done.
            if color_changed {
                app.color_dirty = true;
            }
            app.row_rects = row_rects;
            app.row_heights = row_heights;
        });

    app.stats_scratch = scratch;
}

pub fn bottom(ctx: &egui::Context, app: &mut App) {
    let lang = app.lang();
    let m = app.theme_mode;
    // Taken out of the app so it can be reused while `app.targets` is borrowed. The old
    // `stats()` window started at the newest sample and had no upper bound.
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    egui::TopBottomPanel::bottom("status_bar")
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.window_fill)
                .inner_margin(egui::Margin::symmetric(10, 4)),
        )
        .show(ctx, |ui| {
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

pub fn center(ctx: &egui::Context, app: &mut App) {
    center_ex(ctx, app, false);
}

/// `shot_frame` lets the seeded example history through the "only live samples" rule in
/// `chart`, so a still picture can show a full trace without pretending to be pinging.
pub fn center_ex(ctx: &egui::Context, app: &mut App, shot_frame: bool) {
    let m = app.theme_mode;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(m.plot_fill())
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| match app.view {
            View::Chart => chart(ui, app, shot_frame),
            View::Events => events_view(ui, app),
            View::Stats => stats_view(ui, app),
            View::Settings => settings(ui, app),
        });
}

/* ------------------------------------------------------------------ chart */

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
        // The id whose visibility a click on its name asked to flip; applied after the loop, when
        // `app.targets` is no longer borrowed by it.
        let mut toggle: Option<usize> = None;
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
                .add(Label::new(name).selectable(false))
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
            (ui.available_height() - 150.0).max(160.0),
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
    let zoom = ui.ctx().input(|i| {
        let Some(pos) = i.pointer.hover_pos() else {
            return 0.0;
        };
        if rect.contains(pos) {
            i.raw_scroll_delta.y
        } else {
            0.0
        }
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
    let step = nice_step(horizon);
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
        ui.painter().text(
            egui::pos2(x, rect.bottom() + 3.0),
            egui::Align2::LEFT_TOP,
            label,
            FontId::monospace(10.0),
            m.axis_text(),
        );
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

    let bands = [30.0f32, 80.0, 150.0, 300.0];
    for b in bands {
        if b >= scale_top {
            continue;
        }
        let y = map_y(b, scale_top, rect);
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

    // Event log under the chart.
    ui.add_space(6.0);
    event_log(ui, app, 120.0);
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

/// Distance between two vertical grid lines for a chart window. `pub` so a test can check that
/// a day does not get the minute spacing of a short window.
pub fn nice_step(horizon: f64) -> f64 {
    // Hours and days are the part that matters for a wide window: without them a day would
    // fall back to the 900 s default and print sixty grid lines on top of each other.
    for s in [
        1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1_800.0, 3_600.0,
        7_200.0, 10_800.0, 14_400.0, 21_600.0, 43_200.0, 86_400.0,
    ] {
        if horizon / s <= 12.0 {
            return s;
        }
    }
    86_400.0
}

/// A tick label on the time axis. Short windows count in minutes and seconds; from about an
/// hour upwards that would run past 99 minutes, so wider windows show the wall clock instead —
/// which is also what makes a day readable. `clock` says whether a real time of day is known.
/// `pub` for the tests that check what a day's axis prints.
pub fn fmt_axis(secs: f64, horizon: f64, clock: Option<i64>) -> String {
    if horizon < 2.0 * 3_600.0 {
        let s = secs.max(0.0) as i64;
        format!("{:02}:{:02}", s / 60, s % 60)
    } else if let Some(at_ms) = clock {
        use chrono::{Local, TimeZone};
        match Local.timestamp_millis_opt(at_ms).single() {
            Some(t) => t.format("%H:%M").to_string(),
            None => String::from("?"),
        }
    } else {
        // No absolute stamp to show (a session that has not measured yet): count in hours.
        let s = secs.max(0.0) as i64;
        format!("{:02}:{:02} h", s / 3_600, (s % 3_600) / 60)
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
                // A6: creating a target used to mean finding three controls — the column, the "+", the
                // category. Name and address belong here, and the category decides itself.
                ui.label(RichText::new(lang.targets()).strong());
                ui.add_space(4.0);
                let mut add: Option<(String, String)> = None;
                // The fields are taken out of the app for this frame and put back at the end — otherwise
                // egui writes empty strings into them every frame and the typing disappears.
                let mut pending_host: Option<String> = None;
                let mut pending_label: Option<String> = None;
                let mut drop_host = false;
                let mut drop_label = false;
                let host_id = ui.id().with(SETTINGS_HOST_ID_SALT);
                let label_id = ui.id().with(SETTINGS_LABEL_ID_SALT);
                let mut take =
                    |id: egui::Id| app.settings_target_ids.remove(&id).unwrap_or_default();
                let host_key = ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    let mut host = take(host_id);
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut host)
                                .hint_text(lang.hint_settings_address())
                                .desired_width(200.0),
                        )
                        .changed()
                    {
                        pending_host = Some(host.clone());
                    }
                    let mut label = take(label_id);
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut label)
                                .hint_text(lang.hint_settings_label())
                                .desired_width(160.0),
                        )
                        .changed()
                    {
                        pending_label = Some(label.clone());
                    }
                    // Enter in the address field is enough — the address is what one types first.
                    if (ui.button(lang.add_btn()).clicked() || host_key) && !host.trim().is_empty()
                    {
                        add = Some((host.trim().to_string(), label.trim().to_string()));
                        drop_host = true;
                        drop_label = true;
                    }
                });
                if let Some(host) = pending_host {
                    app.settings_target_ids.insert(host_id, host);
                }
                if let Some(label) = pending_label {
                    app.settings_target_ids.insert(label_id, label);
                }
                if drop_host {
                    app.settings_target_ids.remove(&host_id);
                }
                if drop_label {
                    app.settings_target_ids.remove(&label_id);
                }
                if let Some((host, label)) = add {
                    // A new target is measured right away too — otherwise it just sits there as an empty
                    // row and the first impression is a zero.
                    app.add_suggested_target(&host, &label);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(lang.suggestions()).weak().size(11.5));
                    for (host, label) in [
                        ("8.8.8.8", "Google DNS"),
                        ("1.1.1.1", "Cloudflare"),
                        ("9.9.9.9", "Quad9"),
                        ("224.0.0.1", "Multicast"),
                    ] {
                        if ui
                            .small_button(label)
                            .on_hover_text(i18n::fill(
                                lang.tip_add_suggestion_fmt(),
                                &[("host", host.to_string())],
                            ))
                            .clicked()
                        {
                            app.add_suggested_target(host, label);
                        }
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
                        crate::app::APP_VERSION,
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
