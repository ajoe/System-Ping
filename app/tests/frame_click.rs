// Settles the basis of the click editor: does `Frame::show` report a click? That is not a given —
// the frame registers its area with `Sense::hover()` only. The test
// clicks into a row built the way `ui::left_body()` builds one, using synthetic egui events.

use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Response};

fn press(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }
}

fn release(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }
}

/// One egui pass with the given events. `t` has to increase strictly — egui insists on
/// something about a clock that never runs backwards.
fn pass(
    ctx: &Context,
    t: f64,
    events: Vec<Event>,
    draw: &mut (impl FnMut(&mut egui::Ui) + ?Sized),
) {
    let mut raw = RawInput {
        events,
        time: Some(t),
        predicted_dt: 0.016,
        ..Default::default()
    };
    raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 400.0)));
    // The `FullOutput` that comes back is not interesting here; what matters is what the paint
    // closure saw.
    // The closure has to be able to run several times, so `draw` must not be moved into it.
    let _ = ctx.run(raw, |c| {
        egui::CentralPanel::default().show(c, |ui| draw(ui));
    });
}

/// A row like the one in the target list: a filled frame with two labels inside.
fn row(ui: &mut egui::Ui) -> Response {
    let frame = egui::Frame::new()
        .fill(egui::Color32::from_gray(60))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(7, 5));
    frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Gateway");
                ui.label("192.168.1.1 · vor 2 s");
            });
        })
        .response
}

#[test]
fn what_does_a_frame_report_on_click() {
    let ctx = Context::default();
    ctx.set_pixels_per_point(1.0);

    // Layout pass so egui knows the row's rect. Its middle is where a click has to land; the
    // panel puts 8 px of margin above, and the row is about 25 px tall.
    let at = Pos2::new(60.0, 8.0 + 12.0);

    let mut clicked = false;
    let mut hovered = false;
    let mut observe = |ui: &mut egui::Ui| {
        let resp = row(ui);
        if resp.clicked() {
            clicked = true;
        }
        if resp.hovered() {
            hovered = true;
        }
    };

    pass(&ctx, 0.5, vec![], &mut observe);
    pass(&ctx, 1.0, vec![Event::PointerMoved(at)], &mut observe);
    pass(&ctx, 1.02, vec![press(at)], &mut observe);
    pass(&ctx, 1.04, vec![release(at)], &mut observe);

    println!("at={at:?} hovered={hovered} clicked={clicked}");
    assert!(hovered, "the frame reports at least hover");
    assert!(!clicked, "a frame alone reports NO click");
}
