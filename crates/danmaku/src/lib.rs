//! The danmaku worker: the lanes and the pictures of the comments.
//!
//! The page moves each picture with a CSS animation, so the compositor of the browser
//! runs the motion on every frame of the display without any script. What costs time
//! is here, off the thread of the page and of the player: the layout (20 ms for a few
//! thousand comments) and the pictures (up to 100 ms for one long comment).
//!
//! `extension/danmaku-worker.js` starts this WASM and calls `on_message` for every
//! message.

mod layout;
mod sprites;

use std::cell::RefCell;

use js_sys::{Array, Float64Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::DedicatedWorkerGlobalScope;

use d_tweaks_shared::danmaku as protocol;
use d_tweaks_shared::json;

use crate::layout::{ASCENT_RATIO, Placed, Position, Raw, WidthCache};
use crate::sprites::Painter;

struct Engine {
    painter: Painter,
    raw: Vec<Raw>,
    placed: Vec<Placed>,
    /// Kept for the next resize.
    widths: WidthCache,
    duration: f64,
    /// Device pixels of the height at most, or 0 for the screen.
    max_height: f64,
    /// devicePixelRatio of the pictures.
    ratio: f64,
    /// Changes with every layout, so a request for an old layout is not drawn.
    generation: u32,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

fn scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn work_clock() -> f64 {
    scope()
        .performance()
        .map_or(0.0, |performance| performance.now())
}

fn post(kind: &str, fields: &[(&str, JsValue)], transfer: Option<&JsValue>) {
    let mut entries = vec![(protocol::TYPE, JsValue::from_str(kind))];
    entries.extend(fields.iter().cloned());
    let Ok(message) = json::object(&entries) else {
        return;
    };
    let _ = match transfer {
        Some(value) => scope().post_message_with_transfer(&message, &Array::of1(value)),
        None => scope().post_message(&message),
    };
}

fn log(message: &str) {
    post(
        protocol::LOG,
        &[("message", JsValue::from_str(message))],
        None,
    );
}

/// One message from the content script.
#[wasm_bindgen]
pub fn on_message(data: JsValue) {
    match json::get_string(&data, protocol::TYPE).as_deref() {
        Some(protocol::INIT) => init(&data),
        Some(protocol::SIZE) => with_engine(|engine| engine.resize(&data)),
        Some(protocol::RENDER) => with_engine(|engine| engine.render(&data)),
        _ => {}
    }
}

fn with_engine(f: impl FnOnce(&mut Engine)) {
    ENGINE.with(|engine| {
        if let Some(engine) = engine.borrow_mut().as_mut() {
            f(engine);
        }
    });
}

fn init(data: &JsValue) {
    let Some(painter) = Painter::new() else {
        log("弾幕: 絵を作る canvas がありません");
        return;
    };
    let engine = Engine {
        painter,
        raw: json::get(data, "comments")
            .map(|v| parse(&Array::from(&v)))
            .unwrap_or_default(),
        placed: Vec::new(),
        widths: WidthCache::new(),
        duration: json::get_f64(data, "duration").unwrap_or(4.0),
        max_height: json::get_f64(data, "maxHeight").unwrap_or(0.0),
        ratio: 1.0,
        generation: 0,
    };
    ENGINE.with(|slot| *slot.borrow_mut() = Some(engine));
}

/// The comments from `INIT`. The content script already removed the NG scores and
/// sorted them.
fn parse(comments: &Array) -> Vec<Raw> {
    comments
        .iter()
        .filter_map(|entry| {
            let entry = Array::from(&entry);
            let position = match entry.get(protocol::COMMENT_POSITION).as_f64() {
                Some(protocol::POSITION_UE) => Position::Ue,
                Some(protocol::POSITION_SHITA) => Position::Shita,
                _ => Position::Naka,
            };
            Some(Raw {
                start: entry.get(protocol::COMMENT_START).as_f64()?,
                text: entry.get(protocol::COMMENT_TEXT).as_string()?,
                color: entry.get(protocol::COMMENT_COLOR).as_string()?,
                scale: entry.get(protocol::COMMENT_SCALE).as_f64().unwrap_or(1.0),
                position,
            })
        })
        .collect()
}

impl Engine {
    fn resize(&mut self, data: &JsValue) {
        let width = json::get_f64(data, "width").unwrap_or(0.0);
        let height = json::get_f64(data, "height").unwrap_or(0.0);
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let device_ratio = json::get_f64(data, "ratio").unwrap_or(1.0).max(1.0);
        self.ratio = if self.max_height > 0.0 {
            device_ratio.min(self.max_height / height)
        } else {
            device_ratio
        };

        let started = work_clock();
        self.placed = layout::layout(
            self.painter.context(),
            &self.raw,
            &mut self.widths,
            width,
            height,
            self.duration,
        );
        self.generation = self.generation.wrapping_add(1);

        let flat =
            Float64Array::new_with_length((self.placed.len() * protocol::PLACED_FIELDS) as u32);
        for (index, item) in self.placed.iter().enumerate() {
            let at = (index * protocol::PLACED_FIELDS) as u32;
            flat.set_index(at + protocol::PLACED_START as u32, item.start);
            flat.set_index(at + protocol::PLACED_WIDTH as u32, item.width);
            flat.set_index(
                at + protocol::PLACED_POSITION as u32,
                match item.position {
                    Position::Naka => protocol::POSITION_NAKA,
                    Position::Ue => protocol::POSITION_UE,
                    Position::Shita => protocol::POSITION_SHITA,
                },
            );
        }
        post(
            protocol::LAID_OUT,
            &[
                ("generation", JsValue::from_f64(f64::from(self.generation))),
                ("placed", flat.clone().into()),
            ],
            Some(&flat.buffer().into()),
        );
        log(&format!(
            "弾幕: {}x{} で段割りを組み直しました（{:.1} ms）",
            width as i32,
            height as i32,
            work_clock() - started
        ));
    }

    fn render(&mut self, data: &JsValue) {
        let generation = json::get_f64(data, "generation").map(|g| g as u32);
        if generation != Some(self.generation) {
            return;
        }
        let Some(id) = json::get_f64(data, "id").map(|id| id as usize) else {
            return;
        };
        let Some(item) = self.placed.get(id) else {
            return;
        };
        let Some(source) = self.raw.get(item.index) else {
            return;
        };
        let started = work_clock();
        let Some(painted) = self.painter.paint(
            &source.text,
            &source.color,
            item.font_px,
            item.width,
            self.ratio,
        ) else {
            return;
        };
        let ms = work_clock() - started;
        let top = item.y - item.font_px * ASCENT_RATIO - painted.pad;
        post(
            protocol::RENDERED,
            &[
                ("id", JsValue::from_f64(id as f64)),
                ("generation", JsValue::from_f64(f64::from(self.generation))),
                ("bitmap", painted.bitmap.clone().into()),
                ("pad", JsValue::from_f64(painted.pad)),
                ("top", JsValue::from_f64(top)),
                ("width", JsValue::from_f64(painted.width)),
                ("height", JsValue::from_f64(painted.height)),
                ("ms", JsValue::from_f64(ms)),
                (
                    "chars",
                    JsValue::from_f64(source.text.chars().count() as f64),
                ),
                ("pixelWidth", JsValue::from_f64(f64::from(painted.pixels.0))),
                (
                    "pixelHeight",
                    JsValue::from_f64(f64::from(painted.pixels.1)),
                ),
            ],
            Some(&painted.bitmap.into()),
        );
    }
}
