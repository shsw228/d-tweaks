//! Head bar of the float player, and the skip to the main story.
//!
//! Both come from the same `WS010105` reply, so they are one module and one
//! request.
//!
//! Like the control bar (`controls`), this puts the information outside of the
//! video. The player of the site draws the title over the video, which hides the
//! picture and disappears after a few seconds.
//!
//! ```text
//! work title (to the work page) / 第6話 / episode title
//! 18:44 - main story 1:39-17:13 - continue at 2:57 - latest episode
//! ```
//!
//! # Source: `WS010105`
//!
//! Reads the `data` that `comments::part_data` received (the last reply is kept, so
//! the same episode asks one time). Only the fields for the display are used. The
//! DRM fields (`laUrl`, `contentUrls`, `oneTimeKey`, `viewOneTimeToken`,
//! `castContentUri`) are never read.
//!
//! | Field | Use |
//! |---|---|
//! | `workTitle` | Work title |
//! | `partDispNumber` | Episode number (only the number, such as `6`) |
//! | `partTitle` | Episode title |
//! | `partMeasureSecond` | Length |
//! | `resumePoint` | Last position, in milliseconds |
//! | `chapters[].type == "mainStory"` | The main story; before and after it are the opening and the ending |
//! | `nextTitle` | `null` means the latest episode |
//!
//! # Skip to the main story
//!
//! Measured shapes of `chapters`:
//!
//! ```text
//! (18:44)  none 0:00-1:39 / mainStory 1:39-17:13 / none 17:13-18:44
//! (24:07)  avant 0:00-3:16 / none 3:16-4:45 / mainStory 4:45-22:38 / none 22:38-24:07
//! (2:00)   none 0:00-0:03 / mainStory 0:03-1:34 / none 1:34-2:00
//! ```
//!
//! There is no `op` and no `ed` in `type`. A chapter is `none`, `avant` or
//! `mainStory`, and the opening, the ending, the sponsor card and the preview are
//! all `none`. On all 15 measured works there is one `mainStory`, the last chapter
//! is always `none` (ending and preview), and nothing follows it.
//!
//! The test follows the implementation of the site (`chapterCheck` in
//! `player.min.js`); `skip_at` has the details.
//!
//! `opSkipAvailable` was `"0"` on all seven measured works. The skip UI of the site
//! is controlled by a cookie (`op_skip`) and does not read that field, so this
//! module also ignores it.
//!
//! The episode can change, so this watches the URL of the iframe (there is no
//! event). The watch stops when the bar leaves the DOM. The same watch reads the
//! play position for the skip button, so there is only one timer.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, HtmlIFrameElement, MouseEvent};

use d_tweaks_shared::{json, settings};

use crate::features::comments::{self, Target};
use crate::features::{controls, frame};
use crate::{log, sleep, timestamp};

/// Interval for the URL of the iframe and the play position. Same as `comments`.
const WATCH_INTERVAL_MS: i32 = 500;
/// Default for `minTimeToSkip` when the reply has no value, in seconds.
const MIN_TIME_TO_SKIP_DEFAULT: f64 = 15.0;

/// One chapter of `chapters`.
struct Chapter {
    kind: String,
    start: f64,
    end: f64,
    /// Does the site show its skip UI in this chapter?
    show_interface: bool,
}

/// What the skip needs. Built again for each episode.
#[derive(Default)]
struct Plan {
    chapters: Vec<Chapter>,
    /// No button when the chapter has fewer seconds left than this.
    min_time_to_skip: f64,
    /// Is there a next episode (`nextTitle`)?
    has_next: bool,
}

/// What a click does.
#[derive(PartialEq, Debug)]
enum Action {
    /// Go to this second.
    Seek(f64),
    /// Go to the next episode (with the button of the site).
    Next,
}

/// The button for the current position, or `None` for no button.
///
/// This follows `chapterCheck` of `player.min.js`, which decides:
///
/// 1. A skip is possible only inside a `none` chapter (not in `avant`).
/// 2. The target is the start of the next chapter that is not `none`. In the last
///    chapter it is the end of the chapter, which is the end of the video.
/// 3. No button when the chapter has fewer seconds left than `minTimeToSkip`
///    (measured: 15).
/// 4. No large UI in a chapter with `showInterface: false`.
/// 5. The label is "skip to the main story". In the last chapter it becomes "next
///    episode", "play again" or "end", from the continuous-play setting and the
///    next episode.
///
/// Rules 1 to 4 are the same here. Of rule 5, only "next episode" is used, and only
/// when a next episode exists. This module never skips without a click.
fn skip_at(plan: &Plan, current: f64) -> Option<(&'static str, Action)> {
    // The current chapter. Two chapters touch at the same second; take the later.
    let (index, chapter) = plan
        .chapters
        .iter()
        .enumerate()
        .rfind(|(_, c)| c.start <= current && current <= c.end)?;

    // No button inside the main story or the avant
    if chapter.kind != "none" || !chapter.show_interface {
        return None;
    }
    // No time to click. This also removes a `none` chapter of three seconds.
    if chapter.end - current < plan.min_time_to_skip {
        return None;
    }

    // The target is the start of the next chapter that is not `none`
    let target = plan
        .chapters
        .iter()
        .skip(index + 1)
        .find(|c| c.kind != "none")
        .map(|c| c.start);
    match target {
        Some(to) => Some(("本編へスキップ ▶︎", Action::Seek(to))),
        // Nothing after this chapter means the ending
        None if plan.has_next => Some(("次の話へ ▶︎", Action::Next)),
        None => None,
    }
}

/// Build the skip data from the `WS010105` reply.
fn plan_of(data: &JsValue) -> Plan {
    let has_next = json::get(data, "nextTitle").is_some();
    let min_time_to_skip = json::get_f64(data, "minTimeToSkip")
        .map(|ms| ms / 1000.0)
        .filter(|s| *s >= 0.0)
        .unwrap_or(MIN_TIME_TO_SKIP_DEFAULT);
    let chapters = json::get_array(data, "chapters")
        .map(|array| {
            array
                .iter()
                .filter_map(|chapter| {
                    Some(Chapter {
                        kind: json::get_string(&chapter, "type")?,
                        start: json::get_f64(&chapter, "start")? / 1000.0,
                        end: json::get_f64(&chapter, "end")? / 1000.0,
                        // Default is true; some chapters have no field
                        show_interface: json::get(&chapter, "showInterface")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Plan {
        chapters,
        min_time_to_skip,
        has_next,
    }
}

fn element(document: &Document, tag: &str, class: &str) -> Result<Element, JsValue> {
    let el = document.create_element(tag)?;
    el.set_class_name(class);
    Ok(el)
}

/// The URL of the work page, from a `partId`.
///
/// A `partId` is a `workId` and three digits for the episode (measured on the top
/// page). Another form gives `None`, and then there is no link.
fn work_url(part_id: &str) -> Option<String> {
    if part_id.len() <= 3 || !part_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let work_id = &part_id[..part_id.len() - 3];
    Some(format!("/animestore/ci_pc?workId={work_id}"))
}

/// The episode number. `WS010105` returns only the number, such as `6`.
fn episode_label(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        Some(format!("第{text}話"))
    } else {
        Some(text.to_string())
    }
}

/// The range of the main story (`mainStory` of `chapters`).
///
/// The `none` chapters around it are the opening and the ending. A main story that
/// starts at the beginning and ends at the end gives `None`: there is no opening and
/// no ending, so the range is no information.
fn main_story_range(data: &JsValue, duration: Option<f64>) -> Option<(f64, f64)> {
    let chapters = json::get_array(data, "chapters")?;
    for chapter in chapters.iter() {
        if json::get_string(&chapter, "type").as_deref() != Some("mainStory") {
            continue;
        }
        let start = json::get_f64(&chapter, "start")? / 1000.0;
        let end = json::get_f64(&chapter, "end")? / 1000.0;
        if end <= start {
            return None;
        }
        let covers_all = start < 1.0 && duration.is_some_and(|total| end >= total - 1.0);
        if covers_all {
            return None;
        }
        return Some((start, end));
    }
    None
}

/// One item of the meta line. `label` is the small text on the left.
struct MetaItem {
    label: Option<&'static str>,
    value: String,
    /// A class such as `is-latest`.
    modifier: Option<&'static str>,
}

/// Build the meta chips. An absent field gives no chip.
///
/// One long line of grey text is not readable, so each item is a chip with a label
/// (see `player-modal.css`).
fn meta_items(data: &JsValue) -> Vec<MetaItem> {
    let mut items = Vec::new();

    let duration = json::get_f64(data, "partMeasureSecond").filter(|s| *s > 0.0);
    if let Some(seconds) = duration {
        items.push(MetaItem {
            label: None,
            value: timestamp(seconds),
            modifier: None,
        });
    }
    if let Some((start, end)) = main_story_range(data, duration) {
        items.push(MetaItem {
            label: Some("本編"),
            value: format!("{}〜{}", timestamp(start), timestamp(end)),
            modifier: None,
        });
    }
    // resumePoint is in milliseconds. 0 and "almost at the end" are no information.
    if let Some(resume) = json::get_f64(data, "resumePoint").map(|ms| ms / 1000.0) {
        let near_end = duration.is_some_and(|total| resume >= total - 5.0);
        if resume >= 5.0 && !near_end {
            items.push(MetaItem {
                label: Some("前回"),
                value: timestamp(resume),
                modifier: Some("is-resume"),
            });
        }
    }
    if json::get(data, "nextTitle").is_none() {
        items.push(MetaItem {
            label: None,
            value: "最新話".to_string(),
            modifier: Some("is-latest"),
        });
    }
    items
}

/// Draw the meta chips again.
fn fill_meta(document: &Document, meta: &Element, items: &[MetaItem]) -> Result<(), JsValue> {
    meta.set_inner_html("");
    for item in items {
        let class = match item.modifier {
            Some(modifier) => format!("dt-head__chip {modifier}"),
            None => "dt-head__chip".to_string(),
        };
        let chip = element(document, "span", &class)?;
        if let Some(label) = item.label {
            let el = element(document, "i", "dt-head__chipLabel")?;
            el.set_text_content(Some(label));
            chip.append_child(&el)?;
        }
        let value = element(document, "span", "dt-head__chipValue")?;
        value.set_text_content(Some(&item.value));
        chip.append_child(&value)?;
        meta.append_child(&chip)?;
    }
    Ok(())
}

/// Build the head bar and the skip button, and follow the episode.
pub fn install(
    head: &Element,
    stage: &Element,
    frame: &HtmlIFrameElement,
    target: &Target,
) -> Result<(), JsValue> {
    let document = head
        .owner_document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let main = element(&document, "div", "dt-head__main")?;
    // The work title is a link back to the work page
    let work = element(&document, "a", "dt-head__work")?;
    if let Some(url) = work_url(&target.part_id) {
        work.set_attribute("href", &url)?;
    }
    // Until the reply arrives, show what the card gave
    work.set_text_content(target.work_title.as_deref());
    let number = element(&document, "span", "dt-head__number")?;
    number.set_text_content(target.episode_label.as_deref());
    let title = element(&document, "span", "dt-head__title")?;
    main.append_child(&work)?;
    main.append_child(&number)?;
    main.append_child(&title)?;

    let meta = element(&document, "div", "dt-head__meta")?;
    head.append_child(&main)?;
    head.append_child(&meta)?;

    // The skip button is over the video and visible only when it applies
    let skip = element(&document, "button", "dt-skip")?;
    skip.set_attribute("type", "button")?;
    skip.set_attribute("hidden", "")?;
    install_skip_click(&skip, frame)?;
    stage.append_child(&skip)?;

    let frame = frame.clone();
    let head_for_watch = head.clone();
    let fallback_part = target.part_id.clone();
    let fallback_work = target.work_title.clone();
    let fallback_number = target.episode_label.clone();
    spawn_local(async move {
        // The skip button can be off in the settings
        let skip_enabled = settings::switch_enabled(settings::PLAYER_SKIP).await;
        let mut current = String::new();
        let mut plan = Plan::default();
        loop {
            // The bar is gone, so the player closed
            if !head_for_watch.is_connected() {
                return;
            }

            let part_id = frame::part_id(&frame).unwrap_or_else(|| fallback_part.clone());
            if !part_id.is_empty() && part_id != current {
                current = part_id.clone();
                // A new episode has other chapters
                plan = Plan::default();
                let _ = skip.set_attribute("hidden", "");

                // Remove the text of the episode before, so nothing stays
                title.set_text_content(None);
                meta.set_inner_html("");
                if let Some(url) = work_url(&part_id) {
                    let _ = work.set_attribute("href", &url);
                }

                match comments::part_data(&part_id).await {
                    Some(data) => {
                        work.set_text_content(
                            json::get_string(&data, "workTitle")
                                .or_else(|| fallback_work.clone())
                                .as_deref(),
                        );
                        number.set_text_content(
                            episode_label(json::get_string(&data, "partDispNumber"))
                                .or_else(|| fallback_number.clone())
                                .as_deref(),
                        );
                        title.set_text_content(json::get_string(&data, "partTitle").as_deref());
                        if let Err(err) = fill_meta(&document, &meta, &meta_items(&data)) {
                            log(&format!("メタの描画に失敗: {err:?}"));
                        }
                        plan = plan_of(&data);
                    }
                    None => {
                        // No reply (the account is not signed in). Show what is known.
                        work.set_text_content(fallback_work.as_deref());
                        number.set_text_content(fallback_number.as_deref());
                    }
                }
            }

            if skip_enabled {
                update_skip(&skip, &frame, &plan);
            }
            sleep(WATCH_INTERVAL_MS).await;
        }
    });

    Ok(())
}

/// Show or hide the skip button, from the play position.
fn update_skip(skip: &Element, frame: &HtmlIFrameElement, plan: &Plan) {
    let Some(video) = frame::video_in(frame) else {
        let _ = skip.set_attribute("hidden", "");
        return;
    };
    match skip_at(plan, video.current_time()) {
        Some((label, action)) => show_skip(skip, label, action),
        None => {
            let _ = skip.set_attribute("hidden", "");
        }
    }
}

/// Show the button. Writes nothing when the look is the same: a write on every tick
/// makes the button hard to click.
fn show_skip(skip: &Element, label: &str, action: Action) {
    // The action is in an attribute, so the listener is added one time
    match action {
        Action::Seek(to) => {
            let _ = skip.set_attribute(SKIP_TO_ATTR, &to.to_string());
            let _ = skip.remove_attribute(SKIP_NEXT_ATTR);
        }
        Action::Next => {
            let _ = skip.set_attribute(SKIP_NEXT_ATTR, "1");
            let _ = skip.remove_attribute(SKIP_TO_ATTR);
        }
    }
    if skip.text_content().as_deref() != Some(label) {
        skip.set_text_content(Some(label));
    }
    let _ = skip.remove_attribute("hidden");
}

/// Holds the target of the skip, in seconds.
const SKIP_TO_ATTR: &str = "data-dt-skip-to";
/// Marks the button as "next episode".
const SKIP_NEXT_ATTR: &str = "data-dt-skip-next";

/// Add the click listener of the skip button.
fn install_skip_click(skip: &Element, frame: &HtmlIFrameElement) -> Result<(), JsValue> {
    let frame = frame.clone();
    let button = skip.clone();
    let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
        if button.has_attribute(SKIP_NEXT_ATTR) {
            // The site handles the continuous-play setting, so use its button
            controls::click_native(&frame, ".nextButton");
            let _ = button.set_attribute("hidden", "");
            return;
        }
        let Some(to) = button
            .get_attribute(SKIP_TO_ATTR)
            .and_then(|value| value.parse::<f64>().ok())
        else {
            return;
        };
        let Some(video) = frame::video_in(&frame) else {
            return;
        };
        video.set_current_time(to);
        let _ = button.set_attribute("hidden", "");
        log(&format!("本編へスキップ: {}", crate::timestamp(to)));
    });
    skip.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    on_click.forget();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Action, Chapter, MIN_TIME_TO_SKIP_DEFAULT, Plan, episode_label, skip_at, work_url,
    };

    /// Chapters for a test. `showInterface` is true and the limit is 15 seconds.
    fn planned(chapters: &[(&str, f64, f64)], has_next: bool) -> Plan {
        Plan {
            chapters: chapters
                .iter()
                .map(|(kind, start, end)| Chapter {
                    kind: (*kind).to_string(),
                    start: *start,
                    end: *end,
                    show_interface: true,
                })
                .collect(),
            min_time_to_skip: MIN_TIME_TO_SKIP_DEFAULT,
            has_next,
        }
    }

    #[test]
    fn offers_the_main_story_inside_the_opening() {
        // Measured on a real work
        let plan = planned(
            &[
                ("none", 0.0, 99.0),
                ("mainStory", 99.0, 1033.0),
                ("none", 1033.0, 1124.0),
            ],
            true,
        );
        assert_eq!(
            skip_at(&plan, 10.0),
            Some(("本編へスキップ ▶︎", Action::Seek(99.0)))
        );
        // No button inside the main story
        assert_eq!(skip_at(&plan, 200.0), None);
        // The ending goes to the next episode
        assert_eq!(skip_at(&plan, 1040.0), Some(("次の話へ ▶︎", Action::Next)));
        // No next episode, so no button
        let last = planned(
            &[
                ("none", 0.0, 99.0),
                ("mainStory", 99.0, 1033.0),
                ("none", 1033.0, 1124.0),
            ],
            false,
        );
        assert_eq!(skip_at(&last, 1040.0), None);
    }

    #[test]
    fn never_skips_the_avant() {
        // Measured. An avant is part of the story, so it is never skipped.
        let plan = planned(
            &[
                ("avant", 0.0, 196.0),
                ("none", 196.0, 285.0),
                ("mainStory", 285.0, 1358.0),
                ("none", 1358.0, 1447.0),
            ],
            true,
        );
        // No button inside the avant
        assert_eq!(skip_at(&plan, 100.0), None);
        // Only the `none` after it (the opening) gives a button
        assert_eq!(
            skip_at(&plan, 200.0),
            Some(("本編へスキップ ▶︎", Action::Seek(285.0)))
        );
    }

    #[test]
    fn hides_when_there_is_no_time_left_to_press() {
        // Measured. The first `none` is only three seconds.
        let plan = planned(
            &[
                ("none", 0.0, 3.0),
                ("mainStory", 3.0, 94.0),
                ("none", 94.0, 120.0),
            ],
            true,
        );
        assert_eq!(skip_at(&plan, 1.0), None);
        // Also in a long opening, the button goes at 15 seconds left
        let long = planned(
            &[
                ("none", 0.0, 99.0),
                ("mainStory", 99.0, 1033.0),
                ("none", 1033.0, 1124.0),
            ],
            true,
        );
        assert!(skip_at(&long, 83.0).is_some());
        assert_eq!(skip_at(&long, 85.0), None);
    }

    #[test]
    fn gives_up_without_chapters() {
        let empty = planned(&[], true);
        assert_eq!(skip_at(&empty, 10.0), None);
        // Only a main story (a work without chapters)
        let single = planned(&[("mainStory", 0.0, 1400.0)], true);
        assert_eq!(skip_at(&single, 10.0), None);
    }

    #[test]
    fn builds_work_url_by_dropping_the_part_number() {
        // partId = workId and three digits for the episode
        assert_eq!(
            work_url("28641006").as_deref(),
            Some("/animestore/ci_pc?workId=28641")
        );
        // The same rule works for a four-digit workId
        assert_eq!(
            work_url("9999012").as_deref(),
            Some("/animestore/ci_pc?workId=9999")
        );
        // Another form gives no link
        assert_eq!(work_url("006"), None);
        assert_eq!(work_url("28641abc"), None);
        assert_eq!(work_url(""), None);
    }

    #[test]
    fn formats_episode_numbers() {
        // WS010105 returns only the number
        assert_eq!(episode_label(Some("6".into())).as_deref(), Some("第6話"));
        // A complete form stays
        assert_eq!(
            episode_label(Some("第14話".into())).as_deref(),
            Some("第14話")
        );
        // Text such as "特別編" is not removed
        assert_eq!(
            episode_label(Some("特別編".into())).as_deref(),
            Some("特別編")
        );
        assert_eq!(episode_label(Some("  ".into())), None);
        assert_eq!(episode_label(None), None);
    }
}
