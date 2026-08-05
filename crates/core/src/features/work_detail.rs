//! Draws the summary, the genres, the cast and the staff of a work page as own tables.
//!
//! The original page has two folds, and the content is in these places:
//!
//! | Place | Content |
//! |---|---|
//! | `.outlineContainer > p` | The summary |
//! | `.outlineContainer .footerLink a` | The genres, as a pair (`ドラマ/青春`) |
//! | `.castContainer > p` | `[キャスト]`, `[スタッフ]`, `[製作年]` and the copyright |
//! | `.tagArea` | Cast, staff and other tags, as search links |
//!
//! The same data is in two places. The cast of `.tagArea` is only the names, but
//! `[キャスト]` of `.castContainer` has pairs of a role and a name. The pairs are more
//! readable, so the text comes from there and the links from `.tagArea`.
//!
//! # The separator is also inside a value
//!
//! The items are separated by a full-width slash, and a value can also have one:
//!
//! ```text
//! 原作:著者名「原作タイトル」（レーベル／出版社刊）／キャラクターデザイン:担当者名
//!                                             ^ not a separator
//! ```
//!
//! A simple split gives `（レーベル` and `出版社刊）`, so the code counts the brackets
//! and splits only outside of them.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::log;

/// Marks the page as drawn. The CSS hides the original DOM only with this class.
const RENDERED_CLASS: &str = "dt-detail-rendered";

/// The brackets. A separator inside them is not a separator.
const BRACKETS: [(char, char); 5] = [
    ('（', '）'),
    ('(', ')'),
    ('「', '」'),
    ('『', '』'),
    ('【', '】'),
];

/// Split on `separator`, but only outside of the brackets.
///
/// The depth never goes below 0, so a bracket without its pair does not break this.
fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;

    for ch in text.chars() {
        if BRACKETS.iter().any(|(open, _)| *open == ch) {
            depth += 1;
        } else if BRACKETS.iter().any(|(_, close)| *close == ch) {
            depth = (depth - 1).max(0);
        }

        if ch == separator && depth == 0 {
            parts.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(ch);
        }
    }
    parts.push(current.trim().to_string());
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// Split `role:name`. Without a separator, only the name.
///
/// The separator can be half width or full width (measured: `[キャスト]` uses `:` and the
/// staff of `.tagArea` uses `：`). A colon inside brackets is not a separator.
fn parse_pair(item: &str) -> (Option<String>, String) {
    let mut depth = 0i32;
    for (index, ch) in item.char_indices() {
        if BRACKETS.iter().any(|(open, _)| *open == ch) {
            depth += 1;
        } else if BRACKETS.iter().any(|(_, close)| *close == ch) {
            depth = (depth - 1).max(0);
        } else if (ch == ':' || ch == '：') && depth == 0 {
            let label = item[..index].trim();
            let value = item[index + ch.len_utf8()..].trim();
            if !label.is_empty() && !value.is_empty() {
                return (Some(label.to_string()), value.to_string());
            }
        }
    }
    (None, item.trim().to_string())
}

/// The content of a paragraph that starts with a label, such as `[キャスト] …`.
fn strip_prefix_label<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let text = text.trim();
    let head = format!("[{label}]");
    text.strip_prefix(&head).map(str::trim)
}

fn text_of(root: &Element, selector: &str) -> Option<String> {
    let el = root.query_selector(selector).ok()??;
    let text = el.text_content()?.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn element(document: &Document, tag: &str, class: &str) -> Result<Element, JsValue> {
    let el = document.create_element(tag)?;
    el.set_class_name(class);
    Ok(el)
}

/// A block with a heading.
fn block(document: &Document, title: &str) -> Result<(Element, Element), JsValue> {
    let root = element(document, "section", "dt-detail__block")?;
    let heading = element(document, "h3", "dt-detail__heading")?;
    heading.set_text_content(Some(title));
    root.append_child(&heading)?;
    let body = element(document, "div", "dt-detail__body")?;
    root.append_child(&body)?;
    Ok((root, body))
}

/// A table of role and name pairs. Two columns, like the debug view.
fn pair_list(document: &Document, items: &[String]) -> Result<Element, JsValue> {
    let list = element(document, "div", "dt-detail__pairs")?;
    for item in items {
        let (label, value) = parse_pair(item);
        let key = element(document, "span", "dt-detail__key")?;
        // An item without a pair has only a value; the key column stays empty
        key.set_text_content(Some(label.as_deref().unwrap_or("")));
        list.append_child(&key)?;
        let val = element(document, "span", "dt-detail__value")?;
        val.set_text_content(Some(&value));
        list.append_child(&val)?;
    }
    Ok(list)
}

/// The tags, as search links.
fn chip_list(
    document: &Document,
    source: &Element,
    caption: &str,
) -> Result<Option<Element>, JsValue> {
    let mut found = false;
    let list = element(document, "div", "dt-detail__chips")?;

    // A caption and a list follow each other, so walk them in order
    let children = source.query_selector_all(":scope > p.tagCaption, :scope > ul.tagWrapper")?;
    let mut collecting = false;
    for index in 0..children.length() {
        let Some(node) = children.item(index) else {
            continue;
        };
        let Ok(child) = node.dyn_into::<Element>() else {
            continue;
        };
        if child.matches("p.tagCaption")? {
            collecting = child
                .text_content()
                .map(|text| text.trim() == caption)
                .unwrap_or(false);
            continue;
        }
        if !collecting {
            continue;
        }
        let items = child.query_selector_all("li")?;
        for i in 0..items.length() {
            let Some(node) = items.item(i) else { continue };
            let Ok(li) = node.dyn_into::<Element>() else {
                continue;
            };
            let Some(text) = li.text_content().map(|t| t.trim().to_string()) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let href = li
                .query_selector("a")?
                .and_then(|a| a.get_attribute("href"))
                .filter(|href| !href.is_empty());
            let chip = element(
                document,
                if href.is_some() { "a" } else { "span" },
                "dt-detail__chip",
            )?;
            chip.set_text_content(Some(&text));
            if let Some(href) = &href {
                chip.set_attribute("href", href)?;
            }
            list.append_child(&chip)?;
            found = true;
        }
        collecting = false;
    }
    Ok(if found { Some(list) } else { None })
}

/// Draw the summary, the genres, the cast and the staff. `true` if it was drawn.
pub fn render() -> Result<bool, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let Some(anchor) = document.query_selector(".accordionWrapper")? else {
        return Ok(false);
    };
    if anchor.class_list().contains(RENDERED_CLASS) {
        return Ok(false);
    }
    // The same container as the episodes, so a wide screen can put them side by side
    let target = crate::features::work_hero::work_container(&document)?.unwrap_or(anchor.clone());

    let outline = document.query_selector(".outlineWrapper")?;
    let cast = document.query_selector(".castWrapper")?;
    let tags = document.query_selector(".tagArea")?;
    if outline.is_none() && cast.is_none() {
        return Ok(false);
    }

    let root = element(&document, "div", "dt-detail")?;

    // --- Summary ---
    if let Some(outline) = &outline
        && let Some(text) = text_of(outline, ".outlineContainer > p")
    {
        let (block_root, body) = block(&document, "あらすじ")?;
        let paragraph = element(&document, "p", "dt-detail__prose")?;
        paragraph.set_text_content(Some(&text));
        body.append_child(&paragraph)?;
        root.append_child(&block_root)?;
    }

    // --- Genres. They come as a pair of a group and a genre. ---
    if let Some(outline) = &outline {
        let links = outline.query_selector_all(".footerLink a")?;
        if links.length() > 0 {
            let (block_root, body) = block(&document, "ジャンル")?;
            let list = element(&document, "div", "dt-detail__chips")?;
            for index in 0..links.length() {
                let Some(node) = links.item(index) else {
                    continue;
                };
                let Ok(link) = node.dyn_into::<Element>() else {
                    continue;
                };
                let Some(text) = link.text_content().map(|t| t.trim().to_string()) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                let chip = element(&document, "a", "dt-detail__chip")?;
                // The two parts of the pair get spaces, so they are readable
                chip.set_text_content(Some(&text.replace('/', " / ")));
                if let Some(href) = link.get_attribute("href") {
                    chip.set_attribute("href", &href)?;
                }
                list.append_child(&chip)?;
            }
            body.append_child(&list)?;
            root.append_child(&block_root)?;
        }
    }

    // --- Cast and staff, as pairs with a role ---
    if let Some(cast) = &cast {
        let paragraphs = cast.query_selector_all(".castContainer > p")?;
        let mut copyright = None;
        for index in 0..paragraphs.length() {
            let Some(node) = paragraphs.item(index) else {
                continue;
            };
            let Ok(paragraph) = node.dyn_into::<Element>() else {
                continue;
            };
            let Some(text) = paragraph.text_content() else {
                continue;
            };
            let text = text.trim();
            if text.is_empty() {
                continue;
            }

            for label in ["キャスト", "スタッフ", "製作年"] {
                if let Some(rest) = strip_prefix_label(text, label) {
                    let items = split_top_level(rest, '／');
                    if items.is_empty() {
                        continue;
                    }
                    let (block_root, body) = block(&document, label)?;
                    let pairs = pair_list(&document, &items)?;
                    body.append_child(&pairs)?;
                    root.append_child(&block_root)?;
                }
            }
            if text.starts_with('©') {
                copyright = Some(text.to_string());
            }
        }

        // --- The tags. Only names, but they are the only search links. ---
        if let Some(tags) = &tags
            && let Some(chips) = chip_list(&document, tags, "その他")?
        {
            let (block_root, body) = block(&document, "その他")?;
            body.append_child(&chips)?;
            root.append_child(&block_root)?;
        }

        if let Some(copyright) = copyright {
            let note = element(&document, "p", "dt-detail__copyright")?;
            note.set_text_content(Some(&copyright));
            root.append_child(&note)?;
        }
    }

    if root.child_element_count() == 0 {
        return Ok(false);
    }

    target.append_child(&root)?;
    anchor.class_list().add_1(RENDERED_CLASS)?;
    log(&format!(
        "作品情報を自前の表で描画: {} 塊",
        root.child_element_count()
    ));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{parse_pair, split_top_level, strip_prefix_label};

    #[test]
    fn splits_only_outside_brackets() {
        // Real data. The separator inside the brackets must not split.
        let text =
            "原作:著者名「原作タイトル」（レーベル／出版社刊）／キャラクターデザイン:担当者名";
        let parts = split_top_level(text, '／');
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("レーベル／出版社刊"));
        assert_eq!(parts[1], "キャラクターデザイン:担当者名");
    }

    #[test]
    fn splits_plain_items() {
        let parts = split_top_level("ビクトリア・セラーズ:安済知佳／ノンナ:若山詩音", '／');
        assert_eq!(
            parts,
            vec!["ビクトリア・セラーズ:安済知佳", "ノンナ:若山詩音"]
        );
        // A separator at the end gives no empty item
        assert_eq!(split_top_level("A／B／", '／'), vec!["A", "B"]);
        // A bracket without its pair does not break the split
        assert_eq!(
            split_top_level("（開いたまま／B", '／'),
            vec!["（開いたまま／B"]
        );
    }

    #[test]
    fn parses_role_and_name() {
        assert_eq!(
            parse_pair("ビクトリア・セラーズ:安済知佳"),
            (Some("ビクトリア・セラーズ".into()), "安済知佳".into())
        );
        // A full-width colon also works (the staff of .tagArea uses it)
        assert_eq!(
            parse_pair("アニメーション制作：スタジオディーン"),
            (Some("アニメーション制作".into()), "スタジオディーン".into())
        );
        // Without a pair, only the value
        assert_eq!(
            parse_pair("原作タイトルシリーズ"),
            (None, "原作タイトルシリーズ".into())
        );
        // A colon inside brackets does not split
        assert_eq!(
            parse_pair("原作:著者名「A:B」"),
            (Some("原作".into()), "著者名「A:B」".into())
        );
    }

    #[test]
    fn strips_section_labels() {
        assert_eq!(
            strip_prefix_label("[キャスト] ビクトリア:安済知佳", "キャスト"),
            Some("ビクトリア:安済知佳")
        );
        assert_eq!(
            strip_prefix_label("[スタッフ] 原作:著者名", "キャスト"),
            None
        );
        assert_eq!(strip_prefix_label("©著者名/レーベル", "キャスト"), None);
    }
}
