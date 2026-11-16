//! The whole interface: top bar, target list, the four views, event log, status bar.

use crate::app::{self, App, LogKind, TargetKind, View};
use crate::history;
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

/// Dasselbe Spiel für die beiden Zielfelder in der Einstellungen-Gruppe „Ziele“: eigene Salts, weil
/// das Adressfeld sonst mit dem oben in der Leiste denselben Zustand teilte — inklusive Fokus.
const SETTINGS_HOST_ID_SALT: &str = "settings_host_field";
const SETTINGS_LABEL_ID_SALT: &str = "settings_label_field";

/// Text for a selectable control (the four view buttons, the category chips, „nur Störungen“, the
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

/// The two new-target fields in the top bar have to be found by id, because „Enter adds a target“
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
                ui.label(RichText::new("Latenz-Monitor").weak());
                ui.separator();
                // The chart window's preset picker (up to 24 h). It sits with the title because it
                // says how much of the picture is shown, not what is being measured.
                let combo = egui::ComboBox::from_id_salt("horizon_top")
                    .selected_text(horizon_label(app.options.horizon_secs))
                    .width(72.0)
                    .show_ui(ui, |ui| {
                        for c in app::HORIZON_CHOICES {
                            ui.selectable_value(&mut app.options.horizon_secs, c.secs, c.label);
                        }
                    });
                let _ = combo.response.on_hover_text("Zeitfenster des Diagramms");
                if ui
                    .add(egui::SelectableLabel::new(
                        app.follow_live,
                        if app.follow_live { "● Live" } else { "○ Live" },
                    ))
                    .on_hover_text(
                        "Dem aktuellen Zeitfenster folgen — aus, wenn im Diagramm zurückgeblättert \
                         wird; Doppelklick ins Diagramm oder Esc schaltet es wieder ein",
                    )
                    .clicked()
                {
                    app.follow_live = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Der Weg zum Zustandsordner gehört dorthin, wo die Datei entsteht: wer wissen
                    // will, was gespeichert ist, hat gerade auf den Titel geschaut.
                    if ui
                        .small_button("⌂ Daten")
                        .on_hover_text("Zustandsordner im Explorer öffnen (Datei, Protokoll, Exporte)")
                        .clicked()
                    {
                        app.open_state_folder();
                    }
                    // Der Autostart-Knopf ist nur sichtbar, solange nicht gemessen wird.
                    // `autostart_state()` is the registry's answer (looked up at startup, then kept
                    // current); the raw counter alone would be „aus“ until the user presses this.
                    if !app.running
                        && ui
                            .button(if app.autostart_state() == Some(true) {
                                "Autostart: an"
                            } else {
                                "Autostart: aus"
                            })
                            .on_hover_text("Beim Anmelden von Windows starten")
                            .clicked()
                    {
                        let on = app.autostart_state() != Some(true);
                        app.set_autostart(on);
                    }
                    if ui
                        .button(if app.running {
                            "■ Alle stoppen"
                        } else {
                            "▶ Alle starten"
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
                        .hint_text("Ziel, z. B. 192.168.1.1"),
                );
                ui.add_sized(
                    [130.0, 22.0],
                    egui::TextEdit::singleline(&mut app.new_label)
                        .id_salt(NEW_LABEL_ID_SALT)
                        .hint_text("Name (optional)"),
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
                if ui.button("+ Ziel").clicked() {
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
                    .selected_text(app::interval_label(before))
                    .width(72.0)
                    .show_ui(ui, |ui| {
                        for c in app::INTERVAL_CHOICES {
                            ui.selectable_value(&mut app.options.interval_secs, *c, app::interval_label(*c));
                        }
                    })
                    .response
                    .on_hover_text(app::interval_hint(before));
                if (app.options.interval_secs - before).abs() > f32::EPSILON {
                    save_now = true;
                }
                ui.separator();
                // Views first: they decide what the rest of the window shows. Die Ziffer dahinter ist
                // kein Schmuck, sondern das Kürzel (`View::shortcut`) — und die Ansicht nennt es jetzt
                // selbst, damit Knopf, Tastatur und Einstellungen nicht drei verschiedene Listen
                // pflegen.
                for view in [View::Chart, View::Events, View::Stats, View::Settings] {
                    let sel = app.view == view;
                    if ui
                        .selectable_label(sel, selectable_text(view.label(), view.shortcut()))
                        .on_hover_text(view.hint())
                        .clicked()
                    {
                        app.view = view;
                    }
                }
                ui.separator();
                ui.add_sized(
                    [160.0, 22.0],
                    egui::TextEdit::singleline(&mut app.filter).hint_text("Ziele filtern"),
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
                    // Das Wort „aus“ am Chip: ohne es sieht ein ausgeblendeter Schalter aus wie ein
                    // Knopf, der nicht reagiert. So steht an der Stelle, was Sache ist.
                    let mut chip = ui.selectable_label(
                        on,
                        selectable_text(
                            if on {
                                k.label().to_string()
                            } else {
                                format!("{} aus", k.label())
                            },
                            "",
                        ),
                    );
                    if !on {
                        chip = chip.on_hover_text(format!(
                            "{k} wieder einblenden — im Moment ist die ganze Kategorie verborgen",
                            k = k.label()
                        ));
                    } else {
                        chip = chip.on_hover_text(format!(
                            "{k} ausblenden — alle Ziele dieser Kategorie sind im Diagramm und in den Listen verborgen, gemessen wird weiter",
                            k = k.label()
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
                        .small_button("alle")
                        .on_hover_text("Alle Kategorien wieder einblenden")
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
            let want_add =
                ui.input(|i| i.key_pressed(egui::Key::Enter)) && focused && !app.new_host.is_empty();
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
    // Esc first cancels an open editor; only without one does it mean „back to the live edge“.
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
        ui.label(RichText::new("Ziele").strong());
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
            // Set by the „entfernen“ button inside an open editor: which target to delete once the
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
                let kind_label = t.kind.label().to_string();
                let enabled = t.enabled;
                let auto_idx = app.color_idx_of(t);
                // Which palette slot this line uses, said plainly: automatic or picked by hand.
                // Shown as a tooltip on the colour dash in the row.
                let trace_tip = if auto_idx == crate::app::AUTO_COLOR {
                    "Linienfarbe automatisch — im Editor wählbar".to_string()
                } else {
                    format!("Linienfarbe selbst gewählt ({})", auto_idx + 1)
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
                            // Der Farbstrich: dieselbe Farbe wie die Linie im Diagramm, damit schon
                            // die Liste sagt, welche Kurve zu welchem Ziel gehört.
                            let (line_rect, _) =
                                ui.allocate_exact_size(Vec2::new(16.0, 11.0), egui::Sense::hover());
                            ui.painter().rect_filled(line_rect.shrink(4.5), 1.0, trace);
                            // Das Ziel loswerden, an dem man gerade arbeitet: ersetzt den Knopf
                            // „Ziel entfernen“ oben rechts, der das markierte Ziel traf und damit
                            // leicht das falsche erwischte.
                            if ui
                                .small_button(RichText::new("entfernen").color(m.danger_text()))
                                .on_hover_text(format!("{label} entfernen"))
                                .clicked()
                            {
                                remove_idx = Some(idx);
                            }
                            ui.label(RichText::new("Bearbeiten").strong().size(12.0));

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("✕")
                                        .on_hover_text("Änderungen verwerfen (Esc)")
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
                                .hint_text("Name (optional)")
                                .vertical_align(egui::Align::Center),
                        );
                        ui.add_sized(
                            [215.0, 20.0],
                            egui::TextEdit::singleline(&mut app.edit_host)
                                .hint_text("Adresse")
                                .font(FontId::monospace(12.5))
                                .vertical_align(egui::Align::Center),
                        );

                        // Category and whether to measure it at all.
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("edit_kind")
                                .width(96.0)
                                .selected_text(RichText::new(app.edit_kind.label()).size(12.0))
                                .show_ui(ui, |ui| {
                                    for k in TargetKind::all() {
                                        ui.selectable_value(&mut app.edit_kind, k, k.label());
                                    }
                                });
                            ui.add_enabled_ui(!app.edit_host.trim().is_empty(), |ui| {
                                ui.checkbox(&mut app.edit_enabled, "misst").on_hover_text(
                                    "Aus schaltet die Messung ab, der Verlauf bleibt erhalten",
                                );
                            });
                        });

                        // Farbe der Linie. Ohne Wahl gibt sie die Palette nach der Position in der
                        // Liste her; wer hier etwas anfasst, bekommt eine feste Zahl, die auch nach
                        // einem Neustart und beim Umsortieren dieselbe Farbe ergibt. Der Knopf ist
                        // gleichzeitig die Vorschau: er zeigt genau den Ton, den das Diagramm malt.
                        let picked = app.edit_color_idx != app::AUTO_COLOR;
                        let shown_idx = if picked { app.edit_color_idx } else { auto_idx };
                        let swatch_name = if picked {
                            theme::swatch(app.edit_color_idx).name
                        } else {
                            "Auto"
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
                                                RichText::new(s.name).size(12.0),
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
                                .small_button("Auto")
                                .on_hover_text(
                                    "Farbe wieder automatisch nach der Position in der Liste",
                                )
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
                                if enabled { "aktiv" } else { "pausiert" }
                            ))
                            .weak()
                            .size(11.0),
                        );
                        ui.label(
                            RichText::new(format!("{}/{} Werte · {}", st.replies, st.total, avg))
                                .weak()
                                .size(11.0)
                                .monospace(),
                        );
                        ui.label(
                            RichText::new(format!("{} · {} Verluste", span, st.losses))
                                .weak()
                                .size(11.0)
                                .monospace(),
                        );

                        ui.horizontal(|ui| {
                            if ui.button("✓ Speichern").clicked() {
                                edit_action = Some(App::commit_edit);
                            }
                            if ui.button("✗ Abbrechen").clicked() {
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
                        // Strich in Linienfarbe: wer die Liste liest, sieht sofort, welche Kurve im
                        // Diagramm zu diesem Ziel gehört. Der Strich ist gleich hoch wie der Punkt,
                        // damit beide zusammen eine Zeile bilden und nicht verschoben wirken.
                        let (line_rect, _) =
                            ui.allocate_exact_size(Vec2::new(16.0, 11.0), egui::Sense::hover());
                        ui.painter().rect_filled(line_rect.shrink(4.5), 1.0, trace);
                        // Was der Strich bedeutet: automatisch oder per Hand gewählte Linienfarbe. Der
                        // Knauf ist derselbe wie im Diagramm, damit beide Stellen gleichbedeutend sind.
                        let knob = Rect::from_center_size(
                            line_rect.center(),
                            Vec2::splat(crate::theme::LEGEND_KNOB),
                        );
                        ui.put(knob, Label::new("").selectable(false))
                            .on_hover_text(&trace_tip);
                        // Der Strich gehört zum Text: erst der Abstand, dann die Beschriftung.
                        ui.add_space(7.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(
                                Label::new(RichText::new(label).strong().size(13.0))
                                    .selectable(false),
                            );
                            let age = if secs_ago < 0.0 {
                                "noch nicht geprüft".to_string()
                            } else {
                                format!("vor {} s", secs_ago.max(0.0) as i64)
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
                ui.label(RichText::new(if app.running { "läuft" } else { "gestoppt" }).strong());

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
                ui.label(format!("Prüfungen: {}", probed));
                ui.label(
                    RichText::new(format!("Verluste: {}", lost)).color(m.loss_count(lost > 0)),
                );
                // Gezählt wird alles, gemerkt nur, was ins Protokoll passt — die beiden Zahlen sind
                // selten gleich, und eine allein wäre leicht als „so viel ist passiert“ gelesen.
                if app.log_total > app.log.len() {
                    ui.label(format!(
                        "Ereignisse: {} von {}",
                        app.log.len(),
                        app.log_total
                    ));
                } else {
                    ui.label(format!("Ereignisse: {}", app.log.len()));
                }
                ui.separator();
                ui.label(RichText::new(&app.status_msg).weak().italics());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // A8: ab einer Stunde wäre „65:00“ eine Zahl, die man erst umrechnen muss.
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
                        RichText::new(format!("Laufzeit {uptime}"))
                            .monospace()
                            .weak(),
                    );
                    ui.separator();
                    // A2: die Zustandsdatei schreibt alle 8 s von selbst, ein „Speichern“-Knopf wäre
                    // ein dritter Weg für dieselbe Sache. Was fehlt, ist die Rückmeldung — also steht
                    // hier, wann zuletzt gesichert wurde.
                    let age = app.save_age_secs();
                    let saved = if age < 0.0 {
                        "noch nicht gesichert".to_string()
                    } else if age < 20.0 {
                        format!("gesichert vor {} s", age as i64)
                    } else {
                        format!("zuletzt vor {} min", age as i64 / 60)
                    };
                    ui.label(RichText::new(saved).weak().size(11.5));
                    if ui
                        .small_button("CSV")
                        .on_hover_text(
                            "Messwerte sofort in den Zustandsordner schreiben und ihn öffnen",
                        )
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
        ui.label(RichText::new("Latenzverlauf").strong());
        // No „Live“ switch here: the top bar has it, and two controls for one state only invite
        // confusion about which one is speaking.
        if ui.button("◀ zurück").clicked() {
            app.follow_live = false;
            app.pan_offset_secs += horizon * 0.25;
        }
        let back = ui.button("vor ▶");
        if back.clicked() {
            app.pan_offset_secs = (app.pan_offset_secs - horizon * 0.25).max(0.0);
            if app.pan_offset_secs < 0.5 {
                app.follow_live = true;
            }
        }
        ui.separator();
        if ui.button("⚑ nächste").clicked() {
            app.jump_to_next_outlier();
        }
        if ui.button("frühere ⚐").clicked() {
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
            // Ein kleiner Farbknopf vor dem Namen, damit die Legende dasselbe Zeichen benutzt wie
            // die Zielliste: quadratisch, überall gleich groß. Ein ausgeblendetes Ziel zeigt ihn
            // nur als Umriss — so bleibt erkennbar, welche Farbe es hätte, wenn es wieder da ist.
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
            // Ein Klick auf den Namen blendet genau dieses Ziel ein oder aus — gemessen wird
            // weiter, nur die Darstellung verschwindet. Der Name bleibt lesbar und schwach, damit
            // man das ausgeblendete Ziel wiederfindet.
            let name = RichText::new(&t.label)
                .color(if t.hidden { m.text_weak() } else { c })
                .strong();
            let tip = if t.hidden {
                format!("{} · ausgeblendet — klicken zum Einblenden", t.host)
            } else {
                format!(
                    "{} · {} — klicken zum Ausblenden",
                    t.host,
                    app.status_text(t)
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
            // Anchor the cursor's time between old and new window, then clamp at „now“, which is
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

    // B5: Ein leeres Diagramm ist eine Antwort ohne Frage. Ohne Ziel auf dem Bildschirm sagt die
    // Fläche deshalb, *warum* sie leer ist — Filter, Kategorie oder ausgeblendete Ziele sind drei
    // verschiedene Knöpfe, an denen man drehen kann.
    let shown_count = app.targets.iter().filter(|t| is_shown(app, t)).count();
    if shown_count == 0 && !app.targets.is_empty() {
        let why = if !app.all_kinds_on() {
            "Alle Kategorien sind über die Schalter oben ausgeblendet."
        } else if app.targets.iter().all(|t| t.hidden) {
            "Alle Ziele sind ausgeblendet — klicke ihren Namen in der Legende an, um sie \
             zurückzuholen."
        } else {
            "Kein Ziel passt zum Textfilter oben rechts — mit ✕ oder einem leeren Feld geht es \
             weiter."
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

    // C4: Welche Messwerte als Ausreißer gelten, hängt an der ganzen Serie des Ziels und ändert
    // sich nur, wenn ein neuer Wert dazukommt. Die Kurven, die Ringe und die Hover-Anzeige brauchen
    // denselben Schnitt — also wird er hier einmal pro Frame und Ziel berechnet und in
    // `cut_scratch` abgelegt (indiziert nach Ziel-Id, damit die Auswertung nicht umsortiert werden
    // muss). Ein ausgeblendetes Ziel bekommt `f64::MAX`: es hat nichts zu markieren.
    {
        // Index statt Iterator: `is_shown` braucht `app` unverändert, deshalb darf der Vektor nicht
        // gleichzeitig als mutable Anleihe gehalten werden. Der Schnitt kommt in eine lokale Liste
        // und wird danach umgehängt.
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
            format!("{} ms", b as i32),
            FontId::monospace(10.0),
            m.axis_text(),
        );
    }
    // The top edge of the plot is where the auto scale ends, and that number changes with the
    // data. Said out loud, it stops being a guess — especially when spikes are clamped there.
    ui.painter().text(
        egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        format!("{:.0} ms", scale_top),
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
                    // C4: die Ausreißer-Grenze wird pro Frame und Ziel genau einmal berechnet (siehe
                    // der Schnitt vor den Kurven) und hier nur noch gelesen.
                    let cut = app.cut_scratch.get(t.id).copied().unwrap_or(f64::MAX);
                    rows.push((
                        format!(
                            "{}  {}{}",
                            t.label,
                            if history::is_ok(s.rtt_ms) {
                                format!("{:.1} ms", s.rtt_ms)
                            } else {
                                "Timeout".into()
                            },
                            if (s.rtt_ms as f64) > cut {
                                "  <- Ausr."
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
/// Every row carries the wall clock rather than „+123 s“: the chart counts from program start, so
/// two windows open at different times would disagree about when an outage happened. The same
/// conversion as the CSV export is used, which makes a log line and a CSV line the same fact.
fn event_log(ui: &mut Ui, app: &mut App, max_height: f32) {
    let m = app.theme_mode;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Ereignisse").strong());
        // Gezählt wird alles, gemerkt nur, was ins Protokoll passt — sonst liest man die Länge der
        // Liste als „so viel ist passiert“.
        if app.log_total > app.log.len() {
            ui.label(RichText::new(format!("{} von {}", app.log.len(), app.log_total)).weak());
        } else {
            ui.label(RichText::new(format!("({})", app.log.len())).weak());
        }
        // A7: nach einer langen Nacht will man die Störfälle sehen, nicht jede Meldung. Gefiltert
        // wird nur die Ansicht — „leeren“ bliebe sonst der einzige Weg einzugrenzen.
        let only_bad = app.log_only_outages;
        let filt = ui
            .selectable_label(only_bad, selectable_text("nur Störungen", ""))
            .on_hover_text("Nur Ausfälle und Wiederherstellungen zeigen")
            .clicked();
        if filt {
            app.log_only_outages = !only_bad;
        }
        if ui.small_button("leeren").clicked() {
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
                // A7: die Meldung und der Verlauf an einem Ort. Ein Doppelklick auf eine Störungs-
                // zeile holt das Diagrammfenster auf diesen Zeitpunkt — sonst sitzt man in der Liste,
                // liest „Ausfall um 03:12“ und muss erst suchen, wo das war.
                if e.kind != LogKind::Info && row.double_clicked() {
                    jump = Some(e.secs);
                }
            }
        });
    if let Some(secs) = jump {
        app.jump_to_secs(secs);
    }
}

/// Column widths for the statistics grid: Ziel, Host, Status, Min, Ø, Median,
/// p95, Max, Verlust, Ausr.
/// Column widths of the statistics table: Ziel, Host, Status, Schnitt, Min, Letzter, P95, Max,
/// Ausfälle, Verlust. The first two hold names and addresses, which are what actually gets long;
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
    let m = app.theme_mode;
    ui.label(RichText::new("Statusliste").strong());
    ui.add_space(4.0);
    let now = app.engine.elapsed_secs();
    // Taken out of the app so it can be reused while `app.targets` is borrowed.
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    // B3: geklickt werden kann nur, was schon gezeichnet ist — also die Id merken und nach der
    // Schleife auflösen.
    let mut unhide_id: Option<usize> = None;
    // B5: auch hier ist „keine Zeile“ eine Antwort, die erklärt sein will.
    if app.targets.iter().filter(|t| app.shown(t)).count() == 0 {
        ui.label(RichText::new(weak_note(app)));
    }
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for idx in 0..app.targets.len() {
                // A1/B2: was im Diagramm nicht zu sehen ist, fehlt auch hier nicht — es wird
                // schwächer geschrieben und bleibt so als Beweis des Ausblendens stehen.
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
                        // B3: wer ausgeblendet ist, kann hier ebenso zurückgeholt werden wie über
                        // die Legende — der Aufwärts-Punkt neben dem Namen tut es.
                        if app.targets[idx].hidden
                            && ui
                                .small_button(RichText::new("◉").color(m.text_dim()))
                                .on_hover_text("Ziel wieder anzeigen")
                                .clicked()
                        {
                            unhide_id = Some(app.targets[idx].id);
                        }
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 6.0, color);
                        // Strich in Linienfarbe, wie in der Zielliste links.
                        let (line_rect, _) =
                            ui.allocate_exact_size(Vec2::new(18.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(line_rect.shrink(5.0), 2.0, trace);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            // B2: derselbe Name, dieselbe Stelle — nur deutlich leiser als alles,
                            // was gerade gemessen wird. Der Grund steht daneben, nicht im Tooltip.
                            let mut name = format!("{}   ({})", t.label, t.host);
                            if !shown {
                                name.push_str(if t.hidden {
                                    "  · ausgeblendet"
                                } else {
                                    "  · gefiltert"
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
                                "wartet".to_string()
                            } else {
                                format!("vor {:.0} s", secs_ago.max(0.0))
                            };
                            ui.add(
                                Label::new(
                                    RichText::new(format!(
                                        "{} · {} · {}",
                                        t.kind.label(),
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
                            RichText::new(format!(
                                "min {} · Ø {} · p95 {} · max {} · Verlust {:.1} %{}",
                                crate::app::fmt_ms(st.min),
                                crate::app::fmt_ms(st.avg as f32),
                                crate::app::fmt_ms(st.p95),
                                crate::app::fmt_ms(st.max),
                                st.loss_pct,
                                if st.outliers > 0 {
                                    format!(" · {} Ausr.", st.outliers)
                                } else {
                                    String::new()
                                }
                            ))
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
    if !app.all_kinds_on() {
        "Alle Kategorien sind über die Schalter oben ausgeblendet.".to_string()
    } else if app.targets.iter().all(|t| t.hidden) {
        "Alle Ziele sind ausgeblendet — in der Legende des Diagramms holen sie ihre Namen zurück."
            .to_string()
    } else {
        "Kein Ziel passt zum Textfilter oben rechts.".to_string()
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
    // C2: eine Sparkline ist 16 px hoch und sagt nur „ruhig“ oder „bewegt sich das“. Ein einzelner
    // Ausreißer von 900 ms drückt alles andere auf eine gerade Linie — deshalb endet die Achse beim
    // p95 der Serie, mindestens aber bei 40 ms. Was darüber liegt, bleibt als Ring sichtbar.
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
/// the header, the arrow and [`App::sort_targets`] cannot disagree about what „p95“ means.
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
    let m = app.theme_mode;
    ui.label(RichText::new("Statistiken").strong());
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
    // B4/C6: every column header is a button. The first click sorts descending — „die schlechtesten
    // zuerst“, because that is what one looks for in a latency table — and the second reverses it.
    // The arrow is painted next to the word rather than replacing it, so an unsorted table still
    // says what each column would do.
    let mut sort_click: Option<(app::SortColumn, bool)> = None;
    let widths = column_widths();
    ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (n, c) in [
                "Ziel", "Host", "Status", "Min", "Ø", "Median", "p95", "Max", "Verlust", "Ausr.",
            ]
            .iter()
            .enumerate()
            {
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
                    btn.clone().on_hover_text(format!(
                        "Nach {c} sortieren — nochmaliger Klick kehrt die Richtung um"
                    ));
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
                    // C3: dieselbe Rechnung wie links in der Liste und unten in der Gesamt-Zeile.
                    let st = app.stats_for(t, &mut scratch);
                    let color = app.status_color(t);
                    let trace = app.trace_color(t);
                    let txt = app.status_text(t);
                    let sel = app.selected == idx;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        // Farbstrich vor dem Namen: dieselbe Farbe wie die Linie im Diagramm.
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
    // D4: „Gesamt“ ist keine Summe der Zeilen, sondern dieselbe Rechnung wie pro Ziel — nur über
    // alle Antworten der Auswahl hinweg. Median und p95 zweier Ziele zu mitteln ergäbe eine Latenz,
    // die kein Ziel gemessen hat; also sammelt `stats_total` alle Antworten in einen Puffer und zieht
    // die Perzentile daraus.
    let mut all = std::mem::take(&mut app.all_rtt_scratch);
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    let st = app.stats_total(&mut scratch, &mut all);
    app.stats_scratch = scratch;
    app.all_rtt_scratch = all;
    // Eine Spalte ohne Messwert ist „—“ und nicht 0: ein Ziel, das nie geantwortet hat, hat keine
    // Latenz, und eine Null in einer Latenzspalte wäre eine Aussage, die nicht stimmt.
    let num = |v: f32| {
        if v.is_finite() && st.replies > 0 {
            crate::app::fmt_ms(v)
        } else {
            "—".to_string()
        }
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let widths = column_widths();
        ui.add_sized(
            [widths[0] + widths[1] + widths[2], 20.0],
            Label::new(
                RichText::new(format!("Gesamt · {} Prüfungen", st.total))
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
                "—".to_string()
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

/// The label of the current window length, and a fallback for a value from an older file that
/// is not one of the presets.
fn horizon_label(secs: f32) -> String {
    app::HORIZON_CHOICES
        .iter()
        .find(|c| c.secs == secs)
        .map(|c| String::from(c.label))
        .unwrap_or_else(|| format!("{secs:.0} s"))
}

/// What a window length means in practice: how many values it holds and how much memory that
/// is per target. A day is the first choice where the number is worth showing.
fn horizon_hint(secs: f32) -> String {
    let samples = app::samples_for_horizon(secs, 1.0);
    if secs < 3_600.0 {
        String::from("das Diagramm zeigt diesen Zeitraum")
    } else {
        let mib =
            samples as f64 * std::mem::size_of::<history::Sample>() as f64 / (1024.0 * 1024.0);
        format!("rund {samples} Messwerte pro Ziel ({mib:.1} MiB) bei 1 s Abstand")
    }
}

fn settings(ui: &mut Ui, app: &mut App) {
    let mut dirty = false;
    let m = app.theme_mode;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.group(|ui| {
            ui.label(RichText::new("Prüfung").strong());
            ui.add_space(4.0);
            // Das Intervall hat seinen Regler oben in der Leiste und wird von dort gespeichert;
            // ein zweiter Regler für dieselbe Zahl war nur eine zweite Gelegenheit, sich zu
            // wundern, wo man es gerade geändert hat.
            ui.label(
                RichText::new(format!(
                    "Intervall: {:.2} s — oben in der Leiste einzustellen",
                    app.options.interval_secs
                ))
                .weak(),
            );
            ui.horizontal(|ui| {
                ui.label("Zeitüberschreitung");
                if ui.add(egui::DragValue::new(&mut app.options.timeout_ms).range(200..=10_000).suffix(" ms").speed(50)).changed() { dirty = true; }
            });
            ui.horizontal(|ui| {
                ui.label("Gleichzeitige Prüfungen");
                if ui.add(egui::DragValue::new(&mut app.options.concurrency).range(1..=32)).changed() { dirty = true; }
            });
            ui.horizontal(|ui| {
                ui.label("Zeitfenster");
                // Die Voreinstellungen reichen bis 24 h. Ein Schieberegler wäre hier falsch:
                // zwischen 15 s und einem Tag sieht man den Unterschied von Minuten nicht.
                egui::ComboBox::from_id_salt("horizon")
                    .selected_text(horizon_label(app.options.horizon_secs))
                    .show_ui(ui, |ui| {
                        for c in app::HORIZON_CHOICES {
                            if ui
                                .selectable_value(&mut app.options.horizon_secs, c.secs, c.label)
                                .on_hover_text(horizon_hint(c.secs))
                                .changed()
                            {
                                dirty = true;
                            }
                        }
                    });
                ui.label(
                    RichText::new(horizon_hint(app.options.horizon_secs))
                        .weak()
                        .size(11.5),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Max. Messwerte je Ziel");
                // Ein Tag bei einer Sekunde Abstand sind 86 400 Werte pro Ziel. Die Obergrenze
                // richtet sich deshalb nach dem Zeitfenster statt nach einem festen Maximum.
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
                    .on_hover_text(format!(
                        "0 = automatisch ({} für das aktuelle Fenster). Obergrenze ist das \
                         Zehnfache des Fensters bei {} s Abstand.",
                        app::samples_for_horizon(app.options.horizon_secs, app.options.interval_secs),
                        app.options.interval_secs
                    ))
                    .changed()
                {
                    dirty = true;
                }
            });
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.label(RichText::new("Start").strong());
            ui.add_space(4.0);
            // A3: Ton, Infobereich und minimierter Start waren als Knöpfe zu sehen, obwohl das
            // Programm nichts davon tut. Die Felder bleiben in der Datei für die, die es
            // umsetzen; ein Knopf, der nichts bewirkt, ist schlimmer als kein Knopf.
            ui.horizontal(|ui| {
                let on = app.autostart_state() == Some(true);
                let mut v = on;
                if ui.checkbox(&mut v, "Beim Anmelden starten (Windows)").changed() {
                    app.set_autostart(v);
                }
                if app.autostart_state() == Some(false) {
                    ui.label(RichText::new("Autostart nicht möglich").color(app.theme_mode.warn()));
                }
            });
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            // A6: ein Ziel anlegen heißt heute drei Knöpfe suchen — Spalte, „+“, Kategorie. Hier
            // gehören Name und Adresse hin, und die Kategorie entscheidet sich von selbst.
            ui.label(RichText::new("Ziele").strong());
            ui.add_space(4.0);
            let mut add: Option<(String, String)> = None;
            // Die Felder werden für den Frame aus der App genommen und am Ende zurückgelegt —
            // sonst schreibt egui jeden Frame leere Strings hinein und die Eingabe verschwindet.
            let mut pending_host: Option<String> = None;
            let mut pending_label: Option<String> = None;
            let mut drop_host = false;
            let mut drop_label = false;
            let host_id = ui.id().with(SETTINGS_HOST_ID_SALT);
            let label_id = ui.id().with(SETTINGS_LABEL_ID_SALT);
            let mut take = |id: egui::Id| {
                app.settings_target_ids
                    .remove(&id)
                    .unwrap_or_default()
            };
            let host_key = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let mut host = take(host_id);
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut host)
                            .hint_text("Adresse, z. B. 8.8.8.8")
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
                            .hint_text("Name (leer = Adresse)")
                            .desired_width(160.0),
                    )
                    .changed()
                {
                    pending_label = Some(label.clone());
                }
                // Enter im Adressfeld genügt — die Adresse ist das, was man zuerst tippt.
                if (ui.button("Hinzufügen").clicked() || host_key) && !host.trim().is_empty() {
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
                // Ein neues Ziel wird auch gleich gemessen — sonst steht es als leere Zeile da und
                // der erste Eindruck ist eine Null.
                app.add_suggested_target(&host, &label);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Vorschläge:").weak().size(11.5));
                for (host, label) in [
                    ("8.8.8.8", "Google DNS"),
                    ("1.1.1.1", "Cloudflare"),
                    ("9.9.9.9", "Quad9"),
                    ("224.0.0.1", "Multicast"),
                ] {
                    if ui
                        .small_button(label)
                        .on_hover_text(format!("{host} hinzufügen und sofort messen"))
                        .clicked()
                    {
                        app.add_suggested_target(host, label);
                    }
                }
            });
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.label(RichText::new("Darstellung").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Farbschema");
                for (value, label) in [
                    ("system", "System"),
                    ("light", "Hell"),
                    ("dark", "Dunkel"),
                ] {
                    let on = app.options.theme == value;
                    if ui.selectable_label(on, label).clicked() {
                        app.options.theme = String::from(value);
                        dirty = true;
                    }
                }
            });
            ui.label(
                RichText::new("„System“ folgt der Hell-/Dunkel-Einstellung von Windows und wechselt sofort.")
                    .weak()
                    .size(11.5),
            );
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            // Früher stand hier eine eigene Gruppe „Ziel“ mit „Ziel entfernen“ und „Prüfung
            // zurücksetzen“. Das Entfernen passiert jetzt am Ziel selbst (Inline-Editor), und ein
            // Knopf, der das gerade markierte Ziel traf, war zu leicht misszuverstehen.
            ui.label(RichText::new("Daten").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .button("CSV exportieren")
                    .on_hover_text("Schreibt in den Ordner der gespeicherten Einstellungen")
                    .clicked()
                {
                    let path = default_export_path();
                    app.export_csv(path);
                }
                if ui
                    .button("Speichern unter…")
                    .on_hover_text("Ordner und Dateiname selbst wählen")
                    .clicked()
                {
                    app.open_export_dialog();
                }
                // A2: kein „Speichern“-Knopf — es wird ohnehin bei jeder Änderung geschrieben, was
                // der Knopf also verspricht, stimmt nicht. Die Statusleiste sagt jetzt, wann.
                // Betrifft alle Ziele — deshalb sagt der Knopf das auch. Ein offener Editor wird
                // vorher übernommen, sonst verschwinden die Werte unter den offenen Eingaben weg.
                // Weil sich der Verlauf nicht zurückholen lässt, will der Knopf zwei Klicks: der
                // erste scharft ihn nur an und fragt nach, der zweite löscht. Nach ein paar Sekunden
                // verfängt sich die Nachfrage wieder, sonst löscht ein vergessener Warnhinweis.
                let clear_label = if app.clear_armed {
                    "Wirklich löschen?"
                } else {
                    "Messwerte löschen"
                };
                let clear_btn = ui.button(RichText::new(clear_label).color(if app.clear_armed {
                    m.danger_text()
                } else {
                    Color32::from_rgb(150, 150, 150)
                }));
                // `on_hover_text` consumes the response, so both arms of the choice have to be
                // written as one expression that hands the (unchanged) response back.
                let clear_btn = if app.clear_armed {
                    clear_btn.on_hover_text("Nochmal klicken: Verlauf und Statistik aller Ziele löschen.")
                } else {
                    clear_btn.on_hover_text(
                        "Verlauf und Statistik aller Ziele leeren. Die Ziele selbst bleiben erhalten. Zwei Klicks nötig.",
                    )
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
                ui.label(RichText::new("Einstellungen und Verlauf werden automatisch gesichert.").weak().size(11.5));
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut app.options.protocol_log, "Protokoll-Datei schreiben")
                    .on_hover_text("Jede Messung wird sofort in eine Datei geschrieben, nicht erst beim Export.")
                    .changed()
                {
                    let format = crate::exporter::ProtocolFormat::from_path(&app.options.protocol_path);
                    app.set_protocol(app.options.protocol_log, format);
                    dirty = true;
                }
            });
            ui.horizontal(|ui| {
                ui.add_enabled_ui(app.options.protocol_log, |ui| {
                    let mut path = app.options.protocol_path.clone();
                    if ui
                        .add(egui::TextEdit::singleline(&mut path).hint_text("Standard: neben den gespeicherten Daten (protocol.csv)").desired_width(300.0))
                        .changed()
                    {
                        app.set_protocol(true, crate::exporter::ProtocolFormat::from_path(&path));
                        app.options.protocol_path = path;
                        dirty = true;
                    }
                });
            });
            ui.add_space(4.0);
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            // B1/C5: Kürzel, die nirgends nachzulesen sind, existieren nicht. Was hier steht, ist
            // dieselbe Liste, die in der Leiste unter den Knöpfen steht.
            ui.label(RichText::new("Tastatur").strong());
            ui.add_space(4.0);
            for (keys, what) in [
                ("1 – 4", "Diagramm / Messwerte / Ereignisse / Einstellungen"),
                ("Leertaste", "Alle Ziele starten oder anhalten"),
                ("Esc", "Editor schließen, sonst zurück ins Live-Bild"),
                ("← / →", "Zeitfenster eine halbe Breite weit schieben"),
                ("Mausrad im Diagramm", "Zoomen, der Cursor bleibt dieselbe Zeit"),
                ("Doppelklick im Diagramm", "Zurück zum laufenden Betrieb"),
                ("Doppelklick in der Liste", "Zum Ausschlag springen"),
            ] {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [180.0, 18.0],
                        Label::new(RichText::new(keys).monospace().size(12.0)).selectable(false),
                    );
                    ui.label(RichText::new(what).size(12.0));
                });
            }
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.label(RichText::new("Info").strong());
            ui.label(RichText::new(concat!(
                "System-Ping Open Source — ein Latenz-Monitor, von Grund auf neu geschrieben.\n",
                "Die Prüfungen laufen über das System-Werkzeug \"ping\" in einem eigenen\n",
                "Arbeiterpool, damit die Oberfläche nie blockiert. Der Verlauf wird lokal\n",
                "als JSON gesichert und kann als CSV exportiert werden. Eine optionale\n",
                "Protokoll-Datei schreibt jeden Messwert sofort fortlaufend mit."
            )).weak().size(11.5));
        });
    });

    if dirty {
        app.sync_caps();
        app.sync_engine();
        // Save on the next tick.
        app.save_now();
    }
}

/// Where „CSV exportieren“ writes without asking: the state folder, so the file lands next to the
/// settings instead of wherever the shortcut happened to start the program from.
pub fn default_export_path() -> PathBuf {
    store::state_dir().join(default_export_name())
}

/// The suggested file name — a timestamped name in the root of the target folder. Shared by the
/// automatic export and the pre-filled name of „Speichern unter…“.
pub fn default_export_name() -> String {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("system-ping-{stamp}.csv")
}
