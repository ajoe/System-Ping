//! One egui pass without a window.
//!
//! Both the tests and the screenshot tool drive the real interface from a headless
//! [`egui::Context`], and since egui 0.37 both need the same two things a window used to take
//! care of: the root `Ui` of a pass, and relief from the texture deltas of a pass.

/// Draws one frame on a headless [`egui::Context`], the way a window's repaint callback does.
///
/// Since 0.37 egui does not hand the root `Ui` out through the context any more: the entry point
/// is [`egui::Context::run_ui`], which gives the `Ui` to a closure. Anything that used to work
/// with a `&Context` alone starts here instead.
///
/// The [`egui::FullOutput`] comes back with `textures_delta` emptied. Those deltas are the part of
/// a pass that belongs to a GPU — mostly the font atlas — and a headless pass has no backend to
/// hand them to: tessellation reads the glyphs from egui's own texture store, which is already up
/// to date. epaint asserts about a delta that is dropped unapplied (in debug builds), so it is
/// discharged here once instead of in every caller.
pub fn paint(
    ctx: &egui::Context,
    raw: egui::RawInput,
    draw: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut out = ctx.run_ui(raw, draw);
    out.textures_delta.clear();
    out
}
