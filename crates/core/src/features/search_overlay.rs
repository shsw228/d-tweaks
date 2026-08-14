//! Replaces the header "さがす" link with a float search that does not leave the
//! page.
//!
//! An iframe of the search page of the site is not an option: the HTML from the
//! server has no results in it (measured: zero `.itemModule`; JS inserts the
//! cards later). So this module asks the REST interface and draws own cards.
//!
//! ```text
//! GET /animestore/rest/WS000105
//!   ?searchKey=<words>
//!   &vodTypeList=<svod (no rental) | svod_tvod (all)>
//!   &start=<first item, from 0>
//!   &length=<count, maximum 300>
//!   &sortKey=<sort>
//!   &mainKeyVisualSize=<image size>
//! ```
//!
//! The names come from `js/cms/list.js` (`createUrlFunc`) of the site. The
//! request has the same origin, so the `fetch` sends the cookies and the result
//! includes the state of the account ("favorite", "viewed").
//!
//! The reply gives `maxCount` (all hits) and `count` (items in this reply).
//!
//! `offset` and `from` do nothing. Only `start` (from 0) moves the window; the
//! site itself sends 300 items at a time (`firstloadFunc`). Measured up to
//! `start=7648` of 7649 items.
//!
//! `sortKey` uses the same values as `#listsort` of the search page. All six
//! work (see `SORTS`).
//!
//! # Keep the number of requests low
//!
//! This UI asks while the user types, so without a limit it sends much more than
//! the search page of the site. There are four limits:
//!
//! 1. Ask 400 ms after the last key (`DEBOUNCE_MS`).
//! 2. Ask nothing below two characters (`MIN_QUERY_CHARS`).
//! 3. Keep the replies and use them again (`CACHE`).
//! 4. Abort a request when the words change (`abort_pending`). To receive and
//!    then discard does not reduce the work of the server.
//!
//! The images stay lazy, so only the visible ones are loaded.

use std::cell::{Cell, RefCell};

use js_sys::Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AbortController, Document, Element, HtmlElement, HtmlInputElement, HtmlSelectElement,
    InputEvent, IntersectionObserver, IntersectionObserverInit, KeyboardEvent, MouseEvent,
    RequestInit, Response, UrlSearchParams,
};

use d_tweaks_shared::{json, settings};

use d_tweaks_shared::text::{t, t_fill};

use crate::dom::{document, element};
use crate::features::card_view::{self, Badge, Card};
use crate::log;

/// Outer element of the float search. Also the test for "is it open".
const OVERLAY_CLASS: &str = "dt-search";
/// Put on `<html>` to stop the scroll behind the float.
const OPEN_CLASS: &str = "dt-search-open";
/// The "さがす" links in the header and in the breadcrumbs.
const SEARCH_LINK_SELECTOR: &str = "a[href*=\"CF/search_index\"]";

const ENDPOINT: &str = "/animestore/rest/WS000105";
/// Subscription and rental (the value of the search form of the site).
const VOD_ALL: &str = "svod_tvod";
/// Subscription only. The server removes the rentals.
///
/// Measured on one word: 44 items with `svod_tvod` (four of them `vodType: tvod`)
/// and 40 with `svod`. Exactly the rentals are absent, so `maxCount` stays
/// correct. A filter here would break the paging and the count.
const VOD_SVOD: &str = "svod";
/// Shares the key with the settings (`search-no-rental`), so the toggle in the
/// float and the settings page write the same value.
const NO_RENTAL_KEY: &str = d_tweaks_shared::settings::SEARCH_NO_RENTAL;
const NO_RENTAL_DEFAULT: bool = true;
/// Default image size. 1 = 640x360.
///
/// This follows the resolution setting (`thumb-size`). A large image here means
/// tens of new images for every word, so the setting is not ignored.
const KEY_VISUAL_DEFAULT: &str = "1";
/// The size that the search page of the site uses (288x162).
const KEY_VISUAL_SITE: &str = "2";
/// Items per request. The interface accepts 300, but this UI asks often.
const PAGE_SIZE: u32 = 60;
/// Time after the last key. The main limit on the number of requests.
const DEBOUNCE_MS: i32 = 400;
/// Minimum length of the words.
///
/// One character gives almost no information (7649 items for "あ") and every
/// user types one, so it would be the most wasteful request.
const MIN_QUERY_CHARS: usize = 2;
/// Replies to keep, so that a correction of the words asks nothing.
const CACHE_LIMIT: usize = 64;
/// The first line of the float, before anything is typed.
fn initial_hint() -> &'static str {
    t("search.hint")
}

/// Sort values. The same as `#listsort` of the search page (all measured).
/// The default of the site is by relevance. `5` is absent on the site also.
const SORTS: &[(&str, &str)] = &[
    ("4", "opt.sort.relevance"),
    ("1", "opt.sort.plays"),
    ("7", "opt.sort.favorites"),
    ("3", "opt.sort.release"),
    ("6", "opt.sort.year"),
    ("2", "opt.sort.kana"),
];
const SORT_DEFAULT: &str = "4";

/// One page of results: (all hits, cards).
type Page = (u32, Vec<Card>);

/// Distance before the sentinel at which the prefetch starts.
///
/// At the bottom edge the user must wait. A large value asks for items that
/// nobody sees. A request needs about 200 ms, so half a screen is enough.
const PREFETCH_MARGIN: &str = "400px";

/// State of the search.
///
/// The four values are one structure because separate cells make it possible to
/// increase the generation and to forget to clear the count. Only `begin_search`
/// and `update_progress` write to it.
#[derive(Clone, Copy)]
struct Progress {
    /// Number of searches. Used to discard an old reply.
    ///
    /// A fast typist can get the reply for "あ" after the reply for "あお". The
    /// generation at the request is compared with the current one. It also stops
    /// a prefetch that runs while the words change.
    generation: u32,
    /// Cards on screen now. This is the next `start`.
    loaded: u32,
    /// All hits (`maxCount`).
    total: u32,
    /// A prefetch runs. Stops a second one.
    loading: bool,
}

impl Progress {
    const fn new() -> Self {
        Self {
            generation: 0,
            loaded: 0,
            total: 0,
            loading: false,
        }
    }

    /// Are there more items to get?
    fn has_more(&self) -> bool {
        self.loaded > 0 && self.loaded < self.total
    }
}

/// The current state.
fn progress() -> Progress {
    PROGRESS.get()
}

/// Change the state.
fn update_progress(edit: impl FnOnce(&mut Progress)) {
    let mut next = PROGRESS.get();
    edit(&mut next);
    PROGRESS.set(next);
}

/// Start a new search: clear the counts, increase the generation, return it.
fn begin_search() -> u32 {
    let mut next = Progress::new();
    next.generation = PROGRESS.get().generation.wrapping_add(1);
    PROGRESS.set(next);
    next.generation
}

/// Is the generation of the request still the current one?
fn is_current(generation: u32) -> bool {
    progress().generation == generation
}

thread_local! {
    /// State of the search.
    static PROGRESS: Cell<Progress> = const { Cell::new(Progress::new()) };
    /// Handle of the `setTimeout` of the debounce.
    static DEBOUNCE: Cell<Option<i32>> = const { Cell::new(None) };
    /// Remove the rentals. The setting is asynchronous, so keep the value here.
    static NO_RENTAL: Cell<Option<bool>> = const { Cell::new(None) };
    /// Default sort from the settings. The second open needs no wait.
    static SORT_INITIAL: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Replies by URL, newest first, cut at `CACHE_LIMIT`.
    static CACHE: RefCell<Vec<(String, Page)>> = const { RefCell::new(Vec::new()) };
    /// The request that runs. Aborted when the words change.
    static PENDING: RefCell<Option<AbortController>> = const { RefCell::new(None) };
    /// Image size for the request, from the `thumb-size` setting.
    static KEY_VISUAL: RefCell<String> = RefCell::new(KEY_VISUAL_DEFAULT.to_string());
    /// The closure of the debounce. One instance, used again and again.
    ///
    /// `Closure::once_into_js` frees itself only when it is called, so one per key
    /// leaks all the cancelled ones, which is almost all of them.
    static DEBOUNCE_CALL: RefCell<Option<Closure<dyn FnMut()>>> = const { RefCell::new(None) };
    /// What to search when the time comes. `schedule` replaces it.
    static NEXT_QUERY: RefCell<Option<Query>> = const { RefCell::new(None) };
}

/// Read the image size setting. Called one time from `install`.
async fn load_key_visual_size() {
    let size = settings::choice(settings::THUMB_SIZE_KEY).await;
    // Not a number means "no change", so ask for the size of the site
    let size = if !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()) {
        size
    } else {
        KEY_VISUAL_SITE.to_string()
    };
    KEY_VISUAL.with_borrow_mut(|current| *current = size);
}

/// The stored state of the toggle. The same setting as the settings page and the
/// popup (`search-no-rental` in `settings::SWITCHES`).
async fn stored_no_rental() -> bool {
    settings::switch_enabled(NO_RENTAL_KEY).await
}

async fn save_no_rental(value: bool) {
    if let Err(err) = settings::save_one(NO_RENTAL_KEY, value).await {
        log(&format!(
            "レンタル除外の設定を保存できませんでした: {err:?}"
        ));
    }
}

/// The float search, or `None` if it is absent.
fn overlay(document: &Document) -> Option<Element> {
    document
        .query_selector(&format!(".{OVERLAY_CLASS}"))
        .ok()
        .flatten()
}

fn child(overlay: &Element, class: &str) -> Option<Element> {
    overlay.query_selector(&format!(".{class}")).ok().flatten()
}

fn input_of(root: &Element) -> Option<HtmlInputElement> {
    child(root, "dt-search__input").and_then(|el| el.dyn_into().ok())
}

fn rental_toggle_of(root: &Element) -> Option<HtmlInputElement> {
    child(root, "dt-search__rental").and_then(|el| el.dyn_into().ok())
}

/// Everything that one search needs. Read from the UI, put into the URL.
#[derive(Clone)]
struct Query {
    word: String,
    sort: String,
    /// `vodTypeList`. Depends on the rental toggle.
    vod_types: &'static str,
}

fn query_of(root: &Element) -> Query {
    let word = input_of(root)
        .map(|el| el.value().trim().to_string())
        .unwrap_or_default();
    let sort = child(root, "dt-search__sort")
        .and_then(|el| el.dyn_into::<HtmlSelectElement>().ok())
        .map(|el| el.value())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| SORT_DEFAULT.to_string());
    // Without the toggle, use the default (remove the rentals)
    let no_rental = rental_toggle_of(root)
        .map(|el| el.checked())
        .unwrap_or(NO_RENTAL_DEFAULT);
    Query {
        word,
        sort,
        vod_types: if no_rental { VOD_SVOD } else { VOD_ALL },
    }
}

/// Move one result into the card data of the list.
///
/// A result is a work and not an episode, so there is no `part_id` and the link
/// goes to `ci_pc?workId=`.
fn card_of(entry: &JsValue) -> Option<Card> {
    let info = json::get(entry, "workInfo")?;
    let title = json::get_string(&info, "workTitle")?;
    let work_id = json::get_string(entry, "workId").or_else(|| {
        // The field can also be a number
        json::get_f64(entry, "workId").map(|id| format!("{}", id as u64))
    });

    let mut badges = Vec::new();
    // State of the account. Present because the request sends the cookies.
    if let Some(flags) = json::path(entry, &["userInfo", "memberFlags"])
        .filter(|v| v.is_array())
        .map(|v| Array::from(&v))
    {
        for flag in flags.iter() {
            match flag.as_string().as_deref() {
                Some("complete") => badges.push(Badge {
                    modifier: Some("dt-badge--complete"),
                    text: "COMPLETE".into(),
                }),
                Some("favorite") => badges.push(Badge {
                    modifier: None,
                    text: t("badge.favorite").into(),
                }),
                // This means "one episode or more", so not "seen"
                Some("viewed") => badges.push(Badge {
                    modifier: None,
                    text: t("badge.watching").into(),
                }),
                _ => {}
            }
        }
    }
    if let Some(icons) = json::get_array(&info, "workIcons") {
        for icon in icons.iter() {
            if icon.as_string().as_deref() == Some("r15") {
                badges.push(Badge {
                    modifier: None,
                    text: "R15".into(),
                });
            }
        }
    }
    if json::get_string(&info, "vodType").as_deref() == Some("tvod") {
        badges.push(Badge {
            modifier: None,
            text: t("badge.rental").into(),
        });
    }

    Some(Card {
        // A result is a work, so the link goes to the work page
        link: card_view::link_of(work_id, None),
        work: None,
        number: None,
        title: Some(title),
        thumb: json::get_string(&info, "mainKeyVisualPath"),
        watched: false,
        progress: None,
        badges,
        thumb_size: None,
    })
}

/// Abort the request that runs.
///
/// The words changed, so the reply is not usable. To receive and then discard
/// does not reduce the work of the server.
fn abort_pending() {
    PENDING.with_borrow_mut(|slot| {
        if let Some(controller) = slot.take() {
            controller.abort();
        }
    });
}

/// Look in the cache.
fn cached(url: &str) -> Option<Page> {
    CACHE.with_borrow(|cache| {
        cache
            .iter()
            .find(|(key, _)| key == url)
            .map(|(_, value)| value.clone())
    })
}

fn remember(url: &str, value: &Page) {
    CACHE.with_borrow_mut(|cache| {
        cache.retain(|(key, _)| key != url);
        cache.insert(0, (url.to_string(), value.clone()));
        cache.truncate(CACHE_LIMIT);
    });
}

/// Get one page: (all hits, cards).
///
/// A correction of the words, a change of the sort and a second open are all
/// normal, so every reply is kept. The key is the URL, which has the words, the
/// sort, the rental toggle, the start and the length in it.
async fn fetch_page(query: &Query, start: u32, length: u32) -> Result<Page, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;

    let params = UrlSearchParams::new()?;
    params.append("searchKey", &query.word);
    params.append("vodTypeList", query.vod_types);
    params.append("sortKey", &query.sort);
    // The site also omits `start=0` (its `if (start)` removes it)
    if start > 0 {
        params.append("start", &start.to_string());
    }
    params.append("length", &length.to_string());
    params.append(
        "mainKeyVisualSize",
        &KEY_VISUAL.with_borrow(|size| size.clone()),
    );
    let url = format!("{ENDPOINT}?{}", String::from(params.to_string()));

    if let Some(hit) = cached(&url) {
        return Ok(hit);
    }

    // Make the request abortable
    let controller = AbortController::new()?;
    let init = RequestInit::new();
    init.set_signal(Some(&controller.signal()));
    PENDING.with_borrow_mut(|slot| *slot = Some(controller));

    let result = JsFuture::from(window.fetch_with_str_and_init(&url, &init)).await;
    PENDING.with_borrow_mut(|slot| *slot = None);
    let response: Response = result?.dyn_into()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!("HTTP {}", response.status())));
    }
    let value = JsFuture::from(response.json()?).await?;

    let total = json::path(&value, &["data", "maxCount"])
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .max(0.0) as u32;

    let list = json::path(&value, &["data", "workList"])
        .filter(|v| v.is_array())
        .map(|v| Array::from(&v))
        .unwrap_or_default();

    let cards: Vec<Card> = list.iter().filter_map(|entry| card_of(&entry)).collect();
    let page = (total, cards);
    remember(&url, &page);
    Ok(page)
}

/// Add the cards to the end of the grid. Returns the count.
fn append_cards(root: &Element, cards: &[Card]) -> Result<u32, JsValue> {
    let document = document()?;
    let Some(grid) = child(root, "dt-search__grid") else {
        return Ok(0);
    };
    // Insert tens of cards in one operation
    let fragment = document.create_document_fragment();
    let mut added = 0;
    for card in cards {
        let view = card_view::render(&document, card)?;
        fragment.append_child(&view)?;
        added += 1;
    }
    grid.append_child(&fragment)?;
    Ok(added)
}

/// The count, right of the input.
fn set_status(root: &Element, text: &str) {
    if let Some(status) = child(root, "dt-search__status") {
        status.set_text_content(Some(text));
    }
}

/// The state below the grid (prefetch, end of the results).
fn set_footer(root: &Element, text: &str) {
    if let Some(footer) = child(root, "dt-search__footer") {
        footer.set_text_content(Some(text));
    }
}

/// A message in place of the results. `None` removes it.
///
/// The height of the panel does not depend on the number of results, so an empty
/// message does not make the panel smaller (see `search-overlay.css`).
fn set_message(root: &Element, text: Option<&str>) {
    let Some(el) = child(root, "dt-search__empty") else {
        return;
    };
    match text {
        Some(text) => {
            el.set_text_content(Some(text));
            let _ = el.remove_attribute("hidden");
        }
        None => {
            let _ = el.set_attribute("hidden", "");
        }
    }
}

/// Put the current state into the count.
fn refresh_counts(root: &Element) {
    let Progress { loaded, total, .. } = progress();
    if total == 0 {
        set_status(root, "");
    } else if loaded >= total {
        set_status(
            root,
            &t_fill("search.count", &[("total", &total.to_string())]),
        );
    } else {
        set_status(
            root,
            &t_fill(
                "search.count.loaded",
                &[
                    ("total", &total.to_string()),
                    ("loaded", &loaded.to_string()),
                ],
            ),
        );
    }

    if loaded > 0 && loaded >= total {
        // Say that everything is on screen; do not stay silent
        set_footer(root, t("scroll.all"));
    } else {
        set_footer(root, "");
    }
}

/// Search again with new conditions. Removes the cards on screen.
async fn start_search(query: Query) {
    let generation = begin_search();
    // The request for the words before is not needed
    abort_pending();

    let Ok(document) = document() else { return };
    let Some(root) = overlay(&document) else {
        return;
    };

    if let Some(grid) = child(&root, "dt-search__grid") {
        grid.set_inner_html("");
    }
    // Scroll to the top, else the new results start in the middle
    if let Some(body) =
        child(&root, "dt-search__body").and_then(|el| el.dyn_into::<HtmlElement>().ok())
    {
        body.set_scroll_top(0);
    }

    // Do not ask for a short word
    if query.word.chars().count() < MIN_QUERY_CHARS {
        set_status(&root, "");
        set_footer(&root, "");
        set_message(
            &root,
            Some(if query.word.is_empty() {
                initial_hint()
            } else {
                t("search.short")
            }),
        );
        return;
    }

    let _ = root.class_list().add_1("dt-search--loading");
    set_status(&root, t("search.running"));
    let result = fetch_page(&query, 0, PAGE_SIZE).await;

    // A newer search runs, so this reply is old
    if !is_current(generation) {
        return;
    }
    let Some(root) = overlay(&document) else {
        return;
    };
    let _ = root.class_list().remove_1("dt-search--loading");

    match result {
        Ok((total, cards)) => {
            if cards.is_empty() {
                let hint = if query.vod_types == VOD_SVOD {
                    t("search.rental_hint")
                } else {
                    ""
                };
                set_message(
                    &root,
                    Some(&t_fill(
                        "search.empty",
                        &[("word", &query.word), ("hint", hint)],
                    )),
                );
                update_progress(|p| p.total = 0);
                refresh_counts(&root);
                return;
            }
            set_message(&root, None);
            match append_cards(&root, &cards) {
                Ok(added) => update_progress(|p| {
                    p.loaded = added;
                    p.total = total.max(added);
                }),
                Err(err) => log(&format!("検索結果の描画に失敗: {err:?}")),
            }
            refresh_counts(&root);
            // One page can be too short to fill the screen
            fill_screen().await;
        }
        Err(err) => {
            log(&format!("検索に失敗: {err:?}"));
            set_status(&root, "");
            set_message(&root, Some(t("search.failed")));
        }
    }
}

/// Add pages until the screen is full or all items are on screen.
///
/// The prefetch starts when the sentinel (`.dt-search__footer`) becomes visible.
/// `IntersectionObserver` reports only a change, so if the sentinel stays visible
/// after one page, it never reports again. Add pages here until the panel can
/// scroll.
async fn fill_screen() {
    let start = progress();
    if start.loading {
        return;
    }
    update_progress(|p| p.loading = true);

    loop {
        let Ok(document) = document() else { break };
        let Some(root) = overlay(&document) else {
            break;
        };
        let current = progress();
        if !current.has_more() {
            break;
        }
        // The panel can scroll, so leave the rest to the sentinel
        let scrollable = child(&root, "dt-search__body")
            .and_then(|el| el.dyn_into::<HtmlElement>().ok())
            .map(|body| body.scroll_height() > body.client_height() + 4)
            .unwrap_or(true);
        if scrollable {
            break;
        }
        if !load_page(&root, current.loaded, start.generation).await {
            break;
        }
    }

    update_progress(|p| p.loading = false);
}

/// Add the next page. This is the entry of the prefetch.
async fn load_next() {
    let current = progress();
    if current.loading || !current.has_more() {
        return;
    }
    let Ok(document) = document() else { return };
    let Some(root) = overlay(&document) else {
        return;
    };

    update_progress(|p| p.loading = true);
    set_footer(&root, t("search.more"));
    let ok = load_page(&root, current.loaded, current.generation).await;
    update_progress(|p| p.loading = false);
    if ok {
        // The sentinel can still be visible after the page
        fill_screen().await;
    }
}

/// Get one page from `start` and add it. `true` if more can follow.
///
/// The caller owns `Progress::loading`, because it calls this in a loop.
async fn load_page(root: &Element, start: u32, generation: u32) -> bool {
    let query = query_of(root);
    if query.word.chars().count() < MIN_QUERY_CHARS {
        return false;
    }

    let result = fetch_page(&query, start, PAGE_SIZE).await;

    // The words changed during the request, so this reply is old
    if !is_current(generation) {
        return false;
    }
    let Ok(document) = document() else {
        return false;
    };
    let Some(root) = overlay(&document) else {
        return false;
    };

    match result {
        Ok((total, cards)) => {
            let added = match append_cards(&root, &cards) {
                Ok(added) => added,
                Err(err) => {
                    log(&format!("検索結果の描画に失敗: {err:?}"));
                    0
                }
            };
            update_progress(|p| {
                p.loaded = start + added;
                // An empty reply is the end, even if the total says otherwise
                p.total = if added == 0 {
                    start
                } else {
                    total.max(start + added)
                };
            });
            refresh_counts(&root);
            added > 0
        }
        Err(err) => {
            log(&format!("続きの取得に失敗: {err:?}"));
            set_footer(&root, t("search.more.failed"));
            false
        }
    }
}

/// Search after the input stops. Cancels the wait that runs.
///
/// The conditions go into `NEXT_QUERY` and not into the closure, because the
/// closure is used again (a new one per key would leak the cancelled ones).
fn schedule(query: Query, delay: i32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    NEXT_QUERY.with_borrow_mut(|slot| *slot = Some(query));
    if let Some(handle) = DEBOUNCE.take() {
        window.clear_timeout_with_handle(handle);
    }
    DEBOUNCE_CALL.with_borrow_mut(|slot| {
        let callback = slot.get_or_insert_with(|| {
            Closure::<dyn FnMut()>::new(|| {
                DEBOUNCE.set(None);
                if let Some(query) = NEXT_QUERY.with_borrow_mut(Option::take) {
                    spawn_local(start_search(query));
                }
            })
        });
        if let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            delay,
        ) {
            DEBOUNCE.set(Some(handle));
        }
    });
}

/// Search again with what the UI holds now.
fn rerun(root: &Element, delay: i32) {
    schedule(query_of(root), delay);
}

/// Watch the sentinel and prefetch.
///
/// `IntersectionObserver` and not a scroll listener, so nothing runs per frame.
fn install_prefetch(body: &Element, footer: &Element) -> Result<(), JsValue> {
    let callback = Closure::<dyn FnMut()>::new(move || {
        spawn_local(load_next());
    });
    let options = IntersectionObserverInit::new();
    options.set_root(Some(body));
    options.set_root_margin(PREFETCH_MARGIN);
    let observer =
        IntersectionObserver::new_with_options(callback.as_ref().unchecked_ref(), &options)?;
    observer.observe(footer);

    // The float stays in the DOM after a close, so the observer stays
    callback.forget();
    std::mem::forget(observer);
    Ok(())
}

/// Build the float search. Does nothing if it exists.
fn build(document: &Document) -> Result<Element, JsValue> {
    if let Some(existing) = overlay(document) {
        return Ok(existing);
    }
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("no body"))?;

    let root = element(document, "div", OVERLAY_CLASS)?;
    root.set_attribute("role", "dialog")?;
    root.set_attribute("aria-modal", "true")?;
    root.set_attribute("aria-label", t("search.label"))?;

    let backdrop = element(document, "div", "dt-search__backdrop")?;
    root.append_child(&backdrop)?;

    let panel = element(document, "div", "dt-search__panel")?;

    // --- Top: input, sort, count ---
    let bar = element(document, "div", "dt-search__bar")?;
    let input: HtmlInputElement = element(document, "input", "dt-search__input")?.dyn_into()?;
    input.set_type("search");
    input.set_placeholder(t("search.placeholder"));
    input.set_autocomplete("off");
    input.set_spellcheck(false);
    bar.append_child(&input)?;

    let sort: HtmlSelectElement = element(document, "select", "dt-search__sort")?.dyn_into()?;
    sort.set_attribute("aria-label", t("search.sort.label"))?;
    // The sort comes from the setting (`search-sort`). Only the first build has
    // no stored value; the code below replaces it when the setting arrives.
    let initial_sort = SORT_INITIAL
        .with_borrow(|value| value.clone())
        .unwrap_or_else(|| SORT_DEFAULT.to_string());
    for (value, label) in SORTS {
        let option = document.create_element("option")?;
        option.set_attribute("value", value)?;
        option.set_text_content(Some(d_tweaks_shared::settings::option_label(label)));
        if *value == initial_sort {
            option.set_attribute("selected", "")?;
        }
        sort.append_child(&option)?;
    }
    bar.append_child(&sort)?;

    // Toggle for the rentals. The server removes them through `vodTypeList`. A
    // filter here would break `maxCount` and the paging.
    let filter = element(document, "label", "dt-search__filter")?;
    let rental: HtmlInputElement = element(document, "input", "dt-search__rental")?.dyn_into()?;
    rental.set_type("checkbox");
    rental.set_checked(NO_RENTAL.get().unwrap_or(NO_RENTAL_DEFAULT));
    filter.append_child(&rental)?;
    let filter_text = document.create_element("span")?;
    filter_text.set_text_content(Some(t("search.no_rental")));
    filter.append_child(&filter_text)?;
    bar.append_child(&filter)?;

    let status = element(document, "span", "dt-search__status")?;
    bar.append_child(&status)?;
    panel.append_child(&bar)?;

    // --- Bottom: results ---
    let body_area = element(document, "div", "dt-search__body")?;
    let grid = element(document, "div", "dt-search__grid")?;
    body_area.append_child(&grid)?;
    // Message while there are no results. The panel keeps its height.
    let empty = element(document, "p", "dt-search__empty")?;
    empty.set_text_content(Some(initial_hint()));
    body_area.append_child(&empty)?;
    // Sentinel of the prefetch. Also shows the state.
    let footer = element(document, "p", "dt-search__footer")?;
    body_area.append_child(&footer)?;
    panel.append_child(&body_area)?;

    root.append_child(&panel)?;
    body.append_child(&root)?;

    install_prefetch(&body_area, &footer)?;

    // --- Input ---
    //
    // Do not search while an IME composes (`isComposing`): each key would search
    // for an unconfirmed kana, which is wasteful and makes the results flicker.
    // `compositionend` gives the confirmed text. Chrome also sends `input` after
    // that, but the debounce removes the second search.
    {
        let root_for_input = root.clone();
        let on_input = Closure::<dyn FnMut(InputEvent)>::new(move |event: InputEvent| {
            if event.is_composing() {
                return;
            }
            rerun(&root_for_input, DEBOUNCE_MS);
        });
        input.add_event_listener_with_callback("input", on_input.as_ref().unchecked_ref())?;
        on_input.forget();
    }
    {
        let root_for_composition = root.clone();
        let on_end = Closure::<dyn FnMut()>::new(move || {
            rerun(&root_for_composition, DEBOUNCE_MS);
        });
        input
            .add_event_listener_with_callback("compositionend", on_end.as_ref().unchecked_ref())?;
        on_end.forget();
    }
    // Enter searches without a wait
    {
        let root_for_enter = root.clone();
        let on_key = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            if event.key() != "Enter" || event.is_composing() {
                return;
            }
            event.prevent_default();
            rerun(&root_for_enter, 0);
        });
        input.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref())?;
        on_key.forget();
    }

    // --- Sort ---
    {
        let root_for_sort = root.clone();
        let on_change = Closure::<dyn FnMut()>::new(move || {
            rerun(&root_for_sort, 0);
        });
        sort.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
        on_change.forget();
    }

    // --- Remove the rentals ---
    {
        let root_for_rental = root.clone();
        let toggle = rental.clone();
        let on_change = Closure::<dyn FnMut()>::new(move || {
            let checked = toggle.checked();
            NO_RENTAL.set(Some(checked));
            spawn_local(async move {
                save_no_rental(checked).await;
            });
            rerun(&root_for_rental, 0);
        });
        rental.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
        on_change.forget();
    }

    // Put the stored state into the UI. The sort (`search-sort`) comes from the
    // same place: both are in `storage.sync`, so one wait gives both.
    if NO_RENTAL.get().is_none() || SORT_INITIAL.with_borrow(|value| value.is_none()) {
        let toggle = rental.clone();
        let select = sort.clone();
        let root_for_restore = root.clone();
        spawn_local(async move {
            let stored_rental = stored_no_rental().await;
            let stored_sort = settings::choice(settings::SEARCH_SORT_KEY).await;
            NO_RENTAL.set(Some(stored_rental));
            SORT_INITIAL.with_borrow_mut(|value| *value = Some(stored_sort.clone()));

            let mut changed = false;
            if toggle.checked() != stored_rental {
                toggle.set_checked(stored_rental);
                changed = true;
            }
            if !stored_sort.is_empty() && select.value() != stored_sort {
                select.set_value(&stored_sort);
                changed = true;
            }
            // A search can have run before this, so search again
            if changed && !query_of(&root_for_restore).word.is_empty() {
                rerun(&root_for_restore, 0);
            }
        });
    }

    // --- A click on the background closes. There is no close button; Esc works. ---
    {
        let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
            close();
        });
        backdrop.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    Ok(root)
}

/// Is the float open?
fn is_open(document: &Document) -> bool {
    overlay(document)
        .map(|root| !root.has_attribute("hidden"))
        .unwrap_or(false)
}

/// Open the float search. `query` gives the first words.
pub fn open(query: Option<&str>) -> Result<(), JsValue> {
    let document = document()?;
    let root = build(&document)?;
    root.remove_attribute("hidden")?;
    if let Some(html) = document.document_element() {
        html.class_list().add_1(OPEN_CLASS)?;
    }

    if let Some(input) = input_of(&root) {
        if let Some(query) = query {
            input.set_value(query);
        }
        // A second open keeps the words, so select them for a fast replacement
        input.focus()?;
        input.select();
        if !input.value().trim().is_empty() {
            rerun(&root, 0);
        }
    }
    Ok(())
}

/// Close the float search. The DOM stays, so the words and the results stay.
pub fn close() {
    let Ok(document) = document() else { return };
    if let Some(html) = document.document_element() {
        let _ = html.class_list().remove_1(OPEN_CLASS);
    }
    if let Some(root) = overlay(&document) {
        let _ = root.set_attribute("hidden", "");
    }
}

/// Is the user typing? A shortcut must not take a key from an input.
fn typing(document: &Document) -> bool {
    let Some(active) = document.active_element() else {
        return false;
    };
    let tag = active.tag_name().to_ascii_lowercase();
    if tag == "input" || tag == "textarea" || tag == "select" {
        return true;
    }
    active
        .dyn_ref::<HtmlElement>()
        .map(|el| el.is_content_editable())
        .unwrap_or(false)
}

/// Take the clicks on "さがす" and add the shortcuts.
///
/// With `shortcuts` false, Cmd-K and `/` stay with the page. Esc is taken only
/// while the float is open.
pub fn install(shortcuts: bool) -> Result<(), JsValue> {
    let target = document()?;

    // The image size setting is asynchronous, so read it before the first open
    spawn_local(load_key_visual_size());

    // --- Take the "さがす" links. Capture, to come before the anchor. ---
    let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
        // A modifier or the middle button means "open in a new tab"; do not take it
        if event.ctrl_key() || event.meta_key() || event.shift_key() || event.button() != 0 {
            return;
        }
        let Some(clicked) = event.target() else {
            return;
        };
        let Ok(el) = clicked.dyn_into::<Element>() else {
            return;
        };
        if !matches!(el.closest(SEARCH_LINK_SELECTOR), Ok(Some(_))) {
            return;
        }
        event.prevent_default();
        event.stop_propagation();
        if let Err(err) = open(None) {
            log(&format!("フロート検索を開けませんでした: {err:?}"));
        }
    });
    target.add_event_listener_with_callback_and_bool(
        "click",
        on_click.as_ref().unchecked_ref(),
        true,
    )?;
    on_click.forget();

    // --- Shortcuts: Esc closes, Cmd-K (Ctrl-K) and / open ---
    let on_key = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        let Ok(document) = document() else { return };

        if event.key() == "Escape" {
            if !is_open(&document) {
                return;
            }
            // Stop the event, else the float player also takes this Esc
            event.prevent_default();
            event.stop_propagation();
            close();
            return;
        }

        if !shortcuts {
            return;
        }
        let open_by_k =
            (event.meta_key() || event.ctrl_key()) && event.key().eq_ignore_ascii_case("k");
        // `/` is a character, so do not take it inside an input
        let open_by_slash = event.key() == "/"
            && !event.meta_key()
            && !event.ctrl_key()
            && !event.alt_key()
            && !typing(&document);
        if !open_by_k && !open_by_slash {
            return;
        }
        if is_open(&document) {
            return;
        }
        event.prevent_default();
        event.stop_propagation();
        if let Err(err) = open(None) {
            log(&format!("フロート検索を開けませんでした: {err:?}"));
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
