//! Control bar under the float player.
//!
//! The controls of the site are inside the iframe and over the video, so they hide the
//! picture and they appear and disappear with the pointer. This bar is outside of the
//! video and always in the same place.
//!
//! The iframe has the same origin, so the `<video>` is directly available. A new
//! episode replaces the element, so `is_connected()` decides when to take it again.
//!
//! The play position writes the slider, so a write during a drag would move the handle
//! away from the pointer. After a drag, the write stops for `HOLD_MS`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::Date;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Element, HtmlElement, HtmlIFrameElement, HtmlInputElement, HtmlMediaElement,
    KeyboardEvent, KeyboardEventInit,
};

use crate::features::{frame, player_modal};
use crate::{log, timestamp};

/// Interval of the display. The seek bar is smooth enough at this rate.
const POLL_MS: i32 = 200;
/// Step of the short skip, in seconds.
const SKIP_SECONDS: f64 = 10.0;
/// Step of the long skip. The same 30 seconds as the button of the site.
const LONG_SKIP_SECONDS: f64 = 30.0;
/// Steps of the seek bar. More steps than a pointer can use are of no value.
const SEEK_STEPS: f64 = 1000.0;
/// Time without a write to the slider after a drag.
const HOLD_MS: f64 = 500.0;
/// Order of the play rates.
const SPEEDS: &[f64] = &[1.0, 1.25, 1.5, 2.0, 0.75];
/// Marks a control that is off.
const OFF_CLASS: &str = "dt-bar__button--off";
/// Put on the panel to hide the danmaku.
const HIDE_DANMAKU_CLASS: &str = "dt-hide-danmaku";
/// Marks a button that a narrow screen can remove.
///
/// With the 10-second steps and the seek bar, the 30-second steps are not necessary,
/// and the rate and the UI of the site are rarely used. Play, the episode buttons, the
/// volume, the danmaku and the list always stay.
const SECONDARY_CLASS: &str = "dt-bar__button--secondary";

/// The `<video>` that the bar controls. A new episode replaces it.
type Target = Rc<RefCell<Option<HtmlMediaElement>>>;

fn button(document: &Document, label: &str, title: &str) -> Result<Element, JsValue> {
    let el = document.create_element("button")?;
    el.set_class_name("dt-bar__button");
    el.set_attribute("type", "button")?;
    el.set_attribute("title", title)?;
    el.set_text_content(Some(label));
    Ok(el)
}

fn slider(
    document: &Document,
    class: &str,
    max: f64,
    title: &str,
) -> Result<HtmlInputElement, JsValue> {
    let el: HtmlInputElement = document.create_element("input")?.dyn_into()?;
    el.set_class_name(class);
    el.set_type("range");
    el.set_min("0");
    el.set_max(&max.to_string());
    el.set_attribute("title", title)?;
    Ok(el)
}

fn on_click<F: FnMut() + 'static>(target: &Element, handler: F) -> Result<(), JsValue> {
    let closure = Closure::<dyn FnMut()>::new(handler);
    target.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

fn on_input<F: FnMut() + 'static>(target: &Element, handler: F) -> Result<(), JsValue> {
    let closure = Closure::<dyn FnMut()>::new(handler);
    target.add_event_listener_with_callback("input", closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

/// A class shows the on or off state.
fn set_off(element: &Element, off: bool) {
    let list = element.class_list();
    let _ = if off {
        list.add_1(OFF_CLASS)
    } else {
        list.remove_1(OFF_CLASS)
    };
}

/// Id of the `<style>` that this module puts into the iframe.
const HIDE_STYLE_ID: &str = "dt-hide-native-controls";
/// Hide only `.controller`.
///
/// `.skipUi` (the skip of the site) and `.waitArea` (the loading state) are not hover
/// UI: they appear when they are necessary, so they stay.
const HIDE_RULE: &str = ".controller { display: none !important; }";

/// Hide or show the controls of the site.
///
/// The iframe has the same origin, so a `<style>` can go into it. A new episode
/// replaces the document, so this runs again with every new `<video>`.
fn apply_native_hidden(iframe: &HtmlIFrameElement, hidden: bool) {
    let Some(doc) = iframe.content_document() else {
        return;
    };
    let style = match doc.get_element_by_id(HIDE_STYLE_ID) {
        Some(style) => style,
        None => {
            let Ok(style) = doc.create_element("style") else {
                return;
            };
            style.set_id(HIDE_STYLE_ID);
            // The `Document` of web-sys has no `head()`, so use a selector
            let Ok(Some(head)) = doc.query_selector("head") else {
                return;
            };
            if head.append_child(&style).is_err() {
                return;
            }
            style
        }
    };
    // Only the rule changes; the element stays
    style.set_text_content(Some(if hidden { HIDE_RULE } else { "" }));
}

/// Key of "the episode before" in the player of the site.
///
/// The handler of the site reads `keyCode`, and it has `case 80` (P) and `case 33`
/// (PageUp) for this action.
const PREV_KEY_CODE: u32 = 80;

/// Go to the episode before.
///
/// A `click()` on `.prevButton` of the site does nothing. That button is not one action:
/// `prevBtnClickTouchEvent` reads the classes of `#prevPopupIn` and `#prevPopupInReTop`
/// and calls `goPrev()` for the first and `jump(0)` ("back to the start") for the second.
/// The site writes those classes on `mouseenter` (`prevPlay3SecJudge`), and a `click()`
/// sends no `mouseenter`, so neither of the two agrees and nothing happens.
///
/// The same function also means "back to the start" after the first three seconds of the
/// episode, which is not what this button says.
///
/// The player has the same action on a key, and that path calls `goPrev()` without a
/// state of a popup in it. The handler is on the document of the iframe, so the event
/// goes there. `.nextButton` needs none of this: its click handler calls `goNext()`
/// directly, and it also saves the play position first.
fn go_prev(iframe: &HtmlIFrameElement) {
    let Some(doc) = iframe.content_document() else {
        return;
    };
    let init = KeyboardEventInit::new();
    init.set_key("p");
    init.set_key_code(PREV_KEY_CODE);
    init.set_which(PREV_KEY_CODE);
    init.set_bubbles(true);
    init.set_cancelable(true);
    match KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init) {
        Ok(event) => {
            let _ = doc.dispatch_event(&event);
        }
        Err(err) => log(&format!("前話のキー操作を作れませんでした: {err:?}")),
    }
}

/// Click a button of the site.
///
/// A `click()` also works on a hidden button: it calls the handler and not the
/// appearance. The episode buttons and the settings need the logic of the site, so this
/// module does not build its own.
///
/// `dyn_into` fails across the realm (see `features::frame`).
pub(crate) fn click_native(iframe: &HtmlIFrameElement, selector: &str) {
    let Some(doc) = iframe.content_document() else {
        return;
    };
    if let Ok(Some(element)) = doc.query_selector(selector) {
        element.unchecked_into::<HtmlElement>().click();
    } else {
        log(&format!("サイトのボタンが見つかりません: {selector}"));
    }
}

fn seek_by(video: &Target, delta: f64) {
    let borrowed = video.borrow();
    let Some(video) = borrowed.as_ref() else {
        return;
    };
    let mut time = (video.current_time() + delta).max(0.0);
    let duration = video.duration();
    if duration.is_finite() && duration > 0.0 {
        time = time.min(duration);
    }
    video.set_current_time(time);
}

/// Build the control bar and start it. It stops itself when the bar leaves the DOM.
pub fn install(panel: &Element, bar: &Element, iframe: &HtmlIFrameElement) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = bar
        .owner_document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let video: Target = Rc::new(RefCell::new(None));
    // No write to the slider directly after a drag
    let hold = Rc::new(Cell::new(0.0));
    // Are the controls of the site hidden? A new episode needs the same state.
    let native_hidden_flag = Rc::new(Cell::new(true));

    let prev = button(&document, "前話", "前の話へ")?;
    let play = button(&document, "▶", "再生 / 一時停止")?;
    let next = button(&document, "次話", "次の話へ")?;
    let back30 = button(&document, "-30s", "30 秒戻る")?;
    let back = button(&document, "-10s", "10 秒戻る")?;
    let forward = button(&document, "+10s", "10 秒進む")?;
    let forward30 = button(&document, "+30s", "30 秒進む")?;

    let time = document.create_element("span")?;
    time.set_class_name("dt-bar__time");
    time.set_text_content(Some("0:00 / 0:00"));

    let seek = slider(&document, "dt-bar__seek", SEEK_STEPS, "再生位置")?;
    let speed = button(&document, "1.0×", "再生速度を切り替える")?;
    let mute = button(&document, "音", "ミュート")?;
    let volume = slider(&document, "dt-bar__volume", 100.0, "音量")?;
    let danmaku = button(&document, "弾幕", "コメントの表示を切り替える")?;
    let list = button(&document, "一覧", "コメント一覧の開閉")?;
    let native = button(
        &document,
        "サイトUI",
        "サイト本来のコントロールを出す（画質などの設定はこちらから）",
    )?;
    let fullscreen = button(
        &document,
        "最大化",
        "ブラウザいっぱいに広げる（Esc で戻る）",
    )?;
    // Both start in the off state
    set_off(&native, true);
    set_off(&fullscreen, true);

    // These go away on a narrow screen
    for element in [&back30, &forward30, &speed, &native] {
        element.class_list().add_1(SECONDARY_CLASS)?;
    }

    for element in [
        &prev, &play, &next, &back30, &back, &forward, &forward30, &time,
    ] {
        bar.append_child(element)?;
    }
    bar.append_child(&seek)?;
    bar.append_child(&speed)?;
    bar.append_child(&mute)?;
    bar.append_child(&volume)?;
    for element in [&danmaku, &list, &native, &fullscreen] {
        bar.append_child(element)?;
    }

    // --- Controls ---

    {
        let video = Rc::clone(&video);
        on_click(&play, move || {
            let borrowed = video.borrow();
            let Some(video) = borrowed.as_ref() else {
                return;
            };
            if video.paused() {
                // play() returns a promise; on a failure the next update corrects the
                // display
                let _ = video.play();
            } else {
                let _ = video.pause();
            }
        })?;
    }

    for (element, delta) in [
        (&back30, -LONG_SKIP_SECONDS),
        (&back, -SKIP_SECONDS),
        (&forward, SKIP_SECONDS),
        (&forward30, LONG_SKIP_SECONDS),
    ] {
        let video = Rc::clone(&video);
        on_click(element, move || seek_by(&video, delta))?;
    }

    // The episode buttons need the logic of the site (continuous play).
    //
    // The two are not symmetric: the next button of the site is one action, the previous
    // button of the site is two (see `go_prev`).
    {
        let iframe = iframe.clone();
        on_click(&prev, move || go_prev(&iframe))?;
    }
    {
        let iframe = iframe.clone();
        on_click(&next, move || click_native(&iframe, ".nextButton"))?;
    }

    // The quality setting is only in the panel of the site, so it can appear
    {
        let iframe = iframe.clone();
        let button = native.clone();
        let hidden = Rc::new(Cell::new(true));
        let native_hidden = Rc::clone(&native_hidden_flag);
        on_click(&native, move || {
            let next = !hidden.get();
            hidden.set(next);
            native_hidden.set(next);
            apply_native_hidden(&iframe, next);
            set_off(&button, next);
        })?;
    }

    // Large means the area of the browser, not the Fullscreen API: that removes the
    // tabs and the address bar, so the way back to the list is not visible. The area of
    // the browser is large enough, and the panel takes the danmaku and the bar with it.
    {
        let panel = panel.clone();
        let button = fullscreen.clone();
        on_click(&fullscreen, move || {
            let maximized = player_modal::toggle_maximized(&panel);
            set_off(&button, !maximized);
        })?;
    }

    {
        let video = Rc::clone(&video);
        let hold = Rc::clone(&hold);
        let input = seek.clone();
        on_input(seek.as_ref(), move || {
            hold.set(Date::now() + HOLD_MS);
            let Ok(step) = input.value().parse::<f64>() else {
                return;
            };
            let borrowed = video.borrow();
            let Some(video) = borrowed.as_ref() else {
                return;
            };
            let duration = video.duration();
            if duration.is_finite() && duration > 0.0 {
                video.set_current_time(duration * step / SEEK_STEPS);
            }
        })?;
    }

    {
        let video = Rc::clone(&video);
        let index = Rc::new(Cell::new(0usize));
        let label = speed.clone();
        on_click(&speed, move || {
            let next = (index.get() + 1) % SPEEDS.len();
            index.set(next);
            let rate = SPEEDS[next];
            label.set_text_content(Some(&format!("{rate:.2}×")));
            if let Some(video) = video.borrow().as_ref() {
                video.set_playback_rate(rate);
            }
        })?;
    }

    {
        let video = Rc::clone(&video);
        on_click(&mute, move || {
            if let Some(video) = video.borrow().as_ref() {
                video.set_muted(!video.muted());
            }
        })?;
    }

    {
        let video = Rc::clone(&video);
        let hold = Rc::clone(&hold);
        let input = volume.clone();
        on_input(volume.as_ref(), move || {
            hold.set(Date::now() + HOLD_MS);
            let Ok(value) = input.value().parse::<f64>() else {
                return;
            };
            if let Some(video) = video.borrow().as_ref() {
                video.set_volume((value / 100.0).clamp(0.0, 1.0));
                // A change of the volume also ends the mute; else there is no sound
                // and the reason is not visible
                if value > 0.0 && video.muted() {
                    video.set_muted(false);
                }
            }
        })?;
    }

    // CSS hides the danmaku. The drawing continues, so it is correct when it returns.
    {
        let panel = panel.clone();
        let button = danmaku.clone();
        on_click(&danmaku, move || {
            let hidden = panel
                .class_list()
                .toggle(HIDE_DANMAKU_CLASS)
                .unwrap_or(false);
            set_off(&button, hidden);
        })?;
    }

    // Open or close the list. The same as a double click on the handle.
    {
        let panel = panel.clone();
        let button = list.clone();
        on_click(&list, move || {
            let width = player_modal::toggle_side(&panel);
            set_off(&button, width <= 0.0);
        })?;
    }

    // --- Update of the display ---

    let timer = Rc::new(Cell::new(0));
    let tick = {
        let timer = Rc::clone(&timer);
        let video = Rc::clone(&video);
        let hold = Rc::clone(&hold);
        let bar = bar.clone();
        let iframe = iframe.clone();
        let native_hidden = Rc::clone(&native_hidden_flag);
        let seek = seek.clone();
        let volume = volume.clone();
        Closure::<dyn FnMut()>::new(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            // Closing the modal removes the bar. Stop here.
            if !bar.is_connected() {
                window.clear_interval_with_handle(timer.get());
                return;
            }

            // A new episode replaces the element, so take it again
            let stale = video
                .borrow()
                .as_ref()
                .is_none_or(|video| !video.is_connected());
            if stale {
                *video.borrow_mut() = frame::video_in(&iframe);
                // A new episode replaces the document, so add the rule again
                if video.borrow().is_some() {
                    apply_native_hidden(&iframe, native_hidden.get());
                }
            }
            let borrowed = video.borrow();
            let Some(video) = borrowed.as_ref() else {
                return;
            };

            let current = video.current_time();
            let duration = video.duration();
            play.set_text_content(Some(if video.paused() { "▶" } else { "‖" }));
            time.set_text_content(Some(&format!(
                "{} / {}",
                timestamp(current),
                if duration.is_finite() {
                    timestamp(duration)
                } else {
                    "--:--".to_string()
                }
            )));
            set_off(&mute, video.muted());

            // No write during a drag; the handle would move away from the pointer
            if Date::now() >= hold.get() {
                let step = if duration.is_finite() && duration > 0.0 {
                    current / duration * SEEK_STEPS
                } else {
                    0.0
                };
                seek.set_value(&format!("{step:.0}"));
                volume.set_value(&format!("{:.0}", video.volume() * 100.0));
            }
        })
    };

    let id = window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        POLL_MS,
    )?;
    timer.set(id);
    tick.forget();

    log("操作バーを表示しました");
    Ok(())
}
