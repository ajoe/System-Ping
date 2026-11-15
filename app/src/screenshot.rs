//! Screenshot mode: run the real interface for a fixed number of frames, then save a PNG.
//!
//! eframe gives no public way to read back what it drew, so the frame is rasterised
//! from egui's own draw list (`ClippedPrimitive`s): filled and stroked rectangles,
//! lines and text galley pixels. No engine internals are touched.

use crate::app::{App, LogKind, View};
use crate::theme;
use crate::ui;
use eframe::CreationContext;
use egui::epaint::ClippedPrimitive;
use egui::{Color32, Pos2, Rect, Vec2};
use image::RgbaImage;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Set once a `--shot` run has started: the app must not probe while a still picture is
/// being built, or live samples land on top of the seeded history.
static CAPTURE_ARMED: AtomicBool = AtomicBool::new(false);

pub fn capture_armed() -> bool {
    CAPTURE_ARMED.load(Ordering::Relaxed)
}

pub struct Screencap {
    app: App,
    path: PathBuf,
    view: Option<String>,
    frames_left: u32,
    done: bool,
    seeded: usize,
    /// The last fully drawn frame, taken from egui's own output.
    shapes: Vec<ClippedPrimitive>,
    /// Our own monotone clock for egui; see `update`. The largest time handed to egui so far,
    /// in seconds. Kept so a stalled wall clock can be detected; unused for now.
    #[allow(dead_code)]
    last_egui_time: f64,
    clock: f64,
    /// 1 = the seeded example history stands in for a live trace (see `ui::chart`).
    pub shot_frame: bool,
}

impl Screencap {
    pub fn new(app: App, cc: &CreationContext<'_>, path: PathBuf, view: Option<String>) -> Self {
        // From here on the app must not probe (see `capture_armed`).
        CAPTURE_ARMED.store(true, Ordering::Relaxed);
        theme::apply(&cc.egui_ctx, app.theme_mode);
        // Fixed surface so the shot is reproducible.
        cc.egui_ctx.set_pixels_per_point(1.0);
        // Deterministic picture: the seeded trace is the render, so no live probes may add
        // samples on top of it (that raced with `seed` and made renders differ per run).
        app.engine.stop_all();
        Self {
            app,
            path,
            view,
            frames_left: 120,
            done: false,
            seeded: 0,
            shapes: Vec::new(),
            shot_frame: false,
            last_egui_time: 0.0,
            clock: 0.0,
        }
    }

    /// Feed the app some history so the picture shows a working monitor.
    /// Returns how many samples were planted (0 = no targets, nothing to draw).
    fn seed(&mut self) -> usize {
        use crate::history::{Fail, TIMEOUT};
        let n = self.app.targets.len();
        if n == 0 {
            return 0;
        }
        let horizon = self.app.options.horizon_secs as f64;
        // One clock for the whole program (`runtime`), so a stamped sample and the view's
        // "now" are on the same axis. Capture happens after the shot's frame loop
        // (120 frames, ~1.92 s); stamping backwards from that fills the whole window.
        let base_t = crate::runtime::secs_since_start() + 1.92;
        // Absolute counterpart of `base_t`, so the seeded samples carry a real time too.
        let now_ms = chrono::Utc::now().timestamp_millis() + 1920;
        // A believable trace: a clean baseline, one latency spike per target (kept well
        // below the chart's own y-scaling so the outlier rings land inside the plot), and
        // an outage streak on the second target. The last sample sits just under "now" so
        // every target keeps its newest-value dot in the live window.
        let n = 110usize;
        // Samples must span the whole visible window: `horizon / n` seconds apart, with the
        // newest one a step before "now". A fixed spacing (2 s) would only cover part of a
        // wider window and leave the right side of the plot empty.
        let step = horizon / n as f64;
        for (i, t) in self.app.targets.iter_mut().enumerate() {
            // Keep the seeded history inside the configured window.
            t.series.set_cap(self.app.options.max_samples);
            for k in 0..n {
                let base = match i {
                    0 => 2.4,
                    1 => 13.0,
                    2 => 27.0,
                    _ => 45.0,
                } + (i as f32 * 1.7);
                let jitter = (((k as f32 * 12.9898 + i as f32 * 78.233).sin()) * 43_758.547)
                    .fract()
                    * base
                    * 0.45;
                let rtt = if i == 1 && (62..=71).contains(&k) {
                    TIMEOUT
                } else if k % 37 == 0 {
                    base * 2.4 + 25.0
                } else {
                    base + jitter
                };
                t.series.record(
                    base_t - (n - k) as f64 * step,
                    // The demo history is synthetic, so its absolute time is derived from the
                    // same relative stamps the chart uses.
                    now_ms - ((n - k) as i64) * (step * 1000.0) as i64,
                    rtt,
                    if rtt >= TIMEOUT {
                        Some(Fail::Timeout)
                    } else {
                        None
                    },
                );
            }
            let last = t.series.samples().last().map(|s| s.rtt_ms).unwrap_or(0.0);
            t.last_rtt = last;
            t.last_probe_secs = base_t - step;
        }
        self.app.log_info("Beispielverlauf geladen (Prüfbild)");
        self.app.alert_count += 1;
        self.app.log_event(
            LogKind::Outage,
            format!("Ausfall: {}", self.app.targets[1].label),
        );
        self.app.log_event(
            LogKind::Recovery,
            format!("Wieder online: {}", self.app.targets[1].label),
        );
        n * self.app.targets.len()
    }

    /// Show one of the four views in the shot.
    fn set_view(&mut self, name: &str) {
        if let Some(v) = view_for(name) {
            self.app.view = v;
        }
    }
}

impl eframe::App for Screencap {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.frames_left == 120 {
            // Seeding happens in this same pass, so the history is stamped on the clock
            // that will be current at capture (see `seed`).
            self.seeded = self.seed();
            if let Some(v) = self.view.take() {
                self.set_view(&v);
            }
        }

        // Draw inside a real egui output pass: `tessellate` only sees the last frame that
        // finished, and with a hidden window there is no vsync to force one.
        //
        // The picture pass runs inside ONE `Context::run`, and the capture happens in that
        // same pass. Earlier versions ran ~120 passes through eframe and kept the last
        // tessellated frame; egui then asserted "Time shouldn't move backwards" inside a value
        // history, because `InputState::begin_pass` derives its clock as
        // `new.time.unwrap_or(self.time + new.predicted_dt as f64)` — with `time: None` and
        // `predicted_dt: 0.0` every pass was stamped with the very same instant, so any value
        // history that egui itself keeps (drag/animation machinery) hit the monotonicity
        // assert on the second pass. So every pass below carries its own stamp plus a
        // predicted_dt greater than zero, which keeps that clock strictly increasing.
        if self.frames_left == 120 {
            self.shot_frame = true;
        }

        // Warm-up pass, then the picture pass — two passes, not one and not 120.
        //
        // Why not one: a first pass on a brand-new `Context` lays out text against a font
        // atlas that egui only uploads while preparing output, so the tessellated frame of
        // pass 1 is missing most of its glyphs. Measured with the same scan as the density
        // table: one-pass capture gave 5,50 % lit pixels on the chart view where the old
        // (crashing) multi-pass capture had 8,74 %.
        //
        // Why not many: every pass needs a later clock than the one before it. egui derives
        // its animation clock in `InputState::begin_pass` as
        // `new.time.unwrap_or(self.time + new.predicted_dt)`, so identical stamps make its
        // own value histories assert ("Time shouldn't move backwards"). Passing our own
        // increasing instant into `raw.time` is not enough on its own — the same instant has
        // to be visible to `predicted_dt` as well, otherwise egui's `time` never moves for
        // the machinery that reads the derived value. Both are fed here, and the run stops
        // after the picture pass, so there is no third stamp to get wrong.
        if self.frames_left == 120 {
            let mut warm = egui::RawInput::default();
            if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                warm.screen_rect = Some(rect);
            }
            warm.time = Some(self.clock);
            warm.predicted_dt = 0.016;
            self.clock += 0.016;
            let _ = ctx.run(warm, |c| {
                ui::top(c, &mut self.app);
                ui::left(c, &mut self.app);
                ui::center_ex(c, &mut self.app, false);
                ui::bottom(c, &mut self.app);
            });
        }

        let mut raw = egui::RawInput::default();
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            raw.screen_rect = Some(rect);
        }
        raw.time = Some(self.clock);
        raw.predicted_dt = 0.016;
        self.clock += 0.016;
        let out = ctx.run(raw, |c| {
            ui::top(c, &mut self.app);
            ui::left(c, &mut self.app);
            // The picture pass is the one that gets captured, so it has to be the pass with
            // the seeded history in it.
            ui::center_ex(c, &mut self.app, self.shot_frame);
            ui::bottom(c, &mut self.app);
        });

        // Keep the tessellated frame: eframe hands its own draw list to glow and never
        // gives it back, so this is the only copy we can raster ourselves. `shot_frame` is
        // still on for the picture pass, so the seeded history is inside these shapes.
        if !out.shapes.is_empty() {
            let ppp = ctx.pixels_per_point();
            self.shapes = ctx.tessellate(out.shapes, ppp);
        }

        self.frames_left = self.frames_left.saturating_sub(1);

        if self.frames_left > 0 && !self.done {
            // The picture pass has been tessellated; a live run keeps the normal rule of
            // showing the seeded history only while the flag is on.
            self.shot_frame = false;
            self.done = true;
            let path = self.path.clone();
            match capture(ctx, &self.shapes, &path) {
                Ok((w, h)) => println!(
                    "screenshot: {}x{} -> {} (seeded samples: {})",
                    w,
                    h,
                    path.display(),
                    self.seeded
                ),
                Err(e) => eprintln!("screenshot failed: {}", e),
            }
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        } else if self.frames_left > 0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

/* ------------------------------------------------------------------ self test */

/// Runs the live window for a fixed number of seconds and then reports what it measured.
/// This exists so "does the GUI actually record anything?" can be answered from a script
/// instead of by watching a screen: `--collect 12` prints one line per target and closes the
/// window. It also starts the measurement, because the window itself waits for the user to
/// press „Alle starten", and it saves on the way out so `--export-csv` can show the same run.
pub struct SelfTest {
    app: App,
    until_secs: f64,
    frames_left: u32,
    reported: bool,
}

impl SelfTest {
    pub fn new(mut app: App, cc: &CreationContext<'_>, secs: f64, max_frames: Option<u32>) -> Self {
        // The same look the plain window would get, so the run exercises the live theme path.
        let want = crate::theme::resolve(&cc.egui_ctx, &app.options.theme);
        app.theme_mode = want;
        crate::theme::apply(&cc.egui_ctx, want);
        app.start();
        Self {
            app,
            until_secs: crate::runtime::secs_since_start() + secs.max(1.0),
            // Without a limit an invisible window would sit in the event loop forever.
            frames_left: max_frames.unwrap_or(u32::MAX),
            reported: false,
        }
    }
}

impl eframe::App for SelfTest {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The real frame of the real window.
        self.app.frame(ctx);

        self.frames_left = self.frames_left.saturating_sub(1);
        let time_up = crate::runtime::secs_since_start() >= self.until_secs;
        if !self.reported && (time_up || self.frames_left == 0) {
            self.reported = true;
            // Every target says what it saw, so a probe that resolves but never answers is
            // visible here as well — a plain sample count would hide that. `antworten` counts
            // the replies: a run that only recorded timeouts would otherwise look healthy.
            let mut ok_samples = 0usize;
            for t in &self.app.targets {
                let last = t.series.last();
                let ok = t
                    .series
                    .samples()
                    .iter()
                    .filter(|s| crate::history::is_ok(s.rtt_ms))
                    .count();
                ok_samples += ok;
                println!(
                    "collect: {:<14} {:<20} samples={} antworten={} letzte={}",
                    t.label,
                    t.host,
                    t.series.len(),
                    ok,
                    match last {
                        Some(s) if crate::history::is_ok(s.rtt_ms) => format!("{:.1} ms", s.rtt_ms),
                        Some(_) => "Timeout".to_string(),
                        None => "-".to_string(),
                    }
                );
            }
            let total: usize = self.app.targets.iter().map(|t| t.series.len()).sum();
            println!(
                "collect: {} Messwerte in dieser Zeit, davon {} Antworten",
                total, ok_samples
            );
            // Written down before the window goes away, so a follow-up `--export-csv` reads
            // exactly what this run measured.
            self.app.save_now();
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        }
    }
}

/* ------------------------------------------------------------------ raster */

fn capture(
    ctx: &egui::Context,
    shapes: &[ClippedPrimitive],
    path: &PathBuf,
) -> Result<(u32, u32), String> {
    let size = ctx.input(|i| {
        i.viewport()
            .inner_rect
            .unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, Vec2::new(1080.0, 700.0)))
    });
    let size = size.size();
    let w = size.x.max(64.0) as u32;
    let h = size.y.max(64.0) as u32;
    // The framebuffer starts on the theme's background, not black: egui only tessellates
    // what a panel paints over its own fill, and a `CentralPanel` leaves the rest of the
    // window to the viewport clear colour, which this raster has to supply itself.
    let bg = ctx.style().visuals.panel_fill; // the viewport clear colour: panels only paint their own rects
    let mut img = RgbaImage::from_pixel(w, h, image::Rgba([bg.r(), bg.g(), bg.b(), 255]));

    for clipped in shapes {
        let mesh = match &clipped.primitive {
            egui::epaint::Primitive::Mesh(m) => m,
            _ => continue,
        };
        paint_mesh(&mut img, mesh, clipped.clip_rect, w, h);
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    img.save(path)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok((w, h))
}

fn paint_mesh(img: &mut RgbaImage, mesh: &egui::epaint::Mesh, clip: Rect, w: u32, h: u32) {
    for tri in mesh.indices.as_chunks::<3>().0 {
        let p = |i: usize| -> Pos2 {
            let v = &mesh.vertices[tri[i] as usize];
            egui::pos2(v.pos.x, v.pos.y)
        };
        let c = |i: usize| -> Color32 { mesh.vertices[tri[i] as usize].color };
        fill_triangle(img, p(0), p(1), p(2), c(0), c(1), c(2), clip, w, h);
    }
}

/// Barycentric fill of one glyph triangle. The ten arguments are deliberate: the caller
/// already holds them split out of the mesh (three positions, three colours) plus the clip
/// rect and image size, and bundling them into a struct would only move the cost to the
/// caller without making the loop below any shorter.
#[allow(clippy::too_many_arguments)]
fn fill_triangle(
    img: &mut RgbaImage,
    a: Pos2,
    b: Pos2,
    c: Pos2,
    ca: Color32,
    cb: Color32,
    cc: Color32,
    clip: Rect,
    w: u32,
    h: u32,
) {
    let lo_x = a.x.min(b.x).min(c.x).max(clip.min.x).max(0.0);
    let hi_x = a.x.max(b.x).max(c.x).min(clip.max.x).min(w as f32 - 1.0);
    let lo_y = a.y.min(b.y).min(c.y).max(clip.min.y).max(0.0);
    let hi_y = a.y.max(b.y).max(c.y).min(clip.max.y).min(h as f32 - 1.0);
    if hi_x < lo_x || hi_y < lo_y {
        return;
    }
    let min_x = lo_x.floor() as i32;
    let max_x = hi_x.ceil() as i32;
    let min_y = lo_y.floor() as i32;
    let max_y = hi_y.ceil() as i32;
    if max_x < min_x || max_y < min_y {
        return;
    }
    let d = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
    if d.abs() < 1e-6 {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = ((b.x - px) * (c.y - py) - (c.x - px) * (b.y - py)) / d;
            let w1 = ((c.x - px) * (a.y - py) - (a.x - px) * (c.y - py)) / d;
            let w2 = 1.0 - w0 - w1;
            if w0 < -0.02 || w1 < -0.02 || w2 < -0.02 {
                continue;
            }
            let r = (w0 * ca.r() as f32 + w1 * cb.r() as f32 + w2 * cc.r() as f32) as u32;
            let g = (w0 * ca.g() as f32 + w1 * cb.g() as f32 + w2 * cc.g() as f32) as u32;
            let bl = (w0 * ca.b() as f32 + w1 * cb.b() as f32 + w2 * cc.b() as f32) as u32;
            let al = (w0 * ca.a() as f32 + w1 * cb.a() as f32 + w2 * cc.a() as f32)
                .clamp(0.0, 255.0) as u32;
            if al == 0 {
                continue;
            }
            blend(
                img,
                x as u32,
                y as u32,
                r.min(255) as u8,
                g.min(255) as u8,
                bl.min(255) as u8,
                al as u8,
            );
        }
    }
}

fn blend(img: &mut RgbaImage, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    if x >= img.width() || y >= img.height() {
        return;
    }
    if a == 255 {
        img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
        return;
    }
    let p = img.get_pixel_mut(x, y);
    let af = a as u32;
    let inv = 255 - af;
    p.0[0] = ((r as u32 * af + p.0[0] as u32 * inv) / 255) as u8;
    p.0[1] = ((g as u32 * af + p.0[1] as u32 * inv) / 255) as u8;
    p.0[2] = ((b as u32 * af + p.0[2] as u32 * inv) / 255) as u8;
    p.0[3] = 255;
}

/// Which view a shot should show (`--shot out.png events`).
pub fn view_for(name: &str) -> Option<View> {
    match name.to_ascii_lowercase().as_str() {
        "chart" | "diagramm" => Some(View::Chart),
        "list" | "liste" | "events" => Some(View::Events),
        "stats" | "statistik" => Some(View::Stats),
        "settings" | "einstellungen" => Some(View::Settings),
        _ => None,
    }
}
