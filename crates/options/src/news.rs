//! The list of new episodes, in the popup.
//!
//! The toolbar icon opens this and not the settings: a setting is read one time, and a new
//! episode is the reason to look here again. `popup.html` therefore has `#news`, and that
//! element is also the test that tells `start` which of the two UIs to draw.
//!
//! The service worker owns the list (it is the only writer, see `shared::watch`), so this
//! module only sends messages and draws the reply.
//!
//! Opening the popup marks the entries as read and removes the badge. The entries stay in
//! the list, so this is a history and not an inbox that empties itself.

use js_sys::Date;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Document, Element};

use d_tweaks_shared::text::{t, t_fill};
use d_tweaks_shared::{chrome, json, messages, watch};

use crate::set_status;

/// The work page of the episode. The card of a list uses the same address.
const WORK_URL: &str = "https://animestore.docomo.ne.jp/animestore/ci_pc";
/// Size of the still in a row. `2` is 288x162, the smallest that the site has.
///
/// The address is stored as the interface gave it, so an entry of an older version can hold
/// another size. This makes every row ask for the same one.
const THUMB_SIZE: &str = "2";

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = "create")]
    fn tabs_create(props: &JsValue);
}

/// Send one of the watch messages that need no argument.
async fn send(kind: &str) -> Result<JsValue, JsValue> {
    let message = json::object(&[("type", JsValue::from_str(kind))])?;
    chrome::send_message(&message.into()).await
}

/// How long ago, in words.
///
/// The exact time of a find is not useful ("the check ran at 14:03"), and the popup is
/// narrow, so this is one short word. A time in the future (the clock moved) reads as
/// "just now" and not as a negative number.
fn ago(now: f64, at: f64) -> String {
    let minutes = ((now - at) / 60_000.0).floor();
    if minutes < 1.0 {
        return t("news.now").to_string();
    }
    if minutes < 60.0 {
        return t_fill("news.minutes", &[("n", &format!("{minutes}"))]);
    }
    let hours = (minutes / 60.0).floor();
    if hours < 24.0 {
        return t_fill("news.hours", &[("n", &format!("{hours}"))]);
    }
    let days = (hours / 24.0).floor();
    t_fill("news.days", &[("n", &format!("{days}"))])
}

/// When the next check of a work comes, in words.
///
/// The wait doubles every time a work gives nothing, so this line is the answer to "why
/// have I heard nothing about this work" (see `next_wait_minutes` in the service worker).
fn until(now: f64, at: f64) -> String {
    let minutes = ((at - now) / 60_000.0).ceil();
    if minutes < 1.0 {
        return t("watching.soon").to_string();
    }
    if minutes < 60.0 {
        return t_fill("watching.minutes", &[("n", &format!("{minutes}"))]);
    }
    let hours = (minutes / 60.0).ceil();
    if hours < 24.0 {
        return t_fill("watching.hours", &[("n", &format!("{hours}"))]);
    }
    let days = (hours / 24.0).ceil();
    t_fill("watching.days", &[("n", &format!("{days}"))])
}

fn element(document: &Document, tag: &str, class: &str) -> Result<Element, JsValue> {
    let el = document.create_element(tag)?;
    el.set_class_name(class);
    Ok(el)
}

/// One row of the list.
///
/// A click opens the episode on the work page. An `ended` row has no episode, so it opens
/// the work page itself, which is where the button to register it again is.
fn row(document: &Document, entry: &watch::Found, now: f64) -> Result<Element, JsValue> {
    let row = element(document, "button", "newsItem")?;
    row.set_attribute("type", "button")?;
    if !entry.seen {
        row.class_list().add_1("newsItem--unread")?;
    }
    if entry.ended {
        row.class_list().add_1("newsItem--ended")?;
    }

    // The still of the episode. It came with the answer that found the episode, so the row
    // costs no request to the interface; only the image itself is loaded, and only while the
    // popup is open.
    if let Some(thumb) = &entry.thumb {
        let image = element(document, "img", "newsItem__thumb")?;
        image.set_attribute("src", &watch::thumb_at_size(thumb, THUMB_SIZE))?;
        image.set_attribute("alt", "")?;
        // The list keeps up to 200 rows, so only what is on the screen is loaded
        image.set_attribute("loading", "lazy")?;
        image.set_attribute("decoding", "async")?;
        row.append_child(&image)?;
        row.class_list().add_1("newsItem--withThumb")?;
    }

    let work = element(document, "span", "newsItem__work")?;
    work.set_text_content(Some(&entry.work_title));
    row.append_child(&work)?;

    // The number and the subtitle come from the interface of the site, so they are the
    // words of the service and are not translated. The end of a watch is a word of this
    // extension, so that one is.
    let label = element(document, "span", "newsItem__label")?;
    label.set_text_content(Some(&if entry.ended {
        t("news.ended").to_string()
    } else {
        watch::label_of(entry)
    }));
    row.append_child(&label)?;

    let time = element(document, "span", "newsItem__time")?;
    time.set_text_content(Some(&ago(now, entry.at)));
    row.append_child(&time)?;

    let url = if entry.ended {
        format!("{WORK_URL}?workId={}", entry.work_id)
    } else {
        format!(
            "{WORK_URL}?workId={}&partId={}",
            entry.work_id, entry.part_id
        )
    };
    let on_click = Closure::<dyn FnMut()>::new(move || {
        // A new tab: the popup has no place to show a page, and the user did not ask to
        // leave whatever is in the current tab
        if let Ok(props) = json::object(&[("url", JsValue::from_str(&url))]) {
            tabs_create(&props.into());
        }
    });
    row.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    // Needed as long as the popup lives
    on_click.forget();
    Ok(row)
}

/// One watched work: its name, when it is asked again, and a button that drops it.
fn watched_row(document: &Document, work: &watch::Watched, now: f64) -> Result<Element, JsValue> {
    let row = element(document, "div", "watchItem")?;

    let name = element(document, "span", "watchItem__name")?;
    name.set_text_content(Some(&work.title));
    row.append_child(&name)?;

    let next = element(document, "span", "watchItem__next")?;
    next.set_text_content(Some(&until(now, work.next_at)));
    // The 30 days that end a watch count from here, so it belongs to this row
    let _ = next.set_attribute(
        "title",
        &if work.last_new_at > 0.0 {
            t_fill("watching.last", &[("when", &ago(now, work.last_new_at))])
        } else {
            t("watching.never").to_string()
        },
    );
    row.append_child(&next)?;

    let stop = element(document, "button", "watchItem__stop")?;
    stop.set_attribute("type", "button")?;
    stop.set_text_content(Some(t("watching.stop")));
    row.append_child(&stop)?;

    let work_id = work.work_id.clone();
    let page = document.clone();
    let on_click = Closure::<dyn FnMut()>::new(move || {
        let work_id = work_id.clone();
        let document = page.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let message = match json::object(&[
                ("type", JsValue::from_str(messages::WATCH_REMOVE)),
                ("workId", JsValue::from_str(&work_id)),
            ]) {
                Ok(message) => message,
                Err(err) => {
                    web_sys::console::error_1(&err);
                    return;
                }
            };
            match chrome::send_message(&message.into()).await {
                // The count in the summary changes too, so the whole list is drawn again
                Ok(_) => {
                    if let Err(err) = render(&document).await {
                        web_sys::console::error_1(&err);
                    }
                }
                Err(err) => {
                    web_sys::console::error_1(&err);
                    set_status(&document, t("watch.failed"));
                }
            }
        });
    });
    stop.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    on_click.forget();
    Ok(row)
}

/// Read the list and draw it.
pub async fn render(document: &Document) -> Result<(), JsValue> {
    let Some(list) = document.get_element_by_id("news") else {
        return Ok(());
    };
    let reply = send(messages::WATCH_LIST).await?;
    if json::get(&reply, "ok").and_then(|v| v.as_bool()) != Some(true) {
        set_status(document, t("watch.failed"));
        return Ok(());
    }

    let enabled = json::get(&reply, "enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let works = json::get(&reply, "workList")
        .map(|value| watch::works_from_js(&value))
        .unwrap_or_default();
    let entries = json::get(&reply, "news")
        .map(|value| watch::news_from_js(&value))
        .unwrap_or_default();
    let now = Date::now();

    if let Some(lead) = document.get_element_by_id("newsLead") {
        lead.set_text_content(Some(&if enabled {
            t_fill("news.watching", &[("count", &works.len().to_string())])
        } else {
            t("news.off").to_string()
        }));
    }
    if let Some(list) = document.get_element_by_id("watchingList") {
        list.set_inner_html("");
        if works.is_empty() {
            let empty = element(document, "p", "newsEmpty")?;
            empty.set_text_content(Some(t("watching.none")));
            list.append_child(&empty)?;
        }
        // The one that is asked next is first: that is the row a user looks for
        let mut order: Vec<&watch::Watched> = works.iter().collect();
        order.sort_by(|a, b| {
            a.next_at
                .partial_cmp(&b.next_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for work in order {
            list.append_child(watched_row(document, work, now)?.as_ref())?;
        }
    }

    list.set_inner_html("");
    if entries.is_empty() {
        let empty = element(document, "p", "newsEmpty")?;
        empty.set_text_content(Some(t("news.empty")));
        list.append_child(&empty)?;
        return Ok(());
    }

    let unread = watch::unread(&entries);
    for entry in &entries {
        list.append_child(row(document, entry, now)?.as_ref())?;
    }

    // The list was on the screen, so the badge has done its work
    if unread > 0 {
        let _ = send(messages::WATCH_SEEN).await;
    }
    Ok(())
}

/// The buttons of the popup that act on the list.
pub fn install_actions(document: &Document) -> Result<(), JsValue> {
    if let Some(button) = document.get_element_by_id("checkNow") {
        let page = document.clone();
        let pressed = button.clone();
        let on_click = Closure::<dyn FnMut()>::new(move || {
            let document = page.clone();
            let pressed = pressed.clone();
            // A second press while the first is running would start a second round of
            // requests to the site
            let _ = pressed.set_attribute("disabled", "");
            set_status(&document, t("news.checking"));
            wasm_bindgen_futures::spawn_local(async move {
                let result = send(messages::WATCH_CHECK).await;
                let _ = pressed.remove_attribute("disabled");
                match result {
                    Ok(_) => {
                        set_status(&document, t("news.checked"));
                        if let Err(err) = render(&document).await {
                            web_sys::console::error_1(&err);
                        }
                    }
                    Err(err) => {
                        web_sys::console::error_1(&err);
                        set_status(&document, t("watch.failed"));
                    }
                }
            });
        });
        button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    if let Some(button) = document.get_element_by_id("clearNews") {
        let page = document.clone();
        let on_click = Closure::<dyn FnMut()>::new(move || {
            let document = page.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match send(messages::WATCH_CLEAR).await {
                    Ok(_) => {
                        set_status(&document, "");
                        if let Err(err) = render(&document).await {
                            web_sys::console::error_1(&err);
                        }
                    }
                    Err(err) => {
                        web_sys::console::error_1(&err);
                        set_status(&document, t("watch.failed"));
                    }
                }
            });
        });
        button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }
    Ok(())
}

/// Put the words of the popup in the language of the UI.
pub fn apply_words(document: &Document) {
    let set = |id: &str, key: &str| {
        if let Some(el) = document.get_element_by_id(id) {
            el.set_text_content(Some(t(key)));
        }
    };
    set("checkNow", "news.check");
    set("openOptions", "news.settings");
    set("reload", "popup.reload");
    set("clearNews", "news.clear");
}

#[cfg(test)]
mod tests {
    use super::ago;

    // `t` gives the Japanese word without an init, which is what these cases read
    #[test]
    fn says_how_long_ago_in_one_word() {
        let now = 1_000_000_000.0;
        assert_eq!(ago(now, now - 10_000.0), "たった今");
        assert_eq!(ago(now, now - 5.0 * 60_000.0), "5 分前");
        assert_eq!(ago(now, now - 90.0 * 60_000.0), "1 時間前");
        assert_eq!(ago(now, now - 50.0 * 60.0 * 60_000.0), "2 日前");
    }

    #[test]
    fn a_time_in_the_future_is_not_a_negative_number() {
        let now = 1_000_000_000.0;
        assert_eq!(ago(now, now + 60_000.0), "たった今");
    }
}
