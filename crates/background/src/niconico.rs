//! Gets the comments from nicovideo.
//!
//! A `fetch` of `snapshot.search.nicovideo.jp` from the page (the content script) is
//! blocked by CORS (measured: `TypeError: Failed to fetch`). The service worker has
//! `host_permissions`, so it needs no CORS. All requests to nicovideo are here.
//!
//! Three steps (every reply confirmed with curl):
//!
//! | Step | Endpoint |
//! |---|---|
//! | 1. Search | `GET snapshot.search.nicovideo.jp/api/v2/snapshot/video/contents/search` |
//! | 2. Watch data | `GET www.nicovideo.jp/watch/{videoId}?responseType=json` gives `nvComment` |
//! | 3. Comments | `POST {nvComment.server}/v1/threads` |
//!
//! Step 2 gives `server`, `threadKey` and `params` together, so
//! `nvapi.nicovideo.jp/v1/comment/keys/thread` is not necessary.
//!
//! The necessary headers are `x-frontend-id: 6`, `x-frontend-version: 0` and
//! `x-client-os-type: others`.

use js_sys::{Array, JSON, Object};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use d_tweaks_shared::json;

use crate::matching::Candidate;

const SEARCH_ENDPOINT: &str =
    "https://snapshot.search.nicovideo.jp/api/v2/snapshot/video/contents/search";
const WATCH_ENDPOINT: &str = "https://www.nicovideo.jp/watch/";
/// Identifier that the snapshot interface requires.
const SEARCH_CONTEXT: &str = "d-tweaks";

/// One comment, with only the fields that the drawing needs.
pub struct Comment {
    /// Position in the video, in milliseconds.
    pub vpos_ms: f64,
    pub body: String,
    /// Commands such as `["red", "big", "ue"]`.
    pub commands: Vec<String>,
    /// For the NG filter. A large negative value is a bad comment.
    pub score: f64,
}

fn window() -> Result<web_sys::WorkerGlobalScope, JsValue> {
    js_sys::global()
        .dyn_into::<web_sys::WorkerGlobalScope>()
        .map_err(|_| JsValue::from_str("not a worker global scope"))
}

/// A `Request` with the headers that nicovideo requires.
fn request(url: &str, method: &str, body: Option<&str>) -> Result<Request, JsValue> {
    let init = RequestInit::new();
    init.set_method(method);
    if let Some(body) = body {
        init.set_body(&JsValue::from_str(body));
    }
    let request = Request::new_with_str_and_init(url, &init)?;
    let headers = request.headers();
    headers.set("x-frontend-id", "6")?;
    headers.set("x-frontend-version", "0")?;
    headers.set("x-client-os-type", "others")?;
    if body.is_some() {
        headers.set("Content-Type", "text/plain;charset=UTF-8")?;
    }
    Ok(request)
}

/// Send the request and return the JSON.
async fn fetch_json(request: Request) -> Result<JsValue, JsValue> {
    let response: Response = JsFuture::from(window()?.fetch_with_request(&request))
        .await?
        .dyn_into()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!("HTTP {}", response.status())));
    }
    JsFuture::from(response.json()?).await
}

/// Search for videos. A maximum of 100 items.
///
/// `matching::pick` makes the selection; this only collects the candidates.
pub async fn search(query: &str) -> Result<Vec<Candidate>, JsValue> {
    let url = web_sys::Url::new(SEARCH_ENDPOINT)?;
    let params = url.search_params();
    params.set("q", query);
    params.set("targets", "title");
    params.set(
        "fields",
        "contentId,title,channelId,lengthSeconds,commentCounter,startTime",
    );
    // Sort by the comment count, not by the date.
    //
    // One request gives 100 items, so a sort by date loses the old episodes. Measured:
    // one work had 125 hits, and the first episode was not in the first 100 (videos of
    // users were between them). An episode of the official channel has many more
    // comments (85255 against a few), so the comment count puts it first.
    //
    // With a known episode number, the number is in the search words, so the result is
    // small (see `resolve` in `lib.rs`).
    params.set("_sort", "-commentCounter");
    params.set("_limit", "100");
    params.set("_context", SEARCH_CONTEXT);

    let json = fetch_json(request(&url.href(), "GET", None)?).await?;

    let status = json::path(&json, &["meta", "status"]).and_then(|v| v.as_f64());
    if status != Some(200.0) {
        let message = json::path(&json, &["meta", "errorMessage"])
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| format!("search meta.status = {status:?}"));
        return Err(JsValue::from_str(&message));
    }

    let Some(data) = json::get_array(&json, "data") else {
        return Ok(Vec::new());
    };
    let mut videos = Vec::new();
    for entry in data.iter() {
        let Some(content_id) = json::get_string(&entry, "contentId") else {
            continue;
        };
        videos.push(Candidate {
            content_id,
            title: json::get_string(&entry, "title").unwrap_or_default(),
            channel_id: json::get_f64(&entry, "channelId"),
            comment_count: json::get_f64(&entry, "commentCounter").unwrap_or(0.0),
            length_seconds: json::get_f64(&entry, "lengthSeconds"),
        });
    }
    Ok(videos)
}

/// Get `nvComment` (server, threadKey, params) from `watch?responseType=json`.
async fn nv_comment(video_id: &str) -> Result<(String, String, JsValue), JsValue> {
    let url = format!("{WATCH_ENDPOINT}{video_id}?responseType=json");
    let json = fetch_json(request(&url, "GET", None)?).await?;

    // The reply has it in data.response.comment.nvComment. Also try data.comment, in
    // case the shape changes.
    let nv = json::path(&json, &["data", "response", "comment", "nvComment"])
        .or_else(|| json::path(&json, &["data", "comment", "nvComment"]))
        .ok_or_else(|| JsValue::from_str("nvComment が見つかりません"))?;

    let server = json::get_string(&nv, "server")
        .ok_or_else(|| JsValue::from_str("nvComment.server なし"))?;
    let thread_key = json::get_string(&nv, "threadKey")
        .ok_or_else(|| JsValue::from_str("nvComment.threadKey なし"))?;
    let params =
        json::get(&nv, "params").ok_or_else(|| JsValue::from_str("nvComment.params なし"))?;
    Ok((server, thread_key, params))
}

/// Get the comments of all threads of a video.
pub async fn comments(video_id: &str) -> Result<Vec<Comment>, JsValue> {
    let (server, thread_key, params) = nv_comment(video_id).await?;

    let body = json::object(&[
        ("threadKey", JsValue::from_str(&thread_key)),
        ("params", params),
        ("additionals", Object::new().into()),
    ])?;
    let body = JSON::stringify(&body)?
        .as_string()
        .ok_or_else(|| JsValue::from_str("body の stringify に失敗"))?;

    let url = format!("{server}/v1/threads");
    let json = fetch_json(request(&url, "POST", Some(&body))?).await?;

    let Some(threads) = json::path(&json, &["data", "threads"]).map(|v| Array::from(&v)) else {
        return Ok(Vec::new());
    };

    let mut all = Vec::new();
    for thread in threads.iter() {
        let Some(list) = json::get_array(&thread, "comments") else {
            continue;
        };
        for entry in list.iter() {
            let Some(body) = json::get_string(&entry, "body") else {
                continue;
            };
            let commands = json::get_array(&entry, "commands")
                .map(|a| a.iter().filter_map(|c| c.as_string()).collect())
                .unwrap_or_default();
            all.push(Comment {
                vpos_ms: json::get_f64(&entry, "vposMs").unwrap_or(0.0),
                body,
                commands,
                score: json::get_f64(&entry, "score").unwrap_or(0.0),
            });
        }
    }
    // Sort by time; the threads arrive in another order
    all.sort_by(|a, b| a.vpos_ms.total_cmp(&b.vpos_ms));
    Ok(all)
}

/// The comments as a JS array, the form that the content script receives.
pub fn comments_to_js(comments: &[Comment]) -> Result<Array, JsValue> {
    let array = Array::new();
    for comment in comments {
        let commands = Array::new();
        for command in &comment.commands {
            commands.push(&JsValue::from_str(command));
        }
        let obj = json::object(&[
            ("vposMs", JsValue::from_f64(comment.vpos_ms)),
            ("body", JsValue::from_str(&comment.body)),
            ("commands", commands.into()),
            ("score", JsValue::from_f64(comment.score)),
        ])?;
        array.push(&obj);
    }
    Ok(array)
}
