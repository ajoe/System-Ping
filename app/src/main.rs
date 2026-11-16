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
    // The view is the first word after the file name that does not start with a dash — it may be
    // several words (`stats 6h`), while an option word ends the search. Otherwise
    // `--shot out.png --theme dark stats` would swallow `--theme` as the view name and leave the
    // rest behind, quietly showing the chart instead of the statistics.
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

/// Command line: `--lang <de|en>` picks the interface language for this run, without touching the
/// saved file — the same rule as `--theme`. A word that is neither language simply says nothing, so
/// a typo cannot silently switch the window to something nobody asked for.
fn lang_override_from_args() -> Option<system_ping::i18n::Lang> {
    let known = |s: &str| match s.to_ascii_lowercase().as_str() {
        "de" | "en" => Some(system_ping::i18n::Lang::from_code(s)),
        _ => None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == "--lang" || a == "--language" {
            if let Some(v) = it.next().and_then(|v| known(&v)) {
                return Some(v);
            }
        }
        if let Some(rest) = a
            .strip_prefix("--lang=")
            .or_else(|| a.strip_prefix("--language="))
        {
            if let Some(l) = known(rest) {
                return Some(l);
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

/// The command-line help, in both languages. It lives here rather than in the README because it is
/// the only explanation someone who starts the program in a terminal will actually see — and
/// because a window for `--help` helps nobody. Which table answers depends on `--lang`; without it,
/// German. Keep both between the HELPSTART/HELPEND markers so the READMEs can quote them as the
/// program's own output.
const HELP_DE: &str = "\
System-Ping Open Source — Latenz-Monitor

Ohne Argumente öffnet sich das Fenster mit dem letzten Zustand (Ziele, Einstellungen,
Fenstergröße). Messwerte werden alle 8 s automatisch in den Zustandsordner geschrieben.

Argumente:
  --help, -h              diese Übersicht zeigen und beenden
  --lang <de|en>          Sprache der Oberfläche für diesen Start, ohne sie zu speichern
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

const HELP_EN: &str = "\
System-Ping Open Source — latency monitor

Without arguments the window opens with the last state (targets, settings, window size).
Measurements are written to the state folder automatically every 8 s.

Arguments:
  --help, -h              show this overview and exit
  --lang <de|en>          interface language for this run, without saving it
  --theme <light|dark>    force the look for this run, without saving it
  --collect <seconds>     measure in the window, report and exit (self test)
  --export-csv [file]     write the stored measurements as CSV and exit
                          (without a file name: default folder, timestamp in the name)
  --shot <file.png> [view]
                          render a picture of the interface and exit; views: chart,
                          events, stats, settings, edit, plus a horizon such as 24h

Environment variables:
  SYSTEM_PING_STATE_DIR   force the state folder (files, protocol, default export)
  SYSTEM_PING_MAX_FRAMES  abort the frame loop after n pictures (test runs only)
";

/// The help text for this run's language. `--lang` is read straight from the command line here, so
/// a broken state file can still not keep `--help` from answering.
fn help_text() -> &'static str {
    match lang_override_from_args() {
        Some(system_ping::i18n::Lang::En) => HELP_EN,
        _ => HELP_DE,
    }
}

/// Command line: `--help` prints [`HELP_DE`] or [`HELP_EN`] and exits. Answered before anything else
/// — no state file read, no window built — because help must work even when the saved state is
/// broken.
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
            eprintln!("could not read icon.png: {e}");
            eframe::egui::IconData::default()
        }
    }
}

fn main() -> eframe::Result<()> {
    // Help comes first: it must not read anything or open a window, or it fails at exactly the
    // moment someone is looking for it.
    if help_requested() {
        print!("{}", help_text());
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
    // The same for the interface language: the title bar has to speak the language of this run,
    // and it is built before the app exists.
    let lang = lang_override_from_args().unwrap_or(options.language);
    let mut vp = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 700.0])
        .with_min_inner_size([640.0, 420.0])
        .with_title(lang.app_title())
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
            lang.app_title(),
            native,
            Box::new(move |cc| {
                let mut a = system_ping::app::App::with_theme_override(
                    options,
                    saved,
                    theme_override.as_deref(),
                );
                // A `--lang` run shows that language without recording it in the file.
                app::set_language_silently(&mut a, lang);
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
            lang.app_title(),
            native,
            Box::new(move |cc| {
                let mut a = system_ping::app::App::with_theme_override(
                    options,
                    saved,
                    theme_override.as_deref(),
                );
                // A `--lang` run shows that language without recording it in the file.
                app::set_language_silently(&mut a, lang);
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
