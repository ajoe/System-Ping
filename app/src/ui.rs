//! The whole interface: top bar, target list, the four views, event log, status bar.

use crate::app::{App, View};
use crate::history;
use crate::theme::{self, Mode};
use egui::{Color32, FontId, Label, RichText, ScrollArea, Stroke, Ui, Vec2};
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
                if (enter && !(host_buf.is_empty() && label_buf.is_empty()))
                    || ui.button("+ Ziel").clicked()
                {
                    add = true;
                }
                if add {
                    app.new_host = host_buf;
                    app.new_label = label_buf;
                    app.add_target();
                    app.new_host.clear();
                    app.new_label.clear();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !app.running {
                        if ui
                            .button(RichText::new("Ziel entfernen").color(m.danger_text()))
                            .on_hover_text("Entfernt das in der Liste markierte Ziel")
                            .clicked()
                        {
                            app.remove_selected();
                        }
                        if ui
                            .button(RichText::new(
                                // `autostart_state()` is the registry's answer (looked up at
                                // startup, then kept current); the raw counter alone would be
                                // „aus“ until the user happens to press this button.
                                if app.autostart_state() == Some(true) {
                                    "Autostart: an"
                                } else {
                                    "Autostart: aus"
                                },
                            ))
                            .on_hover_text("Beim Anmelden von Windows starten")
                            .clicked()
                        {
                            let on = app.autostart_state() != Some(true);
                            app.set_autostart(on);
                        }
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

pub fn left(ctx: &egui::Context, app: &mut App) {
    let m = app.theme_mode;
    egui::SidePanel::left("targets")
        .exact_width(250.0)
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Ziele").strong());
                let vis = app.visible_targets().len();
                ui.label(RichText::new(format!("{}/{}", vis, app.targets.len())).weak());
            });
            ui.separator();

            let mut select: Option<usize> = None;
            let now = app.engine.elapsed_secs();

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (idx, t) in app.targets.iter().enumerate() {
                        let color = app.status_color(t);
                        let txt = app.status_text(t);
                        let sel = app.selected == idx;
                        let secs_ago = if t.last_probe_secs > 0.0 {
                            now - t.last_probe_secs
                        } else {
                            -1.0
                        };

                        let frame = egui::Frame::new()
                            .fill(m.row_fill(sel))
                            .corner_radius(5.0)
                            .inner_margin(egui::Margin::symmetric(7, 5));
                        let resp = frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (dot_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(11.0), egui::Sense::hover());
                                let dot_rect = dot_rect.shrink(1.5);
                                ui.painter().rect_filled(dot_rect, 5.5, color);

                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.add(
                                        Label::new(RichText::new(&t.label).strong().size(13.0))
                                            .selectable(false),
                                    );
                                    let age = if secs_ago < 0.0 {
                                        "noch nicht geprüft".to_string()
                                    } else {
                                        format!("vor {} s", secs_ago.max(0.0) as i64)
                                    };
                                    ui.add(
                                        Label::new(
                                            RichText::new(format!("{} · {}", t.host, age))
                                                .weak()
                                                .size(11.0),
                                        )
                                        .selectable(false),
                                    );
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.add(
                                            Label::new(
                                                RichText::new(&txt)
                                                    .color(color)
                                                    .monospace()
                                                    .size(12.0),
                                            )
                                            .selectable(false),
                                        );
                                    },
                                );
                            });
                        });

                        if resp.response.clicked() {
                            select = Some(idx);
                        }
                    }
                });

            if let Some(idx) = select {
                app.selected = idx;
            }
        });
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
            let c = app.status_color(t);
            let sel = app.targets.get(app.selected).map(|x| x.id) == Some(t.id);
            if ui
                .add(
                    Label::new(
                        RichText::new(&t.label)
                            .color(if sel { Color32::WHITE } else { c })
                            .strong(),
                    )
                    .selectable(false),
                )
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
    let mut gt = (t_min / step).ceil() * step;
    while gt <= t_max {
        let x = map_x(gt, t_min, t_max, rect);
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, m.grid()),
        );
        ui.painter().text(
            egui::pos2(x, rect.bottom() + 3.0),
            egui::Align2::LEFT_TOP,
            fmt_clock(gt),
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
        // A line has to be visible before the first reply arrives, so its colour comes from
        // the latency band of the sample being drawn, not from `last_rtt` (which is still
        // "waiting" while a target is cold). The selected trace gets a contrasting outline
        // so it stays readable whichever band it falls in.
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
                // Band colour per segment; a selected trace gets a thin outline in the
                // text colour so a green-on-green or grey-on-grey stretch stays visible.
                ui.painter().line_segment(
                    [egui::pos2(px, py), egui::pos2(x, y)],
                    Stroke::new(width, m.rtt_band(s.rtt_ms)),
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
        // "when did this target go quiet" stays readable. It takes the band colour of the
        // sample it marks, like the line itself.
        if let Some(s) = t.series.samples().last() {
            if s.t_secs >= t_min && s.t_secs < t_max && history::is_ok(s.rtt_ms) {
                let x = map_x(s.t_secs, t_min, t_max, rect);
                let y = map_y(s.rtt_ms.min(scale_top), scale_top, rect);
                ui.painter().circle_filled(
                    egui::pos2(x, y),
                    if sel { 3.4 } else { 2.4 },
                    m.rtt_band(s.rtt_ms),
                );
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
            let mut lines = vec![format!("{} s", t_at.round() as i64)];
            for t in &app.targets {
                if !is_shown(app, t) {
                    continue;
                }
                if let Some(s) = nearest(t, t_at) {
                    let outlier = (s.rtt_ms as f64) > history::outlier_cut(t.series.samples()).0;
                    lines.push(format!(
                        "{}  {}{}",
                        t.label,
                        if history::is_ok(s.rtt_ms) {
                            format!("{:.1} ms", s.rtt_ms)
                        } else {
                            "Timeout".into()
                        },
                        if outlier { "  <- Ausr." } else { "" }
                    ));
                }
            }
            let gal =
                ui.painter()
                    .layout_no_wrap(lines.join("\n"), FontId::monospace(11.0), m.text());
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

fn nice_step(horizon: f64) -> f64 {
    for s in [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0] {
        if horizon / s <= 12.0 {
            return s;
        }
    }
    900.0
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
                    draw_sparkline(ui, &t.series, bar, color);
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
                let txt = app.status_text(t);
                let sel = app.selected == idx;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let widths = column_widths();
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
                if ui.add(egui::Slider::new(&mut app.options.horizon_secs, 15.0..=900.0).logarithmic(true).suffix(" s")).changed() { dirty = true; }
            });
            ui.horizontal(|ui| {
                ui.label("Max. Messwerte je Ziel");
                if ui.add(egui::DragValue::new(&mut app.options.max_samples).range(128..=65_536).speed(64)).changed() { dirty = true; }
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
