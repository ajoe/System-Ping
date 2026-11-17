//! Every character the program wants to show has to be in one of the fonts it ships.
//!
//! A missing glyph is not loud: egui draws the `.notdef` picture — an empty box — and nothing else
//! complains. That is how „Alle starten" became „▯ Alle starten" after egui 0.37 moved the icon
//! fallbacks behind a feature, and how the ✓ of „✓ Speichern" was a box for the same reason. The
//! look installs its fallback chain (see `theme::install_fonts`), and this test reads every
//! non-ascii character of every string literal in `src` and asks the fonts of that look about them.
//!
//! Asked the simple way there is nothing to ask: `has_glyph` answers whether a font of the family
//! knows the character, but a character that only the fallback family has is drawn correctly and
//! reported as missing. So the test compares the character against a known-nonexistent one: egui
//! puts the `.notdef` picture into the font texture for both, and equal pictures mean no glyph.

use egui::{Context, FontId, RawInput};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// A character no font can have: a Unicode non-character, so its cell is always `.notdef`.
const NO_GLYPH: char = '\u{FFFF}';

/// The non-ascii characters of every string literal in `dir`, with the files they appear in.
///
/// Only string literals: comments are never painted, and they quote plenty of things that no font
/// needs to draw.
fn drawn_characters(dir: &Path, out: &mut BTreeMap<char, Vec<String>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            drawn_characters(&path, out);
            continue;
        }
        if path.extension().map(|e| e != "rs").unwrap_or(true) {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            // A string literal: everything up to the next unescaped quote is text on screen.
            loop {
                match chars.next() {
                    None => break,
                    Some('\\') => {
                        chars.next();
                    }
                    Some('"') => break,
                    Some(inner) => {
                        if !inner.is_ascii() {
                            out.entry(inner).or_default().push(name.clone());
                        }
                    }
                }
            }
        }
    }
}

/// The pixels of the character's cell in the font texture of one font family.
///
/// Laying the character out is what puts it into the texture, so this is also the point where egui
/// decides which font of the family answers for it.
fn cell_pixels(ctx: &Context, font_id: &FontId, c: char) -> Vec<u8> {
    let uv = ctx.fonts_mut(|fonts| {
        let galley = fonts.layout_job(egui::text::LayoutJob::simple(
            c.to_string(),
            font_id.clone(),
            egui::Color32::WHITE,
            f32::INFINITY,
        ));
        galley
            .rows
            .iter()
            .flat_map(|row| row.row.glyphs.iter())
            .find(|g| g.chr == c)
            .map(|g| {
                (
                    g.uv_rect.min[0] as usize,
                    g.uv_rect.min[1] as usize,
                    g.uv_rect.max[0] as usize,
                    g.uv_rect.max[1] as usize,
                )
            })
    });
    let Some((x0, y0, x1, y1)) = uv else {
        return Vec::new();
    };
    let atlas = ctx.fonts(|fonts| fonts.image());
    let width = atlas.size[0];
    let mut pixels = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            pixels.push(atlas.pixels[y * width + x].a());
        }
    }
    pixels
}

#[test]
fn every_character_in_a_string_can_be_drawn() {
    let mut found: BTreeMap<char, Vec<String>> = BTreeMap::new();
    drawn_characters(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    assert!(!found.is_empty(), "no characters found to check at all");

    // The context the window runs on: `theme::apply` is what installs the fonts.
    let ctx = Context::default();
    system_ping::theme::apply(&ctx, system_ping::theme::Mode::Dark);
    let mut output = ctx.run_ui(RawInput::default(), |_| {});
    output.textures_delta.clear();

    let mut undrawable: Vec<String> = Vec::new();
    for (c, files) in &mut found {
        let mut missing = Vec::new();
        for (name, font_id) in [
            ("proportional", FontId::proportional(13.0)),
            ("monospace", FontId::monospace(13.0)),
        ] {
            // The `.notdef` picture is read after the character's own, so both see the same
            // texture: laying something out can make room in the atlas and move every cell.
            let pixels = cell_pixels(&ctx, &font_id, *c);
            let tofu = cell_pixels(&ctx, &font_id, NO_GLYPH);
            if pixels.is_empty() || pixels == tofu {
                missing.push(name);
            }
        }
        if !missing.is_empty() {
            files.sort();
            files.dedup();
            undrawable.push(format!(
                "U+{:04X} {:?} has no glyph in {} — used in {}",
                *c as u32,
                c,
                missing.join(" and "),
                files.join(", ")
            ));
        }
    }

    assert!(
        undrawable.is_empty(),
        "{} character(s) of a string literal have no glyph in the installed fonts:\n{}",
        undrawable.len(),
        undrawable.join("\n")
    );
}
