//! The messages between the content script and the danmaku worker.
//!
//! The worker gives each comment a lane and draws its picture. The page shows each
//! picture in its own element and moves it with a CSS animation, which the compositor
//! of the browser runs on every frame of the display without any script.

/// Lanes over the height. nicovideo divides 384px into 24px rows.
pub const LANES: usize = 16;

/// Key of the message type in every message.
pub const TYPE: &str = "type";

/// Content script → worker, one time: `comments`, `duration`, `maxHeight` (device
/// pixels of the height at most, 0 for the screen).
pub const INIT: &str = "init";
/// Content script → worker: `width`, `height` (CSS pixels), `ratio`
/// (devicePixelRatio). The worker answers with `LAID_OUT`.
pub const SIZE: &str = "size";
/// Worker → content script: `generation`, `placed` (a `Float64Array` of
/// `PLACED_FIELDS` numbers for each comment, in the order of the time).
pub const LAID_OUT: &str = "laid-out";
/// Content script → worker: `id` (index into `placed`), `generation`.
pub const RENDER: &str = "render";
/// Worker → content script: `id`, `generation`, `bitmap` (transferred), and in CSS
/// pixels `pad` (left of the text), `top`, `width`, `height` of the picture; `ms`,
/// `chars`, `pixelWidth`, `pixelHeight` for the log.
pub const RENDERED: &str = "rendered";
/// Worker → content script: `message`, for the log of the page.
pub const LOG: &str = "log";
/// Worker → content script: `message`. The worker could not start.
pub const ERROR: &str = "error";

/// One comment in `comments` is an array in this order.
pub const COMMENT_START: u32 = 0;
pub const COMMENT_TEXT: u32 = 1;
pub const COMMENT_COLOR: u32 = 2;
pub const COMMENT_SCALE: u32 = 3;
pub const COMMENT_POSITION: u32 = 4;

/// One comment in `placed`: start (seconds), width of the text (CSS pixels),
/// position.
pub const PLACED_FIELDS: usize = 3;
pub const PLACED_START: usize = 0;
pub const PLACED_WIDTH: usize = 1;
pub const PLACED_POSITION: usize = 2;

/// Values of `COMMENT_POSITION` and `PLACED_POSITION`.
pub const POSITION_NAKA: f64 = 0.0;
pub const POSITION_UE: f64 = 1.0;
pub const POSITION_SHITA: f64 = 2.0;

/// Directory of the worker in the extension, and the files in it.
pub const WORKER_LOADER: &str = "danmaku-worker.js";
pub const WORKER_JS: &str = "pkg-danmaku/danmaku.js";
pub const WORKER_WASM: &str = "pkg-danmaku/danmaku_bg.wasm";
