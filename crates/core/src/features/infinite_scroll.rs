//! Replaces the paging of a list page with an infinite scroll.
//!
//! The paging of the site is `.btnPagingNext > a[onclick="pageLink(2)"]`, and
//! `pageLink(n)` puts `selectPage=n` into `form[name=pageForm]` (`method=get`) and
//! submits it. So a GET of `?<all fields of pageForm>&selectPage=N` returns the
//! complete HTML of the next page (measured).
//!
//! The fields of `pageForm` differ between the pages (the history needs `workType`), so
//! the URL comes from those fields and not from `location.search`.
//!
//! | Page | Pages | Fields of pageForm |
//! |---|---|---|
//! | `mp_viw_pc` (continue) | 24 | `editModeFlag`, `selectPage` |
//! | `mpa_hst_pc` (history) | 30 | `workType`, `editModeFlag`, `selectPage` |
//! | `mpa_cmp_pc` (complete) | 10 | the same |
//!
//! `c_all_pc` (all works) is out of scope: the site has an infinite scroll there. A page
//! without `.paging` is also out of scope.
//!
//! The `img` of a card of the next page has `src` and `data-src` in the HTML from the
//! server, so an append is enough. A scroll also starts lazysizes (measured).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Document, DomParser, Element, HtmlInputElement, IntersectionObserver,
    IntersectionObserverEntry, IntersectionObserverInit, Node, Response, SupportedType, Url,
    Window,
};

use d_tweaks_shared::text::{t, t_fill};

use crate::{LIST_SELECTOR, log};

/// The cards of the HTML of the next page.
///
/// That HTML is a document of its own, so the ancestor scope (`LIST_SELECTOR`) is not
/// used here.
const ITEM_SELECTOR: &str = ".itemWrapper.clearfix:not(.onlySpLayout) > .itemModule";
/// Distance before the sentinel at which the next page starts to load.
///
/// A large value asks for pages that nobody sees. One page is a few screens high, so
/// half a screen is early enough.
const PRELOAD_MARGIN: &str = "600px";

struct State {
    /// The next page to get.
    next_page: u32,
    total_pages: u32,
    loading: bool,
    finished: bool,
}

/// The current page and the number of pages, from `.paging`.
///
/// The PC markup (`ul.onlyPcLayout`) has an ellipsis in it, so this reads the SP
/// markup, which has the form "1 / 24".
fn parse_pager(document: &Document) -> Option<(u32, u32)> {
    let el = document.query_selector(".paging .onlySpLayout").ok()??;
    let text = el.text_content()?;
    let nums: Vec<u32> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    match nums.as_slice() {
        [current, total, ..] => Some((*current, *total)),
        _ => None,
    }
}

/// Build the URL of `selectPage=page` from the fields of `pageForm`.
fn page_url(window: &Window, document: &Document, page: u32) -> Result<String, JsValue> {
    let location = window.location();
    let url = Url::new_with_base(&location.pathname()?, &location.origin()?)?;
    let params = url.search_params();

    let inputs = document.query_selector_all("form[name=pageForm] input")?;
    for i in 0..inputs.length() {
        let Some(node) = inputs.item(i) else { continue };
        let Ok(input) = node.dyn_into::<HtmlInputElement>() else {
            continue;
        };
        let name = input.name();
        if name.is_empty() {
            continue;
        }
        if name == "selectPage" {
            params.set(&name, &page.to_string());
        } else {
            params.set(&name, &input.value());
        }
    }
    // Also work with a pageForm that has no selectPage
    if params.get("selectPage").is_none() {
        params.set("selectPage", &page.to_string());
    }
    Ok(url.href())
}

/// Get `page`, add its cards to `container`, and return the number added.
async fn fetch_and_append(
    window: Window,
    document: Document,
    container: Element,
    page: u32,
) -> Result<u32, JsValue> {
    let url = page_url(&window, &document, page)?;

    // The fetch of a content script has the same origin, so it sends the cookies
    let response: Response = JsFuture::from(window.fetch_with_str(&url))
        .await?
        .dyn_into()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!("HTTP {}", response.status())));
    }
    let body = JsFuture::from(response.text()?)
        .await?
        .as_string()
        .ok_or_else(|| JsValue::from_str("response body is not text"))?;

    let parsed = DomParser::new()?.parse_from_string(&body, SupportedType::TextHtml)?;
    let items = parsed.query_selector_all(ITEM_SELECTOR)?;

    let mut added = 0;
    for i in 0..items.length() {
        let Some(node) = items.item(i) else { continue };
        let Ok(item) = node.dyn_into::<Element>() else {
            continue;
        };
        strip_scripts(&item)?;
        let imported = document.import_node_with_deep(&item, true)?;
        container.append_child(&imported)?;
        added += 1;
    }

    // Draw own cards for the new ones. `card_view` skips the cards that are done, so
    // the cards that are already there do not change.
    if let Err(err) =
        crate::features::card_view::render_in(&container, crate::features::card_view::Source::List)
    {
        crate::log(&format!("追加分のカード描画に失敗: {err:?}"));
    }

    Ok(added)
}

/// Remove everything that could run from a card of the next page.
///
/// `DomParser` does not run a script, but `importNode` and `appendChild` do, so a card of
/// the reply must not bring one into the page. A card of a list has no script and no
/// handler today; this only makes that a property of the code and not of the site.
fn strip_scripts(item: &Element) -> Result<(), JsValue> {
    for selector in ["script", "iframe", "object", "embed"] {
        let nodes = item.query_selector_all(selector)?;
        for index in 0..nodes.length() {
            if let Some(node) = nodes.item(index)
                && let Ok(element) = node.dyn_into::<Element>()
            {
                element.remove();
            }
        }
    }

    // `on*` attributes, on the card itself and on everything inside it
    let all = item.query_selector_all("*")?;
    for index in 0..=all.length() {
        let element = if index == 0 {
            item.clone()
        } else {
            match all.item(index - 1).and_then(|n| n.dyn_into().ok()) {
                Some(element) => element,
                None => continue,
            }
        };
        for name in element
            .get_attribute_names()
            .iter()
            .filter_map(|n| n.as_string())
        {
            if name.to_ascii_lowercase().starts_with("on") {
                let _ = element.remove_attribute(&name);
            }
        }
    }
    Ok(())
}

/// Insert an element directly after `container`.
fn insert_after(container: &Element, new_node: &Node) -> Result<(), JsValue> {
    let parent = container
        .parent_node()
        .ok_or_else(|| JsValue::from_str("container has no parent"))?;
    parent.insert_before(new_node, container.next_sibling().as_ref())?;
    Ok(())
}

/// Add the infinite scroll. `Ok(false)` for a page that is out of scope.
pub fn install() -> Result<bool, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let (window, document) = (&window, &document);

    let Some(container) = document.query_selector(LIST_SELECTOR)? else {
        return Ok(false);
    };
    let Some((current, total)) = parse_pager(document) else {
        return Ok(false);
    };
    if total <= current {
        // One page holds everything
        return Ok(false);
    }

    let status = document.create_element("div")?;
    status.set_class_name("dt-infinite-status");
    insert_after(&container, &status)?;

    let sentinel = document.create_element("div")?;
    sentinel.set_class_name("dt-infinite-sentinel");
    insert_after(&container, &sentinel)?;

    // Hook for the CSS: hide `.paging` and show the status line
    document
        .document_element()
        .and_then(|root| root.class_list().add_1("dt-infinite").map_err(|_| ()).ok());

    let state = Rc::new(RefCell::new(State {
        next_page: current + 1,
        total_pages: total,
        loading: false,
        finished: false,
    }));

    status.set_text_content(Some(&t_fill(
        "scroll.first",
        &[("total", &total.to_string())],
    )));

    let observer_slot: Rc<RefCell<Option<IntersectionObserver>>> = Rc::new(RefCell::new(None));

    let callback = Closure::<dyn FnMut(js_sys::Array, IntersectionObserver)>::new({
        let state = state.clone();
        let window = window.clone();
        let document = document.clone();
        let container = container.clone();
        let status = status.clone();
        let observer_slot = observer_slot.clone();
        let sentinel = sentinel.clone();

        move |entries: js_sys::Array, _observer: IntersectionObserver| {
            let visible = entries.iter().any(|entry| {
                entry
                    .dyn_into::<IntersectionObserverEntry>()
                    .map(|e| e.is_intersecting())
                    .unwrap_or(false)
            });
            if !visible {
                return;
            }

            let page = {
                let mut s = state.borrow_mut();
                if s.loading || s.finished {
                    return;
                }
                s.loading = true;
                s.next_page
            };

            status.set_text_content(Some(&t_fill(
                "scroll.loading",
                &[("page", &page.to_string())],
            )));

            spawn_local({
                let state = state.clone();
                let window = window.clone();
                let document = document.clone();
                let container = container.clone();
                let status = status.clone();
                let observer_slot = observer_slot.clone();
                let sentinel = sentinel.clone();

                async move {
                    let result = fetch_and_append(window, document, container, page).await;
                    let mut s = state.borrow_mut();
                    s.loading = false;

                    match result {
                        Ok(added) => {
                            log(&format!("page {page}: {added} 件追加"));
                            s.next_page = page + 1;
                            if added == 0 || s.next_page > s.total_pages {
                                s.finished = true;
                                status.set_text_content(Some(t("scroll.all")));
                                if let Some(observer) = observer_slot.borrow().as_ref() {
                                    observer.disconnect();
                                }
                            } else {
                                status.set_text_content(Some(&t_fill(
                                    "scroll.progress",
                                    &[
                                        ("page", &page.to_string()),
                                        ("total", &s.total_pages.to_string()),
                                    ],
                                )));
                                // `IntersectionObserver` reports only a change. If the
                                // sentinel stays inside the rootMargin after one page,
                                // it never reports again, so observe it again to get the
                                // current state.
                                if let Some(observer) = observer_slot.borrow().as_ref() {
                                    observer.unobserve(&sentinel);
                                    observer.observe(&sentinel);
                                }
                            }
                        }
                        Err(err) => {
                            // On a failure, stop and give the paging of the site back
                            s.finished = true;
                            log(&format!("page {page} の取得に失敗: {err:?}"));
                            status.set_text_content(Some(t("scroll.failed")));
                            if let Some(observer) = observer_slot.borrow().as_ref() {
                                observer.disconnect();
                            }
                            if let Some(root) = web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| d.document_element())
                            {
                                let _ = root.class_list().remove_1("dt-infinite");
                            }
                        }
                    }
                }
            });
        }
    });

    let options = IntersectionObserverInit::new();
    options.set_root_margin(PRELOAD_MARGIN);
    let observer =
        IntersectionObserver::new_with_options(callback.as_ref().unchecked_ref(), &options)?;
    observer.observe(&sentinel);
    *observer_slot.borrow_mut() = Some(observer);

    // The callback runs as long as the page lives
    callback.forget();

    log(&format!(
        "無限スクロールを有効化（現在 {current} / 全 {total} ページ）"
    ));
    Ok(true)
}
