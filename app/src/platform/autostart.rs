//! Starting the app when the person signs in.
//!
//! Windows has one obvious place for this: the `HKCU\...\Run` key, which a plain `reg` command can
//! write — no registry crate needed. Linux and macOS each have their own mechanism (an autostart
//! `.desktop` entry, a LaunchAgent property list), and both also differ between desktop
//! environments in whether that entry is honoured at all. Writing a file into someone's session
//! folder without being able to verify it works is worse than not offering the switch, so on those
//! systems this says "not possible" and the settings screen shows why instead of a checkbox that
//! quietly does nothing.

/// The name this app claims in the OS's autostart list.
#[cfg(any(windows, target_os = "linux"))]
const ENTRY_NAME: &str = "SystemPing";

/// Add or remove the autostart entry. `false` means it is not there now — either because the change
/// failed, or because this system has no mechanism we can verify.
pub fn set(on: bool) -> bool {
    #[cfg(windows)]
    {
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(_) => return false,
        };
        let exe = exe.display().to_string();
        let mut cmd = hidden(std::process::Command::new("reg"));
        if on {
            cmd.args(["add", RUN_KEY]);
            cmd.args(["/v", ENTRY_NAME, "/t", "REG_SZ", "/d"]);
            cmd.arg(format!("\"{exe}\""));
            cmd.arg("/f");
        } else {
            cmd.args(["delete", RUN_KEY, "/v", ENTRY_NAME, "/f"]);
        }
        cmd.status().map(|s| s.success()).unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        let _ = on;
        false
    }
}

/// Whether the autostart entry exists. This is the read side of [`set`]; without it the switch in
/// the top bar could only ever say "off", because its counter is written only when pressed.
pub fn exists() -> bool {
    #[cfg(windows)]
    {
        // `reg query` exits 0 only when the value is really there, so its status is the whole
        // answer — no registry crate needed.
        let mut cmd = hidden(std::process::Command::new("reg"));
        cmd.args(["query", RUN_KEY, "/v", ENTRY_NAME]);
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

/// Whether this system can do autostart at all. The settings screen uses this to explain a "no"
/// instead of showing a switch that cannot be satisfied — on Linux and macOS the honest message is
/// "not supported here", not "autostart failed".
pub fn is_supported() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Keeps the `reg` console window from flashing on every call.
#[cfg(windows)]
fn hidden(mut cmd: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000);
    cmd
}

/// The path a `.desktop` autostart entry would go to, if this system had one. Kept as a documented
/// stub so whoever adds the Linux behaviour knows where it belongs; `None` when there is no home
/// folder to write into.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn autostart_entry_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    Some(base.join("autostart").join(format!("{ENTRY_NAME}.desktop")))
}
