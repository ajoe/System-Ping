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
    /// Absolute wall-clock time of the sample as Unix milliseconds, paired with the rtt. This
    /// is what makes an export after a restart read correctly.
    pub samples: Vec<(i64, f32)>,
}

impl Clone for SavedTarget {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            label: self.label.clone(),
            host: self.host.clone(),
            kind: self.kind,
            enabled: self.enabled,
            samples: self.samples.clone(),
        }
    }
}

#[derive(serde::Serialize)]
pub struct Persisted<'a> {
    pub version: u32,
    pub options: Option<&'a AppOptions>,
    pub history: &'a [SavedTarget],
}

/// Read shape: `history` is read as raw JSON because v1 and v2 disagree about what a sample
/// pair means, and that difference cannot be expressed with one typed struct.
#[derive(serde::Deserialize)]
struct PersistedIn {
    #[serde(default)]
    version: u32,
    options: Option<AppOptions>,
    #[serde(default)]
    history: Vec<serde_json::Value>,
}

/// Where `load`/`save` read and write. Checked on every call so a test can point it
/// somewhere else at runtime (see the module docs).
///
/// The variable is process-wide, so a caller that sets it for one pass must not let another
/// thread read it at the same time — `collect_once` takes a directory and keeps the variable
/// set for the whole pass instead of leaving that to the caller.
pub fn state_dir() -> PathBuf {
    match std::env::var_os("SYSTEM_PING_STATE_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        // Prefer a writable folder next to the executable; fall back to temp.
        _ => std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(std::env::temp_dir),
    }
}

pub fn state_path() -> PathBuf {
    state_path_in(&state_dir())
}

/// The same path for a directory given by the caller. `collect_once` uses this so its own
/// reads and writes cannot be pulled to another folder by a test running at the same time.
pub fn state_path_in(dir: &std::path::Path) -> PathBuf {
    dir.join(STATE_FILE)
}

pub fn load() -> (AppOptions, Vec<SavedTarget>) {
    load_from(&state_dir())
}

/// `load` for a directory given by the caller. A file that cannot be read or parsed stays a
/// default app rather than taking the program down with it.
pub fn load_from(dir: &std::path::Path) -> (AppOptions, Vec<SavedTarget>) {
    let path = state_path_in(dir);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(p) = serde_json::from_str::<PersistedIn>(&text) {
            let opts = p.options.unwrap_or_default();
            let history = match p.version {
                2..=u32::MAX => p
                    .history
                    .iter()
                    .filter_map(from_value::<SavedTarget>)
                    .collect(),
                // Anything else (including a missing field) is the old format.
                _ => p
                    .history
                    .iter()
                    .filter_map(from_value::<SavedTargetV1>)
                    .map(SavedTarget::from_v1)
                    .collect(),
            };
            return (opts, history);
        }
    }
    (AppOptions::default(), Vec::new())
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
            samples: v
                .samples
                .into_iter()
                .enumerate()
                .map(|(i, (_, rtt))| (now_ms - ((n - i) as i64) * 1000, rtt))
                .collect(),
        }
    }
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
    let path = state_path_in(dir);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let p = Persisted {
        version: VERSION,
        options: Some(options),
        history,
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
}
