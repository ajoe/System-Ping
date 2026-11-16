//! Interface language: every text the interface shows comes from this table.
//!
//! Two languages are offered, German and English, and one [`Lang`] is chosen for the whole session
//! (stored in the state file as `language`). Texts are reached through short methods on [`Lang`], so
//! a call site reads `lang.chart()` — no string keys that can be mistyped. A missing translation is
//! impossible by construction: both languages live in the same macro, and the compiler checks that
//! every arm answers both.
//!
//! Three kinds of text deliberately do *not* come from here:
//! - Log lines (the event log and the protocol file) are German, because they are data that outlives
//!   a language switch — an exported CSV or protocol must read the same however the app is set.
//! - Exported column headers (`timestamp,target,latency_ms,status`) are fixed for the same reason.
//! - Identifiers such as [`crate::app::View::label`] or the theme names "system"/"light"/"dark",
//!   which the screenshot mode, the state file and the keyboard use to find things.

use serde::{Deserialize, Serialize};

/// The two interface languages. German is the default: the program was written in German and its
/// documentation is German.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    De,
    En,
}

impl Lang {
    /// Parse a stored or typed value; anything unknown stays German so an old or damaged state file
    /// cannot break the start.
    pub fn from_code(s: &str) -> Lang {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "english" | "eng" => Lang::En,
            _ => Lang::De,
        }
    }

    /// Code used in the state file and on the command line.
    pub fn code(&self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
        }
    }

    /// The name of the language, always in that language itself — a picker has to be readable even
    /// when it does not yet speak the language it offers.
    pub fn native_name(&self) -> &'static str {
        match self {
            Lang::De => "Deutsch",
            Lang::En => "English",
        }
    }

    /// Both languages, in the order the picker shows them.
    pub const ALL: [Lang; 2] = [Lang::De, Lang::En];

    /// The name of one view in this language. It lives here rather than on `View` so the whole text
    /// table stays in one file.
    pub fn view(&self, v: crate::app::View) -> &'static str {
        use crate::app::View;
        match (self, v) {
            (Lang::De, View::Chart) => "Diagramm",
            (Lang::En, View::Chart) => "Chart",
            (Lang::De, View::Events) => "Ereignisse",
            (Lang::En, View::Events) => "Events",
            (Lang::De, View::Stats) => "Messwerte",
            (Lang::En, View::Stats) => "Statistics",
            (Lang::De, View::Settings) => "Einstellungen",
            (Lang::En, View::Settings) => "Settings",
        }
    }

    /// What one view is good for, as a tooltip on its button.
    pub fn view_hint(&self, v: crate::app::View) -> &'static str {
        use crate::app::View;
        match (self, v) {
            (Lang::De, View::Chart) => "Latenzverlauf als Kurve, mit Ausreißern markiert",
            (Lang::En, View::Chart) => "The latency history as a curve, with outliers marked",
            (Lang::De, View::Events) => "Statusliste und Ereignisprotokoll",
            (Lang::En, View::Events) => "Status list and event log",
            (Lang::De, View::Stats) => "Kennzahlen je Ziel",
            (Lang::En, View::Stats) => "The numbers for every target",
            (Lang::De, View::Settings) => "Intervall, Zeitfenster, Export",
            (Lang::En, View::Settings) => "Interval, time window, export",
        }
    }

    /// The name of one category in this language. [`crate::app::TargetKind::label`] stays German for
    /// the same reason `View::label` does — it is used as an identifier.
    pub fn kind(&self, k: crate::app::TargetKind) -> &'static str {
        use crate::app::TargetKind;
        match (self, k) {
            (Lang::De, TargetKind::Gateway) => "Gateway",
            (Lang::En, TargetKind::Gateway) => "Gateway",
            (Lang::De, TargetKind::Dns) => "DNS",
            (Lang::En, TargetKind::Dns) => "DNS",
            (Lang::De, TargetKind::Internet) => "Internet",
            (Lang::En, TargetKind::Internet) => "Internet",
            (Lang::De, TargetKind::Lan) => "Lokales Netz",
            (Lang::En, TargetKind::Lan) => "Local network",
            (Lang::De, TargetKind::Custom) => "Eigene",
            (Lang::En, TargetKind::Custom) => "Custom",
        }
    }
}

/// One text of the interface. Grouped by where it appears; `{name}` inside a string is a placeholder
/// that [`fill`] replaces.
macro_rules! texts {
    // ---- top bar ----
    (de subtitle) => { "Latenz-Monitor" };
    (en subtitle) => { "Latency monitor" };
    (de tip_horizon) => { "Zeitfenster des Diagramms" };
    (en tip_horizon) => { "The chart's time window" };
    (de tip_follow_live) => { "Dem aktuellen Zeitfenster folgen — aus, wenn im Diagramm zurückgeblättert \
                              wird; Doppelklick ins Diagramm oder Esc schaltet es wieder ein" };
    (en tip_follow_live) => { "Follow the current time window — switches off when the chart is paged \
                             back; double-clicking the chart or pressing Esc turns it on again" };
    (de data_folder_btn) => { "⌂ Daten" };
    (en data_folder_btn) => { "⌂ Data" };
    (de tip_data_folder) => { "Zustandsordner im Dateimanager öffnen (Datei, Protokoll, Exporte)" };
    (en tip_data_folder) => { "Open the state folder in the file manager (state file, protocol, exports)" };
    (de autostart_on) => { "Autostart: an" };
    (en autostart_on) => { "Autostart: on" };
    (de autostart_off) => { "Autostart: aus" };
    (en autostart_off) => { "Autostart: off" };
    (de tip_autostart_top) => { "Beim Anmelden starten" };
    (en tip_autostart_top) => { "Start when you sign in" };
    (de stop_all) => { "■ Alle stoppen" };
    (en stop_all) => { "■ Stop all" };
    (de start_all) => { "▶ Alle starten" };
    (en start_all) => { "▶ Start all" };
    (de hint_new_host) => { "Ziel, z. B. 192.168.1.1" };
    (en hint_new_host) => { "Target, e.g. 192.168.1.1" };
    (de hint_new_label) => { "Name (optional)" };
    (en hint_new_label) => { "Name (optional)" };
    (de add_target_short) => { "+ Ziel" };
    (en add_target_short) => { "+ Target" };
    (de hint_filter) => { "Ziele filtern" };
    (en hint_filter) => { "Filter targets" };

    // ---- top bar: views and category chips ----
    (de all) => { "Alle" };
    (en all) => { "All" };
    (de filter_off_fmt) => { "{k} aus" };
    (en filter_off_fmt) => { "{k} off" };
    (de tip_kind_show_fmt) => { "{k} wieder einblenden — im Moment ist die ganze Kategorie verborgen" };
    (en tip_kind_show_fmt) => { "Show {k} again — right now the whole category is hidden" };
    (de tip_kind_hide_fmt) => { "{k} ausblenden — alle Ziele dieser Kategorie sind im Diagramm und in \
                               den Listen verborgen, gemessen wird weiter" };
    (en tip_kind_hide_fmt) => { "Hide {k} — every target of this category disappears from the chart and \
                               the lists, measuring continues" };
    (de tip_all_kinds) => { "Alle Kategorien wieder einblenden" };
    (en tip_all_kinds) => { "Show every category again" };

    // ---- left panel ----
    (de targets) => { "Ziele" };
    (en targets) => { "Targets" };
    (de paused) => { "Pausiert" };
    (en paused) => { "Paused" };
    (de waiting) => { "wartet …" };
    (en waiting) => { "waiting …" };
    (de not_probed_yet) => { "noch nicht geprüft" };
    (en not_probed_yet) => { "not probed yet" };
    (de tip_color_auto) => { "Linienfarbe automatisch — im Editor wählbar" };
    (en tip_color_auto) => { "Line colour picked automatically — choose your own in the editor" };
    (de tip_color_chosen_fmt) => { "Linienfarbe selbst gewählt ({n})" };
    (en tip_color_chosen_fmt) => { "Line colour chosen by hand ({n})" };

    // ---- inline target editor ----
    (de auto_color) => { "Auto" };
    (en auto_color) => { "Auto" };
    (de loss_count_fmt) => { "{n} Verluste" };
    (en loss_count_fmt) => { "{n} losses" };

    // ---- right panel: status list ----
    (de status_list) => { "Statusliste" };
    (en status_list) => { "Status list" };
    (de tip_show_target) => { "Ziel wieder anzeigen" };
    (en tip_show_target) => { "Show the target again" };
    (de hidden_suffix) => { "  · ausgeblendet" };
    (en hidden_suffix) => { "  · hidden" };
    (de filtered_suffix) => { "  · gefiltert" };
    (en filtered_suffix) => { "  · filtered" };
    (de waiting_short) => { "wartet" };
    (en waiting_short) => { "waiting" };
    (de stats_line_fmt) => { "min {min} · Ø {avg} · p95 {p95} · max {max} · Verlust {loss} %" };
    (en stats_line_fmt) => { "min {min} · avg {avg} · p95 {p95} · max {max} · loss {loss} %" };
    (de outliers_suffix_fmt) => { " · {n} Ausreißer" };
    (en outliers_suffix_fmt) => { " · {n} outliers" };

    // ---- bottom bar ----
    (de running) => { "läuft" };
    (en running) => { "running" };
    (de stopped) => { "gestoppt" };
    (en stopped) => { "stopped" };
    (de probes_fmt) => { "Prüfungen: {n}" };
    (en probes_fmt) => { "Probes: {n}" };
    (de losses_fmt) => { "Verluste: {n}" };
    (en losses_fmt) => { "Losses: {n}" };

    // ---- chart ----
    (de back_btn) => { "◀ zurück" };
    (en back_btn) => { "◀ back" };
    (de next_btn) => { "⚑ nächste" };
    (en next_btn) => { "⚑ next" };
    (de earlier_btn) => { "frühere ⚐" };
    (en earlier_btn) => { "earlier ⚐" };
    (de tip_next_outlier) => { "Zum nächsten Ausreißer springen" };
    (en tip_next_outlier) => { "Jump to the next outlier" };
    (de tip_earlier) => { "Zu einem früheren Ausreißer zurückspringen" };
    (en tip_earlier) => { "Jump back to an earlier outlier" };
    (de forward_btn) => { "vor ▶" };
    (en forward_btn) => { "forward ▶" };
    (de chart_heading) => { "Latenzverlauf" };
    (en chart_heading) => { "Latency history" };
    (de tip_legend_hidden_fmt) => { "{host} · ausgeblendet — klicken zum Einblenden" };
    (en tip_legend_hidden_fmt) => { "{host} · hidden — click to show it again" };
    (de tip_legend_shown_fmt) => { "{host} · {status} — klicken zum Ausblenden" };
    (en tip_legend_shown_fmt) => { "{host} · {status} — click to hide it" };
    (de why_hidden) => { "Alle Ziele sind ausgeblendet — klicke ihren Namen in der Legende an, um sie \
                        zurückzuholen." };
    (en why_hidden) => { "Every target is hidden — click its name in the legend to bring it back." };
    (de why_filter) => { "Kein Ziel passt zum Textfilter oben rechts — mit ✕ oder einem leeren Feld \
                        geht es weiter." };
    (en why_filter) => { "No target matches the text filter at the top right — press ✕ or clear the \
                        field to continue." };
    (de marker_outlier) => { "  <- Ausr." };
    (en marker_outlier) => { "  <- outlier" };
    (de all_kinds_off) => { "Alle Kategorien sind über die Schalter oben ausgeblendet." };
    (en all_kinds_off) => { "Every category is switched off by the chips above." };
    (de all_targets_hidden) => { "Alle Ziele sind ausgeblendet — in der Legende des Diagramms holen sie ihre Namen zurück." };
    (en all_targets_hidden) => { "Every target is hidden — the chart's legend brings their names back." };
    (de filter_no_match) => { "Kein Ziel passt zum Textfilter oben rechts." };
    (en filter_no_match) => { "No target matches the text filter at the top right." };

    // ---- events view ----
    (de events) => { "Ereignisse" };
    (en events) => { "Events" };
    (de only_problems) => { "nur Störungen" };
    (en only_problems) => { "only problems" };
    (de tip_only_problems) => { "Nur Ausfälle und Wiederherstellungen zeigen" };
    (en tip_only_problems) => { "Show only outages and recoveries" };
    (de clear_log_btn) => { "leeren" };
    (en clear_log_btn) => { "clear" };
    (de log_count_fmt) => { "{n} von {all}" };
    (en log_count_fmt) => { "{n} of {all}" };
    (de log_count_paren_fmt) => { "({n})" };
    (en log_count_paren_fmt) => { "({n})" };
    (de events_count_fmt) => { "Ereignisse: {n} von {all}" };
    (en events_count_fmt) => { "Events: {n} of {all}" };
    (de events_count_short_fmt) => { "Ereignisse: {n}" };
    (en events_count_short_fmt) => { "Events: {n}" };
    (de tip_clear_log) => { "Liste leeren — die Dateien bleiben unverändert" };
    (en tip_clear_log) => { "Clear the list — the files stay untouched" };

    // ---- statistics view ----
    (de col_target) => { "Ziel" };
    (en col_target) => { "Target" };
    (de col_host) => { "Host" };
    (en col_host) => { "Host" };
    (de col_status) => { "Status" };
    (en col_status) => { "Status" };
    (de col_min) => { "Min" };
    (en col_min) => { "Min" };
    (de col_avg) => { "Ø" };
    (en col_avg) => { "Avg" };
    (de col_median) => { "Median" };
    (en col_median) => { "Median" };
    (de col_p95) => { "p95" };
    (en col_p95) => { "p95" };
    (de col_max) => { "Max" };
    (en col_max) => { "Max" };
    (de col_loss) => { "Verlust" };
    (en col_loss) => { "Loss" };
    (de col_outliers) => { "Ausr." };
    (en col_outliers) => { "Outl." };
    (de stats_heading) => { "Statistiken" };
    (en stats_heading) => { "Statistics" };
    (de tip_sort_fmt) => { "Nach {c} sortieren — nochmaliger Klick kehrt die Richtung um" };
    (en tip_sort_fmt) => { "Sort by {c} — clicking again reverses the order" };
    (de total_probes_fmt) => { "Gesamt · {n} Prüfungen" };
    (en total_probes_fmt) => { "Total · {n} probes" };
    (de ms_suffix) => { " ms" };
    (en ms_suffix) => { " ms" };
    (de timeout_word) => { "Timeout" };
    (en timeout_word) => { "Timeout" };

    // ---- settings view: headings ----
    (de measuring) => { "Messen" };
    (en measuring) => { "Measuring" };
    (de start_group) => { "Start" };
    (en start_group) => { "Startup" };
    (de data_group) => { "Daten" };
    (en data_group) => { "Data" };
    (de appearance_group) => { "Darstellung" };
    (en appearance_group) => { "Appearance" };
    (de info_group) => { "Info" };
    (en info_group) => { "About" };
    (de language) => { "Sprache" };
    (en language) => { "Language" };
    (de tip_language_fmt) => { "Oberfläche auf {name} umstellen — die Wahl wird sofort gespeichert." };
    (en tip_language_fmt) => { "Switch the interface to {name} — the choice is saved at once." };
    (de theme_scheme_label) => { "Farbschema" };
    (en theme_scheme_label) => { "Colour scheme" };
    (de dark) => { "Dunkel" };
    (en dark) => { "Dark" };
    (de light) => { "Hell" };
    (en light) => { "Light" };
    (de system_theme) => { "System" };
    (en system_theme) => { "System" };
    (de tip_system_theme) => { "„System“ folgt der Hell-/Dunkel-Einstellung von Windows und wechselt sofort." };
    (en tip_system_theme) => { "“System” follows the light/dark setting of Windows and switches at once." };
    (de interval_now_fmt) => { "Intervall: {n} — oben in der Leiste einzustellen" };
    (en interval_now_fmt) => { "Interval: {n} — set it in the bar above" };
    (de timeout_label) => { "Zeitüberschreitung" };
    (en timeout_label) => { "Timeout" };
    (de dash) => { "—" };
    (en dash) => { "—" };
    (de concurrency_label) => { "Gleichzeitige Prüfungen" };
    (en concurrency_label) => { "Concurrent probes" };
    (de max_samples_label) => { "Max. Messwerte je Ziel" };
    (en max_samples_label) => { "Max. samples per target" };
    (de horizon_label_text) => { "Zeitfenster" };
    (en horizon_label_text) => { "Time window" };
    (de tip_max_samples_fmt) => { "0 = automatisch ({n} für das aktuelle Fenster). Obergrenze ist das \
                                 Zehnfache des Fensters bei {interval} Abstand." };
    (en tip_max_samples_fmt) => { "0 = automatic ({n} for the current window). The ceiling is ten \
                                 times the window at an interval of {interval}." };
    (de protocol_label) => { "Protokolldatei schreiben" };
    (en protocol_label) => { "Write a protocol file" };
    (de tip_protocol_log) => { "Jede Messung wird sofort in eine Datei geschrieben, nicht erst beim \
                              Export." };
    (en tip_protocol_log) => { "Every measurement is written to a file at once, not only on export." };
    (de hint_protocol_path) => { "Standard: neben den gespeicherten Daten (protocol.csv)" };
    (en hint_protocol_path) => { "Default: next to the saved data (protocol.csv)" };
    (de autostart_checkbox) => { "Beim Anmelden starten" };
    (en autostart_checkbox) => { "Start at sign-in" };
    (de autostart_blocked) => { "Autostart nicht möglich" };
    (en autostart_blocked) => { "Autostart not possible" };
    (de autostart_unsupported) => { "Autostart wird von diesem System nicht unterstützt" };
    (en autostart_unsupported) => { "This system does not support autostart" };
    (de export_csv) => { "CSV exportieren …" };
    (en export_csv) => { "Export CSV …" };
    (de tip_export_default) => { "Schreibt in den Ordner der gespeicherten Einstellungen" };
    (en tip_export_default) => { "Writes into the folder of the saved settings" };
    (de save_as) => { "Speichern unter…" };
    (en save_as) => { "Save as…" };
    (de tip_save_as) => { "Ordner und Dateiname selbst wählen" };
    (en tip_save_as) => { "Choose the folder and the file name yourself" };
    (de clear_measurements_q) => { "Wirklich löschen?" };
    (en clear_measurements_q) => { "Really delete?" };
    (de clear_measurements) => { "Messwerte löschen" };
    (en clear_measurements) => { "Delete measurements" };
    (de tip_clear_again) => { "Nochmal klicken: Verlauf und Statistik aller Ziele löschen." };
    (en tip_clear_again) => { "Click again: delete the history and statistics of every target." };
    (de tip_clear_measurements) => { "Verlauf und Statistik aller Ziele leeren. Die Ziele selbst bleiben \
                                    erhalten. Zwei Klicks nötig." };
    (en tip_clear_measurements) => { "Empty the history and statistics of every target. The targets \
                                    themselves stay. Two clicks are needed." };
    (de autosave_note) => { "Einstellungen und Verlauf werden automatisch gesichert." };
    (en autosave_note) => { "Settings and history are saved automatically." };
    (de horizon_hint_chart) => { "das Diagramm zeigt diesen Zeitraum" };
    (en horizon_hint_chart) => { "the chart shows this period" };
    (de samples_capacity_fmt) => { "rund {n} Messwerte pro Ziel ({mib} MiB) bei {interval} Abstand" };
    (en samples_capacity_fmt) => { "about {n} samples per target ({mib} MiB) at an interval of {interval}" };

    // ---- settings view: suggestions and about ----
    (de suggestions) => { "Vorschläge:" };
    (en suggestions) => { "Suggestions:" };
    (de tip_add_suggestion_fmt) => { "{host} hinzufügen und sofort messen" };
    (en tip_add_suggestion_fmt) => { "Add {host} and start measuring at once" };
    (de add_btn) => { "Hinzufügen" };
    (en add_btn) => { "Add" };
    (de hint_settings_address) => { "Adresse, z. B. 8.8.8.8" };
    (en hint_settings_address) => { "Address, e.g. 8.8.8.8" };
    (de hint_settings_label) => { "Name (leer = Adresse)" };
    (en hint_settings_label) => { "Name (empty = address)" };
    (de about_body) => { "Die Prüfungen laufen über das System-Werkzeug \"ping\" in einem eigenen\n\
                        Arbeiterpool, damit die Oberfläche nie blockiert. Der Verlauf wird lokal\n\
                        gehalten und alle 30 s in den Zustandsordner geschrieben; CSV-Export und\n\
                        Protokolldatei sind optional." };
    (en about_body) => { "Probes run through the system's own \"ping\" tool in a worker pool, so the\n\
                        interface never blocks. The history is kept locally and written to the state\n\
                        folder every 30 s; CSV export and the protocol file are optional." };

    // ---- keyboard help table ----
    (de keys_heading) => { "Tastatur" };
    (en keys_heading) => { "Keyboard" };
    (de key_views) => { "Diagramm / Messwerte / Ereignisse / Einstellungen" };
    (en key_views) => { "Chart / Statistics / Events / Settings" };
    (de key_space) => { "Alle Ziele starten oder anhalten" };
    (en key_space) => { "Start or stop every target" };
    (de key_esc) => { "Editor schließen, sonst zurück ins Live-Bild" };
    (en key_esc) => { "Close the editor, otherwise back to the live view" };
    (de key_wheel) => { "Zoomen, der Cursor bleibt dieselbe Zeit" };
    (en key_wheel) => { "Zoom, keeping the time under the cursor" };
    (de key_dblclick) => { "Zurück zum laufenden Betrieb" };
    (en key_dblclick) => { "Back to live operation" };
    (de key_arrow) => { "Zeitfenster eine halbe Breite weit schieben" };
    (en key_arrow) => { "Move the time window by half its width" };
    (de key_list_dblclick) => { "Zum Ausschlag springen" };
    (en key_list_dblclick) => { "Jump to the spike" };
    (de key_wheel_label) => { "Mausrad im Diagramm" };
    (en key_wheel_label) => { "Mouse wheel in the chart" };
    (de key_dblclick_chart_label) => { "Doppelklick im Diagramm" };
    (en key_dblclick_chart_label) => { "Double-click in the chart" };
    (de key_dblclick_list_label) => { "Doppelklick in der Liste" };
    (en key_dblclick_list_label) => { "Double-click in the list" };
    (de key_space_label) => { "Leertaste" };
    (en key_space_label) => { "Spacebar" };

    (de uptime_fmt) => { "Laufzeit {t}" };
    (en uptime_fmt) => { "Uptime {t}" };
    (de export_short) => { "CSV" };
    (en export_short) => { "CSV" };
    (de tip_export_short) => { "Messwerte sofort in den Zustandsordner schreiben und ihn öffnen" };
    (en tip_export_short) => { "Write the measurements into the state folder right away and open it" };
    (de not_saved_yet) => { "noch nicht gesichert" };
    (en not_saved_yet) => { "not saved yet" };
    (de saved_secs_ago_fmt) => { "gesichert vor {n} s" };
    (en saved_secs_ago_fmt) => { "saved {n} s ago" };
    (de saved_min_ago_fmt) => { "zuletzt vor {n} min" };
    (en saved_min_ago_fmt) => { "last saved {n} min ago" };

    // ---- status messages ----
    (de msg_ready) => { "Bereit." };
    (en msg_ready) => { "Ready." };
    // ---- outage and recovery log lines ----
    (de log_recovery_fmt) => { "{label} ist wieder erreichbar ({ms} ms)." };
    (en log_recovery_fmt) => { "{label} answers again ({ms} ms)." };
    (de log_outage_fmt) => { "{label} antwortet nicht ({why})." };
    (en log_outage_fmt) => { "{label} does not answer ({why})." };
    (de log_why_resolve) => { "DNS schlägt fehl" };
    (en log_why_resolve) => { "DNS lookup failed" };
    (de log_why_error) => { "Ping-Fehler" };
    (en log_why_error) => { "ping reported an error" };
    (de log_why_timeout) => { "Timeout" };
    (en log_why_timeout) => { "timeout" };

    // Only screenshot mode writes this line: it seeds a demo history so a picture has something to
    // show. Real runs never produce it.
    (de log_seeded_fmt) => { "{n} Beispielwerte geladen (Prüfbild)" };
    (en log_seeded_fmt) => { "loaded {n} sample values (test picture)" };

    // ---- interval hints ----
    (de tip_probes_per_sec_fmt) => { "{n} Prüfungen pro Sekunde" };
    (en tip_probes_per_sec_fmt) => { "{n} probes per second" };
    (de tip_one_probe_every_fmt) => { "eine Prüfung alle {n}" };
    (en tip_one_probe_every_fmt) => { "one probe every {n}" };
    (de tip_per_minute_fmt) => { "{probes} — das sind etwa {per_min} Prüfungen je Ziel und Minute." };
    (en tip_per_minute_fmt) => { "{probes} — that is about {per_min} probes per target and minute." };
    (de msg_empty_host) => { "Host darf nicht leer sein — Änderung verworfen." };
    (en msg_empty_host) => { "The host must not be empty — change discarded." };
    (de msg_protocol_unwritable_fmt) => { "Protokoll-Datei nicht schreibbar: {err}" };
    (en msg_protocol_unwritable_fmt) => { "The protocol file cannot be written: {err}" };
    (de msg_no_outliers) => { "keine Auffälligkeiten in den gespeicherten Daten" };
    (en msg_no_outliers) => { "no anomalies in the stored data" };
    (de msg_measuring) => { "Messung läuft." };
    (en msg_measuring) => { "Measuring." };
    (de msg_stopped_edit) => { "Messung angehalten." };
    (en msg_stopped_edit) => { "Measurement stopped." };
    (de msg_finish_edit_first) => { "Zuerst die laufende Bearbeitung abschließen oder abbrechen." };
    (en msg_finish_edit_first) => { "Finish or cancel the edit that is open first." };
    (de msg_need_address) => { "Bitte eine Adresse eingeben (z. B. 192.168.1.1)." };
    (en msg_need_address) => { "Please enter an address (e.g. 192.168.1.1)." };
    (de msg_added_full_fmt) => { "{host} wurde hinzugefügt." };
    (en msg_added_full_fmt) => { "{host} was added." };
    (de msg_renamed_full_fmt) => { "Umbenannt zu {label}." };
    (en msg_renamed_full_fmt) => { "Renamed to {label}." };
    (de msg_removed_full_fmt) => { "{label} wurde entfernt." };
    (en msg_removed_full_fmt) => { "{label} was removed." };
    (de msg_measurements_cleared) => { "Messwerte gelöscht." };
    (en msg_measurements_cleared) => { "Measurements deleted." };
    (de msg_export_running) => { "Export läuft bereits …" };
    (en msg_export_running) => { "An export is already running …" };
    (de msg_no_samples) => { "Es gibt noch keine Messwerte zum Exportieren." };
    (en msg_no_samples) => { "There are no measurements to export yet." };
    (de msg_writing_csv_path_fmt) => { "Schreibe CSV-Datei … {path}" };
    (en msg_writing_csv_path_fmt) => { "Writing the CSV file … {path}" };
    (de msg_csv_written_fmt) => { "CSV-Export geschrieben: {n} Messwerte." };
    (en msg_csv_written_fmt) => { "CSV export written: {n} measurements." };
    (de msg_dialog_opened) => { "Dialog zum Speichern geöffnet …" };
    (en msg_dialog_opened) => { "Save dialog opened …" };
    (de msg_export_cancelled) => { "Export abgebrochen." };
    (en msg_export_cancelled) => { "Export cancelled." };
    (de msg_autostart_on_registry) => { "Autostart ist aktiv." };
    (en msg_autostart_on_registry) => { "Autostart is enabled." };
    (de msg_autostart_off_registry) => { "Autostart ist deaktiviert." };
    (en msg_autostart_off_registry) => { "Autostart is disabled." };
    (de msg_folder_opened_fmt) => { "Ordner geöffnet: {dir}" };
    (en msg_folder_opened_fmt) => { "Folder opened: {dir}" };
    (de msg_host_to_label_fmt) => { "{old} → {new}, umbenannt." };
    (en msg_host_to_label_fmt) => { "{old} → {new}, renamed." };
    (de msg_host_to_fmt) => { "{old} → {new}." };
    (en msg_host_to_fmt) => { "{old} → {new}." };
    (de msg_resumed_fmt) => { "{label} wird wieder gemessen." };
    (en msg_resumed_fmt) => { "{label} is measured again." };
    (de msg_paused_fmt) => { "{label} pausiert." };
    (en msg_paused_fmt) => { "{label} is paused." };
    (de msg_no_change) => { "Keine Änderung." };
    (en msg_no_change) => { "No change." };
    (de msg_dialog_title) => { "CSV exportieren" };
    (en msg_dialog_title) => { "Export CSV" };
    (de msg_csv_filter) => { "CSV-Datei" };
    (en msg_csv_filter) => { "CSV file" };

    // ---- dialogs ----

    // ---- log lines the interface composes ----
    (de log_language_fmt) => { "Sprache auf {lang} umgestellt" };
    (en log_language_fmt) => { "Interface language set to {lang}" };

    // ---- units ----
    (de seconds_unit) => { "s" };
    (en seconds_unit) => { "s" };
    (de hours_unit) => { "h" };
    (en hours_unit) => { "h" };
    (de minutes_unit) => { "min" };
    (en minutes_unit) => { "min" };

    // ---- window title and tooltip ----
    (de app_title) => { "System-Ping — Netzwerk-Latenz beobachten" };
    (en app_title) => { "System-Ping — watch the network latency" };

    (de active) => { "aktiv" };
    (en active) => { "active" };
    (de ago_fmt) => { "vor {n} s" };
    (en ago_fmt) => { "{n} s ago" };
    (de tip_discard_edit) => { "Änderungen verwerfen (Esc)" };
    (en tip_discard_edit) => { "Discard the changes (Esc)" };
    (de hint_address) => { "Adresse" };
    (en hint_address) => { "Address" };
    (de measures_checkbox) => { "misst" };
    (en measures_checkbox) => { "measures" };
    (de tip_measures) => { "Aus schaltet die Messung ab, der Verlauf bleibt erhalten" };
    (en tip_measures) => { "Switching this off stops the measuring, the history stays" };
    (de tip_color_auto_btn) => { "Farbe wieder automatisch nach der Position in der Liste" };
    (en tip_color_auto_btn) => { "Pick the colour automatically again, by position in the list" };
    (de remove_short) => { "entfernen" };
    (en remove_short) => { "remove" };
    (de edit_heading) => { "Bearbeiten" };
    (en edit_heading) => { "Edit" };
    (de save_btn) => { "✓ Speichern" };
    (en save_btn) => { "✓ Save" };
    (de cancel_btn) => { "✗ Abbrechen" };
    (en cancel_btn) => { "✗ Cancel" };
    (de values_span_fmt) => { "{replies}/{total} Werte · {avg}" };
    (en values_span_fmt) => { "{replies}/{total} values · {avg}" };
    (de tip_remove_fmt) => { "{label} entfernen" };
    (en tip_remove_fmt) => { "Remove {label}" };

    // ---- fallback for an unknown name (never used; keeps the macro forgiving) ----
    (de _) => { "" };
    (en _) => { "" };
}

/// Generate one method per text, each answering with the arm of the current language.
macro_rules! accessors {
    ($($name:ident),* $(,)?) => {
        $(
            #[must_use]
            pub fn $name(&self) -> &'static str {
                match self {
                    Lang::De => texts!(de $name),
                    Lang::En => texts!(en $name),
                }
            }
        )*
    };
}

impl Lang {
    accessors![
        subtitle,
        tip_horizon,
        tip_follow_live,
        data_folder_btn,
        tip_data_folder,
        autostart_on,
        autostart_off,
        tip_autostart_top,
        stop_all,
        start_all,
        hint_new_host,
        hint_new_label,
        add_target_short,
        hint_filter,
        all,
        filter_off_fmt,
        tip_kind_show_fmt,
        tip_kind_hide_fmt,
        tip_all_kinds,
        targets,
        paused,
        waiting,
        not_probed_yet,
        tip_color_auto,
        tip_color_chosen_fmt,
        auto_color,
        loss_count_fmt,
        status_list,
        tip_show_target,
        hidden_suffix,
        filtered_suffix,
        waiting_short,
        stats_line_fmt,
        outliers_suffix_fmt,
        running,
        stopped,
        probes_fmt,
        losses_fmt,
        back_btn,
        next_btn,
        earlier_btn,
        tip_next_outlier,
        tip_earlier,
        forward_btn,
        chart_heading,
        tip_legend_hidden_fmt,
        tip_legend_shown_fmt,
        why_hidden,
        why_filter,
        marker_outlier,
        all_kinds_off,
        all_targets_hidden,
        filter_no_match,
        events,
        only_problems,
        tip_only_problems,
        clear_log_btn,
        log_count_fmt,
        log_count_paren_fmt,
        events_count_fmt,
        events_count_short_fmt,
        tip_clear_log,
        col_target,
        col_host,
        col_status,
        col_min,
        col_avg,
        col_median,
        col_p95,
        col_max,
        col_loss,
        col_outliers,
        stats_heading,
        tip_sort_fmt,
        language,
        tip_language_fmt,
        total_probes_fmt,
        measuring,
        start_group,
        data_group,
        appearance_group,
        info_group,
        theme_scheme_label,
        dark,
        light,
        system_theme,
        tip_system_theme,
        interval_now_fmt,
        timeout_label,
        dash,
        concurrency_label,
        max_samples_label,
        horizon_label_text,
        tip_max_samples_fmt,
        protocol_label,
        tip_protocol_log,
        hint_protocol_path,
        autostart_checkbox,
        autostart_blocked,
        autostart_unsupported,
        export_csv,
        tip_export_default,
        save_as,
        tip_save_as,
        clear_measurements_q,
        clear_measurements,
        tip_clear_again,
        tip_clear_measurements,
        autosave_note,
        active,
        ago_fmt,
        tip_discard_edit,
        hint_address,
        measures_checkbox,
        tip_measures,
        tip_color_auto_btn,
        remove_short,
        edit_heading,
        save_btn,
        cancel_btn,
        values_span_fmt,
        tip_remove_fmt,
        horizon_hint_chart,
        samples_capacity_fmt,
        suggestions,
        tip_add_suggestion_fmt,
        add_btn,
        hint_settings_address,
        hint_settings_label,
        about_body,
        keys_heading,
        key_views,
        key_space,
        key_esc,
        key_wheel,
        key_dblclick,
        key_arrow,
        key_list_dblclick,
        key_wheel_label,
        key_dblclick_chart_label,
        key_dblclick_list_label,
        key_space_label,
        msg_ready,
        log_recovery_fmt,
        log_outage_fmt,
        log_why_resolve,
        log_why_error,
        log_why_timeout,
        log_seeded_fmt,
        tip_probes_per_sec_fmt,
        tip_one_probe_every_fmt,
        tip_per_minute_fmt,
        msg_empty_host,
        msg_protocol_unwritable_fmt,
        msg_no_outliers,
        msg_measuring,
        msg_stopped_edit,
        msg_finish_edit_first,
        msg_need_address,
        msg_added_full_fmt,
        msg_renamed_full_fmt,
        msg_removed_full_fmt,
        msg_measurements_cleared,
        msg_export_running,
        msg_no_samples,
        msg_writing_csv_path_fmt,
        msg_csv_written_fmt,
        msg_dialog_opened,
        msg_export_cancelled,
        msg_autostart_on_registry,
        msg_autostart_off_registry,
        msg_folder_opened_fmt,
        msg_host_to_label_fmt,
        msg_host_to_fmt,
        msg_resumed_fmt,
        msg_paused_fmt,
        msg_no_change,
        msg_dialog_title,
        msg_csv_filter,
        uptime_fmt,
        export_short,
        tip_export_short,
        not_saved_yet,
        saved_secs_ago_fmt,
        saved_min_ago_fmt,
        log_language_fmt,
        seconds_unit,
        hours_unit,
        minutes_unit,
        ms_suffix,
        timeout_word,
        app_title,
    ];
}

/// Fill `{name}` placeholders of a template from the table. Values are given as pairs; every name
/// has to be one the template actually uses, so a typo shows up as an unused value rather than as a
/// literal `{foo}` in the window.
pub fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The texts a visible control can show. A text that is empty in one language would silently
    /// render as a blank button, so this test is the guard for both columns of the table.
    const CONTROLS: [fn(&Lang) -> &'static str; 23] = [
        Lang::subtitle,
        Lang::data_folder_btn,
        Lang::stop_all,
        Lang::start_all,
        Lang::add_target_short,
        Lang::all,
        Lang::targets,
        Lang::paused,
        Lang::waiting,
        Lang::status_list,
        Lang::running,
        Lang::stopped,
        Lang::back_btn,
        Lang::next_btn,
        Lang::earlier_btn,
        Lang::only_problems,
        Lang::col_target,
        Lang::col_status,
        Lang::measuring,
        Lang::language,
        Lang::export_csv,
        Lang::add_btn,
        Lang::app_title,
    ];

    #[test]
    fn both_languages_answer_every_control_text() {
        for f in CONTROLS {
            assert!(!f(&Lang::De).trim().is_empty(), "German text missing");
            assert!(!f(&Lang::En).trim().is_empty(), "English text missing");
        }
    }

    #[test]
    fn language_code_roundtrips() {
        assert_eq!(Lang::from_code("en").code(), "en");
        assert_eq!(Lang::from_code("DE").code(), "de");
        // Unknown values must not break an existing state file.
        assert_eq!(Lang::from_code(""), Lang::De);
        assert_eq!(Lang::from_code("fr"), Lang::De);
    }

    #[test]
    fn fill_replaces_every_placeholder() {
        let s = fill("{a} und {b}", &[("a", "1".into()), ("b", "2".into())]);
        assert_eq!(s, "1 und 2");
    }

    #[test]
    fn views_and_kinds_answer_both_languages() {
        use crate::app::{TargetKind, View};
        for v in [View::Chart, View::Events, View::Stats, View::Settings] {
            for l in Lang::ALL {
                assert!(!l.view(v).is_empty());
            }
        }
        for k in TargetKind::all() {
            for l in Lang::ALL {
                assert!(!l.kind(k).is_empty());
            }
        }
    }
}
