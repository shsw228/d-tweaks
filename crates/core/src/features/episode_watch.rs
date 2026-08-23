//! The button on a work page that registers the work for the new-episode watch.
//!
//! # The page is also a check
//!
//! The button sends the episode list that the page already has. The service worker keeps
//! the last episode of that list and asks the interface for it later (see
//! `crates/background/src/watch.rs`), so:
//!
//! - The episodes that are on the page when the user presses the button are not new.
//! - Every later visit of the page resyncs that mark **without one request**, and it
//!   records what was added since the last visit.
//!
//! So a work that the user opens often is watched for free, and the periodic check only
//! carries the works that the user does not open.
//!
//! # Where the button goes
//!
//! Next to the my-list and the favorite of the site: it is an action on the work, and a
//! user looks for it there. With `work-hero` on, those controls are the own hero
//! (`.dt-hero__actions`), which is drawn in a task, so this waits for it. With the hero
//! off, the control area of the site is the place.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlElement};

use d_tweaks_shared::text::{t, t_fill};
use d_tweaks_shared::{chrome, json, messages, settings, watch};

use crate::dom::{document, element, text_of};
use crate::{log, sleep};

/// The own bar with the button. Also the test for "is it already there".
const BAR_CLASS: &str = "dt-watch";
const BUTTON_CLASS: &str = "dt-watch__button";
const NOTE_CLASS: &str = "dt-watch__note";
/// Marks the button as on.
const ON_CLASS: &str = "is-on";

/// The links of the episode list of the site. `card_view` uses the same mark.
const EPISODE_LINK_SELECTOR: &str = ".episodeWrapper a[id^=\"episodePartId\"]";
/// The controls of the site (my-list, favorite).
const SITE_ACTIONS_SELECTOR: &str = ".actionArea";
/// The same controls after `work_hero` drew them.
const HERO_ACTIONS_SELECTOR: &str = ".dt-hero__actions";
/// The work title in the head of a work page. `comments` uses the same one.
const PAGE_TITLE_SELECTOR: &str = ".titleWrap h1";

/// Size of the image that the popup draws. `2` is 288x162, the smallest that the site has.
const POPUP_THUMB_SIZE: &str = "2";

/// Tries while `work_hero` draws. The hero waits for a settings read, so it is not there
/// at once; after this the control area of the site is used.
const HERO_TRIES: u32 = 10;
const HERO_WAIT_MS: i32 = 200;

/// One episode of the list of the site.
struct Episode {
    part_id: String,
    number: Option<String>,
    title: Option<String>,
    /// The still of the episode. The popup draws it, so the row has a picture without one
    /// more request to the site.
    thumb: Option<String>,
}

/// The `workId` of this page.
///
/// Only the query of the address. A `workId` that is built out of a `partId` would be a
/// guess (an id can also start with `C`), and this value is the key of the entry in the
/// storage and the address in the popup, so a guess would be wrong in a way that is not
/// visible. Without the query, the button is not drawn.
fn work_id() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params
        .get("workId")
        .filter(|id| !id.trim().is_empty())
        .map(|id| id.trim().to_string())
}

/// The work title without the number of episodes.
///
/// The heading of the site is `ワンピース ワノ国編（全197話）` (measured). That number is
/// the reason this feature exists, so it is exactly the part that becomes wrong: the list
/// in the popup would keep saying 197 after the 198th episode arrived.
///
/// Only a group at the end that begins with `全` is removed. A work whose name has
/// brackets in it (`『…』`) keeps them.
fn work_title(heading: &str) -> String {
    let text = heading.trim();
    let Some(open) = text.rfind('（') else {
        return text.to_string();
    };
    if !text.ends_with('）') {
        return text.to_string();
    }
    let inside = &text[open + '（'.len_utf8()..text.len() - '）'.len_utf8()];
    if !inside.starts_with('全') {
        return text.to_string();
    }
    text[..open].trim_end().to_string()
}

/// The episode list of the site, in the order of the page.
///
/// The own UI does not replace this DOM: `card_view` reads it and the CSS hides it, so
/// the original list is here whether or not the own episode grid was drawn.
fn episodes(document: &Document) -> Vec<Episode> {
    let Ok(nodes) = document.query_selector_all(EPISODE_LINK_SELECTOR) else {
        return Vec::new();
    };
    let mut list = Vec::with_capacity(nodes.length() as usize);
    for index in 0..nodes.length() {
        let Some(node) = nodes.item(index) else {
            continue;
        };
        let Ok(anchor) = node.dyn_into::<Element>() else {
            continue;
        };
        let Some(part_id) = anchor
            .id()
            .strip_prefix("episodePartId")
            .map(str::to_string)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        // The same places as `card_view::parse_episode`
        list.push(Episode {
            part_id,
            number: text_of(&anchor, ".textContainer .number"),
            title: text_of(&anchor, ".textContainer h3.line2"),
            thumb: thumb_of(&anchor),
        });
    }
    list
}

/// The still of an episode card, in the size that a row of the popup needs.
///
/// Before the lazyload the address is in `data-src`, and `src` is a placeholder
/// (`lazySpace`), so both are read. The same places as `card_view::parse_episode`.
fn thumb_of(anchor: &Element) -> Option<String> {
    let image = anchor.query_selector(".thumbnailContainer img").ok()??;
    let url = image
        .get_attribute("data-src")
        .filter(|s| !s.is_empty())
        .or_else(|| image.get_attribute("src"))
        .filter(|s| !s.is_empty() && !s.contains("lazySpace"))?;
    // A row of the popup is 72px wide, so the smallest of the site is enough
    Some(watch::thumb_at_size(&url, POPUP_THUMB_SIZE))
}

/// The episode list as the message wants it.
fn episodes_to_js(list: &[Episode]) -> Result<js_sys::Array, JsValue> {
    let array = js_sys::Array::new();
    for episode in list {
        array.push(
            &json::object(&[
                ("partId", JsValue::from_str(&episode.part_id)),
                (
                    "number",
                    episode
                        .number
                        .as_deref()
                        .map(JsValue::from_str)
                        .unwrap_or(JsValue::NULL),
                ),
                (
                    "title",
                    episode
                        .title
                        .as_deref()
                        .map(JsValue::from_str)
                        .unwrap_or(JsValue::NULL),
                ),
                (
                    "thumb",
                    episode
                        .thumb
                        .as_deref()
                        .map(JsValue::from_str)
                        .unwrap_or(JsValue::NULL),
                ),
            ])?
            .into(),
        );
    }
    Ok(array)
}

/// Send one of the watch messages with the work and its episode list.
async fn send(
    kind: &str,
    work_id: &str,
    title: &str,
    list: &[Episode],
) -> Result<JsValue, JsValue> {
    let message = json::object(&[
        ("type", JsValue::from_str(kind)),
        ("workId", JsValue::from_str(work_id)),
        ("title", JsValue::from_str(title)),
        ("episodes", episodes_to_js(list)?.into()),
    ])?;
    chrome::send_message(&message.into()).await
}

/// Put the state on the button.
fn apply_state(button: &Element, watched: bool) {
    button.set_text_content(Some(t(if watched { "watch.on" } else { "watch.add" })));
    let _ = button.set_attribute(
        "title",
        t(if watched {
            "watch.remove.title"
        } else {
            "watch.add.title"
        }),
    );
    let list = button.class_list();
    let _ = if watched {
        list.add_1(ON_CLASS)
    } else {
        list.remove_1(ON_CLASS)
    };
}

/// Where the button goes. `None` when the page has neither place.
async fn host(document: &Document) -> Option<Element> {
    // The hero is drawn in a task, so it can arrive after this code
    if settings::is_enabled("work-hero").await {
        for _ in 0..HERO_TRIES {
            if let Ok(Some(actions)) = document.query_selector(HERO_ACTIONS_SELECTOR) {
                return Some(actions);
            }
            sleep(HERO_WAIT_MS).await;
        }
    }
    document
        .query_selector(SITE_ACTIONS_SELECTOR)
        .ok()
        .flatten()
}

/// Draw the button on the work page.
pub async fn install() {
    let Ok(document) = document() else { return };
    let Some(work_id) = work_id() else {
        log("新着の見張り: workId が読めないので出さない");
        return;
    };

    let list = episodes(&document);
    if list.is_empty() {
        // Without a list there is no mark to start from, and the check could not tell a
        // new episode from the first one. A logged-out page is also this case.
        log("新着の見張り: エピソード一覧が無いので出さない（未ログインの可能性）");
        return;
    }
    let title = document
        .query_selector(PAGE_TITLE_SELECTOR)
        .ok()
        .flatten()
        .and_then(|el| el.text_content())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        // `<title>` is "作品名 | アニメ動画見放題 | dアニメストア"
        .or_else(|| document.title().split(" | ").next().map(str::to_string))
        .map(|text| work_title(&text))
        .unwrap_or_default();

    // Ask the state. The reply also tells the service worker what the page holds, so a
    // visit is a check that costs no request.
    let watched = match send(messages::WATCH_STATE, &work_id, &title, &list).await {
        Ok(reply) => json::get(&reply, "watched")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        Err(err) => {
            log(&format!("新着の見張り: 状態を読めません: {err:?}"));
            false
        }
    };

    let Some(host) = host(&document).await else {
        log("新着の見張り: 置き場所が無いので出さない");
        return;
    };
    if host
        .query_selector(&format!(".{BAR_CLASS}"))
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }

    match build(&document, &host, &work_id, &title, list, watched) {
        Ok(()) => log(&format!(
            "新着の見張り: ボタンを出した（{}）",
            if watched { "登録済み" } else { "未登録" }
        )),
        Err(err) => log(&format!("新着の見張り: ボタンを出せません: {err:?}")),
    }
}

fn build(
    document: &Document,
    host: &Element,
    work_id: &str,
    title: &str,
    list: Vec<Episode>,
    watched: bool,
) -> Result<(), JsValue> {
    let bar = element(document, "div", BAR_CLASS)?;
    let button: Element = element(document, "button", BUTTON_CLASS)?;
    button.set_attribute("type", "button")?;
    apply_state(&button, watched);
    let note = element(document, "span", NOTE_CLASS)?;
    bar.append_child(&button)?;
    bar.append_child(&note)?;
    host.append_child(&bar)?;

    let clicked = button.clone();
    let state = std::rc::Rc::new(std::cell::Cell::new(watched));
    let list = std::rc::Rc::new(list);
    let work_id = work_id.to_string();
    let title = title.to_string();

    let on_click = Closure::<dyn FnMut()>::new(move || {
        let button = button.clone();
        let note = note.clone();
        let state = std::rc::Rc::clone(&state);
        let list = std::rc::Rc::clone(&list);
        let work_id = work_id.clone();
        let title = title.clone();
        wasm_bindgen_futures::spawn_local(async move {
            note.set_text_content(None);
            // A second click while the first is on the way would send two adds
            let disabled = button.unchecked_ref::<HtmlElement>();
            let _ = disabled.set_attribute("disabled", "");

            let kind = if state.get() {
                messages::WATCH_REMOVE
            } else {
                messages::WATCH_ADD
            };
            let reply = send(kind, &work_id, &title, &list).await;
            let _ = disabled.remove_attribute("disabled");

            match reply {
                Ok(reply) if json::get(&reply, "ok").and_then(|v| v.as_bool()) == Some(true) => {
                    let watched = json::get(&reply, "watched")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(!state.get());
                    state.set(watched);
                    apply_state(&button, watched);
                }
                Ok(reply) if json::get(&reply, "full").and_then(|v| v.as_bool()) == Some(true) => {
                    let max = json::get_f64(&reply, "max").unwrap_or(watch::MAX_WORKS as f64);
                    note.set_text_content(Some(&t_fill(
                        "watch.full",
                        &[("max", &max.to_string())],
                    )));
                }
                Ok(reply) => {
                    log(&format!("新着の見張り: 断られました: {reply:?}"));
                    note.set_text_content(Some(t("watch.failed")));
                }
                Err(err) => {
                    log(&format!("新着の見張り: 送れません: {err:?}"));
                    note.set_text_content(Some(t("watch.failed")));
                }
            }
        });
    });
    clicked.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    // Needed as long as the page lives
    on_click.forget();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::work_title;

    #[test]
    fn removes_the_number_of_episodes_from_the_heading() {
        // The heading of the site (measured)
        assert_eq!(
            work_title("ワンピース ワノ国編（全197話）"),
            "ワンピース ワノ国編"
        );
        assert_eq!(
            work_title("機動戦士ガンダム 水星の魔女（全25話）"),
            "機動戦士ガンダム 水星の魔女"
        );
    }

    #[test]
    fn keeps_brackets_that_belong_to_the_name() {
        assert_eq!(work_title("作品名（第2期）"), "作品名（第2期）");
        assert_eq!(
            work_title("舞台『機動戦士ガンダム00』"),
            "舞台『機動戦士ガンダム00』"
        );
        // Nothing to remove
        assert_eq!(work_title("  作品名  "), "作品名");
    }
}
