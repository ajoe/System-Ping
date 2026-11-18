//! The chart's two ways of painting: a line, or a dot per measurement whose size says how far the
//! value stands from its target's average.
//!
//! The view is what a chart is for, and this one has two jobs that pull in opposite directions: a
//! line shows how latency *develops*, while the rare measurement that sticks out — the actual
//! problem with a connection — is exactly what a line flattens into the crowd of the boring values
//! around it. So there are two modes, and this file paints the real chart in both of them and looks
//! at what reaches the draw list: the line mode has to draw a curve and a ring per outlier, the
//! dots mode has to draw no curve and circles of many different sizes, the small ones where the
//! values sit at their average, the largest where one stands far away.

use egui::{Context, Pos2, Rect, Shape, Vec2};
use std::sync::Mutex;
use system_ping::app::{App, AppOptions, ChartMode};
use system_ping::{headless, theme, ui};

/// Only one process-wide state folder can be set at a time (`SYSTEM_PING_STATE_DIR`), so two
/// fixtures in this binary would read each other's targets. See `tests/totals.rs`.
static STATE_DIR_LOCK: Mutex<()> = Mutex::new(());

/// An app with its own empty state folder and its own targets. The lock is held until the test
/// ends (`_guard` therefore has to be the last field).
struct Fixture {
    app: App,
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::env::var_os("SYSTEM_PING_STATE_DIR").as_deref() == Some(self.dir.as_os_str()) {
            std::env::remove_var("SYSTEM_PING_STATE_DIR");
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An app in a 60-second chart window, with one target whose series is filled here instead of
/// measured: `Target::new` is deliberately not public, so an existing row is rewritten.
fn fixture(tag: &str) -> Fixture {
    let guard = STATE_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("sp-chart-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);

    let opts = AppOptions {
        horizon_secs: 60.0,
        ..AppOptions::default()
    };
    let mut app = App::with_theme_override(opts, Vec::new(), Some("dark"));
    let t = &mut app.targets[0];
    t.label = "Ziel".into();
    t.host = "10.0.0.1".into();
    t.hidden = false;
    t.enabled = true;
    t.series = system_ping::history::Series::new(1024);
    app.targets.truncate(1);
    Fixture {
        app,
        dir,
        _guard: guard,
    }
}

/// One series that says everything the two views have to disagree about: 40 replies around 10 ms
/// and one at 200 ms. The 10 ms values are the uninteresting majority — an average has already
/// said what they mean — the 200 ms reply is the reason anyone opens this program.
///
/// The timestamps run up to the moment the program started, which is where the chart's window ends.
fn record(app: &mut App) {
    for k in 0..40i64 {
        let t = -59.0 + k as f64;
        app.targets[0]
            .series
            .record(t, (t * 1000.0) as i64, 10.0, None);
    }
    app.targets[0].series.record(-20.0, -20_000, 200.0, None);
    // The newest sample at the same value as the crowd: its marker is the only filled circle the
    // line view draws at all, so it has to stay small in both views.
    app.targets[0].series.record(-1.0, -1_000, 10.0, None);
}

/// Everything the chart painted: the filled circles it drew, the curves, and the words of the
/// heading.
struct Picture {
    filled: Vec<f32>,
    rings: usize,
    segments: usize,
    words: Vec<String>,
}

/// Paints the interface once with the given mode and collects what the draw list received.
fn paint(app: &mut App, mode: ChartMode) -> Picture {
    app.options.chart_mode = mode;
    let ctx = Context::default();
    theme::apply(&ctx, app.theme_mode);
    ctx.set_pixels_per_point(1.0);
    let surface = Rect::from_min_size(Pos2::ZERO, Vec2::new(1080.0, 700.0));

    let mut picture = Picture {
        filled: Vec::new(),
        rings: 0,
        segments: 0,
        words: Vec::new(),
    };
    // The chart needs one full pass before it knows its own plot rectangle, and egui's animation
    // clock insists on moving forward between passes (see `tests/shot_text.rs`).
    for i in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(surface),
            time: Some(i as f64 * 0.016),
            predicted_dt: 0.016,
            ..Default::default()
        };
        let out = headless::paint(&ctx, raw, |ui| {
            ui::top(ui, app);
            ui::center_ex(ui, app, false);
        });
        for clipped in &out.shapes {
            match &clipped.shape {
                Shape::Circle(c) => {
                    if c.fill.a() > 0 {
                        picture.filled.push(c.radius);
                    } else if c.stroke.width > 0.0 {
                        picture.rings += 1;
                    }
                }
                Shape::LineSegment { .. } => picture.segments += 1,
                Shape::Text(t) => picture.words.push(t.galley.job.text.clone()),
                _ => {}
            }
        }
    }
    picture
}

/// The dots view replaces the curve, and with it the size the curve gave every measurement: the
/// majority sits on its target's average and has to become a speck, the single far value has to
/// become the biggest circle the view draws. The line view knows no such sizes.
#[test]
fn punkte_groessen_sich_nach_dem_abstand_vom_durchschnitt() {
    let mut f = fixture("sizes");
    record(&mut f.app);

    let dots = paint(&mut f.app, ChartMode::Points);
    let crowd = dots.filled.iter().filter(|r| **r < 1.3).count();
    let biggest = dots.filled.iter().copied().fold(0.0_f32, f32::max);
    println!(
        "dots: {} circles, {} of them specks, biggest {biggest}",
        dots.filled.len(),
        crowd
    );
    assert!(
        crowd >= 20,
        "{crowd} of {} circles are specks — the 41 replies at the target's average have to be \
         drawn small, that is the whole idea",
        dots.filled.len()
    );
    assert!(
        biggest > 4.4,
        "biggest dot {biggest}: the 200 ms reply is far outside its target's spread and has to be \
         drawn with the largest size the view has"
    );

    let lines = paint(&mut f.app, ChartMode::Lines);
    let biggest_line = lines.filled.iter().copied().fold(0.0_f32, f32::max);
    println!(
        "lines: {} filled circles, biggest {biggest_line}",
        lines.filled.len()
    );
    assert!(
        biggest_line < 4.0,
        "biggest filled circle in the line view is {biggest_line}: without the dots view there is \
         nothing that grows with a distance"
    );
    assert!(
        lines.filled.iter().all(|r| *r > 1.3),
        "the line view marks single measurements with fixed sizes only"
    );
}

/// In the dots view nothing connects the measurements: a line between them would say that the
/// values in between matter, which is the opposite of what this view is for. The outlier ring the
/// line view draws around the far value belongs to the curve and is gone as well — the size and
/// the outlier colour carry the mark there.
#[test]
fn punkte_zeichnen_keine_kurve() {
    let mut f = fixture("curve");
    record(&mut f.app);

    let lines = paint(&mut f.app, ChartMode::Lines);
    let dots = paint(&mut f.app, ChartMode::Points);
    println!(
        "segments lines {} / dots {}, rings lines {} / dots {}",
        lines.segments, dots.segments, lines.rings, dots.rings
    );
    assert!(
        lines.segments >= dots.segments + 40,
        "the line view draws {} line segments, the dots view {}: the trace alone is 41 \
         measurements, so a curve has to be visible in the first number only",
        lines.segments,
        dots.segments
    );
    assert!(
        lines.rings >= 1,
        "the far reply is an outlier by the same rule the statistics use and has to be ringed in \
         the line view"
    );
    assert_eq!(
        dots.rings, 0,
        "the dots view marks an outlier by size and colour, not by a second mark"
    );
}

/// Both modes have to be reachable from the chart's heading, otherwise the setting is only in the
/// state file and nobody finds it.
#[test]
fn beide_moden_sind_im_kopf_des_diagramms() {
    let mut f = fixture("switch");
    record(&mut f.app);
    for mode in [ChartMode::Lines, ChartMode::Points] {
        let p = paint(&mut f.app, mode);
        for want in ["Linien", "Punkte"] {
            assert!(
                p.words.iter().any(|t| t == want),
                "`{want}` is missing from the heading in {mode:?}: {:#?}",
                p.words
            );
        }
    }
}
