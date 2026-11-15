//! system-ping — System-Ping Open Source, a clean-room network latency monitor.
//!
//! Rebuilt from the person's request only (the original program was never read).
//! Core features: ping several targets on a schedule, keep per-target history,
//! show a live latency chart, statistics, status list, alerts and CSV export.

#![windows_subsystem = "windows"]

// Everything lives in the library crate (`src/lib.rs`); this file is only the entry point,
// so `tests/` can use the same code without a window.
use system_ping::{app, store, ui};

/// Command line: `--shot <file.png> [view]` draws ~120 frames and exits.
fn screenshot_path_from_args() -> Option<(std::path::PathBuf, Option<String>)> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == "--screenshot" || a == "--shot" {
            return it.next().map(|p| (std::path::PathBuf::from(p), it.next()));
        }
        if let Some(rest) = a
            .strip_prefix("--screenshot=")
            .or_else(|| a.strip_prefix("--shot="))
        {
            return Some((std::path::PathBuf::from(rest), None));
        }
    }
    None
}

/// Command line: `--export-csv [file]` dumps the saved history and exits.
fn export_csv_from_args() -> Option<std::path::PathBuf> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == "--export-csv" || a == "--csv" {
            return Some(match it.next() {
                Some(p) if !p.starts_with("--") => std::path::PathBuf::from(p),
                _ => ui::default_export_path(),
            });
        }
    }
    None
}

/// Optional `--theme light|dark`, for checking both looks.
fn theme_override_from_args() -> Option<String> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == "--theme" {
            if let Some(v) = it.next() {
                return Some(v.to_ascii_lowercase());
            }
        }
    }
    None
}

/// Command line: `--collect <secs>` runs the real window for a while, prints how many samples
/// the live loop actually recorded, and exits. This is the check that a GUI run measures at all:
/// before the frame loop called `tick`, such a run reported zero samples forever.
fn collect_secs_from_args() -> Option<f64> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if let Some(rest) = a
            .strip_prefix("--collect=")
            .or_else(|| a.strip_prefix("--collect-secs="))
        {
            return rest.parse::<f64>().ok();
        }
        if a == "--collect" || a == "--collect-secs" {
            if let Some(v) = it.next().and_then(|v| v.parse::<f64>().ok()) {
                return Some(v);
            }
        }
    }
    None
}

fn main() -> eframe::Result<()> {
    // Load saved settings before the window exists.
    let (options, saved) = store::load();

    // `--export-csv <file>` writes the recorded samples and quits: it must never open a
    // window, because a hidden eframe window is only closed by an event loop that keeps
    // painting, and this path has nothing to paint.
    if let Some(path) = export_csv_from_args() {
        return match app::export_history_to(&saved, &path) {
            Ok(n) => {
                println!("csv: {} Zeilen -> {}", n, path.display());
                Ok(())
            }
            Err(e) => {
                eprintln!("csv failed: {}", e);
                std::process::exit(1);
            }
        };
    }

    let shot = screenshot_path_from_args();
    // `--collect <secs>` keeps the window live for a while and then reports what it measured.
    let collect = collect_secs_from_args();
    // `--max-frames <n>` is a safety net for automated runs: without it eframe would keep an
    // invisible window open forever if the self-test ever stopped closing its own window.
    let limit = std::env::var("SYSTEM_PING_MAX_FRAMES")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok());
    // A command-line theme wins over the saved one (used to check both looks). It is passed
    // to the app as an override so it never gets written back to the state file.
    let theme_override = theme_override_from_args();
    let mut vp = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 700.0])
        .with_min_inner_size([640.0, 420.0])
        .with_title("System-Ping Open Source — Latenz-Monitor");
    if shot.is_some() {
        // Off-screen run: no window on the desktop while writing the PNG.
        vp = vp.with_visible(false);
    }

    let native = eframe::NativeOptions {
        viewport: vp,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    match shot {
        Some((path, view)) => eframe::run_native(
            "System-Ping Open Source",
            native,
            Box::new(move |cc| {
                let a = system_ping::app::App::with_theme_override(
                    options,
                    saved,
                    theme_override.as_deref(),
                );
                system_ping::theme::apply(&cc.egui_ctx, a.theme_mode);
                let edit_row = view
                    .as_deref()
                    .filter(|v| system_ping::screenshot::wants_edit(v))
                    .map(|_| 0);
                let mut cap = system_ping::screenshot::Screencap::new(a, cc, path, view);
                cap.show_editor(edit_row);
                Ok(Box::new(cap))
            }),
        ),
        None => eframe::run_native(
            "System-Ping Open Source",
            native,
            Box::new(move |cc| {
                let a = system_ping::app::App::with_theme_override(
                    options,
                    saved,
                    theme_override.as_deref(),
                );
                // Paint the chosen look before the first frame is drawn.
                system_ping::theme::apply(&cc.egui_ctx, a.theme_mode);
                match collect {
                    Some(secs) => Ok(Box::new(system_ping::screenshot::SelfTest::new(
                        a, cc, secs, limit,
                    ))),
                    None => Ok(Box::new(a)),
                }
            }),
        ),
    }
}
