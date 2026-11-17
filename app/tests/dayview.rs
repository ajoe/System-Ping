// Checks the 24 h chart: the presets reach a full day, and the number of measurements a target
// keeps grows with the window automatically. Without that second part a day would just be an
// empty chart — with a fixed cap of 2048 and one-second spacing, everything older than half an
// hour would be gone.

use system_ping::app::{
    effective_max_samples, samples_for_horizon, App, AppOptions, HorizonChoice, HORIZON_CHOICES,
    MIN_SAMPLES,
};

/// The longest window is a day, the shortest 15 s, and in between are the steps one expects.
/// Reaching a full day is the actual wish behind this.
#[test]
fn the_presets_reach_a_day() {
    let secs: Vec<f32> = HORIZON_CHOICES.iter().map(|c| c.secs).collect();
    assert_eq!(secs.first().copied(), Some(15.0));
    assert_eq!(
        secs.last().copied(),
        Some(86_400.0),
        "longest window is 24 h"
    );
    assert!(
        secs.windows(2).all(|w| w[0] < w[1]),
        "sorted and without repetition: {secs:?}"
    );
    for want in [60.0, 3_600.0, 86_400.0] {
        assert!(secs.contains(&want), "{want} s is missing");
    }

    // A day also needs a label of its own, otherwise the dropdown would show a bare number.
    let day: &HorizonChoice = HORIZON_CHOICES.last().unwrap();
    assert_eq!(day.label, "24 h");
}

/// The automatic cap follows window and interval: a day at one second is 86,400 probes, and since
/// paging back needs three windows, there is some headroom on top.
#[test]
fn the_sample_budget_follows_the_window() {
    // The default: a short window, so the minimum cap.
    assert_eq!(samples_for_horizon(60.0, 1.0), MIN_SAMPLES);

    let day = samples_for_horizon(86_400.0, 1.0);
    assert!(
        day >= 86_400,
        "a day at 1 s needs at least 86,400 values, planned are {day}"
    );

    // A larger interval saves memory: every two seconds is roughly half.
    let half = samples_for_horizon(86_400.0, 2.0);
    assert!(
        (half as f64 - day as f64 / 2.0).abs() < 1_000.0,
        "half the values at a 2 s interval: {day} -> {half}"
    );

    // There has to be an upper bound — otherwise a typo in the settings would fill up memory.
    assert_eq!(samples_for_horizon(86_400.0, 0.25), 1_000_000);
}

/// `max_samples == 0` is the default and means "automatic". Anyone who enters a number gets it;
/// numbers that are too small are raised to the minimum cap so the window never falls behind its
/// own memory.
#[test]
fn zero_means_auto_and_a_number_wins() {
    let mut o = AppOptions::default();
    assert_eq!(o.max_samples, 0, "the default is automatic");
    assert_eq!(effective_max_samples(&o), samples_for_horizon(60.0, 1.0));

    // A day and one-second spacing: the very number the chart needs.
    o.horizon_secs = 86_400.0;
    let auto = effective_max_samples(&o);
    assert_eq!(auto, samples_for_horizon(86_400.0, 1.0));
    assert!(auto > 2_048, "a day no longer fits the old cap");

    o.max_samples = 5_000;
    assert_eq!(effective_max_samples(&o), 5_000);
    o.max_samples = 7;
    assert_eq!(
        effective_max_samples(&o),
        MIN_SAMPLES,
        "too small is raised"
    );
}

/// The actual end-to-end proof: a target measured for a day at a fast interval must not lose
/// anything through the automatic cap.
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

    // A day full of measurements at a 10 s spacing: 86,400 s is exactly one day, hence a span a
    // little below it. The series is a ring buffer — a cap that is too small would silently drop
    // the oldest half, and that was exactly what the old value of 2048 did.
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
        "no value lost: capacity {} for {n} values",
        t.series.cap()
    );

    // And the chart shows all of them: the span of 89,990 s fits a day window, and the buffer
    // holds at least twice as much — otherwise it would not be enough for the page-back button.
    let horizon = app.options.horizon_secs as f64;
    let span = t.series.last().unwrap().t_secs - t.series.samples()[0].t_secs;
    assert!(
        span <= horizon,
        "a day of chart filled with {span:.0} s of history"
    );
    assert!(
        (t.series.cap() as f64) >= 2.0 * n as f64,
        "the buffer also holds twice as much for paging back: {} against {n}",
        t.series.cap()
    );
}

/// A day also needs matching axes: a step of 900 s would print sixty gridlines on top of each
/// other, and "minutes:seconds" runs past a hundred minutes of them.
#[test]
fn a_day_gets_a_sensible_grid() {
    use system_ping::ui::{fmt_axis, nice_step};

    // Grid spacing: short windows stay on seconds, a day goes in hours.
    assert_eq!(nice_step(60.0), 5.0);
    assert_eq!(nice_step(3_600.0), 300.0);
    assert_eq!(nice_step(86_400.0), 7_200.0, "twelve lines per day");
    // Never more than twelve lines — otherwise the axis is packed solid.
    for h in [15.0f64, 30.0, 900.0, 7_200.0, 43_200.0, 86_400.0] {
        assert!(h / nice_step(h) <= 12.0, "{h} s: step {}", nice_step(h));
    }

    // Short: minutes and seconds. That is how it was before and how it stays.
    assert_eq!(fmt_axis(0.0, 60.0, None), "00:00");
    assert_eq!(fmt_axis(95.0, 600.0, Some(0)), "01:35");

    // Past two hours the clock time appears — a day counted in minutes would be unreadable.
    // The stamp is 1 h after midnight UTC; whether the display says 01:00 locally or another
    // hour is not fixed here, but it has to be HH:MM.
    let tag = 86_400.0;
    let midnight_utc = 0i64;
    let first_hour = fmt_axis(3_600.0, tag, Some(midnight_utc + 3_600_000));
    assert_eq!(first_hour.len(), 5, "HH:MM expected, got {first_hour:?}");
    assert!(
        first_hour.chars().nth(2) == Some(':'),
        "clock format expected, got {first_hour:?}"
    );

    // Without a real stamp (a session that has not measured anything yet): hours, not minutes.
    assert_eq!(fmt_axis(5_400.0, tag, None), "01:30 h");
    assert_eq!(fmt_axis(86_400.0, tag, None), "24:00 h");
}

/// A window can reach further back than the session has been running — right after a start with a
/// one-minute window, or at any time with a day. Those ticks are positions *before* the session
/// began, and clamping them to zero made the whole axis read „00:00": a clock that looks stopped.
#[test]
fn an_axis_reaching_past_the_start_reads_as_such() {
    use system_ping::ui::fmt_axis;

    assert_eq!(fmt_axis(-25.0, 60.0, None), "-00:25");
    assert_eq!(fmt_axis(-95.0, 600.0, None), "-01:35");
    assert_eq!(fmt_axis(-0.5, 60.0, None), "00:00", "not -00:00");
    assert_eq!(fmt_axis(-5_400.0, 86_400.0, None), "-01:30 h");
    // Nothing on the axis may repeat: a chart whose every label says the same thing is not a chart.
    let labels: Vec<String> = (0..12)
        .map(|i| fmt_axis(-300.0 + i as f64 * 30.0, 600.0, None))
        .collect();
    let distinct: std::collections::HashSet<&String> = labels.iter().collect();
    assert_eq!(distinct.len(), labels.len(), "{labels:?}");
}

/// A narrow plot cannot show the twelve lines a day is drawn with in a wide window: their labels
/// land on one another. The ladder is the same, only climbed further.
#[test]
fn a_narrow_window_counts_a_day_in_longer_strides() {
    use system_ping::ui::{nice_step, step_for_width};

    // Enough room: the plain twelve lines per day.
    assert_eq!(
        step_for_width(86_400.0, 86_400.0, 1_200.0),
        nice_step(86_400.0)
    );
    // The narrowest window the app allows (its central panel is what is left of 640 px): the step
    // has to grow until the labels fit.
    let narrow = step_for_width(86_400.0, 86_400.0, 370.0);
    assert!(narrow > nice_step(86_400.0), "step stayed at {narrow}");
    assert!(
        (86_400.0 / narrow) * 56.0 <= 370.0,
        "{narrow} s still does not fit 370 px"
    );
    // A short window in a wide plot keeps the plain seconds spacing: nothing has to be squeezed.
    assert_eq!(step_for_width(60.0, 60.0, 1_200.0), 5.0);
    // And in the narrow one even a short window is counted in coarser strides — thirteen „00:05"
    // labels do not fit 370 px either.
    let short_narrow = step_for_width(60.0, 60.0, 370.0);
    assert!(short_narrow > 5.0, "step stayed at {short_narrow}");
    assert!(
        (60.0 / short_narrow) * 56.0 <= 370.0,
        "{short_narrow} s still does not fit 370 px"
    );
}

/// The horizontal lines used to be four fixed numbers of milliseconds. On a chart that runs into the
/// thousands they all land within a finger's width of the bottom, labels on top of one another, so
/// the spacing has to follow the scale.
#[test]
fn the_millisecond_axis_follows_its_scale() {
    use system_ping::ui::ms_band_step;

    // A LAN chart, tall: fine round steps, and the familiar 30 and 80 reappear among them.
    let lan = ms_band_step(100.0, 440.0);
    assert!(
        lan <= 30.0,
        "a tall chart may not go coarser than 30 ms ({lan})"
    );
    // Hundreds of milliseconds, short plot: coarse enough that the numbers keep their distance.
    let slow = ms_band_step(1_400.0, 160.0);
    assert!(
        slow > 100.0,
        "1400 ms in 160 px must be coarser than 100 ms ({slow})"
    );
    // Never more than ten lines to count, however the chart is scaled.
    for scale in [40.0f32, 100.0, 400.0, 1_400.0, 9_000.0, 40_000.0] {
        for height in [60.0f32, 160.0, 440.0, 900.0] {
            let s = ms_band_step(scale, height);
            assert!(
                scale / s <= 10.0,
                "{scale} ms in {height} px: {s} ms gives {} lines",
                scale / s
            );
            assert!(
                height * s / scale >= 14.0 - f32::EPSILON || scale / s <= 1.0,
                "{scale} ms in {height} px: {s} ms labels collide"
            );
        }
    }
}

/// The time-window presets have to match the axes: every window needs a grid one can count, and
/// every window from two hours up is named after the clock.
#[test]
fn every_preset_window_labels_its_axis() {
    use system_ping::ui::{fmt_axis, nice_step};

    for c in HORIZON_CHOICES {
        let h = f64::from(c.secs);
        assert!(h / nice_step(h) <= 12.0, "{}: too many lines", c.label);
        let clock = fmt_axis(h, h, Some(1_700_000_000_000));
        if h >= 2.0 * 3_600.0 {
            assert_eq!(clock.len(), 5, "{}: no clock time ({clock:?})", c.label);
        } else {
            assert_eq!(clock.len(), 5, "{}: not MM:SS ({clock:?})", c.label);
        }
    }
}
