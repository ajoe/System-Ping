//! Text colour: one black in the light look, one white in the dark one — everywhere.
//!
//! The rule this file defends: an ordinary word never gets a grey of its own. Accents (outage red,
//! warning amber, a target's line colour) are allowed to be coloured, but normal text, secondary
//! text and axis labels are always the *same* black or white, only at different strength. That is
//! what `theme::apply` implements by writing one colour into all five widget states of egui.

use egui::{Context, Style};
use system_ping::theme::{self, Mode, BLACK_TEXT, WHITE_TEXT};

/// The style the app installs for one look.
fn style_for(mode: Mode) -> Style {
    let ctx = Context::default();
    theme::apply(&ctx, mode);
    (*ctx.style()).clone()
}

/// Every text slot egui picks a colour from must hold the look's own black or white. A widget that
/// reads `inactive` (a plain button) and one that reads `noninteractive` (a label) have to agree —
/// otherwise the same screen shows four strengths of grey next to each other, which is exactly what
/// egui's built-in looks do.
#[test]
fn alle_widget_zustande_nutzen_dieselbe_schriftfarbe() {
    for (mode, want) in [(Mode::Light, BLACK_TEXT), (Mode::Dark, WHITE_TEXT)] {
        let v = style_for(mode).visuals;
        assert_eq!(v.widgets.noninteractive.fg_stroke.color, want);
        assert_eq!(v.widgets.inactive.fg_stroke.color, want);
        assert_eq!(v.widgets.hovered.fg_stroke.color, want);
        assert_eq!(v.widgets.active.fg_stroke.color, want);
        assert_eq!(v.widgets.open.fg_stroke.color, want);
        // `text_color()` is what every unstyled label ends up using.
        assert_eq!(v.text_color(), want);
        // And egui's own derived flavours have to stay on that colour: `weak` fades it towards the
        // background, `strong` reads `active`. Both are only consistent if they start from black or
        // white — a grey in either slot would be a second text colour again.
        assert_eq!(v.strong_text_color(), want);
        let weak = v.weak_text_color();
        assert!(
            weak != want && weak != egui::Color32::TRANSPARENT,
            "schwacher Text muss derselben Farbe gehören, nur leiser sein ({mode:?})"
        );
    }
}

/// Painter text (axis labels, tooltips, the hover readout) does not go through egui's widgets — the
/// app hands a colour to `Painter::text` itself. Those colours are the same two.
#[test]
fn gemalter_text_und_schwacher_text_passen_zusammen() {
    for (mode, want) in [(Mode::Light, BLACK_TEXT), (Mode::Dark, WHITE_TEXT)] {
        assert_eq!(mode.text(), want);
        assert_eq!(mode.text_normal(), want);
        assert_eq!(mode.text_cursor(), want);
        // Secondary text is the same colour with less strength, never a hue or grey of its own. The
        // look's background is what it fades towards, so the result has to stay on that one channel:
        // black fading towards white stays neutral, and so does white fading towards near-black.
        for c in [mode.text_dim(), mode.axis_text()] {
            let (r, g, b, _) = c.to_tuple();
            assert!(
                r == g && g == b,
                "{mode:?}: schwacher Text darf nicht farbig sein ({c:?})"
            );
            let faded = match mode {
                Mode::Light => r < 255, // black, lightened towards the white panel
                Mode::Dark => r > 0,    // white, darkened towards the near-black panel
            };
            assert!(
                faded,
                "{mode:?}: schwacher Text muss leiser sein als {want:?}, nicht {c:?}"
            );
        }
        // `text_weak()` is the older name for the same thing — one function, not a second grey.
        assert_eq!(mode.text_weak(), mode.text_dim());
    }
}

/// Marked text is the one place where black-on-white cannot work: on a blue selection background
/// the text has to be the opposite. Still only two colours in the whole program.
#[test]
fn ausgewaehlter_text_ist_gegenueber_dem_hintergrund_leuchtend() {
    for mode in [Mode::Light, Mode::Dark] {
        let bg = mode.selection_fill();
        let fg = mode.selection_text();
        // Enough distance to read, and the fill really is a coloured background, not text colour.
        let dist = (i32::from(fg.r()) - i32::from(bg.r()))
            .abs()
            .max((i32::from(fg.g()) - i32::from(bg.g())).abs())
            .max((i32::from(fg.b()) - i32::from(bg.b())).abs());
        assert!(
            dist > 120,
            "{mode:?}: Auswahl wäre nicht lesbar ({fg:?} auf {bg:?})"
        );
    }
}
