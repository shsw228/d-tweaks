//! Bindings for the parts of the `chrome.*` API that this extension uses.
//!
//! The crates that exist (`chrome-sys` and others) have an unclear maintenance state, so
//! these declarations are written by hand. A `wasm_bindgen` import is resolved when it is
//! called, so a namespace that the context does not have is not a problem while nothing
//! calls it (a content script has no `chrome.scripting`).

use js_sys::{Object, Promise, Reflect};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "storage", "sync"], js_name = "get")]
    fn storage_sync_get(keys: &JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "storage", "sync"], js_name = "set")]
    fn storage_sync_set(items: &JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "scripting"], js_name = "registerContentScripts")]
    fn scripting_register(scripts: &JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "scripting"], js_name = "unregisterContentScripts")]
    fn scripting_unregister(filter: &JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "scripting"], js_name = "getRegisteredContentScripts")]
    fn scripting_get_registered() -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "scripting"], js_name = "insertCSS")]
    fn scripting_insert_css(injection: &JsValue) -> Promise;

    // --- declarativeNetRequest. Only the enabled state of the static rulesets. ---
    #[wasm_bindgen(js_namespace = ["chrome", "declarativeNetRequest"], js_name = "updateEnabledRulesets")]
    fn dnr_update_enabled_rulesets(options: &JsValue) -> Promise;

    // --- storage.local, for the comment cache. storage.sync is too small. ---
    #[wasm_bindgen(js_namespace = ["chrome", "storage", "local"], js_name = "get")]
    fn storage_local_get(keys: &JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "storage", "local"], js_name = "set")]
    fn storage_local_set(items: &JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "storage", "local"], js_name = "remove")]
    fn storage_local_remove(keys: &JsValue) -> Promise;

    // --- Messages between the content script and the service worker ---
    #[wasm_bindgen(js_namespace = ["chrome", "runtime"], js_name = "sendMessage")]
    fn runtime_send_message(message: &JsValue) -> Promise;

    // --- Changes of the settings. A content script also receives them. ---
    #[wasm_bindgen(js_namespace = ["chrome", "storage", "onChanged"], js_name = "addListener")]
    fn storage_on_changed_add(callback: &JsValue);

    // --- alarms, for the periodic check for a new episode ---
    #[wasm_bindgen(js_namespace = ["chrome", "alarms"], js_name = "create")]
    fn alarms_create(name: &str, info: &JsValue);

    #[wasm_bindgen(js_namespace = ["chrome", "alarms"], js_name = "get")]
    fn alarms_get(name: &str) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "alarms"], js_name = "clear")]
    fn alarms_clear(name: &str) -> Promise;

    // --- The badge on the toolbar icon: the number of unread new episodes ---
    #[wasm_bindgen(js_namespace = ["chrome", "action"], js_name = "setBadgeText")]
    fn action_set_badge_text(details: &JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "action"], js_name = "setBadgeBackgroundColor")]
    fn action_set_badge_background_color(details: &JsValue) -> Promise;
}

/// The period of the alarm with this name, in minutes. `None` if it does not exist.
pub async fn alarm_period(name: &str) -> Option<f64> {
    let alarm = JsFuture::from(alarms_get(name)).await.ok()?;
    if alarm.is_undefined() || alarm.is_null() {
        return None;
    }
    Reflect::get(&alarm, &JsValue::from_str("periodInMinutes"))
        .ok()
        .and_then(|value| value.as_f64())
}

/// Make (or replace) a periodic alarm.
///
/// `create` on a name that exists replaces the alarm and starts the period again, so the
/// caller reads `alarm_period` first and only calls this when the period changed. Without
/// that test, every change of a setting would postpone the next check.
pub fn create_alarm(name: &str, period_minutes: f64, delay_minutes: f64) -> Result<(), JsValue> {
    let info = object_from(&[
        ("periodInMinutes", JsValue::from_f64(period_minutes)),
        ("delayInMinutes", JsValue::from_f64(delay_minutes)),
    ])?;
    alarms_create(name, info.as_ref());
    Ok(())
}

/// Remove the alarm with this name. Nothing runs while the feature is off.
pub async fn clear_alarm(name: &str) {
    let _ = JsFuture::from(alarms_clear(name)).await;
}

/// The text of the badge on the toolbar icon. An empty text removes the badge.
pub async fn set_badge(text: &str, colour: &str) -> Result<(), JsValue> {
    let details = object_from(&[("text", JsValue::from_str(text))])?;
    JsFuture::from(action_set_badge_text(details.as_ref())).await?;
    if text.is_empty() {
        return Ok(());
    }
    let colour = object_from(&[("color", JsValue::from_str(colour))])?;
    JsFuture::from(action_set_badge_background_color(colour.as_ref())).await?;
    Ok(())
}

/// Listen to `chrome.storage.onChanged`.
///
/// The callback receives `(changes, areaName)`. There is no way to remove it: every
/// listener must stay as long as the page lives.
pub fn on_storage_changed(callback: &JsValue) {
    storage_on_changed_add(callback);
}

/// `chrome.storage.local.get` with an array of keys.
pub async fn local_get(keys: &js_sys::Array) -> Result<Object, JsValue> {
    JsFuture::from(storage_local_get(keys.as_ref()))
        .await?
        .dyn_into()
}

/// All of `chrome.storage.local` (`get(null)`).
///
/// Used to enumerate the keys, to remove the entries of an old key version.
pub async fn local_all() -> Result<Object, JsValue> {
    JsFuture::from(storage_local_get(&JsValue::NULL))
        .await?
        .dyn_into()
}

/// `chrome.storage.local.set`
pub async fn local_set(items: &Object) -> Result<(), JsValue> {
    JsFuture::from(storage_local_set(items.as_ref())).await?;
    Ok(())
}

/// `chrome.storage.local.remove`
pub async fn local_remove(keys: &js_sys::Array) -> Result<(), JsValue> {
    JsFuture::from(storage_local_remove(keys.as_ref())).await?;
    Ok(())
}

/// `chrome.runtime.sendMessage`. Waits for the reply of the service worker.
pub async fn send_message(message: &JsValue) -> Result<JsValue, JsValue> {
    JsFuture::from(runtime_send_message(message)).await
}

/// Build the JS object `{ key: value, ... }`.
pub fn object_from(entries: &[(&str, JsValue)]) -> Result<Object, JsValue> {
    let obj = Object::new();
    for (key, value) in entries {
        Reflect::set(&obj, &JsValue::from_str(key), value)?;
    }
    Ok(obj)
}

/// `chrome.storage.sync.get`. The keys of `defaults` that are absent get their default.
pub async fn storage_get(defaults: &Object) -> Result<Object, JsValue> {
    JsFuture::from(storage_sync_get(defaults.as_ref()))
        .await?
        .dyn_into()
}

/// `chrome.storage.sync.set`
pub async fn storage_set(items: &Object) -> Result<(), JsValue> {
    JsFuture::from(storage_sync_set(items.as_ref())).await?;
    Ok(())
}

/// `chrome.scripting.registerContentScripts`
pub async fn register_content_scripts(scripts: &js_sys::Array) -> Result<(), JsValue> {
    JsFuture::from(scripting_register(scripts.as_ref())).await?;
    Ok(())
}

/// `chrome.scripting.unregisterContentScripts({ ids })`
///
/// Not called with an empty `ids`: an implementation could read that as "remove all".
pub async fn unregister_content_scripts(ids: &[String]) -> Result<(), JsValue> {
    if ids.is_empty() {
        return Ok(());
    }
    let array = js_sys::Array::new();
    for id in ids {
        array.push(&JsValue::from_str(id));
    }
    let filter = object_from(&[("ids", array.into())])?;
    JsFuture::from(scripting_unregister(filter.as_ref())).await?;
    Ok(())
}

/// The ids of the dynamic content scripts that are registered now.
pub async fn registered_script_ids() -> Result<Vec<String>, JsValue> {
    let result = JsFuture::from(scripting_get_registered()).await?;
    let array: js_sys::Array = result.dyn_into()?;
    let mut ids = Vec::new();
    for entry in array.iter() {
        if let Ok(id) = Reflect::get(&entry, &JsValue::from_str("id"))
            && let Some(id) = id.as_string()
        {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// `chrome.scripting.insertCSS` into one tab.
///
/// A registration reaches the next load of a page. This puts the same files into a page
/// that is open now.
pub async fn insert_css(tab_id: f64, files: &[&str]) -> Result<(), JsValue> {
    let target = object_from(&[("tabId", JsValue::from_f64(tab_id))])?;
    let injection = object_from(&[
        ("target", target.into()),
        ("files", string_array(files).into()),
    ])?;
    JsFuture::from(scripting_insert_css(injection.as_ref())).await?;
    Ok(())
}

/// `chrome.declarativeNetRequest.updateEnabledRulesets`.
///
/// The rulesets of this extension change a header of the site, so they are declared as
/// disabled and only the feature that needs one enables it. The state is kept by Chrome,
/// and `on_installed` and `on_startup` set it again.
pub async fn update_enabled_rulesets(enable: &[&str], disable: &[&str]) -> Result<(), JsValue> {
    if enable.is_empty() && disable.is_empty() {
        return Ok(());
    }
    let options = object_from(&[
        ("enableRulesetIds", string_array(enable).into()),
        ("disableRulesetIds", string_array(disable).into()),
    ])?;
    JsFuture::from(dnr_update_enabled_rulesets(options.as_ref())).await?;
    Ok(())
}

/// A string slice as a JS array.
pub fn string_array(values: &[&str]) -> js_sys::Array {
    let array = js_sys::Array::new();
    for value in values {
        array.push(&JsValue::from_str(value));
    }
    array
}
