//! The look of the window: follows the OS light/dark setting, with a manual override.

use crate::app::LogKind;
use egui::{Color32, Stroke, Visuals};

/// Accents that read well on both looks (amber, red, muted blue, orange).
pub const WARN: Color32 = Color32::from_rgb(236, 180, 62);
pub const BAD: Color32 = Color32::from_rgb(224, 82, 82);
pub const GOOD: Color32 = Color32::from_rgb(58, 163, 120);
pub const OFFLINE: Color32 = Color32::from_rgb(120, 128, 142);
pub const ORANGE: Color32 = Color32::from_rgb(236, 130, 56);

/// The two text colours of the whole program: black in the light look, white in the dark one.
/// Every word that is not an accent (an outage, a warning, a target's own line colour) uses one of
/// them, in both looks — see [`apply`], which also hands them to egui so widgets inherit them.
pub const BLACK_TEXT: Color32 = Color32::from_rgb(0, 0, 0);
pub const WHITE_TEXT: Color32 = Color32::from_rgb(255, 255, 255);

/// One entry of the automatic trace palette: a name to show in the picker (in both languages) and
/// the two shades.
pub struct Swatch {
    pub name_de: &'static str,
    pub name_en: &'static str,
    light: [u8; 3],
    dark: [u8; 3],
}

/// The colours a new target is given automatically, in order. They are picked so several traces
/// stay distinguishable even though the chart paints every line at 1.5 px on the same grid.
pub const PALETTE: &[Swatch] = &[
    Swatch {
        name_de: "Blau",
        name_en: "Blue",
        light: [30, 105, 190],
        dark: [86, 160, 235],
    },
    Swatch {
        name_de: "Grün",
        name_en: "Green",
        light: [24, 122, 76],
        dark: [70, 190, 110],
    },
    Swatch {
        name_de: "Violett",
        name_en: "Purple",
        light: [128, 74, 186],
        dark: [172, 132, 235],
    },
    Swatch {
        name_de: "Türkis",
        name_en: "Teal",
        light: [18, 122, 132],
        dark: [70, 190, 200],
    },
    Swatch {
        name_de: "Beere",
        name_en: "Berry",
        light: [176, 54, 122],
        dark: [235, 118, 172],
    },
    Swatch {
        name_de: "Sand",
        name_en: "Sand",
        light: [140, 112, 24],
        dark: [214, 186, 96],
    },
    Swatch {
        name_de: "Zinnober",
        name_en: "Vermilion",
        light: [186, 62, 44],
        dark: [230, 104, 88],
    },
    Swatch {
        name_de: "Schiefer",
        name_en: "Slate",
        light: [74, 92, 116],
        dark: [144, 162, 188],
    },
];

/// How many automatic colours there are. The picker uses it to decide when to offer "auto".
pub const PALETTE_LEN: usize = PALETTE.len();

/// Size of a colour knob — the little square that says "this colour belongs to this line" in the
/// chart legend, the target list and anywhere else a trace is named. One constant so every place
/// draws the same size and the meaning never looks different from panel to panel.
pub const LEGEND_KNOB: f32 = 10.0;

/// Colour number `n` of the palette. A target stores this number instead of a colour, so the same
/// target keeps its colour across restarts and both looks stay readable.
pub fn swatch(n: usize) -> &'static Swatch {
    &PALETTE[n % PALETTE.len()]
}

impl Swatch {
    /// The shade for one look.
    pub fn color(&self, mode: Mode) -> Color32 {
        let rgb = match mode {
            Mode::Light => self.light,
            Mode::Dark => self.dark,
        };
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    /// The name in one language.
    pub fn name(&self, lang: crate::i18n::Lang) -> &'static str {
        match lang {
            crate::i18n::Lang::De => self.name_de,
            crate::i18n::Lang::En => self.name_en,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    pub fn from_dark(dark: bool) -> Self {
        if dark {
            Mode::Dark
        } else {
            Mode::Light
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }

    /// The shade of palette colour `n` for this look.
    pub fn swatch(self, n: usize) -> Color32 {
        swatch(n).color(self)
    }

    /// Colour used for a target that is currently online.
    pub fn online_fill(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(20, 124, 66),
            Mode::Dark => GOOD,
        }
    }

    /// Colour for the grid lines and other quiet background marks.
    pub fn grid(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(206, 210, 216),
            Mode::Dark => Color32::from_rgb(48, 54, 66),
        }
    }

    /// Colour for the axis labels. Same black / white as all other text, only quieter — a grey of
    /// its own would read as a second text colour on the same screen.
    pub fn axis_text(self) -> Color32 {
        self.text_dim()
    }

    /// Colour for the panel behind the chart.
    pub fn plot_fill(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(255, 255, 255),
            Mode::Dark => Color32::from_rgb(14, 17, 22),
        }
    }

    /// Colour for the window border and separators.
    pub fn stroke(self) -> Stroke {
        match self {
            Mode::Light => Stroke::new(1.0_f32, Color32::from_rgb(198, 202, 210)),
            Mode::Dark => Stroke::new(1.0_f32, Color32::from_rgb(46, 52, 64)),
        }
    }

    /// Colour of the window background behind the panels.
    pub fn frame_fill(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(236, 238, 241),
            Mode::Dark => Color32::from_rgb(18, 21, 27),
        }
    }

    /// Colour of the bars in the target list and the stats table.
    pub fn row_fill(self, selected: bool) -> Color32 {
        match self {
            Mode::Light => {
                if selected {
                    Color32::from_rgb(214, 226, 244)
                } else {
                    Color32::from_rgb(247, 248, 250)
                }
            }
            Mode::Dark => {
                if selected {
                    Color32::from_rgb(38, 46, 60)
                } else {
                    Color32::from_rgb(26, 29, 36)
                }
            }
        }
    }

    /// Colour of the text drawn directly by the painter (axis labels, tooltips).
    pub fn text(self) -> Color32 {
        match self {
            Mode::Light => BLACK_TEXT,
            Mode::Dark => WHITE_TEXT,
        }
    }

    /// Secondary text: deliberately not a grey of its own but the normal colour with alpha, so a
    /// faint word is the *same* black or white as every other word, only quieter. That keeps one
    /// look consistent while still letting a disabled row read as disabled.
    pub fn text_dim(self) -> Color32 {
        match self {
            Mode::Light => BLACK_TEXT.gamma_multiply(0.7),
            Mode::Dark => WHITE_TEXT.gamma_multiply(0.68),
        }
    }

    /// Colour of selected (marked) text, i.e. the text on top of [`Self::selection_fill`].
    pub fn selection_text(self) -> Color32 {
        match self {
            Mode::Light => WHITE_TEXT,
            Mode::Dark => Color32::from_gray(16),
        }
    }

    /// Background of selected text.
    pub fn selection_fill(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(40, 110, 200),
            Mode::Dark => Color32::from_rgb(70, 130, 210),
        }
    }

    /// Colour of a cursor in an input field.
    pub fn text_cursor(self) -> Color32 {
        self.text()
    }

    /// The normal text colour of this look — black on white, white on dark. Everything that is not
    /// an accent uses it: labels, buttons, headings, input fields, painter text. See [`apply`].
    pub fn text_normal(self) -> Color32 {
        self.text()
    }

    /// Colour of the tooltip box behind that text.
    pub fn tooltip_fill(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgba_premultiplied(255, 255, 255, 235),
            Mode::Dark => Color32::from_rgba_premultiplied(12, 14, 18, 220),
        }
    }

    /// Colour of the marks for a target that stopped answering.
    pub fn loss_fill(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(196, 60, 52),
            Mode::Dark => Color32::from_rgb(200, 70, 60),
        }
    }

    /// Colour of the band along the bottom of the plot that holds those marks.
    pub fn loss_band(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgba_premultiplied(214, 150, 145, 90),
            Mode::Dark => Color32::from_rgba_premultiplied(60, 30, 30, 110),
        }
    }

    /// Colour of the dot that shows whether the probes are running.
    pub fn run_dot(self, running: bool) -> Color32 {
        match self {
            Mode::Light => {
                if running {
                    Color32::from_rgb(24, 130, 78)
                } else {
                    Color32::from_rgb(120, 126, 136)
                }
            }
            Mode::Dark => {
                if running {
                    Color32::from_rgb(90, 200, 120)
                } else {
                    Color32::from_rgb(150, 150, 150)
                }
            }
        }
    }

    /// Colour of the "Verluste" count in the status bar.
    pub fn loss_count(self, any: bool) -> Color32 {
        match self {
            Mode::Light => {
                if any {
                    Color32::from_rgb(176, 96, 20)
                } else {
                    Color32::from_rgb(40, 104, 68)
                }
            }
            Mode::Dark => {
                if any {
                    Color32::from_rgb(225, 170, 90)
                } else {
                    Color32::from_rgb(150, 190, 160)
                }
            }
        }
    }

    /// Colour of the "Alle entfernen" button's label.
    pub fn danger_text(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(172, 60, 50),
            Mode::Dark => Color32::from_rgb(200, 120, 110),
        }
    }

    /// Accent colour (title, links).
    pub fn warn(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(176, 120, 20),
            Mode::Dark => Color32::from_rgb(255, 210, 90),
        }
    }

    /// Secondary text colour.
    pub fn text_weak(self) -> Color32 {
        self.text_dim()
    }

    /// Text colour for a count of outliers.
    pub fn warn_count(self, any: bool) -> Color32 {
        if any {
            self.warn()
        } else {
            self.text_weak()
        }
    }

    /// Colour of the crosshair line drawn where the pointer is.
    pub fn hover_line(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgba_premultiplied(60, 64, 72, 110),
            Mode::Dark => Color32::from_rgba_premultiplied(200, 200, 200, 110),
        }
    }

    /// Text colour for a log line of this kind.
    pub fn log_kind(self, kind: LogKind) -> Color32 {
        match kind {
            LogKind::Outage => self.bad_text(),
            LogKind::Recovery => {
                if self == Mode::Light {
                    Color32::from_rgb(24, 122, 76)
                } else {
                    Color32::from_rgb(100, 200, 130)
                }
            }
            LogKind::Info => self.axis_text(),
        }
    }

    /// Text colour for something that went wrong.
    pub fn bad_text(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(184, 54, 44),
            Mode::Dark => Color32::from_rgb(225, 90, 80),
        }
    }

    /// Text colour for a warning (a spike count, an outlier marker).
    pub fn warn_text(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(166, 118, 8),
            Mode::Dark => Color32::from_rgb(255, 210, 90),
        }
    }

    /// Text colour for a loss percentage worth noticing.
    pub fn loss_text(self, bad: bool) -> Color32 {
        if bad {
            self.bad_text()
        } else {
            self.text_weak()
        }
    }

    /// Text colour for something that is fine. In the dark look this used to be a grey, i.e. a
    /// third kind of light text next to normal and weak text; now it is simply the normal white.
    pub fn ok_text(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(40, 104, 68),
            Mode::Dark => WHITE_TEXT,
        }
    }

    /// The accent colour of the title and other highlights.
    pub fn accent(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(30, 106, 190),
            Mode::Dark => Color32::from_rgb(90, 170, 250),
        }
    }

    /// Colour of a status dot: paused, waiting for the first reply, or in a latency band.
    pub fn status_dot(self, paused: bool, running: bool, rtt: f32) -> Color32 {
        if paused {
            return self.paused();
        }
        if !rtt.is_finite() {
            return if running {
                self.waiting()
            } else {
                self.paused()
            };
        }
        self.rtt_band(rtt)
    }

    /// A target that is switched off.
    pub fn paused(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(150, 156, 166),
            Mode::Dark => Color32::from_rgb(90, 96, 108),
        }
    }

    /// A target that is switched on but has not answered yet.
    pub fn waiting(self) -> Color32 {
        match self {
            Mode::Light => Color32::from_rgb(30, 106, 190),
            Mode::Dark => Color32::from_rgb(120, 170, 240),
        }
    }

    /// The latency bands, green to red; shared by the status dots and the chart lines.
    pub fn rtt_band(self, rtt: f32) -> Color32 {
        if !rtt.is_finite() {
            return self.bad_text();
        }
        let (light, dark) = if rtt < 30.0 {
            ((24, 122, 76), (70, 190, 110))
        } else if rtt < 80.0 {
            ((110, 140, 30), (150, 205, 80))
        } else if rtt < 150.0 {
            ((160, 124, 10), (235, 190, 70))
        } else if rtt < 300.0 {
            ((180, 96, 20), (235, 140, 60))
        } else {
            ((184, 54, 44), (220, 80, 70))
        };
        match self {
            Mode::Light => Color32::from_rgb(light.0, light.1, light.2),
            Mode::Dark => Color32::from_rgb(dark.0, dark.1, dark.2),
        }
    }
}

/// Apply the look to an egui context.
pub fn apply(ctx: &egui::Context, mode: Mode) {
    let mut style = egui::Style::default();
    let mut visuals = match mode {
        Mode::Light => Visuals::light(),
        Mode::Dark => Visuals::dark(),
    };

    // The window background: what shows through wherever no panel paints over it.
    visuals.panel_fill = mode.frame_fill();
    // The panels in `ui` ask for these two by name, so they have to follow the look too.
    visuals.window_fill = mode.frame_fill();
    visuals.extreme_bg_color = mode.plot_fill();

    // One text colour for everything: black on a bright look, white on a dark one. egui ships its
    // light look with grey button and label text (gray 60–80) and its dark look with gray 140/180,
    // so without this the same screen would mix three or four strengths of the same colour. All
    // five widget states get the same value — a hovered button must not be brighter than a label
    // next to it. `weak` and `strong` text then follow automatically: egui derives them from
    // `noninteractive.fg_stroke` (fading towards the background) and `active.fg_stroke`, both of
    // which are black / white now.
    for state in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        state.fg_stroke.color = mode.text_normal();
    }
    // Input fields: the caret in the normal colour instead of egui's light-blue default. (The type
    // itself is not re-exported by egui 0.31, so only the field is reachable — which is enough to
    // change the stroke.) The placeholder of an empty field does not need its own colour: egui
    // paints it with `weak_text_color()`, i.e. this look's black or white, faded a little.
    visuals.text_cursor.stroke = Stroke::new(1.5_f32, mode.text_cursor());
    visuals.override_text_color = None;
    // Marked text: the look's own text colour would be invisible on the selection background, so
    // this one combination is deliberately inverted (white on blue / dark on blue).
    visuals.selection.bg_fill = mode.selection_fill();
    visuals.selection.stroke = Stroke::new(1.0_f32, mode.selection_text());

    style.visuals = visuals;
    ctx.set_style(style);
}

/// Decide which look to use for this frame.
pub fn resolve(ctx: &egui::Context, override_name: &str) -> Mode {
    match override_name.trim().to_ascii_lowercase().as_str() {
        "light" | "hell" | "bright" => Mode::Light,
        "dark" | "dunkel" => Mode::Dark,
        _ => {
            if ctx.style().visuals.dark_mode {
                Mode::Dark
            } else {
                Mode::Light
            }
        }
    }
}
