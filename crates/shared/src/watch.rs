//! The works that are watched for a new episode, and the episodes that were found.
//!
//! # Two keys, one writer
//!
//! | Key | Content |
//! |---|---|
//! | `dt:watch:works:v1` | The watched works, with the last episode that was seen |
//! | `dt:watch:news:v1` | The new episodes, newest first |
//!
//! Both are in `storage.local` (the comment cache is there for the same reason: the
//! quota of `storage.sync` is small and this list grows with the use).
//!
//! **Only the service worker writes them.** The work page and the popup send a message
//! (`messages::WATCH_*`). `chrome.storage` is only asynchronous, so a read, a change and
//! a write from two places lose one of the two changes; the comment index had that
//! defect. One writer removes the problem instead of solving it.
//!
//! # What is kept per work
//!
//! `tail` is the last episode of the chain of the site (`nextPartId` of `WS030101`).
//! The check asks for that episode and reads `nextPartId`: one request of about 1KB
//! answers "is there a new episode?". The work page is 108KB and gives the same answer.
//!
//! `next_at` and `misses` hold the back-off. A work that does not change is asked less
//! often (`misses` doubles the wait), so a work that ended settles at one request a day.
//! A new episode sets `misses` back to 0.

use js_sys::Array;
use wasm_bindgen::JsValue;

use crate::json;

/// The watched works.
pub const WORKS_KEY: &str = "dt:watch:works:v1";
/// The new episodes that were found, newest first.
pub const NEWS_KEY: &str = "dt:watch:news:v1";

/// Entries in the list. The popup shows a history, not only the unread ones, so this is
/// more than one screen; it is a limit against a list that grows for ever.
pub const MAX_NEWS: usize = 200;

/// Works that can be watched.
///
/// Every work costs one request per check, so a limit keeps the traffic of the extension
/// under the traffic of one page of the site.
pub const MAX_WORKS: usize = 100;

/// A work that is watched.
#[derive(Clone, Debug, PartialEq)]
pub struct Watched {
    pub work_id: String,
    /// The work title, as the work page writes it. Only for the display.
    pub title: String,
    /// The last episode of the chain that is known. `None` until the first read.
    pub tail: Option<String>,
    /// Time of the last answer (a check or a visit of the page).
    pub checked_at: f64,
    /// The earliest time of the next check. Holds the back-off.
    pub next_at: f64,
    /// Checks in sequence that found nothing. Doubles the wait.
    pub misses: u32,
    /// When an episode was added the last time, or the time of the registration.
    ///
    /// A work that gives nothing for a long time is not watched any more (see
    /// `STOP_AFTER_DAYS` in the service worker). `0` means "not known", and a work with
    /// that value is never stopped: an entry that an older version wrote must not
    /// disappear because of a value that it never had.
    pub last_new_at: f64,
}

impl Watched {
    pub fn to_js(&self) -> Result<JsValue, JsValue> {
        Ok(json::object(&[
            ("workId", JsValue::from_str(&self.work_id)),
            ("title", JsValue::from_str(&self.title)),
            (
                "tail",
                self.tail
                    .as_deref()
                    .map(JsValue::from_str)
                    .unwrap_or(JsValue::NULL),
            ),
            ("checkedAt", JsValue::from_f64(self.checked_at)),
            ("nextAt", JsValue::from_f64(self.next_at)),
            ("misses", JsValue::from_f64(self.misses as f64)),
            ("lastNewAt", JsValue::from_f64(self.last_new_at)),
        ])?
        .into())
    }

    pub fn from_js(value: &JsValue) -> Option<Self> {
        let work_id = json::get_string(value, "workId")?;
        if work_id.trim().is_empty() {
            return None;
        }
        Some(Self {
            work_id,
            title: json::get_string(value, "title").unwrap_or_default(),
            tail: json::get_string(value, "tail").filter(|id| !id.trim().is_empty()),
            checked_at: json::get_f64(value, "checkedAt").unwrap_or(0.0),
            next_at: json::get_f64(value, "nextAt").unwrap_or(0.0),
            misses: json::get_f64(value, "misses").unwrap_or(0.0).max(0.0) as u32,
            last_new_at: json::get_f64(value, "lastNewAt").unwrap_or(0.0).max(0.0),
        })
    }
}

/// A row of the list: a new episode, or the end of a watch.
///
/// The end of a watch is in the same list, because it is the same question ("what happened
/// to the works I registered?") and a second list would need a second place in the popup.
/// An `ended` row has no `part_id`.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub work_id: String,
    pub work_title: String,
    pub part_id: String,
    /// `第20話`, `PROLOGUE`. The interface gives it as it is written on the site.
    pub number: Option<String>,
    /// The subtitle of the episode.
    pub title: Option<String>,
    /// The still of the episode (`mainScenePath` of the interface, or the image of the
    /// card on the work page). The popup draws it.
    pub thumb: Option<String>,
    /// When this extension found it (not the time of the release).
    pub at: f64,
    /// Was the popup open after this entry arrived?
    pub seen: bool,
    /// This row says "the watch of this work ended", not "an episode arrived".
    pub ended: bool,
}

/// The row that says that a watch ended.
pub fn ended_row(work_id: &str, work_title: &str, at: f64) -> Found {
    Found {
        work_id: work_id.to_string(),
        work_title: work_title.to_string(),
        part_id: String::new(),
        number: None,
        title: None,
        thumb: None,
        at,
        // Not a new episode, so it must not put a number on the badge: the badge would
        // then say "an episode arrived" when nothing arrived. The row stays in the list.
        seen: true,
        ended: true,
    }
}

impl Found {
    pub fn to_js(&self) -> Result<JsValue, JsValue> {
        Ok(json::object(&[
            ("workId", JsValue::from_str(&self.work_id)),
            ("workTitle", JsValue::from_str(&self.work_title)),
            ("partId", JsValue::from_str(&self.part_id)),
            (
                "number",
                self.number
                    .as_deref()
                    .map(JsValue::from_str)
                    .unwrap_or(JsValue::NULL),
            ),
            (
                "title",
                self.title
                    .as_deref()
                    .map(JsValue::from_str)
                    .unwrap_or(JsValue::NULL),
            ),
            (
                "thumb",
                self.thumb
                    .as_deref()
                    .map(JsValue::from_str)
                    .unwrap_or(JsValue::NULL),
            ),
            ("at", JsValue::from_f64(self.at)),
            ("seen", JsValue::from_bool(self.seen)),
            ("ended", JsValue::from_bool(self.ended)),
        ])?
        .into())
    }

    pub fn from_js(value: &JsValue) -> Option<Self> {
        let ended = json::get(value, "ended")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // An episode row without an id cannot be drawn or opened; an `ended` row has no id
        let part_id = json::get_string(value, "partId").unwrap_or_default();
        if !ended && part_id.trim().is_empty() {
            return None;
        }
        Some(Self {
            work_id: json::get_string(value, "workId").unwrap_or_default(),
            work_title: json::get_string(value, "workTitle").unwrap_or_default(),
            part_id,
            number: json::get_string(value, "number").filter(|t| !t.trim().is_empty()),
            title: json::get_string(value, "title").filter(|t| !t.trim().is_empty()),
            thumb: json::get_string(value, "thumb").filter(|t| !t.trim().is_empty()),
            at: json::get_f64(value, "at").unwrap_or(0.0),
            seen: json::get(value, "seen")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            ended,
        })
    }
}

/// Read a stored list. An entry that cannot be read is dropped, not fatal: the shape can
/// come from an older version.
pub fn works_from_js(value: &JsValue) -> Vec<Watched> {
    if !value.is_array() {
        return Vec::new();
    }
    Array::from(value)
        .iter()
        .filter_map(|entry| Watched::from_js(&entry))
        .collect()
}

pub fn news_from_js(value: &JsValue) -> Vec<Found> {
    if !value.is_array() {
        return Vec::new();
    }
    Array::from(value)
        .iter()
        .filter_map(|entry| Found::from_js(&entry))
        .collect()
}

pub fn works_to_js(works: &[Watched]) -> Result<Array, JsValue> {
    let array = Array::new();
    for work in works {
        array.push(&work.to_js()?);
    }
    Ok(array)
}

pub fn news_to_js(news: &[Found]) -> Result<Array, JsValue> {
    let array = Array::new();
    for entry in news {
        array.push(&entry.to_js()?);
    }
    Ok(array)
}

/// The same image in another size.
///
/// The site keeps one image in more than one size, and the size is the last number of the
/// file name (`…_1_2.png`; measured: `1` is 640x360 and `2` is 288x162). A row of the popup
/// is 72px wide, so the 640 version would be nine times the bytes for no difference.
///
/// `card_view::resize_thumb` of the content script does the same rewrite, but it also obeys
/// the resolution setting of the lists. That setting is about the cards of a list, not about
/// a row of the popup, so this is the plain mechanism without it.
///
/// A name that does not end in a size is given back as it is.
pub fn thumb_at_size(url: &str, size: &str) -> String {
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    };
    let Some((stem, extension)) = path.rsplit_once('.') else {
        return url.to_string();
    };
    let Some((head, current)) = stem.rsplit_once('_') else {
        return url.to_string();
    };
    if current.is_empty() || !current.bytes().all(|b| b.is_ascii_digit()) {
        return url.to_string();
    }
    let mut resized = format!("{head}_{size}.{extension}");
    if let Some(query) = query {
        resized.push('?');
        resized.push_str(query);
    }
    resized
}

/// The unread entries. The badge on the toolbar icon shows this number.
pub fn unread(news: &[Found]) -> usize {
    news.iter().filter(|entry| !entry.seen).count()
}

/// `第20話 一番じゃないやり方`, or what of it exists.
///
/// The interface writes the number as the site writes it (`第20話`, `PROLOGUE`), so it is
/// not built here and it is not translated: it is the text of the service.
pub fn label_of(entry: &Found) -> String {
    match (&entry.number, &entry.title) {
        (Some(number), Some(title)) => format!("{number} {title}"),
        (Some(number), None) => number.clone(),
        (None, Some(title)) => title.clone(),
        (None, None) => entry.part_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(number: Option<&str>, title: Option<&str>) -> Found {
        Found {
            work_id: "25767".into(),
            work_title: "作品".into(),
            part_id: "25767051".into(),
            number: number.map(str::to_string),
            title: title.map(str::to_string),
            thumb: None,
            at: 0.0,
            seen: false,
            ended: false,
        }
    }

    #[test]
    fn builds_a_label_from_the_parts_that_exist() {
        assert_eq!(
            label_of(&entry(Some("第20話"), Some("一番じゃないやり方"))),
            "第20話 一番じゃないやり方"
        );
        assert_eq!(label_of(&entry(Some("PROLOGUE"), None)), "PROLOGUE");
        assert_eq!(label_of(&entry(None, Some("特番"))), "特番");
        // Without either, the id is still something the user can recognise
        assert_eq!(label_of(&entry(None, None)), "25767051");
    }

    #[test]
    fn counts_only_the_unread_entries() {
        let mut read = entry(None, None);
        read.seen = true;
        assert_eq!(unread(&[entry(None, None), read]), 1);
    }

    #[test]
    fn changes_the_size_of_an_image_and_keeps_the_query() {
        assert_eq!(
            thumb_at_size("https://x/25767008_1_1.png?168", "2"),
            "https://x/25767008_1_2.png?168"
        );
        // Already that size
        assert_eq!(
            thumb_at_size("https://x/25767008_1_2.png", "2"),
            "https://x/25767008_1_2.png"
        );
        // Nothing that reads as a size: give the address back and load what there is
        assert_eq!(
            thumb_at_size("https://x/cover.png", "2"),
            "https://x/cover.png"
        );
        assert_eq!(
            thumb_at_size("https://x/no-extension", "2"),
            "https://x/no-extension"
        );
    }

    #[test]
    fn the_end_of_a_watch_puts_no_number_on_the_badge() {
        let row = ended_row("25767", "作品", 1.0);
        assert!(row.ended);
        assert!(row.part_id.is_empty());
        // The badge counts new episodes, and this is not one
        assert_eq!(unread(&[row]), 0);
    }
}
