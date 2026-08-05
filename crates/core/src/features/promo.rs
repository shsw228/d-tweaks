//! Closes the promotion dialog (a coupon).
//!
//! The site opens `<dialog class="popup-wrapper" open>` with `.popup-overlay` (the
//! background) and `.popup-modal` (a 480x480 image) over the page.
//!
//! CSS alone is not enough: while a `<dialog>` is open as a modal, the rest of the page
//! is inert, so `display: none` hides it but nothing below can be clicked. Measured
//! with `elementFromPoint` over the showcase of the top page:
//!
//! | State | Result |
//! |---|---|
//! | No change | `div.popup-overlay` (the dialog is over it) |
//! | `display: none` | `html` (not visible, and not clickable) |
//! | `close()` | `a.dt-top__heroHit` (clickable) |
//!
//! So the dialog is closed. `styles/no-promo.css` also hides it, but only for the time
//! until the WASM runs; the click needs this module.
//!
//! The dialog opens a moment after the load, so a `MutationObserver` on `<body>` finds
//! it.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Document, HtmlDialogElement, MutationObserver, MutationObserverInit};

use crate::log;

/// The promotion dialog of the site.
const DIALOG_SELECTOR: &str = "dialog.popup-wrapper";

/// Close what is open. Returns the number closed.
fn close_open(document: &Document) -> u32 {
    let Ok(list) = document.query_selector_all(DIALOG_SELECTOR) else {
        return 0;
    };
    let mut closed = 0;
    for index in 0..list.length() {
        let Some(node) = list.item(index) else {
            continue;
        };
        let Ok(dialog) = node.dyn_into::<HtmlDialogElement>() else {
            continue;
        };
        if !dialog.open() {
            continue;
        }
        dialog.close();
        closed += 1;
    }
    closed
}

/// Close the dialog, also one that opens later.
pub fn install() -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("no body"))?;

    let closed = close_open(&document);
    if closed > 0 {
        log(&format!("ポップアップ広告を閉じました: {closed} 件"));
    }

    // The dialog opens a moment later, so watch for it
    let watched = document.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let closed = close_open(&watched);
        if closed > 0 {
            log(&format!("ポップアップ広告を閉じました: {closed} 件"));
        }
    });
    let observer = MutationObserver::new(callback.as_ref().unchecked_ref())?;
    let options = MutationObserverInit::new();
    options.set_child_list(true);
    // The `<dialog>` is a direct child of the body, so no deep watch
    observer.observe_with_options(&body, &options)?;

    callback.forget();
    std::mem::forget(observer);
    Ok(())
}
