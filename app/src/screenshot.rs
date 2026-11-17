//! Screenshot mode: run the real interface for a fixed number of frames, then save a PNG.
//!
//! eframe gives no public way to read back what it drew, so the frame is rasterised
//! from egui's own draw list (`ClippedPrimitive`s): filled and stroked rectangles,
//! lines and text. Text is not drawn here — its shape is taken from the glyph atlas egui
//! already built for the GPU, so a letter in the PNG has the same picture as in the window.
//! No engine internals are touched.

use crate::app::{App, LogKind, View};
use crate::i18n;
use crate::theme;
use crate::ui;
use eframe::CreationContext;
use egui::epaint::ClippedPrimitive;
use egui::{Color32, Pos2, Rect, Vec2};
use image::RgbaImage;
use std::path::{Path, PathBuf};
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
    /// The context the picture is rendered in. Deliberately not the window's own context: a
    /// second egui pass inside the pass `eframe` is already running would have to borrow the
    /// root `Ui` twice, so the capture runs in a context of its own, sized like the window.
    cap_ctx: egui::Context,
    /// The last fully drawn frame, taken from egui's own output.
    shapes: Vec<ClippedPrimitive>,
    /// Our own monotone clock for egui; see [`Screencap::ui`]. The largest time handed to egui so far,
    /// in seconds. Kept so a stalled wall clock can be detected; unused for now.
    #[allow(dead_code)]
    last_egui_time: f64,
    clock: f64,
    /// 1 = the seeded example history stands in for a live trace (see `ui::chart`).
    pub shot_frame: bool,
    /// Open the inline editor of this target in the picture, so the edit UI can be checked
    /// without clicking by hand. `None` leaves every row as a plain label.
    edit_row: Option<usize>,
}

/// Which target the seeded story in the log names: an outage and the recovery of it, so the picture
/// shows the log doing its job. The second row reads better than the top one, but a machine with a
/// single target has only the top one — and asking for `targets[1]` unconditionally was an
/// `index out of bounds` that took a whole `--shot` down on exactly such a machine (a build server
/// whose state file held one row).
fn log_target(app: &App) -> Option<&crate::app::Target> {
    app.targets.get(1).or_else(|| app.targets.first())
}

impl Screencap {
    pub fn new(app: App, cc: &CreationContext<'_>, path: PathBuf, view: Option<String>) -> Self {
        // From here on the app must not probe (see `capture_armed`).
        CAPTURE_ARMED.store(true, Ordering::Relaxed);
        theme::apply(&cc.egui_ctx, app.theme_mode);
        // Fixed surface so the shot is reproducible.
        cc.egui_ctx.set_pixels_per_point(1.0);
        // The capture context: same look, same scale, its own font atlas and its own pass
        // state, so nothing here disturbs the window (see the field).
        let cap_ctx = egui::Context::default();
        theme::apply(&cap_ctx, app.theme_mode);
        cap_ctx.set_pixels_per_point(1.0);
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
            cap_ctx,
            shapes: Vec::new(),
            shot_frame: false,
            edit_row: None,
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
        // The planted history covers half the window (see the step below), so the session the
        // picture shows has to be older than that window by the same margin — a process of two
        // seconds has no day of its own, and everything left of its start would be time before
        // the app began (see [`crate::runtime::backdate`]).
        crate::runtime::backdate(horizon * 0.5 + 120.0);
        // One clock for the whole program (`runtime`), so a stamped sample and the view's
        // "now" are on the same axis. Capture happens after the shot's frame loop
        // (120 frames, ~1.92 s); stamping backwards from that fills the window.
        let base_t = crate::runtime::secs_since_start() + 1.92;
        // Absolute counterpart of `base_t`, so the seeded samples carry a real time too.
        let now_ms = chrono::Utc::now().timestamp_millis() + 1920;
        // A believable trace: a clean baseline, one latency spike per target (kept well
        // below the chart's own y-scaling so the outlier rings land inside the plot), and
        // an outage streak on the second target. The last sample sits just under "now" so
        // every target keeps its newest-value dot in the live window.
        //
        // One hundred samples are enough for a minute, but a day drawn from 110 points would
        // be a dotted line, so wide windows get one point per pixel of the plot instead. The
        // cap grows with the window (see `effective_max_samples`), so nothing is thrown away.
        let n = ((self.app.options.horizon_secs / 2.0) as usize).clamp(110, 3_000);
        // Samples must span the visible window: `horizon / n` seconds apart, with the newest one
        // a step before "now". A fixed spacing (2 s) would only cover part of a wider window and
        // leave the right side of the plot empty. Half a window, because that is how a session
        // that was started and left alone actually looks — a monitor that filled a year down to
        // the last pixel reads as a demo, and the first thing the eye checks on a day picture is
        // whether there was a night.
        let step = horizon * 0.5 / n as f64;
        for (i, t) in self.app.targets.iter_mut().enumerate() {
            // Keep the seeded history inside the configured window.
            t.series
                .set_cap(crate::app::effective_max_samples(&self.app.options));
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
                // The outage and the spike used to be fixed sample numbers, which only land
                // inside the picture while `n` is around 110. As fractions of the trace they
                // stay visible in a minute and in a day alike.
                let outage = i == 1 && (0.56..=0.64).contains(&(k as f64 / n as f64));
                let rtt = if outage {
                    TIMEOUT
                } else if k % ((n / 3).max(1)) == 0 {
                    base * 2.4 + 25.0
                } else {
                    base + jitter
                };
                t.series.record(
                    base_t - (n - k) as f64 * step,
                    // Absolute counterpart of the relative stamp. A wide window is labelled
                    // with the wall clock, so a seeded sample has to carry a believable time of
                    // day too — derived from the same spacing the chart uses.
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
        let lang = self.app.lang();
        self.app.log_info(i18n::fill(
            lang.log_seeded_fmt(),
            &[("n", (n * self.app.targets.len()).to_string())],
        ));
        self.app.alert_count += 1;
        let label = log_target(&self.app)
            .map(|t| t.label.clone())
            .unwrap_or_default();
        self.app.log_event(
            LogKind::Outage,
            i18n::fill(
                lang.log_outage_fmt(),
                &[
                    ("label", label.clone()),
                    ("why", lang.log_why_timeout().to_string()),
                ],
            ),
        );
        self.app.log_event(
            LogKind::Recovery,
            i18n::fill(
                lang.log_recovery_fmt(),
                &[("label", label), ("ms", "3".into())],
            ),
        );
        n * self.app.targets.len()
    }

    /// Show one of the four views in the shot, or set the chart window from a length
    /// (`--shot out.png 24h`). Both happen before `seed` plants the history, because that
    /// history is spread over whatever window is current (see [`Screencap::ui`]). The two can be
    /// combined — `settings 24h` pictures the settings panel of a day window.
    fn set_view(&mut self, name: &str) {
        for word in name.split_whitespace() {
            if let Some(v) = view_for(word) {
                self.app.view = v;
            } else if let Some(secs) = horizon_from_arg(word) {
                self.app.options.horizon_secs = secs;
                self.app.sync_caps();
            }
        }
    }

    /// Open the editor of one target in the picture (see `wants_edit`).
    pub fn show_editor(&mut self, row: Option<usize>) {
        self.edit_row = row;
    }
}

impl eframe::App for Screencap {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The window's own context is only asked for its size and told to close; the picture is
        // rendered in `cap_ctx` (see the field).
        let ctx = root.ctx();
        if self.frames_left == 120 {
            // The view and the window length come from the command line and have to be in
            // place before the history is planted: `seed` spreads its samples over whatever
            // window is current.
            if let Some(v) = self.view.take() {
                self.set_view(&v);
            }
            // Seeding happens in this same pass, so the history is stamped on the clock
            // that will be current at capture (see `seed`).
            self.seeded = self.seed();
            // `--shot out.png edit` shows the inline editor of the first target.
            if self.edit_row == Some(0) {
                self.app.begin_edit(0);
            }
        }

        // Draw inside a real egui output pass: `tessellate` only sees the last frame that
        // finished, and with a hidden window there is no vsync to force one.
        //
        // The picture pass runs inside ONE `Context::run_ui` on `cap_ctx`, and the capture
        // happens in that same pass. Earlier versions ran ~120 passes through eframe and kept
        // the last tessellated frame; egui then asserted "Time shouldn't move backwards" inside
        // a value history, because `InputState::begin_pass` derives its clock as
        // `new.time.unwrap_or(self.time + new.predicted_dt as f64)` — with `time: None` and
        // `predicted_dt: 0.0` every pass was stamped with the very same instant, so any value
        // history that egui itself keeps (drag/animation machinery) hit the monotonicity
        // assert on the second pass. So every pass below carries its own stamp plus a
        // predicted_dt greater than zero, which keeps that clock strictly increasing.
        if self.frames_left == 120 {
            self.shot_frame = true;
        }

        // The window's size in logical points: the picture has to be the surface the window is
        // showing, and this is the one place that knows it.
        let rect = ctx.input(|i| i.viewport().inner_rect);

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
            if let Some(rect) = rect {
                warm.screen_rect = Some(rect);
            }
            warm.time = Some(self.clock);
            warm.predicted_dt = 0.016;
            self.clock += 0.016;
            let _ = crate::headless::paint(&self.cap_ctx, warm, |ui| {
                ui::top(ui, &mut self.app);
                ui::left(ui, &mut self.app);
                ui::center_ex(ui, &mut self.app, false);
                ui::bottom(ui, &mut self.app);
            });
        }

        let mut raw = egui::RawInput::default();
        if let Some(rect) = rect {
            raw.screen_rect = Some(rect);
        }
        raw.time = Some(self.clock);
        raw.predicted_dt = 0.016;
        self.clock += 0.016;
        let out = crate::headless::paint(&self.cap_ctx, raw, |ui| {
            ui::top(ui, &mut self.app);
            ui::left(ui, &mut self.app);
            // The picture pass is the one that gets captured, so it has to be the pass with
            // the seeded history in it.
            ui::center_ex(ui, &mut self.app, self.shot_frame);
            ui::bottom(ui, &mut self.app);
        });

        // Keep the tessellated frame: this program has no way to read a framebuffer back, so the
        // only picture it can save is the one made from egui's own draw list. `shot_frame` is
        // still on for the picture pass, so the seeded history is inside these shapes.
        if !out.shapes.is_empty() {
            let ppp = out.pixels_per_point;
            self.shapes = self.cap_ctx.tessellate(out.shapes, ppp);
        }

        self.frames_left = self.frames_left.saturating_sub(1);

        if self.frames_left > 0 && !self.done {
            // The picture pass has been tessellated; a live run keeps the normal rule of
            // showing the seeded history only while the flag is on.
            self.shot_frame = false;
            self.done = true;
            let path = self.path.clone();
            match capture(&self.cap_ctx, rect, &self.shapes, &path) {
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
/// press "start all", and it saves on the way out so `--export-csv` can show the same run.
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
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The real frame of the real window.
        self.app.frame_ui(root);
        let ctx = root.ctx();

        self.frames_left = self.frames_left.saturating_sub(1);
        let time_up = crate::runtime::secs_since_start() >= self.until_secs;
        if !self.reported && (time_up || self.frames_left == 0) {
            self.reported = true;
            // Every target says what it saw, so a probe that resolves but never answers is
            // visible here as well — a plain sample count would hide that. `replies` counts them:
            // a run that only recorded timeouts would otherwise look healthy. These lines are the
            // machine-readable part of the tool, so they stay English in both interface languages.
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
                    "collect: {:<17} {:<20} samples={} replies={} last={}",
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
            println!("collect: {total} measurements in this time, {ok_samples} of them replies");
            // Written down before the window goes away, so a follow-up `--export-csv` reads
            // exactly what this run measured.
            self.app.save_now();
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        }
    }
}

/* ------------------------------------------------------------------ drag self test */

/// Watches a real person drag a row and prints what egui makes of every pass.
///
/// The headless tests in `tests/drag_order.rs` drive a synthetic pointer without a window, and they are the ones that
/// guard the behaviour. This is for what they cannot show: whether the gesture works against input that comes through the
/// OS and the windowing library. Started with `DRAG_WATCH=1`; move the mouse over the list and drag a row, and every pass
/// prints where egui thinks the pointer is, which row strip it lands on, and what the list order is. The window closes
/// itself once nothing has been touched for a few seconds.
///
/// Deliberately passive. A scripted cursor was tried first and does not work: `SetCursorPos` moves the pointer, but
/// Windows only reports a move that it finds *between* two polls, so a programmatic warp inside one frame never reaches
/// winit at all — egui sat at `hover=None` for every pass while the cursor was demonstrably in the right place. Only a
/// real mouse produces events, so this watches one instead of faking it.
pub struct DragSelfTest {
    app: App,
    /// Passes with no pointer movement since the last one; the run ends once enough have piled up.
    idle: usize,
    /// Whether the pointer has been inside the window at least once. Idle only starts counting after
    /// that, because a watcher nobody has come back to yet is not a watcher that is done.
    seen_pointer: bool,
    /// Whether the row strips have been printed yet. They only exist after a pass that drew them, so
    /// this is written on the first pass that can answer — which is what lets a script aim at a row by
    /// reading the log instead of guessing where the list happens to start.
    rows_logged: bool,
}

impl DragSelfTest {
    pub fn new(mut app: App, cc: &CreationContext<'_>) -> Self {
        let want = crate::theme::resolve(&cc.egui_ctx, &app.options.theme);
        app.theme_mode = want;
        crate::theme::apply(&cc.egui_ctx, want);
        app.start();
        // The watcher drives its own frames and must not race the sampler.
        app.running = false;
        eprintln!("DRAGWATCH: drag a row in the list; the window closes when you stop.");
        Self {
            app,
            idle: 0,
            seen_pointer: false,
            rows_logged: false,
        }
    }
}

impl eframe::App for DragSelfTest {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let order = self
            .app
            .targets
            .iter()
            .map(|t| t.label.clone())
            .collect::<Vec<_>>();
        // Where egui believes the pointer is, and what our own layout says about that point. Both are printed because a
        // disagreement between them is precisely the bug being hunted: egui's hit test versus `App::row_rects`.
        //
        // The three answers from egui are the gesture's own decision points, so they are printed as egui gives them and not as
        // our code later interprets them. `hit` is the widget a press belongs to — it names a row from the moment the button
        // goes down, which is what keeps the editor off a press that turns into a pull. `drag` only appears once the pointer has
        // left egui's slop, and `stop` marks the single pass in which a drop can be measured. A press that reads `hit=Some` then
        // never `drag=` is a click; one that reads `drag=Some` then `stop=Some` is a pull, and the order printed on that same line
        // is what the drop had to work from.
        let seen = root.ctx().input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.any_down(),
                i.pointer.delta(),
                i.time,
            )
        });
        let rows = self.app.target_row_rects();
        let egui_says = crate::ui::gesture_report(root.ctx());
        // Once, as soon as they exist: the strips in list order. A script that wants to drive this
        // gesture needs a place to press, and these are the only numbers that say where a row really
        // stands — the window's own position and the panel's height both move.
        if !self.rows_logged && !rows.is_empty() {
            self.rows_logged = true;
            let dump = rows
                .iter()
                .map(|r| {
                    format!(
                        "[{:.0},{:.0}..{:.0},{:.0}]",
                        r.min.x, r.min.y, r.max.x, r.max.y
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("DRAGWATCH rows {dump}");
        }
        match seen.0 {
            Some(pos) => {
                let row = rows.iter().position(|r| r.contains(pos));
                eprintln!(
                    "DRAGWATCH ptr=[{:.0},{:.0}] delta=[{:.1},{:.1}] down={} row={:?} {} ours={:?} order={:?}",
                    pos.x,
                    pos.y,
                    seen.2.x,
                    seen.2.y,
                    seen.1,
                    row,
                    egui_says,
                    self.app.drag_row,
                    order
                );
                // Movement keeps the run alive; a still pointer means the person is done.
                self.seen_pointer = true;
                if seen.2 == egui::Vec2::ZERO && !seen.1 {
                    self.idle += 1;
                } else {
                    self.idle = 0;
                }
            }
            None => {
                eprintln!("DRAGWATCH no pointer, rows={:?}", rows.len());
                // Only a hand that has been here and left counts as "the person is done"; before the
                // first pass with a pointer the watcher is simply waiting to be used.
                if self.seen_pointer {
                    self.idle += 1;
                }
            }
        }

        self.app.frame_ui(root);

        let ctx = root.ctx();
        if self.idle > 300 {
            eprintln!("DRAGWATCH final order: {:?}", order);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Repaint on our own schedule so a drag is sampled continuously rather than only when the OS feels like it.
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.stop();
    }
}

/* ------------------------------------------------------------------ raster */

/// Rasterises one tessellated frame into a PNG. `surface` is the window the picture stands in
/// for; `cap` supplies the look, which is what the framebuffer has to start on, and the glyph
/// atlas the text is cut out of.
pub fn capture(
    cap: &egui::Context,
    surface: Option<Rect>,
    shapes: &[ClippedPrimitive],
    path: &Path,
) -> Result<(u32, u32), String> {
    let surface =
        surface.unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, Vec2::new(1080.0, 700.0)));
    let size = surface.size();
    let w = size.x.max(64.0) as u32;
    let h = size.y.max(64.0) as u32;
    // A draw list speaks screen coordinates: the window's top-left corner is where the window
    // stands on the desktop. The picture is the window on its own, so that corner is moved to the
    // origin — kept where it is, it leaves the desktop's empty space along two sides of the
    // picture and cuts the window off along the other two.
    let shift = -surface.min.to_vec2();
    // The framebuffer starts on the theme's background, not black: egui only tessellates
    // what a panel paints over its own fill, and a `CentralPanel` leaves the rest of the
    // window to the viewport clear colour, which this raster has to supply itself.
    let bg = cap.global_style().visuals.panel_fill; // the viewport clear colour: panels only paint their own rects
    let mut img = RgbaImage::from_pixel(w, h, image::Rgba([bg.r(), bg.g(), bg.b(), 255]));
    // The glyph atlas, read once up front: `Context::fonts` needs the context for writing and
    // no single texel below should be paid for with that lock.
    let font = FontTex::grab(cap);

    for clipped in shapes {
        let mesh = match &clipped.primitive {
            egui::epaint::Primitive::Mesh(m) => m,
            _ => continue,
        };
        // Every mesh of this program is either text or a shape, and both come in on the font
        // texture — a shape samples the atlas's guaranteed white pixel, which is what
        // `WHITE_UV` points at. A mesh on any other texture has no picture here.
        let tex = match mesh.texture_id {
            egui::epaint::TextureId::Managed(0) => Some(&font),
            _ => None,
        };
        paint_mesh(
            &mut img,
            mesh,
            clipped.clip_rect.translate(shift),
            w,
            h,
            tex,
            shift,
        );
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    img.save(path)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok((w, h))
}

fn paint_mesh(
    img: &mut RgbaImage,
    mesh: &egui::epaint::Mesh,
    clip: Rect,
    w: u32,
    h: u32,
    tex: Option<&FontTex>,
    shift: Vec2,
) {
    for tri in mesh.indices.as_chunks::<3>().0 {
        let v = |i: usize| -> &egui::epaint::Vertex { &mesh.vertices[tri[i] as usize] };
        fill_triangle(img, [v(0), v(1), v(2)], clip, w, h, tex, shift);
    }
}

/// The glyph atlas in the form the raster wants it: texels held by hand, so reading one while
/// pixels are written never needs the context.
struct FontTex {
    size: [usize; 2],
    px: Vec<Color32>,
}

impl FontTex {
    fn grab(ctx: &egui::Context) -> Self {
        let img = ctx.fonts(|f| f.image());
        Self {
            size: img.size,
            px: img.pixels,
        }
    }

    /// The texels behind a texture coordinate, interpolated the way
    /// [`egui::epaint::TextureAtlas::texture_options`] asks the GPU to: bilinear, and edge
    /// texels repeated instead of wrapping around the atlas. A glyph's quad corners carry the
    /// uv of that glyph's rect in the atlas, so a triangle reaching outside that rect reaches
    /// past the glyph — clamping is what the GPU's clamp mode does there and keeps the
    /// neighbour's picture out of it.
    fn sample(&self, uv: Pos2) -> Color32 {
        let [cw, ch] = self.size;
        let u = uv.x.clamp(0.0, 1.0);
        let v = uv.y.clamp(0.0, 1.0);
        // Texel centres sit half a texel inside their corner, which is what the `- 0.5` is.
        let x = (u * cw as f32 - 0.5).clamp(0.0, cw as f32 - 1.0);
        let y = (v * ch as f32 - 0.5).clamp(0.0, ch as f32 - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(ch - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let texel = |gx: usize, gy: usize| -> Color32 { self.px[gy * cw + gx] };
        let (a, b) = (texel(x0, y0), texel(x1, y0));
        let (c, d) = (texel(x0, y1), texel(x1, y1));
        Color32::from_rgba_unmultiplied(
            mix4(a.r(), b.r(), c.r(), d.r(), fx, fy),
            mix4(a.g(), b.g(), c.g(), d.g(), fx, fy),
            mix4(a.b(), b.b(), c.b(), d.b(), fx, fy),
            mix4(a.a(), b.a(), c.a(), d.a(), fx, fy),
        )
    }
}

/// One channel of the bilinear mix of four texels, with `fx` and `fy` as the weights inside
/// the cell they span.
fn mix4(a: u8, b: u8, c: u8, d: u8, fx: f32, fy: f32) -> u8 {
    let top = a as f32 + (b as f32 - a as f32) * fx;
    let bot = c as f32 + (d as f32 - c as f32) * fx;
    (top + (bot - top) * fy).round() as u8
}

/// Vertex colour times texture picture: the multiply a fragment shader does. The atlas keeps a
/// glyph as white with the coverage in its alpha, so a letter takes its colour from the vertex
/// and its shape from the picture; the prerasterised discs the atlas hands out for round dots
/// work the same way, and colour glyphs carry their own colours for a white vertex to let
/// through. An untextured shape's `WHITE_UV` lands on the atlas's white pixel and is unchanged.
fn mul(a: Color32, b: Color32) -> Color32 {
    // `+ 127` rounds the division: cut off instead, a half-lit pixel would come out a shade
    // darker than the one next to it, which is a grain over the whole picture.
    let m = |x: u8, y: u8| ((x as u32 * y as u32 + 127) / 255) as u8;
    Color32::from_rgba_unmultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

/// Barycentric fill of one triangle of the draw list, texture sample included. `shift` moves the
/// triangle from the screen it was drawn on onto the picture.
fn fill_triangle(
    img: &mut RgbaImage,
    v: [&egui::epaint::Vertex; 3],
    clip: Rect,
    w: u32,
    h: u32,
    tex: Option<&FontTex>,
    shift: Vec2,
) {
    let (a, b, c) = (v[0].pos + shift, v[1].pos + shift, v[2].pos + shift);
    let (ca, cb, cc) = (v[0].color, v[1].color, v[2].color);
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
            // Rounding, not truncation: the weights sum to one only as well as floating point
            // allows, so a solid colour often lands as 254.9999998 — truncated that is a pixel a
            // shade too light, in a pattern as regular as the arithmetic that made it.
            let r = (w0 * ca.r() as f32 + w1 * cb.r() as f32 + w2 * cc.r() as f32).round();
            let g = (w0 * ca.g() as f32 + w1 * cb.g() as f32 + w2 * cc.g() as f32).round();
            let bl = (w0 * ca.b() as f32 + w1 * cb.b() as f32 + w2 * cc.b() as f32).round();
            let al = (w0 * ca.a() as f32 + w1 * cb.a() as f32 + w2 * cc.a() as f32).round();
            let col = Color32::from_rgba_unmultiplied(
                r.clamp(0.0, 255.0) as u8,
                g.clamp(0.0, 255.0) as u8,
                bl.clamp(0.0, 255.0) as u8,
                al.clamp(0.0, 255.0) as u8,
            );
            let col = match tex {
                Some(t) => mul(
                    col,
                    t.sample(egui::pos2(
                        w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x,
                        w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y,
                    )),
                ),
                None => col,
            };
            if col.a() == 0 {
                continue;
            }
            blend(img, x as u32, y as u32, col.r(), col.g(), col.b(), col.a());
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
        // Not a view: keeps the chart, but opens the editor of the first target.
        "edit" | "bearbeiten" => None,
        _ => None,
    }
}

/// Whether a shot argument asks for the inline editor to be open in the picture.
pub fn wants_edit(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "edit" | "bearbeiten")
}

/// A shot argument may also name the chart window, so a day can be pictured without touching
/// the saved settings: `--shot out.png 24h`. Returns seconds, or `None` if the word is not a
/// length. Both German and English spellings are accepted because the UI is German while the
/// command line usually is not.
pub fn horizon_from_arg(name: &str) -> Option<f32> {
    let s = name.trim().to_ascii_lowercase();
    // Longest unit first so "stunden" is not read as a number with a stray suffix.
    for (unit, secs) in [
        ("sekunden", 1.0f32),
        ("stunden", 3_600.0),
        ("minute", 60.0),
        ("minuten", 60.0),
        ("stunde", 3_600.0),
        ("sekunde", 1.0),
        ("sek", 1.0),
        ("min", 60.0),
        ("std", 3_600.0),
        ("hrs", 3_600.0),
        ("hr", 3_600.0),
        ("s", 1.0),
        ("h", 3_600.0),
    ] {
        if let Some(rest) = s.strip_suffix(unit) {
            let n: f32 = rest.trim().parse().ok()?;
            if n > 0.0 {
                return Some(n * secs);
            }
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::log_target;
    use crate::app::{App, AppOptions};

    /// An app with exactly these hosts. `set_test_targets` replaces the built-in three and writes
    /// nothing to disk, which is the only way to end up with one row on purpose.
    fn app_with(hosts: &[&str]) -> App {
        let mut app = App::with_theme_override(AppOptions::default(), Vec::new(), None);
        let rows: Vec<(&str, &str)> = hosts.iter().map(|h| (*h, *h)).collect();
        app.set_test_targets(&rows);
        app
    }

    /// The whole point of the helper: however many rows there are, the row it names exists.
    #[test]
    fn the_log_names_a_row_that_exists() {
        assert!(
            log_target(&app_with(&[])).is_none(),
            "no row, nothing to name"
        );

        let one = app_with(&["10.0.0.1"]);
        assert_eq!(
            log_target(&one).map(|t| t.host.clone()),
            Some("10.0.0.1".to_string()),
            "one row is that row"
        );

        let two = app_with(&["10.0.0.1", "10.0.0.2"]);
        assert_eq!(
            log_target(&two).map(|t| t.host.clone()),
            Some("10.0.0.2".to_string()),
            "two rows: the second one, like before"
        );
    }
}
