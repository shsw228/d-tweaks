//! Draws the head of a work page (`ci_pc`) as a full-width hero.
//!
//! The original head is 944px wide, centred, and from the top:
//!
//! | Content | Height |
//! |---|---|
//! | Key visuals (one large, two small) | 354px |
//! | Title, my-list and favorite | 65px |
//! | 1080p, free episodes, related books | 19px |
//!
//! That is 563px, one screen before the episode list. The list below is a full-width
//! grid, so the head looks like an island in the middle.
//!
//! CSS would have to remove the fixed width, the white background and the table display
//! of the site one by one, which gives lost specificity battles and exceptions (one
//! attempt left the image small and the white band visible). Like the cards of a list,
//! this module reads the values and builds its own elements.
//!
//! The interactive controls stay: the my-list and the favorite belong to the JS of the
//! site. The original DOM stays and the CSS puts it at the bottom right of the hero (the
//! same as `.check` in `card_view`).

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlAnchorElement, MutationObserver, MutationObserverInit};

use crate::log;

/// Marks the page as drawn. The CSS hides the original DOM only with this class.
const RENDERED_CLASS: &str = "dt-hero-rendered";
/// Size of the key visual. `_10` is 1920x1080 (measured).
const HERO_IMAGE_SIZE: &str = "10";
/// Marks the image on screen.
const CURRENT_CLASS: &str = "is-current";
/// Interval between the images. Text is over them, so this is slow.
const SLIDE_INTERVAL_MS: i32 = 7000;

fn text_of(root: &Element, selector: &str) -> Option<String> {
    let el = root.query_selector(selector).ok()??;
    let text = el.text_content()?.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn href_of(root: &Element, selector: &str) -> Option<String> {
    let el = root.query_selector(selector).ok()??;
    el.dyn_ref::<HtmlAnchorElement>().map(|a| a.href())
}

/// Change an image URL to the largest size (1920x1080).
///
/// The file name is `<workId>_1_<size>.png` or `<workId>_1_<size>_<hash>.png`, so the
/// third part is the size (measured: `_1` 640x360, `_9` 1280x720, `_10` 1920x1080).
///
/// The hero is as wide as the window, so the 1280 of the site is too small on a large
/// screen.
fn upgrade_key_visual(url: &str) -> Option<String> {
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    };
    let (base, file) = path.rsplit_once('/')?;
    let (stem, extension) = file.rsplit_once('.')?;

    let mut parts: Vec<&str> = stem.split('_').collect();
    if parts.len() < 3 || !parts[2].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    parts[2] = HERO_IMAGE_SIZE;

    let mut upgraded = format!("{base}/{}.{extension}", parts.join("_"));
    if let Some(query) = query {
        upgraded.push('?');
        upgraded.push_str(query);
    }
    Some(upgraded)
}

fn element(document: &Document, tag: &str, class: &str) -> Result<Element, JsValue> {
    let el = document.create_element(tag)?;
    el.set_class_name(class);
    Ok(el)
}

/// Add one chip of the meta row (`1080p`, a rank).
fn add_chip(
    document: &Document,
    row: &Element,
    class: &str,
    text: &str,
    href: Option<&str>,
) -> Result<(), JsValue> {
    let el = element(document, if href.is_some() { "a" } else { "span" }, class)?;
    el.set_text_content(Some(text));
    if let Some(href) = href {
        el.set_attribute("href", href)?;
    }
    row.append_child(&el)?;
    Ok(())
}

/// Draw the head of a work page. `true` if it was drawn.
pub fn render() -> Result<bool, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let Some(header) = document.query_selector(".productWrapperIn")? else {
        return Ok(false);
    };
    // A page without a key visual has another layout; leave it
    if header.query_selector(".keyVisual")?.is_none() {
        return Ok(false);
    }
    if header.class_list().contains(RENDERED_CLASS) {
        return Ok(false);
    }

    let Some(title) = text_of(&header, ".titleWrap h1") else {
        log("ヒーロー: タイトルが取れないので元のままにします");
        return Ok(false);
    };

    let hero = element(&document, "div", "dt-hero")?;

    // The images. A work page can have more than one, and all are used.
    let slides = build_slides(&document, &header, &hero)?;

    // A shade, so the text is readable
    let shade = element(&document, "div", "dt-hero__shade")?;
    hero.append_child(&shade)?;

    let body = element(&document, "div", "dt-hero__body")?;
    let heading = element(&document, "h2", "dt-hero__title")?;
    heading.set_text_content(Some(&title));
    body.append_child(&heading)?;

    let meta = element(&document, "div", "dt-hero__meta")?;
    if let Some(quality) = text_of(&header, ".titleWrap ul.optionIconContainer li") {
        add_chip(&document, &meta, "dt-hero__chip", &quality, None)?;
    }
    if let Some(campaign) = text_of(&header, ".c-campaignBubble") {
        add_chip(
            &document,
            &meta,
            "dt-hero__chip dt-hero__chip--free",
            &campaign,
            None,
        )?;
    }
    if let Some(rank) = text_of(&header, ".watchRankingCount__text") {
        let href = href_of(&header, ".watchRankingCount__text");
        add_chip(&document, &meta, "dt-hero__link", &rank, href.as_deref())?;
    }
    // The number of favorites belongs next to the button, so it is not here. The
    // related books are not shown.
    if meta.child_element_count() > 0 {
        body.append_child(&meta)?;
    }

    // My-list, favorite, number of favorites
    let actions = element(&document, "div", "dt-hero__actions")?;
    if let Some(site) = header.query_selector(".actionArea .btnAddMyList")? {
        let button = action_button(
            &document,
            "dt-action dt-action--mylist",
            "マイリスト",
            &site,
        )?;
        actions.append_child(&button)?;
    }
    if let Some(site) = header.query_selector(".actionArea .btnConcerned")? {
        let button = action_button(&document, "dt-action dt-action--favo", "気になる", &site)?;
        watch_favorite_state(&button, &site)?;
        actions.append_child(&button)?;
    }
    if let Some(count) = text_of(&header, ".actionArea .favoriteCount") {
        let note = element(&document, "span", "dt-hero__note")?;
        note.set_text_content(Some(&count));
        actions.append_child(&note)?;
    }
    if actions.child_element_count() > 0 {
        body.append_child(&actions)?;
    }

    hero.append_child(&body)?;

    // Insert at the start of the original DOM; the controls of the site stay
    match header.first_element_child() {
        Some(first) => {
            header.insert_before(&hero, Some(&first))?;
        }
        None => {
            header.append_child(&hero)?;
        }
    }
    header.class_list().add_1(RENDERED_CLASS)?;

    // With more than one image, change them slowly
    if slides.len() > 1 {
        start_slideshow(slides);
    }
    Ok(true)
}

/// The classes that the site puts on the favorite control when it is on (measured).
///
/// Off: `btnConcerned favo ui-favo` (the icon is `iconHeart`).
/// On: `btnConcerned favo checked on` (the icon is `iconCheck`).
const FAVORITE_ON_CLASSES: [&str; 2] = ["checked", "on"];
/// Marks the own button as on.
const ACTION_ON_CLASS: &str = "is-on";

/// The element of the control of the site that must receive the click.
///
/// `.btnConcerned` is an `<a>`, but `.btnAddMyList` is a `<div>` with a `<label>` in it.
/// The handler can be on the inner element, so use an interactive element if there is
/// one.
fn click_target(site: &Element) -> Element {
    site.query_selector("a, button, label, input")
        .ok()
        .flatten()
        .unwrap_or_else(|| site.clone())
}

/// An own button that sends its click to the control of the site.
///
/// A `click()` also works on a hidden element: it calls the handler and not the
/// appearance. The dialog of the my-list and the favorite request stay with the site.
fn action_button(
    document: &Document,
    class: &str,
    label: &str,
    site: &Element,
) -> Result<Element, JsValue> {
    let button = element(document, "button", class)?;
    button.set_attribute("type", "button")?;
    button.set_text_content(Some(label));

    let target = click_target(site);
    let on_click = Closure::<dyn FnMut()>::new(move || {
        target.unchecked_ref::<web_sys::HtmlElement>().click();
    });
    button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    on_click.forget();
    Ok(button)
}

/// Put the state of the favorite on the own button.
fn apply_favorite_state(button: &Element, site: &Element) {
    let on = site
        .class_name()
        .split_whitespace()
        .any(|class| FAVORITE_ON_CLASSES.contains(&class));
    let list = button.class_list();
    let _ = if on {
        list.add_1(ACTION_ON_CLASS)
    } else {
        list.remove_1(ACTION_ON_CLASS)
    };
}

/// Follow the state of the favorite.
///
/// The JS of the site writes the result of a click into its classes, so this reads them.
/// This module holds no state of its own; the site is the source.
fn watch_favorite_state(button: &Element, site: &Element) -> Result<(), JsValue> {
    apply_favorite_state(button, site);

    let button = button.clone();
    let watched = site.clone();
    let callback = Closure::<dyn FnMut()>::new(move || apply_favorite_state(&button, &watched));
    let observer = MutationObserver::new(callback.as_ref().unchecked_ref())?;
    let options = MutationObserverInit::new();
    options.set_attributes(true);
    observer.observe_with_options(site, &options)?;

    // Watch as long as the page lives
    callback.forget();
    std::mem::forget(observer);
    Ok(())
}

/// Own container for the episodes and the details.
///
/// With both in the same parent, a wide screen can put them side by side. A grid on
/// `.accordionWrapper` itself would also take the notes and the book section.
const WORK_CONTAINER_CLASS: &str = "dt-work";

/// The container for the episodes and the details. Makes it if it is absent.
///
/// The episodes and the details are drawn independently (some works have only one), so
/// both the get and the make are here and the order does not matter.
pub(crate) fn work_container(document: &Document) -> Result<Option<Element>, JsValue> {
    let Some(wrapper) = document.query_selector(".accordionWrapper")? else {
        return Ok(None);
    };
    if let Some(existing) = wrapper.query_selector(&format!(":scope > .{WORK_CONTAINER_CLASS}"))? {
        return Ok(Some(existing));
    }

    let container = element(document, "div", WORK_CONTAINER_CLASS)?;
    // Put it where the original episode list is, so the reading order stays
    match wrapper.query_selector(".episodeWrapper")? {
        Some(episodes) => {
            wrapper.insert_before(&container, Some(&episodes))?;
        }
        None => {
            wrapper.append_child(&container)?;
        }
    }
    Ok(Some(container))
}

/// Marks the original episode list. The CSS hides only a list with this class.
const EPISODES_HIDDEN_CLASS: &str = "dt-episodes-rendered";

/// Draw the episodes as an own section. `true` if it was drawn.
///
/// The episode list of the site is a Swiper (horizontal), with a fold on a narrow
/// screen, an SP close button and a section title. CSS on that gives exceptions:
///
/// - A fixed `display` for the fold stops the open and close.
/// - The SP close button takes one cell of the grid.
/// - The inline width and transform of the Swiper must be removed again and again.
///
/// The thumbnails are enough, so this module parses them into an own container and hides
/// the original list.
///
/// An episode has no interactive control (unlike a card of a list), so the original DOM
/// is not necessary.
pub fn render_episodes() -> Result<bool, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let Some(wrapper) = document.query_selector(".episodeWrapper")? else {
        return Ok(false);
    };
    if wrapper.class_list().contains(EPISODES_HIDDEN_CLASS) {
        return Ok(false);
    }
    let Some(container) = wrapper.query_selector(".episodeContainer")? else {
        return Ok(false);
    };

    let section = element(&document, "div", "dt-episodes")?;
    let count = crate::features::card_view::render_into(
        &container,
        &section,
        crate::features::card_view::Source::Episode,
    )?;
    if count == 0 {
        return Ok(false);
    }

    let Some(container) = work_container(&document)? else {
        return Ok(false);
    };
    container.append_child(&section)?;
    wrapper.class_list().add_1(EPISODES_HIDDEN_CLASS)?;
    log(&format!("エピソードを自前セクションで描画: {count} 件"));
    Ok(true)
}

/// Collect the key visuals and put them on each other. Returns the `<img>` elements.
///
/// A work page can have one large image and two images of episodes (measured:
/// `29079_1_1.png`, `29079001_1_2.png`, `29079002_1_1.png`). The original layout puts
/// the three side by side over 354px; the hero shows them one after the other.
fn build_slides(
    document: &Document,
    header: &Element,
    hero: &Element,
) -> Result<Vec<Element>, JsValue> {
    let sources = document_sources(header)?;
    let mut slides = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let picture = element(document, "img", "dt-hero__image")?;
        picture.set_attribute(
            "src",
            upgrade_key_visual(source).as_deref().unwrap_or(source),
        )?;
        picture.set_attribute("alt", "")?;
        // Only the first image is visible at the start
        if index == 0 {
            picture.class_list().add_1(CURRENT_CLASS)?;
        }
        hero.append_child(&picture)?;
        slides.push(picture);
    }
    Ok(slides)
}

/// Collect the URLs of the key visuals, without a repetition.
fn document_sources(header: &Element) -> Result<Vec<String>, JsValue> {
    let nodes = header.query_selector_all(".keyVisual img")?;
    let mut sources: Vec<String> = Vec::new();
    for index in 0..nodes.length() {
        let Some(node) = nodes.item(index) else {
            continue;
        };
        let Ok(img) = node.dyn_into::<Element>() else {
            continue;
        };
        // After lazyload, src is the real URL. Before that, use data-src.
        let source = img
            .get_attribute("src")
            .filter(|s| !s.is_empty() && !s.contains("lazySpace"))
            .or_else(|| img.get_attribute("data-src"))
            .filter(|s| !s.is_empty());
        if let Some(source) = source
            && !sources.contains(&source)
        {
            sources.push(source);
        }
    }
    Ok(sources)
}

/// Change the images. Stops when they leave the page.
fn start_slideshow(slides: Vec<Element>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let timer = Rc::new(Cell::new(0));
    let index = Rc::new(Cell::new(0usize));

    let tick = {
        let timer = Rc::clone(&timer);
        let index = Rc::clone(&index);
        Closure::<dyn FnMut()>::new(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            // Stop after the page changed
            let Some(first) = slides.first() else { return };
            if !first.is_connected() {
                window.clear_interval_with_handle(timer.get());
                return;
            }
            let current = index.get();
            let next = (current + 1) % slides.len();
            let _ = slides[current].class_list().remove_1(CURRENT_CLASS);
            let _ = slides[next].class_list().add_1(CURRENT_CLASS);
            index.set(next);
        })
    };

    match window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        SLIDE_INTERVAL_MS,
    ) {
        Ok(id) => {
            timer.set(id);
            tick.forget();
        }
        Err(err) => log(&format!("ヒーローの切り替えを始められません: {err:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::upgrade_key_visual;

    #[test]
    fn upgrades_key_visual_to_the_largest_size() {
        // With a hash (the form after lazyload)
        assert_eq!(
            upgrade_key_visual("https://cs1.example/a/29056_1_9_8b.png"),
            Some("https://cs1.example/a/29056_1_10_8b.png".into())
        );
        // Without a hash (the form of data-src). The query stays.
        assert_eq!(
            upgrade_key_visual("https://cs1.example/a/29056_1_3.png?1780461308349"),
            Some("https://cs1.example/a/29056_1_10.png?1780461308349".into())
        );
        // The largest size gives the same URL again, which does no harm
        assert_eq!(
            upgrade_key_visual("https://cs1.example/a/29056_1_10.png"),
            Some("https://cs1.example/a/29056_1_10.png".into())
        );
        // An unknown form is not changed
        assert_eq!(
            upgrade_key_visual("https://cs1.example/a/keyvisual.png"),
            None
        );
        assert_eq!(
            upgrade_key_visual("https://cs1.example/a/29056_1_x.png"),
            None
        );
        assert_eq!(upgrade_key_visual("no-slash.png"), None);
    }
}
