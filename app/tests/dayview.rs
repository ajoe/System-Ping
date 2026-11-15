// Prüft das 24-h-Diagramm: Die Voreinstellungen reichen bis einen Tag, und die Zahl der
// Messwerte, die ein Ziel behält, wächst automatisch mit dem Fenster. Ohne diesen zweiten
// Teil wäre ein Tag nur ein leeres Diagramm — bei einer festen Grenze von 2048 Wären bei
// einer Sekunde Abstand nach einer halben Stunde alle älteren Werte weg.

use system_ping::app::{
    effective_max_samples, samples_for_horizon, App, AppOptions, HorizonChoice, HORIZON_CHOICES,
    MIN_SAMPLES,
};

/// Das längste Fenster ist ein Tag, das kürzeste 15 s, und dazwischen liegen die Stufen,
/// die man erwartet. Auf einen Tag zu kommen ist der eigentliche Wunsch dahinter.
#[test]
fn the_presets_reach_a_day() {
    let secs: Vec<f32> = HORIZON_CHOICES.iter().map(|c| c.secs).collect();
    assert_eq!(secs.first().copied(), Some(15.0));
    assert_eq!(
        secs.last().copied(),
        Some(86_400.0),
        "längstes Fenster ist 24 h"
    );
    assert!(
        secs.windows(2).all(|w| w[0] < w[1]),
        "sortiert und ohne Wiederholung: {secs:?}"
    );
    for want in [60.0, 3_600.0, 86_400.0] {
        assert!(secs.contains(&want), "{want} s fehlt");
    }

    // Ein Tag muss auch einen deutschen Namen haben, sonst zeigt das Auswahlfeld eine Zahl.
    let day: &HorizonChoice = HORIZON_CHOICES.last().unwrap();
    assert_eq!(day.label, "24 h");
}

/// Die automatische Grenze richtet sich nach Fenster und Abstand: ein Tag bei einer Sekunde
/// sind 86 400 Prüfungen, und weil Rückblättern drei Fenster braucht, kommt Luft dazu.
#[test]
fn the_sample_budget_follows_the_window() {
    // Der Standard: kurzes Fenster, also die Mindestgrenze.
    assert_eq!(samples_for_horizon(60.0, 1.0), MIN_SAMPLES);

    let day = samples_for_horizon(86_400.0, 1.0);
    assert!(
        day >= 86_400,
        "ein Tag bei 1 s braucht mindestens 86 400 Werte, geplant sind {day}"
    );

    // Größerer Abstand spart Speicher: alle zwei Sekunden etwa die Hälfte.
    let half = samples_for_horizon(86_400.0, 2.0);
    assert!(
        (half as f64 - day as f64 / 2.0).abs() < 1_000.0,
        "bei 2 s Abstand die Hälfte: {day} -> {half}"
    );

    // Eine Obergrenze muss bleiben — sonst schreibt ein Tipp in den Einstellungen das Konto voll.
    assert_eq!(samples_for_horizon(86_400.0, 0.25), 1_000_000);
}

/// `max_samples == 0` ist der neue Standard und heißt „automatisch“. Wer eine Zahl einträgt,
/// bekommt sie; zu kleine Zahlen werden auf die Mindestgrenze gehoben, damit das Fenster nicht
/// hinter seinem eigenen Speicher zurückbleibt.
#[test]
fn zero_means_auto_and_a_number_wins() {
    let mut o = AppOptions::default();
    assert_eq!(o.max_samples, 0, "Standard ist automatisch");
    assert_eq!(effective_max_samples(&o), samples_for_horizon(60.0, 1.0));

    // Tag und ein Sekundentakt: dieselbe Zahl, die das Diagramm braucht.
    o.horizon_secs = 86_400.0;
    let auto = effective_max_samples(&o);
    assert_eq!(auto, samples_for_horizon(86_400.0, 1.0));
    assert!(auto > 2_048, "ein Tag passt nicht mehr in die alte Grenze");

    o.max_samples = 5_000;
    assert_eq!(effective_max_samples(&o), 5_000);
    o.max_samples = 7;
    assert_eq!(
        effective_max_samples(&o),
        MIN_SAMPLES,
        "zu klein wird angehoben"
    );
}

/// Der eigentliche End-zu-End-Beweis: Ein Ziel, das ein Tag lang bei schnellem Abstand
/// gemessen hat, darf durch die automatische Grenze nichts verlieren.
#[test]
fn a_day_of_history_survives_the_auto_budget() {
    std::env::set_var(
        "SYSTEM_PING_STATE_DIR",
        std::env::temp_dir().join("sp-dayview"),
    );
    let o = AppOptions {
        horizon_secs: 86_400.0,
        interval_secs: 1.0,
        ..AppOptions::default()
    };
    let mut app = App::with_theme_override(o, Vec::new(), None);
    app.targets.clear();
    app.new_host = "127.0.0.1".to_string();
    app.new_label = "Test".to_string();
    app.add_target();

    // Ein Tag voller Messwerte im Abstand von 10 s: 86 400 s sind genau ein Tag, deshalb eine
    // Spanne knapp darunter. Die Serie ist ein Ringpuffer — zu kleine Speichergrenze würde die
    // älteste Hälfte stillschweigend verschwinden lassen, und genau das war der alte Wert 2048.
    let n = 8_500usize;
    let t = &mut app.targets[0];
    for k in 0..n {
        t.series.record(
            (k as f64) * 10.0,
            1_700_000_000_000 + (k as i64) * 10_000,
            12.0 + (k as f32 % 5.0),
            None,
        );
    }
    assert_eq!(t.series.cap(), effective_max_samples(&app.options));
    assert_eq!(
        t.series.len(),
        n,
        "kein Wert verloren: Kapazität {} bei {n} Werten",
        t.series.cap()
    );

    // Und das Diagramm zeigt sie alle: Die Spanne von 89 990 s passt in ein Tagesfenster, und
    // der Puffer trägt mindestens das Doppelte — sonst reichte er nicht für die Rückblättertaste.
    let horizon = app.options.horizon_secs as f64;
    let span = t.series.last().unwrap().t_secs - t.series.samples()[0].t_secs;
    assert!(
        span <= horizon,
        "ein Tag Diagramm mit {span:.0} s Verlauf gefüllt"
    );
    assert!(
        (t.series.cap() as f64) >= 2.0 * n as f64,
        "der Puffer trägt auch das Doppelte für das Rückblättern: {} gegen {n}",
        t.series.cap()
    );
}

/// Das Diagramm braucht zu einem Tag auch passende Achsen: Ein Schritt von 900 s würde
/// sechzig Gitterlinien übereinander drucken, und „Minuten:Sekunden“ läuft über einhundert
/// Minuten davon.
#[test]
fn a_day_gets_a_sensible_grid() {
    use system_ping::ui::{fmt_axis, nice_step};

    // Rasterweite: kurze Fenster bleiben im Sekunden-Takt, ein Tag geht in Stunden.
    assert_eq!(nice_step(60.0), 5.0);
    assert_eq!(nice_step(3_600.0), 300.0);
    assert_eq!(nice_step(86_400.0), 7_200.0, "zwölf Linien pro Tag");
    // Nie mehr als zwölf Linien — sonst steht die Achse voll.
    for h in [15.0f64, 30.0, 900.0, 7_200.0, 43_200.0, 86_400.0] {
        assert!(h / nice_step(h) <= 12.0, "{h} s: Schritt {}", nice_step(h));
    }

    // Kurz: Minuten und Sekunden. Das war vorher und bleibt so.
    assert_eq!(fmt_axis(0.0, 60.0, None), "00:00");
    assert_eq!(fmt_axis(95.0, 600.0, Some(0)), "01:35");

    // Ab zwei Stunden kommt die Uhrzeit — ein Tag in Minuten gezählt wäre unlesbar.
    // Der Stempel ist 1 h nach Mitternacht UTC; ob die Anzeige dazu lokal 01:00 oder eine
    // andere Stunde sagt, steht hier nicht fest, aber sie muss HH:MM sein.
    let tag = 86_400.0;
    let mitternacht_utc = 0i64;
    let erste_stunde = fmt_axis(3_600.0, tag, Some(mitternacht_utc + 3_600_000));
    assert_eq!(
        erste_stunde.len(),
        5,
        "HH:MM erwartet, kam {erste_stunde:?}"
    );
    assert!(
        erste_stunde.chars().nth(2) == Some(':'),
        "Uhrzeit-Format erwartet, kam {erste_stunde:?}"
    );

    // Ohne echten Stempel (eine Sitzung, die noch nichts gemessen hat): Stunden statt Minuten.
    assert_eq!(fmt_axis(5_400.0, tag, None), "01:30 h");
    assert_eq!(fmt_axis(86_400.0, tag, None), "24:00 h");
}

/// Die Zeitfenster-Voreinstellungen müssen zu den Achsen passen: Zu jedem Fenster gehört ein
/// Raster, das man zählen kann, und jedes Fenster ab zwei Stunden wird nach der Uhr benannt.
#[test]
fn every_preset_window_labels_its_axis() {
    use system_ping::ui::{fmt_axis, nice_step};

    for c in HORIZON_CHOICES {
        let h = f64::from(c.secs);
        assert!(h / nice_step(h) <= 12.0, "{}: zu viele Linien", c.label);
        let uhr = fmt_axis(h, h, Some(1_700_000_000_000));
        if h >= 2.0 * 3_600.0 {
            assert_eq!(uhr.len(), 5, "{}: keine Uhrzeit ({uhr:?})", c.label);
        } else {
            assert_eq!(uhr.len(), 5, "{}: kein MM:SS ({uhr:?})", c.label);
        }
    }
}
