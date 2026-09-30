//! Lanes for the comments. All comments are known at the start, so this runs once
//! and not per frame; only a size change runs it again.

use d_tweaks_shared::danmaku::LANES;
use web_sys::OffscreenCanvasRenderingContext2d;

/// Margin left and right of a fixed comment, relative to the width.
const FIXED_MARGIN_RATIO: f64 = 0.02;
/// Reference size for the width measurement.
const MEASURE_FONT_PX: f64 = 100.0;
/// Part of the font size above the baseline.
pub const ASCENT_RATIO: f64 = 0.85;

#[derive(Clone, Copy, PartialEq)]
pub enum Position {
    Naka,
    Ue,
    Shita,
}

/// A comment as received. The lane needs the canvas size, so it comes later.
pub struct Raw {
    /// Seconds.
    pub start: f64,
    pub text: String,
    pub color: String,
    /// Factor on the lane height.
    pub scale: f64,
    pub position: Position,
}

/// A comment with a lane.
///
/// The text and the colour stay in `Raw` (`index`). A copy here would rebuild
/// thousands of strings on every resize.
pub struct Placed {
    /// Index into the comments.
    pub index: usize,
    pub start: f64,
    pub font_px: f64,
    pub width: f64,
    /// Baseline.
    pub y: f64,
    pub position: Position,
}

/// Text to its width at `MEASURE_FONT_PX`.
pub type WidthCache = std::collections::HashMap<String, f64>;

/// Width of the text.
///
/// `measureText` shapes the text again on every call, which is expensive for
/// thousands of comments on every resize. The width is nearly proportional to
/// the font size, so measure once at the reference size and scale. The same text
/// repeats often, so the result is kept: a resize then measures nothing.
fn measured_width(
    ctx: &OffscreenCanvasRenderingContext2d,
    cache: &mut WidthCache,
    text: &str,
) -> f64 {
    if let Some(width) = cache.get(text) {
        return *width;
    }
    ctx.set_font(&font(MEASURE_FONT_PX));
    let width = ctx.measure_text(text).map(|m| m.width()).unwrap_or(0.0);
    cache.insert(text.to_string(), width);
    width
}

/// Does a moving comment touch the one before it?
///
/// A wider comment is faster (it crosses in the same time), so "did the comment
/// before enter completely" is not sufficient: it can also overtake. Both
/// positions are linear in time, so the two ends of the interval are enough.
fn collides(prev: &(f64, f64), start: f64, width: f64, screen: f64, duration: f64) -> bool {
    let (prev_start, prev_width) = *prev;
    let prev_speed = (screen + prev_width) / duration;
    let speed = (screen + width) / duration;

    // 1. At the entry, the right end of the comment before is still off screen
    let prev_x = screen - prev_speed * (start - prev_start);
    if prev_x + prev_width > screen {
        return true;
    }
    // 2. When the comment before leaves, this one is already past the left edge
    let prev_exit = prev_start + duration;
    screen - speed * (prev_exit - start) < 0.0
}

/// Assign a lane to every comment.
pub fn layout(
    ctx: &OffscreenCanvasRenderingContext2d,
    raw: &[Raw],
    widths: &mut WidthCache,
    width: f64,
    height: f64,
    duration: f64,
) -> Vec<Placed> {
    let lane_px = height / LANES as f64;

    // For a moving comment, the last one in the lane is enough for the test
    let mut naka: Vec<Option<(f64, f64)>> = vec![None; LANES];
    // A fixed comment does not move, so only the free time matters
    let mut ue: Vec<f64> = vec![f64::NEG_INFINITY; LANES];
    let mut shita: Vec<f64> = vec![f64::NEG_INFINITY; LANES];

    // Width available to a fixed comment
    let usable = width * (1.0 - FIXED_MARGIN_RATIO * 2.0);

    let mut placed = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let mut font_px = lane_px * item.scale;
        let mut text_width = measured_width(ctx, widths, &item.text) * font_px / MEASURE_FONT_PX;

        // A fixed comment does not move, so a long text stays off screen for
        // seconds. Make it smaller until it fits (nicovideo does the same).
        if item.position != Position::Naka && text_width > usable && text_width > 0.0 {
            font_px *= usable / text_width;
            text_width = usable;
        }

        // Large text needs more than one lane. Use the size after the reduction,
        // not the factor: the reduction can bring it back into one lane.
        let span = ((font_px / lane_px).ceil() as usize).clamp(1, LANES);

        let lane = match item.position {
            Position::Naka => {
                let free = (0..=LANES.saturating_sub(span)).find(|&i| {
                    (i..i + span).all(|j| {
                        naka[j].as_ref().is_none_or(|prev| {
                            !collides(prev, item.start, text_width, width, duration)
                        })
                    })
                });
                // No free lane: use the lane that becomes free first. Never drop.
                let lane = free.unwrap_or_else(|| {
                    (0..=LANES.saturating_sub(span))
                        .min_by(|&a, &b| {
                            exit_of(&naka, a, span, duration)
                                .total_cmp(&exit_of(&naka, b, span, duration))
                        })
                        .unwrap_or(0)
                });
                for slot in naka.iter_mut().skip(lane).take(span) {
                    *slot = Some((item.start, text_width));
                }
                lane
            }
            Position::Ue | Position::Shita => {
                let slots = if item.position == Position::Ue {
                    &mut ue
                } else {
                    &mut shita
                };
                let free = (0..=LANES.saturating_sub(span))
                    .find(|&i| (i..i + span).all(|j| slots[j] <= item.start));
                let lane = free.unwrap_or_else(|| {
                    (0..=LANES.saturating_sub(span))
                        .min_by(|&a, &b| max_of(slots, a, span).total_cmp(&max_of(slots, b, span)))
                        .unwrap_or(0)
                });
                for slot in slots.iter_mut().skip(lane).take(span) {
                    *slot = item.start + duration;
                }
                lane
            }
        };

        // `shita` counts from the bottom
        let top = match item.position {
            Position::Shita => height - (lane + span) as f64 * lane_px,
            _ => lane as f64 * lane_px,
        };
        placed.push(Placed {
            index,
            start: item.start,
            font_px,
            width: text_width,
            // Baseline sits one text height below the top of the lane
            y: top + font_px * ASCENT_RATIO,
            position: item.position,
        });
    }
    placed
}

/// Latest time at which the lanes become free (moving comments).
fn exit_of(lanes: &[Option<(f64, f64)>], from: usize, span: usize, duration: f64) -> f64 {
    lanes
        .iter()
        .skip(from)
        .take(span)
        .map(|slot| slot.map_or(f64::NEG_INFINITY, |(start, _)| start + duration))
        .fold(f64::NEG_INFINITY, f64::max)
}

/// Latest time at which the lanes become free (fixed comments).
fn max_of(lanes: &[f64], from: usize, span: usize) -> f64 {
    lanes
        .iter()
        .skip(from)
        .take(span)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
}

pub fn font(px: f64) -> String {
    // Bold gothic, like nicovideo
    format!("bold {px}px \"Hiragino Kaku Gothic ProN\", sans-serif")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Screen 800px, comment 100px wide, at t=0. With 4 seconds to cross, the
    /// speed is (800+100)/4 = 225px/s, so the right end passes the right edge
    /// after 100/225 = 0.444 s.
    #[test]
    fn lane_is_busy_until_previous_comment_fully_entered() {
        let prev = (0.0, 100.0);
        assert!(
            collides(&prev, 0.4, 100.0, 800.0, 4.0),
            "the right end is still on screen"
        );
        assert!(!collides(&prev, 0.5, 100.0, 800.0, 4.0), "the lane is free");
    }
    /// A shorter duration is faster: 2 seconds gives 450px/s, so the lane is
    /// free after 100/450 = 0.222 s.
    #[test]
    fn a_shorter_duration_frees_the_lane_sooner() {
        let prev = (0.0, 100.0);
        assert!(
            collides(&prev, 0.2, 100.0, 800.0, 2.0),
            "the right end is still on screen"
        );
        assert!(!collides(&prev, 0.3, 100.0, 800.0, 2.0), "the lane is free");
    }
    #[test]
    fn detects_overtaking_by_a_wider_comment() {
        // A 3000px comment overtakes a slow one of width 0. The "did it enter
        // completely" test alone does not find this.
        let slow = (0.0, 0.0);
        assert!(
            !collides(&slow, 1.0, 100.0, 800.0, 4.0),
            "a similar width is safe"
        );
        assert!(
            collides(&slow, 1.0, 3000.0, 800.0, 4.0),
            "a wider comment overtakes"
        );
    }
}
