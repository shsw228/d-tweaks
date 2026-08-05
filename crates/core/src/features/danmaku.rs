//! Danmaku overlay for the float player. Drawn on a 2D canvas (no npm).
//!
//! The clock is `currentTime` of the `<video>` in the iframe, so seek and pause
//! follow for free. An own clock always drifts.
//!
//! All comments are known when the player opens, so the layout runs once and not
//! per frame. A frame then only draws what is visible (2500 comments stay
//! cheap), and a seek gives the same picture again. Only a size change re-runs
//! the layout.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Array, Date};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    CanvasRenderingContext2d, Document, Element, HtmlCanvasElement, HtmlElement, HtmlIFrameElement,
    HtmlMediaElement,
};

use d_tweaks_shared::{chrome, json};

use crate::features::frame;
use crate::{log, timestamp};

/// Frame counter fps until `FrameClock` has a measurement. This is the fps of
/// the video itself, not the draw rate that `start` receives.
const FALLBACK_FPS: f64 = 24.0;
/// Measurements outside this range are not a real fps.
const FPS_RANGE: std::ops::RangeInclusive<f64> = 5.0..=130.0;
/// Observation time needed before an fps estimate is used.
const FPS_SETTLE_SECONDS: f64 = 0.5;
/// Window for the measured draw rate. A short window is unstable.
const DEBUG_FPS_WINDOW_MS: f64 = 1000.0;
/// Seconds for one comment to cross the screen (nicovideo default).
const DURATION_DEFAULT: f64 = 4.0;
/// Range accepted from the setting.
const DURATION_RANGE: std::ops::RangeInclusive<f64> = 2.0..=12.0;

/// Collision test, layout and drawing must all use the same duration, so it is
/// a parameter and not a mutable global: a global cannot show that order.
fn clamp_duration(seconds: f64) -> f64 {
    if seconds.is_finite() {
        seconds.clamp(*DURATION_RANGE.start(), *DURATION_RANGE.end())
    } else {
        DURATION_DEFAULT
    }
}

/// Comments below this score are not shown (nicovideo "weak" shared NG).
const NG_SCORE: f64 = -1000.0;
/// Lanes over the height. nicovideo divides 384px into 24px rows.
const LANES: usize = 16;
/// Outline width, relative to the font size.
const STROKE_RATIO: f64 = 0.1;
/// Margin left and right of a fixed comment, relative to the width.
const FIXED_MARGIN_RATIO: f64 = 0.02;
/// Time without auto-scroll after the user scrolls the list.
const MANUAL_SCROLL_GRACE_MS: f64 = 4000.0;
/// Where the current row sits in the list (0 = top, 1 = bottom).
const LIST_ANCHOR: f64 = 0.65;
/// Row limit for the list. More rows are thinned out (see `CommentList::new`).
const LIST_MAX_ROWS: usize = 20_000;
/// Reference size for the width measurement.
const MEASURE_FONT_PX: f64 = 100.0;
/// Interval for the slow debug values (dropped frames, prefetch).
const DEBUG_SLOW_INTERVAL_MS: f64 = 250.0;
const STROKE_COLOR: &str = "rgba(0, 0, 0, 0.8)";

/// Text to its width at `MEASURE_FONT_PX`.
type WidthCache = std::collections::HashMap<String, f64>;

/// Width of the text.
///
/// `measureText` shapes the text again on every call, which is expensive for
/// thousands of comments on every resize. The width is nearly proportional to
/// the font size, so measure once at the reference size and scale. The same text
/// repeats often, so the result is kept: a resize then measures nothing.
fn measured_width(ctx: &CanvasRenderingContext2d, cache: &mut WidthCache, text: &str) -> f64 {
    if let Some(width) = cache.get(text) {
        return *width;
    }
    ctx.set_font(&font(MEASURE_FONT_PX));
    let width = ctx.measure_text(text).map(|m| m.width()).unwrap_or(0.0);
    cache.insert(text.to_string(), width);
    width
}

/// `video.requestVideoFrameCallback`, or `None` if it is absent.
///
/// web-sys has no binding, and a `#[wasm_bindgen(method)]` declaration would be
/// an inherent impl on a web-sys type (orphan rule), so use `Reflect`.
///
/// `dyn_into::<Function>()` fails here: the function belongs to the realm of the
/// iframe, so `instanceof Function` is false. `is_function()` uses `typeof`.
fn frame_callback_of(video: &HtmlMediaElement) -> Option<js_sys::Function> {
    let value =
        js_sys::Reflect::get(video, &JsValue::from_str("requestVideoFrameCallback")).ok()?;
    if !value.is_function() {
        return None;
    }
    Some(value.unchecked_into())
}

/// fps of the video and the frame number of the frame on screen.
///
/// `requestVideoFrameCallback` gives `mediaTime` and `presentedFrames` for each
/// frame that the browser presented. Both are exact, so two points give the fps
/// (measured: 19 frames / 0.792458 s = 23.976 = 24000/1001).
///
/// `currentTime * 24` drifts by one frame every 42 s on a 23.976 fps video.
struct FrameClock {
    media_time: f64,
    /// Reference point (presentedFrames, mediaTime).
    base: Option<(f64, f64)>,
    fps: Option<f64>,
    /// A callback is already requested (one per tick).
    pending: bool,
    /// Frames presented since the element was made, not the frame number of the
    /// medium.
    presented: f64,
    /// Decode time of that frame, in seconds.
    processing: f64,
    size: Option<(f64, f64)>,
}

impl FrameClock {
    fn new() -> Self {
        Self {
            media_time: 0.0,
            base: None,
            fps: None,
            pending: false,
            presented: 0.0,
            processing: 0.0,
            size: None,
        }
    }

    /// Take one frame from rVFC.
    fn observe(&mut self, media_time: f64, presented: f64) {
        self.pending = false;
        self.media_time = media_time;
        self.presented = presented;

        let Some((base_presented, base_media)) = self.base else {
            self.base = Some((presented, media_time));
            return;
        };
        let frames = presented - base_presented;
        let seconds = media_time - base_media;
        // A seek keeps presentedFrames but jumps mediaTime. An impossible ratio
        // means the reference point must be taken again.
        let measured = frames / seconds;
        if seconds <= 0.0 || frames <= 0.0 || !FPS_RANGE.contains(&measured) {
            self.base = Some((presented, media_time));
            return;
        }
        // One frame of difference is not precise enough
        if seconds >= FPS_SETTLE_SECONDS {
            self.fps = Some(snap_fps(measured));
        }
    }

    /// Frame number on screen. Falls back to the play position until rVFC has
    /// delivered a frame.
    fn frame(&self, current_time: f64) -> i64 {
        let time = if self.media_time > 0.0 {
            self.media_time
        } else {
            current_time
        };
        (time * self.fps.unwrap_or(FALLBACK_FPS)).round() as i64
    }
}

/// Snap to a standard fps so that a dropped frame does not change the number.
///
/// Take the nearest candidate. 23.976 and 24 differ by 0.1%, so "the first
/// candidate in range" turns a 24 fps video into 23.976.
fn snap_fps(measured: f64) -> f64 {
    const CANDIDATES: &[f64] = &[
        24000.0 / 1001.0,
        24.0,
        25.0,
        30000.0 / 1001.0,
        30.0,
        48000.0 / 1001.0,
        48.0,
        50.0,
        60000.0 / 1001.0,
        60.0,
    ];
    let Some(best) = CANDIDATES
        .iter()
        .copied()
        .min_by(|a, b| (a - measured).abs().total_cmp(&(b - measured).abs()))
    else {
        return measured;
    };
    if (best - measured).abs() / best < 0.01 {
        best
    } else {
        // Unknown fps (variable frame rate): keep the measurement
        measured
    }
}

/// Request the data of the next frame, one time.
///
/// `once_into_js` frees itself when it is called, so a request per tick does not
/// accumulate. A pause leaves at most one request unused.
fn request_frame(video: &HtmlMediaElement, clock: &Rc<RefCell<FrameClock>>) -> bool {
    let Some(request) = frame_callback_of(video) else {
        return false;
    };
    let clock = Rc::clone(clock);
    let callback = Closure::once_into_js(move |_now: f64, metadata: JsValue| {
        let media_time = json::get_f64(&metadata, "mediaTime").unwrap_or(0.0);
        let presented = json::get_f64(&metadata, "presentedFrames").unwrap_or(0.0);
        let mut clock = clock.borrow_mut();
        clock.observe(media_time, presented);
        clock.processing = json::get_f64(&metadata, "processingDuration").unwrap_or(0.0);
        clock.size = match (
            json::get_f64(&metadata, "width"),
            json::get_f64(&metadata, "height"),
        ) {
            (Some(width), Some(height)) => Some((width, height)),
            _ => None,
        };
    });
    request.call1(video, &callback).is_ok()
}

#[derive(Clone, Copy, PartialEq)]
enum Position {
    Naka,
    Ue,
    Shita,
}

/// A comment as received. The lane needs the canvas size, so it comes later.
struct Raw {
    /// Seconds.
    start: f64,
    text: String,
    color: String,
    /// Factor on the lane height.
    scale: f64,
    position: Position,
}

/// A comment with a lane.
///
/// The text and the colour stay in `Raw` (`index`). A copy here would rebuild
/// thousands of strings on every resize.
struct Placed {
    /// Index into `State::raw`.
    index: usize,
    start: f64,
    font_px: f64,
    width: f64,
    /// Baseline.
    y: f64,
    position: Position,
}

struct State {
    raw: Vec<Raw>,
    placed: Vec<Placed>,
    /// Canvas size that the layout used. A change re-runs the layout.
    laid_out_for: (f64, f64),
    video: Option<HtmlMediaElement>,
    has_frame_callback: bool,
    /// Ticks without a `<video>`. Without a message this state draws nothing and
    /// says nothing, which is hard to diagnose (the realm trap did exactly that),
    /// so it is reported one time.
    waited: u32,
    debug: Option<DebugView>,
    list: CommentList,
    /// Comments received, before the NG filter.
    comments_total: usize,
    video_id: String,
    video_title: String,
    video_seconds: Option<f64>,
    /// Kept for the next resize.
    widths: WidthCache,
    /// Slow values (dropped frames, prefetch). Not read every frame.
    slow_stats: (Option<(f64, f64, f64)>, Option<f64>),
    /// Time to read `slow_stats` again.
    slow_next: f64,
}

/// Colour from `commands`. Also accepts `#RRGGBB` (premium). Default is white.
fn color_of(commands: &[String]) -> String {
    for command in commands {
        let named = match command.as_str() {
            "red" => "#ff0000",
            "pink" => "#ff8080",
            "orange" => "#ffc000",
            "yellow" => "#ffff00",
            "green" => "#00ff00",
            "cyan" => "#00ffff",
            "blue" => "#0000ff",
            "purple" => "#c000ff",
            "black" => "#000000",
            "white" => "#ffffff",
            // Premium colours
            "white2" | "niconicowhite" => "#cccc99",
            "red2" | "truered" => "#cc0033",
            "pink2" => "#ff33cc",
            "orange2" | "passionorange" => "#ff6600",
            "yellow2" | "madyellow" => "#999900",
            "green2" | "elementalgreen" => "#00cc66",
            "cyan2" => "#00cccc",
            "blue2" | "marineblue" => "#3399ff",
            "purple2" | "nobleviolet" => "#6633cc",
            "black2" | "niconicoblack" => "#666666",
            _ => {
                // Direct #RRGGBB
                if let Some(hex) = command.strip_prefix('#')
                    && hex.len() == 6
                    && hex.chars().all(|c| c.is_ascii_hexdigit())
                {
                    return command.clone();
                }
                continue;
            }
        };
        return named.to_string();
    }
    "#ffffff".to_string()
}

fn position_of(commands: &[String]) -> Position {
    for command in commands {
        match command.as_str() {
            "ue" => return Position::Ue,
            "shita" => return Position::Shita,
            _ => {}
        }
    }
    Position::Naka
}

/// Font factor. On the 384px height of nicovideo: medium 24px, small 15px,
/// big 39px.
fn scale_of(commands: &[String]) -> f64 {
    for command in commands {
        match command.as_str() {
            "big" => return 1.625,
            "small" => return 0.625,
            _ => {}
        }
    }
    1.0
}

/// Read the array from the service worker. Removes the NG scores.
fn parse(comments: &Array) -> Vec<Raw> {
    let mut items = Vec::with_capacity(comments.length() as usize);
    for entry in comments.iter() {
        let Some(text) = json::get_string(&entry, "body") else {
            continue;
        };
        if json::get_f64(&entry, "score").unwrap_or(0.0) <= NG_SCORE {
            continue;
        }
        let commands: Vec<String> = json::get_array(&entry, "commands")
            .map(|a| a.iter().filter_map(|c| c.as_string()).collect())
            .unwrap_or_default();
        items.push(Raw {
            start: json::get_f64(&entry, "vposMs").unwrap_or(0.0) / 1000.0,
            // Flatten to one line, else the lane assignment breaks
            text: text.replace(['\n', '\r'], " "),
            color: color_of(&commands),
            scale: scale_of(&commands),
            position: position_of(&commands),
        });
    }
    // The drawing walks forward in time. The SW also sorts; do not depend on it.
    items.sort_by(|a, b| a.start.total_cmp(&b.start));
    items
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
fn layout(
    ctx: &CanvasRenderingContext2d,
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
            y: top + font_px * 0.85,
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

fn font(px: f64) -> String {
    // Bold gothic, like nicovideo
    format!("bold {px}px \"Hiragino Kaku Gothic ProN\", sans-serif")
}

/// Draw the comments that are visible now. Returns the count.
///
/// Set `font` and `fillStyle` only after a change: each assignment parses a
/// string, and most comments have the same size and the same white.
fn draw(
    ctx: &CanvasRenderingContext2d,
    raw: &[Raw],
    placed: &[Placed],
    time: f64,
    width: f64,
    duration: f64,
) -> usize {
    // Sorted by time, so a binary search finds the first visible comment
    let from = placed.partition_point(|item| item.start < time - duration);
    let mut drawn = 0;

    ctx.set_stroke_style_str(STROKE_COLOR);
    let mut last_font = f64::NAN;
    let mut last_color: Option<&str> = None;

    for item in &placed[from..] {
        if item.start > time {
            break;
        }
        let Some(source) = raw.get(item.index) else {
            continue;
        };
        let progress = (time - item.start) / duration;
        let x = match item.position {
            // From off the right edge to off the left edge
            Position::Naka => width - progress * (width + item.width),
            // Fixed comments are centred
            _ => (width - item.width) / 2.0,
        };

        if item.font_px != last_font {
            ctx.set_font(&font(item.font_px));
            ctx.set_line_width(item.font_px * STROKE_RATIO);
            last_font = item.font_px;
        }
        if last_color != Some(source.color.as_str()) {
            ctx.set_fill_style_str(&source.color);
            last_color = Some(source.color.as_str());
        }
        // Outline first: the other order cuts the outline into the glyphs
        let _ = ctx.stroke_text(&source.text, x, item.y);
        let _ = ctx.fill_text(&source.text, x, item.y);
        drawn += 1;
    }
    drawn
}

/// Values for the debug view.
struct Snapshot {
    /// Frame number on the fps of the video.
    frame: i64,
    source_fps: Option<f64>,
    presented: f64,
    /// Decode time, in seconds.
    processing: f64,
    frame_size: Option<(f64, f64)>,
    current_time: f64,
    duration: f64,
    paused: bool,
    playback_rate: f64,
    /// Seconds buffered after the current position.
    buffered_ahead: Option<f64>,
    ready_state: u16,
    network_state: u16,
    /// (decoded frames, dropped frames, corrupted frames)
    quality: Option<(f64, f64, f64)>,
    draw_fps_setting: f64,
    video_id: String,
    video_title: String,
    /// Length of the nicovideo video.
    source_seconds: Option<f64>,
    comments_total: usize,
    /// Comments after the NG filter.
    comments_target: usize,
    /// Comments on screen now.
    comments_now: usize,
    /// (CSS width, CSS height, devicePixelRatio)
    canvas: (f64, f64, f64),
}

const CURRENT_ROW_CLASS: &str = "dt-commentList__row--current";

/// Comment list on the right side. It follows the play position.
///
/// All rows are made at the start, in one `DocumentFragment`: the user must be
/// able to scroll back, and one insert per row gives one reflow per row. CSS
/// `content-visibility: auto` skips the layout of the rows off screen, but the
/// elements stay, so `LIST_MAX_ROWS` thins out an extreme count.
///
/// Auto-scroll stops for `MANUAL_SCROLL_GRACE_MS` after the user scrolls. A jump
/// back to the current row makes reading impossible.
struct CommentList {
    root: Element,
    rows: Vec<HtmlElement>,
    /// Time of each row, in seconds, ascending.
    times: Vec<f64>,
    current: Option<usize>,
    /// No auto-scroll until this time.
    manual_until: Rc<Cell<f64>>,
}

impl CommentList {
    fn new(document: &Document, side: &Element, items: &[Raw]) -> Result<Self, JsValue> {
        let root = document.create_element("div")?;
        root.set_class_name("dt-commentList");

        // Thin out at a constant step. Keeping only the start would stop the
        // list in the middle of the video.
        let step = items.len().div_ceil(LIST_MAX_ROWS).max(1);
        if step > 1 {
            log(&format!(
                "コメント一覧: {} 件は多いので {step} 件ごとに間引いて出します（弾幕は全件）",
                items.len()
            ));
        }

        let fragment = document.create_document_fragment();
        let capacity = items.len() / step + 1;
        let mut rows = Vec::with_capacity(capacity);
        let mut times = Vec::with_capacity(capacity);
        for item in items.iter().step_by(step) {
            let row: HtmlElement = document.create_element("div")?.dyn_into()?;
            row.set_class_name("dt-commentList__row");

            let time = document.create_element("span")?;
            time.set_class_name("dt-commentList__time");
            time.set_text_content(Some(&timestamp(item.start)));
            row.append_child(&time)?;

            let body = document.create_element("span")?;
            body.set_class_name("dt-commentList__body");
            body.set_text_content(Some(&item.text));
            row.append_child(&body)?;

            fragment.append_child(&row)?;
            rows.push(row);
            times.push(item.start);
        }
        root.append_child(&fragment)?;
        side.append_child(&root)?;

        // Hold the auto-scroll after a manual scroll
        let manual_until = Rc::new(Cell::new(0.0));
        {
            let manual_until = Rc::clone(&manual_until);
            let on_manual = Closure::<dyn FnMut()>::new(move || {
                manual_until.set(Date::now() + MANUAL_SCROLL_GRACE_MS);
            });
            for event in ["wheel", "pointerdown", "touchstart"] {
                root.add_event_listener_with_callback(event, on_manual.as_ref().unchecked_ref())?;
            }
            on_manual.forget();
        }

        Ok(Self {
            root,
            rows,
            times,
            current: None,
            manual_until,
        })
    }

    /// Move the highlight and the scroll to the play position.
    ///
    /// Called every frame. Touches the DOM only after a change of the row.
    fn sync(&mut self, time: f64) {
        // Last comment posted up to this time
        let count = self.times.partition_point(|start| *start <= time);
        let index = count.checked_sub(1);
        if index == self.current {
            return;
        }

        if let Some(previous) = self.current.and_then(|i| self.rows.get(i)) {
            let _ = previous.class_list().remove_1(CURRENT_ROW_CLASS);
        }
        self.current = index;
        let Some(row) = index.and_then(|i| self.rows.get(i)) else {
            return;
        };
        let _ = row.class_list().add_1(CURRENT_ROW_CLASS);

        if Date::now() < self.manual_until.get() {
            return;
        }
        // offsetTop is relative to `.dt-commentList` (position: relative in the
        // CSS) and not to the scroll, so it is the target directly.
        let view = f64::from(self.root.client_height());
        let target =
            f64::from(row.offset_top()) - view * LIST_ANCHOR + f64::from(row.offset_height()) / 2.0;
        self.root.set_scroll_top(target.max(0.0) as i32);
    }
}

/// Key prefix in `storage.local` for the comment offset.
const OFFSET_PREFIX: &str = "dt:offset:";
/// Offset range. Enough for a missing opening or a different start.
const OFFSET_LIMIT: f64 = 120.0;
const OFFSET_STEPS: &[f64] = &[-5.0, -1.0, 1.0, 5.0];

/// Label of the row that can wrap. A video title rarely fits on one line.
const DEBUG_WRAP_LABEL: &str = "取得元";
/// Values longer than this get a tooltip, so a cut value stays readable.
const DEBUG_TOOLTIP_MIN: usize = 18;

/// Format an offset as "+1.5s".
fn format_offset(seconds: f64) -> String {
    if seconds == 0.0 {
        "0.0s".to_string()
    } else {
        format!("{seconds:+.1}s")
    }
}

/// Controls for the comment offset.
///
/// The default is 0. The nicovideo version is often longer than the dAnime
/// version, but the difference is at the end (advertisements), so the starts
/// agree.
///
/// Do not use the difference of the lengths as the offset: the start would then
/// move in the wrong direction.
///
/// Some works have a sponsor card at the start, so a manual offset stays
/// necessary. It is kept per videoId in `storage.local`.
fn build_offset(
    document: &Document,
    side: &Element,
    video_id: &str,
    offset: &Rc<Cell<f64>>,
) -> Result<Element, JsValue> {
    let root = document.create_element("div")?;
    root.set_class_name("dt-offset");

    let label = document.create_element("span")?;
    label.set_class_name("dt-offset__label");
    label.set_text_content(Some("コメント位置"));
    label.set_attribute(
        "title",
        "既定は先頭揃え（0）。ニコニコ版の尺が長いぶんは末尾の広告なので、通常はずらさなくてよい",
    )?;
    root.append_child(&label)?;

    let value = document.create_element("span")?;
    value.set_class_name("dt-offset__value");
    value.set_attribute("title", "クリックで 0 に戻す")?;
    value.set_text_content(Some(&format_offset(0.0)));

    // Order: -5 -1 value +1 +5. The value goes before the first positive step.
    let mut value_placed = false;
    for step in OFFSET_STEPS {
        if *step > 0.0 && !value_placed {
            root.append_child(&value)?;
            value_placed = true;
        }

        let button = document.create_element("button")?;
        button.set_class_name("dt-offset__button");
        button.set_attribute("type", "button")?;
        button.set_attribute("title", &format!("コメントを {step:+.0} 秒ずらす"))?;
        button.set_text_content(Some(&format!("{step:+.0}")));

        let offset = Rc::clone(offset);
        let display = value.clone();
        let key = format!("{OFFSET_PREFIX}{video_id}");
        let step = *step;
        let on_click = Closure::<dyn FnMut()>::new(move || {
            let next = (offset.get() + step).clamp(-OFFSET_LIMIT, OFFSET_LIMIT);
            offset.set(next);
            display.set_text_content(Some(&format_offset(next)));
            let key = key.clone();
            spawn_local(async move { save_offset(&key, next).await });
        });
        button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
        root.append_child(&button)?;
    }
    if !value_placed {
        root.append_child(&value)?;
    }

    // A click on the value goes back to 0
    {
        let offset = Rc::clone(offset);
        let display = value.clone();
        let key = format!("{OFFSET_PREFIX}{video_id}");
        let on_click = Closure::<dyn FnMut()>::new(move || {
            offset.set(0.0);
            display.set_text_content(Some(&format_offset(0.0)));
            let key = key.clone();
            spawn_local(async move { save_offset(&key, 0.0).await });
        });
        value.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    side.append_child(&root)?;

    // Read the last value. The storage is asynchronous, so this arrives later.
    {
        let offset = Rc::clone(offset);
        let display = value.clone();
        let key = format!("{OFFSET_PREFIX}{video_id}");
        spawn_local(async move {
            if let Some(seconds) = stored_offset(&key).await {
                offset.set(seconds);
                display.set_text_content(Some(&format_offset(seconds)));
            }
        });
    }

    Ok(root)
}

async fn stored_offset(key: &str) -> Option<f64> {
    let keys = Array::new();
    keys.push(&JsValue::from_str(key));
    let stored: JsValue = chrome::local_get(&keys).await.ok()?.into();
    json::get_f64(&stored, key)
        .filter(|seconds| seconds.is_finite())
        .map(|seconds| seconds.clamp(-OFFSET_LIMIT, OFFSET_LIMIT))
}

async fn save_offset(key: &str, seconds: f64) {
    let Ok(items) = json::object(&[(key, JsValue::from_f64(seconds))]) else {
        return;
    };
    if let Err(err) = chrome::local_set(&items).await {
        log(&format!("コメント位置を保存できませんでした: {err:?}"));
    }
}

/// Row labels of the debug view. Keep the same order as `debug_values`.
const DEBUG_LABELS: &[&str] = &[
    "配信フレームレート",
    "コマ番号",
    "解像度",
    "再生位置",
    "先読み",
    "再生状態",
    "読み込み",
    "フレーム数",
    "復号時間",
    "弾幕描画",
    "コメント",
    "尺差",
    "描画面",
    "取得元",
    "動画",
];

/// Debug view with the values of the video.
///
/// It is DOM and not canvas: on the canvas every frame would draw it again.
///
/// The label and the value are separate elements in a CSS grid. Spaces in one
/// string do not align Japanese text, because the glyph widths differ.
///
/// The rows are built one time; a frame only replaces the value strings, and
/// only where the string changed.
struct DebugView {
    root: Element,
    /// Value elements, in the order of `DEBUG_LABELS`.
    values: Vec<Element>,
    /// Text written before, to skip the rows that did not change.
    last: Vec<String>,
    /// Window for the measured draw rate.
    window_start: f64,
    ticks: u32,
    draw_fps: Option<f64>,
}

impl DebugView {
    fn new(document: &Document, side: &Element) -> Result<Self, JsValue> {
        let root = document.create_element("div")?;
        root.set_class_name("dt-debug");

        let mut values = Vec::with_capacity(DEBUG_LABELS.len());
        for label in DEBUG_LABELS {
            let key = document.create_element("span")?;
            key.set_class_name("dt-debug__key");
            key.set_text_content(Some(label));
            root.append_child(&key)?;

            let value = document.create_element("span")?;
            value.set_class_name(if *label == DEBUG_WRAP_LABEL {
                "dt-debug__value dt-debug__value--wrap"
            } else {
                "dt-debug__value"
            });
            value.set_text_content(Some("—"));
            root.append_child(&value)?;
            values.push(value);
        }
        side.append_child(&root)?;

        Ok(Self {
            root,
            values,
            last: vec![String::new(); DEBUG_LABELS.len()],
            window_start: Date::now(),
            ticks: 0,
            draw_fps: None,
        })
    }

    fn update(&mut self, snapshot: &Snapshot) {
        // Average the draw rate over a window; one frame is too unstable
        self.ticks += 1;
        let elapsed = Date::now() - self.window_start;
        if elapsed >= DEBUG_FPS_WINDOW_MS {
            self.draw_fps = Some(f64::from(self.ticks) * 1000.0 / elapsed);
            self.ticks = 0;
            self.window_start = Date::now();
        }

        for (index, text) in debug_values(snapshot, self.draw_fps)
            .into_iter()
            .enumerate()
        {
            let Some(slot) = self.values.get(index) else {
                break;
            };
            if self.last[index] != text {
                slot.set_text_content(Some(&text));
                // A long value is cut, so give the full text on hover
                if text.chars().count() > DEBUG_TOOLTIP_MIN {
                    let _ = slot.set_attribute("title", &text);
                }
                self.last[index] = text;
            }
        }
    }
}

fn optional_seconds(value: Option<f64>) -> String {
    match value {
        Some(seconds) => format!("{seconds:.1} 秒"),
        None => "—".to_string(),
    }
}

fn ready_state_name(state: u16) -> &'static str {
    match state {
        0 => "未取得",
        1 => "メタデータのみ",
        2 => "現在位置のみ",
        3 => "先まで再生可",
        4 => "十分",
        _ => "不明",
    }
}

fn network_state_name(state: u16) -> &'static str {
    match state {
        0 => "空",
        1 => "待機",
        2 => "読み込み中",
        3 => "ソースなし",
        _ => "不明",
    }
}

/// Build the value strings in the order of `DEBUG_LABELS`.
fn debug_values(snapshot: &Snapshot, draw_fps: Option<f64>) -> Vec<String> {
    let source_fps = match snapshot.source_fps {
        Some(fps) => format!("{fps:.3} fps"),
        // No frame arrives before the play starts
        None => "測定中".to_string(),
    };
    let size = match snapshot.frame_size {
        Some((width, height)) => format!("{width:.0}×{height:.0}"),
        None => "—".to_string(),
    };
    let measured_draw = match draw_fps {
        Some(fps) => format!("{fps:.1} fps"),
        None => "測定中".to_string(),
    };
    let quality = match snapshot.quality {
        Some((total, dropped, corrupted)) => {
            format!("復号 {total:.0} / 落ち {dropped:.0} / 破損 {corrupted:.0}")
        }
        None => "—".to_string(),
    };
    let remaining = (snapshot.duration - snapshot.current_time).max(0.0);
    // Difference of the lengths. This is mostly the advertisement at the end, so
    // it must not become the offset.
    let gap = match snapshot.source_seconds {
        Some(source) if snapshot.duration.is_finite() => format!(
            "{:+.0}s（ニコ {source:.0} / 配信 {:.0}）",
            source - snapshot.duration,
            snapshot.duration
        ),
        _ => "—".to_string(),
    };
    let (canvas_width, canvas_height, ratio) = snapshot.canvas;

    vec![
        source_fps,
        format!("f{}", snapshot.frame),
        size,
        format!(
            "{:.2} / {:.2} 秒（残り {remaining:.1}）",
            snapshot.current_time, snapshot.duration
        ),
        optional_seconds(snapshot.buffered_ahead),
        format!(
            "{} / 速度 {:.2}x",
            if snapshot.paused {
                "停止中"
            } else {
                "再生中"
            },
            snapshot.playback_rate
        ),
        format!(
            "{} / {}",
            ready_state_name(snapshot.ready_state),
            network_state_name(snapshot.network_state)
        ),
        format!("表示 {:.0} / {quality}", snapshot.presented),
        format!("{:.2} ms", snapshot.processing * 1000.0),
        format!(
            "設定 {:.0} fps / 実測 {measured_draw}",
            snapshot.draw_fps_setting
        ),
        format!(
            "取得 {} / 対象 {} / 表示中 {}",
            snapshot.comments_total, snapshot.comments_target, snapshot.comments_now
        ),
        gap,
        format!("{LANES} 段 / {canvas_width:.0}×{canvas_height:.0} CSS px @{ratio:.1}x"),
        if snapshot.video_title.is_empty() {
            "—".to_string()
        } else {
            snapshot.video_title.clone()
        },
        snapshot.video_id.clone(),
    ]
}

/// Call `getVideoPlaybackQuality()`.
///
/// Also absent from web-sys, so use `Reflect`. The key for the dropped frames is
/// `droppedVideoFrames`; `droppedFrames` does not exist (measured).
fn playback_quality(video: &HtmlMediaElement) -> Option<(f64, f64, f64)> {
    let getter = js_sys::Reflect::get(video, &JsValue::from_str("getVideoPlaybackQuality")).ok()?;
    if !getter.is_function() {
        return None;
    }
    let quality = getter
        .unchecked_into::<js_sys::Function>()
        .call0(video)
        .ok()?;
    Some((
        json::get_f64(&quality, "totalVideoFrames").unwrap_or(0.0),
        json::get_f64(&quality, "droppedVideoFrames").unwrap_or(0.0),
        json::get_f64(&quality, "corruptedVideoFrames").unwrap_or(0.0),
    ))
}

/// Seconds buffered after the current position.
fn buffered_ahead(video: &HtmlMediaElement) -> Option<f64> {
    let ranges = video.buffered();
    let time = video.current_time();
    for index in 0..ranges.length() {
        let start = ranges.start(index).ok()?;
        let end = ranges.end(index).ok()?;
        if start <= time && time <= end {
            return Some(end - time);
        }
    }
    None
}

/// Settings for `start`.
pub struct Options<'a> {
    pub video_id: &'a str,
    /// Title on nicovideo. Shows which video gave the comments.
    pub video_title: &'a str,
    /// Length on nicovideo. A difference to the dAnime version shifts the
    /// comments.
    pub video_seconds: Option<f64>,
    /// Draw rate, 24 to 60.
    pub draw_fps: f64,
    /// Seconds for one comment to cross the screen, 2 to 12.
    pub duration: f64,
    pub debug: bool,
}

/// The elements that were made. Removed when the episode changes.
///
/// The tick stops itself when the canvas leaves the DOM, so a remove of the
/// elements is the complete clean-up.
pub struct Handle {
    elements: Vec<Element>,
}

impl Handle {
    pub fn dispose(&self) {
        for element in &self.elements {
            element.remove();
        }
    }
}

/// Start the danmaku and the comment list.
///
/// `stage` is over the video (for the canvas). `side` is the right column (list
/// and debug view).
pub fn start(
    stage: &Element,
    side: &Element,
    iframe: &HtmlIFrameElement,
    comments: &Array,
    options: Options<'_>,
) -> Result<Handle, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = stage
        .owner_document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    // Layout, collision test and drawing all use this value
    let duration = clamp_duration(options.duration);

    let canvas: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
    canvas.set_class_name("dt-danmaku");
    stage.append_child(&canvas)?;

    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("no 2d context"))?
        .dyn_into()?;

    let raw = parse(comments);
    let comments_total = comments.length() as usize;
    log(&format!(
        "弾幕: {comments_total} 件のうち {} 件を描画対象にしました",
        raw.len()
    ));

    // Side column from the top: offset controls, list, debug view. Only the list
    // grows to fill the height.
    let offset = Rc::new(Cell::new(0.0));
    let offset_row = build_offset(&document, side, options.video_id, &offset)?;
    let list = CommentList::new(&document, side, &raw)?;
    let debug = if options.debug {
        Some(DebugView::new(&document, side)?)
    } else {
        None
    };

    let mut elements = vec![Element::from(canvas.clone()), offset_row, list.root.clone()];
    if let Some(debug) = &debug {
        elements.push(debug.root.clone());
    }
    let handle = Handle { elements };

    let video_id = options.video_id.to_string();
    let video_title = options.video_title.to_string();
    let video_seconds = options.video_seconds;
    let fps = options.draw_fps.clamp(24.0, 60.0);
    let state = Rc::new(RefCell::new(State {
        raw,
        placed: Vec::new(),
        laid_out_for: (0.0, 0.0),
        video: None,
        has_frame_callback: false,
        waited: 0,
        debug,
        list,
        comments_total,
        video_id,
        video_title,
        video_seconds,
        widths: WidthCache::new(),
        slow_stats: (None, None),
        slow_next: 0.0,
    }));
    let clock = Rc::new(RefCell::new(FrameClock::new()));

    // The closure needs the id to stop itself
    let timer = Rc::new(Cell::new(0));

    let tick = {
        let state = Rc::clone(&state);
        let clock = Rc::clone(&clock);
        let offset = Rc::clone(&offset);
        let timer = Rc::clone(&timer);
        let canvas = canvas.clone();
        let iframe = iframe.clone();
        Closure::<dyn FnMut()>::new(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            // Closing the modal removes the canvas. Stop here.
            if !canvas.is_connected() {
                window.clear_interval_with_handle(timer.get());
                return;
            }

            let mut state = state.borrow_mut();

            if state.video.is_none() {
                state.video = frame::video_in(&iframe);
                if let Some(video) = &state.video {
                    state.has_frame_callback = frame_callback_of(video).is_some();
                    log(&format!(
                        "弾幕: プレイヤーの video を捕まえました（フレーム情報: {}）",
                        if state.has_frame_callback {
                            "取得できる"
                        } else {
                            "取得できないので 24fps 換算"
                        }
                    ));
                }
            }
            // Wait for the player to load
            let Some(video) = state.video.clone() else {
                state.waited += 1;
                // Five seconds is a defect. Report it one time, do not stay silent.
                if state.waited == (fps * 5.0) as u32 {
                    log("弾幕: プレイヤーの video が見つかりません（5 秒待機）");
                }
                return;
            };

            // Follow the CSS size; the real size gets the devicePixelRatio
            let ratio = window.device_pixel_ratio().max(1.0);
            let css_width = canvas.client_width() as f64;
            let css_height = canvas.client_height() as f64;
            if css_width <= 0.0 || css_height <= 0.0 {
                return;
            }
            let pixel_width = (css_width * ratio).round();
            let pixel_height = (css_height * ratio).round();
            if canvas.width() as f64 != pixel_width || canvas.height() as f64 != pixel_height {
                canvas.set_width(pixel_width as u32);
                canvas.set_height(pixel_height as u32);
            }

            // The layout uses CSS pixels. The drawing scales.
            if state.laid_out_for != (css_width, css_height) {
                let _ = ctx.reset_transform();
                ctx.scale(ratio, ratio).ok();
                // Take the widths out: raw and widths cannot be borrowed together
                let mut widths = std::mem::take(&mut state.widths);
                let placed = layout(
                    &ctx,
                    &state.raw,
                    &mut widths,
                    css_width,
                    css_height,
                    duration,
                );
                state.widths = widths;
                state.placed = placed;
                state.laid_out_for = (css_width, css_height);
                log(&format!(
                    "弾幕: {}x{} で段割りを組み直しました",
                    css_width as i32, css_height as i32
                ));
            }

            let time = video.current_time();
            // The two versions can start at different points
            let comment_time = time - offset.get();
            ctx.clear_rect(0.0, 0.0, css_width, css_height);
            let drawn = draw(
                &ctx,
                &state.raw,
                &state.placed,
                comment_time,
                css_width,
                duration,
            );
            state.list.sync(comment_time);

            // Read the clock first, so no borrow crosses the call below
            let (frame_number, source_fps, presented, processing, frame_size, want_frame) = {
                let mut clock = clock.borrow_mut();
                let want = state.has_frame_callback && !clock.pending;
                if want {
                    clock.pending = true;
                }
                (
                    clock.frame(time),
                    clock.fps,
                    clock.presented,
                    clock.processing,
                    clock.size,
                    want,
                )
            };
            if want_frame && !request_frame(&video, &clock) {
                // The request failed; try again on the next tick
                clock.borrow_mut().pending = false;
            }

            if state.debug.is_some() {
                // Dropped frames and prefetch change slowly
                if Date::now() >= state.slow_next {
                    state.slow_next = Date::now() + DEBUG_SLOW_INTERVAL_MS;
                    state.slow_stats = (playback_quality(&video), buffered_ahead(&video));
                }
                let (quality, buffered) = state.slow_stats;
                let snapshot = Snapshot {
                    frame: frame_number,
                    source_fps,
                    presented,
                    processing,
                    frame_size,
                    current_time: time,
                    duration: video.duration(),
                    paused: video.paused(),
                    playback_rate: video.playback_rate(),
                    buffered_ahead: buffered,
                    ready_state: video.ready_state(),
                    network_state: video.network_state(),
                    quality,
                    draw_fps_setting: fps,
                    video_id: state.video_id.clone(),
                    video_title: state.video_title.clone(),
                    source_seconds: state.video_seconds,
                    comments_total: state.comments_total,
                    comments_target: state.placed.len(),
                    comments_now: drawn,
                    canvas: (css_width, css_height, ratio),
                };
                if let Some(debug) = state.debug.as_mut() {
                    debug.update(&snapshot);
                }
            }
        })
    };

    let id = window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        (1000.0 / fps).round() as i32,
    )?;
    timer.set(id);
    // The canvas disconnect stops the tick, so the closure can outlive us
    tick.forget();

    Ok(handle)
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

    #[test]
    fn clamps_the_duration_to_a_usable_range() {
        assert_eq!(clamp_duration(6.0), 6.0);
        assert_eq!(clamp_duration(0.5), 2.0);
        assert_eq!(clamp_duration(60.0), 12.0);
        assert_eq!(clamp_duration(f64::NAN), DURATION_DEFAULT);
    }

    #[test]
    fn snaps_to_the_nearest_standard_frame_rate() {
        // Measured: 19 frames / 0.792458 s, which snaps to 24000/1001
        assert!((snap_fps(23.976) - 24000.0 / 1001.0).abs() < 1e-6);
        // 24.0 must not become 23.976 ("first candidate in range" does that)
        assert_eq!(snap_fps(24.0), 24.0);
        assert!((snap_fps(29.97) - 30000.0 / 1001.0).abs() < 1e-6);
        assert_eq!(snap_fps(30.001), 30.0);
        // A value far from every candidate (variable frame rate) stays
        assert_eq!(snap_fps(37.5), 37.5);
    }

    #[test]
    fn measures_source_fps_from_frame_metadata() {
        let mut clock = FrameClock::new();
        // The first point is only the reference
        clock.observe(10.0, 100.0);
        assert_eq!(clock.fps, None);
        // Less than 0.5 s is not used
        clock.observe(10.25, 106.0);
        assert_eq!(clock.fps, None);
        // 24 frames in 1.001 s gives 23.976 fps
        clock.observe(11.001, 124.0);
        assert!((clock.fps.unwrap() - 24000.0 / 1001.0).abs() < 1e-6);
        // Frame number is mediaTime * fps
        let expected = 11.001_f64 * 24000.0 / 1001.0;
        assert_eq!(clock.frame(0.0), expected.round() as i64);
    }

    #[test]
    fn rebases_after_a_seek() {
        let mut clock = FrameClock::new();
        clock.observe(10.0, 100.0);
        clock.observe(11.001, 124.0);
        let before = clock.fps;
        assert!(before.is_some());

        // A seek keeps presentedFrames but jumps mediaTime. That ratio would give
        // an impossible fps, so only the reference point is taken again.
        clock.observe(600.0, 125.0);
        assert_eq!(clock.fps, before, "keep the estimate, move the reference");
        // A backward seek (mediaTime decreases) also stays correct
        clock.observe(5.0, 130.0);
        assert_eq!(clock.fps, before);
    }

    #[test]
    fn debug_labels_and_values_stay_in_step() {
        let snapshot = Snapshot {
            frame: 16193,
            source_fps: Some(24000.0 / 1001.0),
            presented: 90.0,
            processing: 0.000_451_2,
            frame_size: Some((1920.0, 1080.0)),
            current_time: 675.266,
            duration: 1421.086,
            paused: false,
            playback_rate: 1.0,
            buffered_ahead: Some(45.45),
            ready_state: 4,
            network_state: 2,
            quality: Some((404.0, 0.0, 0.0)),
            draw_fps_setting: 30.0,
            video_id: "so46518613".into(),
            video_title: "作品C　第5話".into(),
            source_seconds: Some(1440.0),
            comments_total: 2514,
            comments_target: 2510,
            comments_now: 18,
            canvas: (1388.0, 781.0, 2.0),
        };
        let values = debug_values(&snapshot, Some(29.6));
        // A new row must not move a value under another label
        assert_eq!(values.len(), DEBUG_LABELS.len());
        assert_eq!(values[0], "23.976 fps");
        assert_eq!(values[1], "f16193");
        assert_eq!(values[2], "1920×1080");
        assert_eq!(values[8], "0.45 ms");
        assert_eq!(values[11], "+19s（ニコ 1440 / 配信 1421）");
        assert_eq!(values[13], "作品C　第5話");
        assert_eq!(values[14], "so46518613");
        assert!(values.iter().all(|value| !value.is_empty()));
    }

    #[test]
    fn reads_display_commands() {
        let commands = vec!["red".to_string(), "big".to_string(), "ue".to_string()];
        assert_eq!(color_of(&commands), "#ff0000");
        assert_eq!(scale_of(&commands), 1.625);
        assert!(matches!(position_of(&commands), Position::Ue));

        // No command: white, normal size, moving
        let plain: Vec<String> = Vec::new();
        assert_eq!(color_of(&plain), "#ffffff");
        assert_eq!(scale_of(&plain), 1.0);
        assert!(matches!(position_of(&plain), Position::Naka));
    }

    #[test]
    fn accepts_hex_colors_but_rejects_broken_ones() {
        assert_eq!(color_of(&["#12ab34".to_string()]), "#12ab34");
        // Too few digits, or not hexadecimal, is not a colour
        assert_eq!(color_of(&["#12ab3".to_string()]), "#ffffff");
        assert_eq!(color_of(&["#gggggg".to_string()]), "#ffffff");
        // A command that is not a colour is skipped
        assert_eq!(
            color_of(&["184".to_string(), "green".to_string()]),
            "#00ff00"
        );
    }
}
