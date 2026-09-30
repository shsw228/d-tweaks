//! One picture per comment.
//!
//! Each comment is drawn one time into an `ImageBitmap`, which the page shows in
//! its own element and moves with a CSS animation. One long comment in a large font
//! takes 100 ms (measured: 48 characters, 7904×241 pixels), so this is off the
//! thread of the page.

use wasm_bindgen::JsCast;
use web_sys::{ImageBitmap, OffscreenCanvas, OffscreenCanvasRenderingContext2d};

use crate::layout::{ASCENT_RATIO, font};

/// Outline width, relative to the font size.
const STROKE_RATIO: f64 = 0.1;
const STROKE_COLOR: &str = "rgba(0, 0, 0, 0.8)";
/// Part of the font size below the baseline that a sprite keeps.
const DESCENT_RATIO: f64 = 0.35;

/// Draws one sprite.
pub struct Painter {
    scratch: OffscreenCanvas,
    ctx: OffscreenCanvasRenderingContext2d,
}

/// What `Painter::paint` made.
pub struct Painted {
    pub bitmap: ImageBitmap,
    pub pad: f64,
    pub width: f64,
    pub height: f64,
    pub pixels: (u32, u32),
}

impl Painter {
    pub fn new() -> Option<Self> {
        let scratch = OffscreenCanvas::new(1, 1).ok()?;
        let ctx = scratch.get_context("2d").ok()??.dyn_into().ok()?;
        Some(Self { scratch, ctx })
    }

    /// The context, for the width measurements of the layout.
    pub fn context(&self) -> &OffscreenCanvasRenderingContext2d {
        &self.ctx
    }

    /// `layout_width` is the width that the layout gave, in CSS pixels.
    pub fn paint(
        &self,
        text: &str,
        color: &str,
        font_px: f64,
        layout_width: f64,
        ratio: f64,
    ) -> Option<Painted> {
        let ctx = &self.ctx;
        let font_css = font(font_px);
        ctx.set_font(&font_css);
        // The layout scales one measurement; the real width at this size can be a
        // little wider, and a cut sprite loses the end of the text
        let text_width = ctx
            .measure_text(text)
            .map(|m| m.width())
            .unwrap_or(0.0)
            .max(layout_width);
        let pad = (font_px * STROKE_RATIO).ceil() + 2.0;
        let width = text_width + pad * 2.0;
        let height = font_px * (ASCENT_RATIO + DESCENT_RATIO) + pad * 2.0;

        let pixels = (
            (width * ratio).ceil().max(1.0) as u32,
            (height * ratio).ceil().max(1.0) as u32,
        );
        self.scratch.set_width(pixels.0);
        self.scratch.set_height(pixels.1);
        // A size change resets the context and clears it, so the state comes after it
        ctx.scale(ratio, ratio).ok()?;
        ctx.set_font(&font_css);
        let baseline = pad + font_px * ASCENT_RATIO;

        ctx.set_line_width(font_px * STROKE_RATIO);
        ctx.set_line_join("round");
        ctx.set_stroke_style_str(STROKE_COLOR);
        ctx.set_fill_style_str(color);
        // Outline first: the other order cuts the outline into the glyphs
        ctx.stroke_text(text, pad, baseline).ok()?;
        ctx.fill_text(text, pad, baseline).ok()?;

        Some(Painted {
            bitmap: self.scratch.transfer_to_image_bitmap().ok()?,
            pad,
            width,
            height,
            pixels,
        })
    }
}
