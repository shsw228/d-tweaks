//! Plays an episode in a float window on the same page.
//!
//! Takes the click on the play link of an own card (`.dt-card__main`, which goes to
//! `sc_d_pc?partId=`) and puts the player page into an `iframe` over the list. Esc
//! and a click on the background close it; there is no close button.
//!
//! Do not remove the iframe at once on a close: the site loses the play position
//! (see `close`).
//!
//! # The CSP of the site must change
//!
//! The CSP of the parent page is:
//!
//! ```text
//! frame-src <12 external hosts>; frame-ancestors https://animestore.docomo.ne.jp
//! ```
//!
//! Its own origin is not in `frame-src`, so the site forbids an iframe of its own
//! player (while `frame-ancestors` permits it). A content script is free of the CSP
//! of the page for its own fetch and its own scripts, but an iframe in the DOM of
//! the page follows the CSP of the page.
//!
//! So `extension/rules.json` (`declarativeNetRequest`) must `set` a CSP that has
//! `'self'` in `frame-src`. That ruleset weakens a security header of the site, so it is
//! declared as disabled and the service worker enables it only while this feature is on
//! (`settings::RULESET_CSP`). Without the rule the feature must not break, so the code
//! finds the block and shows another UI: with the same origin,
//! `iframe.contentDocument` is available; a block gives an error page of another
//! origin, so it is `None`.
//!
//! The `load` event cannot give that answer, because a block also sends `load` (the
//! error page loads).
//!
//! # If the video is black
//!
//! Test if a screen capture runs. With a capture, Chrome blanks protected video
//! (DRM), so the sound plays and the picture is black. This happens in an iframe, in
//! picture-in-picture and in the player of the site. A screenshot still shows the
//! picture, so a screenshot is not evidence.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, KeyboardEvent, MouseEvent};

use d_tweaks_shared::{chrome, json};

use d_tweaks_shared::text::t;

use crate::dom::{document, element};
use crate::features::{comments, controls, frame, player_meta};
use crate::{log, sleep};

const MODAL_CLASS: &str = "dt-modal";
/// Put on `<html>` to stop the scroll behind the modal.
const OPEN_CLASS: &str = "dt-modal-open";
/// Marks a modal that is closing.
///
/// It stays in the DOM while the site saves the play position, so this separates it
/// from a modal that is open.
const CLOSING_CLASS: &str = "dt-modal--closing";
/// Time between `about:blank` and the remove from the DOM.
const UNLOAD_GRACE_MS: i32 = 300;
/// The play link. Keep it the same as the class that `card_view` adds.
const PLAY_LINK_SELECTOR: &str = "a.dt-card__main";

/// Width of the side column. Keep it the same as the default in `player-modal.css`.
const SIDE_WIDTH_DEFAULT: f64 = 340.0;
/// A width below this becomes 0, which means "closed".
const SIDE_WIDTH_SNAP: f64 = 90.0;
/// Key in `storage.local` for the width.
const SIDE_WIDTH_KEY: &str = "dt:side-width";

thread_local! {
    /// The last width, so a second open on the same page uses it at once.
    ///
    /// `chrome.storage` is asynchronous and not ready at the moment of the open.
    static SIDE_WIDTH: Cell<Option<f64>> = const { Cell::new(None) };
}

/// Write the width to the panel.
fn apply_side_width(panel: &Element, width: f64) {
    let width = width.max(0.0);
    if let Some(style) = panel.dyn_ref::<HtmlElement>().map(|el| el.style()) {
        let _ = style.set_property("--dt-side-width", &format!("{width}px"));
    }
    SIDE_WIDTH.set(Some(width));
}

/// Marks the modal as large.
///
/// This is the area of the browser and not the Fullscreen API. The modal is
/// `position: fixed`, so one class is enough.
pub(crate) const MAXIMIZED_CLASS: &str = "dt-maximized";

/// Change the size. Returns the state after the change.
pub(crate) fn toggle_maximized(panel: &Element) -> bool {
    panel
        .class_list()
        .toggle(MAXIMIZED_CLASS)
        .unwrap_or_default()
}

/// Open or close the comment list. Returns the width after the change.
///
/// Used by the button of the control bar and by a double click on the handle.
pub(crate) fn toggle_side(panel: &Element) -> f64 {
    let width = if SIDE_WIDTH.get().unwrap_or(SIDE_WIDTH_DEFAULT) > 0.0 {
        0.0
    } else {
        SIDE_WIDTH_DEFAULT
    };
    apply_side_width(panel, width);
    spawn_local(async move {
        save_side_width(width).await;
    });
    width
}

/// Let the handle change the width of the side column.
///
/// The `mousemove` listener is on the document and not on the panel, so the drag
/// continues when the pointer leaves the handle.
fn install_resizer(panel: &Element, resizer: &Element) -> Result<(), JsValue> {
    // Restore the last width: at once from the copy, else wait for the storage.
    match SIDE_WIDTH.get() {
        Some(width) => apply_side_width(panel, width),
        None => {
            let panel = panel.clone();
            spawn_local(async move {
                if let Some(width) = stored_side_width().await {
                    apply_side_width(&panel, width);
                }
            });
        }
    }

    let doc = document()?;
    let dragging = Rc::new(Cell::new(false));

    {
        let dragging = Rc::clone(&dragging);
        let panel = panel.clone();
        let on_down = Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
            if event.button() != 0 {
                return;
            }
            event.prevent_default();
            dragging.set(true);
            let _ = panel.class_list().add_1("dt-resizing");
        });
        resizer.add_event_listener_with_callback("mousedown", on_down.as_ref().unchecked_ref())?;
        on_down.forget();
    }

    {
        let dragging = Rc::clone(&dragging);
        let panel = panel.clone();
        let on_move = Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
            if !dragging.get() {
                return;
            }
            let Some(bounds) = panel
                .dyn_ref::<HtmlElement>()
                .map(|el| el.get_bounding_client_rect())
            else {
                return;
            };
            // The distance from the right edge to the pointer is the width
            let width = bounds.right() - f64::from(event.client_x());
            // A limit, so the video keeps its size
            let width = width.clamp(0.0, bounds.width() * 0.8);
            apply_side_width(&panel, if width < SIDE_WIDTH_SNAP { 0.0 } else { width });
        });
        doc.add_event_listener_with_callback("mousemove", on_move.as_ref().unchecked_ref())?;
        on_move.forget();
    }

    {
        let dragging = Rc::clone(&dragging);
        let panel = panel.clone();
        let on_up = Closure::<dyn FnMut()>::new(move || {
            if !dragging.replace(false) {
                return;
            }
            let _ = panel.class_list().remove_1("dt-resizing");
            // Keep it for the next page
            if let Some(width) = SIDE_WIDTH.get() {
                spawn_local(async move {
                    save_side_width(width).await;
                });
            }
        });
        doc.add_event_listener_with_callback("mouseup", on_up.as_ref().unchecked_ref())?;
        on_up.forget();
    }

    // A double click opens or closes
    {
        let panel = panel.clone();
        let on_double = Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
            event.prevent_default();
            toggle_side(&panel);
        });
        resizer.add_event_listener_with_callback("dblclick", on_double.as_ref().unchecked_ref())?;
        on_double.forget();
    }

    Ok(())
}

async fn stored_side_width() -> Option<f64> {
    let keys = js_sys::Array::new();
    keys.push(&JsValue::from_str(SIDE_WIDTH_KEY));
    let stored: JsValue = chrome::local_get(&keys).await.ok()?.into();
    json::get_f64(&stored, SIDE_WIDTH_KEY).filter(|width| width.is_finite() && *width >= 0.0)
}

async fn save_side_width(width: f64) {
    let Ok(items) = json::object(&[(SIDE_WIDTH_KEY, JsValue::from_f64(width))]) else {
        return;
    };
    if let Err(err) = chrome::local_set(&items).await {
        log(&format!("サイド幅を保存できませんでした: {err:?}"));
    }
}

/// Close the modal and stop the playback.
///
/// Do not remove the iframe. Navigate it.
///
/// The player of the site sends the play position with `sendBeacon` in
/// `beforeunload` (both are in `player.min.js`), but a remove from the DOM does not
/// send `beforeunload`, so the last position is lost. The next open then starts at 0
/// and saves that 0 (measured: an episode watched to 744 s had a resumePoint of 0,
/// with the timestamp of the open).
///
/// A navigation does send `beforeunload`, so go to `about:blank` and remove the
/// iframe after a short time. The UI is already gone, so nobody waits.
pub fn close() -> Result<(), JsValue> {
    let doc = document()?;
    if let Some(root) = doc.document_element() {
        let _ = root.class_list().remove_1(OPEN_CLASS);
    }

    // Not a modal that is closing, else a fast second open would close the new one
    let Some(modal) = doc.query_selector(&format!(".{MODAL_CLASS}:not(.{CLOSING_CLASS})"))? else {
        return Ok(());
    };
    modal.class_list().add_1(CLOSING_CLASS)?;
    if let Some(el) = modal.dyn_ref::<HtmlElement>() {
        // Remove it from the screen at once
        el.style().set_property("display", "none")?;
    }

    if let Some(iframe) = modal
        .query_selector(".dt-modal__frame")?
        .and_then(|el| el.dyn_into::<HtmlIFrameElement>().ok())
    {
        // Stop the sound without a wait
        if let Some(video) = frame::video_in(&iframe) {
            let _ = video.pause();
        }
        iframe.set_src("about:blank");
    }

    spawn_local(async move {
        sleep(UNLOAD_GRACE_MS).await;
        modal.remove();
    });
    Ok(())
}

/// Open `url` (`sc_d_pc?partId=…`) in the float window.
///
/// With a `target`, the comments of nicovideo also arrive. `None` means that the work
/// title was not available.
fn open(url: &str, target: Option<comments::Target>) -> Result<(), JsValue> {
    let doc = document()?;
    let body = doc.body().ok_or_else(|| JsValue::from_str("no body"))?;

    // Never two modals
    close()?;

    let modal = element(&doc, "div", MODAL_CLASS)?;
    modal.set_attribute("role", "dialog")?;
    modal.set_attribute("aria-modal", "true")?;

    let backdrop = element(&doc, "div", "dt-modal__backdrop")?;
    modal.append_child(&backdrop)?;

    let panel = element(&doc, "div", "dt-modal__panel")?;

    // The head bar is over the video and not on it: the site draws the title on the
    // video and removes it after a few seconds.
    let head = element(&doc, "div", "dt-modal__head")?;
    panel.append_child(&head)?;
    // The video (16:9) and the side column (comment list, debug view)
    let stage = element(&doc, "div", "dt-modal__stage")?;
    let side = element(&doc, "div", "dt-modal__side")?;

    // UI for a CSP block. Hidden by default.
    let fallback = element(&doc, "div", "dt-modal__fallback")?;
    fallback.set_attribute("hidden", "")?;
    let message = element(&doc, "p", "dt-modal__message")?;
    message.set_text_content(Some(t("modal.csp")));
    let hint = element(&doc, "p", "dt-modal__hint")?;
    hint.set_text_content(Some(t("modal.csp.hint")));
    // This extension does not open tabs, so the link uses the same tab
    let open_link = element(&doc, "a", "dt-modal__openTab")?;
    open_link.set_attribute("href", url)?;
    open_link.set_text_content(Some(t("modal.open_tab")));
    fallback.append_child(&message)?;
    fallback.append_child(&hint)?;
    fallback.append_child(&open_link)?;
    stage.append_child(&fallback)?;

    let frame: HtmlIFrameElement = element(&doc, "iframe", "dt-modal__frame")?.dyn_into()?;
    // No `allowfullscreen`: fullscreen in `allow` is enough, and both together give
    // a Chrome warning on every load.
    frame.set_attribute("allow", "encrypted-media; fullscreen; autoplay")?;

    // Find a CSP block. `load` also arrives on a block, so test contentDocument.
    {
        let frame_for_check = frame.clone();
        let fallback = fallback.clone();
        let on_load = Closure::<dyn FnMut()>::new(move || {
            if frame_for_check.content_document().is_none() {
                log("フロート再生: サイトの CSP にブロックされました（rules.json が必要）");
                let _ = frame_for_check.set_attribute("hidden", "");
                let _ = fallback.remove_attribute("hidden");
            }
        });
        frame.set_onload(Some(on_load.as_ref().unchecked_ref()));
        on_load.forget();
    }

    stage.append_child(&frame)?;
    stage.append_child(&side)?;

    // The handle for the width.
    //
    // Not a child of the side column: that has `overflow: hidden`, so at width 0 the
    // handle is also cut and nobody can take it again. It is a child of the stage,
    // over the left edge of the side column.
    let resizer = element(&doc, "div", "dt-modal__resizer")?;
    resizer.set_attribute("title", t("modal.resizer.title"))?;
    stage.append_child(&resizer)?;
    install_resizer(&panel, &resizer)?;

    panel.append_child(&stage)?;

    // The control bar is under the video, not on it
    let bar = element(&doc, "div", "dt-modal__bar")?;
    panel.append_child(&bar)?;
    if let Err(err) = controls::install(&panel, &bar, &frame) {
        log(&format!("操作バーの初期化に失敗: {err:?}"));
    }

    // The head bar shows what is known and fills the rest when the reply arrives. It
    // uses the same `WS010105` as the comments, and the last reply is shared, so
    // there is one request.
    match &target {
        Some(target) => {
            if let Err(err) = player_meta::install(&head, &stage, &frame, target) {
                log(&format!("見出しバーの初期化に失敗: {err:?}"));
            }
        }
        // Without a partId from the card, remove the bar instead of leaving it empty
        None => head.remove(),
    }

    // Start the comments before the playback: the search needs a few hundred ms
    if let Some(target) = target {
        if let Err(err) = comments::attach(&stage, &side, &frame, target) {
            log(&format!("コメント表示の準備に失敗: {err:?}"));
        }
    } else {
        log("コメント: 作品名が取れなかったので引きません");
    }

    modal.append_child(&panel)?;
    body.append_child(&modal)?;

    // Set src after the insert, so the load handler is already there
    frame.set_src(url);

    if let Some(root) = doc.document_element() {
        root.class_list().add_1(OPEN_CLASS)?;
    }

    // A click on the background closes. There is no close button; Esc works.
    {
        let on_click = Closure::<dyn FnMut()>::new(move || {
            if let Err(err) = close() {
                log(&format!("フロート再生を閉じられませんでした: {err:?}"));
            }
        });
        backdrop.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    Ok(())
}

/// Take the clicks on the play links, and Esc.
pub fn install() -> Result<(), JsValue> {
    // The closures call the `document()` function, so do not hide that name here
    let target = document()?;

    // --- Take the clicks on a play link. Capture, to come before the anchor. ---
    let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
        // A modifier or the middle button keeps the action of the browser
        if event.ctrl_key() || event.meta_key() || event.shift_key() || event.button() != 0 {
            return;
        }
        let Some(clicked) = event.target() else {
            return;
        };
        let Ok(el) = clicked.dyn_into::<Element>() else {
            return;
        };
        let Ok(Some(link)) = el.closest(PLAY_LINK_SELECTOR) else {
            return;
        };
        let Some(href) = link.get_attribute("href") else {
            return;
        };
        // Only a play URL. A card that goes to a work page navigates.
        if !href.contains("sc_d_pc") {
            return;
        }

        event.prevent_default();
        event.stop_propagation();
        let target = document()
            .ok()
            .and_then(|doc| comments::target_from(&doc, &link));
        if let Err(err) = open(&href, target) {
            log(&format!("フロート再生を開けませんでした: {err:?}"));
        }
    });
    target.add_event_listener_with_callback_and_bool(
        "click",
        on_click.as_ref().unchecked_ref(),
        true,
    )?;
    on_click.forget();

    // --- Esc makes it small, then closes ---
    let on_key = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        if event.key() != "Escape" {
            return;
        }
        let Ok(doc) = document() else { return };
        let Ok(Some(modal)) = doc.query_selector(&format!(".{MODAL_CLASS}:not(.{CLOSING_CLASS})"))
        else {
            return;
        };
        event.prevent_default();
        event.stop_propagation();

        // A large modal goes back to its size first, like the fullscreen of the
        // browser. One Esc does not close it.
        if let Ok(Some(panel)) = modal.query_selector(".dt-modal__panel")
            && panel.class_list().contains(MAXIMIZED_CLASS)
        {
            let _ = panel.class_list().remove_1(MAXIMIZED_CLASS);
            return;
        }

        if let Err(err) = close() {
            log(&format!("フロート再生を閉じられませんでした: {err:?}"));
        }
    });
    target.add_event_listener_with_callback_and_bool(
        "keydown",
        on_key.as_ref().unchecked_ref(),
        true,
    )?;
    on_key.forget();

    Ok(())
}
