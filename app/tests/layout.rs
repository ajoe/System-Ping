//! Nothing in a painted frame may lie on top of something else, or outside the window.
//!
//! Every one of these was a real bug, and every one of them only showed up at some window width or
//! in some clock situation: the chart's legend row grew past the right edge and took the plot's axis
//! labels with it, the axis labels were painted into a band the layout never reserved, the last label
//! of the time axis hung over the window's edge, a target row's second line ran under its own value,
//! and a window reaching back before the session started printed the same „00:00" on every tick.
//! Reading the painted text back out of egui's output is what makes that testable — a widget's own
//! rect says nothing about the text it paints outside of itself.

use egui::{Context, FontId, Pos2, Rect, Shape, Vec2};
use system_ping::app::{App, AppOptions, LogKind};
use system_ping::{headless, history, runtime, theme, ui};

/// The window widths this is looked at: the default, the one the app starts with, and one under the
/// width the top bar needs for all its controls — which is where things start to wrap.
const SIZES: [(f32, f32); 4] = [
    (1440.0, 900.0),
    (1080.0, 700.0),
    (900.0, 620.0),
    (780.0, 560.0),
];

/// A frame with something in it: rows with a name, a host and a value, a chart with axis labels and
/// a legend, an event log with entries. Empty, the rows would show a single dash and the chart
/// would have no ticks at all, and both are exactly where the collisions were.
fn populated_app() -> App {
    let mut app = App::new(AppOptions::default(), Vec::new());
    app.add_named_target("fritzbox.fritz.box", "FritzBox 6890 LTE");
    app.add_named_target("fritzrepeater.fritz.box", "Fritz-Repeater-3000AX");
    app.add_named_target("192.168.1.250", "Switch unten");
    // Stamped the way a loaded history is: the samples span the window that is about to be shown,
    // counted from the start of the session (see `Screencap::seed`, which stamps a picture the same
    // way). A minute-wide window in a process that has just started reaches back before the start,
    // and that is the case where every tick used to read the same.
    let now = runtime::secs_since_start() + 1.92;
    for (i, t) in app.targets.iter_mut().enumerate() {
        for k in 0..30 {
            t.series.push(history::Sample {
                t_secs: now - 60.0 + k as f64 * 2.0,
                rtt_ms: 1234.0 + i as f32,
                fail: None,
                at_ms: 1_700_000_000_000 + (now * 1_000.0) as i64,
            });
        }
        t.last_probe_secs = now - 2.4;
        t.last_rtt = 1234.0 + i as f32;
    }
    app.log_event(
        LogKind::Outage,
        "fritzrepeater.fritz.box ist nicht erreichbar",
    );
    app
}

/// Where the words of one painted frame stand: the text, its font, and the rect it was painted into.
fn painted_words(width: f32, height: f32) -> Vec<(String, FontId, Rect)> {
    let mut app = populated_app();
    let ctx = Context::default();
    theme::apply(&ctx, app.theme_mode);
    ctx.set_pixels_per_point(1.0);
    let surface = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height));
    let mut words = Vec::new();
    for i in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(surface),
            // egui's animation clock has to move forward between passes (see `shot_text`).
            time: Some(i as f64 * 0.016),
            predicted_dt: 0.016,
            ..Default::default()
        };
        // The shot frame, so the example history comes through: `chart` only shows samples the
        // running engine has measured, and a test has no engine.
        let out = headless::paint(&ctx, raw, |root| {
            ui::top(root, &mut app);
            ui::left(root, &mut app);
            ui::center_ex(root, &mut app, true);
            ui::bottom(root, &mut app);
        });
        words.clear();
        if i == 1 {
            for clipped in &out.shapes {
                if let Shape::Text(t) = &clipped.shape {
                    // Galley rect relative to the paint position. Empty galley rects (a label with no
                    // text) carry nothing that could overlap.
                    let r = Rect::from_min_max(
                        t.pos + t.galley.rect.min.to_vec2(),
                        t.pos + t.galley.rect.max.to_vec2(),
                    );
                    if !r.is_negative() {
                        // The font the job was laid out in, taken from its first section: the
                        // axis rows are the only thing painted in 10 px monospace.
                        let font = t
                            .galley
                            .job
                            .sections
                            .first()
                            .map(|s| s.format.font_id.clone())
                            .unwrap_or_else(|| FontId::proportional(0.0));
                        words.push((t.galley.job.text.clone(), font, r));
                    }
                }
            }
        }
    }
    words
}

/// A pair worth complaining about: two texts with real area, sharing more than a point of each.
fn collides(a: &(String, FontId, Rect), b: &(String, FontId, Rect)) -> Option<Rect> {
    let over = a.2.intersect(b.2);
    (over.width() > 1.0 && over.height() > 1.0).then_some(over)
}

#[test]
fn no_text_is_painted_on_top_of_other_text() {
    for (width, height) in SIZES {
        let words = painted_words(width, height);
        assert!(
            words.len() > 40,
            "frame at {width} painted too little: {}",
            words.len()
        );
        for (i, a) in words.iter().enumerate() {
            for b in words.iter().skip(i + 1) {
                if let Some(over) = collides(a, b) {
                    panic!(
                        "{width}x{height}: {:?} [{:.0},{:.0}..{:.0},{:.0}] lies on {:?} \
                         [{:.0},{:.0}..{:.0},{:.0}] ({:.0}x{:.0} at {:.0},{:.0})",
                        a.0,
                        a.2.min.x,
                        a.2.min.y,
                        a.2.max.x,
                        a.2.max.y,
                        b.0,
                        b.2.min.x,
                        b.2.min.y,
                        b.2.max.x,
                        b.2.max.y,
                        over.width(),
                        over.height(),
                        over.min.x,
                        over.min.y,
                    );
                }
            }
        }
    }
}

#[test]
fn no_text_is_painted_outside_the_window() {
    for (width, height) in SIZES {
        let words = painted_words(width, height);
        for (text, _, r) in &words {
            // A whole point of slack: text is laid out on pixels, and a glyph's box may round out.
            if !Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))
                .expand(1.0)
                .contains_rect(*r)
            {
                panic!(
                    "{width}x{height}: {text:?} [{:.0},{:.0}..{:.0},{:.0}] is outside the window",
                    r.min.x, r.min.y, r.max.x, r.max.y
                );
            }
        }
    }
}

/// One row of the time axis — same line, same font — has to say something different at every tick.
/// A row that repeats one label is not a scale but a stopped clock, which is what a window reaching
/// back before the session start used to produce.
#[test]
fn an_axis_row_does_not_repeat_its_label() {
    for (width, height) in SIZES {
        let words = painted_words(width, height);
        let mut rows: std::collections::BTreeMap<i64, Vec<String>> = Default::default();
        for (text, font, r) in &words {
            if *font == FontId::monospace(10.0) {
                rows.entry((r.min.y * 2.0) as i64)
                    .or_default()
                    .push(text.clone());
            }
        }
        for (row, labels) in rows {
            if labels.len() < 3 {
                continue; // one or two labels of a kind: nothing repeats by itself
            }
            let distinct: std::collections::HashSet<&String> = labels.iter().collect();
            assert_eq!(
                distinct.len(),
                labels.len(),
                "{width}x{height}: axis row {row} repeats itself: {labels:?}"
            );
        }
    }
}
