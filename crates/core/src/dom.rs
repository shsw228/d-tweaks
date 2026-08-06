//! The DOM helpers that every feature needs.
//!
//! Each of these was a private copy in five to seven modules. The copies were the same,
//! or the same with another spelling, which is one more place to correct after a change.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

/// The document of the page.
pub(crate) fn document() -> Result<Document, JsValue> {
    web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))
}

/// A new element with a class.
pub(crate) fn element(document: &Document, tag: &str, class: &str) -> Result<Element, JsValue> {
    let el = document.create_element(tag)?;
    el.set_class_name(class);
    Ok(el)
}

/// The text of the first element of `selector`. Empty text gives `None`.
pub(crate) fn text_of(root: &Element, selector: &str) -> Option<String> {
    let text = root.query_selector(selector).ok()??.text_content()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// An attribute. An attribute of only spaces gives `None`.
pub(crate) fn attr(el: &Element, name: &str) -> Option<String> {
    el.get_attribute(name).filter(|v| !v.trim().is_empty())
}
