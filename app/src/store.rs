//! Saved settings and history, as JSON next to the executable.
//!
//! The file lives next to the binary by default (the app folder stays self-contained).
//! `SYSTEM_PING_STATE_DIR` overrides that location for the window and for a whole test
//! binary. The headless seams (`app::collect_once`, `App::save_now_to`) additionally take the
//! directory as an argument, so tests running side by side never share one file.
//!
//! Two things this module guarantees about that file, because the app reads it at every start and
//! writes it on a timer: a part that cannot be read costs only that part (see `PersistedIn` and
//! `MAX_HISTORY_DEPTH`), and a save that is interrupted costs nothing (see `write_atomically`).

use crate::app::{AppOptions, TargetKind};
use std::path::PathBuf;

const STATE_FILE: &str = "system-ping-state.json";

pub const VERSION: u32 = 2;

/// `VERSION` 1 stored each sample as "seconds since the app started", which cannot survive a
/// restart: the anchor is gone with the process, so an export had to invent wall-clock times
/// running backwards from "now" (see the old `append_saved_rows`). Version 2 stores absolute
/// Unix milliseconds. The v1 shape stays in the file so old data can still be read.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SavedTargetV1 {
    #[serde(default)]
    pub id: usize,
    pub label: String,
    pub host: String,
    pub kind: TargetKind,
    pub enabled: bool,
    /// Seconds since the app started — only meaningful inside the run that wrote them.
    pub samples: Vec<(f64, f32)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SavedTarget {
    /// Stable id so a saved row can be matched back to a live target.
    #[serde(default)]
    pub id: usize,
    pub label: String,
    pub host: String,
    pub kind: TargetKind,
    pub enabled: bool,
    /// Palette number chosen by hand, or [`crate::app::AUTO_COLOR`] for an automatic colour.
    /// Missing in older files -> automatic.
    #[serde(default = "auto_color")]
    pub color_idx: usize,
    /// Absolute wall-clock time of the sample as Unix milliseconds, paired with the rtt. This
    /// is what makes an export after a restart read correctly.
    pub samples: Vec<(i64, f32)>,
}

/// The persisted marker for "colour picked automatically". A function rather than a path so serde
/// can use it as a field default.
fn auto_color() -> usize {
    crate::app::AUTO_COLOR
}

impl Clone for SavedTarget {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            label: self.label.clone(),
            host: self.host.clone(),
            kind: self.kind,
            enabled: self.enabled,
            color_idx: self.color_idx,
            samples: self.samples.clone(),
        }
    }
}

/// Where the window was last time. Deliberately our own small struct instead of
/// `egui_winit::WindowSettings`: that one keeps its fields private, so we can only build it from
/// a live winit window — but this file must also be writable (and readable) from tests and from a
/// headless pass, where no window exists. The numbers are the same ones egui-winit uses: inner
/// position in physical pixels, inner size in logical points.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowGeom {
    /// Top-left of the window content (not the title bar), in physical pixels.
    pub x: i32,
    pub y: i32,
    /// Size of the window content in logical points — what `ViewportBuilder::with_inner_size`
    /// expects.
    pub width: f32,
    pub height: f32,
}

/// Smallest window we accept from a state file. Anything below is either a broken file or a
/// drag onto a monitor that has since been smaller; the app's own minimum is a bit larger, so
/// this only guards against nonsense.
const MIN_GEOM_POINTS: f32 = 64.0;

impl WindowGeom {
    /// Is anything in here usable? A non-finite or absurdly small size would give a window that
    /// cannot be seen, which is worse than falling back to the default geometry.
    pub fn sane(&self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.width >= MIN_GEOM_POINTS
            && self.height >= MIN_GEOM_POINTS
    }
}

#[derive(serde::Serialize)]
pub struct Persisted<'a> {
    pub version: u32,
    pub options: Option<&'a AppOptions>,
    pub history: &'a [SavedTarget],
    /// Missing in files written before the window remembered its place -> `None`, and the next
    /// start uses the built-in default size.
    pub window: Option<&'a WindowGeom>,
}

/// Read shape: every part is kept as raw JSON. Two reasons, and both are about not losing more than a
/// file deserves to lose. `history` is raw because v1 and v2 disagree about what a sample pair means,
/// and that difference cannot be expressed with one typed struct. The others are raw because a typed
/// struct fails as a whole: an `options` block this version cannot read — one field dropped by a hand
/// edit, one type changed by a newer app — would otherwise take the measurement history with it, which
/// is the opposite of what a settings file is for. Each part is read on its own terms (see [`part`]),
/// so only the file as a whole staying JSON is a hard requirement.
#[derive(serde::Deserialize)]
struct PersistedIn {
    #[serde(default)]
    version: Option<serde_json::Value>,
    #[serde(default)]
    options: Option<serde_json::Value>,
    #[serde(default)]
    history: Option<serde_json::Value>,
    #[serde(default)]
    window: Option<serde_json::Value>,
}

/// Largest state file this app reads.
///
/// Not a limit anyone is meant to reach: a real one is a few hundred kilobytes even with years of
/// samples, and this is 32 MiB. It is here because reading a file into a string has no bound of its
/// own otherwise, and this file is read at every start — a wrong state directory pointing at some
/// other program's data, a disk that filled mid-write into something absurd, or a file written on
/// purpose would otherwise be pulled into memory whole before the window has even appeared.
const MAX_STATE_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// The file's text, or `None`: absent, unreadable, or bigger than `max_bytes` (the app passes
/// [`MAX_STATE_FILE_BYTES`]; a test passes something small to exercise the same path).
///
/// The oversize case is the one worth a line on stderr, because it is indistinguishable from "no
/// settings yet" otherwise — and a person who has collected measurements for a year and sees the
/// default list deserves to be told the file was refused rather than pretend it was never there.
fn read_state(path: &std::path::Path, max_bytes: u64) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > max_bytes {
        eprintln!(
            "{} is {len} bytes, more than the {max_bytes} bytes this app reads; starting without saved settings",
            path.display()
        );
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Deepest nesting one history element may have.
///
/// Counted in JSON containers inside the element: a saved target is an object, its `samples` an
/// array, one sample a pair — three. Anything deeper is not a damaged target, it is not a target, and
/// dropping that one element keeps the rest of the list, which is the same rule [`part`] applies to
/// the parts of the file.
///
/// This is the policy half. The stack half is already bounded by the parser itself: serde_json stops
/// nesting at 128 levels and reports an error instead of running out of stack, and that limit is only
/// lifted by a feature this crate does not enable. Because of that bound, walking a value's depth
/// recursively is safe — no value that got this far can be deeper than 128.
const MAX_HISTORY_DEPTH: usize = 6;

/// How many containers deep this value goes, counting itself.
fn depth_of(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Array(xs) => 1 + deepest(xs.iter()),
        serde_json::Value::Object(map) => 1 + deepest(map.values()),
        _ => 1,
    }
}

/// The deepest of a set of values, or 0 for none — so a container reports its own level and not one
/// more than it has to.
fn deepest<'a>(xs: impl Iterator<Item = &'a serde_json::Value>) -> usize {
    xs.map(depth_of).max().unwrap_or(0)
}

/// One part of the file, or `None` when it is absent or cannot be read in this version's terms. Costs
/// that part and nothing else (see [`PersistedIn`]).
fn part<T: serde::de::DeserializeOwned>(v: &Option<serde_json::Value>) -> Option<T> {
    v.as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// The entries of a history that may not be a list at all. Unreadable *elements* are dropped further
/// down, one at a time, by [`from_value`]; absurdly nested ones are dropped here, before anything is
/// built out of them (see [`MAX_HISTORY_DEPTH`]).
fn items(v: &Option<serde_json::Value>) -> Vec<serde_json::Value> {
    v.as_ref()
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter(|row| depth_of(row) <= MAX_HISTORY_DEPTH)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Where `load`/`save` read and write. Checked on every call so a test can point it
/// somewhere else at runtime (see the module docs).
///
/// The variable is process-wide, so a caller that sets it for one pass must not let another
/// thread read it at the same time — `collect_once` takes a directory and keeps the variable
/// set for the whole pass instead of leaving that to the caller.
///
/// Which folder is used when nothing is forced differs by system (next to the binary on Windows,
/// the per-user data folder on Linux and macOS) and is decided by [`crate::platform::statedir`].
pub fn state_dir() -> PathBuf {
    crate::platform::statedir::default_dir()
}

pub fn state_path() -> PathBuf {
    state_path_in(&state_dir())
}

/// The same path for a directory given by the caller. `collect_once` uses this so its own
/// reads and writes cannot be pulled to another folder by a test running at the same time.
pub fn state_path_in(dir: &std::path::Path) -> PathBuf {
    dir.join(STATE_FILE)
}

/// `load` plus the window geometry of the last run. Kept separate from [`load_from`] because most
/// callers only ever want options and history, and a headless pass has no window to place.
pub fn load_with_window(
    dir: &std::path::Path,
) -> (AppOptions, Vec<SavedTarget>, Option<WindowGeom>) {
    let path = state_path_in(dir);
    if let Some(text) = read_state(&path, MAX_STATE_FILE_BYTES) {
        if let Ok(p) = serde_json::from_str::<PersistedIn>(&text) {
            let opts = part(&p.options).unwrap_or_default();
            let rows = items(&p.history);
            // A version this version cannot read as a number is treated like a missing one: the old
            // format, which is the shape every file from before version 2 has.
            let history = match part(&p.version).unwrap_or(1u32) {
                2..=u32::MAX => rows.iter().filter_map(from_value::<SavedTarget>).collect(),
                _ => rows
                    .iter()
                    .filter_map(from_value::<SavedTargetV1>)
                    .map(SavedTarget::from_v1)
                    .collect(),
            };
            // A geometry that fails `sane` is treated as absent, so the caller falls back to the
            // default size instead of trying to honour a broken number. Named here rather than left
            // to inference: `part` is generic, and `sane` alone does not tell it which part to read.
            let window = part(&p.window).filter(|g: &WindowGeom| g.sane());
            return (opts, history, window);
        }
    }
    (AppOptions::default(), Vec::new(), None)
}

fn from_value<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> Option<T> {
    serde_json::from_value(v.clone()).ok()
}

impl SavedTarget {
    /// Reads a v1 row and puts its samples on a real timeline. Their anchor is lost, so they
    /// are placed as one second apart directly in front of the last save — which keeps their
    /// spacing and makes an export read as monotone time instead of a clock running back to
    /// the start of a session that never existed.
    fn from_v1(v: SavedTargetV1) -> Self {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let n = v.samples.len();
        Self {
            id: v.id,
            label: v.label,
            host: v.host,
            kind: v.kind,
            enabled: v.enabled,
            color_idx: crate::app::AUTO_COLOR,
            samples: v
                .samples
                .into_iter()
                .enumerate()
                .map(|(i, (_, rtt))| (now_ms - ((n - i) as i64) * 1000, rtt))
                .collect(),
        }
    }
}

/// `load` for a directory given by the caller. A file that cannot be read or parsed stays a
/// default app rather than taking the program down with it.
pub fn load() -> (AppOptions, Vec<SavedTarget>) {
    let (o, h, _) = load_with_window(&state_dir());
    (o, h)
}

/// Same as [`load`], for a directory given by the caller.
pub fn load_from(dir: &std::path::Path) -> (AppOptions, Vec<SavedTarget>) {
    let (o, h, _) = load_with_window(dir);
    (o, h)
}

pub fn save(options: &AppOptions, history: &[SavedTarget]) -> std::io::Result<()> {
    save_to(&state_dir(), options, history)
}

/// `save` into a directory given by the caller.
pub fn save_to(
    dir: &std::path::Path,
    options: &AppOptions,
    history: &[SavedTarget],
) -> std::io::Result<()> {
    save_window_to(dir, options, history, None)
}

/// The one writer. `window` is the geometry to remember; `None` keeps the file's window field
/// empty, which means "next start uses the default size".
pub fn save_window_to(
    dir: &std::path::Path,
    options: &AppOptions,
    history: &[SavedTarget],
    window: Option<&WindowGeom>,
) -> std::io::Result<()> {
    let path = state_path_in(dir);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let p = Persisted {
        version: VERSION,
        options: Some(options),
        history,
        window,
    };
    let text = serde_json::to_string_pretty(&p).unwrap_or_default();
    write_atomically(&path, text.as_bytes())
}

/// Puts `bytes` at `path` in a way that leaves a readable file behind no matter when the process
/// stops.
///
/// The new contents go to a sibling name, are handed to the disk, and only then are moved onto the
/// real name. Written straight over the old file — which is what `fs::write` does — a power failure
/// or a kill halfway through leaves the file the next start reads truncated or half one JSON object,
/// and the app comes up with default settings and no history. That is the whole reason for the
/// detour: the app saves on a timer and on every change, so there is always a save in progress.
///
/// The temporary name carries the process id, because two instances of the app may point at the same
/// folder and must not write each other's half-built file. If the move fails, the leftover is
/// removed and the error is handed to the caller, which reports it the way it reports any failed
/// save. The real file, untouched, is still the last good one.
fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| STATE_FILE.to_string());
    let tmp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));

    let mut file = std::fs::File::create(&tmp)?;
    let written = std::io::Write::write_all(&mut file, bytes).and_then(|()| file.sync_all());
    // Closed before the move: on Windows a file still open here cannot be renamed over the target.
    drop(file);

    let moved = written.and_then(|()| std::fs::rename(&tmp, path));
    if moved.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A v1 file (samples as "seconds since the app started") has to load without losing rows,
    /// and its stamps have to become real times: a monotone sequence that ends near now. This
    /// is the migration path for every user who updates with a history on disk.
    #[test]
    fn reads_a_version_1_history() {
        let dir = std::env::temp_dir().join(format!("system-ping-v1-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let v1 = r#"{
            "version": 1,
            "options": null,
            "history": [{
                "id": 3,
                "label": "gateway",
                "host": "192.168.1.1",
                "kind": "Gateway",
                "enabled": true,
                "samples": [[0.0, 1.5], [1.0, 2.0], [2.0, -1.0]]
            }]
        }"#;
        std::fs::write(state_path_in(&dir), v1).unwrap();

        let (_, saved) = load_from(&dir);
        assert_eq!(saved.len(), 1);
        let t = &saved[0];
        assert_eq!((t.id, t.host.as_str()), (3, "192.168.1.1"));
        assert_eq!(t.samples.len(), 3);

        let stamps: Vec<i64> = t.samples.iter().map(|s| s.0).collect();
        for w in stamps.windows(2) {
            assert!(w[0] < w[1], "stamps must increase: {stamps:?}");
        }
        // One second apart, and the newest lands in the present, not in 1970.
        assert_eq!(stamps[2] - stamps[1], 1000);
        let age_ms = chrono::Utc::now().timestamp_millis() - stamps[2];
        assert!(
            age_ms.abs() < 60_000,
            "newest stamp is {age_ms} ms from now"
        );

        // A v2 file written from what was read keeps the same absolute stamps.
        let rt = SavedTarget {
            samples: t.samples.clone(),
            ..t.clone()
        };
        save_to(&dir, &AppOptions::default(), &[rt]).unwrap();
        let (_, again) = load_from(&dir);
        assert_eq!(again[0].samples, t.samples);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The window's place has to survive a save/load pair unchanged, and a file that was written
    /// before the app remembered it must still load — with "no geometry" rather than a broken one.
    #[test]
    fn remembers_where_the_window_was() {
        let dir = std::env::temp_dir().join(format!("system-ping-geom-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let opts = AppOptions::default();
        let geom = WindowGeom {
            x: 120,
            y: -8,
            width: 1280.0,
            height: 800.5,
        };

        save_window_to(&dir, &opts, &[], Some(&geom)).unwrap();
        let (_, _, back) = load_with_window(&dir);
        assert_eq!(back, Some(geom));

        // A plain `save` (the headless path) leaves the field out again.
        save_to(&dir, &opts, &[]).unwrap();
        assert_eq!(load_with_window(&dir).2, None);

        // An old file without the field at all reads as "no geometry", not as an error.
        std::fs::write(
            state_path_in(&dir),
            r#"{"version":2,"options":null,"history":[]}"#,
        )
        .unwrap();
        assert_eq!(load_with_window(&dir).2, None);

        // A size of zero comes from a broken file or a hidden window; it must not be honoured.
        let junk = WindowGeom {
            x: 0,
            y: 0,
            width: 0.0,
            height: 400.0,
        };
        save_window_to(&dir, &opts, &[], Some(&junk)).unwrap();
        assert_eq!(load_with_window(&dir).2, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One unreadable part of the file may cost that part and nothing else. The case that prompted
    /// this is small and easy to hit: an options block that does not spell out every setting — written
    /// by hand, or left with one setting by a newer app that has since renamed the others. Read as one
    /// typed struct, such a block fails, and the failure used to reach the whole file: the measurement
    /// history came back empty, which is the one thing a settings file must never do to data a person
    /// cannot type again.
    #[test]
    fn a_broken_options_block_costs_only_the_options() {
        let dir = std::env::temp_dir().join(format!("system-ping-parts-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            state_path_in(&dir),
            r#"{
                "version": 2,
                "options": { "interval_secs": "one second", "horizon_secs": 600.0 },
                "history": [
                    { "id": 1, "label": "a", "host": "1.1.1.1", "kind": "Dns", "enabled": true, "color_idx": 0, "samples": [] },
                    { "id": 2, "label": "b", "host": "8.8.8.8", "kind": "Dns", "enabled": true, "color_idx": 1, "samples": [] }
                ],
                "window": { "x": 10, "y": 20, "width": 800.0, "height": 600.0 }
            }"#,
        )
        .unwrap();

        let (opts, saved, geom) = load_with_window(&dir);
        let want = AppOptions::default();
        assert_eq!(
            saved.len(),
            2,
            "both measured targets have to survive an unreadable settings block"
        );
        assert_eq!(
            (opts.interval_secs, opts.horizon_secs),
            (want.interval_secs, want.horizon_secs),
            "an options block this version cannot read falls back to the built-in settings"
        );
        assert_eq!(
            geom.map(|g| (g.x, g.y)),
            Some((10, 20)),
            "the rest of the file is still honoured"
        );

        // The other direction: a block that is merely incomplete, not wrong. Everything it does say is
        // kept, so a file written by another version of the app does not reset a person's settings just
        // because it does not know about the newest one.
        std::fs::write(
            state_path_in(&dir),
            r#"{"version":2,"options":{"horizon_secs":30.0},"history":[]}"#,
        )
        .unwrap();
        let (opts, _, _) = load_with_window(&dir);
        assert_eq!(opts.horizon_secs, 30.0);
        assert_eq!(opts.interval_secs, want.interval_secs);

        // And a file that is not JSON at all stays a default app rather than taking the program down.
        std::fs::write(state_path_in(&dir), "not json").unwrap();
        let (opts, saved, geom) = load_with_window(&dir);
        assert_eq!((saved.len(), geom), (0, None));
        assert_eq!(opts.horizon_secs, want.horizon_secs);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The chart's display mode is a setting like any other: it has to survive a restart, and a
    /// file written before the option existed has to keep drawing what it always drew — the line.
    /// Reading a missing setting as "dots" would change somebody's chart under their feet.
    #[test]
    fn the_chart_mode_survives_a_restart_and_an_old_file() {
        let dir = std::env::temp_dir().join(format!("system-ping-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Written before the option existed: no `chart_mode` anywhere in the block.
        std::fs::write(
            state_path_in(&dir),
            r#"{"version":2,"options":{"horizon_secs":300.0},"history":[]}"#,
        )
        .unwrap();
        let (opts, _, _) = load_with_window(&dir);
        assert_eq!(opts.chart_mode, crate::app::ChartMode::Lines);
        assert_eq!(opts.horizon_secs, 300.0, "the rest of the block is read");

        // Written by this version: the choice comes back as it was left.
        let opts = AppOptions {
            chart_mode: crate::app::ChartMode::Points,
            ..AppOptions::default()
        };
        save_to(&dir, &opts, &[]).unwrap();
        let (back, _, _) = load_with_window(&dir);
        assert_eq!(back.chart_mode, crate::app::ChartMode::Points);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file too big to be this app's own settings is refused instead of being read.
    ///
    /// The size limit is handed to [`read_state`] as an argument rather than read from the constant
    /// inside it for exactly this test: the real limit is 32 MiB, and writing 33 of them into the
    /// temp folder to prove one line of code works is not a test anyone runs twice.
    #[test]
    fn a_file_that_is_too_big_is_not_read() {
        let dir = std::env::temp_dir().join(format!("system-ping-size-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = state_path_in(&dir);
        std::fs::write(&path, r#"{"version":2,"history":[]}"#).unwrap();

        assert!(
            read_state(&path, u64::MAX).is_some(),
            "the file is readable"
        );
        assert!(read_state(&path, 8).is_none(), "…but not past its limit");
        // A missing file is not an error worth printing: that is the normal first start.
        assert!(read_state(&dir.join("nowhere.json"), u64::MAX).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A history element that nests deeper than the format ever goes costs that element and nothing
    /// else — the same rule as [`a_broken_options_block_costs_only_the_options`], one level down.
    /// The stack itself is already safe (serde_json refuses to nest past 128 levels), so this is
    /// about keeping the rows that are good rather than about not crashing.
    #[test]
    fn a_history_element_that_nests_too_deeply_is_dropped() {
        let dir = std::env::temp_dir().join(format!("system-ping-depth-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let deep = "[".repeat(MAX_HISTORY_DEPTH + 4) + &"]".repeat(MAX_HISTORY_DEPTH + 4);
        std::fs::write(
            state_path_in(&dir),
            format!(
                r#"{{"version":2,"history":[{deep},
                    {{"id":2,"label":"b","host":"8.8.8.8","kind":"Dns","enabled":true,"color_idx":1,"samples":[[1,2.0]]}}]}}"#,
            ),
        )
        .unwrap();

        let (_, saved, _) = load_with_window(&dir);
        assert_eq!(saved.len(), 1, "the sane row survives the insane one");
        assert_eq!(saved[0].label, "b");
        assert_eq!(saved[0].samples.len(), 1, "and its samples are intact");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Saves are written to a sibling name and moved onto the real one, so the folder after a save
    /// holds nothing a person would not recognise — and garbage from a run that died mid-save stays
    /// garbage rather than turning into a second settings file.
    #[test]
    fn a_save_leaves_no_temporary_file_behind() {
        let dir = std::env::temp_dir().join(format!("system-ping-atomic-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let opts = AppOptions::default();
        let geom = WindowGeom {
            x: 1,
            y: 2,
            width: 800.0,
            height: 600.0,
        };

        save_to(&dir, &opts, &[]).unwrap();
        save_window_to(&dir, &opts, &[], Some(&geom)).unwrap();
        let extras: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|name| name != STATE_FILE)
            .collect();
        assert!(extras.is_empty(), "unexpected files: {extras:?}");

        std::fs::write(dir.join(format!("{STATE_FILE}.1234.tmp")), "half a file").unwrap();
        assert_eq!(load_with_window(&dir).2, Some(geom));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
