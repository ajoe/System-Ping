//! Saved settings and history, as JSON next to the executable.
//!
//! The file lives next to the binary by default (the app folder stays self-contained).
//! `SYSTEM_PING_STATE_DIR` overrides that location for the window and for a whole test
//! binary. The headless seams (`app::collect_once`, `App::save_now_to`) additionally take the
//! directory as an argument, so tests running side by side never share one file.

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

/// One part of the file, or `None` when it is absent or cannot be read in this version's terms. Costs
/// that part and nothing else (see [`PersistedIn`]).
fn part<T: serde::de::DeserializeOwned>(v: &Option<serde_json::Value>) -> Option<T> {
    v.as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// The entries of a history that may not be a list at all. Unreadable *elements* are dropped further
/// down, one at a time, by [`from_value`].
fn items(v: &Option<serde_json::Value>) -> Vec<serde_json::Value> {
    v.as_ref()
        .and_then(|v| v.as_array())
        .cloned()
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
    if let Ok(text) = std::fs::read_to_string(&path) {
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
    std::fs::write(&path, text)
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
}
