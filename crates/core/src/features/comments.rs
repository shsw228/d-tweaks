//! Draws the nicovideo comments of the episode over the float player.
//!
//! A `fetch` of `nicovideo.jp` from a dAnime page is blocked by CORS (measured), so
//! this module sends the work title, the episode and the length to the service worker,
//! which selects the video and returns the comments. The messages are in
//! `shared::messages`.
//!
//! # The data for the match comes from a REST interface of the site
//!
//! `/animestore/rest/WS010105?viewType=5&partId=` has it in a usable form:
//!
//! ```text
//! workTitle          "作品A シーズン2"
//! partDispNumber     "第241話"  (some works give a bare number, such as "6")
//! partMeasureSecond  93
//! ```
//!
//! `partDispNumber` has no fixed form: the measurement gives `第241話` and `6`. A bare
//! number is also inside `第16話`, so `episode_label` gives it the form `第6話` first.
//!
//! The request has the same origin, so the content script can send it. This is more
//! exact than the DOM, and it gives the length. The length agrees with `lengthSeconds`
//! of nicovideo (measured: 93 seconds on both), so it decides when the episode number
//! cannot.
//!
//! The data attributes of `card_view` stay as the second source: without a signed-in
//! account the REST request can fail, and the episode number alone can be enough.

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Document, Element, HtmlIFrameElement, Response};

use d_tweaks_shared::messages::{CommentQuery, CommentReply, parse_reply};
use d_tweaks_shared::{chrome, json, settings};

use crate::features::{danmaku, frame};
use crate::{log, sleep};

/// The data attributes that `card_view` puts on a card. Keep both the same.
pub const PART_ATTR: &str = "data-dt-part";
pub const WORK_ATTR: &str = "data-dt-work";
pub const NUMBER_ATTR: &str = "data-dt-number";

/// The work title in the head of a work page.
const PAGE_TITLE_SELECTOR: &str = ".titleWrap h1";

/// REST interface for an episode. Same origin, so the content script can use it.
const PART_INFO_URL: &str = "/animestore/rest/WS010105?viewType=5&partId=";

/// Interval for the URL of the iframe. A change of the episode is a user action, so
/// this can be slow.
const WATCH_INTERVAL_MS: i32 = 500;

/// What the comment search needs.
pub struct Target {
    pub part_id: String,
    /// The work title from the DOM. The REST reply replaces it.
    pub work_title: Option<String>,
    /// `第363話`, `第十四回`.
    pub episode_label: Option<String>,
}

fn attr(el: &Element, name: &str) -> Option<String> {
    el.get_attribute(name).filter(|v| !v.trim().is_empty())
}

fn page_work_title(document: &Document) -> Option<String> {
    let el = document.query_selector(PAGE_TITLE_SELECTOR).ok()??;
    let text = el.text_content()?.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Collect what the play link that was clicked can give.
pub fn target_from(document: &Document, link: &Element) -> Option<Target> {
    let card = link.closest(".dt-card").ok()??;
    Some(Target {
        part_id: attr(&card, PART_ATTR).unwrap_or_default(),
        // An episode card of a work page has no work title (`card_view` does not put
        // it there, because the page has it). Take it from the head of the page.
        work_title: attr(&card, WORK_ATTR).or_else(|| page_work_title(document)),
        episode_label: attr(&card, NUMBER_ATTR),
    })
}

thread_local! {
    /// The `data` of the last `WS010105` reply, with its partId.
    ///
    /// Two places want the same data for one episode (the comment match and the head
    /// bar), so the last reply is kept and the request runs one time. A new episode
    /// replaces it.
    static LAST_PART: RefCell<Option<(String, JsValue)>> = const { RefCell::new(None) };
}

/// The `data` of `WS010105`, or `None`.
///
/// The DRM fields (`laUrl`, `contentUrls`, `oneTimeKey`, `viewOneTimeToken`,
/// `castContentUri`) are never read. Only the fields for the display are used.
pub(crate) async fn part_data(part_id: &str) -> Option<JsValue> {
    if let Some(cached) = LAST_PART.with_borrow(|slot| {
        slot.as_ref()
            .filter(|(id, _)| id == part_id)
            .map(|(_, value)| value.clone())
    }) {
        return Some(cached);
    }

    let window = web_sys::window()?;
    let url = format!("{PART_INFO_URL}{part_id}");
    let response: Response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !response.ok() {
        log(&format!(
            "パート情報の取得に失敗: HTTP {}",
            response.status()
        ));
        return None;
    }
    let body = JsFuture::from(response.json().ok()?).await.ok()?;
    let data = json::get(&body, "data")?;
    LAST_PART.with_borrow_mut(|slot| *slot = Some((part_id.to_string(), data.clone())));
    Some(data)
}

/// The work title, the episode and the length from `WS010105`, or `None`.
async fn part_info(part_id: &str) -> Option<PartInfo> {
    let data = part_data(part_id).await?;
    // Without a work title there is no search, so this is a failure
    let work = json::get_string(&data, "workTitle").filter(|s| !s.trim().is_empty())?;
    Some(PartInfo {
        work,
        label: episode_label(json::get_string(&data, "partDispNumber")),
        title: json::get_string(&data, "partTitle").filter(|s| !s.trim().is_empty()),
        duration: json::get_f64(&data, "partMeasureSecond").filter(|s| *s > 0.0),
    })
}

/// What `WS010105` gives for the match.
struct PartInfo {
    work: String,
    /// `第363話`. Some works have none.
    label: Option<String>,
    /// The episode title, to confirm a candidate.
    title: Option<String>,
    duration: Option<f64>,
}

/// Give the episode number a fixed form.
///
/// `partDispNumber` returns `第241話` and also `6` (measured). A bare number is not
/// usable: it is also inside `第16話`, and the head bar would show only a number. Only
/// digits become `第N話`; every other form stays (`第十四回`).
fn episode_label(raw: Option<String>) -> Option<String> {
    let text = raw?.trim().to_string();
    if text.is_empty() {
        return None;
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        Some(format!("第{text}話"))
    } else {
        Some(text)
    }
}

/// Add the comments to the float window.
///
/// "Next episode" in the player navigates the iframe to another partId. There is no
/// event for that, so this watches the URL of the iframe every `WATCH_INTERVAL_MS` and
/// starts again on a change. A change of the episode is a user action, so that delay is
/// not visible.
pub fn attach(
    stage: &Element,
    side: &Element,
    frame: &HtmlIFrameElement,
    target: Target,
) -> Result<(), JsValue> {
    let document = side
        .owner_document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let status = document.create_element("div")?;
    status.set_class_name("dt-comments");
    status.set_text_content(Some("コメントを検索中…"));
    side.append_child(&status)?;

    let stage = stage.clone();
    let side = side.clone();
    let frame = frame.clone();
    spawn_local(async move {
        if !settings::is_enabled("comments").await {
            status.set_text_content(Some("コメント: 設定で無効"));
            return;
        }

        // Start with the partId of the card, then follow the URL of the iframe
        let mut current = String::new();
        let mut session: Option<danmaku::Handle> = None;

        loop {
            // The modal is closed
            if !side.is_connected() {
                if let Some(handle) = &session {
                    handle.dispose();
                }
                return;
            }

            // While the URL of the iframe is not readable (it loads, or a CSP block),
            // use the partId of the card
            let part_id = frame::part_id(&frame).unwrap_or_else(|| target.part_id.clone());
            if !part_id.is_empty() && part_id != current {
                current = part_id.clone();
                // Discard the episode before; the danmaku stops with its canvas
                if let Some(handle) = session.take() {
                    handle.dispose();
                }
                status.set_text_content(Some("コメントを検索中…"));

                let (text, handle) = load(&stage, &side, &frame, &part_id, &target).await;
                status.set_text_content(Some(&text));
                session = handle;
            }

            sleep(WATCH_INTERVAL_MS).await;
        }
    });

    Ok(())
}

/// Get the comments of one episode and start the drawing. Returns (status text, handle).
async fn load(
    stage: &Element,
    side: &Element,
    frame: &HtmlIFrameElement,
    part_id: &str,
    target: &Target,
) -> (String, Option<danmaku::Handle>) {
    // The REST reply wins. Without it, use what the DOM gave.
    let info = part_info(part_id).await;
    let (work_title, episode_label, episode_title, duration) = match info {
        Some(info) => (
            Some(info.work),
            info.label.or_else(|| target.episode_label.clone()),
            info.title,
            info.duration,
        ),
        None => (
            target.work_title.clone(),
            target.episode_label.clone(),
            None,
            None,
        ),
    };

    let Some(work_title) = work_title else {
        log("コメント: 作品名が取れないので引きません");
        return ("コメント: 作品名が取れませんでした".to_string(), None);
    };

    let query = CommentQuery {
        part_id,
        work_title: &work_title,
        episode_label: episode_label.as_deref(),
        episode_title: episode_title.as_deref(),
        duration_seconds: duration,
    };
    let message = match query.to_js() {
        Ok(message) => message,
        Err(err) => {
            log(&format!("コメント依頼の組み立てに失敗: {err:?}"));
            return ("コメントを取得できませんでした".to_string(), None);
        }
    };
    log(&format!(
        "コメントを依頼: {work_title} / {} / {}",
        episode_label.as_deref().unwrap_or("話数なし"),
        duration.map_or("尺なし".to_string(), |s| format!("{s}秒"))
    ));

    match chrome::send_message(&message).await {
        Ok(value) => match parse_reply(&value) {
            CommentReply::Ok {
                video_id,
                video_title,
                video_seconds,
                comments,
                cached,
            } => {
                let source = if cached { "キャッシュ" } else { "取得" };
                let count = comments.length();
                log(&format!(
                    "コメント: {count} 件 / {video_id} {video_title} / {source}"
                ));
                let options = danmaku::Options {
                    video_id: &video_id,
                    video_title: &video_title,
                    video_seconds,
                    draw_fps: settings::danmaku_fps().await,
                    duration: settings::danmaku_duration().await,
                    debug: settings::is_enabled("debug-view").await,
                };
                let handle = match danmaku::start(stage, side, frame, &comments, options) {
                    Ok(handle) => Some(handle),
                    Err(err) => {
                        log(&format!("弾幕の描画を開始できませんでした: {err:?}"));
                        None
                    }
                };
                let label = episode_label.as_deref().unwrap_or("");
                (
                    format!("{work_title} {label}／コメント {count} 件（{video_id}）"),
                    handle,
                )
            }
            CommentReply::NotFound => {
                log("コメント: ニコニコに該当する公式動画がありません");
                (
                    "ニコニコに該当する公式配信が見つかりません".to_string(),
                    None,
                )
            }
            CommentReply::Error(message) => {
                log(&format!("コメント取得に失敗: {message}"));
                (format!("コメントを取得できません: {message}"), None)
            }
        },
        // The service worker is not there, or it sent no reply
        Err(err) => {
            log(&format!("コメント依頼を送れませんでした: {err:?}"));
            (
                "コメントを取得できません（拡張の再読み込みが必要かもしれません）".to_string(),
                None,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::episode_label;

    #[test]
    fn normalizes_bare_episode_numbers() {
        // Measured: some works give a bare number
        assert_eq!(episode_label(Some("6".into())).as_deref(), Some("第6話"));
        // A complete form stays
        assert_eq!(
            episode_label(Some("第241話".into())).as_deref(),
            Some("第241話")
        );
        assert_eq!(
            episode_label(Some("第十四回".into())).as_deref(),
            Some("第十四回")
        );
        assert_eq!(episode_label(Some("  ".into())), None);
        assert_eq!(episode_label(None), None);
    }
}
