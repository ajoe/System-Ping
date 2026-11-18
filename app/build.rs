// Builds the Windows resources of the EXE: a file icon in Explorer, plus version and description
// in the file properties. `winresource` generates a .res file from this and hands it to the linker
// (rc.exe or llvm-rc), so the result is exactly what a regular Windows program gets — no need to
// post-process the finished EXE.
//
// Linux and macOS have no equivalent of an embedded .res: there the icon travels as a file next to
// the binary (an .icns in a .app bundle, or a .desktop entry plus a PNG). So on those systems this
// script does nothing at all beyond declaring what it watched.
//
// The decision is made at run time from CARGO_CFG_TARGET_OS and not with #[cfg(windows)] around
// `main`. Reason: when a foreign target is cross-compiled on Windows, cargo may re-use the
// host-built build-script binary for that target. Compiled for the host that binary would contain
// the Windows branch, and would then complain about a missing rc.exe for a target that never wants
// resources. Reading the environment keeps one binary correct for every target.

use std::env;
use std::path::Path;

/// Name of the OS the crate is currently being built for, as cargo reports it.
fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn main() {
    // Re-run whenever the icon or the manifest changes: feature changes (e.g. the eframe back
    // ends) must not leave a stale build behind.
    println!("cargo::rerun-if-changed=assets/icon.ico");
    println!("cargo::rerun-if-changed=Cargo.toml");

    emit_git_hash();

    if target_os() != "windows" {
        return;
    }

    embed_windows_resources();
}

/// Put the commit this source stands on into `GIT_HASH` for the compiled program (shown in the
/// settings' Info group — the question every bug report starts with is which build someone has).
/// Empty for a tree without git, like a source tarball: a wrong hash would be worse than none, so
/// an unknown commit stays unknown and the UI leaves it out.
fn emit_git_hash() {
    // A caller may say the hash out loud — a release built from an export, a reproducible build.
    let given = env::var("SYSTEM_PING_GIT_HASH")
        .ok()
        .filter(|s| !s.is_empty());
    let hash = given.or_else(|| {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(env::var("CARGO_MANIFEST_DIR").ok()?)
            .output()
            .ok()?;
        if out.status.success() {
            String::from_utf8(out.stdout)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        }
    });
    println!("cargo::rustc-env=GIT_HASH={}", hash.unwrap_or_default());

    // Only when git is what asks: HEAD moves on checkout and rebase, refs on a commit — either has
    // to produce a new answer. Without a .git folder there is nothing to watch; the empty value
    // stands until the source itself changes, which Cargo.toml already covers.
    if Path::new("../.git").exists() {
        println!("cargo::rerun-if-changed=../.git/HEAD");
        println!("cargo::rerun-if-changed=../.git/refs");
    }
}

/// Generates and hands the Windows resource file to the linker. A failure here is only a warning:
/// the program itself builds fine, it just appears without an icon.
#[cfg(windows)]
fn embed_windows_resources() {
    let mut res = winresource::WindowsResource::new();
    // The path is relative to the manifest folder (app/), so "app/" is left off here.
    res.set_icon("../assets/icon.ico");
    res.set_language(0x0409); // English (US), so the resource is found reliably
    if let Err(e) = res.compile() {
        println!("cargo::warning=embedding the icon failed: {e}");
    }
}

/// Without a Windows host toolchain there is nothing to embed. A cross build from Linux or macOS
/// to Windows would need `x86_64-w64-mingw32-gcc` (or llvm-rc); that combination is not supported.
#[cfg(not(windows))]
fn embed_windows_resources() {
    println!("cargo::warning=icon embedding needs a Windows host; building without an icon");
}
