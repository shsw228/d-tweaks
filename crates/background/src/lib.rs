//! Logic of the service worker.
//!
//! # Why the CSS is registered dynamically
//!
//! Every feature can be off, but `chrome.storage` is asynchronous, so at
//! `document_start` nothing can answer "is this feature on?". Static CSS in the
//! manifest with a remove after it would show the CSS of a feature that is off.
//!
//! So only the CSS of the features that are on is registered, with
//! `chrome.scripting.registerContentScripts` and `runAt: document_start`. The
//! registration is kept with `persistAcrossSessions`, so the next page load has the
//! CSS before the first paint and reads no storage.
//!
//! # `sw.js` adds the listeners
//!
//! An MV3 service worker must add its listeners synchronously. A wait for the WASM
//! init would lose `onInstalled`, so `extension/sw.js` adds the listeners and calls
//! this crate after the init.

mod cache;
mod matching;
mod niconico;

use js_sys::Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::console;

use d_tweaks_shared::settings::{self, EXCLUDE_MATCHES, FEATURES, MATCHES};
use d_tweaks_shared::{chrome, json, messages};

fn log(msg: &str) {
    console::log_1(&JsValue::from_str(&format!("[d-tweaks/sw] {msg}")));
}

/// Read the settings and make the registrations agree with them.
async fn sync_registrations() -> Result<(), JsValue> {
    let enabled = settings::load().await?;

    // Only the registrations of this extension
    let ours: Vec<String> = chrome::registered_script_ids()
        .await?
        .into_iter()
        .filter(|id| id.starts_with(settings::SCRIPT_ID_PREFIX))
        .collect();
    chrome::unregister_content_scripts(&ours).await?;

    // With the extension off, register nothing.
    //
    // The content script is declared in the manifest and always runs, so it tests this
    // registration through a CSS variable and returns (see `settings::ENABLED_CSS`).
    if !settings::is_extension_enabled().await {
        sync_rulesets(false, false).await;
        log("全体が無効なので、登録をすべて外した");
        return Ok(());
    }

    let scripts = Array::new();
    let mut on = Vec::new();
    for (feature, is_on) in FEATURES.iter().zip(&enabled) {
        if !is_on || feature.css.is_empty() {
            continue;
        }
        let entry = chrome::object_from(&[
            ("id", JsValue::from_str(&settings::script_id(feature.id))),
            ("matches", chrome::string_array(MATCHES).into()),
            (
                "excludeMatches",
                chrome::string_array(EXCLUDE_MATCHES).into(),
            ),
            ("css", chrome::string_array(feature.css).into()),
            ("runAt", JsValue::from_str("document_start")),
            ("allFrames", JsValue::from_bool(false)),
            ("persistAcrossSessions", JsValue::from_bool(true)),
        ])?;
        scripts.push(&entry);
        on.push(feature.id);
    }

    // The marker that tells the content script, synchronously, that this is on
    let marker = chrome::object_from(&[
        ("id", JsValue::from_str(&settings::script_id("enabled"))),
        ("matches", chrome::string_array(MATCHES).into()),
        (
            "excludeMatches",
            chrome::string_array(EXCLUDE_MATCHES).into(),
        ),
        ("css", chrome::string_array(&[settings::ENABLED_CSS]).into()),
        ("runAt", JsValue::from_str("document_start")),
        ("allFrames", JsValue::from_bool(false)),
        ("persistAcrossSessions", JsValue::from_bool(true)),
    ])?;
    scripts.push(&marker);

    if scripts.length() > 0 {
        chrome::register_content_scripts(&scripts).await?;
    }

    let feature_on = |id: &str| {
        FEATURES
            .iter()
            .zip(&enabled)
            .any(|(feature, is_on)| feature.id == id && *is_on)
    };
    sync_rulesets(
        feature_on(settings::PLAYER_MODAL),
        feature_on(settings::COMMENTS),
    )
    .await;

    log(&format!("登録を更新: [{}]", on.join(", ")));
    Ok(())
}

/// Enable a header rule only while the feature that needs it is on.
///
/// Both rulesets change a header of a request of the site, so neither may be active
/// because the extension is installed. `extension/rules.json` weakens the `frame-src` of
/// the site, and that must stop the moment the float player is off.
async fn sync_rulesets(player_modal: bool, comments: bool) {
    let mut enable = Vec::new();
    let mut disable = Vec::new();
    for (id, on) in [
        (settings::RULESET_CSP, player_modal),
        (settings::RULESET_NICO_UA, comments),
    ] {
        if on {
            enable.push(id)
        } else {
            disable.push(id)
        }
    }
    if let Err(err) = chrome::update_enabled_rulesets(&enable, &disable).await {
        log(&format!("ヘッダ規則の更新に失敗: {}", describe(&err)));
    }
}

fn run_sync() {
    spawn_local(async {
        if let Err(err) = sync_registrations().await {
            log(&format!("登録の更新に失敗: {err:?}"));
        }
    });
}

/// Called from `chrome.runtime.onInstalled`.
#[wasm_bindgen]
pub fn on_installed() {
    log("onInstalled");
    run_sync();
    // An old map after a change of the match logic makes a correction have no effect
    spawn_local(async {
        match cache::drop_stale_video_entries().await {
            Ok(0) => {}
            Ok(count) => log(&format!("古い対応表を捨てた: {count} 件")),
            Err(err) => log(&format!("古い対応表の掃除に失敗: {}", describe(&err))),
        }
        // The same for a "not found": it would hide a correction of the match for a day
        match cache::drop_missing_video_entries().await {
            Ok(0) => {}
            Ok(count) => log(&format!("「該当なし」の控えを捨てた: {count} 件")),
            Err(err) => log(&format!("「該当なし」の掃除に失敗: {}", describe(&err))),
        }
    });
}

/// Called from `chrome.storage.onChanged`.
#[wasm_bindgen]
pub fn on_settings_changed() {
    log("設定が変更された");
    // The settings are kept in memory, so discard them and read again
    settings::invalidate();
    run_sync();
}

/// Called when the settings page removed the comment cache (the index key is gone).
#[wasm_bindgen]
pub fn on_comment_cache_cleared() {
    log("コメントの控えが消された");
    // Remove the ids that point to comments that are gone
    cache::forget_index();
}

/// Also make the registrations agree at the start of the service worker.
#[wasm_bindgen]
pub fn on_startup() {
    log("onStartup");
    run_sync();
}

/// Called from `chrome.runtime.onMessage`. Returns the reply.
///
/// An exception would stop `sw.js` from sending a reply, and the sender would wait for
/// ever, so this function always returns a value. A failure is an error reply.
#[wasm_bindgen]
pub async fn on_message(message: JsValue, sender: JsValue) -> JsValue {
    match handle(&message, &sender).await {
        Ok(reply) => reply,
        Err(err) => {
            let message = describe(&err);
            log(&format!("処理に失敗: {message}"));
            messages::reply_error(&message).unwrap_or(JsValue::UNDEFINED)
        }
    }
}

/// A readable string of a `JsValue` error.
fn describe(err: &JsValue) -> String {
    if let Some(text) = err.as_string() {
        return text;
    }
    // An Error object has a message
    json::get_string(err, "message").unwrap_or_else(|| format!("{err:?}"))
}

async fn handle(message: &JsValue, sender: &JsValue) -> Result<JsValue, JsValue> {
    match json::get_string(message, "type").as_deref() {
        Some(messages::COMMENTS) => comments(message).await,
        Some(messages::ENABLE_NOW) => enable_now(sender).await,
        // Not for this extension. Return undefined and leave it to another listener.
        _ => Ok(JsValue::UNDEFINED),
    }
}

/// Put the CSS of the enabled features into the tab that asked.
///
/// The master switch went on while that page was open. A registration only reaches the
/// next load, and the page has no CSS at all, because the worker removes every
/// registration while the extension is off.
async fn enable_now(sender: &JsValue) -> Result<JsValue, JsValue> {
    let Some(tab_id) = json::path(sender, &["tab", "id"]).and_then(|id| id.as_f64()) else {
        return Err(JsValue::from_str("送信元のタブが分かりません"));
    };
    if !settings::is_extension_enabled().await {
        return Err(JsValue::from_str("全体が無効です"));
    }

    let enabled = settings::load().await?;
    let mut files = vec![settings::ENABLED_CSS];
    for (feature, is_on) in FEATURES.iter().zip(&enabled) {
        if *is_on {
            files.extend_from_slice(feature.css);
        }
    }
    chrome::insert_css(tab_id, &files).await?;
    log(&format!(
        "開いているタブに CSS を入れた: {} 本",
        files.len()
    ));

    json::object(&[("ok", JsValue::TRUE)]).map(Into::into)
}

/// Select a video from the work title and the episode, and return the comments.
async fn comments(message: &JsValue) -> Result<JsValue, JsValue> {
    let part_id = json::get_string(message, "partId").unwrap_or_default();
    let work_title = json::get_string(message, "workTitle").unwrap_or_default();
    let episode_label = json::get_string(message, "episodeLabel");
    let episode_title = json::get_string(message, "episodeTitle");
    let duration = json::get_f64(message, "durationSeconds");

    if work_title.trim().is_empty() {
        return Err(JsValue::from_str("作品名がありません"));
    }
    // Without a partId, the work title and the episode are the key
    let key = if part_id.is_empty() {
        format!("{work_title}|{}", episode_label.clone().unwrap_or_default())
    } else {
        part_id
    };

    let video = match cache::video_id(&key).await {
        Some(cache::VideoIdHit::Found(video)) => video,
        Some(cache::VideoIdHit::Missing) => return messages::reply_not_found(),
        None => match resolve(
            &key,
            &work_title,
            episode_label.as_deref(),
            episode_title.as_deref(),
            duration,
        )
        .await?
        {
            Some(picked) => picked,
            None => return messages::reply_not_found(),
        },
    };
    let (video_id, video_title, video_seconds) = (video.id, video.title, video.seconds);

    if let Some(cached) = cache::comments(&video_id).await {
        log(&format!(
            "{video_id}: キャッシュから {} 件",
            cached.length()
        ));
        return messages::reply_ok(&video_id, &video_title, video_seconds, &cached, true);
    }

    let list = niconico::comments(&video_id).await?;
    let array = niconico::comments_to_js(&list)?;
    log(&format!("{video_id}: 取得 {} 件", array.length()));
    // The comments are usable without the cache, so only log a failure
    if let Err(err) = cache::put_comments(&video_id, &array).await {
        log(&format!("キャッシュ保存に失敗: {}", describe(&err)));
    }
    messages::reply_ok(&video_id, &video_title, video_seconds, &array, false)
}

/// Search, select one video, and keep the result in the map. Also keeps "not found".
///
/// Returns the title and the length also: the title shows which video gave the
/// comments, and a difference of the lengths shifts the comments (the nicovideo
/// version can be some seconds longer, because of a logo or a sponsor card).
async fn resolve(
    key: &str,
    work_title: &str,
    episode_label: Option<&str>,
    episode_title: Option<&str>,
    duration: Option<f64>,
) -> Result<Option<cache::VideoRef>, JsValue> {
    let query = matching::sanitize_title(work_title);
    if query.is_empty() {
        return Err(JsValue::from_str(&format!(
            "作品名から検索語を作れません: {work_title}"
        )));
    }

    let want = matching::Want {
        // The match uses the title after `sanitize_title`
        work_title: &query,
        episode_label,
        episode_title,
        duration_seconds: duration,
        // The season comes from the original title; `query` has no season in it
        season: matching::season_token(work_title),
    };

    // With a known episode number, put it in the search words.
    //
    // One request gives 100 items, so the work title alone loses the old episodes.
    // Measured: one work had 125 hits and the first episode was absent. With the
    // number the result is two items (a kanji number also works). If that gives
    // nothing, search again with the work title alone, for another form of the title.
    let mut queries = Vec::new();
    if let Some(label) = episode_label.map(str::trim).filter(|l| !l.is_empty()) {
        queries.push(format!("{query} {label}"));
    }
    queries.push(query.clone());

    let mut picked: Option<cache::VideoRef> = None;
    for attempt in &queries {
        let candidates = niconico::search(attempt).await?;
        match matching::pick(&candidates, &want) {
            Some(found) => {
                log(&format!(
                    "検索 \"{attempt}\" {} 件 → {} {} ({} 秒)",
                    candidates.len(),
                    found.content_id,
                    found.title,
                    found.length_seconds.unwrap_or(0.0)
                ));
                picked = Some(cache::VideoRef {
                    id: found.content_id.clone(),
                    title: found.title.clone(),
                    seconds: found.length_seconds,
                });
                break;
            }
            None => log(&format!(
                "検索 \"{attempt}\" {} 件 / 該当なし（{}）",
                candidates.len(),
                episode_label.unwrap_or("話数なし")
            )),
        }
    }

    if let Err(err) = cache::put_video_id(key, picked.as_ref()).await {
        log(&format!("対応表の保存に失敗: {}", describe(&err)));
    }
    Ok(picked)
}
