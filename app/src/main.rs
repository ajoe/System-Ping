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
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Die Ansicht ist das erste Wort ohne Bindestrich nach dem Dateinamen — mehrere Wörter dürfen
    // es sein (`stats 6h`), ein Optionswort beendet die Suche dagegen. Sonst frisst
    // `--shot out.png --theme dark stats` das `--theme` als Ansichtsname auf und läßt den Rest
    // liegen, was stillschweigend das Diagramm statt der Messwerte zeigt.
    let view_after = |i: usize| -> Option<String> {
        let mut j = i;
        while j < args.len() && !args[j].starts_with('-') {
            j += 1;
        }
        if j == i {
            None
        } else {
            Some(args[i..j].join(" "))
        }
    };
    for (i, a) in args.iter().enumerate() {
        if a == "--screenshot" || a == "--shot" {
            return args
                .get(i + 1)
                .map(|p| (std::path::PathBuf::from(p), view_after(i + 2)));
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

/// Die Hilfe zu den Befehlszeilenargumenten. Sie steht hier und nicht in der README, weil sie die
/// einzige Erklärung ist, die jemand sieht, der das Programm im Terminal startet — und weil ein
/// Fenster für `--help` niemandem hilft. Bewusst auf Deutsch: das ganze Programm spricht Deutsch.
const HELP: &str = "\
System-Ping Open Source — Latenz-Monitor

Ohne Argumente öffnet sich das Fenster mit dem letzten Zustand (Ziele, Einstellungen,
Fenstergröße). Messwerte werden alle 8 s automatisch in den Zustandsordner geschrieben.

Argumente:
  --help, -h              diese Übersicht zeigen und beenden
  --theme <light|dark>    Aussehen für diesen Start erzwingen, ohne es zu speichern
  --collect <sekunden>    im Fenster messen, berichten und beenden (Selbsttest)
  --export-csv [datei]    gespeicherte Messwerte als CSV schreiben und beenden
                          (ohne Dateiname: Standardordner, Zeitstempel im Namen)
  --shot <datei.png> [ansicht]
                          Bild der Oberfläche rechnen und beenden; Ansichten: chart,
                          events, stats, settings, edit, dazu ein Zeitfenster wie 24h

Umgebungsvariablen:
  SYSTEM_PING_STATE_DIR   Zustandsordner (Dateien, Protokoll, Standard-Export) erzwingen
  SYSTEM_PING_MAX_FRAMES  Bilderdurchlauf nach n Bildern abbrechen (nur für Testläufe)
";

/// Command line: `--help` prints [`HELP`] and exits. Answered before anything else — no state file
/// read, no window built — because help must work even when the saved state is broken.
fn help_requested() -> bool {
    std::env::args()
        .skip(1)
        .any(|a| a == "--help" || a == "-h" || a == "/?")
}

/// The task-bar and title-bar icon, compiled into the program so no file has to travel with it.
/// Decoded from PNG at startup; on failure an empty icon is handed out, which makes egui fall
/// back to its own default rather than failing the whole start over a picture.
fn app_icon() -> eframe::egui::IconData {
    const PNG: &[u8] = include_bytes!("../../assets/icon.png");
    match image::load_from_memory(PNG) {
        Ok(img) => {
            let img = img.into_rgba8();
            let (w, h) = img.dimensions();
            eframe::egui::IconData {
                rgba: img.into_raw(),
                width: w,
                height: h,
            }
        }
        Err(e) => {
            eprintln!("icon.png konnte nicht gelesen werden: {e}");
            eframe::egui::IconData::default()
        }
    }
}

fn main() -> eframe::Result<()> {
    // Zuerst die Hilfe: sie darf nichts lesen und kein Fenster bauen, sonst scheitert sie genau in
    // dem Moment, in dem jemand sie sucht.
    if help_requested() {
        print!("{HELP}");
        return Ok(());
    }

    // Load saved settings before the window exists — including where the window sat last time.
    let (options, saved, window) = store::load_with_window(&store::state_dir());

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
        .with_title("System-Ping Open Source — Latenz-Monitor")
        .with_icon(app_icon());
    // The window comes back where it was. A geometry from a file that has since gone bad (a size
    // of zero, numbers that are not numbers) is dropped silently and the default above stands.
    if let Some(g) = window.filter(|g| g.sane()) {
        if shot.is_none() {
            // `ViewportBuilder::with_position` wants the top-left of the *frame*, in physical
            // pixels — the same thing `inner_position` plus a title-bar shift describes, which is
            // what was stored. The title bar's height is not known before the window exists, so
            // the saved inner position is used as-is and then clamped to a monitor below; on
            // Windows that offset is small and never leaves the window off screen.
            vp = vp.with_position([g.x as f32, g.y as f32]);
        }
        vp = vp.with_inner_size([g.width, g.height]);
    }
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
