//! The comments on screen as elements that the compositor moves.
//!
//! Each comment is a `<canvas>` with its picture (`bitmaprenderer`, no copy) and one
//! Web Animation of `transform` and `opacity`. The compositor of the browser runs these
//! on every frame of the display, at the time of that frame, without any script, so a
//! late frame of the page or of a worker does not show. A canvas over the whole video
//! that is drawn again on every frame costs the compositor the area of the screen; here
//! it is only the area of the comments.
//!
//! The animations run on the time of the document, not of the video. `Timing` finds
//! the relation between the two. A pause, a seek or another rate places every
//! animation again. A small drift never does: a new place is a jump of every comment
//! on screen, which looks like a stop or a step back. The animations take a rate a
//! little faster or slower until they are back with the video, as the player of
//! nicovideo does.

use std::collections::HashMap;

use js_sys::{Array, Float64Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    Animation, Document, Element, HtmlCanvasElement, ImageBitmap, ImageBitmapRenderingContext,
};

use d_tweaks_shared::danmaku as protocol;
use d_tweaks_shared::json;

use crate::log;

/// Seconds ahead of the play position that the pictures are asked for. One long
/// comment takes 100 ms in the worker.
const LOOKAHEAD_SECONDS: f64 = 2.0;
/// Pictures asked for in one tick, so that a seek does not send thousands at once.
const REQUESTS_PER_TICK: usize = 32;
/// A sample this far from the estimate is a seek, not the lag of `currentTime`.
const JUMP_MS: f64 = 150.0;
/// How fast the estimate may move up, in ms per ms. The smallest sample is the best
/// one, but a real drift of the two clocks must still come through: this is 1 ms in a
/// second, far over the drift of two clocks and far under what shows.
const ESTIMATE_CREEP: f64 = 0.001;
/// Ticks without a new `currentTime` that are a stall. `currentTime` moves at least
/// once in a frame of the video (42 ms).
const STALL_TICKS: u32 = 3;
/// Over this the animations jump to the video: a seek.
const HARD_MS: f64 = 300.0;
/// Under this the animations keep the rate of the video.
const DEADBAND_MS: f64 = 5.0;
/// Seconds in which a drift is taken back.
const CORRECT_SECONDS: f64 = 2.0;
/// Largest change of the rate for a correction. 3% does not show.
const MAX_SLEW: f64 = 0.03;
/// Step of the corrected rate, so that noise does not change the rate on every tick.
const RATE_STEP: f64 = 0.005;

/// Where the video is, on the time of the document.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sync {
    pub playing: bool,
    /// Rate of the animations. A correction makes it differ from `video_rate`.
    pub rate: f64,
    pub video_rate: f64,
    /// Time of the document (ms) at the position 0 of the video, while it plays.
    pub anchor: f64,
    /// Position of the video (seconds) while it stands.
    pub paused_at: f64,
    /// Seconds between the video and the comments.
    pub offset: f64,
}

impl Sync {
    /// Video time at the time of the document `now` (ms).
    pub fn video_time(&self, now: f64) -> f64 {
        if self.playing {
            (now - self.anchor) * self.rate / 1000.0
        } else {
            self.paused_at
        }
    }

    /// Comment time at the time of the document `now` (ms).
    pub fn comment_time(&self, now: f64) -> f64 {
        self.video_time(now) - self.offset
    }

    /// The same position at `now`, from there at `rate`.
    fn with_rate(&self, now: f64, rate: f64) -> Sync {
        let position = self.video_time(now);
        Sync {
            rate,
            anchor: now - position * 1000.0 / rate,
            ..*self
        }
    }

    /// What the animations show next, for `target` (where the video is).
    fn follow(&self, target: &Sync, now: f64) -> Sync {
        let changed = self.playing != target.playing
            || self.offset != target.offset
            || self.video_rate != target.video_rate
            || (!target.playing && self.paused_at != target.paused_at);
        if changed {
            return *target;
        }
        if !target.playing {
            return *self;
        }
        // Positive: the comments are ahead of the video
        let error = (self.video_time(now) - target.video_time(now)) * 1000.0;
        if error.abs() > HARD_MS {
            return *target;
        }
        let correction = if error.abs() < DEADBAND_MS {
            0.0
        } else {
            (-error / (CORRECT_SECONDS * 1000.0)).clamp(-MAX_SLEW, MAX_SLEW)
        };
        let factor = 1.0 + (correction / RATE_STEP).round() * RATE_STEP;
        let rate = target.video_rate * factor;
        if rate == self.rate {
            *self
        } else {
            self.with_rate(now, rate)
        }
    }
}

/// The relation between the video and the time of the document.
///
/// `currentTime` lags behind the real position by up to one step of its own, and never
/// runs ahead. So of the samples `now - currentTime`, the smallest is the nearest to the
/// real relation. The estimate keeps the smallest and only creeps up
/// (`ESTIMATE_CREEP`): a window that drops its best sample makes the estimate jump.
pub struct Timing {
    estimate: Option<f64>,
    last_now: f64,
    rate: f64,
    last_current: f64,
    still: u32,
}

impl Timing {
    pub fn new() -> Self {
        Self {
            estimate: None,
            last_now: 0.0,
            rate: 1.0,
            last_current: f64::NAN,
            still: 0,
        }
    }

    /// One sample: the time of the document `now` (ms) and the state of the video.
    ///
    /// `ready_state` under `HAVE_CURRENT_DATA`, or a `currentTime` that does not move,
    /// is a stall. `HAVE_FUTURE_DATA` alone is not: it falls for a moment at the edges
    /// of the segments while the video plays on.
    pub fn sample(
        &mut self,
        now: f64,
        current: f64,
        rate: f64,
        running: bool,
        ready_state: u16,
        offset: f64,
    ) -> Sync {
        if current == self.last_current {
            self.still += 1;
        } else {
            self.still = 0;
        }
        self.last_current = current;
        let playing = running && rate > 0.0 && ready_state >= 2 && self.still < STALL_TICKS;
        let stand = Sync {
            playing: false,
            rate,
            video_rate: rate,
            anchor: 0.0,
            paused_at: current,
            offset,
        };
        if !playing {
            self.estimate = None;
            return stand;
        }
        if rate != self.rate {
            self.estimate = None;
            self.rate = rate;
        }
        let candidate = now - current * 1000.0 / rate;
        let estimate = match self.estimate {
            Some(estimate) if (candidate - estimate).abs() <= JUMP_MS => {
                candidate.min(estimate + (now - self.last_now) * ESTIMATE_CREEP)
            }
            _ => candidate,
        };
        self.estimate = Some(estimate);
        self.last_now = now;
        Sync {
            playing: true,
            anchor: estimate,
            ..stand
        }
    }
}

impl Default for Timing {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct Placed {
    start: f64,
    /// Width of the text, CSS pixels.
    width: f64,
    moving: bool,
}

enum Layer {
    /// Asked for; the worker has not answered yet.
    Pending,
    Shown {
        element: HtmlCanvasElement,
        animation: Animation,
    },
}

/// The comments on screen and of the next seconds.
pub struct Layers {
    document: Document,
    root: Element,
    duration: f64,
    debug: bool,
    generation: u32,
    placed: Vec<Placed>,
    items: HashMap<usize, Layer>,
    /// What the animations show.
    applied: Option<Sync>,
    /// CSS width that the layout used.
    width: f64,
}

impl Layers {
    pub fn new(document: Document, root: Element, duration: f64, debug: bool) -> Self {
        Self {
            document,
            root,
            duration,
            debug,
            generation: 0,
            placed: Vec::new(),
            items: HashMap::new(),
            applied: None,
            width: 0.0,
        }
    }

    pub fn placed_count(&self) -> usize {
        self.placed.len()
    }

    /// Comments on screen at the comment time `time`.
    pub fn visible_count(&self, time: f64) -> usize {
        let from = self
            .placed
            .partition_point(|item| item.start < time - self.duration);
        let to = self.placed.partition_point(|item| item.start <= time);
        to.saturating_sub(from)
    }

    /// A `protocol::LAID_OUT`: the lanes for a new size.
    pub fn laid_out(&mut self, data: &JsValue, width: f64) {
        let Some(flat) = json::get(data, "placed").map(|v| v.unchecked_into::<Float64Array>())
        else {
            return;
        };
        self.clear();
        self.generation = json::get_f64(data, "generation").unwrap_or(0.0) as u32;
        self.width = width;
        let flat = flat.to_vec();
        self.placed = flat
            .as_chunks::<{ protocol::PLACED_FIELDS }>()
            .0
            .iter()
            .map(|item| Placed {
                start: item[protocol::PLACED_START],
                width: item[protocol::PLACED_WIDTH],
                moving: item[protocol::PLACED_POSITION] == protocol::POSITION_NAKA,
            })
            .collect();
    }

    fn clear(&mut self) {
        for (_, layer) in self.items.drain() {
            if let Layer::Shown { element, animation } = layer {
                animation.cancel();
                element.remove();
            }
        }
    }

    /// A `protocol::RENDERED`: show the picture.
    pub fn rendered(&mut self, data: &JsValue) {
        let Some(bitmap) = json::get(data, "bitmap").map(|v| v.unchecked_into::<ImageBitmap>())
        else {
            return;
        };
        let generation = json::get_f64(data, "generation").map(|g| g as u32);
        let id = json::get_f64(data, "id").map(|id| id as usize);
        let wanted = generation == Some(self.generation)
            && id.is_some_and(|id| matches!(self.items.get(&id), Some(Layer::Pending)));
        let (Some(id), true) = (id, wanted) else {
            // For a layout that is gone, or for a comment that left the screen
            bitmap.close();
            return;
        };
        // What matters is not the time of one picture in the worker but a comment
        // that should already be on screen
        if self.debug
            && let (Some(sync), Some(item)) = (self.applied, self.placed.get(id))
        {
            let now = web_sys::window()
                .and_then(|window| window.performance())
                .map_or(0.0, |performance| performance.now());
            let late = sync.comment_time(now) - item.start;
            if late > 0.0 {
                log(&format!(
                    "弾幕: 絵が {late:.2} 秒遅れて届きました（{} 字 {}×{}、作るのに {:.1} ms）",
                    json::get_f64(data, "chars").unwrap_or(0.0),
                    json::get_f64(data, "pixelWidth").unwrap_or(0.0),
                    json::get_f64(data, "pixelHeight").unwrap_or(0.0),
                    json::get_f64(data, "ms").unwrap_or(0.0),
                ));
            }
        }
        match self.show(id, &bitmap, data) {
            Ok(layer) => {
                self.items.insert(id, layer);
            }
            Err(err) => {
                bitmap.close();
                self.items.remove(&id);
                log(&format!("弾幕: コメントを出せませんでした: {err:?}"));
            }
        }
    }

    fn show(&self, id: usize, bitmap: &ImageBitmap, data: &JsValue) -> Result<Layer, JsValue> {
        let item = self
            .placed
            .get(id)
            .copied()
            .ok_or_else(|| JsValue::from_str("no comment"))?;
        let pad = json::get_f64(data, "pad").unwrap_or(0.0);
        let top = json::get_f64(data, "top").unwrap_or(0.0);
        let width = json::get_f64(data, "width").unwrap_or(0.0);
        let height = json::get_f64(data, "height").unwrap_or(0.0);

        let element: HtmlCanvasElement = self.document.create_element("canvas")?.dyn_into()?;
        element.set_class_name("dt-danmaku__item");
        element.set_width(bitmap.width());
        element.set_height(bitmap.height());
        let context: ImageBitmapRenderingContext = element
            .get_context("bitmaprenderer")?
            .ok_or_else(|| JsValue::from_str("no bitmaprenderer"))?
            .dyn_into()?;
        context.transfer_from_image_bitmap(bitmap);
        let style = element.style();
        style.set_property("top", &format!("{top}px"))?;
        style.set_property("width", &format!("{width}px"))?;
        style.set_property("height", &format!("{height}px"))?;
        self.root.append_child(&element)?;

        // From off the right edge to off the left edge; a fixed comment is centred
        let (from, to) = if item.moving {
            (self.width - pad, -item.width - pad)
        } else {
            let left = (self.width - item.width) / 2.0 - pad;
            (left, left)
        };
        let frame = |x: f64| -> Result<JsValue, JsValue> {
            Ok(json::object(&[
                (
                    "transform",
                    JsValue::from_str(&format!("translateX({x}px)")),
                ),
                ("opacity", JsValue::from_f64(1.0)),
            ])?
            .into())
        };
        let keyframes = Array::of2(&frame(from)?, &frame(to)?);
        // Outside its interval the element has its own opacity 0, so it is not seen
        let options = json::object(&[
            ("duration", JsValue::from_f64(self.duration * 1000.0)),
            ("easing", JsValue::from_str("linear")),
            ("fill", JsValue::from_str("none")),
        ])?;
        let animation = animate(&element, &keyframes, &options)?;
        if let Some(sync) = &self.applied {
            place(&animation, item.start, sync);
        }
        Ok(Layer::Shown { element, animation })
    }

    /// One tick: follow the video (`target`) and ask for the next pictures.
    pub fn update(&mut self, target: Sync, now: f64, request: &dyn Fn(usize, u32)) {
        let next = self
            .applied
            .map_or(target, |applied| applied.follow(&target, now));
        if self.applied != Some(next) {
            for (id, layer) in &self.items {
                if let (Layer::Shown { animation, .. }, Some(item)) = (layer, self.placed.get(*id))
                {
                    place(animation, item.start, &next);
                }
            }
            self.applied = Some(next);
        }
        let Some(applied) = self.applied else {
            return;
        };

        let time = applied.comment_time(now);
        let from = self
            .placed
            .partition_point(|item| item.start < time - self.duration);
        let ahead = self
            .placed
            .partition_point(|item| item.start <= time + LOOKAHEAD_SECONDS);
        for (_, layer) in self.items.extract_if(|id, _| !(from..ahead).contains(id)) {
            if let Layer::Shown { element, animation } = layer {
                animation.cancel();
                element.remove();
            }
        }
        let mut budget = REQUESTS_PER_TICK;
        for id in from..ahead {
            if budget == 0 {
                break;
            }
            if let std::collections::hash_map::Entry::Vacant(slot) = self.items.entry(id) {
                slot.insert(Layer::Pending);
                request(id, self.generation);
                budget -= 1;
            }
        }
    }
}

impl Drop for Layers {
    fn drop(&mut self) {
        self.clear();
    }
}

/// `element.animate(keyframes, options)`.
///
/// web-sys has `animate` only behind `web_sys_unstable_apis`, a flag for the whole
/// build. The method is in every browser that this extension supports.
fn animate(
    element: &Element,
    keyframes: &Array,
    options: &js_sys::Object,
) -> Result<Animation, JsValue> {
    let method: js_sys::Function =
        js_sys::Reflect::get(element, &JsValue::from_str("animate"))?.dyn_into()?;
    Ok(method.call2(element, keyframes, options)?.unchecked_into())
}

/// Put one animation where the video is. `start` is the comment time of the comment.
fn place(animation: &Animation, start: f64, sync: &Sync) {
    // The comment time is the video time minus the offset
    let video_start = start + sync.offset;
    if sync.playing {
        animation.set_playback_rate(sync.rate);
        animation.set_start_time(Some(sync.anchor + video_start * 1000.0 / sync.rate));
    } else {
        let _ = animation.pause();
        animation.set_current_time(Some((sync.paused_at - video_start) * 1000.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(anchor: f64) -> Sync {
        Sync {
            playing: true,
            rate: 1.0,
            video_rate: 1.0,
            anchor,
            paused_at: 0.0,
            offset: 0.0,
        }
    }

    /// `currentTime` that moves in 40 ms steps gives the relation of the real time.
    #[test]
    fn timing_finds_the_relation_under_a_stepped_current_time() {
        let mut timing = Timing::new();
        let mut sync = None;
        for n in 0..40 {
            let now = 1000.0 + f64::from(n) * 50.0 + f64::from(n % 3) * 7.0;
            let real = (now - 1000.0) / 1000.0 + 10.0;
            let stepped = (real / 0.04).floor() * 0.04;
            sync = Some(timing.sample(now, stepped, 1.0, true, 4, 0.0));
        }
        // The real relation: the document time 1000 ms is the video time 10 s
        let anchor = sync.unwrap().anchor;
        assert!((anchor - (1000.0 - 10_000.0)).abs() < 5.0, "{anchor}");
    }

    /// The estimate must not jump up when the best sample is old.
    #[test]
    fn timing_does_not_jump_when_the_best_sample_gets_old() {
        let mut timing = Timing::new();
        // One sample without lag, then samples that lag 30 ms
        timing.sample(0.0, 0.0, 1.0, true, 4, 0.0);
        let mut last = 0.0;
        for n in 1..200 {
            let now = f64::from(n) * 50.0;
            last = timing
                .sample(now, now / 1000.0 - 0.03, 1.0, true, 4, 0.0)
                .anchor;
        }
        // 10 s at 1 ms per second is at most 10 ms, not the 30 ms of the lag
        assert!(last <= 10.0 + 1e-9, "{last}");
    }

    #[test]
    fn timing_follows_a_seek_back() {
        let mut timing = Timing::new();
        for n in 0..10 {
            let now = f64::from(n) * 50.0;
            timing.sample(now, 100.0 + now / 1000.0, 1.0, true, 4, 0.0);
        }
        // Back to 20 s: the old relation must not stay
        let sync = timing.sample(500.0, 20.0, 1.0, true, 4, 0.0);
        assert!((sync.comment_time(500.0) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn a_video_that_does_not_move_is_a_stall() {
        let mut timing = Timing::new();
        for n in 0..3 {
            assert!(
                timing
                    .sample(
                        f64::from(n) * 50.0,
                        5.0 + f64::from(n) * 0.05,
                        1.0,
                        true,
                        4,
                        0.0
                    )
                    .playing
            );
        }
        // HAVE_FUTURE_DATA falls for a moment: still playing
        assert!(timing.sample(150.0, 5.15, 1.0, true, 2, 0.0).playing);
        let mut stalled = false;
        for n in 4..8 {
            stalled |= !timing
                .sample(f64::from(n) * 50.0, 5.15, 1.0, true, 4, 0.0)
                .playing;
        }
        assert!(stalled);
    }

    /// A small drift changes the rate and keeps the position: no comment jumps.
    #[test]
    fn a_small_drift_changes_the_rate_and_not_the_position() {
        let shown = playing(0.0);
        // The video is 40 ms behind the comments
        let target = playing(40.0);
        let now = 10_000.0;
        let next = shown.follow(&target, now);
        assert!(next.rate < 1.0, "{}", next.rate);
        assert!((next.video_time(now) - shown.video_time(now)).abs() < 1e-9);
        // Under the deadband nothing changes
        assert_eq!(shown.follow(&playing(3.0), now), shown);
    }

    #[test]
    fn a_seek_and_a_pause_jump() {
        let shown = playing(0.0);
        assert_eq!(shown.follow(&playing(5000.0), 10_000.0), playing(5000.0));
        let paused = Sync {
            playing: false,
            paused_at: 3.0,
            ..shown
        };
        assert_eq!(shown.follow(&paused, 10_000.0), paused);
    }

    #[test]
    fn comment_time_uses_the_rate_and_the_offset() {
        let sync = Sync {
            playing: true,
            rate: 2.0,
            video_rate: 2.0,
            anchor: 1000.0,
            paused_at: 0.0,
            offset: 1.0,
        };
        // 1 s of the document at rate 2 is 2 s of the video, minus the offset
        assert_eq!(sync.comment_time(2000.0), 1.0);
    }
}
