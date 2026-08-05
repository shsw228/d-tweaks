//! Cache of the comments, in `chrome.storage.local`.
//!
//! # Two layers
//!
//! | Layer | Key | Content | Time |
//! |---|---|---|---|
//! | Map | `dt:vid:<partId>` | The videoId and the title, or `null` | 30 days, or one day for "not found" |
//! | Comments | `dt:cmt:<videoId>` | The comments | 6 hours |
//!
//! The times differ. The search and the selection (`matching`) are the least certain
//! step, and 100 items with a title comparison for every open of the same episode is
//! wasteful, so a map that was correct is kept for a long time. The comments grow with
//! the time, so they are read again after a short time.
//!
//! A "not found" is also kept (`videoId: null`): a work that is not on nicovideo is
//! still not there on the next open. It is kept for a short time only, because the
//! channel can add the video later.
//!
//! # The eviction uses the write order, not the read order
//!
//! One video can have some MB of comments, which fills the 10MB of `storage.local`. So
//! the number of videos has a limit, and the oldest write goes first. A read order
//! (LRU) would write the index on every read, and the comments live only six hours, so
//! the better order is not worth that.
//!
//! # The index has a copy in memory
//!
//! `chrome.storage` is only asynchronous. If another request comes between the read,
//! the change and the write, one of the two changes is lost (two tabs at the same time
//! are enough). So the index has a copy in memory, a change happens only in a block
//! without an `await`, and the write sends the copy. Then no change is lost.
//!
//! A write can also be too large for the storage (one video with many comments). If
//! `set` fails, the older half goes and the write runs one more time.
//!
//! The comments are usable without the cache, so a caller must not treat an error
//! here as fatal.

use std::cell::RefCell;

use js_sys::{Array, Date};
use wasm_bindgen::JsValue;

use d_tweaks_shared::{chrome, json};

use d_tweaks_shared::cache_keys::{COMMENT_INDEX as INDEX_KEY, COMMENT_PREFIX, VIDEO_PREFIX};

const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
/// The map from an episode to a video does not change, so this is long.
const VIDEO_TTL_MS: f64 = 30.0 * DAY_MS;
/// A "not found". The channel can add the video later, so this is short.
const VIDEO_MISS_TTL_MS: f64 = DAY_MS;
/// The comments grow, so this is short.
const COMMENT_TTL_MS: f64 = 6.0 * 60.0 * 60.0 * 1000.0;
/// Videos with comments in the storage.
const MAX_ENTRIES: usize = 20;

thread_local! {
    /// Copy of the index (the video ids in write order). `None` before the first read.
    ///
    /// It goes with the service worker, and then the storage gives it again.
    static INDEX: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// The video that was selected.
pub struct VideoRef {
    pub id: String,
    /// The title on nicovideo. An old cache entry has none, so it can be empty.
    pub title: String,
    /// The length on nicovideo. A difference to dAnime shifts the comments.
    pub seconds: Option<f64>,
}

/// Result of a look in the map. Separates "nothing here" from "known to be absent".
pub enum VideoIdHit {
    Found(VideoRef),
    /// A search ran and found no official video.
    Missing,
}

fn comment_key(video_id: &str) -> String {
    format!("{COMMENT_PREFIX}{video_id}")
}

async fn read(key: &str) -> Option<JsValue> {
    let keys = Array::new();
    keys.push(&JsValue::from_str(key));
    let stored: JsValue = chrome::local_get(&keys).await.ok()?.into();
    json::get(&stored, key)
}

async fn write(key: &str, value: &JsValue) -> Result<(), JsValue> {
    let items = json::object(&[(key, value.clone())])?;
    chrome::local_set(&items).await
}

/// Milliseconds since the write. `None` for an entry without `at`.
fn age_ms(entry: &JsValue) -> Option<f64> {
    Some(Date::now() - json::get_f64(entry, "at")?)
}

/// The videoId of a partId. `None` if it is absent or too old.
/// Remove the map entries of an old key version. Returns the number removed.
///
/// A new version of the key makes the old entries unread but not absent, so they would
/// hold storage for nothing.
pub async fn drop_stale_video_entries() -> Result<u32, JsValue> {
    use d_tweaks_shared::cache_keys::VIDEO_ROOT;

    let all = chrome::local_all().await?;
    let stale: Vec<String> = js_sys::Object::keys(&all)
        .iter()
        .filter_map(|key| key.as_string())
        .filter(|key| key.starts_with(VIDEO_ROOT) && !key.starts_with(VIDEO_PREFIX))
        .collect();
    if stale.is_empty() {
        return Ok(0);
    }
    let keys = Array::new();
    for key in &stale {
        keys.push(&JsValue::from_str(key));
    }
    chrome::local_remove(&keys).await?;
    Ok(stale.len() as u32)
}

/// Remove the "not found" entries of the map. Returns the number removed.
///
/// A "not found" is kept for one day, which stops a search for a work that nicovideo
/// does not have. But after an update of the extension the match logic can be another
/// one, and then a "not found" of the old logic hides the correction for a day. This ran
/// in a real session: one episode said "not found" although the interface returned the
/// correct video, because an earlier version of the match had written that entry.
///
/// The cost of this is one search for the episodes that a user opens again on the day of
/// the update.
pub async fn drop_missing_video_entries() -> Result<u32, JsValue> {
    let all = chrome::local_all().await?;
    let value: JsValue = all.clone().into();

    let keys = Array::new();
    for key in js_sys::Object::keys(&all)
        .iter()
        .filter_map(|key| key.as_string())
        .filter(|key| key.starts_with(VIDEO_PREFIX))
    {
        // `json::get` makes null a None, so an entry without a videoId is a "not found"
        let missing = json::get(&value, &key)
            .map(|entry| json::get_string(&entry, "videoId").is_none())
            .unwrap_or(false);
        if missing {
            keys.push(&JsValue::from_str(&key));
        }
    }
    if keys.length() == 0 {
        return Ok(0);
    }
    chrome::local_remove(&keys).await?;
    Ok(keys.length())
}

pub async fn video_id(key: &str) -> Option<VideoIdHit> {
    let entry = read(&format!("{VIDEO_PREFIX}{key}")).await?;
    let age = age_ms(&entry)?;
    // `json::get` makes null a None, so the presence of the value is the answer to
    // "found" or "not found"
    match json::get_string(&entry, "videoId") {
        Some(id) if age < VIDEO_TTL_MS => Some(VideoIdHit::Found(VideoRef {
            id,
            title: json::get_string(&entry, "videoTitle").unwrap_or_default(),
            seconds: json::get_f64(&entry, "videoSeconds"),
        })),
        None if age < VIDEO_MISS_TTL_MS => Some(VideoIdHit::Missing),
        _ => None,
    }
}

/// Write to the map. `None` is kept as "not found".
pub async fn put_video_id(key: &str, picked: Option<&VideoRef>) -> Result<(), JsValue> {
    let (id, title, seconds) = match picked {
        Some(video) => (
            JsValue::from_str(&video.id),
            JsValue::from_str(&video.title),
            video
                .seconds
                .map(JsValue::from_f64)
                .unwrap_or(JsValue::NULL),
        ),
        None => (JsValue::NULL, JsValue::NULL, JsValue::NULL),
    };
    let entry = json::object(&[
        ("videoId", id),
        ("videoTitle", title),
        ("videoSeconds", seconds),
        ("at", JsValue::from_f64(Date::now())),
    ])?;
    write(&format!("{VIDEO_PREFIX}{key}"), &entry.into()).await
}

/// The comments from the cache. `None` if they are too old.
pub async fn comments(video_id: &str) -> Option<Array> {
    let entry = read(&comment_key(video_id)).await?;
    if age_ms(&entry)? >= COMMENT_TTL_MS {
        return None;
    }
    json::get_array(&entry, "comments")
}

/// Write the comments. Removes what is over the limit, and more if the write fails.
pub async fn put_comments(video_id: &str, comments: &Array) -> Result<(), JsValue> {
    let value: JsValue = json::object(&[
        ("comments", comments.clone().into()),
        ("at", JsValue::from_f64(Date::now())),
    ])?
    .into();

    load_index().await;

    // The end of the index is the newest. The same video moves to the end.
    let evicted = edit_index(|ids| {
        ids.retain(|id| id != video_id);
        ids.push(video_id.to_string());
        let over = ids.len().saturating_sub(MAX_ENTRIES);
        ids.drain(..over).collect::<Vec<String>>()
    });
    remove_comments(&evicted).await;

    let key = comment_key(video_id);
    let mut result = write(&key, &value).await;
    if result.is_err() {
        // Too large for the storage. Keep the newer half, which has this video in it.
        let evicted = edit_index(|ids| {
            let half = ids.len().saturating_sub(1) / 2;
            ids.drain(..half).collect::<Vec<String>>()
        });
        remove_comments(&evicted).await;
        result = write(&key, &value).await;
    }
    if result.is_err() {
        // An id in the index without its comments would be evicted for ever
        edit_index(|ids| ids.retain(|id| id != video_id));
    }
    // Write the index also after a failure: the eviction did happen
    save_index().await?;
    result
}

/// Make the copy of the index available, with one read of the storage.
async fn load_index() {
    if INDEX.with_borrow(Option::is_some) {
        return;
    }
    let ids: Vec<String> = match read(INDEX_KEY).await {
        Some(value) if value.is_array() => Array::from(&value)
            .iter()
            .filter_map(|v| v.as_string())
            .collect(),
        _ => Vec::new(),
    };
    // Another request can have made it during the wait; keep that one
    INDEX.with_borrow_mut(|slot| {
        slot.get_or_insert(ids);
    });
}

/// Change the copy of the index. No `await`, so nothing comes between.
fn edit_index<T>(edit: impl FnOnce(&mut Vec<String>) -> T) -> T {
    INDEX.with_borrow_mut(|slot| edit(slot.get_or_insert_with(Vec::new)))
}

/// Write the copy of the index to the storage.
async fn save_index() -> Result<(), JsValue> {
    let array = Array::new();
    INDEX.with_borrow(|slot| {
        for id in slot.iter().flatten() {
            array.push(&JsValue::from_str(id));
        }
    });
    write(INDEX_KEY, &array.into()).await
}

/// Discard the copy of the index. Call it when the settings page removed the cache.
///
/// Without this call, the copy keeps ids of comments that are gone, the count of the
/// videos is wrong, and an eviction happens before the storage is full.
pub fn forget_index() {
    INDEX.with_borrow_mut(|slot| *slot = None);
}

async fn remove_comments(video_ids: &[String]) {
    if video_ids.is_empty() {
        return;
    }
    let keys = Array::new();
    for id in video_ids {
        keys.push(&JsValue::from_str(&comment_key(id)));
    }
    // A failure is not a problem; the next write tries again
    let _ = chrome::local_remove(&keys).await;
}
