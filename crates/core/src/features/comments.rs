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
//!
//! # The user can give the video
//!
//! The match is strict and shows nothing when it is not certain, and the search returns
//! 100 items, so an episode can get no comments although the video exists. The side
//! column therefore has a field for the address of a video (`build_pin_row`). The
//! service worker then uses that video and keeps it for the episode, and the button next
//! to the field goes back to the automatic selection.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Document, Element, HtmlIFrameElement, HtmlInputElement, KeyboardEvent, Response};

use d_tweaks_shared::messages::{CommentQuery, CommentReply, parse_reply};
use d_tweaks_shared::{chrome, json, nicovideo, settings};

use d_tweaks_shared::text::{t, t_fill};

use crate::dom::attr;
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
///
/// `player_meta` shows the same value in the head bar, so it uses this function.
/// `top_page` has its own rule: it takes the number from the `partId` when the site
/// gives a broken label, which only that page needs.
pub(crate) fn episode_label(raw: Option<String>) -> Option<String> {
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

/// What one load asks for.
enum Request {
    /// The normal path: the service worker searches, or uses a video that the user gave
    /// before.
    Auto,
    /// Use this video for the episode and keep it.
    Pin(String),
    /// Forget the video that the user gave, and search again.
    Unpin,
}

/// The state of the side column, in one structure.
///
/// The watch loop and the buttons of the input row both start a load, so the episode on
/// the screen, the running generation and the drawing cannot be separate cells: a load
/// that finishes late would then stop the drawing of a newer one.
struct Session {
    /// The partId that is on the screen. Empty before the first load.
    part_id: String,
    /// Increases with every load. A load that finishes after a newer one is discarded.
    generation: u32,
    handle: Option<danmaku::Handle>,
}

/// The row where the user gives the address of a video.
#[derive(Clone)]
struct PinRow {
    input: HtmlInputElement,
    /// Back to the automatic selection. Only visible for a video that the user gave.
    reset: Element,
}

/// Everything that a load needs. Two places start one, so this is one structure.
#[derive(Clone)]
struct Ctx {
    stage: Element,
    side: Element,
    frame: HtmlIFrameElement,
    /// The line over the list. It shows the state of the request.
    status: Element,
    pin: PinRow,
    /// What the card gave. The REST reply is better, but it can fail.
    target: Rc<Target>,
    state: Rc<RefCell<Session>>,
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
    status.set_text_content(Some(t("comments.searching")));
    side.append_child(&status)?;

    let stage = stage.clone();
    let side = side.clone();
    let frame = frame.clone();
    spawn_local(async move {
        if !settings::is_enabled("comments").await {
            status.set_text_content(Some(t("comments.off")));
            return;
        }

        // The row comes after the test: with the feature off there is nothing to give
        let (pin, load_button) = match build_pin_row(&document, &side) {
            Ok(row) => row,
            Err(err) => {
                log(&format!("動画指定の行を作れませんでした: {err:?}"));
                status.set_text_content(Some(t("comments.setup_failed")));
                return;
            }
        };

        let ctx = Ctx {
            stage,
            side,
            frame,
            status,
            pin,
            target: Rc::new(target),
            state: Rc::new(RefCell::new(Session {
                part_id: String::new(),
                generation: 0,
                handle: None,
            })),
        };
        if let Err(err) = install_pin_handlers(&ctx, &load_button) {
            log(&format!("動画指定の操作を付けられませんでした: {err:?}"));
        }

        loop {
            // The modal is closed
            if !ctx.side.is_connected() {
                if let Some(handle) = ctx.state.borrow_mut().handle.take() {
                    handle.dispose();
                }
                return;
            }

            // While the URL of the iframe is not readable (it loads, or a CSP block),
            // use the partId of the card
            let part_id = frame::part_id(&ctx.frame).unwrap_or_else(|| ctx.target.part_id.clone());
            let changed = !part_id.is_empty() && part_id != ctx.state.borrow().part_id;
            if changed {
                run(&ctx, part_id, Request::Auto).await;
            }

            sleep(WATCH_INTERVAL_MS).await;
        }
    });

    Ok(())
}

/// The episode that is on the screen: the state, else the iframe, else the card.
fn current_part_id(ctx: &Ctx) -> String {
    let known = ctx.state.borrow().part_id.clone();
    if !known.is_empty() {
        return known;
    }
    frame::part_id(&ctx.frame).unwrap_or_else(|| ctx.target.part_id.clone())
}

/// Build the row where the user gives the address of a video.
///
/// The automatic selection accepts a candidate only when it is certain (`matching::pick`
/// in the service worker), and the search returns 100 items, so an episode can find
/// nothing although the video exists. This row is the way out of that: paste the address
/// of the video and its comments arrive.
///
/// Not a `<form>`: a submit inside the page of the site navigates.
fn build_pin_row(document: &Document, side: &Element) -> Result<(PinRow, Element), JsValue> {
    let root = document.create_element("div")?;
    root.set_class_name("dt-pin");

    let input: HtmlInputElement = document.create_element("input")?.dyn_into()?;
    input.set_class_name("dt-pin__input");
    input.set_type("text");
    input.set_placeholder(t("pin.placeholder"));
    input.set_autocomplete("off");
    input.set_spellcheck(false);
    input.set_title(t("pin.title"));
    root.append_child(&input)?;

    let load = document.create_element("button")?;
    load.set_class_name("dt-pin__button");
    load.set_attribute("type", "button")?;
    load.set_text_content(Some(t("pin.load")));
    root.append_child(&load)?;

    // Only for an episode that has a video from the user
    let reset = document.create_element("button")?;
    reset.set_class_name("dt-pin__button dt-pin__reset");
    reset.set_attribute("type", "button")?;
    reset.set_attribute("title", t("pin.auto.title"))?;
    reset.set_attribute("hidden", "")?;
    reset.set_text_content(Some(t("pin.auto")));
    root.append_child(&reset)?;

    side.append_child(&root)?;
    Ok((PinRow { input, reset }, load))
}

fn install_pin_handlers(ctx: &Ctx, load_button: &Element) -> Result<(), JsValue> {
    {
        let ctx = ctx.clone();
        let on_click = Closure::<dyn FnMut()>::new(move || submit_pin(&ctx));
        load_button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    // Enter does the same as the button.
    //
    // Every other key stops here also. The float search of this extension opens with `/`
    // and the player of the site takes keys, and none of them may see what is typed into
    // this field. Escape goes through, so it still closes the float window.
    {
        let owned = ctx.clone();
        let on_key = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                return;
            }
            event.stop_propagation();
            if event.key() == "Enter" {
                event.prevent_default();
                submit_pin(&owned);
            }
        });
        ctx.pin
            .input
            .add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref())?;
        on_key.forget();
    }

    {
        let owned = ctx.clone();
        let on_click = Closure::<dyn FnMut()>::new(move || {
            let ctx = owned.clone();
            spawn_local(async move {
                let part_id = current_part_id(&ctx);
                run(&ctx, part_id, Request::Unpin).await;
            });
        });
        ctx.pin
            .reset
            .add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    Ok(())
}

/// Read the field and start a load with that video.
fn submit_pin(ctx: &Ctx) {
    let Some(video_id) = nicovideo::video_id_from(&ctx.pin.input.value()) else {
        // `run` would write over this text, so it does not run
        ctx.status.set_text_content(Some(t("pin.bad_url")));
        return;
    };
    let ctx = ctx.clone();
    spawn_local(async move {
        let part_id = current_part_id(&ctx);
        run(&ctx, part_id, Request::Pin(video_id)).await;
    });
}

/// Start one load and put its result on the screen.
///
/// The result of a load that is not the newest one is discarded: the user can give an
/// address while the search of the episode before still runs, and two drawings on one
/// canvas is not recoverable.
async fn run(ctx: &Ctx, part_id: String, request: Request) {
    let generation = {
        let mut state = ctx.state.borrow_mut();
        state.generation = state.generation.wrapping_add(1);
        state.part_id = part_id.clone();
        // Discard the episode before; the danmaku stops with its canvas
        if let Some(handle) = state.handle.take() {
            handle.dispose();
        }
        state.generation
    };
    ctx.status.set_text_content(Some(match &request {
        Request::Pin(_) => t("comments.loading"),
        _ => t("comments.searching"),
    }));

    let (text, handle, pinned) = load(ctx, &part_id, &request).await;

    // A newer load started while this one waited
    if ctx.state.borrow().generation != generation {
        if let Some(handle) = handle {
            handle.dispose();
        }
        return;
    }

    ctx.status.set_text_content(Some(&text));
    let _ = if pinned {
        ctx.pin.input.set_value("");
        ctx.pin.reset.remove_attribute("hidden")
    } else {
        ctx.pin.reset.set_attribute("hidden", "")
    };
    ctx.state.borrow_mut().handle = handle;
}

/// Get the comments of one episode and start the drawing.
///
/// Returns (status text, handle, is the video one that the user gave).
async fn load(
    ctx: &Ctx,
    part_id: &str,
    request: &Request,
) -> (String, Option<danmaku::Handle>, bool) {
    let (stage, side, frame, target) = (&ctx.stage, &ctx.side, &ctx.frame, ctx.target.as_ref());
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
        return (t("comments.no_work").to_string(), None, false);
    };

    let (pin_video_id, unpin) = match request {
        Request::Pin(video_id) => (Some(video_id.as_str()), false),
        Request::Unpin => (None, true),
        Request::Auto => (None, false),
    };
    let query = CommentQuery {
        part_id,
        work_title: &work_title,
        episode_label: episode_label.as_deref(),
        episode_title: episode_title.as_deref(),
        duration_seconds: duration,
        pin_video_id,
        unpin,
    };
    let message = match query.to_js() {
        Ok(message) => message,
        Err(err) => {
            log(&format!("コメント依頼の組み立てに失敗: {err:?}"));
            return (t("comments.failed").to_string(), None, false);
        }
    };
    log(&format!(
        "コメントを依頼: {work_title} / {} / {} / {}",
        episode_label.as_deref().unwrap_or("話数なし"),
        duration.map_or("尺なし".to_string(), |s| format!("{s}秒")),
        pin_video_id.unwrap_or(if unpin { "指定を解除" } else { "自動" })
    ));

    match chrome::send_message(&message).await {
        Ok(value) => match parse_reply(&value) {
            CommentReply::Ok {
                video_id,
                video_title,
                video_seconds,
                comments,
                cached,
                pinned,
            } => {
                let source = if cached { "キャッシュ" } else { "取得" };
                let how = if pinned { "指定" } else { "自動" };
                let count = comments.length();
                log(&format!(
                    "コメント: {count} 件 / {video_id} {video_title} / {source} / {how}"
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
                // The address that the user gave is shown, so a wrong one is visible
                let mark = if pinned { t("pin.mark") } else { "" };
                (
                    t_fill(
                        "comments.count",
                        &[
                            ("work", &work_title),
                            ("label", label),
                            ("count", &count.to_string()),
                            ("video", &video_id),
                            ("mark", mark),
                        ],
                    ),
                    handle,
                    pinned,
                )
            }
            CommentReply::NotFound => {
                log("コメント: ニコニコに該当する公式動画がありません");
                (
                    t("comments.not_found").to_string(),
                    without_comments(ctx).await,
                    false,
                )
            }
            CommentReply::Error(message) => {
                log(&format!("コメント取得に失敗: {message}"));
                (
                    t_fill("comments.error", &[("message", &message)]),
                    without_comments(ctx).await,
                    false,
                )
            }
        },
        // The service worker is not there, or it sent no reply
        Err(err) => {
            log(&format!("コメント依頼を送れませんでした: {err:?}"));
            (
                t("comments.no_reply").to_string(),
                without_comments(ctx).await,
                false,
            )
        }
    }
}

/// Start the side column for an episode that has no comments.
///
/// The debug view shows what the player does (the frame rate, the prefetch, the size of
/// the picture). That belongs to the video and not to the comments, so a failure of the
/// search must not remove it: there is no other place with those values.
///
/// Only the debug view needs this. Without it there is nothing to draw, so nothing runs.
async fn without_comments(ctx: &Ctx) -> Option<danmaku::Handle> {
    if !settings::is_enabled("debug-view").await {
        return None;
    }
    let options = danmaku::Options {
        video_id: "",
        video_title: t("comments.none"),
        video_seconds: None,
        draw_fps: settings::danmaku_fps().await,
        duration: settings::danmaku_duration().await,
        debug: true,
    };
    match danmaku::start(&ctx.stage, &ctx.side, &ctx.frame, &Array::new(), options) {
        Ok(handle) => Some(handle),
        Err(err) => {
            log(&format!("動画情報の表示を開始できませんでした: {err:?}"));
            None
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
