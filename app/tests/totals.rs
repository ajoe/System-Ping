//! Die Gesamtzeile der Statistik: Median und p95 aus allen Antworten zusammen, nicht als Mittel
//! der Mediane. Ausserdem die Helfer, die dahinterstecken (`App::stats_total`, `history::pick`).

use std::sync::Mutex;

use system_ping::app::{App, AppOptions};

/// Der Zustandsordner wird über eine Prozessweite Variable gewählt (`SYSTEM_PING_STATE_DIR`), und
/// ein App liest beim Erzeugen die Datei daraus. Zwei Tests in diesem Binary würden sich also in
/// die Quere kommen — einer sähe die Ziele des anderen. Diese Kette hält den Ordner zusammen mit
/// dem dazugehörigen App, damit klar ist, wer ihn gerade besitzt.
static STATE_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Ein App mit eigenem, leerem Zustandsordner: kein echter Nutzer bekommt etwas davon ab, und der
/// Test beginnt garantiert ohne gespeicherte Ziele. Die Sperre bleibt bis zum Ende des Tests
/// gehalten (`_guard` muss deshalb das letzte Feld sein).
struct Fixture {
    app: App,
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::env::var_os("SYSTEM_PING_STATE_DIR").as_deref() == Some(self.dir.as_os_str()) {
            std::env::remove_var("SYSTEM_PING_STATE_DIR");
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Ein App mit eigenem, leerem Ordner. Die drei Standardziele werden durchgegeben, aber nicht als
/// gespeicherte Liste: sonst läse das App beim Erzeugen eine Datei, die ein früherer Test in
/// demselben Ordner hinterlassen hat.
fn fixture(tag: &str) -> Fixture {
    let guard = STATE_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("sp-totals-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SYSTEM_PING_STATE_DIR", &dir);
    Fixture {
        app: App::with_theme_override(AppOptions::default(), Vec::new(), Some("dark")),
        dir,
        _guard: guard,
    }
}

/// Zwei Ziele mit sehr verschiedenen Latenzen, umgeschrieben statt neu erzeugt: `Target::new` ist
/// absichtlich nicht nach außen offen. Ohne frische Serien läge noch der Inhalt eines früheren
/// Durchlaufs in denselben Objekten.
fn two_targets(app: &mut App) {
    assert!(app.targets.len() >= 2, "der Test braucht zwei Ziele");
    for (i, (label, host)) in [("Schnell", "10.0.0.1"), ("Langsam", "10.0.0.2")]
        .into_iter()
        .enumerate()
    {
        let t = &mut app.targets[i];
        t.label = label.into();
        t.host = host.into();
        t.kind = system_ping::app::TargetKind::Lan;
        t.hidden = false;
        t.enabled = true;
        t.series = system_ping::history::Series::new(1024);
    }
    app.targets.truncate(2);
}

/// `n` Antworten im Sekundentakt, deren letzte auf `t_secs` endet. Die Zeitstempel müssen ins
/// Zeitfenster fallen — bei 40 Antworten und dem Standardfenster von 60 s wären die ältesten sonst
/// schon wieder draußen, und der Test hätte gegen eine kürzere Liste gerechnet als er behauptet.
fn record(app: &mut App, idx: usize, last_t_secs: f64, n: i64, ms: f32) {
    for k in 0..n {
        let t = last_t_secs - (n - 1 - k) as f64;
        app.targets[idx]
            .series
            .record(t, (t * 1000.0) as i64, ms, None);
    }
}

/// Die Gesamtzeile über alle Ziele, die gerade in der Auswahl sind.
fn total(app: &App) -> system_ping::history::Stats {
    let mut s = Vec::new();
    let mut a = Vec::new();
    app.stats_total(&mut s, &mut a)
}

/// Der Median *der Mediane* wäre ~255 ms — eine Zahl, die kein einziges Messergebnis hat. Richtig
/// ist ~10 ms, weil das schnelle Ziel deutlich mehr Antworten beisteuert.
#[test]
fn median_und_p95_kommen_aus_allen_antworten() {
    let mut f = fixture("medians");
    two_targets(&mut f.app);
    assert!(
        f.app.shown(&f.app.targets[0]) && f.app.shown(&f.app.targets[1]),
        "beide Ziele müssen in der Auswahl sein, sonst stimmt die Summe nicht"
    );

    // 44 Antworten zu 10 ms und eine zu 500 ms. Über alle Antworten zusammen sind Median wie p95
    // (Rang 42 von 45) noch beim schnellen Ziel; der Mittelwert *der beiden* Mediane wäre 255 ms.
    record(&mut f.app, 0, 400.0, 44, 10.0);
    record(&mut f.app, 1, 400.0, 1, 500.0);

    let mut scratch = Vec::new();
    let mut all = Vec::new();
    let st = f.app.stats_total(&mut scratch, &mut all);

    assert_eq!(st.total, 45);
    assert_eq!(
        st.replies, 45,
        "alle vierundvierzig plus eine Antwort zählen"
    );
    assert!(
        st.median < 20.0,
        "Median kommt aus allen Antworten, nicht als Mittel der Mediane: {}",
        st.median
    );
    assert!(
        st.p95 < 100.0,
        "p95 über alle Antworten: {}, erwartet noch beim schnellen Ziel (Median {}, min {}, max {})",
        st.p95,
        st.median,
        st.min,
        st.max
    );
    assert!(st.min < st.max);
    assert_eq!(st.max, 500.0);

    // Ein Gegenbeweis, damit der Test nicht durch Zufall besteht: dieselben zwei Ziele, aber das
    // langsame stellt jetzt die Mehrheit — dann müssen Median und p95 dort landen. Die Serie wird
    // neu gesetzt, sonst stünden die 44 schnellen Antworten von oben noch darin.
    f.app.targets[0].series = system_ping::history::Series::new(1024);
    record(&mut f.app, 0, 900.0, 5, 10.0);
    record(&mut f.app, 1, 900.0, 40, 500.0);
    let inv = total(&f.app);
    assert_eq!(inv.replies, 45, "wieder genau die zwei Ziele: {inv:?}");
    assert!(
        inv.p95 > 400.0,
        "mit dem langsamen Ziel als Mehrheit muss p95 dort landen: {}",
        inv.p95
    );
    assert!(inv.median > 400.0);
}

/// Ein ausgeblendetes Ziel fehlt auch in der Gesamtzeile — sonst zählt sie Zeilen zusammen, die
/// niemand sieht. Und ein Ziel ohne Antwort zieht das Minimum aller Ziele nicht auf 0.
#[test]
fn ausgeblendete_ziele_zaehlen_nicht_mit() {
    let mut f = fixture("hidden-total");
    two_targets(&mut f.app);
    record(&mut f.app, 0, 400.0, 6, 10.0);
    record(&mut f.app, 1, 400.0, 6, 20.0);
    assert_eq!(total(&f.app).total, 12, "beide Ziele zusammen zählen");

    f.app.targets[0].hidden = true;
    let after = total(&f.app);
    assert_eq!(after.total, 6, "das ausgeblendete Ziel fehlt");
    assert_eq!(after.min, 20.0, "und seine Latenzen fehlen ebenfalls");
}

/// Ein Ausfall hat keine Latenz; er darf `min` nicht auf 0 ziehen, geht aber in den Verlust ein.
#[test]
fn ein_ausfall_senkt_das_minimum_nicht() {
    let mut f = fixture("loss-min");
    two_targets(&mut f.app);
    record(&mut f.app, 0, 400.0, 4, 30.0);
    for k in 0..4i64 {
        let t = 400.0 - (3 - k) as f64;
        f.app.targets[1].series.record(
            t,
            (t * 1000.0) as i64,
            f32::NAN,
            Some(system_ping::history::Fail::Timeout),
        );
    }
    let st = total(&f.app);
    assert_eq!(
        st.min, 30.0,
        "ein Ausfall ohne Latenz darf min nicht senken"
    );
    assert!(st.loss_pct > 0.0, "verloren geht er trotzdem nicht");
}

/// „Messwerte löschen“ leert die Serien, lässt die Ziele aber stehen. Die Gesamtzeile darf dann
/// keine Null vortäuschen: ohne jede Antwort gibt es kein Minimum, und die Oberfläche macht aus dem
/// Randwert „—“.
#[test]
fn ohne_messwerte_ist_nichts_gemessen() {
    let mut f = fixture("empty-total");
    two_targets(&mut f.app);
    record(&mut f.app, 0, 400.0, 6, 12.0);
    f.app.reset_stats();
    assert!(
        !f.app.targets.is_empty(),
        "die Ziele selbst bleiben erhalten"
    );

    let st = total(&f.app);
    assert_eq!(st.total, 0);
    assert_eq!(st.replies, 0);
    assert_eq!(st.median, 0.0);
    assert_eq!(st.p95, 0.0);
    assert_eq!(st.loss_pct, 0.0);
    assert!(
        st.min.is_infinite() && st.min > 0.0,
        "ohne Antwort ist min der Randwert, nicht 0: {}",
        st.min
    );
}
