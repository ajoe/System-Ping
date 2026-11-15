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
                Ok(Box::new(system_ping::screenshot::Screencap::new(
                    a, cc, path, view,
                )))
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
                Ok(Box::new(a))
            }),
        ),
    }
}
