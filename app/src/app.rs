//! Application state, the probe scheduler and the event plumbing.

use crate::exporter;
use crate::history::{self, is_ok, Fail, Sample, Series, Stats};
use crate::i18n::{self, Lang};
use crate::ping::{ProbeJob, ProbeResult};
use crate::store;
use crate::theme;
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

/// Position of a category in [`TargetKind::all`]. The category filter stores these numbers, so the
/// list of switched-off categories stays a plain vec of small integers.
pub fn kind_index(kind: TargetKind) -> usize {
    TargetKind::all()
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0)
}

/// Marker for "this target's colour comes from the palette, picked by its position in the list".
pub const AUTO_COLOR: usize = usize::MAX;

/// The program's version, shown in the About group. It is the one from `Cargo.toml` so the two
/// cannot drift apart.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The git commit this binary was built from, short form — filled in by `build.rs`, empty for a
/// build from a source tree without git (a tarball, say). It answers the question a bug report
/// starts with: which version, exactly?
pub const GIT_HASH: &str = env!("GIT_HASH");

/// The version as the About group shows it: `0.9.1 (1a2b3c4)`, or bare when the commit is not
/// known — empty parentheses would be a hole in the text, not an answer.
pub fn about_version() -> String {
    if GIT_HASH.is_empty() {
        APP_VERSION.to_string()
    } else {
        format!("{APP_VERSION} ({GIT_HASH})")
    }
}

/// One monitored address.
pub struct Target {
    pub id: usize,
    pub label: String,
    pub host: String,
    pub kind: TargetKind,
    pub enabled: bool,
    /// Index into [`crate::theme::PALETTE`]. `usize::MAX` means "whatever the palette gives me",
    /// i.e. a colour picked automatically from the position in the list. A manual choice stores
    /// the number and therefore survives both a restart and a reordering of the list.
    pub color_idx: usize,
    /// Hidden from the chart, legend and lists by a click on the target's name in the legend.
    /// Only presentation: the target keeps being probed and its history keeps growing. Not stored
    /// — after a restart every target is visible again.
    pub hidden: bool,
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
            color_idx: AUTO_COLOR,
            hidden: false,
            // The real capacity comes from the app's settings (`sync_caps`), which this
            // constructor cannot see. It only has to be big enough for a target that is added
            // and measured before the first sync, so it uses the same floor the settings use.
            series: Series::new(MIN_SAMPLES),
            last_probe_secs: f64::NAN,
            last_rtt: f32::NAN,
            streak_bad: 0,
            alert_raised: false,
        }
    }
}

/// User-facing settings. All persisted.
///
/// `default` on the container is for reading, not for constructing: a file that does not mention a
/// setting — one written before that setting existed, or a block a person edited down to one line —
/// gets the built-in value for what is missing instead of being rejected whole.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppOptions {
    /// Seconds between probes for every target.
    pub interval_secs: f32,
    /// Per-probe timeout in ms.
    pub timeout_ms: u32,
    /// How many probes may run at the same time.
    pub concurrency: usize,
    /// Chart window width in seconds. [`HORIZON_CHOICES`] offers presets up to a day.
    pub horizon_secs: f32,
    /// Max samples kept per target. `0` means "as many as the chart window needs" — see
    /// [`samples_for_horizon`]. That default is what makes a 24 h window possible at all: with
    /// a fixed cap of 2048 the day would be cut off after half an hour.
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
    /// Categories the chart and the lists show, as numbers into [`TargetKind::all`] — a list of
    /// indices rather than a `HashSet` so it stays trivially serialisable and keeps its order.
    /// Missing in old files -> every category is on. See [`App::all_kinds_on`].
    #[serde(default = "all_kinds_on")]
    pub kinds_off: Vec<usize>,
    /// Interface language, "de" or "en"; see [`crate::i18n::Lang`]. Missing in old files -> German,
    /// which is what every state file written before this option showed.
    #[serde(default)]
    pub language: Lang,
}

/// The default for [`AppOptions::kinds_off`]: nothing switched off, so a state file written before
/// the category filter existed keeps showing every target.
fn all_kinds_on() -> Vec<usize> {
    Vec::new()
}

/// "system", "light" or "dark": which look to use.
fn default_theme() -> String {
    String::from("system")
}

/// One choice of the chart window: a label and its length in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HorizonChoice {
    pub secs: f32,
    pub label: &'static str,
}

/// The window lengths the chart offers, shortest first. A day is the longest on purpose:
/// anything wider needs a coarser probe interval to stay readable.
pub const HORIZON_CHOICES: &[HorizonChoice] = &[
    HorizonChoice {
        secs: 15.0,
        label: "15 s",
    },
    HorizonChoice {
        secs: 30.0,
        label: "30 s",
    },
    HorizonChoice {
        secs: 60.0,
        label: "1 min",
    },
    HorizonChoice {
        secs: 120.0,
        label: "2 min",
    },
    HorizonChoice {
        secs: 300.0,
        label: "5 min",
    },
    HorizonChoice {
        secs: 600.0,
        label: "10 min",
    },
    HorizonChoice {
        secs: 900.0,
        label: "15 min",
    },
    HorizonChoice {
        secs: 1_800.0,
        label: "30 min",
    },
    HorizonChoice {
        secs: 3_600.0,
        label: "1 h",
    },
    HorizonChoice {
        secs: 7_200.0,
        label: "2 h",
    },
    HorizonChoice {
        secs: 14_400.0,
        label: "4 h",
    },
    HorizonChoice {
        secs: 21_600.0,
        label: "6 h",
    },
    HorizonChoice {
        secs: 43_200.0,
        label: "12 h",
    },
    HorizonChoice {
        secs: 86_400.0,
        label: "24 h",
    },
];

/// The stored minimum for [`AppOptions::max_samples`]. Below this a chart window of an hour
/// would not fit its own samples.
pub const MIN_SAMPLES: usize = 1_024;

/// The probe intervals the program offers, in seconds. A free slider between 0.25 s and 10 s
/// produced numbers nobody could hit on purpose (3.7 s), and the gap between two of them is not
/// meaningful — so every control that sets the interval snaps to this ladder instead. It starts
/// where a LAN reply can still be followed usefully and ends where a second probe would overlap
/// the timeout of the previous one anyway.
pub const INTERVAL_CHOICES: &[f32] = &[0.25, 0.5, 1.0, 2.0, 5.0, 10.0];

/// The label for one rung of [`INTERVAL_CHOICES`]: a quarter second reads as "0,25 s" in German and
/// "0.25 s" in English; whole seconds are written without decimals in both.
pub fn interval_label(lang: Lang, secs: f32) -> String {
    if (secs - secs.round()).abs() < f32::EPSILON {
        format!("{} {}", secs as i64, lang.seconds_unit())
    } else if lang == Lang::De {
        format!("{secs:.2} s").replace('.', ",")
    } else {
        format!("{secs:.2} s")
    }
}

/// Switch the interface language and say so in the log. The log line is written in the language
/// that results, which is what a person who has just switched wants to read.
pub fn set_language(app: &mut App, lang: Lang) {
    apply_language(app, lang, true);
}

/// Same switch, without saving or logging — used once at start-up, where the language comes from the
/// command line and must not be written back to the file, and where a log line about a language
/// nobody switched to would only be noise.
pub fn set_language_silently(app: &mut App, lang: Lang) {
    apply_language(app, lang, false);
}

fn apply_language(app: &mut App, lang: Lang, announce: bool) {
    if app.options.language == lang {
        return;
    }
    app.options.language = lang;
    if !announce {
        return;
    }
    let text = crate::i18n::fill(
        lang.log_language_fmt(),
        &[("lang", lang.native_name().to_string())],
    );
    app.log_info(text);
    app.save_now();
}

/// The interval closest to an arbitrary number, i.e. the rung of [`INTERVAL_CHOICES`] a saved or
/// typed value lands on. Used by the settings field so free input stays possible while the stored
/// value remains one of the ladder's rungs.
pub fn nearest_interval(secs: f32) -> f32 {
    INTERVAL_CHOICES
        .iter()
        .min_by(|a, b| {
            ((**a - secs).abs())
                .partial_cmp(&(**b - secs).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
        .unwrap_or(1.0)
}

/// What one rung of the interval ladder means for measuring, in one sentence. It stands as a
/// tooltip wherever the interval is set: the number alone does not say that 0.25 s is four probes a
/// second and therefore four starts of `ping.exe`.
pub fn interval_hint(lang: Lang, secs: f32) -> String {
    let per_min = 60.0 / secs.max(0.01);
    let probes = if secs < 1.0 {
        i18n::fill(
            lang.tip_probes_per_sec_fmt(),
            &[("n", ((1.0 / secs).round().max(1.0) as i64).to_string())],
        )
    } else {
        let n = interval_label(lang, secs).replace(' ', "");
        i18n::fill(lang.tip_one_probe_every_fmt(), &[("n", n)])
    };
    i18n::fill(
        lang.tip_per_minute_fmt(),
        &[("probes", probes), ("per_min", format!("{per_min:.0}"))],
    )
}

/// How many samples one target has to keep to fill `horizon_secs` at `interval_secs`, with a
/// little slack so the newest sample is never the limit. The chart window alone is not enough:
/// panning back walks through three windows of history (see the trim in `tick`), and the day
/// view would otherwise lose everything outside what is visible.
pub fn samples_for_horizon(horizon_secs: f32, interval_secs: f32) -> usize {
    let interval = interval_secs.max(0.25) as f64;
    let needed = (f64::from(horizon_secs) * 3.0 / interval).ceil() as usize + 64;
    needed.clamp(MIN_SAMPLES, 1_000_000)
}

/// The cap a target should use for the current settings: the explicit choice from the settings
/// panel, or the number the chart window needs when that choice is `0` (the default).
pub fn effective_max_samples(options: &AppOptions) -> usize {
    if options.max_samples == 0 {
        samples_for_horizon(options.horizon_secs, options.interval_secs)
    } else {
        options.max_samples.max(MIN_SAMPLES)
    }
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            interval_secs: 1.0,
            timeout_ms: 2000,
            concurrency: 4,
            horizon_secs: 60.0,
            // 0 = "as many as the chart window needs" (see `effective_max_samples`). A fixed number
            // would cut a 24 h window off after half an hour.
            max_samples: 0,
            sound_alerts: false,
            system_tray_icon: true,
            autostart: false,
            minimised_to_tray: false,
            theme: String::from("system"),
            protocol_log: false,
            protocol_path: String::new(),
            kinds_off: Vec::new(),
            language: Lang::default(),
        }
    }
}

/// A column of the statistics table that can be sorted by. Named like its heading so the UI and the
/// sorting logic use the same words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Label,
    Host,
    Min,
    Avg,
    Median,
    P95,
    Max,
    Loss,
    Outliers,
}

/// The sorting the statistics table was last ordered by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortState {
    pub column: SortColumn,
    pub descending: bool,
}

/// The number or text a target offers for one sort column. Its own type rather than an `f64`
/// comparison inside the UI: the statistics then have to be computed exactly once for the whole
/// sort, and a target without measurements can be pushed to the bottom instead of landing somewhere
/// in the middle.
#[derive(Clone, Debug)]
struct SortKey {
    text: Option<String>,
    value: f64,
}

impl SortKey {
    fn of(column: SortColumn, t: &Target, st: &crate::history::Stats) -> Self {
        let num = |v: f32| {
            if v.is_finite() && st.replies > 0 {
                f64::from(v)
            } else {
                // No measurement: always larger than any real value, so descending order puts it
                // at the bottom.
                f64::MAX
            }
        };
        match column {
            SortColumn::Label => SortKey {
                text: Some(t.label.to_lowercase()),
                value: 0.0,
            },
            SortColumn::Host => SortKey {
                text: Some(t.host.to_lowercase()),
                value: 0.0,
            },
            SortColumn::Min => SortKey {
                text: None,
                value: num(st.min),
            },
            SortColumn::Avg => SortKey {
                text: None,
                value: if st.replies > 0 { st.avg } else { f64::MAX },
            },
            SortColumn::Median => SortKey {
                text: None,
                value: num(st.median),
            },
            SortColumn::P95 => SortKey {
                text: None,
                value: num(st.p95),
            },
            SortColumn::Max => SortKey {
                text: None,
                value: num(st.max),
            },
            SortColumn::Loss => SortKey {
                text: None,
                value: st.loss_pct,
            },
            SortColumn::Outliers => SortKey {
                text: None,
                value: st.outliers as f64,
            },
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
    /// German name, used as a stable identifier by screenshot mode and the log. The window shows
    /// [`crate::i18n::Lang::view`] instead so the label follows the chosen language.
    pub fn label(&self) -> &'static str {
        match self {
            View::Chart => "Diagramm",
            View::Events => "Ereignisse",
            View::Stats => "Statistik",
            View::Settings => "Einstellungen",
        }
    }

    /// The keyboard shortcut for this view. It is printed on the button because a shortcut nobody
    /// knows is not one — and because the digit on the button would otherwise be an unexplained
    /// number.
    pub fn shortcut(&self) -> &'static str {
        match self {
            View::Chart => "1",
            View::Events => "2",
            View::Stats => "3",
            View::Settings => "4",
        }
    }

    /// The view a shortcut key selects, or `None` for a digit that means nothing.
    pub fn from_shortcut(key: &str) -> Option<View> {
        match key {
            "1" => Some(View::Chart),
            "2" => Some(View::Events),
            "3" => Some(View::Stats),
            "4" => Some(View::Settings),
            _ => None,
        }
    }
}

pub struct App {
    pub options: AppOptions,
    pub targets: Vec<Target>,
    pub selected: usize,
    /// What the statistics table was last sorted by, and in which direction. `None` means: the
    /// order the targets were added in. Deliberately not persisted — after a restart the list is
    /// back in the order it has in the file too.
    pub sort: Option<SortState>,
    pub view: View,
    pub running: bool,
    pub engine: crate::ping::PingEngine,
    pub started_wall: chrono::DateTime<chrono::Local>,

    // UI text buffers
    pub new_host: String,
    pub new_label: String,
    pub filter: String,

    /// The target being edited in the inline editor, if any. Set by clicking a row in the left
    /// panel; cleared when the user commits or cancels. `edit_idx` is the index into `targets`, which
    /// is kept in step with the entry it names: `add_target` and `remove_selected` refuse to touch the
    /// list while something is being edited, and a drag that shuffles the list moves it along — see
    /// [`App::move_target`] and [`App::edit_id`].
    pub edit_idx: Option<usize>,
    pub edit_host: String,
    pub edit_label: String,
    /// Category picked in the editor. Normally filled in by `classify`, but it can be overridden.
    pub edit_kind: TargetKind,
    /// Whether this target is measured. Turning it off pauses probing without deleting history.
    pub edit_enabled: bool,
    /// Colour picked in the editor; [`AUTO_COLOR`] keeps the automatic choice.
    pub edit_color_idx: usize,
    /// Set when the colour picker changed something. The panel that owns the editor saves the
    /// state and clears it, because a colour is stored per target and not in `AppOptions`.
    pub color_dirty: bool,

    /// The the "clear measurements" button asks for a second click instead of deleting on the first:
    /// the first click only arms it and turns its label into a warning. See [`Self::clear_at`].
    pub clear_armed: bool,
    /// When [`Self::clear_armed`] was set. The arming lapses after a few seconds so an ignored
    /// warning cannot delete history minutes later by accident.
    pub clear_at: Option<std::time::Instant>,

    // Chart interaction
    pub hover_x: Option<f64>,
    pub follow_live: bool,
    /// Ids of the two new-target fields, filled while the top bar is drawn. The Enter shortcut
    /// compares `Memory::focused` against them (see `ui::NEW_HOST_ID_SALT`).
    pub new_target_ids: [egui::Id; 2],
    pub pan_offset_secs: f64,

    // Log of notable events (outages, recoveries).
    pub log: Vec<LogEntry>,
    pub log_total: usize,
    /// When on, hides the informational lines from the log. View only — the list itself stays
    /// complete, otherwise "clear" would be the only way to narrow it down.
    pub log_only_outages: bool,

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
    /// The "save as…" dialog: whether one is open, and its answer. A channel rather than a
    /// slot because the dialog's result may legitimately be `None` (the user cancelled). Only the
    /// receiving end is kept; the sending half is handed to the dialog thread each time. It never
    /// leaves the UI thread — the dialog thread only ever holds its own sender — so an `Rc` is
    /// enough and Clippy agrees that an `Arc` here would be a promise the type cannot keep.
    pending_dialog: Arc<AtomicBool>,
    export_dialog: std::rc::Rc<std::sync::mpsc::Receiver<Option<PathBuf>>>,
    /// Registry answer for the autostart entry, written by a background thread. Read through
    /// `autostart_state()`, which turns "not yet looked up" into `None`.
    autostart_state: Arc<AtomicUsize>,

    /// Session clock of the last write to the state file — the autosave in [`Self::tick`] as well
    /// as every explicit save. Drives "saved N s ago" in the status bar, which is what tells the
    /// user a change is on disk; a plain "save" button next to it was never needed.
    pub last_save_secs: f64,

    pub status_msg: String,
    /// What the last action said, kept apart from what reported by itself. Two lines share one status bar, and they compete for
    /// it every frame; this is what settles that. Automatic lines (a protocol file that went bad, a save that did not go
    /// through, a registry answer) write into
    /// [`Self::status_auto`] and go away on their own — the protocol warning returns for its whole 30 s of retrying. Actions
    /// write here instead, through [`Self::say`], and stay until something newer replaces them. The bar shows this whenever it is
    /// not empty, which is why a click wins: see the end of [`Self::tick`].
    status_note: Option<String>,
    /// What reported by itself most recently. Kept so one tick's warning cannot be handed to the bar as if it were the last word:
    /// only this line may come back on the next frame, never a sentence the user caused.
    status_auto: Option<String>,
    /// Scratch buffer for the per-frame p95 scan in `ui::chart`, so the chart doesn't
    /// allocate a fresh vector every frame.
    pub chart_p95: Vec<f32>,
    /// Scratch buffer for [`history::Series::stats_window`], reused every frame.
    pub stats_scratch: Vec<f32>,
    /// One millisecond value per target: the threshold above which a measurement counts as an
    /// outlier. Filled once per frame by the chart and read by both the curves and the hover read-
    /// out, so the same computation does not run several times per picture.
    pub cut_scratch: Vec<f64>,
    /// Every latency of the shown targets in one vector, for the median and p95 of the totals row.
    /// Only as long as the number of replies together — see [`App::stats_total`].
    pub all_rtt_scratch: Vec<f32>,
    /// Last theme the widgets were drawn for, so a switch re-tints everything.
    pub theme_mode: crate::theme::Mode,
    /// The click area of every row in the target list, as reported by `ui::left_body` during the last pass: one entry per row
    /// *of [`App::targets`]*, counted after the live move of that pass. A row the panel did not draw — an open editor's form, or
    /// one below a scrollbar's edge — is `None` rather than left out, because every consumer counts rows by position here and a
    /// hole silently shifts all the places below it up by one.
    ///
    /// Counted in the list rather than in the order the rows were drawn: a pass publishes its layout only once its live move has
    /// been applied (see `move_now` in `ui::left_body`), so from then on the two are the same order again and `row_rects[row]`
    /// means "the rect of `targets[row]`" — which is the only contract a pointer position can be read against without also naming
    /// the layout it was read in. See [`App::row_of`].
    pub row_rects: Vec<Option<egui::Rect>>,
    /// Height of every row in the target list from the last pass, counted in the same rows as [`App::row_rects`]. While a
    /// row is being edited its frame is replaced by a much taller form; remembering the old height keeps that row's click
    /// area where it was, so the rows below it do not move out from under the mouse.
    pub row_heights: Vec<f32>,
    /// Which entry each row of [`App::row_rects`] is, in the same order — normally just the ids of [`App::targets`]. The rects
    /// alone say where a row was; this says what it was, which is how anything that knows an entry by id asks the layout about
    /// it (see [`App::row_of`]). It is stored rather than read from `targets` because a pass can publish its layout while the
    /// list behind it has rows the panel never drew.
    pub row_ids: Vec<usize>,
    /// The two last published layouts of the target list, oldest first — each one pairing a row's click strip with the entry it showed.
    ///
    /// A single layout is enough while a drag only has to follow the pointer, because the newest one is always what the hand can see. It stops
    /// being enough at the moment the press ends: that press began in an older layout, and if the list was redrawn in between — by a live move, or
    /// because an editor closed and pulled every row under it back up — the newest one no longer has a strip where the pointer came to rest. Keeping
    /// the previous pass's as well is what lets `ui::release_slot` answer such a release for the layout the hand actually travelled over while
    /// still counting the carried entry's own place in the current one. Two generations are all a gesture can reach: only one pass lies between
    /// reading a pointer and moving the list it aimed at.
    pub drag_layouts: Vec<crate::ui::RowLayout>,
    /// The row currently being dragged to another place in the list, as its id — not as an index,
    /// because the point of a drag is that indices change while it is happening. `ui::left_body`
    /// sets and clears it; [`App::drop_moved_target`] ends the drag on release.
    pub drag_row: Option<usize>,
    /// Where the window was during the last drawn pass, so a save at any moment — including the
    /// one on exit — can write it. `None` before the first frame and for a hidden window.
    pub window_geom: Option<store::WindowGeom>,
}

/// The window's place as egui sees it: `content_rect` is the content area in logical points, and
/// `pixels_per_point` relates that to the physical pixels [`eframe::egui::ViewportBuilder`] wants
/// for its position. Together they round-trip through the state file without a winit handle, which
/// `eframe::App::on_exit` does not offer.
///
/// A window that is hidden or minimised reports an empty content rect; `WindowGeom::sane` rejects
/// that, so such a pass simply does not update the remembered place.
pub fn window_geom(ctx: &egui::Context) -> Option<store::WindowGeom> {
    let (rect, ppu) = ctx.input(|i| (i.content_rect(), i.pixels_per_point()));
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        // A hidden or minimised window reports an empty area; keep whatever was last good.
        return None;
    }
    let ppu = if ppu > 0.0 { ppu } else { 1.0 };
    let g = store::WindowGeom {
        x: (rect.left() * ppu) as i32,
        y: (rect.top() * ppu) as i32,
        width: rect.width(),
        height: rect.height(),
    };
    g.sane().then_some(g)
}

impl App {
    /// One frame of the live window: advance the engine, then paint. This is the whole
    /// measurement loop of the GUI — `tick` is what schedules probes and moves results into
    /// the series the chart draws, so a frame that skips it paints an empty plot forever.
    /// It lives here (and not only in `eframe::App::ui`) so a test can drive a real frame
    /// without a window: egui hands the root `Ui` of a pass out of
    /// [`egui::Context::run_ui`], and that is what the tests pass in.
    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        // Checked on every pass because `on_exit` has no window handle of its own; the last
        // frame before the close is what puts the window back next time. Cheap, and a hidden
        // window reports no content rect (see `window_geom`).
        self.window_geom = window_geom(ui.ctx());
        let now = self.engine.elapsed_secs();
        self.tick(now);
        // The "clear measurements" warning lapses on its own, also when the view is elsewhere or
        // nothing is clicked. Checked per frame so no repaint has to be forced for it.
        self.expire_clear_armed();
        crate::ui::draw(ui, self);
        // A live monitor repaints even when nobody touches it: samples arrive on their own,
        // and egui would otherwise sit idle until the next mouse event.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A look chosen in the settings has to be installed before anything is painted, or the
        // frame would use the old accent colours for one pass.
        let want = crate::theme::resolve(ui.ctx(), &self.options.theme);
        if want != self.theme_mode {
            self.theme_mode = want;
            crate::theme::apply(ui.ctx(), want);
        }
        self.frame_ui(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // The last write of the run also carries the window's place, so the next start can put
        // it back where it was.
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
        // Goes to the terminal, not to the window: this path runs without one.
        return Err(String::from("there are no measurements to export yet."));
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
            sort: None,
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
            edit_color_idx: AUTO_COLOR,
            color_dirty: false,
            clear_armed: false,
            clear_at: None,
            hover_x: None,
            follow_live: true,
            pan_offset_secs: 0.0,
            log: Vec::new(),
            log_total: 0,
            log_only_outages: false,
            alert_count: 0,
            next_probe_at: BTreeMap::new(),
            last_tick_secs: 0.0,
            save_due_secs: None,
            pending_export: Arc::new(AtomicBool::new(false)),
            export_result: Arc::new(MutexSlot::new()),
            pending_dialog: Arc::new(AtomicBool::new(false)),
            // An empty channel to start with; `open_export_dialog` swaps in the real receiver.
            export_dialog: std::rc::Rc::new(std::sync::mpsc::channel().1),
            autostart_state: Arc::new(AtomicUsize::new(usize::MAX)),
            last_save_secs: 0.0,
            status_msg: Lang::default().msg_ready().to_string(),
            status_note: None,
            status_auto: None,
            chart_p95: Vec::new(),
            stats_scratch: Vec::new(),
            cut_scratch: Vec::new(),
            all_rtt_scratch: Vec::new(),
            row_rects: Vec::new(),
            row_heights: Vec::new(),
            row_ids: Vec::new(),
            drag_layouts: Vec::new(),
            drag_row: None,
            window_geom: None,
            // Placeholder until the top bar has been drawn once, which happens before any key can
            // reach the fields.
            new_target_ids: [
                egui::Id::unique("unset_host"),
                egui::Id::unique("unset_label"),
            ],
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
                tgt.color_idx = t.color_idx;
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
        // button shows "off"; afterwards `poll_autostart` keeps it honest.
        app.check_autostart_once();
        app
    }

    pub fn sync_caps(&mut self) {
        let cap = effective_max_samples(&self.options);
        for t in &mut self.targets {
            t.series.set_cap(cap);
        }
    }

    /// Hand settings the engine owns over to it. Called whenever the settings panel reports a
    /// change, so the "concurrent probes" takes effect immediately instead of on the next
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
        // without this the status bar would keep showing the "writing CSV" message forever.
        self.take_export_result();
        // And the the "save as…" dialog once it comes back.
        self.take_export_dialog();
        // Same for the autostart lookup that started at launch.
        self.poll_autostart();

        // The status bar is refreshed before anything below can stop this tick: every line written above, and every sentence an
        // action asked for, belongs to the window whether or not measuring is running. A window that is paused still has to show
        // what a click reported — otherwise the bar keeps sitting on its previous text and the list looks like it never answered.
        self.refresh_status();

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
                self.status_auto = Some(i18n::fill(
                    self.lang().msg_protocol_unwritable_fmt(),
                    &[("err", err)],
                ));
            }
        }
        // One line, two sources: what the user did last wins over what reported by itself. The automatic line is not thrown away
        // when it loses — a protocol file that cannot be written retries every 30 s and would otherwise erase an action for half
        // a minute (see `status_note`). It reappears as soon as nothing newer is in the way, which is right: the warning is still
        // true, only the click was more recent.
        self.refresh_status();

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
        // Read the language out first: inside this block `self.targets` is borrowed mutably.
        let lang = self.lang();
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
                // A recovery is always logged. It used to hang on `options.sound_alerts`, although
                // that option switches nothing but a sound that does not exist — so the line was
                // missing from the log without a visible reason.
                if t.streak_bad > 0 {
                    act = Act::Recovery(i18n::fill(
                        lang.log_recovery_fmt(),
                        &[("label", t.label.clone()), ("ms", fmt_ms(r.rtt_ms))],
                    ));
                }
                t.streak_bad = 0;
                t.alert_raised = false;
            } else {
                t.streak_bad += 1;
                if t.streak_bad >= 2 && !t.alert_raised {
                    t.alert_raised = true;
                    let why = match fail {
                        Some(Fail::Resolve) => lang.log_why_resolve(),
                        Some(Fail::Error) => lang.log_why_error(),
                        _ => lang.log_why_timeout(),
                    };
                    act = Act::Outage(i18n::fill(
                        lang.log_outage_fmt(),
                        &[("label", t.label.clone()), ("why", why.to_string())],
                    ));
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

    /// The interface language in use. Every text the window shows is looked up here, so switching
    /// needs no rebuild — the next frame simply asks again.
    pub fn lang(&self) -> Lang {
        self.options.language
    }

    /// The view's name in the current language. [`View::label`] stays German because screenshot mode
    /// and the log use it as a stable identifier.
    pub fn view_label(&self, v: View) -> &'static str {
        self.lang().view(v)
    }

    /// Seconds since the app started (the chart's x axis).
    pub fn now_secs(&self) -> f64 {
        self.engine.elapsed_secs()
    }

    /// The clock time for a moment given in seconds since start-up — the same computation as in the
    /// CSV export and the protocol file, so a protocol line, an event line and the file all talk
    /// about the same moment. `HH:MM:SS`, because a day would not fit in the list otherwise.
    pub fn wall_clock(&self, secs: f64) -> String {
        let at = self.started_wall + chrono::Duration::milliseconds((secs * 1000.0) as i64);
        at.format("%H:%M:%S").to_string()
    }

    /// Jumps the chart window onto the next sample above its target's outlier cut,
    /// starting after the visible window: where the next outlier occurred.
    /// Moves the chart window onto a moment so that it sits in the middle. Used by double-clicking
    /// an outage line: the message and the history it belongs to, in one place.
    pub fn jump_to_secs(&mut self, secs: f64) {
        let horizon = self.options.horizon_secs as f64;
        let now = self.engine.elapsed_secs();
        if !(secs.is_finite() && secs >= 0.0) {
            return;
        }
        self.pan_offset_secs = (now - secs - horizon * 0.5).clamp(0.0, now.max(0.0));
        self.follow_live = self.pan_offset_secs < 0.5;
        self.view = View::Chart;
    }

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
            None => self.log_info(self.lang().msg_no_outliers()),
        }
    }

    /// Puts the two waiting lines into the status bar: what the user did last if there is such a thing, otherwise whatever
    /// reported by itself. Kept apart from [`Self::tick`] because a paused app ticks too — its actions have to reach the bar just
    /// the same (see the call before `tick`'s early return).
    fn refresh_status(&mut self) {
        self.status_msg = self
            .status_note
            .clone()
            .or_else(|| self.status_auto.clone())
            .unwrap_or_default();
    }

    /// Ask for a sentence in the status bar and keep it there: it is written at the end of the next [`Self::tick`] and stays until
    /// something newer replaces it. Use this for anything the user did — a line that reports by itself (see `status_auto`) only
    /// gets the bar while nothing the user caused is waiting for it.
    pub fn say(&mut self, msg: impl Into<String>) {
        self.status_note = Some(msg.into());
    }

    /// The line that reported by itself most recently (see `status_auto`), without the user's
    /// own sentence in front of it. Tests use this to ask whether a failure reached the bar at all:
    /// [`Self::status_msg`] is only what the bar shows right now, and that also depends on whether
    /// the user said something more recently.
    pub fn status_auto_note(&self) -> Option<&str> {
        self.status_auto.as_deref()
    }

    /// The same rule for a sentence taken from the language table — the usual case for a message that
    /// comes from the UI text (`self.lang().msg_…()`).
    pub fn say_lang(&mut self, msg: &'static str) {
        self.say(msg);
    }

    pub fn start(&mut self) {
        self.start_quiet(false);
    }

    /// `start`, with the option to keep the status bar out of it. A programmatic start (a test, a headless run) is not
    /// something the user did and has nothing to report about it; the button in the top bar keeps saying what it always said.
    pub fn start_quiet(&mut self, quiet: bool) {
        self.running = true;
        // Stagger the first probes so a big list doesn't fire all at once.
        let now = self.engine.elapsed_secs();
        for (i, t) in self.targets.iter().enumerate() {
            if t.enabled {
                self.next_probe_at
                    .insert(t.id, now + (i as f64 * 0.12).min(1.5));
            }
        }
        if !quiet {
            self.say(self.lang().msg_measuring().to_string());
        }
    }

    pub fn stop(&mut self) {
        self.stop_quiet(false);
    }

    /// `stop`, with the option to keep the status bar out of it — same reason as [`Self::start_quiet`].
    pub fn stop_quiet(&mut self, quiet: bool) {
        self.running = false;
        self.engine.stop_all();
        if !quiet {
            self.say(self.lang().msg_stopped_edit().to_string());
        }
    }

    pub fn add_target(&mut self) {
        // The editor keeps its target by index, so the list must not grow underneath it.
        if self.edit_idx.is_some() {
            self.say(self.lang().msg_finish_edit_first().to_string());
            return;
        }
        let host = self.new_host.trim().to_string();
        if host.is_empty() {
            self.say(self.lang().msg_need_address().to_string());
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
        // A fresh target has the constructor's floor; the window it has to hold is only known
        // here. Without this a new target would keep 24 h of data in a 1 024-sample ring.
        self.targets
            .last_mut()
            .unwrap()
            .series
            .set_cap(effective_max_samples(&self.options));
        self.new_host.clear();
        self.new_label.clear();
        self.selected = self.targets.len() - 1;
        self.say(i18n::fill(
            self.lang().msg_added_full_fmt(),
            &[("host", host)],
        ));
    }

    /// Replace the whole list with a known set of named addresses. Tests and headless runs use this to be independent of what the
    /// machine's network happens to offer: an app built from nothing starts with the built-in three (see `default_targets`), and
    /// a test that asks for A/B/C has to get exactly those three rows, not five. The selection starts at the top.
    ///
    /// The list is written directly rather than grown one `add_target` at a time: that path refuses to touch a list while an
    /// editor is open, and this function's whole point is "this is the list now" — a half-applied list would be worse than a
    /// surprise. Ids count from 0 like the loader's do, so nothing downstream can tell the two apart.
    pub fn set_test_targets(&mut self, targets: &[(&str, &str)]) {
        for t in &self.targets {
            self.engine.stop_target(t.id);
        }
        self.next_probe_at.clear();
        self.targets.clear();
        for (i, (label, host)) in targets.iter().enumerate() {
            let mut t = Target::new(i, (*label).to_string(), (*host).to_string(), classify(host));
            // Same rule as `add_target`: a new target only knows the constructor's floor, and the window it has to hold is a
            // setting this app does have.
            t.series.set_cap(effective_max_samples(&self.options));
            self.targets.push(t);
        }
        self.selected = 0;
    }

    /// Add a target with an explicit label (used by the quick-pick buttons).
    pub fn add_named_target(&mut self, host: &str, label: &str) {
        self.new_host = host.to_string();
        self.new_label = label.to_string();
        self.add_target();
    }

    /// The click areas of the rows, as the panel drew them on the last pass: one entry per row *of [`App::targets`]*, in the
    /// order of that list after the live move of that pass, holding `None` where an open editor drew its form instead of a row.
    /// The click test uses them to aim a synthetic pointer at a row; `ui::left_body()` fills them. See the field for why the holes
    /// are kept rather than dropped, and [`App::row_of`] for how to ask this by entry rather than by place.
    pub fn row_rects(&self) -> &[Option<egui::Rect>] {
        &self.row_rects
    }

    /// The same click areas as a plain list of rects, with the holes left out: what a pointer aimed at "the third row the panel
    /// actually drew" wants. A point cannot land on a row that was not drawn, so skipping those is exactly right — but note that
    /// this is *not* the numbering [`App::row_rects`] uses, and a caller that needs to say which entry it means has to ask
    /// [`App::row_of`] with the id rather than counting here.
    pub fn target_row_rects(&self) -> Vec<egui::Rect> {
        self.row_rects.iter().flatten().copied().collect()
    }

    /// The entries whose rows the panel laid out its last pass, one for each row of [`App::row_rects`] — so the two describe one
    /// and the same layout, and a row that drew no strip still holds its place. It is the order of [`App::targets`], because a
    /// pass publishes its layout only after its live move (see `ui::left_body`), which is what lets anything count rows of the list
    /// and read the matching rect.
    pub fn row_ids(&self) -> &[usize] {
        &self.row_ids
    }

    /// Which row of the last layout the entry with this id stood in, or `None` when that pass drew no row for it. This is how
    /// anything that knows an entry — rather than a place on screen — asks a question of [`App::row_rects`].
    pub fn row_of(&self, id: usize) -> Option<usize> {
        self.row_ids.iter().position(|drawn| *drawn == id)
    }

    /// Where the row of the target with this id stood on the last pass, or `None` when the panel did not draw it.
    /// A test uses this to aim a pointer at a real row by name rather than by counting rows.
    pub fn row_rect_of(&self, id: usize) -> Option<egui::Rect> {
        self.row_of(id)
            .and_then(|row| self.row_rects.get(row).copied().flatten())
    }

    /// Which entry the inline editor is showing, by id — or `None` when no form is open. Asking this instead of
    /// `edit_idx` is what lets a caller care about *the entry being edited* rather than about a row that may already
    /// have moved: a drag shuffles the list while its rows are drawn, and an index then names whoever happens to stand
    /// there. The index is kept in step with the entry (see [`App::move_target`]), so both answers agree — but only this
    /// one says so by construction.
    pub fn edit_id(&self) -> Option<usize> {
        self.edit_idx
            .and_then(|idx| self.targets.get(idx))
            .map(|t| t.id)
    }

    /// Which row of the list a target with this id currently sits in, or `None` when no such target
    /// exists. A drag keeps its entry by id precisely because its row changes while it is carried.
    pub fn target_row_of(&self, id: usize) -> Option<usize> {
        self.targets.iter().position(|t| t.id == id)
    }

    /// Puts a measurement into one target's history by id, as the real engine does through
    /// `App::apply_result`. Tests use it to give a list something to draw: rows are laid out at the
    /// same height either way, but a row that has never answered shows "waiting" instead of a latency,
    /// and text that long can be what makes egui need a scrollbar.
    pub fn record_sample_for(&mut self, id: usize, rtt_ms: f32) {
        if let Some(t) = self.targets.iter_mut().find(|t| t.id == id) {
            // The engine clock only counts while the app runs, so a stopped test app would stamp every
            // sample with the same time and the rows would read as one instant. A real measurement gets
            // its time from the engine; here the app clock is simply where that time comes from.
            let now = self.engine.elapsed_secs().max(0.0);
            t.series.record(now, 0, rtt_ms, None);
            t.last_probe_secs = now;
            t.last_rtt = rtt_ms;
        }
    }

    /// Open the inline editor for a target. Called when a row in the left panel is clicked, so a
    /// single click is enough to reach the settings. A row that has just been dragged somewhere else
    /// must not open its editor on the way — see [`App::drop_moved_target`].
    ///
    /// An editor that was already open is saved first, so switching rows never loses an edit. The
    /// commit closes the editor, which is why it has to happen before the new row is loaded here.
    pub fn begin_edit(&mut self, idx: usize) {
        if let Some(t) = self.targets.get(idx) {
            let id = t.id;
            self.begin_edit_by_id(id);
        }
    }

    /// Opens the inline editor for the entry with this id, wherever it happens to sit.
    ///
    /// The row loop collects "open this one" as an id and applies it once the list is no longer borrowed — by then a
    /// live move may have shuffled everything, so the index that row was drawn at means nothing. This is the lookup that
    /// turns the id back into today's row.
    pub fn begin_edit_by_id(&mut self, id: usize) {
        let Some(idx) = self.targets.iter().position(|t| t.id == id) else {
            return;
        };
        // Any form that was open is committed first, so switching rows never loses an edit.
        if self.edit_idx.is_some() {
            self.close_edit();
        }
        let t = &self.targets[idx];
        self.selected = idx;
        self.edit_idx = Some(idx);
        self.edit_host = t.host.clone();
        self.edit_label = t.label.clone();
        self.edit_kind = t.kind;
        self.edit_enabled = t.enabled;
        self.edit_color_idx = t.color_idx;
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
        // Read before `self.targets` is borrowed mutably below.
        let lang = self.lang();
        let Some(idx) = self.edit_idx else { return };
        let Some(t) = self.targets.get_mut(idx) else {
            self.edit_idx = None;
            return;
        };

        let new_host = self.edit_host.trim().to_string();
        if new_host.is_empty() {
            self.say(self.lang().msg_empty_host().to_string());
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

        // Same idea for the colour: automatic by default, but what the picker chooses is stored
        // as a fixed palette number so it no longer moves when targets are added or removed.
        if self.edit_color_idx != t.color_idx {
            t.color_idx = self.edit_color_idx;
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

        // Built while the target is still borrowed (its label and enabled switch are part of the
        // sentence), then handed over once that borrow is over — `say` needs `&mut self`.
        let note = if host_changed && label_changed {
            i18n::fill(
                lang.msg_host_to_label_fmt(),
                &[("old", old_host), ("new", new_host)],
            )
        } else if host_changed {
            i18n::fill(
                lang.msg_host_to_fmt(),
                &[("old", old_host), ("new", new_host)],
            )
        } else if label_changed {
            i18n::fill(lang.msg_renamed_full_fmt(), &[("label", t.label.clone())])
        } else if t.enabled != was_enabled {
            let key = if t.enabled {
                lang.msg_resumed_fmt()
            } else {
                lang.msg_paused_fmt()
            };
            i18n::fill(key, &[("label", t.label.clone())])
        } else {
            lang.msg_no_change().to_string()
        };
        self.say(note);

        self.edit_idx = None;
        self.save_now();
    }

    /// Discard the inline editor without saving.
    pub fn cancel_edit(&mut self) {
        self.edit_idx = None;
    }

    pub fn remove_selected(&mut self) {
        // By id, not by index: the row that is selected now is not necessarily the one this call was asked for (see
        // `remove_target_by_id`).
        if let Some(t) = self.targets.get(self.selected) {
            let id = t.id;
            self.remove_target_by_id(id);
        }
    }

    /// Takes the entry with this id out of the list, stopping its measurements.
    ///
    /// Asked by id rather than through [`App::remove_selected`] because a caller that decided to delete while the list
    /// was being drawn cannot know where the entry stands by the time it acts: only an id survives whatever the list did
    /// in between. The selection moves to the neighbour the removed row leaves behind, which is what "delete this one and
    /// carry on next door" means.
    pub fn remove_target_by_id(&mut self, id: usize) {
        let Some(i) = self.targets.iter().position(|t| t.id == id) else {
            return;
        };
        // Removing a row would shift the index the editor points at.
        if self.edit_idx.is_some() {
            self.say(self.lang().msg_finish_edit_first().to_string());
            return;
        }
        self.engine.stop_target(id);
        self.next_probe_at.remove(&id);
        let label = self.targets[i].label.clone();
        self.targets.remove(i);
        // The row that was removed is gone, so the selection follows it down to whoever stands there now.
        self.selected = i.min(self.targets.len().saturating_sub(1));
        self.say(i18n::fill(
            self.lang().msg_removed_full_fmt(),
            &[("label", label)],
        ));
    }

    /// Moves the selection onto the entry with this id, leaving it where it was when there is no such entry.
    pub fn selected_by_id(&mut self, id: usize) {
        if let Some(pos) = self.targets.iter().position(|t| t.id == id) {
            self.selected = pos;
        }
    }

    pub fn reset_stats(&mut self) {
        let cap = effective_max_samples(&self.options);
        for t in &mut self.targets {
            t.series = Series::new(cap);
            t.last_rtt = f32::NAN;
            t.last_probe_secs = f64::NAN;
            t.streak_bad = 0;
        }
        self.log.clear();
        self.log_total = 0;
        self.alert_count = 0;
        self.say(self.lang().msg_measurements_cleared().to_string());
    }

    /// Start an export to `path`. Returns whether a file was actually handed to the worker: with no
    /// samples, or a second click while an export is running, the caller gets `false` and the reason
    /// goes into the status bar instead.
    pub fn export_csv(&mut self, path: PathBuf) -> bool {
        if self.pending_export.load(Ordering::Acquire) {
            self.say(self.lang().msg_export_running().to_string());
            return false;
        }
        let rows = collect_export_rows(&self.targets);
        if rows.is_empty() {
            self.say(self.lang().msg_no_samples().to_string());
            return false;
        }
        // The name belongs in the status bar, and stays there until the writer thread's message
        // replaces it: `write_csv` mentions it too, because it is also called with paths that should
        // not be shown. An export over a slow network drive is indistinguishable from a stuck window
        // after a second, and "which file?" is the first question then.
        self.write_csv(path, rows);
        true
    }

    /// Writes a CSV file into the state folder and opens that folder in Explorer. The button for it
    /// sits in the status bar: someone who wants to export has the numbers in front of them already
    /// and should not have to look for it in the settings first.
    pub fn quick_export(&mut self) {
        let path = crate::ui::default_export_path();
        // Whether the worker was handed a file at all decides what follows, so `export_csv` reports
        // that instead of having its status text read back — the wording changes with the language.
        if !self.export_csv(path) {
            return;
        }
        self.open_state_folder();
    }

    /// The actual write, shared by the automatic path and the the "save as…" dialog. It runs
    /// on a worker thread so a slow disk or a network share never stops the window from drawing.
    fn write_csv(&mut self, path: PathBuf, rows: Vec<exporter::Row>) {
        self.pending_export.store(true, Ordering::Release);
        let flag = Arc::clone(&self.pending_export);
        let slot = Arc::clone(&self.export_result);
        // Without the target path, "Writing CSV file…" cannot be told apart from a stuck window
        // after one second — and the file one is waiting for would still be an unnamed one. The
        // writer thread replaces the line with its result.
        let lang = self.lang();
        self.say(i18n::fill(
            lang.msg_writing_csv_path_fmt(),
            &[("path", path.display().to_string())],
        ));
        std::thread::spawn(move || {
            let msg = match exporter::write_csv(&path, &rows) {
                Ok(n) => i18n::fill(lang.msg_csv_written_fmt(), &[("n", n.to_string())]),
                Err(e) => e,
            };
            slot.put(msg);
            flag.store(false, Ordering::Release);
        });
    }

    /// Open the system's the "save as…" dialog and export to whatever it returns.
    ///
    /// The dialog runs on its own thread: on Windows it pumps messages of its own, which would
    /// fight egui's paint pass for the same raw window handle if this were called inline. So the
    /// caller only has to be somewhere that is allowed to spawn a thread, and the result arrives
    /// through [`App::take_export_dialog`]. Cancelling leaves `status_msg` alone.
    pub fn open_export_dialog(&mut self) {
        if self.pending_dialog.load(Ordering::Acquire) {
            return;
        }
        let rows = collect_export_rows(&self.targets);
        if rows.is_empty() {
            self.say(self.lang().msg_no_samples().to_string());
            return;
        }
        self.pending_dialog.store(true, Ordering::Release);
        let flag = Arc::clone(&self.pending_dialog);
        let (tx, rx) = std::sync::mpsc::channel();
        let start = crate::ui::default_export_name();
        // Read out here: the thread that runs the dialog may not capture `self`.
        let title = self.lang().msg_dialog_title().to_string();
        let filter = self.lang().msg_csv_filter().to_string();
        std::thread::spawn(move || {
            // `rfd` needs a parent window on Windows to behave like a modal; without one the
            // dialog still works, it just is not tied to the app's z-order. Running it here, off
            // the paint pass, is what keeps its message pump from fighting egui for the window.
            let chosen = rfd::FileDialog::new()
                .set_title(&title)
                .set_file_name(&start)
                .add_filter(&filter, &["csv"])
                .save_file();
            // The receiver may already be gone if the app closed while the dialog was open.
            let _ = tx.send(chosen);
            flag.store(false, Ordering::Release);
        });
        // The receiver is swapped in here rather than passed to the thread: `take_export_dialog`
        // reads it on the next frame, and a channel that was never handed out simply stays empty.
        self.export_dialog = std::rc::Rc::new(rx);
        let _ = tx;
        self.say(self.lang().msg_dialog_opened().to_string());
    }

    /// Take the dialog's answer, if there is one. A chosen path starts the export right away; a
    /// cancelled dialog reports nothing so the previous status message stays visible.
    pub fn take_export_dialog(&mut self) {
        let Some(path) = self.export_dialog.try_recv().ok() else {
            return;
        };
        match path {
            Some(path) => {
                let rows = collect_export_rows(&self.targets);
                if !rows.is_empty() {
                    self.write_csv(path, rows);
                }
            }
            None => self.say(self.lang().msg_export_cancelled().to_string()),
        }
    }

    pub fn take_export_result(&mut self) {
        if let Some(msg) = self.export_result.take() {
            // This is the answer to the click that started the export, so it goes in as an action: it outranks an automatic line
            // and stays until something newer replaces it, instead of being overwritten by the next warning.
            self.status_note = Some(msg);
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
            self.status_auto = Some(if found {
                self.lang().msg_autostart_on_registry().to_string()
            } else {
                self.lang().msg_autostart_off_registry().to_string()
            });
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

    /// Set the chart window from a control that lives outside the options form (the wheel zoom in
    /// the plot). The cap is the same one the settings clamp to, so a zoomed horizon cannot ask for
    /// more samples than [`effective_max_samples`] allows.
    pub fn horizon_secs_set(&mut self, secs: f32) {
        let secs = secs.clamp(5.0, 86_400.0);
        if (self.options.horizon_secs - secs).abs() > f32::EPSILON {
            self.options.horizon_secs = secs;
            self.save_now();
        }
    }

    pub fn set_autostart(&mut self, on: bool) {
        // The settings screen hides this switch where the OS has no mechanism we can write, but a
        // stored file could still ask for it. Saying so in the log beats starting a thread that can
        // only answer "no" — and keeps `options.autostart` from claiming something the system refused.
        if !crate::platform::autostart::is_supported() {
            if on {
                self.log_info("Autostart wird auf diesem System nicht unterstützt.");
            }
            self.options.autostart = false;
            return;
        }
        self.options.autostart = on;
        let slot = Arc::clone(&self.autostart_state);
        std::thread::spawn(move || {
            let ok = set_autostart(on);
            slot.store(if ok { 1 } else { 0 }, Ordering::Release);
        });
    }

    pub fn save_now(&mut self) {
        let dir = store::state_dir();
        // `window_geom` is the last frame's answer; on the very first save of a run it is still
        // None, which simply leaves the file's window field empty.
        let saved = store::save_window_to(
            &dir,
            &self.options,
            &self.saved_targets(),
            self.window_geom.as_ref(),
        );
        // A save that did not go through says so, and does not count as a save: the bar's "saved
        // 3 s ago" would otherwise promise a file that was never written. What is on disk is still
        // the last complete one (see `write_atomically`), so this is a warning to act on rather
        // than measurements already lost — which is why it says what the operating system said.
        match saved {
            Ok(()) => self.last_save_secs = self.engine.elapsed_secs(),
            Err(e) => {
                self.status_auto = Some(i18n::fill(
                    self.lang().msg_save_failed_fmt(),
                    &[("err", e.to_string())],
                ))
            }
        }
    }

    /// How long ago the state file was last written, in seconds. Negative or very large means:
    /// nothing has been saved during this run yet (the autosave runs every 8 s).
    pub fn save_age_secs(&self) -> f64 {
        if self.last_save_secs <= 0.0 {
            return -1.0;
        }
        (self.engine.elapsed_secs() - self.last_save_secs).max(0.0)
    }

    /// The folder that holds the state file, the protocol file and the default export.
    pub fn state_folder(&self) -> PathBuf {
        store::state_dir()
    }

    /// Open the state folder in the system's file manager. A separate process, because the helper
    /// would otherwise block until its window closes; failures stay silent and the status line only
    /// says what was attempted. Which helper that is (`explorer.exe`, `open`, `xdg-open`) is decided
    /// by [`crate::platform::folder`].
    pub fn open_state_folder(&mut self) {
        let dir = store::state_dir();
        self.say(i18n::fill(
            self.lang().msg_folder_opened_fmt(),
            &[("dir", dir.display().to_string())],
        ));
        std::thread::spawn(move || {
            let _ = crate::platform::folder::open(&dir);
        });
    }

    /// The targets in their file shape — everything `store` needs and nothing else.
    pub fn saved_targets(&self) -> Vec<store::SavedTarget> {
        self.targets
            .iter()
            .map(|t| store::SavedTarget {
                id: t.id,
                label: t.label.clone(),
                host: t.host.clone(),
                kind: t.kind,
                enabled: t.enabled,
                color_idx: t.color_idx,
                samples: t
                    .series
                    .samples()
                    .iter()
                    .map(|s| (s.at_ms, s.rtt_ms))
                    .collect(),
            })
            .collect()
    }

    /// `save_now` into a directory given by the caller, so a headless pass keeps its files to
    /// itself even while another thread points the store somewhere else. Returns what landed.
    /// A headless pass has no window, so it never writes a geometry.
    pub fn save_now_to(&self, dir: &std::path::Path) -> std::io::Result<Vec<store::SavedTarget>> {
        let saved = self.saved_targets();
        store::save_to(dir, &self.options, &saved)?;
        Ok(saved)
    }

    pub fn stats_of(&self, id: usize) -> Option<Stats> {
        let t = self.targets.iter().find(|t| t.id == id)?;
        // Same window as the panels. This is a one-off call from outside the frame loop, so
        // a fresh small vector here is fine; the per-frame views reuse `App::stats_scratch`.
        let mut scratch = Vec::new();
        Some(self.stats_for(t, &mut scratch))
    }

    /// The numbers for one target over the current time window — one computation used everywhere:
    /// target list, status list, statistics table, totals row and sorting. Before, the same
    /// expression stood five times in the UI, and each copy was a chance to mean another window
    /// somewhere (the whole stored history instead of the visible one, for instance).
    ///
    /// The window runs from the newest stored measurement minus `horizon_secs` up to the end; with
    /// no measurements it starts at zero. `scratch` is the buffer `stats_window` needs for the
    /// percentiles — take it from the `App` once per frame and put it back when done.
    pub fn stats_for(&self, t: &Target, scratch: &mut Vec<f32>) -> Stats {
        let horizon = self.options.horizon_secs as f64;
        let from = t.series.last().map_or(0.0, |s| s.t_secs - horizon);
        t.series.stats_window(from, f64::MAX, scratch)
    }

    /// The numbers of every shown target together. Min, max, loss and outliers are already right
    /// when the per-target values are combined, but median and p95 are not: the average of the
    /// medians of two very different targets is nobody's median. So all replies are collected into
    /// one buffer here and the percentiles are drawn from it — same procedure as per target, just
    /// over the whole pile.
    ///
    /// A target without a reply contributes nothing to `min` (otherwise a single outage would pull
    /// the minimum of every target down to 0), and with no replies at all `min` is not 0 but
    /// infinity — the interface turns that into "—".
    ///
    /// `scratch` is the buffer for the per-target computations, `all` the one for the collected
    /// list; both live on the `App` so nothing new has to be allocated per frame.
    pub fn stats_total(&self, scratch: &mut Vec<f32>, all: &mut Vec<f32>) -> Stats {
        let horizon = self.options.horizon_secs as f64;
        all.clear();
        let mut total = 0usize;
        let mut losses = 0usize;
        let mut dns_fails = 0usize;
        let mut min = f32::INFINITY;
        let mut max = 0f32;
        let mut outliers = 0usize;
        for t in &self.targets {
            // The same targets as the table above, so "Total" adds up exactly the rows one can see.
            if !self.shown(t) {
                continue;
            }
            let from = t.series.last().map_or(0.0, |s| s.t_secs - horizon);
            let st = t.series.stats_window(from, f64::MAX, scratch);
            total += st.total;
            losses += st.losses;
            dns_fails += st.dns_fails;
            outliers += st.outliers;
            if st.replies > 0 {
                min = min.min(st.min);
                max = max.max(st.max);
            }
            // Only the replies, no losses: percentiles of a latency only make sense over
            // measurements, and a loss has none.
            all.extend(
                t.series
                    .samples()
                    .iter()
                    .filter(|s| s.t_secs >= from && history::is_ok(s.rtt_ms))
                    .map(|s| s.rtt_ms),
            );
        }
        // Percentiles from the collected list, exactly as `stats_window` computes them per target.
        let (median, p95) = if all.is_empty() {
            (0.0, 0.0)
        } else {
            all.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            (history::pick(all, 0.5), history::pick(all, 0.95))
        };
        let replies = all.len();
        let sum: f64 = all.iter().fold(0.0, |acc, v| acc + f64::from(*v));
        Stats {
            total,
            replies,
            losses,
            min,
            max,
            sum,
            avg: if replies > 0 {
                sum / replies as f64
            } else {
                0.0
            },
            median,
            p95,
            loss_pct: if total > 0 {
                100.0 * losses as f64 / total as f64
            } else {
                0.0
            },
            outliers,
            dns_fails,
        }
    }

    /// Ids of the targets that pass every filter: the text filter and the category switches in the
    /// top bar. Chart, events list, statistics and the legend all draw exactly these, so "was ist
    /// zu sehen" is decided in one place only. Hidden single targets stay in this list — the legend
    /// has to keep drawing them so they can be switched back on.
    pub fn visible_targets(&self) -> Vec<usize> {
        self.targets
            .iter()
            .filter(|t| self.passes_filter(t))
            .map(|t| t.id)
            .collect()
    }

    /// Does this target pass the text filter and the category switches? Says nothing about
    /// [`Target::hidden`]: that is per-target presentation, handled by [`Self::shown`].
    pub fn passes_filter(&self, t: &Target) -> bool {
        if self.options.kinds_off.contains(&kind_index(t.kind)) {
            return false;
        }
        let f = self.filter.trim().to_lowercase();
        f.is_empty()
            || t.label.to_lowercase().contains(&f)
            || t.host.to_lowercase().contains(&f)
            || t.kind.label().to_lowercase().contains(&f)
    }

    /// Is this target drawn right now — in the chart, the legend and the lists? It has to pass the
    /// filters and must not have been hidden by a click on its name in the legend.
    pub fn shown(&self, t: &Target) -> bool {
        !t.hidden && self.passes_filter(t)
    }

    /// Ids of the targets [`Self::shown`] accepts — what the statistics table and the summary rows
    /// count.
    pub fn shown_target_ids(&self) -> Vec<usize> {
        self.targets
            .iter()
            .filter(|t| self.shown(t))
            .map(|t| t.id)
            .collect()
    }

    /// Toggles the visibility of one target by id (a click on its name in the legend). Returns the
    /// new state so the caller can report it.
    pub fn toggle_hidden_by_id(&mut self, id: usize) -> bool {
        if let Some(t) = self.targets.iter_mut().find(|t| t.id == id) {
            t.hidden = !t.hidden;
            t.hidden
        } else {
            false
        }
    }

    /// Orders the targets by a column of the statistics table. Labels stay as they are: only the
    /// order of the list changes, and because every target keeps its id and with it its manually
    /// chosen colour, nothing wanders between curves. `descending` flips the result — the first
    /// click on a column sorts by value descending ("the worst first"), the second ascending.
    pub fn sort_targets(&mut self, column: SortColumn, descending: bool) {
        let horizon = self.options.horizon_secs as f64;
        let mut scratch = std::mem::take(&mut self.stats_scratch);
        // The statistics for every id up front, so the comparison itself never has to compute.
        // The value is stored as a finished key pair right away — `sort_by` compares each target
        // more than once, and scanning a list of keys per comparison would walk the whole set of
        // targets (linearly) every time.
        let keys: BTreeMap<usize, (Option<String>, f64)> = self
            .targets
            .iter()
            .map(|t| {
                let from = t.series.last().map_or(0.0, |s| s.t_secs - horizon);
                let st = t.series.stats_window(from, f64::MAX, &mut scratch);
                let k = SortKey::of(column, t, &st);
                (t.id, (k.text, k.value))
            })
            .collect();
        self.stats_scratch = scratch;
        self.targets.sort_by(|a, b| {
            let ka = keys.get(&a.id);
            let kb = keys.get(&b.id);
            match (ka, kb) {
                (Some(x), Some(y)) => {
                    let ord = (x.0.as_deref(), x.1)
                        .partial_cmp(&(y.0.as_deref(), y.1))
                        .unwrap_or(std::cmp::Ordering::Equal);
                    if descending {
                        ord.reverse()
                    } else {
                        ord
                    }
                }
                _ => std::cmp::Ordering::Equal,
            }
        });
        // The selection follows the target it was: otherwise re-sorting would jump the marker to a
        // neighbour that has nothing to do with the click.
        if let Some(pos) = self.targets.iter().position(|t| t.id == self.selected_id()) {
            self.selected = pos;
        }
    }

    /// The id of the currently selected target (if it still exists).
    fn selected_id(&self) -> usize {
        self.targets.get(self.selected).map_or(usize::MAX, |t| t.id)
    }

    /// Moves a row to another place in the list — the live follow of a drag started by holding a row in the left
    /// panel and pulling it up or down, and the final move at its end. `id` is the dragged target and `to` the index
    /// it should end up at, both as they were when the pointer was last read; passing the target's own index is
    /// allowed and simply changes nothing.
    ///
    /// Ids rather than indices because a drag is exactly the situation in which the numbers under a row stop being
    /// valid: an open editor can occupy a row, and while the list is being re-drawn after every move the old index
    /// already points somewhere else. An id always finds its target.
    ///
    /// The selection follows the moved target (see `App::selected_id`).
    ///
    /// Writing the file is deliberately *not* part of this: a drag that follows the pointer calls it on almost every
    /// pass, and one write per shuffle would be a disk access per frame. A finished gesture writes through
    /// [`Self::drop_moved_target`] instead, and so does every other way of reordering.
    pub fn move_target(&mut self, id: usize, to: usize) {
        // The editor keeps its target by index and the list is about to change. While a form is
        // open the order therefore stays as it is — same rule as `add_target` and
        // `remove_selected` follow, and the drag handle simply does not appear on an open row. Asked before the entry is looked up
        // because the answer is the same whether or not such an entry exists: a form that is open means no reordering, and nothing
        // about a particular id can change that.
        if self.edit_idx.is_some() {
            self.say(self.lang().msg_finish_edit_first().to_string());
            return;
        }
        let Some(from) = self.targets.iter().position(|t| t.id == id) else {
            return;
        };
        // `to` counts the list with the carried entry still in it, which is what every caller measures by (see `ui::slot_at`).
        // That has two consequences this function has to honour.
        //
        // A slot one past the entry's own place is "leave it where it is": the pointer was over the row below, and that row has
        // moved up into the hole. Without this rule, a hand that wobbles over the next row's centre mid-drag visibly swaps two
        // entries.
        //
        // And the clamp belongs here, before the entry is taken out — not after. A slot one past the end (a drop below the bottom
        // row, or an index that ran away past it) means "at the very end": index `len` while the entry is in the list, `len - 1`
        // once it is gone. Clamping after the removal makes those two the same number on the last row, so such a drop lands on the
        // entry's own place and silently does nothing although the pointer clearly said otherwise.
        let len = self.targets.len();
        let to = to.min(len);
        if to == from || to == from + 1 {
            // Dropped where it already was: nothing happened, so nothing is said about it either. A drag that shuffles the list
            // passes through this case on its way back, and a status line claiming "nothing moved" would be wrong half the time.
            return;
        }
        let t = self.targets.remove(from);
        // Above, `to` counted the list with the entry still in it. Here the entry is out, so a slot behind the last row has to
        // come down by one — that is all this is, not a second clamp: `from < to <= len` always leaves an index inside the new
        // length, and `insert` at that end means "behind everything", which is what such a drop asked for.
        let to = if to > from { to - 1 } else { to };
        self.targets.insert(to, t);
        // Said out loud because the row that used to be there is not where it was: a list of ten rows is long enough that a moved
        // entry can leave the visible part of the panel.
        let label = self.targets[to].label.clone();
        self.say(i18n::fill(
            self.lang().msg_moved_fmt(),
            &[("label", label), ("n", (to + 1).to_string())],
        ));
        if let Some(pos) = self.targets.iter().position(|t| t.id == id) {
            self.selected = pos;
        }
    }

    /// Ends a row drag by moving the target to slot `to` and letting go of it. This is the one place a
    /// reordering drag writes the state file: while the entry is in flight the list may be shuffled every
    /// pass, and only the final place is what the user wants to meet after a restart.
    ///
    /// The release that ended the drag is still also a click on a row as far as egui is concerned, but
    /// [`crate::ui::left_body`] already knows an entry was in flight in this pass and opens no editor for
    /// it (see `tests/frame_click.rs` for why a row's click is read from the rects of the previous pass).
    pub fn drop_moved_target(&mut self, id: usize, to: usize) {
        // The move happens before the drag ends, so the caller can still describe the list as it was
        // while the entry was carried — the slot `to` counts the carried entry in.
        self.move_target(id, to);
        self.drag_row = None;
        // Written whether or not the last pass moved anything: the order may well have changed on an
        // earlier pass of the same drag, and this is the only moment that knows the gesture is over.
        self.save_now();
    }

    /// The place a row being dragged should move to when the pointer rests on `target_row`, the
    /// index of the row it is over, at height `pointer_y` inside that row's `rect`. A row is
    /// inserted *before* the one pointed at from above and *after* it from below, so hovering the
    /// middle of a row does not make the entry jump: only a clear push past its centre decides the
    /// direction.
    ///
    /// The index in the list *as it is now*, counting the dragged entry in: dropping behind the row
    /// at index 3 asks for index 4. [`App::move_target`] counts the same way — it takes the entry out
    /// of its old slot and inserts it at the requested place — so a row pulled downwards lands where
    /// the pointer was. See `ui::slot_at`, which turns the pointer's position into this number.
    pub fn drop_index_for(target_row: usize, pointer_y: f32, rect: egui::Rect) -> usize {
        if pointer_y < rect.center().y {
            target_row
        } else {
            target_row + 1
        }
    }

    /// How long a click on the "clear measurements" stays armed before the warning lapses. Long enough
    /// to read the changed label and click again, short enough that a forgotten warning never
    /// deletes anything.
    const CLEAR_ARM_SECS: u64 = 5;

    /// Drops an expired arming of the the "clear measurements" button. Called wherever the button is
    /// drawn, so leaving the view or waiting too long disarms it.
    pub fn expire_clear_armed(&mut self) {
        if !self.clear_armed {
            return;
        }
        let expired = self
            .clear_at
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(Self::CLEAR_ARM_SECS));
        if expired {
            self.clear_armed = false;
            self.clear_at = None;
        }
    }

    /// Switches one category on or off, and reports what it is now. Returns the new state so the
    /// caller can paint its chip accordingly.
    pub fn toggle_kind(&mut self, kind: TargetKind) -> bool {
        let i = kind_index(kind);
        match self.options.kinds_off.iter().position(|&n| n == i) {
            Some(pos) => {
                self.options.kinds_off.remove(pos);
                true
            }
            None => {
                self.options.kinds_off.push(i);
                false
            }
        }
    }

    /// Are all categories currently on? Used by the "alle" button in the filter row.
    pub fn all_kinds_on(&self) -> bool {
        self.options.kinds_off.is_empty()
    }

    /// Switches every category back on.
    pub fn show_all_kinds(&mut self) {
        self.options.kinds_off.clear();
    }

    /// Colour a status dot: the last measurement is what matters here.
    pub fn status_color(&self, t: &Target) -> Color32 {
        self.theme_mode
            .status_dot(!t.enabled, self.running, t.last_rtt)
    }

    /// Colour of one target's line, dot and legend entry. Every target gets its own colour so
    /// several traces on the same grid stay separable; see [`App::color_idx_of`] for how that
    /// colour is chosen. The status dot deliberately does *not* use it: there the latency band is
    /// what has to be visible at a glance, and `status_dot` already encodes paused/waiting/loss.
    pub fn trace_color(&self, t: &Target) -> Color32 {
        self.theme_mode.swatch(self.color_idx_of(t))
    }

    /// Palette number this target uses: the stored one if the user picked a colour, otherwise the
    /// first free colour of the palette. The automatic pick counts only the targets that were
    /// given a colour before it (in list order), so two targets never end up with the same line
    /// while there is still an unused colour — and a target whose colour was chosen by hand frees
    /// its slot for the others.
    pub fn color_idx_of(&self, t: &Target) -> usize {
        if t.color_idx != AUTO_COLOR {
            return t.color_idx % theme::PALETTE_LEN;
        }
        self.first_auto_idx(t)
    }

    /// The palette number an automatic target gets: the first slot no other target occupies. Only
    /// targets ahead of this one in the list count as occupants, so every target can be answered
    /// without knowing what comes after it. When the list is longer than the palette, colours are
    /// reused by position instead of leaving a target without a line colour.
    fn first_auto_idx(&self, t: &Target) -> usize {
        let mut used = [false; theme::PALETTE_LEN];
        for other in self.targets.iter().take_while(|o| o.id != t.id) {
            let idx = if other.color_idx != AUTO_COLOR {
                other.color_idx % theme::PALETTE_LEN
            } else {
                match self.first_auto_idx(other) {
                    // An earlier target that already had to reuse the palette must not block this
                    // one; otherwise a long list would run out of colours too early.
                    theme::PALETTE_LEN..=usize::MAX => continue,
                    n => n,
                }
            };
            used[idx] = true;
        }
        match used.iter().position(|free| !*free) {
            Some(n) => n,
            None => self.targets.iter().take_while(|o| o.id != t.id).count() % theme::PALETTE_LEN,
        }
    }

    /// The last measurement of a target as text — paused, waiting, milliseconds, or the reason it is
    /// missing. The same short form everywhere: the target list on the left, the status list, the
    /// legend above the chart and the hover read-out in the chart all read this one function.
    pub fn status_text(&self, t: &Target) -> String {
        let lang = self.lang();
        if !t.enabled {
            return lang.paused().into();
        }
        if t.last_rtt.is_nan() {
            return if self.running {
                lang.waiting().into()
            } else {
                "—".into()
            };
        }
        if is_ok(t.last_rtt) {
            format!("{} ms", fmt_ms(t.last_rtt))
        } else {
            // The reason rather than just "timeout": an unresolvable address and a silent host are
            // two different problems, and this list is where one reads them first. `Fail::short` is
            // the same short form that goes into the protocol file, so it stays German there.
            let why = self
                .last_fail
                .iter()
                .zip(self.targets.iter())
                .find(|(_, x)| x.id == t.id)
                .and_then(|(f, _)| f.as_ref().copied())
                .map(Fail::short)
                .unwrap_or("timeout");
            why.into()
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
        Target::new(2, "DNS (Cloudflare)", "1.1.1.1", TargetKind::Dns),
    ]
}

/// Best-effort default gateway from the system's own routing table; falls back to the common
/// home-router address. The per-system commands and their column layouts live in
/// [`crate::platform::gateway`]. Never blocks longer than a second.
fn gateway_hint() -> String {
    crate::platform::gateway::default_gateway().unwrap_or_else(|| String::from("192.168.1.1"))
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

/// Write or remove the autostart entry that starts the app at sign-in. Which mechanism that is (the
/// Windows `Run` key, nothing on Linux and macOS — see [`crate::platform::autostart`]) stays out of
/// sight here.
fn set_autostart(on: bool) -> bool {
    crate::platform::autostart::set(on)
}

/// Whether the autostart entry that starts the app at sign-in exists. This is the read side of
/// `set_autostart`; without it the button in the top bar can only ever say "off", because the
/// counter it reads is only ever written when the user presses it.
fn autostart_exists() -> bool {
    crate::platform::autostart::exists()
}

#[cfg(test)]
mod tests {
    use super::{about_version, APP_VERSION, GIT_HASH};

    /// The About line names the version and, when the build knows its commit, that commit too —
    /// and never a version with empty parentheses, which is a hole in the text, not an answer.
    #[test]
    fn the_about_line_says_version_and_commit() {
        let line = about_version();
        assert!(
            line.starts_with(APP_VERSION),
            "{line} must name the version"
        );
        if GIT_HASH.is_empty() {
            assert_eq!(line, APP_VERSION, "unknown commit: bare version only");
        } else {
            assert_eq!(
                line,
                format!("{APP_VERSION} ({GIT_HASH})"),
                "known commit: spelled out in parentheses"
            );
        }
    }
}
