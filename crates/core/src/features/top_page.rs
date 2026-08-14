//! Builds an own top page (`tp_pc`) from the data of the original page.
//!
//! The top page of the site has 15 horizontal sliders over 6432px. Each slider is
//! 4360 to 10410px wide, so one slider needs four to eight screens of horizontal
//! movement. The 15 sliders also have the same shape, so nothing shows what is
//! important.
//!
//! This module does not keep that order. It reads all sections into data and
//! builds this:
//!
//! ```text
//! [showcase]  the ranking, large, with the key visual and the summary
//! [today]     "on air now", with the update time. This is the daily view.
//! [ranking]   the first items, with a number
//! [find]      the other 10 sliders as one grid with chips
//! ```
//!
//! The result is one screen and a half. The rank is not in the DOM of the site
//! (only the order is), so this module adds it.
//!
//! # The data is in the attributes of a card
//!
//! `.p-slider__item` holds the data (not in every template):
//!
//! ```text
//! <div class="p-slider__item itemModule"
//!      data-workid data-worktitle data-link data-workexp>
//!   <a class="c-slide" href="…">
//!     <div class="c-thumbnail"><img class="c-thumbnail__img" data-src="…_3.png">
//!     <div class="c-infoDetails">
//!       <h1 class="c-infoDetails__title">work title</h1>
//!       <div class="c-infoDetails__onAir"><span …>12:00更新</span>
//! ```
//!
//! Some templates have no `data-*` (on air now, exclusive), so read the attribute
//! first and the element after it. The destination is not always `ci_pc` (a series,
//! a feature), so use `data-link` or the href of the `a`.
//!
//! # The image size depends on the place
//!
//! - Grid card (150 to 200px): `_6` (208x117). The `_3` (144x81) of the site is too
//!   small for its own 210x118 frame, so the site makes it blurred. `_6` fits, at
//!   18KB in place of 10KB.
//! - Showcase (one image, full width): `_1` (640x360).
//!
//! The `_1` of the lists on a grid card would be 32 times the bytes for each of
//! tens of cards.
//!
//! # The original DOM is hidden, not removed
//!
//! A section that this module used gets `dt-top-rendered`, and the CSS hides only
//! those. If the WASM does not run, the top page of the site stays complete.
//!
//! Nothing that is not 16:9 is read (the covers of the books, the four-image tiles
//! of a my-list), so those sliders stay as they are (see `parse_item`).
//!
//! The favorite control (`input.favo`) is in the original DOM. 300 own buttons
//! would be too expensive, so this page has no heart (the work page has one).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, HtmlAnchorElement, HtmlElement, MouseEvent, Url};

use d_tweaks_shared::settings;
use d_tweaks_shared::text::{t, t_fill};

use crate::dom::{attr, element, text_of};
use crate::features::card_view::{self, Badge, Card};
use crate::features::rental;
use crate::log;

/// A section of the site.
const SECTION_SELECTOR: &str = ".l-section";
/// A horizontal slider. Only a section with one is used.
const SLIDER_SELECTOR: &str = ".p-slider__itemList";
const ITEM_SELECTOR: &str = ".p-slider__item";
/// Marks a section that was used. The CSS hides it.
const RENDERED_CLASS: &str = "dt-top-rendered";
const ROOT_CLASS: &str = "dt-top";
/// Size for a grid card: 208x117, exactly the 210x118 frame of the site.
const CARD_THUMB_SIZE: &str = "6";
/// Size for the showcase: 640x360, 319KB per image.
///
/// The 1280 version (1.17MB) is too much for the width, and the showcase changes
/// the image, so it needs one per item: 10 items are 11MB of transfer and 37MB of
/// pixels in memory.
///
/// The `<img>` elements stay, so no image is loaded twice (see `render_showcase`).
const HERO_THUMB_SIZE: &str = "1";
/// Cards in the grid at the start. A button adds the others.
///
/// An image that is not drawn is not loaded, so this number is the transfer.
const INITIAL_ITEMS: usize = 12;
const RANKING_ITEMS: usize = 10;
/// With this many cards or fewer left, show all and no button.
///
/// A button for one more card after 12 of 13 makes no sense.
const SHOW_ALL_SLACK: usize = 3;
/// Interval and number of attempts while the items arrive.
///
/// The items of a slider are not in the HTML from the server; the JS of the site
/// asks a REST interface. A fixed wait (0, 800, 2500ms before) shows the original
/// page until the next step, also when the items are already there. So look often.
const POLL_INTERVAL_MS: i32 = 150;
const POLL_ATTEMPTS: u32 = 60;
/// Ticks without a new section or a new item before the page is built.
///
/// The sections do not arrive together. A build at the first possible moment used only
/// 3 of the 15 sections (measured), and then the ranking was absent, so the showcase had
/// to use another section. A short wait for the rest costs nothing that the user sees:
/// the CSS hides the sliders of the site until this module builds.
const SETTLE_TICKS: u32 = 2;
/// Longest wait after the page can be built, in milliseconds.
///
/// The CSS gives the site back after 2500ms (`top-page.css`), so the build must happen
/// before that, also when the sections keep arriving.
const SETTLE_MAX_MS: u32 = 1200;
/// Sections that must be readable before the page is built.
const MIN_SECTIONS: usize = 2;
/// Seconds until the showcase goes to the next item.
const SLIDE_INTERVAL_SECONDS: u32 = 8;
/// Holds the position of an item of the rail.
const RAIL_INDEX_ATTR: &str = "data-dt-index";

/// What a section is for. This decides the place, not the order of the site.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Role {
    /// On air now, with an update time.
    OnAir,
    Ranking,
    /// Everything else. Reached with a chip.
    Browse,
}

struct Section {
    role: Role,
    title: String,
    /// Text next to the title, for example `（24）`.
    note: Option<String>,
    /// Destination of "すべて見る".
    all_href: Option<String>,
    items: Vec<Card>,
    /// Summaries (`data-workexp`) for the showcase, in the order of `items`.
    exps: Vec<Option<String>>,
}

/// Move one item into a card. Returns (card, summary).
///
/// Anything that one 16:9 image cannot show is not used:
///
/// - The cover of a book (`.c-thumbnail.isBook`) is higher than wide, so 16:9 cuts
///   it. Its destination `/animestore/book/*` is also out of scope.
/// - The tile of a my-list (`.c-fourImages`) holds four works in one image.
///
/// A section with only those gives no card, and the slider of the site stays. That
/// is correct.
fn parse_item(item: &Element) -> Option<(Card, Option<String>)> {
    for selector in [".c-thumbnail.isBook", ".c-fourImages"] {
        if item.query_selector(selector).ok().flatten().is_some() {
            return None;
        }
    }

    // Destination: data-link, then this element as an <a>, then an <a> inside
    let href = attr(item, "data-link")
        .or_else(|| {
            item.dyn_ref::<HtmlAnchorElement>()
                .map(|a| a.href())
                .filter(|href| !href.is_empty())
        })
        .or_else(|| {
            item.query_selector("a[href]")
                .ok()
                .flatten()
                .and_then(|a| a.dyn_into::<HtmlAnchorElement>().ok())
                .map(|a| a.href())
                .filter(|href| !href.is_empty() && !href.contains("void(0)"))
        })?;

    let work = attr(item, "data-worktitle").or_else(|| text_of(item, ".c-infoDetails__title"));
    let thumb = item
        .query_selector(".c-thumbnail__img")
        .ok()
        .flatten()
        .and_then(|img| attr(&img, "data-src").or_else(|| attr(&img, "src")))
        .filter(|src| !src.contains("lazySpace"));
    if work.is_none() && thumb.is_none() {
        return None;
    }

    // Read workId and partId from the destination.
    //
    // The links of "on air now" have a partId
    // (`ci_pc?workId=28641&partId=28641006`). With it, `card_view` makes the
    // thumbnail go to `sc_d_pc?partId=`, so a click plays that episode. The work
    // title still goes to the work page.
    let params = Url::new(&href).ok().map(|url| url.search_params());
    let work_id = params
        .as_ref()
        .and_then(|p| p.get("workId"))
        .or_else(|| attr(item, "data-workid"));
    let part_id = params.as_ref().and_then(|p| p.get("partId"));

    // A rental goes out here, so no section, no chip and no image of it is made
    if work_id.as_deref().is_some_and(is_rental) {
        return None;
    }

    // An update time such as "12:00更新" becomes a badge
    let mut badges = Vec::new();
    if let Some(update) = text_of(item, ".c-infoDetails__onAirUpdate") {
        badges.push(Badge {
            modifier: Some("dt-badge--new"),
            text: update,
        });
    }

    let number = episode_label(
        text_of(item, ".c-infoDetails__onAirRange"),
        work_id.as_deref(),
        part_id.as_deref(),
    );

    // Build the destination from the ids. With an enum, an item with an id cannot
    // be without a link (an earlier version read only `href`, ignored `work_id`,
    // and a click did nothing). Only an item without both ids uses the raw href.
    let link =
        card_view::link_of(work_id.clone(), part_id).or(Some(card_view::Link::External(href)));

    let card = Card {
        link,
        // The work title goes into `work` and not into `title`: `card_view` makes
        // `work` a link and also puts it into a data attribute, and the comment
        // search reads the work title and the number from those attributes.
        work,
        number,
        title: None,
        thumb,
        watched: false,
        progress: None,
        badges,
        thumb_size: Some(CARD_THUMB_SIZE),
    };
    Some((card, attr(item, "data-workexp")))
}

/// The episode number, as `第6話`.
///
/// `.c-infoDetails__onAirRange` holds it, but the measurement gives three forms:
///
/// - `第6話`: usable
/// - `6`: only the number, so give it the same form
/// - `...sode 12`: the site cut "Episode 12" from the left (the real DOM value)
///
/// A broken value is discarded and the number comes from the `partId`, which is the
/// `workId` and three digits for the episode (measured on all 15 sections).
fn episode_label(
    text: Option<String>,
    work_id: Option<&str>,
    part_id: Option<&str>,
) -> Option<String> {
    if let Some(text) = &text {
        if text.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("第{text}話"));
        }
        if text.starts_with('第') && text.ends_with('話') {
            return Some(text.clone());
        }
    }

    // A broken or unknown form comes from the partId
    let number = part_id?.strip_prefix(work_id?)?.trim_start_matches('0');
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("第{number}話"))
}

/// The role, from the title.
fn role_of(title: &str) -> Role {
    if title.contains("放送中") {
        Role::OnAir
    } else if title.contains("ランキング") {
        Role::Ranking
    } else {
        Role::Browse
    }
}

thread_local! {
    /// The workIds of the rentals, when the setting is on and the answer arrived.
    ///
    /// `render` is not async (a timer calls it), so the answer cannot be awaited there.
    /// `start_rental_load` fills this while the items of the site arrive, which takes at
    /// least two ticks, so the answer of the cache is always in time. Without the answer
    /// nothing is removed: a page with rentals is better than a page that waits.
    static RENTAL_IDS: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };
}

/// Start reading the rental works. Does nothing when the setting is off.
async fn start_rental_load() {
    if !settings::switch_enabled(settings::TOP_NO_RENTAL).await {
        return;
    }
    if let Some(ids) = rental::work_ids().await {
        RENTAL_IDS.with_borrow_mut(|slot| *slot = Some(ids));
    }
}

fn is_rental(work_id: &str) -> bool {
    RENTAL_IDS.with_borrow(|slot| slot.as_ref().is_some_and(|ids| ids.contains(work_id)))
}

/// Is the rental filter on and ready? Then the section of the rentals is also hidden.
fn rental_filter_ready() -> bool {
    RENTAL_IDS.with_borrow(|slot| slot.is_some())
}

/// Does this section hold the rentals?
///
/// Every item of it is removed by `parse_item`, so it gives no cards and would stay on
/// the screen as a slider of the site. The id of the `<section>` is `rental` (measured in
/// the HTML of `tp_pc`); the title is the second test, in case that id changes.
fn is_rental_section(section: &Element) -> bool {
    let by_id = section
        .closest("section[id]")
        .ok()
        .flatten()
        .map(|el| el.id() == "rental")
        .unwrap_or(false);
    by_id
        || text_of(section, ".p-title__text")
            .map(|title| title.contains("レンタル"))
            .unwrap_or(false)
}

/// Read a section. `None` without a slider or without a usable item.
fn parse_section(section: &Element) -> Option<Section> {
    section.query_selector(SLIDER_SELECTOR).ok()??;

    let nodes = section.query_selector_all(ITEM_SELECTOR).ok()?;
    let parsed: Vec<(Card, Option<String>)> = (0..nodes.length())
        .filter_map(|i| nodes.item(i))
        .filter_map(|node| node.dyn_into::<Element>().ok())
        .filter_map(|item| parse_item(&item))
        .collect();
    if parsed.is_empty() {
        return None;
    }

    // "すべて見る" is next to the title. Find it by its text, not by a class.
    let mut all_href = None;
    if let Ok(anchors) = section.query_selector_all("a[href]") {
        for i in 0..anchors.length() {
            let Some(node) = anchors.item(i) else {
                continue;
            };
            let Ok(anchor) = node.dyn_into::<Element>() else {
                continue;
            };
            let text = anchor.text_content().unwrap_or_default();
            if text.contains("すべて見る") || text.contains("もっと見る") {
                all_href = anchor
                    .dyn_ref::<HtmlAnchorElement>()
                    .map(|a| a.href())
                    .filter(|href| !href.is_empty());
                break;
            }
        }
    }

    let title = text_of(section, ".p-title__text").unwrap_or_else(|| t("top.other").to_string());
    let (items, exps) = parsed.into_iter().unzip();
    Some(Section {
        role: role_of(&title),
        title,
        note: text_of(section, ".p-title__subText"),
        all_href,
        items,
        exps,
    })
}

/// Add the cards `from`..`to` to `grid`.
fn append_cards(
    document: &Document,
    grid: &Element,
    items: &[Card],
    from: usize,
    to: usize,
    ranked: bool,
) -> Result<(), JsValue> {
    let fragment = document.create_document_fragment();
    for (offset, card) in items.iter().take(to).skip(from).enumerate() {
        let view = card_view::render(document, card)?;
        // The rank is not in the DOM of the site, only the order
        if ranked {
            let rank = element(document, "span", "dt-top__rank")?;
            rank.set_text_content(Some(&(from + offset + 1).to_string()));
            view.append_child(&rank)?;
        }
        fragment.append_child(&view)?;
    }
    grid.append_child(&fragment)?;
    Ok(())
}

/// Cards to show at the start. Shows all when only a few would be left.
fn shown_count(total: usize, limit: usize) -> usize {
    if total <= limit + SHOW_ALL_SLACK {
        total
    } else {
        limit
    }
}

/// The head of a block: title, note, "see all".
fn head_of(
    document: &Document,
    title: &str,
    note: Option<&str>,
    all_href: Option<&str>,
) -> Result<Element, JsValue> {
    let head = element(document, "div", "dt-top__head")?;
    let el = element(document, "h2", "dt-top__title")?;
    el.set_text_content(Some(title));
    head.append_child(&el)?;
    if let Some(note) = note {
        let el = element(document, "span", "dt-top__note")?;
        el.set_text_content(Some(note));
        head.append_child(&el)?;
    }
    if let Some(href) = all_href {
        let el = element(document, "a", "dt-top__all")?;
        el.set_attribute("href", href)?;
        el.set_text_content(Some(t("top.all")));
        head.append_child(&el)?;
    }
    Ok(head)
}

/// Make the ranking one large showcase.
///
/// The banner and the ranking list were two blocks; this is one. A rail of
/// thumbnails is at the bottom, and a frame marks the item on screen. A click
/// changes the item at once, and without a click it goes on every
/// `SLIDE_INTERVAL_SECONDS`.
///
/// The images are the 640 version: the showcase changes the image, so it needs one
/// per item, and the 1280 version (1MB each) would be 10MB for 10 items. 640 gives
/// 3.2MB, and the URL stays the same, so the second round loads nothing. The rail
/// uses the 208 version, at 18KB each.
///
/// The showcase stops while nobody sees it: off screen, in a background tab, or
/// under the pointer (the `spawn_local` below tests this every second).
fn render_showcase(
    document: &Document,
    section: &Section,
    label: &str,
) -> Result<Element, JsValue> {
    let hero = element(document, "section", "dt-top__hero")?;
    let count = section.items.len().min(RANKING_ITEMS);

    // One `<img>` per item, and every `<img>` stays.
    //
    // With one `<img>` and a new `src`, the image is loaded again on the way back.
    // Measured: item 1, 2, 1, 2 gave three HTTP 200 for the same URL (the images
    // have no `Cache-Control`, so the browser decides and cannot be trusted). An
    // `<img>` in the DOM loads nothing while its `src` does not change.
    let images = element(document, "div", "dt-top__heroImgs")?;
    hero.append_child(&images)?;

    // Every place over the rail goes to the work page.
    //
    // An `<a>` around the text would put the "ranking" link inside another link,
    // which is not valid HTML. So this anchor is only a click area, and the text
    // passes its clicks with `pointer-events` (see the CSS).
    let hit = element(document, "a", "dt-top__heroHit")?;
    hit.set_attribute("aria-label", t("top.work.label"))?;
    hero.append_child(&hit)?;

    let body = element(document, "div", "dt-top__heroBody")?;
    // The source ("デイリーランキング 3位") is the entry to that list
    let tag = match &section.all_href {
        Some(href) => {
            let el = element(document, "a", "dt-top__heroTag")?;
            el.set_attribute("href", href)?;
            el
        }
        None => element(document, "span", "dt-top__heroTag")?,
    };
    body.append_child(&tag)?;
    let title = element(document, "h1", "dt-top__heroTitle")?;
    body.append_child(&title)?;
    let exp = element(document, "p", "dt-top__heroExp")?;
    body.append_child(&exp)?;
    let link = element(document, "a", "dt-top__heroLink")?;
    link.set_text_content(Some(t("top.watch")));
    body.append_child(&link)?;
    hero.append_child(&body)?;

    // The rail. The item on screen gets `is-on`.
    let rail = element(document, "div", "dt-top__rail")?;
    for (index, card) in section.items.iter().take(count).enumerate() {
        let item = element(document, "button", "dt-top__railItem")?;
        item.set_attribute("type", "button")?;
        item.set_attribute(RAIL_INDEX_ATTR, &index.to_string())?;
        if let Some(src) = &card.thumb {
            let thumb = element(document, "img", "dt-top__railImg")?;
            let small =
                card_view::resize_thumb(src, Some(CARD_THUMB_SIZE)).unwrap_or_else(|| src.clone());
            thumb.set_attribute("src", &small)?;
            thumb.set_attribute("alt", "")?;
            thumb.set_attribute("loading", "lazy")?;
            item.append_child(&thumb)?;
        }
        let rank = element(document, "span", "dt-top__railRank")?;
        rank.set_text_content(Some(&(index + 1).to_string()));
        item.append_child(&rank)?;
        rail.append_child(&item)?;
    }
    hero.append_child(&rail)?;

    // --- Change the item ---
    let slides: Rc<Vec<(Card, Option<String>)>> = Rc::new(
        section
            .items
            .iter()
            .take(count)
            .cloned()
            .zip(section.exps.iter().take(count).cloned())
            .collect(),
    );
    let current = Rc::new(Cell::new(usize::MAX));
    let label = label.to_string();
    // index to its `<img>`. An image stays, so it is never loaded twice.
    let loaded: Rc<RefCell<HashMap<usize, Element>>> = Rc::new(RefCell::new(HashMap::new()));

    let show = {
        let slides = Rc::clone(&slides);
        let current = Rc::clone(&current);
        let rail = rail.clone();
        Rc::new(move |index: usize| {
            let Some((card, summary)) = slides.get(index) else {
                return;
            };
            if current.get() == index {
                return;
            }
            current.set(index);

            if let Some(src) = &card.thumb {
                let mut cache = loaded.borrow_mut();
                if !cache.contains_key(&index)
                    && let Some(document) = web_sys::window().and_then(|w| w.document())
                    && let Ok(img) = element(&document, "img", card_view::HERO_IMG_CLASS)
                {
                    // If the larger size does not exist,
                    // `card_view::install_thumb_fallback` puts the original back
                    let _ = card_view::set_thumb(&img, src, Some(HERO_THUMB_SIZE));
                    let _ = img.set_attribute("alt", "");
                    let _ = images.append_child(&img);
                    cache.insert(index, img);
                }
                // Only the item on screen gets `is-on`. The others stay, invisible.
                for (key, img) in cache.iter() {
                    let _ = if *key == index {
                        img.class_list().add_1("is-on")
                    } else {
                        img.class_list().remove_1("is-on")
                    };
                }
            }
            tag.set_text_content(Some(&t_fill(
                "top.rank",
                &[("label", &label), ("rank", &(index + 1).to_string())],
            )));
            title.set_text_content(card.work.as_deref().or(card.title.as_deref()));
            exp.set_text_content(summary.as_deref());
            // Ask `card_view` for the destination. An earlier version read a raw
            // `href` field, which is empty when the workId is known, so a click on
            // a ranking card did nothing.
            match card_view::destination(card) {
                Some(href) => {
                    let _ = link.set_attribute("href", &href);
                    let _ = link.remove_attribute("hidden");
                    let _ = hit.set_attribute("href", &href);
                }
                None => {
                    let _ = link.set_attribute("hidden", "");
                    let _ = hit.remove_attribute("href");
                }
            }

            // Move the frame in the rail, and scroll if the item is not visible
            let mut item = rail.first_element_child();
            let mut position = 0usize;
            while let Some(current_item) = item {
                if position == index {
                    let _ = current_item.class_list().add_1("is-on");
                    if let Some(el) = current_item.dyn_ref::<HtmlElement>() {
                        scroll_into_rail(&rail, el);
                    }
                } else {
                    let _ = current_item.class_list().remove_1("is-on");
                }
                item = current_item.next_element_sibling();
                position += 1;
            }
        })
    };

    // A click on the rail goes to that item and starts the interval again
    let held = Rc::new(Cell::new(false));
    // Seconds since the last change. A click sets it to 0.
    let elapsed = Rc::new(Cell::new(0u32));
    {
        let show = Rc::clone(&show);
        let elapsed = Rc::clone(&elapsed);
        let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
            let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                return;
            };
            let Ok(Some(item)) = target.closest(".dt-top__railItem") else {
                return;
            };
            let Some(index) = item
                .get_attribute(RAIL_INDEX_ATTR)
                .and_then(|value| value.parse::<usize>().ok())
            else {
                return;
            };
            elapsed.set(0);
            show(index);
        });
        rail.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    // Stop under the pointer: a change while the user reads the summary is bad
    for (event, value) in [("mouseenter", true), ("mouseleave", false)] {
        let held = Rc::clone(&held);
        let on_hover = Closure::<dyn FnMut(MouseEvent)>::new(move |_| held.set(value));
        hero.add_event_listener_with_callback(event, on_hover.as_ref().unchecked_ref())?;
        on_hover.forget();
    }

    show(0);

    // Every second, test if the showcase can go on
    {
        let hero_for_tick = hero.clone();
        let show = Rc::clone(&show);
        let current = Rc::clone(&current);
        let elapsed = Rc::clone(&elapsed);
        spawn_local(async move {
            elapsed.set(0);
            loop {
                crate::sleep(1000).await;
                // Stop when the element is gone
                if !hero_for_tick.is_connected() {
                    return;
                }
                if held.get() || !on_screen(&hero_for_tick) {
                    continue;
                }
                elapsed.set(elapsed.get() + 1);
                if elapsed.get() < SLIDE_INTERVAL_SECONDS {
                    continue;
                }
                elapsed.set(0);
                show((current.get() + 1) % count.max(1));
            }
        });
    }

    Ok(hero)
}

/// Scroll the rail until the item is visible.
///
/// `scrollIntoView` moves the whole page, so this moves `scrollLeft` of the rail.
fn scroll_into_rail(rail: &Element, item: &HtmlElement) {
    let Some(rail) = rail.dyn_ref::<HtmlElement>() else {
        return;
    };
    let left = item.offset_left();
    let right = left + item.offset_width();
    let view_left = rail.scroll_left();
    let view_right = view_left + rail.client_width();
    if left < view_left {
        rail.set_scroll_left(left - 8);
    } else if right > view_right {
        rail.set_scroll_left(right - rail.client_width() + 8);
    }
}

/// Is the element visible? Tests the vertical direction only.
fn on_screen(el: &Element) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    // Not in a background tab
    if window.document().map(|doc| doc.hidden()).unwrap_or(false) {
        return false;
    }
    let height = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let rect = el.get_bounding_client_rect();
    rect.bottom() > 0.0 && rect.top() < height
}

/// The "more" button. A click draws the other cards, and not before.
fn more_button(
    document: &Document,
    grid: &Element,
    items: &[Card],
    shown: usize,
    ranked: bool,
) -> Result<Element, JsValue> {
    let more = element(document, "button", "dt-top__more")?;
    more.set_attribute("type", "button")?;
    more.set_text_content(Some(&t_fill(
        "top.more",
        &[("count", &(items.len() - shown).to_string())],
    )));

    let items = Rc::new(items.to_vec());
    let grid = grid.clone();
    let button = more.clone();
    let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        if let Err(err) = append_cards(&document, &grid, &items, shown, items.len(), ranked) {
            log(&format!("トップの追加描画に失敗: {err:?}"));
            return;
        }
        button.remove();
    });
    more.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    on_click.forget();
    Ok(more)
}

/// One section as a head, a grid and a "more" button.
fn render_row(
    document: &Document,
    section: &Section,
    limit: usize,
    ranked: bool,
) -> Result<Element, JsValue> {
    let root = element(document, "section", "dt-top__section")?;
    let head = head_of(
        document,
        &section.title,
        section.note.as_deref(),
        section.all_href.as_deref(),
    )?;
    root.append_child(&head)?;

    let grid = element(document, "div", "dt-top__grid")?;
    let shown = shown_count(section.items.len(), limit);
    append_cards(document, &grid, &section.items, 0, shown, ranked)?;
    root.append_child(&grid)?;

    if section.items.len() > shown {
        let more = more_button(document, &grid, &section.items, shown, ranked)?;
        root.append_child(&more)?;
    }
    Ok(root)
}

/// The other sections as one grid with chips, in place of 10 sliders of the same
/// shape. Only the selected chip is drawn, so only its images are loaded.
fn render_browse(document: &Document, sections: Rc<Vec<Section>>) -> Result<Element, JsValue> {
    let root = element(document, "section", "dt-top__browse")?;
    let head = head_of(document, t("top.browse"), None, None)?;
    root.append_child(&head)?;

    let chips = element(document, "div", "dt-top__chips")?;
    let body = element(document, "div", "dt-top__browseBody")?;

    // A click on any chip builds this container again
    let draw = {
        let body = body.clone();
        let chips_for_draw = chips.clone();
        let sections = Rc::clone(&sections);
        Rc::new(move |index: usize| {
            let Some(document) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Some(section) = sections.get(index) else {
                return;
            };
            body.set_inner_html("");

            // Mark the chip that is selected
            let mut chip = chips_for_draw.first_element_child();
            let mut position = 0usize;
            while let Some(current) = chip {
                let _ = if position == index {
                    current.class_list().add_1("is-on")
                } else {
                    current.class_list().remove_1("is-on")
                };
                chip = current.next_element_sibling();
                position += 1;
            }

            let Ok(grid) = element(&document, "div", "dt-top__grid") else {
                return;
            };
            let shown = shown_count(section.items.len(), INITIAL_ITEMS);
            if let Err(err) = append_cards(&document, &grid, &section.items, 0, shown, false) {
                log(&format!("トップの描画に失敗: {err:?}"));
                return;
            }
            let _ = body.append_child(&grid);

            if section.items.len() > shown
                && let Ok(button) = more_button(&document, &grid, &section.items, shown, false)
            {
                let _ = body.append_child(&button);
            }
            if let Some(href) = &section.all_href
                && let Ok(link) = element(&document, "a", "dt-top__all")
            {
                let _ = link.set_attribute("href", href);
                link.set_text_content(Some(t("top.list.all")));
                let _ = body.append_child(&link);
            }
        })
    };

    for (index, section) in sections.iter().enumerate() {
        let chip = element(document, "button", "dt-top__chip")?;
        chip.set_attribute("type", "button")?;
        chip.set_text_content(Some(&section.title));
        let draw = Rc::clone(&draw);
        let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |_| draw(index));
        chip.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
        chips.append_child(&chip)?;
    }

    root.append_child(&chips)?;
    root.append_child(&body)?;
    // Open the first chip
    draw(0);
    Ok(root)
}

/// Build the own top page from the sections and insert it before `anchor`.
fn build(document: &Document, sections: Vec<Section>, anchor: &Element) -> Result<(), JsValue> {
    let root = element(document, "div", ROOT_CLASS)?;

    let onair = sections.iter().find(|s| s.role == Role::OnAir);
    let ranking = sections.iter().find(|s| s.role == Role::Ranking);

    // The showcase is the ranking, so the ranking is not a block of its own. A page
    // without a ranking uses the first section.
    let showcase = ranking
        .map(|s| (s, t("top.ranking")))
        .or_else(|| sections.first().map(|s| (s, s.title.as_str())));
    if let Some((section, label)) = showcase
        && !section.items.is_empty()
    {
        let view = render_showcase(document, section, label)?;
        root.append_child(&view)?;
    }

    // The daily view is the update, so it goes under the showcase
    if let Some(section) = onair {
        let row = render_row(document, section, INITIAL_ITEMS, false)?;
        root.append_child(&row)?;
    }

    let browse: Vec<Section> = sections
        .into_iter()
        .filter(|s| s.role == Role::Browse)
        .collect();
    if !browse.is_empty() {
        let view = render_browse(document, Rc::new(browse))?;
        root.append_child(&view)?;
    }

    let parent = anchor
        .parent_element()
        .ok_or_else(|| JsValue::from_str("no parent"))?;
    parent.insert_before(&root, Some(anchor))?;
    Ok(())
}

/// Build the top page. Returns the number of sections that were used.
pub fn render() -> Result<u32, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    // Do nothing if the page is already built
    if document
        .query_selector(&format!(".{ROOT_CLASS}"))?
        .is_some()
    {
        return Ok(0);
    }

    let nodes = document.query_selector_all(SECTION_SELECTOR)?;
    let mut sections = Vec::new();
    let mut used: Vec<Element> = Vec::new();
    // Where the own page goes. Not `used[0]`: the section of the rentals is also in that
    // list and it gives no cards, so it must not decide the place.
    let mut anchor: Option<Element> = None;
    for index in 0..nodes.length() {
        let Some(node) = nodes.item(index) else {
            continue;
        };
        let Ok(section) = node.dyn_into::<Element>() else {
            continue;
        };
        let Some(parsed) = parse_section(&section) else {
            // The rentals give no card, so hide that slider instead of leaving it
            if rental_filter_ready() && is_rental_section(&section) {
                used.push(section);
            }
            continue;
        };
        sections.push(parsed);
        if anchor.is_none() {
            anchor = Some(section.clone());
        }
        used.push(section);
    }

    // Fewer than two sliders means the site has not finished; wait
    if sections.len() < MIN_SECTIONS {
        return Ok(0);
    }

    let Some(anchor) = anchor else {
        return Ok(0);
    };
    build(&document, sections, &anchor)?;
    for section in &used {
        section.class_list().add_1(RENDERED_CLASS)?;
    }
    Ok(used.len() as u32)
}

/// Wait for the items and build at once.
///
/// The JS of the site inserts the items later, so one pass is not enough. A fixed
/// wait (0, 800, 2500ms) shows the original page until the next step also when the
/// items are already there, so look often.
///
/// While this runs, the CSS hides the sliders of the site
/// (`.pageWrapper:not(:has(.dt-top))`). After four seconds without a result, the
/// site is visible again (see `top-page.css`).
pub async fn install() {
    // Read the rentals while the site inserts its items (see `RENTAL_IDS`)
    spawn_local(start_rental_load());

    let mut seen = (0usize, 0u32);
    let mut stable = 0;
    let mut waited = 0;

    for _ in 0..POLL_ATTEMPTS {
        let counts = ready_counts();
        if counts.0 >= MIN_SECTIONS {
            // Wait while the site is still adding sections, but not past the moment at
            // which the CSS gives the sliders of the site back.
            if counts == seen {
                stable += 1;
            } else {
                stable = 0;
                seen = counts;
            }
            waited += POLL_INTERVAL_MS as u32;

            if stable >= SETTLE_TICKS || waited >= SETTLE_MAX_MS {
                match render() {
                    Ok(0) => {}
                    Ok(count) => {
                        log(&format!(
                            "トップを組み替えました（元セクション {count} 本 / 待ち {waited}ms）"
                        ));
                        return;
                    }
                    Err(err) => log(&format!("トップの組み替えに失敗: {err:?}")),
                }
            }
        }
        crate::sleep(POLL_INTERVAL_MS).await;
    }
    log("トップ: 組み替える帯が見つかりませんでした（サイトの表示のまま）");
}

/// How much the site has inserted: (sections with an item, items).
///
/// Both numbers are needed. A section appears before its items, so the number of
/// sections alone stops too early.
fn ready_counts() -> (usize, u32) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return (0, 0);
    };
    let Ok(sections) = document.query_selector_all(SECTION_SELECTOR) else {
        return (0, 0);
    };
    let mut with_items = 0;
    let mut items = 0;
    for index in 0..sections.length() {
        let Some(section) = sections
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        let Ok(found) = section.query_selector_all(ITEM_SELECTOR) else {
            continue;
        };
        if found.length() > 0 {
            with_items += 1;
            items += found.length();
        }
    }
    (with_items, items)
}

#[cfg(test)]
mod tests {
    use super::episode_label;

    #[test]
    fn uses_the_sites_label_when_it_is_usable() {
        // A usable form
        assert_eq!(
            episode_label(Some("第6話".into()), Some("28641"), Some("28641006")),
            Some("第6話".into())
        );
        // Only the number (measured)
        assert_eq!(
            episode_label(Some("6".into()), Some("28641"), Some("28641006")),
            Some("第6話".into())
        );
    }

    #[test]
    fn falls_back_to_part_id_when_the_label_is_broken() {
        // The site cut "Episode 12" from the left (the real DOM value)
        assert_eq!(
            episode_label(Some("...sode 12".into()), Some("28915"), Some("28915012")),
            Some("第12話".into())
        );
        // Without a label, the partId is enough
        assert_eq!(
            episode_label(None, Some("26311"), Some("26311136")),
            Some("第136話".into())
        );
    }

    #[test]
    fn gives_up_when_there_is_nothing_to_go_on() {
        // No partId (a card that goes to a work page)
        assert_eq!(episode_label(None, Some("29008"), None), None);
        // The partId does not start with the workId (the form changed)
        assert_eq!(episode_label(None, Some("11111"), Some("28915012")), None);
        // The episode number is only zeros
        assert_eq!(episode_label(None, Some("28915"), Some("28915000")), None);
    }
}
