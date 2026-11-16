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

/// Name of the OS the crate is currently being built for, as cargo reports it.
fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn main() {
    // Re-run whenever the icon or the manifest changes: feature changes (e.g. the eframe back
    // ends) must not leave a stale build behind.
    println!("cargo::rerun-if-changed=assets/icon.ico");
    println!("cargo::rerun-if-changed=Cargo.toml");

    if target_os() != "windows" {
        return;
    }

    embed_windows_resources();
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
