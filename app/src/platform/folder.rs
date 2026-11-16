//! Opening the state folder in the system's file manager.
//!
//! The button next to "CSV" does one thing: show the person the file that was just written. Each
//! system has its own way of being asked, and none of them is portable, so the choice lives here
//! rather than scattered through the UI code.

use std::path::Path;

/// Open `dir` in whatever shows folders on this system. Returns whether a helper process could be
/// started at all — not whether a window appeared, which nobody outside the desktop session can
/// tell. The caller keeps its status message honest with that answer.
pub fn open(dir: &Path) -> bool {
    spawn_for(dir).is_some()
}

/// The command to run for one folder. `None` means "this system has no file manager we know how to
/// ask", which is the honest answer on a bare server and better than silently doing nothing while
/// the status line claims success.
fn spawn_for(dir: &Path) -> Option<std::process::Child> {
    #[cfg(windows)]
    {
        // `explorer.exe` is a separate process, so it does not block until its window closes — and
        // unlike `cmd /c start` it needs no shell to interpret the path.
        hidden(&mut std::process::Command::new("explorer.exe"))
            .arg(dir)
            .spawn()
            .ok()
    }

    #[cfg(target_os = "macos")]
    {
        // `open` is the documented way; it hands the request to Finder and returns immediately.
        std::process::Command::new("open").arg(dir).spawn().ok()
    }

    #[cfg(target_os = "linux")]
    {
        // No single command owns this on Linux, so try them in order of how standard they are:
        // `xdg-open` respects the desktop's choice, `gio open` works on a GTK session without the
        // xdg-utils package, and the three managers cover a desktop that ships neither.
        let mut cmds: Vec<(&str, Vec<&str>)> = vec![("xdg-open", vec![])];
        if let Some(s) = dir.to_str() {
            cmds.push(("gio", vec!["open", s]));
        }
        for (exe, extra) in cmds {
            if let Some(child) = std::process::Command::new(exe)
                .args(extra)
                .arg(dir)
                .spawn()
                .ok()
            {
                return Some(child);
            }
        }
        None
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = dir;
        None
    }
}

/// Keeps the helper's own console window from flashing (Windows only — the flag does not exist
/// elsewhere).
#[cfg(windows)]
fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}
