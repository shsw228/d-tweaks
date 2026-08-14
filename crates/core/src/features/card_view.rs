//! Reads the cards of a list and draws own cards from the data.
//!
//! The site puts the work title in a different place on each page:
//!
//! | Page | Work title | Number | Subtitle |
//! |---|---|---|---|
//! | `mp_viw_pc` (continue) | `.textContainerIn > h2.line1` | `.number.line1` | `.episode.line1` |
//! | `mpa_hst_pc` (history) | `section > header > p.line2` | `.number.line1` | `h3.line2` |
//! | `mpa_fav_pc` (favorites) | `section > header > p.line2` | yes | `h3.line2` |
//! | `mpa_cmp_pc` (complete) | `h3.line2` | — | — |
//! | `c_all_pc` (all works) | `h3.line2` | — | — |
//!
//! `h3.line2` is the work title on some pages and the subtitle on others (the
//! `header` decides). CSS cannot absorb that difference without one exception per
//! page. A parse layer can, and then one `render` gives all pages the same look.
//!
//! The original DOM is hidden and not removed: the JS of the site reads it (edit
//! mode, the favorite toggle). The interactive elements (`.check`) stay. This
//! module makes text and links only.
//!
//! The site starts the playback with `href="void(0)"` and JS. A content script is
//! in an isolated world and cannot call that JS, but the URLs are known:
//!
//! - Play: `sc_d_pc?partId=`
//! - Work: `ci_pc?workId=`
//! - Work with an episode selected: `ci_pc?workId=&partId=`

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Element, Event, HtmlAnchorElement, HtmlInputElement, MutationObserver,
    MutationObserverInit, Url,
};

use d_tweaks_shared::text::t;

use crate::dom::{element, text_of};
use crate::features::comments;
use crate::log;

/// Marks an own card. The CSS hides the original DOM only where this is present.
const RENDERED_CLASS: &str = "dt-rendered";

#[derive(Clone)]
pub(crate) struct Badge {
    /// A class such as `dt-badge--new`. `None` gives the default look.
    pub(crate) modifier: Option<&'static str>,
    pub(crate) text: String,
}

/// Where a card goes on a click.
///
/// This was three `Option` fields (`work_id`, `part_id`, `href`). The order of
/// preference was not in the type, so code could read one field and ignore the
/// others. The hero of the top page did exactly that and a click did nothing.
/// An enum makes that error impossible.
#[derive(Clone)]
pub(crate) enum Link {
    /// Play the episode (`sc_d_pc?partId=`).
    ///
    /// With a `work_id`, the work title also links to the work page and the
    /// subtitle to the work page with this episode selected.
    Play {
        part_id: String,
        work_id: Option<String>,
    },
    /// The work page (`ci_pc?workId=`).
    Work { work_id: String },
    /// Anything else (a feature, a series, a book).
    External(String),
}

impl Link {
    /// The `partId`, if the card can play an episode.
    pub(crate) fn part_id(&self) -> Option<&str> {
        match self {
            Link::Play { part_id, .. } => Some(part_id),
            _ => None,
        }
    }

    /// The `workId`, if the card has a work page.
    pub(crate) fn work_id(&self) -> Option<&str> {
        match self {
            Link::Play { work_id, .. } => work_id.as_deref(),
            Link::Work { work_id } => Some(work_id),
            Link::External(_) => None,
        }
    }

    /// Destination of the main click area (the thumbnail).
    fn main_href(&self) -> String {
        match self {
            Link::Play { part_id, .. } => url_for("sc_d_pc", &[("partId", part_id)]),
            Link::Work { work_id } => url_for("ci_pc", &[("workId", work_id)]),
            Link::External(href) => href.clone(),
        }
    }
}

/// Build a destination from `work_id` and `part_id`. `None` if both are absent.
pub(crate) fn link_of(work_id: Option<String>, part_id: Option<String>) -> Option<Link> {
    match (work_id, part_id) {
        (work_id, Some(part_id)) => Some(Link::Play { part_id, work_id }),
        (Some(work_id), None) => Some(Link::Work { work_id }),
        (None, None) => None,
    }
}

/// The data of one card.
///
/// Both the parse of the original DOM (this module) and the JSON of the REST
/// interface (`search_overlay`) end here. Only `render` decides the look.
#[derive(Clone)]
pub(crate) struct Card {
    /// `None` makes a card that is not a link.
    pub(crate) link: Option<Link>,
    pub(crate) work: Option<String>,
    pub(crate) number: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) thumb: Option<String>,
    pub(crate) watched: bool,
    /// Inline width of `.progressCompleted`, for example `"94%"`.
    pub(crate) progress: Option<String>,
    pub(crate) badges: Vec<Badge>,
    /// Image size for this card only.
    ///
    /// The "no change" setting wins over this (see `set_thumb`). The top page has
    /// many cards on one screen, so it asks for a small size.
    pub(crate) thumb_size: Option<&'static str>,
}

/// The `width` of an inline style (`"width: 94%;"` gives `Some("94%")`).
///
/// An episode that nobody started has `width: 0%`. That would give every card an
/// empty progress bar, so 0 means "no progress".
fn parse_width(style: &str) -> Option<String> {
    let value = style.split("width").nth(1)?;
    let value = value.trim_start().strip_prefix(':')?;
    let value = value.split(';').next()?.trim();
    if value.is_empty() {
        return None;
    }
    // Read the number without the unit ("94%" gives 94)
    let numeric = value.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.');
    match numeric.parse::<f64>() {
        Ok(n) if n > 0.0 => Some(value.to_string()),
        _ => None,
    }
}

fn inline_width(el: &Element) -> Option<String> {
    parse_width(&el.get_attribute("style")?)
}

/// Which list to draw. The DOM of a card differs, so the parse differs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `.itemWrapper.clearfix > .itemModule` of the my-page lists and all works.
    List,
    /// `.itemModule` under `.episodeContainer` of a work page.
    Episode,
}

/// Parse an episode card.
///
/// The structure differs from a list card: there is no `.textContainerIn`, and
/// `.textContainer` holds `span.line1 > span.number` and `h3.line2` directly. The
/// work title is not used, because the page is the work page.
fn parse_episode(card: &Element) -> Card {
    // The partId is the end of the id of a[id^=episodePartId]
    let anchor = card
        .query_selector("a[id^=\"episodePartId\"]")
        .ok()
        .flatten();
    let part_id = anchor
        .as_ref()
        .map(|a| a.id())
        .and_then(|id| id.strip_prefix("episodePartId").map(str::to_string))
        .filter(|s| !s.is_empty());

    let watched = anchor
        .as_ref()
        .map(|a| a.class_list().contains("watched"))
        .unwrap_or(false);

    let thumb = card
        .query_selector(".thumbnailContainer img")
        .ok()
        .flatten()
        .and_then(|img| {
            img.get_attribute("data-src")
                .filter(|s| !s.is_empty())
                .or_else(|| img.get_attribute("src"))
        })
        .filter(|s| !s.is_empty() && !s.contains("lazySpace"));

    let progress = card
        .query_selector(".progressCompleted")
        .ok()
        .flatten()
        .and_then(|el| inline_width(&el))
        .or_else(|| if watched { Some("100%".into()) } else { None });

    let mut badges = Vec::new();
    if let Ok(icons) = card.query_selector_all("ul.optionIconContainer i.icon") {
        for i in 0..icons.length() {
            let Some(node) = icons.item(i) else { continue };
            let Ok(el) = node.dyn_into::<Element>() else {
                continue;
            };
            let class = el.class_name();
            if class.contains("iconTextNew") {
                badges.push(Badge {
                    modifier: Some("dt-badge--new"),
                    text: "NEW".into(),
                });
            } else if class.contains("iconTextComplete") {
                badges.push(Badge {
                    modifier: Some("dt-badge--complete"),
                    text: "COMPLETE".into(),
                });
            }
        }
    }

    Card {
        link: link_of(None, part_id),
        work: None,
        number: text_of(card, ".textContainer .number"),
        title: text_of(card, ".textContainer h3.line2"),
        thumb,
        watched,
        progress,
        badges,
        thumb_size: None,
    }
}

fn parse_card(card: &Element) -> Card {
    // workId and partId come from the query of a.textContainer
    let params = card
        .query_selector("a.textContainer[href]")
        .ok()
        .flatten()
        .and_then(|a| a.dyn_ref::<HtmlAnchorElement>().map(|a| a.href()))
        .and_then(|href| Url::new(&href).ok())
        .map(|url| url.search_params());

    let work_id = params.as_ref().and_then(|p| p.get("workId")).or_else(|| {
        card.query_selector("input.workId")
            .ok()
            .flatten()
            .and_then(|i| i.dyn_ref::<HtmlInputElement>().map(|i| i.value()))
            .filter(|v| !v.is_empty())
    });
    let part_id = params.as_ref().and_then(|p| p.get("partId"));

    // With a header, the header is the work title and h3.line2 the subtitle.
    // Without a header, h3.line2 is the work title.
    let header_title = text_of(card, "section > header p.line2");
    let has_header = header_title.is_some();
    let h3 = text_of(card, ".textContainerIn h3.line2");

    let work = header_title
        .or_else(|| text_of(card, ".textContainerIn h2.line1"))
        .or_else(|| if has_header { None } else { h3.clone() });

    let title =
        text_of(card, ".textContainerIn .episode.line1").or(if has_header { h3 } else { None });

    // Until lazysizes replaces it, src is a placeholder (img_lazySpace.gif), so
    // prefer data-src. It also has the higher resolution.
    let thumb = card
        .query_selector(".thumbnailContainer img")
        .ok()
        .flatten()
        .and_then(|img| {
            img.get_attribute("data-src")
                .filter(|s| !s.is_empty())
                .or_else(|| img.get_attribute("src"))
        })
        .filter(|s| !s.is_empty() && !s.contains("lazySpace"));

    let watched = card.class_list().contains("watched");

    let progress = card
        .query_selector(".progressCompleted")
        .ok()
        .flatten()
        .and_then(|el| inline_width(&el))
        .or_else(|| if watched { Some("100%".into()) } else { None });

    let mut badges = Vec::new();
    if let Ok(icons) = card.query_selector_all("ul.iconContainer i.icon") {
        for i in 0..icons.length() {
            let Some(node) = icons.item(i) else { continue };
            let Ok(el) = node.dyn_into::<Element>() else {
                continue;
            };
            let class = el.class_name();
            if class.contains("iconTextNew") {
                badges.push(Badge {
                    modifier: Some("dt-badge--new"),
                    text: "NEW".into(),
                });
            } else if class.contains("iconTextComplete") {
                badges.push(Badge {
                    modifier: Some("dt-badge--complete"),
                    text: "COMPLETE".into(),
                });
            }
        }
    }
    if let Ok(items) = card.query_selector_all("ul.option li") {
        for i in 0..items.length() {
            let Some(node) = items.item(i) else { continue };
            let Ok(el) = node.dyn_into::<Element>() else {
                continue;
            };
            if let Some(text) = el.text_content() {
                let text = text.trim();
                if !text.is_empty() {
                    badges.push(Badge {
                        modifier: None,
                        text: text.to_string(),
                    });
                }
            }
        }
    }

    Card {
        link: link_of(work_id, part_id),
        work,
        number: text_of(card, ".textContainerIn .number.line1"),
        title,
        thumb,
        watched,
        progress,
        badges,
        thumb_size: None,
    }
}

const THUMB_SIZE_DEFAULT: &str = "1";

thread_local! {
    /// The size from the settings. `None` means "no change".
    ///
    /// `chrome.storage` is asynchronous, so the value is read one time at the
    /// start. Until then the cards use the default and `refresh_thumbs` replaces
    /// the images.
    static THUMB_SIZE: std::cell::RefCell<Option<String>> =
        std::cell::RefCell::new(Some(THUMB_SIZE_DEFAULT.to_string()));
}

/// Set the size and replace the images of the cards that are already on screen.
/// Returns the number of images that changed.
///
/// A value that is not a number (`off`) stops the change, so the traffic stays the
/// same as on the site.
///
/// Set and replace are one function because a caller that does only one of the two
/// leaves the screen and the setting in disagreement.
pub fn apply_thumb_size(size: &str) -> Result<u32, JsValue> {
    let value = if !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()) {
        Some(size.to_string())
    } else {
        None
    };
    THUMB_SIZE.with_borrow_mut(|current| *current = value);
    refresh_thumbs()
}

fn thumb_size() -> Option<String> {
    THUMB_SIZE.with_borrow(|size| size.clone())
}

/// Change a thumbnail URL to another size. `None` if no change is possible.
///
/// The same image is available in these sizes; the number is at the end of the
/// file name (measured):
///
/// | Number | Size |
/// |---|---|
/// | `_3` `_4` | 144x81 |
/// | `_6` `_7` | 208x117 |
/// | `_5` | 256x144 |
/// | `_2` | 288x162 |
/// | `_8` | 380x214 |
/// | `_1` | 640x360 |
/// | `_9` | 1280x720 |
/// | `_10` | 1920x1080 |
///
/// Most pages give `_2` (288px), but the complete list gives `_6` (208px). That
/// is enough for the 208px frame of the site, but a full-width grid makes it
/// blurred.
///
/// So the target is `_1` (640x360). Not more, because a list has 20 cards or
/// more, and 640 is enough for a 320 CSS px card at twice the density.
pub(crate) fn resize_thumb(url: &str, size: Option<&str>) -> Option<String> {
    // A query for the cache can follow, so look at the path only
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    };
    let (stem, extension) = path.rsplit_once('.')?;
    let (head, size_segment) = stem.rsplit_once('_')?;
    if size_segment.is_empty() || !size_segment.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // "No change" in the settings wins over the size of the card
    let target = size
        .map(str::to_string)
        .filter(|_| thumb_size().is_some())
        .or_else(thumb_size)?;
    if size_segment == target {
        return None;
    }
    let mut upgraded = format!("{head}_{target}.{extension}");
    if let Some(query) = query {
        upgraded.push('?');
        upgraded.push_str(query);
    }
    Some(upgraded)
}

fn url_for(path: &str, query: &[(&str, &str)]) -> String {
    let mut url = format!("/animestore/{path}");
    for (i, (key, value)) in query.iter().enumerate() {
        url.push(if i == 0 { '?' } else { '&' });
        url.push_str(key);
        url.push('=');
        url.push_str(value);
    }
    url
}

/// Holds the URL that the site gave.
///
/// After a change the original is lost, so it stays here: the setting can go back,
/// and a size that does not exist can fall back to it.
const ORIGINAL_SRC_ATTR: &str = "data-dt-src";
/// Holds the size of this card, for the next replacement.
const THUMB_SIZE_ATTR: &str = "data-dt-size";

/// Marks an image whose larger size does not exist.
const THUMB_FAILED_ATTR: &str = "data-dt-thumb-failed";

/// Class of the large image of the top page. Also replaced on a size change.
pub(crate) const HERO_IMG_CLASS: &str = "dt-top__heroImg";

/// Put a thumbnail on an image. `src` must be the URL that the site gave.
///
/// `size` is the size of this card only, and "no change" in the settings wins.
/// `install_thumb_fallback` goes back to `src` if the new size does not exist.
pub(crate) fn set_thumb(img: &Element, src: &str, size: Option<&str>) -> Result<(), JsValue> {
    // Keep the size, so a replacement can choose the same one
    if let Some(size) = size {
        img.set_attribute(THUMB_SIZE_ATTR, size)?;
    }
    let Some(upgraded) = resize_thumb(src, size) else {
        return img.set_attribute("src", src);
    };
    img.set_attribute(ORIGINAL_SRC_ATTR, src)?;
    img.set_attribute("src", &upgraded)
}

/// Add one listener that puts back the original URL of an image that failed.
///
/// Do not add a listener per image. `Closure::once_into_js` frees itself only when
/// it is called, so every image that loaded keeps its closure for ever, and the
/// closure holds the `<img>`, so even an image out of the DOM stays. The float
/// search discards its cards on every word, so this accumulated with every key.
///
/// `error` does not bubble, but the capture phase receives it, so one listener is
/// enough for all images.
///
/// The test is the attribute `data-dt-src` and not a class, so every image that
/// this module changed goes back, not only the cards.
pub(crate) fn install_thumb_fallback() -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let on_error = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let Some(img) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
        else {
            return;
        };
        let Some(original) = img.get_attribute(ORIGINAL_SRC_ATTR) else {
            return;
        };
        // Mark it first, so a failure of the original URL does not repeat
        let _ = img.set_attribute(THUMB_FAILED_ATTR, "");
        let _ = img.set_attribute("src", &original);
    });
    document.add_event_listener_with_callback_and_bool(
        "error",
        on_error.as_ref().unchecked_ref(),
        true,
    )?;
    // Needed as long as the page lives, and there is only one
    on_error.forget();
    Ok(())
}

/// Replace the images to the size that the settings hold now.
///
/// The setting is in `chrome.storage` and only asynchronous, but the cards are
/// drawn at the start. To wait for the setting first would show the original cards
/// of the site, so the cards use the default and this function replaces the images
/// when the setting arrives.
///
/// The base is always the URL that the site gave (`data-dt-src`), so "no change"
/// can go back and more calls give the same result.
fn refresh_thumbs() -> Result<u32, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let images = document.query_selector_all(&format!(".dt-card__img, .{HERO_IMG_CLASS}"))?;
    let mut changed = 0;
    for index in 0..images.length() {
        let Some(node) = images.item(index) else {
            continue;
        };
        let Ok(img) = node.dyn_into::<Element>() else {
            continue;
        };
        // A size that does not exist gives the same result again
        if img.has_attribute(THUMB_FAILED_ATTR) {
            continue;
        }
        // Base: the original URL if present, else the current src
        let Some(original) = img
            .get_attribute(ORIGINAL_SRC_ATTR)
            .or_else(|| img.get_attribute("src"))
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let size = img.get_attribute(THUMB_SIZE_ATTR);
        let wanted = resize_thumb(&original, size.as_deref()).unwrap_or_else(|| original.clone());
        if img.get_attribute("src").as_deref() == Some(wanted.as_str()) {
            continue;
        }
        set_thumb(&img, &original, size.as_deref())?;
        changed += 1;
    }
    Ok(changed)
}

/// Destination of the main click area. `None` for a card without a link.
pub(crate) fn destination(card: &Card) -> Option<String> {
    card.link.as_ref().map(Link::main_href)
}

pub(crate) fn render(document: &Document, card: &Card) -> Result<Element, JsValue> {
    let root = element(document, "div", "dt-card")?;

    // Keep what the comment search needs, so nothing is parsed twice. A click on
    // the play link finds this with closest(".dt-card").
    let part_id = card.link.as_ref().and_then(Link::part_id);
    let work_id = card.link.as_ref().and_then(Link::work_id);
    for (attr, value) in [
        (comments::PART_ATTR, part_id),
        (comments::WORK_ATTR, card.work.as_deref()),
        (comments::NUMBER_ATTR, card.number.as_deref()),
    ] {
        if let Some(value) = value {
            root.set_attribute(attr, value)?;
        }
    }

    // Thumbnail and main click area. Play, or the work page without a partId.
    let main = element(document, "a", "dt-card__main")?;
    let main_href = destination(card);
    if let Some(href) = &main_href {
        main.set_attribute("href", href)?;
    }
    if let Some(src) = &card.thumb {
        let img = element(document, "img", "dt-card__img")?;
        set_thumb(&img, src, card.thumb_size)?;
        img.set_attribute("alt", "")?;
        img.set_attribute("loading", "lazy")?;
        main.append_child(&img)?;
    }
    let play = element(document, "span", "dt-card__play")?;
    main.append_child(&play)?;
    root.append_child(&main)?;

    // Progress
    if let Some(progress) = &card.progress {
        let bar = element(document, "div", "dt-card__progress")?;
        let fill = element(document, "span", "")?;
        fill.set_attribute("style", &format!("width:{progress}"))?;
        bar.append_child(&fill)?;
        root.append_child(&bar)?;
    }
    if card.watched {
        let mark = element(document, "span", "dt-card__watched")?;
        mark.set_text_content(Some(t("card.watched")));
        root.append_child(&mark)?;
    }

    // Text under the thumbnail
    let body = element(document, "div", "dt-card__body")?;

    if !card.badges.is_empty() {
        let badges = element(document, "div", "dt-card__badges")?;
        for badge in &card.badges {
            let class = match badge.modifier {
                Some(modifier) => format!("dt-badge {modifier}"),
                None => "dt-badge".to_string(),
            };
            let el = element(document, "span", &class)?;
            el.set_text_content(Some(&badge.text));
            badges.append_child(&el)?;
        }
        body.append_child(&badges)?;
    }

    // The work title goes to the work page, without a partId
    if let Some(work) = &card.work {
        let tag = if work_id.is_some() { "a" } else { "span" };
        let el = element(document, tag, "dt-card__work")?;
        el.set_text_content(Some(work));
        if let Some(id) = work_id {
            el.set_attribute("href", &url_for("ci_pc", &[("workId", id)]))?;
        }
        body.append_child(&el)?;
    }

    if let Some(number) = &card.number {
        let el = element(document, "span", "dt-card__number")?;
        el.set_text_content(Some(number));
        body.append_child(&el)?;
    }

    // The subtitle goes to the work page with this episode selected
    if let Some(title) = &card.title {
        // Only possible when both ids are known
        let episode = work_id.zip(part_id);
        let el = element(
            document,
            if episode.is_some() { "a" } else { "span" },
            "dt-card__title",
        )?;
        el.set_text_content(Some(title));
        if let Some((work, part)) = episode {
            el.set_attribute(
                "href",
                &url_for("ci_pc", &[("workId", work), ("partId", part)]),
            )?;
        }
        body.append_child(&el)?;
    }

    root.append_child(&body)?;
    Ok(root)
}

/// Parse the cards under `source` and put own cards into `target`.
///
/// Unlike `render_in`, this does not touch the original DOM. The caller hides the
/// original container. Use it where the site has no interactive element that must
/// stay, for example the episode list.
pub fn render_into(source: &Element, target: &Element, kind: Source) -> Result<u32, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let selector = match kind {
        Source::List => ":scope > .itemModule",
        Source::Episode => ".itemModule",
    };
    let cards = source.query_selector_all(selector)?;
    let mut rendered = 0;

    // Hundreds of cards, so insert them in one operation
    let fragment = document.create_document_fragment();
    for index in 0..cards.length() {
        let Some(node) = cards.item(index) else {
            continue;
        };
        let Ok(card) = node.dyn_into::<Element>() else {
            continue;
        };
        let model = match kind {
            Source::List => parse_card(&card),
            Source::Episode => parse_episode(&card),
        };
        if model.work.is_none() && model.title.is_none() && model.thumb.is_none() {
            continue;
        }
        let view = render(&document, &model)?;
        fragment.append_child(&view)?;
        rendered += 1;
    }
    target.append_child(&fragment)?;
    Ok(rendered)
}

pub fn render_in(root: &Element, source: Source) -> Result<u32, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    // A list card is a direct child. An episode card is under a .swiper-slide.
    let selector = match source {
        Source::List => ":scope > .itemModule",
        Source::Episode => ".itemModule",
    };
    let cards = root.query_selector_all(selector)?;
    let mut rendered = 0;

    for i in 0..cards.length() {
        let Some(node) = cards.item(i) else { continue };
        let Ok(card) = node.dyn_into::<Element>() else {
            continue;
        };
        // The infinite scroll comes back here, so skip what is done
        if card.class_list().contains(RENDERED_CLASS) {
            continue;
        }

        let model = match source {
            Source::List => parse_card(&card),
            Source::Episode => parse_episode(&card),
        };
        // Without a title and without a thumbnail, leave the original
        if model.work.is_none() && model.title.is_none() && model.thumb.is_none() {
            continue;
        }

        let view = render(&document, &model)?;
        card.append_child(&view)?;
        card.class_list().add_1(RENDERED_CLASS)?;
        rendered += 1;
    }

    Ok(rendered)
}

/// Watch `root` and draw the cards that arrive later.
///
/// One pass is not enough: `/animestore/CF/*` (new episodes and others) returns
/// only the frame of the list in the HTML, and the JS inserts the cards later. At
/// the time of the content script there are zero cards.
///
/// The CSS is already active at that moment, so the failure looks like this: the
/// original cards stand in the own grid. Measured: 20 cards, then 60.
///
/// The paging and the filters of the site also replace cards, so the observer
/// stays.
pub fn observe(root: &Element, source: Source) -> Result<(), JsValue> {
    let target = root.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        // `render_in` skips what is done, so more calls do no harm
        match render_in(&target, source) {
            Ok(0) => {}
            Ok(count) => log(&format!("後から追加されたカードを描画: {count} 件")),
            Err(err) => log(&format!("追加カードの描画に失敗: {err:?}")),
        }
    });

    let observer = MutationObserver::new(callback.as_ref().unchecked_ref())?;
    let options = MutationObserverInit::new();
    options.set_child_list(true);
    observer.observe_with_options(root, &options)?;

    // The observer must live as long as the page
    callback.forget();
    std::mem::forget(observer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_width, resize_thumb};

    #[test]
    fn resizes_thumbnail_size_suffix() {
        // The 208px of the complete list becomes 640px. The query stays.
        assert_eq!(
            resize_thumb(
                "https://cs1.example/anime/1a6F5A/28790_1_6.png?1774855808677=",
                None
            ),
            Some("https://cs1.example/anime/1a6F5A/28790_1_1.png?1774855808677=".into())
        );
        // The 288px of the other pages also changes
        assert_eq!(
            resize_thumb("https://cs1.example/anime/AoZY1Q/29056006_1_2.png", None),
            Some("https://cs1.example/anime/AoZY1Q/29056006_1_1.png".into())
        );
        // No change when the size is already correct
        assert_eq!(
            resize_thumb("https://cs1.example/a/28790_1_1.png", None),
            None
        );
        // No size, or a size that is not a number: no change
        assert_eq!(
            resize_thumb("https://cs1.example/a/keyvisual.png", None),
            None
        );
        assert_eq!(
            resize_thumb("https://cs1.example/a/28790_1_x.png", None),
            None
        );
        assert_eq!(
            resize_thumb("https://cs1.example/a/noextension", None),
            None
        );
    }

    #[test]
    fn parses_progress_width_and_treats_zero_as_absent() {
        assert_eq!(parse_width("width: 94%;"), Some("94%".into()));
        assert_eq!(parse_width("width:100%"), Some("100%".into()));
        assert_eq!(parse_width("width: 0.5%;"), Some("0.5%".into()));
        // An episode that nobody started has 0%, and gets no progress bar
        assert_eq!(parse_width("width: 0%;"), None);
        assert_eq!(parse_width("width:0"), None);
        // No width, or a broken value
        assert_eq!(parse_width("color: red;"), None);
        assert_eq!(parse_width("width:"), None);
        assert_eq!(parse_width(""), None);
    }
}
