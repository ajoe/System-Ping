//! The whole interface: top bar, target list, the four views, event log, status bar.

use crate::app::{self, App, TargetKind, View};
use crate::history;
use crate::theme::{self, Mode};
use egui::{Color32, FontId, Label, Rect, RichText, ScrollArea, Stroke, Ui, Vec2};
use std::path::PathBuf;

pub fn draw(ctx: &egui::Context, app: &mut App) {
    top(ctx, app);
    left(ctx, app);
    bottom(ctx, app);
    center(ctx, app);
}

pub fn top(ctx: &egui::Context, app: &mut App) {
    let m = app.theme_mode;
    egui::TopBottomPanel::top("top_bar")
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.window_fill)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("System-Ping").color(m.accent()).strong());
                ui.label(RichText::new("Latenz-Monitor").weak());
                ui.separator();

                let mut add = false;
                let host_buf = app.new_host.clone();
                let label_buf = app.new_label.clone();
                ui.add_sized(
                    [190.0, 22.0],
                    egui::TextEdit::singleline(&mut app.new_host)
                        .hint_text("Ziel, z. B. 192.168.1.1"),
                );
                ui.add_sized(
                    [130.0, 22.0],
                    egui::TextEdit::singleline(&mut app.new_label).hint_text("Name (optional)"),
                );
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let combo = egui::ComboBox::from_id_salt("horizon_top")
                    .selected_text(horizon_label(app.options.horizon_secs))
                    .width(72.0)
                    .show_ui(ui, |ui| {
                        for c in app::HORIZON_CHOICES {
                            ui.selectable_value(&mut app.options.horizon_secs, c.secs, c.label);
                        }
                    });
                let _ = combo.response.on_hover_text("Zeitfenster des Diagramms");
                if (enter && !(host_buf.is_empty() && label_buf.is_empty()))
                    || ui.button("+ Ziel").clicked()
                {
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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

            ui.horizontal(|ui| {
                for view in [View::Chart, View::Events, View::Stats, View::Settings] {
                    let sel = app.view == view;
                    if ui
                        .selectable_label(sel, RichText::new(view.label()).weak())
                        .on_hover_text(view.hint())
                        .clicked()
                    {
                        app.view = view;
                    }
                }
                ui.separator();
                ui.add(
                    egui::Slider::new(&mut app.options.interval_secs, 0.25..=10.0)
                        .suffix(" s")
                        .text("Intervall"),
                );
                ui.add(
                    egui::Slider::new(&mut app.options.horizon_secs, 15.0..=900.0)
                        .logarithmic(true)
                        .suffix(" s")
                        .text("Zeitfenster"),
                );
                ui.separator();
                ui.add_sized(
                    [160.0, 22.0],
                    egui::TextEdit::singleline(&mut app.filter).hint_text("Ziele filtern"),
                );
                if !app.filter.is_empty() && ui.button("✕").clicked() {
                    app.filter.clear();
                }
            });
        });
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
                            ui.painter().rect_filled(line_rect.shrink(4.0), 1.5, trace);
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

                let frame = egui::Frame::new()
                    .fill(m.row_fill(sel))
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(7, 5));
                // The labels are explicitly not selectable: selecting text is not what a row is for,
                // and a selectable label senses clicks, which would let it swallow the press that
                // belongs to the row. With every label on `Sense::hover()` the whole row stays one
                // click target.
                let resp = frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(11.0), egui::Sense::hover());
                        let dot_rect = dot_rect.shrink(1.5);
                        ui.painter().rect_filled(dot_rect, 5.5, color);
                        // Strich in Linienfarbe: wer die Liste liest, sieht sofort, welche Kurve im
                        // Diagramm zu diesem Ziel gehört.
                        let (line_rect, _) =
                            ui.allocate_exact_size(Vec2::new(16.0, 11.0), egui::Sense::hover());
                        ui.painter().rect_filled(line_rect.shrink(4.0), 1.5, trace);

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
                });

                // The click area of the row: `Frame::show` registers its area with `Sense::hover()`
                // only, so this is the rect that says where a row is. It is stored on the app and
                // used by `clicked_in_row` (and by the click test) on the next pass.
                row_rects.push(resp.response.rect);
                row_heights.push(resp.response.rect.height());
                if clicked_row == Some(idx) {
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
                ui.label(format!("Ereignisse: {}", app.alert_count));
                ui.separator();
                ui.label(RichText::new(&app.status_msg).weak().italics());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let secs = app.engine.elapsed_secs() as i64;
                    ui.label(
                        RichText::new(format!("Laufzeit {:02}:{:02}", secs / 60, secs % 60))
                            .monospace()
                            .weak(),
                    );
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
        if ui
            .selectable_label(app.follow_live, "Live")
            .on_hover_text("Dem aktuellen Zeitfenster folgen")
            .clicked()
        {
            app.follow_live = true;
        }
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
        for t in &app.targets {
            let c = app.trace_color(t);
            if ui
                .add(Label::new(RichText::new(&t.label).color(c).strong()).selectable(false))
                .on_hover_text(format!("{} · {}", t.host, app.status_text(t)))
                .clicked()
            {
                if let Some(i) = app.targets.iter().position(|x| x.id == t.id) {
                    app.selected = i;
                }
            }
        }
    });

    let (rect, resp) = ui.allocate_at_least(
        Vec2::new(
            ui.available_width(),
            (ui.available_height() - 150.0).max(160.0),
        ),
        egui::Sense::click_and_drag(),
    );
    if resp.dragged() {
        app.follow_live = false;
        app.pan_offset_secs += (resp.drag_delta().x as f64) * (horizon / rect.width() as f64);
        if app.pan_offset_secs < 0.0 {
            app.pan_offset_secs = 0.0;
        }
    }

    // The p95 line sits at the 95th percentile of the replies inside the visible window, so
    // it follows the trace instead of being pushed off by one long spike. The scan goes into
    // a scratch buffer on the app, which keeps this allocation-free per frame.
    let mut t_max = now - app.pan_offset_secs;
    let mut t_min = t_max - horizon;
    // In a still picture the trace has to end at the right edge, like it does while the
    // probes are running. The example history stops at its own last stamp, so the window is
    // pulled back to that point instead of leaving empty space on the right.
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
                t_min = t_max - horizon;
            }
        }
    }

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
        let (cut, _n) = history::outlier_cut(t.series.samples());
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
                    let outlier = (s.rtt_ms as f64) > history::outlier_cut(t.series.samples()).0;
                    rows.push((
                        format!(
                            "{}  {}{}",
                            t.label,
                            if history::is_ok(s.rtt_ms) {
                                format!("{:.1} ms", s.rtt_ms)
                            } else {
                                "Timeout".into()
                            },
                            if outlier { "  <- Ausr." } else { "" }
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
            let pos = egui::pos2(
                (p.x + 10.0).min(rect.right() - gal.size().x - 6.0),
                rect.top() + 6.0,
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
    ui.horizontal(|ui| {
        ui.label(RichText::new("Ereignisse").strong());
        ui.label(RichText::new(format!("({})", app.log.len())).weak());
        if ui.small_button("leeren").clicked() {
            app.log.clear();
            app.log_total = 0;
        }
    });
    ScrollArea::vertical()
        .max_height(120.0)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for e in &app.log {
                let color = m.log_kind(e.kind);
                ui.add(
                    Label::new(
                        RichText::new(format!("[{:>8}] {}", fmt_clock(e.secs), e.text))
                            .color(color)
                            .monospace()
                            .size(11.5),
                    )
                    .selectable(false),
                );
            }
        });
}

/// Column widths for the statistics grid: Ziel, Host, Status, Min, Ø, Median,
/// p95, Max, Verlust, Ausr.
fn column_widths() -> [f32; 10] {
    [150.0, 130.0, 84.0, 64.0, 64.0, 72.0, 64.0, 64.0, 72.0, 56.0]
}

fn is_shown(app: &App, t: &crate::app::Target) -> bool {
    match app.view {
        View::Chart => true,
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

fn fmt_clock(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/* ------------------------------------------------------------------- list */

fn events_view(ui: &mut Ui, app: &mut App) {
    let m = app.theme_mode;
    ui.label(RichText::new("Statusliste").strong());
    ui.add_space(4.0);
    let now = app.engine.elapsed_secs();
    let horizon = app.options.horizon_secs as f64;
    // Taken out of the app so it can be reused while `app.targets` is borrowed.
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for idx in 0..app.targets.len() {
                let color = app.status_color(&app.targets[idx]);
                let trace = app.trace_color(&app.targets[idx]);
                let txt = app.status_text(&app.targets[idx]);
                let st = app.targets[idx].series.stats_window(
                    app.targets[idx]
                        .series
                        .last()
                        .map_or(0.0, |s| s.t_secs - horizon),
                    f64::MAX,
                    &mut scratch,
                );
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
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 6.0, color);
                        // Strich in Linienfarbe, wie in der Zielliste links.
                        let (line_rect, _) =
                            ui.allocate_exact_size(Vec2::new(18.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(line_rect.shrink(5.0), 2.0, trace);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(
                                Label::new(
                                    RichText::new(format!("{}   ({})", t.label, t.host)).strong(),
                                )
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
}

fn draw_sparkline(ui: &Ui, series: &history::Series, rect: egui::Rect, color: Color32) {
    let m = Mode::from_dark(ui.visuals().dark_mode);
    let samples = series.samples();
    if samples.is_empty() {
        return;
    }
    let t0 = samples[0].t_secs;
    let t1 = samples.last().map(|s| s.t_secs).unwrap_or(t0 + 1.0);
    let mut y_max = 40.0f32;
    for s in samples {
        if history::is_ok(s.rtt_ms) {
            y_max = y_max.max(s.rtt_ms);
        }
    }
    let painter = ui.painter();
    // Which samples count as outliers is decided over the whole stored series, so
    // the marks match what the statistics list as an outlier.
    let (cut, _n) = history::outlier_cut(samples);

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

fn stats_view(ui: &mut Ui, app: &mut App) {
    let m = app.theme_mode;
    ui.label(RichText::new("Statistiken").strong());
    ui.add_space(6.0);

    // The visible targets are looked up once, so the row count and the row lookup agree.
    let ids: Vec<usize> = app.visible_targets();
    let horizon = app.options.horizon_secs as f64;
    // Taken out of the app so it can be reused while `app.targets` is borrowed.
    let mut scratch = std::mem::take(&mut app.stats_scratch);
    let mut select: Option<usize> = None;

    // Plain grid instead of egui_extras' table: the look stays the same (header row,
    // one row per target) without pulling in another crate.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        for c in [
            "Ziel", "Host", "Status", "Min", "Ø", "Median", "p95", "Max", "Verlust", "Ausr.",
        ] {
            ui.add_sized(
                [110.0, 20.0],
                Label::new(RichText::new(c).strong().size(12.0)).selectable(false),
            );
        }
    });
    ui.separator();
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for &idx in &ids {
                let t = &app.targets[idx];
                let st = t.series.stats_window(
                    t.series.last().map_or(0.0, |s| s.t_secs - horizon),
                    f64::MAX,
                    &mut scratch,
                );
                let color = app.status_color(t);
                let trace = app.trace_color(t);
                let txt = app.status_text(t);
                let sel = app.selected == idx;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let widths = column_widths();
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
                        Label::new(RichText::new(&t.host).monospace().size(12.0)).selectable(false),
                    );
                    ui.add_sized(
                        [widths[2], 20.0],
                        Label::new(RichText::new(&txt).color(color).size(12.0)).selectable(false),
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
    app.stats_scratch = scratch;

    if let Some(idx) = select {
        app.selected = idx;
    }

    ui.add_space(10.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Gesamt").strong());
        let mut total = 0usize;
        let mut replies = 0usize;
        let mut sum = 0f64;
        let mut max = 0f32;
        let mut outliers = 0usize;
        // Taken out of the app so it can be reused while `app.targets` is borrowed.
        let mut scratch = std::mem::take(&mut app.stats_scratch);
        for t in &app.targets {
            let s = t.series.stats_window(
                t.series.last().map_or(0.0, |s| s.t_secs - horizon),
                f64::MAX,
                &mut scratch,
            );
            total += s.total;
            replies += s.replies;
            sum += s.sum;
            max = max.max(s.max);
            outliers += s.outliers;
        }
        let avg = if replies > 0 {
            sum / replies as f64
        } else {
            0.0
        };
        let loss = if total > 0 {
            100.0 * (total - replies) as f64 / total as f64
        } else {
            0.0
        };
        ui.label(format!("{} Prüfungen", total));
        ui.label(format!("Ø {}", crate::app::fmt_ms(avg as f32)));
        ui.label(format!("max {}", crate::app::fmt_ms(max)));
        ui.label(format!("Verlust {:.1} %", loss));
        ui.label(format!("{} Auffälligkeiten", outliers));
        app.stats_scratch = scratch;
    });
}

/* --------------------------------------------------------------- settings */

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
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.group(|ui| {
            ui.label(RichText::new("Prüfung").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Intervall");
                if ui.add(egui::Slider::new(&mut app.options.interval_secs, 0.25..=10.0).suffix(" s")).changed() { dirty = true; }
            });
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
            ui.label(RichText::new("Benachrichtigung & Start").strong());
            ui.add_space(4.0);
            // Three settings are stored but have no effect yet: the program plays no sound and
            // has no tray icon, so the boxes are shown disabled instead of promising a feature
            // that is not there. The options stay in the file for whoever implements them.
            ui.add_enabled_ui(false, |ui| {
                ui.checkbox(&mut app.options.sound_alerts, "Ton bei Ausfall / Wiederherstellung");
                ui.checkbox(&mut app.options.system_tray_icon, "Symbol im Infobereich");
                ui.checkbox(&mut app.options.minimised_to_tray, "Minimiert starten");
            });
            ui.label(
                RichText::new("Noch nicht umgesetzt: Die Optionen werden gespeichert, haben aber keine Wirkung.")
                    .weak()
                    .size(11.5),
            );
            ui.add_space(4.0);
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
            ui.label(RichText::new("Ziel").strong());
            if app.selected < app.targets.len() {
                let t = &app.targets[app.selected];
                ui.label(format!("{} — {}", t.label, t.host));
                ui.label(format!("Art: {}", t.kind.label()));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Prüfung zurücksetzen").clicked() {
                        app.reset_stats();
                    }
                    if ui.button("Ziel entfernen").clicked() {
                        // Removing a target would shift the index the inline editor points at, so
                        // any open editor is committed first.
                        app.close_edit();
                        app.remove_selected();
                    }
                });
            } else {
                ui.label(RichText::new("Kein Ziel ausgewählt.").weak());
            }
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.label(RichText::new("Daten").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("CSV exportieren").clicked() {
                    let path = default_export_path();
                    app.export_csv(path);
                }
                if ui.button("Speichern").clicked() {
                    app.save_now();
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
            ui.horizontal(|ui| {
                ui.label(RichText::new("Vorschläge:").strong());
                for (host, label) in [("8.8.8.8", "Google DNS"), ("1.1.1.1", "Cloudflare"), ("9.9.9.9", "Quad9")] {
                    if ui.small_button(label).clicked() {
                        app.add_named_target(host, label);
                    }
                }
            });
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

pub fn default_export_path() -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(format!("system-ping-{}.csv", stamp))
}
