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
