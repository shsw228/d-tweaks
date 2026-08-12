//! The messages between the content script and the service worker.
//!
//! A fetch of nicovideo from a content script is blocked by CORS (measured), so the
//! service worker does it. A key name that differs between the two sides gives only an
//! `undefined`, which is hard to diagnose, so every name is here.

use js_sys::Array;
use wasm_bindgen::JsValue;

use crate::json;

/// Type of the request for comments.
pub const COMMENTS: &str = "dt/comments";

/// The page asks for the CSS because the master switch went on while it was open.
///
/// The service worker removes every registration while the extension is off, so a page
/// that loaded in that state has no CSS at all. A registration only reaches the next
/// page load, so the worker puts the CSS into the tab of the sender.
pub const ENABLE_NOW: &str = "dt/enable-now";

/// The request for comments.
pub struct CommentQuery<'a> {
    /// The dAnime partId. Also the key of the cache.
    pub part_id: &'a str,
    /// The work title, as the site writes it (before `sanitize_title`).
    pub work_title: &'a str,
    /// `第363話`, `第十四回`. `None` for a work without episodes (a film).
    pub episode_label: Option<&'a str>,
    /// The episode title (`partTitle` of `WS010105`), to confirm a candidate.
    pub episode_title: Option<&'a str>,
    /// The length. It decides when the episode number cannot.
    pub duration_seconds: Option<f64>,
    /// A video that the user gave by its address.
    ///
    /// The search is then not used, and the service worker keeps this video for the
    /// episode (`cache_keys::PIN_PREFIX`).
    pub pin_video_id: Option<&'a str>,
    /// Forget the video that the user gave, and search again.
    pub unpin: bool,
}

impl CommentQuery<'_> {
    pub fn to_js(&self) -> Result<JsValue, JsValue> {
        let obj = json::object(&[
            ("type", JsValue::from_str(COMMENTS)),
            ("partId", JsValue::from_str(self.part_id)),
            ("workTitle", JsValue::from_str(self.work_title)),
            (
                "episodeLabel",
                self.episode_label
                    .map(JsValue::from_str)
                    .unwrap_or(JsValue::NULL),
            ),
            (
                "episodeTitle",
                self.episode_title
                    .map(JsValue::from_str)
                    .unwrap_or(JsValue::NULL),
            ),
            (
                "durationSeconds",
                self.duration_seconds
                    .map(JsValue::from_f64)
                    .unwrap_or(JsValue::NULL),
            ),
            (
                "pinVideoId",
                self.pin_video_id
                    .map(JsValue::from_str)
                    .unwrap_or(JsValue::NULL),
            ),
            ("unpin", JsValue::from_bool(self.unpin)),
        ])?;
        Ok(obj.into())
    }
}

/// The reply of the service worker.
pub enum CommentReply {
    Ok {
        video_id: String,
        /// The title of the video that was selected.
        ///
        /// The match of the work title and the episode can be wrong, so the title shows
        /// which video gave the comments.
        video_title: String,
        /// The length on nicovideo. A difference to dAnime shifts the comments.
        video_seconds: Option<f64>,
        comments: Array,
        /// Did it come from the cache? For the log.
        cached: bool,
        /// Did the video come from the user and not from the search?
        ///
        /// The float player shows the button that goes back to the automatic selection
        /// only for a video that the user gave.
        pinned: bool,
    },
    /// The search ran and found no official video. A second try gives the same.
    NotFound,
    Error(String),
}

/// Build a reply with the comments (in the service worker).
pub fn reply_ok(
    video_id: &str,
    video_title: &str,
    video_seconds: Option<f64>,
    comments: &Array,
    cached: bool,
    pinned: bool,
) -> Result<JsValue, JsValue> {
    Ok(json::object(&[
        ("ok", JsValue::TRUE),
        ("videoId", JsValue::from_str(video_id)),
        ("videoTitle", JsValue::from_str(video_title)),
        (
            "videoSeconds",
            video_seconds
                .map(JsValue::from_f64)
                .unwrap_or(JsValue::NULL),
        ),
        ("comments", comments.clone().into()),
        ("cached", JsValue::from_bool(cached)),
        ("pinned", JsValue::from_bool(pinned)),
    ])?
    .into())
}

/// Build a "not found" reply (in the service worker).
pub fn reply_not_found() -> Result<JsValue, JsValue> {
    Ok(json::object(&[("ok", JsValue::FALSE), ("notFound", JsValue::TRUE)])?.into())
}

/// Build an error reply (in the service worker).
pub fn reply_error(message: &str) -> Result<JsValue, JsValue> {
    Ok(json::object(&[
        ("ok", JsValue::FALSE),
        ("error", JsValue::from_str(message)),
    ])?
    .into())
}

/// Read a reply (in the content script).
pub fn parse_reply(value: &JsValue) -> CommentReply {
    if json::get(value, "ok").and_then(|v| v.as_bool()) == Some(true) {
        return CommentReply::Ok {
            video_id: json::get_string(value, "videoId").unwrap_or_default(),
            // An old cache entry has no title, so an absent title is empty
            video_title: json::get_string(value, "videoTitle").unwrap_or_default(),
            video_seconds: json::get_f64(value, "videoSeconds"),
            comments: json::get_array(value, "comments").unwrap_or_default(),
            cached: json::get(value, "cached")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            pinned: json::get(value, "pinned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        };
    }
    if json::get(value, "notFound").and_then(|v| v.as_bool()) == Some(true) {
        return CommentReply::NotFound;
    }
    CommentReply::Error(
        json::get_string(value, "error").unwrap_or_else(|| "応答がありません".to_string()),
    )
}
