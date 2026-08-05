//! The settings UI. One WASM draws both the options page and the popup.
//!
//! No UI framework; only `web-sys`. The rows come from the three tables of
//! `d_tweaks_shared::settings` (`FEATURES`, `SWITCHES`, `CHOICES`), so a new setting
//! does not change this file.
//!
//! The popup (`popup.html`) and the options page (`options.html`) show the same rows. Two
//! crates would read the tables twice, so both draw into the same `#settings` and
//! `body.compact` changes only the appearance. With `compact`, the description goes into
//! the `title` attribute: a tall page does not fit in a popup.
//!
//! A write sends `chrome.storage.onChanged`, which starts the service worker, and that
//! replaces the registrations (`crates/background`). A tab that is already open needs a
//! reload, so the popup has a reload button.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, Event, HtmlInputElement, HtmlOptionElement, HtmlSelectElement};

use d_tweaks_shared::settings::{self, CHOICES, FEATURES, SWITCHES};

/// Message after a write. A tab that is already open needs a reload.
const SAVED: &str = "保存しました。開いているタブは再読み込みしてください。";
const FAILED: &str = "保存に失敗しました。";

fn set_status(document: &Document, message: &str) {
    if let Some(status) = document.get_element_by_id("status") {
        status.set_text_content(Some(message));
    }
}

/// The frame of a row (label and description). With `compact`, the description goes into
/// the `title`.
fn row(
    document: &Document,
    label: &str,
    description: &str,
    compact: bool,
) -> Result<(Element, Element), JsValue> {
    let row = document.create_element("label")?;
    row.set_class_name("feature");
    if compact {
        row.set_attribute("title", description)?;
    }

    let text = document.create_element("div")?;
    text.set_class_name("text");

    let label_el = document.create_element("span")?;
    label_el.set_class_name("label");
    label_el.set_text_content(Some(label));
    text.append_child(&label_el)?;

    if !compact {
        let description_el = document.create_element("span")?;
        description_el.set_class_name("description");
        description_el.set_text_content(Some(description));
        text.append_child(&description_el)?;
    }

    Ok((row, text))
}

/// A row with a switch. Used by `FEATURES` and `SWITCHES`.
fn build_toggle(
    document: &Document,
    list: &Element,
    id: &'static str,
    label: &str,
    description: &str,
    checked: bool,
    compact: bool,
) -> Result<(), JsValue> {
    let (row_el, text) = row(document, label, description, compact)?;

    let checkbox: HtmlInputElement = document.create_element("input")?.dyn_into()?;
    checkbox.set_type("checkbox");
    checkbox.set_checked(checked);
    checkbox.set_id(id);

    row_el.append_child(&checkbox)?;
    row_el.append_child(&text)?;
    list.append_child(&row_el)?;

    // A change writes at once; there is no save button
    let status_target = document.clone();
    let on_change = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let Some(target) = event.target() else { return };
        let Ok(input) = target.dyn_into::<HtmlInputElement>() else {
            return;
        };
        let value = input.checked();
        let document = status_target.clone();
        spawn_local(async move {
            let message = match settings::save_one(id, value).await {
                Ok(()) => SAVED,
                Err(_) => FAILED,
            };
            set_status(&document, message);
            // The main switch also changes the appearance of the other rows
            if id == settings::ENABLED {
                mark_disabled(&document, !value);
            }
        });
    });
    checkbox.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
    // Needed as long as the page lives
    on_change.forget();

    Ok(())
}

/// A row with a list.
fn build_choice(
    document: &Document,
    list: &Element,
    choice: &'static settings::ChoiceDef,
    current: &str,
    compact: bool,
) -> Result<(), JsValue> {
    let (row_el, text) = row(document, choice.label, choice.description, compact)?;

    let select: HtmlSelectElement = document.create_element("select")?.dyn_into()?;
    select.set_class_name("choice");
    select.set_id(choice.id);
    for (value, label) in choice.options {
        let option: HtmlOptionElement = document.create_element("option")?.dyn_into()?;
        option.set_value(value);
        option.set_text_content(Some(label));
        if *value == current {
            option.set_selected(true);
        }
        select.append_child(&option)?;
    }

    // A click on the row does not open a list, so the order is text and then the list
    row_el.append_child(&text)?;
    row_el.append_child(&select)?;
    list.append_child(&row_el)?;

    let id = choice.id;
    let status_target = document.clone();
    let on_change = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let Some(target) = event.target() else { return };
        let Ok(select) = target.dyn_into::<HtmlSelectElement>() else {
            return;
        };
        let value = select.value();
        let document = status_target.clone();
        spawn_local(async move {
            let message = match settings::save_choice(id, &value).await {
                Ok(()) => SAVED,
                Err(_) => FAILED,
            };
            set_status(&document, message);
        });
    });
    select.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
    on_change.forget();

    Ok(())
}

/// Put a class on `<body>` that makes the other rows grey while the extension is off.
///
/// Rows that look normal but do nothing are confusing.
fn mark_disabled(document: &Document, disabled: bool) {
    let Some(body) = document.body() else { return };
    let _ = if disabled {
        body.class_list().add_1("dt-disabled")
    } else {
        body.class_list().remove_1("dt-disabled")
    };
}

/// A section heading. Also with `compact`: there are too many rows without them.
fn build_heading(document: &Document, list: &Element, text: &str) -> Result<(), JsValue> {
    let heading = document.create_element("h2")?;
    heading.set_class_name("group");
    heading.set_text_content(Some(text));
    list.append_child(&heading)?;
    Ok(())
}

/// Add the buttons that only the popup has (reload, open the options page).
///
/// On a page without them (the options page), this does nothing.
fn install_actions(document: &Document) -> Result<(), JsValue> {
    if let Some(button) = document.get_element_by_id("reload") {
        let on_click = Closure::<dyn FnMut()>::new(move || {
            reload_active_tab();
            // Close the popup, so the result is visible
            if let Some(window) = web_sys::window() {
                let _ = window.close();
            }
        });
        button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    // Remove the comment cache.
    //
    // A correction of the match logic has no visible effect while a wrong result or a
    // "not found" is still in the cache (30 days for a map, one day for a "not found"). A
    // new key version also removes them, but this button does not wait for that.
    if let Some(button) = document.get_element_by_id("clearCache") {
        let status_target = document.clone();
        let on_click = Closure::<dyn FnMut()>::new(move || {
            let document = status_target.clone();
            spawn_local(async move {
                let message = match clear_comment_cache().await {
                    Ok(0) => "消すものがありませんでした。".to_string(),
                    Ok(count) => {
                        format!("コメントの控えを {count} 件消しました。再生し直すと引き直します。")
                    }
                    Err(_) => "控えを消せませんでした。".to_string(),
                };
                set_status(&document, &message);
            });
        });
        button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    if let Some(button) = document.get_element_by_id("openOptions") {
        let on_click = Closure::<dyn FnMut()>::new(move || {
            open_options_page();
        });
        button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
        on_click.forget();
    }

    // Show the version, so the build is known
    if let Some(el) = document.get_element_by_id("version") {
        el.set_text_content(Some(&format!("v{}", manifest_version())));
    }
    Ok(())
}

/// Remove the comment cache (the map and the comments). Returns the number removed.
///
/// The settings (`storage.sync`) are not touched; only `storage.local`.
async fn clear_comment_cache() -> Result<u32, JsValue> {
    use d_tweaks_shared::cache_keys::{COMMENT_INDEX, COMMENT_PREFIX, VIDEO_ROOT};

    let all = d_tweaks_shared::chrome::local_all().await?;
    let targets: Vec<String> = js_sys::Object::keys(&all)
        .iter()
        .filter_map(|key| key.as_string())
        .filter(|key| {
            key.starts_with(VIDEO_ROOT) || key.starts_with(COMMENT_PREFIX) || key == COMMENT_INDEX
        })
        .collect();
    if targets.is_empty() {
        return Ok(0);
    }
    let keys = js_sys::Array::new();
    for key in &targets {
        keys.push(&JsValue::from_str(key));
    }
    d_tweaks_shared::chrome::local_remove(&keys).await?;
    Ok(targets.len() as u32)
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "runtime"], js_name = "openOptionsPage")]
    fn open_options_page();

    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = "reload")]
    fn tabs_reload();

    #[wasm_bindgen(js_namespace = ["chrome", "runtime"], js_name = "getManifest")]
    fn get_manifest() -> JsValue;
}

/// Reload the tab (`chrome.tabs.reload` without an argument is the current tab).
fn reload_active_tab() {
    tabs_reload();
}

fn manifest_version() -> String {
    d_tweaks_shared::json::get_string(&get_manifest(), "version").unwrap_or_default()
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    if let Err(err) = install_actions(&document) {
        web_sys::console::error_1(&err);
    }

    spawn_local(async move {
        // The popup has no descriptions
        let compact = document
            .body()
            .map(|body| body.class_list().contains("compact"))
            .unwrap_or(false);

        let Some(list) = document.get_element_by_id("settings") else {
            return;
        };

        let snapshot = match settings::snapshot().await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                web_sys::console::error_1(&err);
                set_status(&document, "設定を読み込めませんでした。");
                return;
            }
        };

        list.set_inner_html("");
        mark_disabled(&document, !snapshot.enabled);
        let render = |result: Result<(), JsValue>| {
            if let Err(err) = result {
                web_sys::console::error_1(&err);
            }
        };

        // The main switch is first, and alone
        render(build_heading(&document, &list, "全体"));
        render(build_toggle(
            &document,
            &list,
            settings::ENABLED,
            "この拡張を有効にする",
            "切ると、すべての表示改造を止めてサイト本来の見た目に戻します（Chrome の拡張機能そのものは切りません）。開いているタブはその場で戻り、こちらの UI を出し直すにはページの再読み込みが必要です。",
            snapshot.enabled,
            compact,
        ));

        render(build_heading(&document, &list, "表示を変える"));
        for (feature, on) in FEATURES.iter().zip(&snapshot.features) {
            render(build_toggle(
                &document,
                &list,
                feature.id,
                feature.label,
                feature.description,
                *on,
                compact,
            ));
        }

        render(build_heading(&document, &list, "細かい設定"));
        for (switch, on) in SWITCHES.iter().zip(&snapshot.switches) {
            render(build_toggle(
                &document,
                &list,
                switch.id,
                switch.label,
                switch.description,
                *on,
                compact,
            ));
        }
        for (choice, current) in CHOICES.iter().zip(&snapshot.choices) {
            render(build_choice(&document, &list, choice, current, compact));
        }
    });

    Ok(())
}
