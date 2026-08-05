//! Reads inside the iframe of the float player.
//!
//! The iframe has the same origin, so `contentDocument` is available.
//!
//! `dyn_into` cannot be used here. An element of the iframe belongs to the realm of the
//! iframe, and `dyn_into` uses `instanceof`, which is false across a realm (measured:
//! `video instanceof HTMLMediaElement` is false in the parent frame and true only for
//! `iframe.contentWindow.HTMLMediaElement`).
//!
//! So `dyn_into` always gives an `Err`, and the result is a silent "not found". The
//! selector gives the type, so the code converts without a test.
//!
//! `query_selector` is not a problem: web-sys converts its result without a test. Only
//! an explicit `dyn_into` cannot cross a realm.

use wasm_bindgen::JsCast;
use web_sys::{HtmlIFrameElement, HtmlMediaElement, Url};

/// The `<video>` of the player. `None` while the page loads.
pub fn video_in(frame: &HtmlIFrameElement) -> Option<HtmlMediaElement> {
    let video = frame.content_document()?.query_selector("video").ok()??;
    Some(video.unchecked_into())
}

/// The partId that the iframe has open, or `None`.
///
/// Reads `document.URL`, which is a string, so the realm does not matter.
pub fn part_id(frame: &HtmlIFrameElement) -> Option<String> {
    let url = frame.content_document()?.url().ok()?;
    Url::new(&url).ok()?.search_params().get("partId")
}
