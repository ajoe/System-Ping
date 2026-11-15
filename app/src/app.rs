//! Application state, the probe scheduler and the event plumbing.

use crate::exporter;
use crate::history::{self, is_ok, Fail, Sample, Series, Stats};
use crate::ping::{ProbeJob, ProbeResult};
use crate::store;
use egui::Color32;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Why a probe did not answer, as the app records it. Mirrors
/// [`crate::ping::ProbeFail`]; see [`crate::history::Fail`].
pub type ProbeKind = crate::history::Fail;

/// What a target is, and how it should be labelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TargetKind {
    Gateway,
    Dns,
    Internet,
    Lan,
    Custom,
}

impl TargetKind {
    pub fn label(&self) -> &'static str {
        match self {
            TargetKind::Gateway => "Gateway",
            TargetKind::Dns => "DNS",
            TargetKind::Internet => "Internet",
            TargetKind::Lan => "LAN-Gerät",
            TargetKind::Custom => "Eigene",
        }
    }

    pub fn all() -> [TargetKind; 5] {
        [
            TargetKind::Gateway,
            TargetKind::Dns,
            TargetKind::Internet,
            TargetKind::Lan,
            TargetKind::Custom,
        ]
    }
}

/// One monitored address.
pub struct Target {
    pub id: usize,
    pub label: String,
    pub host: String,
    pub kind: TargetKind,
    pub enabled: bool,
    pub series: Series,
    /// Seconds since start of the last completed probe (NaN = never).
    pub last_probe_secs: f64,
    pub last_rtt: f32,
    pub streak_bad: u32,
    /// Alert was raised for the current outage (so it fires once).
    pub alert_raised: bool,
}

impl Target {
    fn new(id: usize, label: impl Into<String>, host: impl Into<String>, kind: TargetKind) -> Self {
        Self {
            id,
            label: label.into(),
            host: host.into(),
            kind,
            enabled: true,
            series: Series::new(2048),
            last_probe_secs: f64::NAN,
            last_rtt: f32::NAN,
            streak_bad: 0,
            alert_raised: false,
        }
    }
}

/// User-facing settings. All persisted.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AppOptions {
    /// Seconds between probes for every target.
    pub interval_secs: f32,
    /// Per-probe timeout in ms.
    pub timeout_ms: u32,
    /// How many probes may run at the same time.
    pub concurrency: usize,
    /// Chart window width in seconds.
    pub horizon_secs: f32,
    /// Max samples kept per target.
    pub max_samples: usize,
    pub sound_alerts: bool,
    pub system_tray_icon: bool,
    pub autostart: bool,
    pub minimised_to_tray: bool,
    /// "system", "light" or "dark". Missing in old files -> "system".
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Append every measurement to a protocol file. Off by default.
    #[serde(default)]
    pub protocol_log: bool,
    /// Where that file goes; empty means next to the saved data.
    #[serde(default)]
    pub protocol_path: String,
}

/// "system", "light" or "dark": which look to use.
fn default_theme() -> String {
    String::from("system")
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            interval_secs: 1.0,
            timeout_ms: 2000,
            concurrency: 4,
            horizon_secs: 60.0,
            max_samples: 2048,
            sound_alerts: false,
            system_tray_icon: true,
            autostart: false,
            minimised_to_tray: false,
            theme: String::from("system"),
            protocol_log: false,
            protocol_path: String::new(),
        }
    }
}

/// What the main area shows. `Events` is the status list (live rows per target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Chart,
    Events,
    Stats,
    Settings,
}

impl View {
    pub fn label(&self) -> &'static str {
        match self {
            View::Chart => "Diagramm",
            View::Events => "Ereignisse",
            View::Stats => "Statistik",
            View::Settings => "Einstellungen",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self {
            View::Chart => "Latenzverlauf als Kurve, mit Ausreißern markiert",
            View::Events => "Statusliste und Ereignisprotokoll",
            View::Stats => "Kennzahlen je Ziel",
            View::Settings => "Intervall, Zeitfenster, Export",
        }
    }
}

pub struct App {
    pub options: AppOptions,
    pub targets: Vec<Target>,
    pub selected: usize,
    pub view: View,
    pub running: bool,
    pub engine: crate::ping::PingEngine,
    pub started_wall: chrono::DateTime<chrono::Local>,

    // UI text buffers
    pub new_host: String,
    pub new_label: String,
    pub filter: String,

    /// The target being edited in the inline editor, if any. Set by clicking a row in the left
    /// panel; cleared when the user commits or cancels. `edit_idx` is the index into `targets`,
    /// which is safe because the list cannot change while the editor is open — `add_target` and
    /// `remove_selected` refuse to touch the list while something is being edited.
    pub edit_idx: Option<usize>,
    pub edit_host: String,
    pub edit_label: String,
    /// Category picked in the editor. Normally filled in by `classify`, but it can be overridden.
    pub edit_kind: TargetKind,
    /// Whether this target is measured. Turning it off pauses probing without deleting history.
    pub edit_enabled: bool,

    // Chart interaction
    pub hover_x: Option<f64>,
    pub follow_live: bool,
    pub pan_offset_secs: f64,

    // Log of notable events (outages, recoveries).
    pub log: Vec<LogEntry>,
    pub log_total: usize,

    /// Optional protocol file: every measurement is appended as it arrives.
    pub protocol: Option<crate::exporter::ProtocolFile>,
    /// Seconds since start of the last protocol retry, so a broken path is not reopened
    /// on every probe.
    protocol_retry_secs: f64,
    /// Why each target's newest probe failed, so a protocol line can say why.
    last_fail: Vec<Option<Fail>>,
    pub alert_count: usize,

    // Scheduling bookkeeping
    next_probe_at: BTreeMap<usize, f64>,
    last_tick_secs: f64,
    /// Session clock of the next autosave. `None` until the first tick: a headless pass that
    /// never ticks would otherwise compare against a due time from a clock that has not been
    /// started yet and save on its very first frame.
    save_due_secs: Option<f64>,

    // Background work handoff
    pending_export: Arc<AtomicBool>,
    export_result: Arc<MutexSlot<String>>,
    /// Registry answer for the autostart entry, written by a background thread. Read through
    /// `autostart_state()`, which turns „not yet looked up“ into `None`.
    autostart_state: Arc<AtomicUsize>,

    pub status_msg: String,
    /// Scratch buffer for the per-frame p95 scan in [`ui::chart`], so the chart doesn't
    /// allocate a fresh vector every frame.
    pub chart_p95: Vec<f32>,
    /// Scratch buffer for [`history::Series::stats_window`], reused every frame.
    pub stats_scratch: Vec<f32>,
    /// Last theme the widgets were drawn for, so a switch re-tints everything.
    pub theme_mode: crate::theme::Mode,
    /// The click area of every row in the target list, as reported by [`ui::left_body`] during
    /// the last pass. Only consumer is the click test, which needs it to aim a synthetic mouse
    /// at a real row instead of guessing coordinates.
    pub row_rects: Vec<egui::Rect>,
    /// Line spacing inside a target row's label block, reported by [`ui::left_body`]. Together
    /// with [`Self::row_rects`] it lets the click test aim at the gap between the two lines —
    /// the only spot in a row that no label covers.
    pub row_label_gap: f32,
    /// Height of every row in the target list from the last pass. While a row is being edited its
    /// frame is replaced by a much taller form; remembering the old height keeps that row's click
    /// area where it was, so the rows below it do not move out from under the mouse.
    pub row_heights: Vec<f32>,
}

impl App {
    /// One frame of the live window: advance the engine, then paint. This is the whole
    /// measurement loop of the GUI — `tick` is what schedules probes and moves results into
    /// the series the chart draws, so a frame that skips it paints an empty plot forever.
    /// It lives here (and not only in `eframe::App::update`) so a test can drive a real frame
    /// without a window.
    pub fn frame(&mut self, ctx: &egui::Context) {
        let now = self.engine.elapsed_secs();
        self.tick(now);
        crate::ui::draw(ctx, self);
        // A live monitor repaints even when nobody touches it: samples arrive on their own,
        // and egui would otherwise sit idle until the next mouse event.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let want = crate::theme::resolve(ctx, &self.options.theme);
        if want != self.theme_mode {
            self.theme_mode = want;
            crate::theme::apply(ctx, want);
        }
        self.frame(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
    }
}

/// Tiny slot for a background result (string payload).
pub struct MutexSlot<T> {
    inner: std::sync::Mutex<Option<T>>,
}
impl<T> MutexSlot<T> {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(None),
        }
    }
    pub fn take(&self) -> Option<T> {
        self.inner.lock().ok().and_then(|mut o| o.take())
    }
    pub fn put(&self, v: T) {
        if let Ok(mut o) = self.inner.lock() {
            *o = Some(v);
        }
    }
}

impl<T> Default for MutexSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub secs: f64,
    pub text: String,
    pub kind: LogKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Outage,
    Recovery,
    Info,
}

/// A headless probe pass: run the same engine the window uses, wait `wait_secs`, and write
/// what came back to a JSON history file. No eframe, no window — this is the seam the
/// GUI-free tests (`tests/collection.rs`) drive. Returns one `(host, samples_before,
/// samples_after)` entry per target, so a caller can see whether data actually arrived.
pub fn collect_once(state_dir: &std::path::Path, wait_secs: f64) -> Vec<(String, usize, usize)> {
    let (mut options, mut saved) = store::load_from(state_dir);
    if options.horizon_secs <= 0.0 {
        options.horizon_secs = 60.0;
    }
    let horizon = options.horizon_secs.max(3_600.0);
    let mut app = App::new(options, std::mem::take(&mut saved));
    // A saved history is kept only for the horizon that was chosen in the window; widen it
    // so a headless pass keeps everything it collects.
    app.options.horizon_secs = horizon;
    // and probe every second: the default interval is 15 s, which would leave a
    // seven-second run with nothing to record.
    app.options.interval_secs = 1.0;
    // The window starts probing on its own; a headless pass must start them itself, and
    // against hosts it names itself — otherwise a machine with no internet would probe
    // nothing at all and the pass would prove nothing either way.
    app.targets = vec![
        Target::new(1, "loopback", "127.0.0.1", TargetKind::Internet),
        Target::new(2, "unreachable", "192.0.2.123", TargetKind::Internet),
    ];
    app.running = true;
    // The scheduler only issues probes from `tick`, and a headless pass has no frames to
    // drive it; the loop below supplies the clock (the same monotone origin the engine
    // stamps its results with, see `runtime`).
    app.start();
    let engine = app.engine.clone();
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs_f64(wait_secs.max(1.0));
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let now = engine.elapsed_secs();
        app.tick(now);
        if std::time::Instant::now() >= deadline {
            break;
        }
    }
    // Leave no jobs behind: without this the workers would keep pinging while the caller waits.
    engine.stop_all();
    // The window saves from its own autosave timer; a short pass ends before that fires, so
    // write what was collected now — `save_now_to` serialises exactly the live series, and
    // returns it so the report below counts the samples that are really on disk.
    let written = app.save_now_to(state_dir).unwrap_or_default();
    let mut out = Vec::with_capacity(app.targets.len());
    for t in &app.targets {
        // The report counts what `save_now_to` really wrote, and the file keeps everything:
        // what was restored from disk plus what this pass collected.
        let fresh = written
            .iter()
            .find(|r| r.id == t.id)
            .map_or(0, |r| r.samples.len());
        let before = saved
            .iter()
            .find(|r| r.id == t.id)
            .map_or(0, |r| r.samples.len());
        out.push((t.host.clone(), before, fresh));
    }
    let _ = store::save_to(state_dir, &app.options, &written);
    out
}

/// Headless export: write `saved` (the persisted history) to `path` and report how many
/// rows landed on disk. No window, no probe thread — the samples are already on file.
pub fn export_history_to(
    saved: &[store::SavedTarget],
    path: &std::path::Path,
) -> Result<usize, String> {
    let mut rows = Vec::new();
    for s in saved {
        append_saved_rows(s, &mut rows);
    }
    if rows.is_empty() {
        // Nothing was persisted yet (a fresh install exports right after start), so probe
        // once with the same engine the window uses and export what comes back.
        let mut app = App::new(AppOptions::default(), Vec::new());
        app.start();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            app.tick(app.now_secs());
            if app.targets.iter().all(|t| t.last_probe_secs > 0.0) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        app.stop();
        rows = collect_export_rows(&app.targets);
    }
    if rows.is_empty() {
        return Err(String::from(
            "Es gibt noch keine Messwerte zum Exportieren.",
        ));
    }
    exporter::write_csv(path, &rows)
}

/// Turns one persisted target into CSV rows. Every stored sample carries its own absolute
/// time, so a history written across several restarts exports as real, monotone time.
fn append_saved_rows(s: &store::SavedTarget, rows: &mut Vec<exporter::Row>) {
    let mut series = Series::new(s.samples.len().max(16));
    for &(at_ms, rtt_ms) in &s.samples {
        series.push(Sample {
            // The chart axis is irrelevant here; only the absolute stamp is read.
            t_secs: 0.0,
            rtt_ms,
            fail: if is_ok(rtt_ms) {
                None
            } else {
                Some(Fail::Timeout)
            },
            at_ms,
        });
    }
    rows.extend(exporter::collect_rows(
        &s.label,
        &series,
        chrono::Local::now(),
    ));
}

pub fn collect_export_rows(targets: &[Target]) -> Vec<exporter::Row> {
    let mut rows = Vec::new();
    for t in targets {
        // Each sample knows its own time; the value passed here is only a fallback.
        rows.extend(exporter::collect_rows(
            &t.label,
            &t.series,
            chrono::Local::now(),
        ));
    }
    rows
}

impl App {
    /// Builds the app. `theme_override` is what `--theme` asked for: it picks the look for
    /// this run only and is never written back to the saved file, which is why it arrives as
    /// its own argument instead of inside `options`.
    pub fn new(options: AppOptions, saved: Vec<store::SavedTarget>) -> Self {
        Self::with_theme_override(options, saved, None)
    }

    pub fn with_theme_override(
        mut options: AppOptions,
        saved: Vec<store::SavedTarget>,
        theme_override: Option<&str>,
    ) -> Self {
        // The command line wins over the saved choice, but only for this session.
        let wanted_theme = theme_override.unwrap_or(&options.theme).to_string();
        // Only needed to ask the OS what it wants; the widget code re-checks every frame.
        let ctx_for_theme = egui::Context::default();
        let theme_mode = crate::theme::resolve(&ctx_for_theme, &wanted_theme);
        if theme_override.is_some() {
            options.theme = String::from("system");
        }
        let engine = crate::ping::PingEngine::new(options.concurrency);
        let trimmed_path = options.protocol_path.trim().to_string();
        let protocol_on = options.protocol_log;
        let mut app = Self {
            options,
            targets: Vec::new(),
            selected: 0,
            view: View::Chart,
            running: false,
            engine,
            started_wall: chrono::Local::now(),
            new_host: String::new(),
            new_label: String::new(),
            filter: String::new(),
            edit_idx: None,
            edit_host: String::new(),
            edit_label: String::new(),
            edit_kind: TargetKind::Custom,
            edit_enabled: true,
            hover_x: None,
            follow_live: true,
            pan_offset_secs: 0.0,
            log: Vec::new(),
            log_total: 0,
            alert_count: 0,
            next_probe_at: BTreeMap::new(),
            last_tick_secs: 0.0,
            save_due_secs: None,
            pending_export: Arc::new(AtomicBool::new(false)),
            export_result: Arc::new(MutexSlot::new()),
            autostart_state: Arc::new(AtomicUsize::new(usize::MAX)),
            status_msg: String::from(
                "Bereit. Ziel hinzufügen und mit „Start“ die Messung beginnen.",
            ),
            chart_p95: Vec::new(),
            stats_scratch: Vec::new(),
            row_rects: Vec::new(),
            row_label_gap: 0.0,
            row_heights: Vec::new(),
            protocol: if protocol_on {
                let path = if trimmed_path.is_empty() {
                    store::state_dir().join("protocol.csv")
                } else {
                    PathBuf::from(trimmed_path)
                };
                Some(crate::exporter::ProtocolFile::new(
                    path,
                    crate::exporter::ProtocolFormat::Csv,
                ))
            } else {
                None
            },
            protocol_retry_secs: 0.0,
            last_fail: Vec::new(),
            theme_mode,
        };

        if saved.is_empty() {
            app.targets = default_targets();
        } else {
            for (next_id, t) in saved.into_iter().enumerate() {
                let id = next_id;
                let mut tgt = Target::new(id, t.label, t.host, t.kind);
                tgt.enabled = t.enabled;
                for (at_ms, rtt) in &t.samples {
                    // `t_secs` is filled by `reanchor` below: the stored seconds belong to a
                    // session that has already ended.
                    tgt.series.push(Sample {
                        t_secs: 0.0,
                        rtt_ms: *rtt,
                        fail: if is_ok(*rtt) {
                            None
                        } else {
                            Some(Fail::Timeout)
                        },
                        at_ms: *at_ms,
                    });
                }
                tgt.series.reanchor();
                if let Some(last) = tgt.series.samples().last() {
                    tgt.last_probe_secs = last.t_secs;
                    tgt.last_rtt = last.rtt_ms;
                }
                app.targets.push(tgt);
            }
        }
        app.sync_caps();
        // Ask Windows once, in the background, whether a Run entry exists. Until it answers the
        // button shows „aus“; afterwards `poll_autostart` keeps it honest.
        app.check_autostart_once();
        app
    }

    pub fn sync_caps(&mut self) {
        let cap = self.options.max_samples;
        for t in &mut self.targets {
            t.series.set_cap(cap);
        }
    }

    /// Hand settings the engine owns over to it. Called whenever the settings panel reports a
    /// change, so „Gleichzeitige Prüfungen“ takes effect immediately instead of on the next
    /// launch. The engine clamps the number itself.
    pub fn sync_engine(&mut self) {
        self.engine.set_concurrency(self.options.concurrency);
    }

    pub fn selected_target(&self) -> Option<&Target> {
        self.targets.get(self.selected)
    }

    pub fn next_probe_secs(&self, id: usize) -> f64 {
        self.next_probe_at.get(&id).copied().unwrap_or(0.0)
    }

    /// Probe every enabled target as fast as the interval and timeout allow.
    pub fn tick(&mut self, now_secs: f64) {
        // Drain results first so state stays fresh even while paused.
        let results = self.engine.drain_results();
        for r in results {
            self.apply_result(r);
        }

        // A finished CSV export reports itself here; nothing else picks the message up, and
        // without this the status bar would read „Schreibe CSV-Datei …“ forever.
        self.take_export_result();
        // Same for the autostart lookup that started at launch.
        self.poll_autostart();

        if !self.running {
            return;
        }

        // A probe must never overlap its own timeout, or samples arrive doubled.
        let min_gap = (self.options.interval_secs as f64)
            .max((self.options.timeout_ms as f64 / 1000.0) + 0.25);

        let mut jobs: Vec<ProbeJob> = Vec::new();
        // The still picture must show exactly the seeded history, so a run that is going to
        // be captured never probes. `--shot` also closes after two passes, which is normally
        // too fast for a probe to land anyway; this makes it certain instead of a race.
        if crate::screenshot::capture_armed() {
            return;
        }
        for t in &mut self.targets {
            if !t.enabled {
                continue;
            }
            let due = self.next_probe_at.get(&t.id).copied().unwrap_or(0.0);
            if now_secs >= due {
                jobs.push(ProbeJob {
                    id: t.id,
                    host: t.host.clone(),
                    timeout_ms: self.options.timeout_ms,
                });
                self.next_probe_at.insert(t.id, now_secs + min_gap);
            }
        }
        for j in jobs {
            self.engine.start(j);
        }

        // A protocol file that went bad says so once (the flag clears on a successful reopen).
        if let Some(p) = &self.protocol {
            if let Some(err) = p.failed.clone() {
                self.status_msg = format!("Protokoll-Datei nicht schreibbar: {err}");
            }
        }

        // Periodic housekeeping + autosave.
        if now_secs - self.last_tick_secs > 1.0 {
            self.last_tick_secs = now_secs;
            let horizon = self.options.horizon_secs as f64 * 3.0 + 30.0;
            for t in &mut self.targets {
                t.series.trim_to_window(horizon);
            }
        }
        // Autosave. The due time is armed by the first tick, so a pass that never ticks (or a
        // process whose clock starts at an arbitrary point) cannot save before it collected.
        match self.save_due_secs {
            Some(due) if now_secs >= due => {
                self.save_due_secs = Some(now_secs + 8.0);
                self.save_now();
            }
            Some(_) => {}
            None => self.save_due_secs = Some(now_secs + 8.0),
        }
    }

    fn apply_result(&mut self, r: ProbeResult) {
        let Some(idx) = self.targets.iter().position(|t| t.id == r.id) else {
            return;
        };
        // Sample times are on the app clock so the chart x axis is continuous.
        let t_secs = self.engine.elapsed_secs();
        let _ = r.t_secs;

        // The protocol file gets every measurement, before anything can stop the write.
        if self.protocol.is_some() {
            let label = self.targets[idx].label.clone();
            self.protocol_write(idx, t_secs, &label, r.rtt_ms);
        }

        // Work out what to log first, then write it without holding two &mut borrows.
        enum Act {
            None,
            Recovery(String),
            Outage(String),
        }
        let mut act = Act::None;
        {
            let t = &mut self.targets[idx];
            let fail = r.fail.map(Fail::from);
            // Remember why this probe failed; the protocol line uses it for failures.
            if let Some(slot) = self.last_fail.get_mut(idx) {
                *slot = fail;
            }
            t.series.record(t_secs, r.at_ms, r.rtt_ms, fail);
            t.last_probe_secs = t_secs;
            t.last_rtt = r.rtt_ms;

            if is_ok(r.rtt_ms) {
                if t.streak_bad > 0 && self.options.sound_alerts {
                    act = Act::Recovery(format!(
                        "{} ist wieder erreichbar ({} ms).",
                        t.label,
                        fmt_ms(r.rtt_ms)
                    ));
                }
                t.streak_bad = 0;
                t.alert_raised = false;
            } else {
                t.streak_bad += 1;
                if t.streak_bad >= 2 && !t.alert_raised {
                    t.alert_raised = true;
                    let why = match fail {
                        Some(Fail::Resolve) => "DNS schlägt fehl",
                        Some(Fail::Error) => "Ping-Fehler",
                        _ => "Timeout",
                    };
                    act = Act::Outage(format!("{} antwortet nicht ({}).", t.label, why));
                }
            }
        }
        match act {
            Act::None => {}
            Act::Recovery(text) => self.push_log(t_secs, text, LogKind::Recovery),
            Act::Outage(text) => {
                self.alert_count += 1;
                self.push_log(t_secs, text, LogKind::Outage);
            }
        }
    }

    /// Append every measurement of one probe to the protocol file, if one is on.
    fn protocol_write(&mut self, idx: usize, t_secs: f64, label: &str, rtt_ms: f32) {
        // Nothing to write while the option is off, and no retry storm while a path is bad.
        let now = crate::runtime::secs_since_start();
        let dead = match &self.protocol {
            Some(p) => !p.is_open(),
            None => return,
        };
        if dead {
            if now - self.protocol_retry_secs < 30.0 {
                return;
            }
            self.protocol_retry_secs = now;
            if let Some(p) = &mut self.protocol {
                p.open();
                if !p.is_open() {
                    return;
                }
                p.clear_failed();
            }
        }
        // Stored stamps are seconds since start; the protocol shows wall-clock time, so the
        // app's own start is the anchor (same reckoning as the CSV export).
        let at = self.started_wall + chrono::Duration::milliseconds((t_secs * 1000.0) as i64);
        let row = exporter::Row {
            timestamp: at.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
            target: label.to_string(),
            latency_ms: if is_ok(rtt_ms) { Some(rtt_ms) } else { None },
            status: if is_ok(rtt_ms) {
                String::from("ok")
            } else {
                self.last_fail
                    .get(idx)
                    .copied()
                    .flatten()
                    .map(Fail::short)
                    .unwrap_or("timeout")
                    .to_string()
            },
        };
        if let Some(p) = &mut self.protocol {
            p.write(&row);
        }
    }

    /// Turn the protocol file on or off. `path` empty means "next to the saved data".
    pub fn set_protocol(&mut self, on: bool, format: crate::exporter::ProtocolFormat) {
        if !on {
            self.protocol = None;
            return;
        }
        let path = if self.options.protocol_path.trim().is_empty() {
            store::state_dir().join("protocol.csv")
        } else {
            PathBuf::from(self.options.protocol_path.trim())
        };
        self.protocol = Some(crate::exporter::ProtocolFile::new(path, format));
        self.protocol_retry_secs = 0.0;
    }

    fn push_log(&mut self, secs: f64, text: String, kind: LogKind) {
        if !secs.is_finite() || secs <= 0.0 {
            return;
        }
        self.log_total += 1;
        self.log.push(LogEntry { secs, text, kind });
        if self.log.len() > 400 {
            let cut = self.log.len() - 300;
            self.log.drain(0..cut);
        }
    }

    /// Convenience for UI/screenshot code that only has a message.
    pub fn log_info(&mut self, text: impl Into<String>) {
        let secs = self.engine.elapsed_secs();
        self.push_log(secs, text.into(), LogKind::Info);
    }

    /// Convenience for UI/screenshot code (outage / recovery lines).
    pub fn log_event(&mut self, kind: LogKind, text: impl Into<String>) {
        let secs = self.engine.elapsed_secs();
        if kind == LogKind::Outage {
            self.alert_count += 1;
        }
        self.push_log(secs, text.into(), kind);
    }

    /// Seconds since the app started (the chart's x axis).
    pub fn now_secs(&self) -> f64 {
        self.engine.elapsed_secs()
    }

    /// Jumps the chart window onto the next sample above its target's outlier cut,
    /// starting after the visible window: where the next outlier occurred.
    pub fn jump_to_next_outlier(&mut self) {
        let horizon = self.options.horizon_secs as f64;
        let now = self.engine.elapsed_secs();
        let t_max = now - self.pan_offset_secs;
        self.seek_outlier(now, horizon, t_max + 1.0, true);
    }

    pub fn jump_to_prev_outlier(&mut self) {
        let horizon = self.options.horizon_secs as f64;
        let now = self.engine.elapsed_secs();
        let t_min = now - self.pan_offset_secs - horizon;
        self.seek_outlier(now, horizon, t_min - 1.0, false);
    }

    pub fn seek_outlier(&mut self, now: f64, horizon: f64, from: f64, forward: bool) {
        let mut best: Option<f64> = None;
        // The cut is per target and over all its stored samples, so it only has to be
        // worked out once each; then the nearest outlier sample is scanned for in place.
        // No vector of candidate times is built here - this runs on a button press, but
        // one pass per target with no allocation is both cheaper and shorter.
        for t in self.targets.iter().filter(|t| !t.series.is_empty()) {
            let (cut, n_cut) = history::outlier_cut(t.series.samples());
            if !cut.is_finite() || n_cut == 0 {
                continue;
            }
            for s in t.series.samples() {
                if !history::is_ok(s.rtt_ms) || (s.rtt_ms as f64) <= cut {
                    continue;
                }
                let t_secs = s.t_secs;
                let candidate = if forward {
                    t_secs >= from
                } else {
                    t_secs <= from
                };
                if !candidate {
                    continue;
                }
                match best {
                    Some(b) => {
                        if (t_secs - from).abs() < (b - from).abs() {
                            best = Some(t_secs);
                        }
                    }
                    None => best = Some(t_secs),
                }
            }
        }
        match best {
            Some(t) => {
                self.pan_offset_secs = (now - t - horizon * 0.5).clamp(0.0, now.max(0.0));
                self.follow_live = self.pan_offset_secs < 0.5;
            }
            None => self.log_info("keine Auffälligkeiten in den gespeicherten Daten"),
        }
    }

    pub fn start(&mut self) {
        self.running = true;
        // Stagger the first probes so a big list doesn't fire all at once.
        let now = self.engine.elapsed_secs();
        for (i, t) in self.targets.iter().enumerate() {
            if t.enabled {
                self.next_probe_at
                    .insert(t.id, now + (i as f64 * 0.12).min(1.5));
            }
        }
        self.status_msg = String::from("Messung läuft.");
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.engine.stop_all();
        self.status_msg = String::from("Messung angehalten.");
    }

    pub fn add_target(&mut self) {
        // The editor keeps its target by index, so the list must not grow underneath it.
        if self.edit_idx.is_some() {
            self.status_msg =
                String::from("Zuerst die laufende Bearbeitung abschließen oder abbrechen.");
            return;
        }
        let host = self.new_host.trim().to_string();
        if host.is_empty() {
            self.status_msg = String::from("Bitte eine Adresse eingeben (z. B. 192.168.1.1).");
            return;
        }
        let label = if self.new_label.trim().is_empty() {
            guess_label(&host)
        } else {
            self.new_label.trim().to_string()
        };
        let id = self.targets.iter().map(|t| t.id + 1).max().unwrap_or(0);
        let kind = classify(&host);
        self.targets
            .push(Target::new(id, label, host.clone(), kind));
        self.new_host.clear();
        self.new_label.clear();
        self.selected = self.targets.len() - 1;
        self.status_msg = format!("{host} wurde hinzugefügt.");
    }

    /// Add a target with an explicit label (used by the quick-pick buttons).
    pub fn add_named_target(&mut self, host: &str, label: &str) {
        self.new_host = host.to_string();
        self.new_label = label.to_string();
        self.add_target();
    }

    /// The click areas of the rows, in target order, as egui saw them in the last pass. The click
    /// test uses them to aim a synthetic pointer at a row; `ui::left_body()` fills them.
    pub fn row_rects(&self) -> &[egui::Rect] {
        &self.row_rects
    }

    /// Open the inline editor for a target. Called when a row in the left panel is clicked, so a
    /// single click is enough to reach the settings.
    ///
    /// An editor that was already open is saved first, so switching rows never loses an edit. The
    /// commit closes the editor, which is why it has to happen before the new row is loaded here.
    pub fn begin_edit(&mut self, idx: usize) {
        if self.targets.get(idx).is_some() {
            self.close_edit();
        }
        if let Some(t) = self.targets.get(idx) {
            self.selected = idx;
            self.edit_idx = Some(idx);
            self.edit_host = t.host.clone();
            self.edit_label = t.label.clone();
            self.edit_kind = t.kind;
            self.edit_enabled = t.enabled;
        }
    }

    /// Close the editor and commit what is in it. Used when the user clicks a second target, so
    /// edits are never lost by switching rows.
    pub fn close_edit(&mut self) {
        if self.edit_idx.is_some() {
            self.commit_edit();
        }
    }

    /// Save the edits made in the inline editor. A changed host restarts that target's probes;
    /// a changed label only renames it. An empty host reverts to the old one.
    pub fn commit_edit(&mut self) {
        let Some(idx) = self.edit_idx else { return };
        let Some(t) = self.targets.get_mut(idx) else {
            self.edit_idx = None;
            return;
        };

        let new_host = self.edit_host.trim().to_string();
        if new_host.is_empty() {
            self.status_msg = String::from("Host darf nicht leer sein — Änderung verworfen.");
            self.edit_idx = None;
            return;
        }

        let old_host = t.host.clone();
        let host_changed = new_host != old_host;
        let label_changed = !self.edit_label.trim().is_empty() && self.edit_label.trim() != t.label;
        let was_enabled = t.enabled;

        if host_changed {
            // Stop the old probes, then restart with the new address.
            self.engine.stop_target(t.id);
            self.next_probe_at.remove(&t.id);
            t.host = new_host.clone();
            t.kind = classify(&new_host);
            t.last_rtt = f32::NAN;
            t.last_probe_secs = f64::NAN;
            t.streak_bad = 0;
        }

        if label_changed {
            t.label = self.edit_label.trim().to_string();
        }

        // The category is automatic, but an explicit choice wins: the picker in the editor lets
        // a user relabel a device that `classify` cannot recognise.
        if !host_changed && self.edit_kind != t.kind {
            t.kind = self.edit_kind;
        }

        // Pausing stops the running probe and drops the schedule entry; enabling puts the target
        // back into the rotation on the next tick.
        if self.edit_enabled != was_enabled {
            t.enabled = self.edit_enabled;
            if !self.edit_enabled {
                self.engine.stop_target(t.id);
                self.next_probe_at.remove(&t.id);
            } else if self.running {
                let now = self.engine.elapsed_secs();
                self.next_probe_at.insert(t.id, now);
            }
        }

        self.status_msg = if host_changed && label_changed {
            format!("{old_host} → {new_host}, umbenannt.")
        } else if host_changed {
            format!("{old_host} → {new_host}.")
        } else if label_changed {
            format!("Umbenannt zu {}.", t.label)
        } else if t.enabled != was_enabled {
            if t.enabled {
                format!("{} wird wieder gemessen.", t.label)
            } else {
                format!("{} pausiert.", t.label)
            }
        } else {
            String::from("Keine Änderung.")
        };

        self.edit_idx = None;
        self.save_now();
    }

    /// Discard the inline editor without saving.
    pub fn cancel_edit(&mut self) {
        self.edit_idx = None;
    }

    pub fn remove_selected(&mut self) {
        if self.targets.is_empty() {
            return;
        }
        // Removing a row would shift the index the editor points at.
        if self.edit_idx.is_some() {
            self.status_msg =
                String::from("Zuerst die laufende Bearbeitung abschließen oder abbrechen.");
            return;
        }
        let i = self.selected.min(self.targets.len() - 1);
        let t = &self.targets[i];
        self.engine.stop_target(t.id);
        self.next_probe_at.remove(&t.id);
        let label = t.label.clone();
        self.targets.remove(i);
        if self.selected >= self.targets.len() {
            self.selected = self.targets.len().saturating_sub(1);
        }
        self.status_msg = format!("{label} wurde entfernt.");
    }

    pub fn reset_stats(&mut self) {
        for t in &mut self.targets {
            t.series = Series::new(self.options.max_samples);
            t.last_rtt = f32::NAN;
            t.last_probe_secs = f64::NAN;
            t.streak_bad = 0;
        }
        self.log.clear();
        self.log_total = 0;
        self.alert_count = 0;
        self.status_msg = String::from("Messwerte gelöscht.");
    }

    pub fn export_csv(&mut self, path: PathBuf) {
        if self.pending_export.load(Ordering::Acquire) {
            self.status_msg = String::from("Export läuft bereits …");
            return;
        }
        let rows = collect_export_rows(&self.targets);
        if rows.is_empty() {
            self.status_msg = String::from("Es gibt noch keine Messwerte zum Exportieren.");
            return;
        }
        self.pending_export.store(true, Ordering::Release);
        let flag = Arc::clone(&self.pending_export);
        let slot = Arc::clone(&self.export_result);
        std::thread::spawn(move || {
            let msg = match exporter::write_csv(&path, &rows) {
                Ok(n) => format!("CSV-Export geschrieben: {} Messwerte.", n),
                Err(e) => e,
            };
            slot.put(msg);
            flag.store(false, Ordering::Release);
        });
        self.status_msg = String::from("Schreibe CSV-Datei …");
    }

    pub fn take_export_result(&mut self) {
        if let Some(msg) = self.export_result.take() {
            self.status_msg = msg;
        }
    }

    /// Ask the background thread whether the autostart entry exists, and correct the option if
    /// the registry disagrees with what we assumed. Called once per frame from `tick`. The value
    /// is kept in the atomic so the top-bar button can read it every frame; this function only
    /// updates `options.autostart` when the answer arrives for the first time.
    pub fn poll_autostart(&mut self) {
        let v = self.autostart_state.load(Ordering::Acquire);
        if v == usize::MAX {
            return;
        }
        // The value stays in place — the button reads it every frame. We only sync options once.
        let found = v == 1;
        let wanted = self.options.autostart;
        if wanted != found {
            self.options.autostart = found;
            self.status_msg = if found {
                String::from("Autostart ist aktiv (Windows-Registry).")
            } else {
                String::from("Autostart ist deaktiviert.")
            };
        }
    }

    /// Start the registry lookup in the background. Only ever run once at startup: after that
    /// the button owns the value, and asking again would overwrite what the user just clicked.
    pub fn check_autostart_once(&self) {
        let slot = Arc::clone(&self.autostart_state);
        std::thread::spawn(move || {
            let found = autostart_exists();
            slot.store(if found { 1 } else { 0 }, Ordering::Release);
        });
    }

    /// Whether the autostart entry exists. `Some(true)` / `Some(false)` once the registry has
    /// answered, `None` while the check is still running. Unlike the other one-shot results this
    /// value is *kept* — the top-bar button reads it every frame, so consuming it would make the
    /// label flicker. `poll_autostart()` is what consumes it to correct `options.autostart`.
    /// `pub` rather than `pub(crate)`: the integration tests in `tests/` compile against the
    /// crate as an outside user and must see the same answer the button sees.
    pub fn autostart_state(&self) -> Option<bool> {
        match self.autostart_state.load(Ordering::Acquire) {
            usize::MAX => None,
            v => Some(v == 1),
        }
    }

    pub fn set_autostart(&mut self, on: bool) {
        self.options.autostart = on;
        let slot = Arc::clone(&self.autostart_state);
        std::thread::spawn(move || {
            let ok = set_autostart(on);
            slot.store(if ok { 1 } else { 0 }, Ordering::Release);
        });
    }

    pub fn save_now(&mut self) {
        let _ = self.save_now_to(&store::state_dir());
    }

    /// `save_now` into a directory given by the caller, so a headless pass keeps its files to
    /// itself even while another thread points the store somewhere else. Returns what landed.
    pub fn save_now_to(&self, dir: &std::path::Path) -> std::io::Result<Vec<store::SavedTarget>> {
        let saved: Vec<store::SavedTarget> = self
            .targets
            .iter()
            .map(|t| store::SavedTarget {
                id: t.id,
                label: t.label.clone(),
                host: t.host.clone(),
                kind: t.kind,
                enabled: t.enabled,
                samples: t
                    .series
                    .samples()
                    .iter()
                    .map(|s| (s.at_ms, s.rtt_ms))
                    .collect(),
            })
            .collect();
        store::save_to(dir, &self.options, &saved)?;
        Ok(saved)
    }

    pub fn stats_of(&self, id: usize) -> Option<Stats> {
        let horizon = self.options.horizon_secs as f64;
        let t = self.targets.iter().find(|t| t.id == id)?;
        // Same window as the panels. This is a one-off call from outside the frame loop, so
        // a fresh small vector here is fine; the per-frame views reuse `App::stats_scratch`.
        let mut scratch = Vec::new();
        Some(t.series.stats_window(
            t.series.last().map_or(0.0, |s| s.t_secs - horizon),
            f64::MAX,
            &mut scratch,
        ))
    }

    pub fn visible_targets(&self) -> Vec<usize> {
        let f = self.filter.trim().to_lowercase();
        self.targets
            .iter()
            .filter(|t| {
                f.is_empty()
                    || t.label.to_lowercase().contains(&f)
                    || t.host.to_lowercase().contains(&f)
            })
            .map(|t| t.id)
            .collect()
    }

    /// Colour a status dot: the last measurement is what matters here.
    pub fn status_color(&self, t: &Target) -> Color32 {
        self.theme_mode
            .status_dot(!t.enabled, self.running, t.last_rtt)
    }

    /// Colour of one target's trace. Unused by the chart, which colours each sample by its own
    /// latency band — a whole-line colour must not depend on `last_rtt`, because that value
    /// only refreshes when a probe comes back, so the first samples would be drawn in the
    /// "waiting" grey and stay invisible against the plot background until the first reply
    /// arrives. Kept for callers that want one representative colour per target.
    pub fn trace_color(&self, t: &Target) -> Color32 {
        self.status_color(t)
    }

    pub fn status_text(&self, t: &Target) -> String {
        if !t.enabled {
            return "pausiert".into();
        }
        if t.last_rtt.is_nan() {
            return if self.running {
                "wartet …".into()
            } else {
                "—".into()
            };
        }
        if is_ok(t.last_rtt) {
            format!("{} ms", fmt_ms(t.last_rtt))
        } else {
            "Timeout".into()
        }
    }
}

pub fn fmt_ms(v: f32) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn default_targets() -> Vec<Target> {
    vec![
        Target::new(0, "Gateway", gateway_hint(), TargetKind::Gateway),
        Target::new(1, "DNS (Google)", "8.8.8.8", TargetKind::Dns),
        Target::new(2, "Internet", "1.1.1.1", TargetKind::Internet),
    ]
}

/// Best-effort default gateway from `ipconfig` / `route`; falls back to the
/// common home-router address. Never blocks longer than a second.
fn gateway_hint() -> String {
    #[cfg(windows)]
    {
        if let Some(g) = windows_gateway() {
            return g;
        }
    }
    String::from("192.168.1.1")
}

#[cfg(windows)]
fn windows_gateway() -> Option<String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("route");
    cmd.args(["print", "0.0.0.0"]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000);
    let out = cmd.output().ok()?;
    let text: String = String::from_utf8_lossy(&out.stdout).to_string();
    for line in text.lines() {
        let l = line.trim();
        if !l.starts_with("0.0.0.0") {
            continue;
        }
        // Columns differ by language; the gateway is a dotted quad that is not
        // the netmask (which is 0.0.0.0 or 255.*).
        let mut ips: Vec<String> = Vec::new();
        for tok in l.split_whitespace() {
            if tok.matches('.').count() == 3 && tok.chars().all(|c| c.is_ascii_digit() || c == '.')
            {
                ips.push(tok.to_string());
            }
        }
        for ip in ips.iter().skip(2) {
            if !ip.starts_with("0.") && !ip.starts_with("255.") {
                return Some(ip.clone());
            }
        }
    }
    None
}

fn guess_label(host: &str) -> String {
    match classify(host) {
        TargetKind::Gateway => "Gateway".into(),
        _ => host.to_string(),
    }
}

fn classify(host: &str) -> TargetKind {
    if host.starts_with("192.168.") || host.starts_with("10.") || host.starts_with("172.") {
        if host.ends_with(".1") || host.ends_with(".254") {
            return TargetKind::Gateway;
        }
        return TargetKind::Lan;
    }
    if host == "8.8.8.8" || host == "1.1.1.1" || host.contains("dns") {
        return TargetKind::Dns;
    }
    TargetKind::Custom
}

/// Write or remove the HKCU Run entry that starts the app at logon.
fn set_autostart(on: bool) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(_) => return false,
        };
        let exe = exe.display().to_string();
        let mut cmd = std::process::Command::new("reg");
        if on {
            cmd.args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"]);
            cmd.args(["/v", "SystemPing", "/t", "REG_SZ", "/d"]);
            cmd.arg(format!("\"{exe}\""));
            cmd.arg("/f");
        } else {
            cmd.args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "SystemPing",
                "/f",
            ]);
        }
        cmd.creation_flags(0x0800_0000);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        false
    }
}

/// Whether the HKCU Run entry that starts the app at logon exists. This is the read side of
/// `set_autostart`; without it the button in the top bar can only ever say „aus“, because the
/// counter it reads is only ever written when the user presses it.
fn autostart_exists() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `reg query` exits 0 only when the value is really there, so its status is the whole
        // answer — no registry crate needed.
        let mut cmd = std::process::Command::new("reg");
        cmd.args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "SystemPing",
        ]);
        cmd.creation_flags(0x0800_0000);
        // The answer is not interesting, only whether the key was found.
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.status().map(|s| s.success()).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}
