//! Reads the necessary values out of a JS value, such as the result of
//! `fetch().json()`.
//!
//! serde would be easier, but the WASM is instantiated on every page load, so the size
//! matters more and this uses `js_sys::Reflect`. `path()` walks a nested value.

use js_sys::{Array, Reflect};
use wasm_bindgen::JsValue;

/// `obj[key]`. `undefined` and `null` give `None`.
pub fn get(obj: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(obj, &JsValue::from_str(key))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
}

/// Walk `obj[a][b][c]…`. An absent step gives `None`.
pub fn path(obj: &JsValue, keys: &[&str]) -> Option<JsValue> {
    let mut current = obj.clone();
    for key in keys {
        current = get(&current, key)?;
    }
    Some(current)
}

pub fn get_string(obj: &JsValue, key: &str) -> Option<String> {
    get(obj, key)?.as_string()
}

pub fn get_f64(obj: &JsValue, key: &str) -> Option<f64> {
    get(obj, key)?.as_f64()
}

/// Read it as an array. `None` if it is not an array.
pub fn get_array(obj: &JsValue, key: &str) -> Option<Array> {
    let value = get(obj, key)?;
    if value.is_array() {
        Some(Array::from(&value))
    } else {
        None
    }
}

/// Build `{ key: value, … }`.
pub fn object(entries: &[(&str, JsValue)]) -> Result<js_sys::Object, JsValue> {
    let obj = js_sys::Object::new();
    for (key, value) in entries {
        Reflect::set(&obj, &JsValue::from_str(key), value)?;
    }
    Ok(obj)
}
