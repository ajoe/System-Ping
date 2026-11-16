//! Where the state file, the protocol file and the default export go.
//!
//! Two ideas collide here. A Windows user expects a self-contained folder — double-click the EXE,
//! everything stays next to it. On Linux and macOS writing next to the binary is usually not
//! possible at all: the program sits in `/usr/bin` or an application bundle that only root may
//! write to, and even where it would work, a hidden JSON file inside a system folder is not where
//! anyone looks for their data. So the default differs by system, while `SYSTEM_PING_STATE_DIR`
//! overrides both — which is also what the tests use to keep out of each other's way.

use std::path::PathBuf;

/// The name of the environment variable that forces a folder. Public because `--help` promises it
/// and the integration tests set it.
pub const ENV_DIR: &str = "SYSTEM_PING_STATE_DIR";

/// Where to look for the files if nothing is forced. Never fails: when even the home folder is
/// unknown, the temporary folder will do — losing history on a reboot is better than not starting.
pub fn default_dir() -> PathBuf {
    match std::env::var_os(ENV_DIR) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => os_default_dir(),
    }
}

/// Windows: next to the executable, so the app stays portable on a stick. If the binary's own
/// folder cannot even be named (a stripped `/proc/self/exe`), fall through to the data folder.
#[cfg(windows)]
fn os_default_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(data_dir)
}

/// Linux and macOS: the per-user data folder. On Windows this is not used, which is why it is only
/// compiled where it is needed.
#[cfg(not(windows))]
fn os_default_dir() -> PathBuf {
    data_dir()
}

/// The OS's own answer for "where a program keeps its files": `%LOCALAPPDATA%` on Windows, XDG on
/// Linux, `~/Library/Application Support` on macOS. Only used with a name appended, so the app
/// never shares its folder with everything else that asks.
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    let base = || {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            // `%USERPROFILE%\AppData\Local` is where LOCALAPPDATA points by default; spelling it
            // out keeps the app working for someone who runs with a stripped environment.
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(|p| PathBuf::from(p).join("AppData").join("Local"))
            })
    };

    #[cfg(target_os = "linux")]
    let base = || {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
    };

    #[cfg(target_os = "macos")]
    let base = || {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    };

    // Anything else: the temporary folder, which is at least writable somewhere.
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let base = || Option::<PathBuf>::None;

    base()
        .map(|d| d.join(APP_FOLDER))
        // Last resort, and the same one on every system.
        .unwrap_or_else(std::env::temp_dir)
}

/// The application's own folder name inside the OS data folder. Kept in lower case with a dash:
// that is what both XDG and `%LOCALAPPDATA%` conventionally look like, and it matches the crate
// name so `cargo run` and an installed build agree about where the file went.
const APP_FOLDER: &str = "system-ping";

#[cfg(test)]
mod tests {
    use super::*;

    /// The forced folder wins over everything — this is the seam every test relies on, so it has to
    /// hold on all three systems.
    #[test]
    fn env_var_wins() {
        // Set in-process: the variable is only read by these calls, and no other thread of a unit
        // test reads it (the integration tests use their own process).
        std::env::set_var(ENV_DIR, std::env::temp_dir().join("sp-env-wins"));
        assert!(default_dir().ends_with("sp-env-wins"));
        std::env::remove_var(ENV_DIR);
    }

    /// An empty `SYSTEM_PING_STATE_DIR=` must not turn into the current directory, which is what
    /// `PathBuf::from("")` would mean.
    #[test]
    fn empty_env_var_is_ignored() {
        std::env::set_var(ENV_DIR, "");
        assert_ne!(default_dir(), PathBuf::from(""));
        std::env::remove_var(ENV_DIR);
    }

    /// The data folder is the application's own — not the shared OS folder itself. Otherwise the
    /// state file would sit beside every other program's files.
    #[test]
    fn data_dir_is_app_scoped() {
        assert!(data_dir().ends_with(APP_FOLDER));
    }

    /// Whatever the system, a name has to come out — the settings screen shows this path and the
    /// "Data" button opens it, so an empty answer would be a dead control.
    #[test]
    fn default_dir_is_never_empty() {
        assert!(!default_dir().as_os_str().is_empty());
    }
}
