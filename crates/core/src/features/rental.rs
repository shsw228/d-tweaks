//! The works that are rentals, so the own top page can leave them out.
//!
//! # Where the answer comes from
//!
//! `rest/WS000120` returns every rental work in one reply (measured: 300 works, 148KB,
//! and the request needs no account). The site itself calls it `getRentalList` in
//! `common.js`. The cards of the top page have no mark that says "rental", so this list
//! and the workId of the card are the only way to tell.
//!
//! # Only the ids are kept, and only for one day
//!
//! 148KB for one page load would be more than the images of a screen of cards, so the
//! workIds go into `storage.local` (about 2KB for 300 ids) with the time of the write.
//! One day later the list is read again: a work becomes a rental or stops being one on
//! the day of a release, not on the hour.
//!
//! The reply of the interface is not kept, only the ids. Nothing else of it is used.

use std::collections::HashSet;

use js_sys::Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use d_tweaks_shared::{chrome, json};

use crate::log;

/// Rental works. Same origin, so the content script can ask.
const RENTAL_URL: &str = "/animestore/rest/WS000120";
/// Key in `storage.local`. The version is in the key: a change of what is stored must not
/// read an old shape.
const CACHE_KEY: &str = "dt:rental:v1";
/// Time until the list is read again.
const TTL_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// The workIds of the rentals. `None` when neither the cache nor the request gives them.
///
/// A `None` must not remove a card: without an answer the page shows what the site sends,
/// which is the normal page.
pub async fn work_ids() -> Option<HashSet<String>> {
    if let Some(ids) = cached().await {
        return Some(ids);
    }
    let ids = fetch().await?;
    if let Err(err) = store(&ids).await {
        // The ids are usable without the cache; the next load asks again
        log(&format!("レンタル一覧を控えられませんでした: {err:?}"));
    }
    Some(ids)
}

async fn cached() -> Option<HashSet<String>> {
    let keys = Array::new();
    keys.push(&JsValue::from_str(CACHE_KEY));
    let stored: JsValue = chrome::local_get(&keys).await.ok()?.into();
    let entry = json::get(&stored, CACHE_KEY)?;
    let age = js_sys::Date::now() - json::get_f64(&entry, "at")?;
    if age >= TTL_MS {
        return None;
    }
    let ids = json::get_array(&entry, "ids")?;
    Some(ids.iter().filter_map(|id| id.as_string()).collect())
}

async fn store(ids: &HashSet<String>) -> Result<(), JsValue> {
    let array = Array::new();
    for id in ids {
        array.push(&JsValue::from_str(id));
    }
    let entry = json::object(&[
        ("ids", array.into()),
        ("at", JsValue::from_f64(js_sys::Date::now())),
    ])?;
    let items = json::object(&[(CACHE_KEY, entry.into())])?;
    chrome::local_set(&items).await
}

async fn fetch() -> Option<HashSet<String>> {
    let window = web_sys::window()?;
    let response: Response = JsFuture::from(window.fetch_with_str(RENTAL_URL))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !response.ok() {
        log(&format!(
            "レンタル一覧の取得に失敗: HTTP {}",
            response.status()
        ));
        return None;
    }
    let body = JsFuture::from(response.json().ok()?).await.ok()?;
    let list = json::path(&body, &["data", "workList"])
        .filter(|value| value.is_array())
        .map(|value| Array::from(&value))?;
    let ids: HashSet<String> = list
        .iter()
        .filter_map(|work| json::get_string(&work, "workId"))
        .collect();
    log(&format!("レンタル作品: {} 件", ids.len()));
    Some(ids)
}
