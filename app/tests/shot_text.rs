//! A `--shot` picture has to show words, not bars.
//!
//! Text reaches the raster as triangles — two per glyph, covering the box of that glyph — and the
//! shape of a letter is nowhere in them: it is in the font atlas egui builds for the GPU. A raster
//! that never looks at that atlas paints every glyph as a solid box in the text's colour, which
//! gives a picture that shows where words stand and nothing else. So this test paints the real
//! interface the way `--shot` does and looks at what a label's box is made of: letters have edges,
//! pixels between the bare background and full ink, while a filled box has only its own border.

use egui::epaint::{ClippedPrimitive, Mesh, Primitive, TextureId, Vertex};
use egui::{Context, Pos2, Rect, Shape, Vec2};
use std::collections::BTreeMap;
use std::path::PathBuf;
use system_ping::app::{App, AppOptions};
use system_ping::{headless, screenshot, theme, ui};

/// A painted frame, plus what the draw list knows about it.
struct Frame {
    ctx: Context,
    shapes: Vec<ClippedPrimitive>,
    surface: Rect,
    /// Where the words of the picture stand, as the draw list says.
    words: Vec<(String, Rect)>,
}

/// Runs the interface for the two passes a picture needs, and hands back what the second one
/// tessellated into. Both passes are needed: a first pass on a fresh context lays out its text
/// against an atlas egui uploads only while preparing output, so its frame is missing glyphs (see
/// `Screencap::ui`).
fn paint_interface() -> Frame {
    let mut app = App::new(AppOptions::default(), Vec::new());
    let ctx = Context::default();
    theme::apply(&ctx, app.theme_mode);
    ctx.set_pixels_per_point(1.0);
    let surface = Rect::from_min_size(Pos2::ZERO, Vec2::new(1080.0, 700.0));

    let mut shapes = Vec::new();
    let mut words = Vec::new();
    for i in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(surface),
            // egui derives its animation clock from `time` and `predicted_dt` and insists on it
            // moving forward, so every pass needs a later stamp than the one before (see
            // `Screencap::ui`, which has the same problem for the same reason).
            time: Some(i as f64 * 0.016),
            predicted_dt: 0.016,
            ..Default::default()
        };
        let out = headless::paint(&ctx, raw, |ui| {
            ui::top(ui, &mut app);
            ui::left(ui, &mut app);
            ui::center_ex(ui, &mut app, false);
            ui::bottom(ui, &mut app);
        });
        if i == 1 {
            for clipped in &out.shapes {
                if let Shape::Text(t) = &clipped.shape {
                    let g = t.galley.clone();
                    words.push((
                        g.job.text.clone(),
                        Rect::from_min_max(
                            t.pos + g.rect.min.to_vec2(),
                            t.pos + g.rect.max.to_vec2(),
                        ),
                    ));
                }
            }
            shapes = ctx.tessellate(out.shapes, out.pixels_per_point);
        }
    }
    Frame {
        ctx,
        shapes,
        surface,
        words,
    }
}

/// A path in the temp folder, for one run of one test.
fn temp_png(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "system-ping-test-{}-{}.png",
        name,
        std::process::id()
    ))
}

/// What a label's box is made of: how much of it is painted at all, and how much of it is the
/// edge of a stroke — a pixel between the colour the area stands on and full ink. The colour
/// underneath is taken from the area itself rather than from the look, so a panel with its own
/// background does not turn the whole box into text.
fn ink(img: &image::RgbaImage, box_rect: Rect) -> (f32, f32) {
    let x0 = box_rect.min.x.floor().max(0.0) as u32;
    let y0 = box_rect.min.y.floor().max(0.0) as u32;
    let x1 = box_rect.max.x.ceil().min(img.width() as f32) as u32;
    let y1 = box_rect.max.y.ceil().min(img.height() as f32) as u32;
    let mut counts = BTreeMap::<[u8; 3], usize>::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y).0;
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
    }
    let (ground, _) = counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .expect("a label box always has pixels");
    // How far each pixel stands out from that ground, and how far the strongest one does: that
    // strongest one is the middle of a stroke, so its distance is what a full stroke looks like.
    let dist = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        (0..3).map(|i| p[i].abs_diff(ground[i])).max().unwrap_or(0) as u32
    };
    let mut full = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            full = full.max(dist(x, y));
        }
    }
    let mut lit = 0usize;
    let mut soft = 0usize;
    let mut total = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let d = dist(x, y);
            total += 1;
            if d > 16 {
                lit += 1;
            }
            // A stroke that is neither bare ground nor fully inked: the edge of a letter. Only
            // something with a shape can have edges; a filled box has one at its border only.
            if d * 100 > full * 20 && d * 100 < full * 85 {
                soft += 1;
            }
        }
    }
    (lit as f32 / total as f32, soft as f32 / total as f32)
}

/// Every letter of a label has to be a letter: painted, and with edges that show a shape.
#[test]
fn shot_text_is_letters_and_not_blocks() {
    let frame = paint_interface();
    let path = temp_png("shot-text");
    let (w, h) = screenshot::capture(&frame.ctx, Some(frame.surface), &frame.shapes, &path)
        .expect("the picture writes");
    let img = image::open(&path).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (w, h));

    // Words of different size, weight and colour: the heading of the window, a plain row label,
    // and a faint one, all of them in the proportional font.
    let wanted = ["System-Ping", "DNS (Cloudflare)", "Latenzverlauf"];
    let mut seen = 0;
    for want in wanted {
        let (_, box_rect) = frame
            .words
            .iter()
            .find(|(text, _)| text == want)
            .unwrap_or_else(|| panic!("`{want}` is not in the picture"));
        let (lit, soft) = ink(&img, *box_rect);
        println!("{want}: {lit:.3} of the box is painted, {soft:.3} of it is a stroke edge");
        assert!(
            lit > 0.05,
            "`{want}` has nothing drawn in its {box_rect:?} box ({lit:.3} lit): the letters are \
             missing from the picture"
        );
        assert!(
            soft > 0.03,
            "`{want}` shows no stroke edge anywhere in its {box_rect:?} box ({soft:.3}): every \
             painted pixel is flat colour, so the box is filled and the letters are missing from \
             it"
        );
        seen += 1;
    }
    assert_eq!(seen, wanted.len());
    let _ = std::fs::remove_file(&path);
}

/// A shape is not text and has to stay as solid as it was: its vertices ask for the atlas's white
/// pixel (`WHITE_UV`), and multiplying a colour by white has to give that same colour again.
#[test]
fn shot_shapes_stay_solid() {
    let frame = paint_interface();
    let path = temp_png("shot-solid");
    let fill = egui::Color32::from_rgb(17, 200, 60);
    let rect = Rect::from_min_max(Pos2::new(40.0, 40.0), Pos2::new(120.0, 90.0));
    let corner = |x: f32, y: f32| Vertex {
        pos: Pos2::new(x, y),
        uv: egui::epaint::WHITE_UV,
        color: fill,
    };
    let mesh = Mesh {
        indices: vec![0, 1, 2, 2, 3, 0],
        vertices: vec![
            corner(rect.min.x, rect.min.y),
            corner(rect.max.x, rect.min.y),
            corner(rect.max.x, rect.max.y),
            corner(rect.min.x, rect.max.y),
        ],
        texture_id: TextureId::Managed(0),
    };
    let shapes = vec![ClippedPrimitive {
        clip_rect: frame.surface,
        primitive: Primitive::Mesh(mesh),
    }];
    screenshot::capture(&frame.ctx, Some(frame.surface), &shapes, &path)
        .expect("the picture writes");
    let img = image::open(&path).unwrap().to_rgba8();
    let bg = frame.ctx.global_style().visuals.panel_fill;
    // Inside the corners every pixel has to be the colour of the shape, to the unit: the four
    // corners of the mesh ask for the atlas's white pixel, so any change at all is the sampler
    // having reached something that was not white.
    for y in (42..88).step_by(7) {
        for x in (42..118).step_by(11) {
            let p = img.get_pixel(x, y).0;
            assert_eq!(
                [p[0], p[1], p[2], p[3]],
                [fill.r(), fill.g(), fill.b(), 255],
                "the white of `WHITE_UV` changed a solid shape at {x},{y}"
            );
        }
    }
    // And just outside it nothing was painted.
    let p = img.get_pixel(130, 55).0;
    assert_eq!(
        [p[0], p[1], p[2]],
        [bg.r(), bg.g(), bg.b()],
        "the shape reached past its corners"
    );
    let _ = std::fs::remove_file(&path);
}
