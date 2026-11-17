//! Does the app tell when its own saving goes wrong?
//!
//! The state file is written on a timer and on every change, so a save that fails is easy to miss —
//! and the atomic writer makes one thing harder to notice, not easier: the old file stays perfectly
//! readable, so nothing about the app *looks* broken. These two tests hold the two ends of that
//! apart — a save that could not happen must say so and must not claim to have happened, a save that
//! worked must be on disk and must age the status line.
//!
//! Both tests own `SYSTEM_PING_STATE_DIR` for as long as they run (see `tests/collection.rs` for why
//! that needs a lock), which is also why they are a file of their own: the variable is process-wide.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static STATE_DIR_LOCK: Mutex<()> = Mutex::new(());

/// A throwaway path used as the state folder, removed at the end. `blocking` is the trick one of the
/// tests needs: a regular *file* where a folder would have to be, so every write below it fails for
/// the reason the operating system gives rather than for a reason this test arranged on purpose.
struct StateDir {
    dir: PathBuf,
    blocking: Option<PathBuf>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl StateDir {
    /// Normal case: an empty folder that accepts writes.
    fn new(name: &str) -> Self {
        Self::with(name, false)
    }

    /// Broken case: `<name>-blocked` is a file, and the state folder is meant to be inside it.
    fn blocked(name: &str) -> Self {
        Self::with(name, true)
    }

    fn with(name: &str, blocked: bool) -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let tag = format!(
            "system-ping-test-{}-{}",
            name,
            SEQ.fetch_add(1, Ordering::SeqCst)
        );
        let mut dir = std::env::temp_dir();
        dir.push(&tag);
        let mut blocking = None;
        if blocked {
            let file = dir;
            let _ = std::fs::remove_file(&file);
            std::fs::write(&file, "in the way").expect("the blocking file must be writable");
            blocking = Some(file.clone());
            dir = file.join("state");
        } else {
            let _ = std::fs::create_dir_all(&dir);
        }
        let _guard = STATE_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);
        Self {
            dir,
            blocking,
            _guard,
        }
    }

    fn state_file(&self) -> PathBuf {
        self.dir.join("system-ping-state.json")
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        if let Some(f) = &self.blocking {
            let _ = std::fs::remove_file(f);
        }
        if std::env::var_os("SYSTEM_PING_STATE_DIR").as_deref() == Some(self.dir.as_os_str()) {
            std::env::remove_var("SYSTEM_PING_STATE_DIR");
        }
    }
}

/// The part of the save-failure text that comes before the error, in the language the app starts in.
/// Matching on this rather than on a fixed sentence keeps the test about *whether* the line appeared
/// and not about the spelling of one translation.
fn failure_prefix() -> String {
    let template = system_ping::i18n::Lang::default().msg_save_failed_fmt();
    let brace = template.find('{').unwrap_or(template.len());
    let prefix = template[..brace].trim_end();
    assert!(!prefix.is_empty(), "the text has nothing before the error");
    prefix.to_string()
}

/// A state folder that cannot be written to: the app says so, and does not afterwards claim it just
/// saved something.
#[test]
fn a_save_that_cannot_write_says_so_and_does_not_count() {
    let dir = StateDir::blocked("save-blocked");
    let mut app = system_ping::app::App::new(system_ping::app::AppOptions::default(), Vec::new());
    assert!(
        app.save_age_secs() < 0.0,
        "a fresh app has not saved anything yet"
    );

    app.save_now();

    let note = app
        .status_auto_note()
        .expect("a failed save must report itself");
    let prefix = failure_prefix();
    assert!(
        note.starts_with(&prefix),
        "expected {prefix:?}…, got {note:?}"
    );
    assert!(
        note.len() > prefix.len() + 2,
        "the line must carry the error, not just its heading: {note:?}"
    );
    assert!(
        app.save_age_secs() < 0.0,
        "a save that did not happen must not age the status line"
    );
    assert!(
        !dir.state_file().exists(),
        "and it must not leave a state file behind either"
    );
}

/// The same call with a folder that accepts writes: the file is there, and the status line starts
/// counting from that moment.
#[test]
fn a_save_that_works_leaves_a_file_and_ages_the_line() {
    let dir = StateDir::new("save-works");
    let mut app = system_ping::app::App::new(system_ping::app::AppOptions::default(), Vec::new());
    // The age is measured from the process start, so give that clock something to show. Without
    // this a save in the first microseconds would still report "nothing saved yet".
    std::thread::sleep(std::time::Duration::from_millis(20));

    app.save_now();

    assert!(dir.state_file().is_file(), "the state file must exist");
    assert!(
        app.save_age_secs() >= 0.0,
        "a save that worked must age the status line"
    );
    assert!(
        !app.status_auto_note()
            .unwrap_or_default()
            .starts_with(&failure_prefix()),
        "nothing went wrong, so no failure line may stand in the bar"
    );
}
