//! d-tweaks / core
//!
//! The content script: it reads the DOM of the site and draws the own UI.
//!
//! The layout is in `extension/styles/*.css` at `document_start`. WASM cannot do that
//! work, because its instantiate is asynchronous and the original layout would be
//! visible until it ends.
//!
//! This crate does the work that CSS cannot do: parse the DOM, build own elements,
//! ask the REST interfaces, and hold the state.

mod features;
mod page;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlElement, Window, console};

use d_tweaks_shared::settings;
use page::PageKind;

/// Selector of a list container. Keep it the same as in `list-grid.css`.
///
/// - Without `.onlySpLayout`: the site sends the PC and the SP DOM.
/// - With `clearfix`: else this also takes `.p-slider__itemList` of the top page and
///   `.swiper-wrapper` of a work page.
/// - A direct child of `.contentsWrapper` or `.pageWrapper`: a feature page
///   (`/animestore/CF/summer`) also has `.itemWrapper.clearfix` under `.weekWrapper`,
///   with another structure inside.
///
/// If the CSS and this selector do not agree, the grid is active but the cards are
/// the original ones (or the other way). Never change only one of the two.
pub(crate) const LIST_SELECTOR: &str =
    ":is(.contentsWrapper, .pageWrapper) > .itemWrapper.clearfix:not(.onlySpLayout)";

/// The kill switch. Every CSS file tests this class.
const OFF_CLASS: &str = "dt-off";

pub(crate) fn log(msg: &str) {
    console::log_1(&JsValue::from_str(&format!("[d-tweaks] {msg}")));
}

/// Wait with `setTimeout`.
pub(crate) async fn sleep(millis: i32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Seconds as `11:15` or `1:02:03`. Used by the comment list and the control bar.
pub(crate) fn timestamp(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as i64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

/// The computed `display` of an element.
fn display_of(window: &Window, el: &Element) -> Result<String, JsValue> {
    window
        .get_computed_style(el)?
        .ok_or_else(|| JsValue::from_str("no computed style"))?
        .get_property_value("display")
}

/// Report if the first element of `selector` has `display: grid`.
///
/// `label` is for the log, `child_selector` counts the cards.
fn verify_grid(
    window: &Window,
    document: &Document,
    label: &str,
    selector: &str,
    child_selector: &str,
) -> Result<(), JsValue> {
    let containers = document.query_selector_all(selector)?;
    if containers.length() == 0 {
        log(&format!("{label}: コンテナなし（このページでは対象外）"));
        return Ok(());
    }

    let el: Element = containers
        .item(0)
        .ok_or_else(|| JsValue::from_str("no item"))?
        .dyn_into()?;
    let display = display_of(window, &el)?;
    let cards = el.query_selector_all(child_selector)?.length();

    log(&format!(
        "{label}: containers={} display={display} cards={cards}",
        containers.length()
    ));

    if display == "grid" {
        log(&format!("OK: {label} のグリッド化が効いています"));
    } else {
        log(&format!(
            "WARN: {label} のグリッド化が効いていません（display={display}）"
        ));
    }
    Ok(())
}

/// Put the appearance settings on the page.
///
/// - Minimum card width: writes the CSS variable `--dt-card-min` on `<html>`. An
///   inline style is stronger than a rule for `:root`, so this is enough. `auto`
///   writes nothing and leaves the `clamp` of the CSS.
/// - Thumbnail size: the size that `card_view` asks for.
async fn apply_appearance() {
    let size = settings::choice(settings::THUMB_SIZE_KEY).await;
    // The cards drawn at the start use the default, so replace them here
    match features::card_view::apply_thumb_size(&size) {
        Ok(0) => {}
        Ok(count) => log(&format!("サムネイルを {size} 版に貼り替え: {count} 枚")),
        Err(err) => log(&format!("サムネイルの貼り替えに失敗: {err:?}")),
    }

    let width = settings::choice(settings::CARD_MIN_WIDTH_KEY).await;
    if width == "auto" || width.is_empty() {
        return;
    }
    let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .and_then(|el| el.dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    if let Err(err) = root.style().set_property("--dt-card-min", &width) {
        log(&format!("カード幅の設定を反映できませんでした: {err:?}"));
    }
}

/// Entry of the content script.
///
/// The manifest says `document_start`, and this function waits. Measured with
/// `document_idle`:
///
/// | | Time |
/// |---|---|
/// | responseEnd | 253ms |
/// | DOMContentLoaded | 398ms |
/// | load | 913ms |
/// | Fetch and compile of the WASM | 4ms (290KB) |
///
/// `document_idle` runs after `load`, so the original page was visible for about
/// 500ms. The WASM needs only 4ms, so an early load and a wait for the DOM is
/// faster.
///
/// At `document_start` there is no `document.body`, so `run` waits for
/// `DOMContentLoaded`. If the document is ready, it runs at once.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;

    if document.ready_state() == "loading" {
        let on_ready = Closure::once_into_js(move || {
            if let Err(err) = run() {
                log(&format!("初期化に失敗: {err:?}"));
            }
        });
        document.add_event_listener_with_callback("DOMContentLoaded", on_ready.unchecked_ref())?;
        return Ok(());
    }
    run()
}

/// Add or remove `dt-off` (the kill switch) on `<html>`.
///
/// Every CSS file tests this class, so this is enough to give the site back. The own
/// elements also disappear, through the `html.dt-off …` rules.
fn apply_off(off: bool) {
    let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    else {
        return;
    };
    let list = root.class_list();
    let _ = if off {
        list.add_1(OFF_CLASS)
    } else {
        list.remove_1(OFF_CLASS)
    };
}

/// Is the extension enabled? Synchronous.
///
/// Reads a variable of the CSS that the service worker registers only when the
/// extension is enabled (`settings::ENABLED_CSS`). `chrome.storage` is asynchronous,
/// so it cannot answer before the first paint, which the cards need. Only when the
/// variable is absent does the code ask the storage.
fn enabled_marker(window: &Window, document: &Document) -> Option<bool> {
    let root = document.document_element()?;
    let style = window.get_computed_style(&root).ok()??;
    let value = style.get_property_value(settings::ENABLED_VAR).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value != "0")
    }
}

/// Follow the enabled setting.
fn watch_enabled() {
    let on_change =
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |changes: JsValue, area: JsValue| {
            if area.as_string().as_deref() != Some("sync") {
                return;
            }
            let Some(entry) = d_tweaks_shared::json::get(&changes, settings::ENABLED) else {
                return;
            };
            let Some(enabled) =
                d_tweaks_shared::json::get(&entry, "newValue").and_then(|value| value.as_bool())
            else {
                return;
            };
            apply_off(!enabled);
            if enabled {
                log("全体を有効にしました（自前 UI はページを再読み込みすると出ます）");
            } else {
                log("全体を無効にしました（サイト本来の表示に戻しました）");
            }
        });
    d_tweaks_shared::chrome::on_storage_changed(on_change.as_ref());
    // Listen as long as the page lives
    on_change.forget();
}

fn run() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    // Do nothing while the extension is off.
    //
    // With the variable (the service worker registered the CSS), decide here. Without
    // it, ask the storage first: the registration can be late.
    watch_enabled();
    match enabled_marker(&window, &document) {
        Some(true) => {}
        Some(false) => {
            apply_off(true);
            log("全体が無効です（設定で切り替えられます）");
            return Ok(());
        }
        None => {
            wasm_bindgen_futures::spawn_local(async {
                if settings::is_extension_enabled().await {
                    if let Err(err) = install_all() {
                        log(&format!("初期化に失敗: {err:?}"));
                    }
                } else {
                    apply_off(true);
                    log("全体が無効です（設定で切り替えられます）");
                }
            });
            return Ok(());
        }
    }

    install_all()
}

fn install_all() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let kind = PageKind::detect(&window)?;

    // The container decides if the list is drawn, not a list of page names.
    //
    // The CSS only tests `LIST_SELECTOR`, so a list of names here would not agree
    // with it. `/animestore/CF/new_anime` was absent from that list, and the failure
    // was: the original cards stand in the own grid.
    let list = document.query_selector(LIST_SELECTOR)?;
    log(&format!(
        "wasm booted / page = {kind:?} / 一覧の枠 = {}",
        if list.is_some() { "あり" } else { "なし" }
    ));

    // One listener for the failures of all images
    if let Err(err) = features::card_view::install_thumb_fallback() {
        log(&format!(
            "サムネイルの戻し処理を仕掛けられませんでした: {err:?}"
        ));
    }

    // Close the promotion dialog. It appears on every page.
    //
    // CSS alone is not enough: while a modal `<dialog>` is open, the rest of the page
    // is inert, so nothing below it can be clicked (see `promo`).
    wasm_bindgen_futures::spawn_local(async {
        if !settings::is_enabled("no-promo").await {
            return;
        }
        if let Err(err) = features::promo::install() {
            log(&format!("ポップアップ広告の処理に失敗: {err:?}"));
        }
    });

    // The appearance settings (card width, image size).
    //
    // Both should be known before the cards, but `chrome.storage` is asynchronous. The
    // CSS has a default for `:root`, so a late value only means the default.
    wasm_bindgen_futures::spawn_local(async {
        apply_appearance().await;
    });

    // The float search. Every page has the header, so this is not only for a list.
    wasm_bindgen_futures::spawn_local(async {
        if !d_tweaks_shared::settings::is_enabled("search-overlay").await {
            log("フロート検索: 設定で無効");
            return;
        }
        let shortcuts = d_tweaks_shared::settings::switch_enabled(settings::SEARCH_SHORTCUTS).await;
        if let Err(err) = features::search_overlay::install(shortcuts) {
            log(&format!("フロート検索の初期化に失敗: {err:?}"));
        }
    });

    // The float player, on a list, a work page and the top page.
    //
    // The top page is here because the cards of "on air now" have a play link
    // (`sc_d_pc?partId=`, see `top_page`).
    //
    // The setting is asynchronous, so this is a task. The links work without it, so a
    // late start only means a navigation.
    if list.is_some() || matches!(kind, PageKind::Work | PageKind::Top) {
        wasm_bindgen_futures::spawn_local(async {
            if !d_tweaks_shared::settings::is_enabled("player-modal").await {
                log("フロート再生: 設定で無効");
                return;
            }
            if let Err(err) = features::player_modal::install() {
                log(&format!("フロート再生の初期化に失敗: {err:?}"));
            }
        });
    }

    if let Some(container) = &list {
        verify_grid(
            &window,
            &document,
            "一覧",
            LIST_SELECTOR,
            ":scope > .itemModule",
        )?;
        // Draw own cards. The parse absorbs the differences between the pages.
        match features::card_view::render_in(container, features::card_view::Source::List) {
            Ok(n) => log(&format!("カードを自前 UI で描画: {n} 件")),
            Err(err) => log(&format!("カード描画に失敗: {err:?}")),
        }
        // Under CF, the JS of the site inserts cards later
        if let Err(err) = features::card_view::observe(container, features::card_view::Source::List)
        {
            log(&format!("カードの監視を仕掛けられませんでした: {err:?}"));
        }

        // The layout is in the CSS at `document_start`, so a late start here does not
        // show the original layout.
        wasm_bindgen_futures::spawn_local(async move {
            if !d_tweaks_shared::settings::is_enabled("infinite-scroll").await {
                log("無限スクロール: 設定で無効");
                return;
            }
            match features::infinite_scroll::install() {
                Ok(true) => {}
                Ok(false) => log("無限スクロール: 対象外（ページャなし / 1 ページで収まっている）"),
                Err(err) => log(&format!("無限スクロールの初期化に失敗: {err:?}")),
            }
        });
    }

    match kind {
        PageKind::Top => {
            // Build the top page. The CSS hides nothing before `.dt-top-rendered`, so
            // a late start only leaves the sliders of the site.
            wasm_bindgen_futures::spawn_local(async {
                if !settings::is_enabled("top-page").await {
                    log("トップページ: 設定で無効");
                    return;
                }
                features::top_page::install().await;
            });
        }
        PageKind::Work => {
            // Replace the head with a full-width hero. The CSS hides nothing before
            // `.dt-hero-rendered`, so a late start only leaves the original head.
            wasm_bindgen_futures::spawn_local(async {
                if !d_tweaks_shared::settings::is_enabled("work-hero").await {
                    log("作品ヒーロー: 設定で無効");
                    return;
                }
                match features::work_hero::render() {
                    Ok(true) => log("作品ページの見出しを描き直しました"),
                    Ok(false) => log("作品ヒーロー: 対象外（見出しの形が想定と違う）"),
                    Err(err) => log(&format!("作品ヒーローの描画に失敗: {err:?}")),
                }
            });

            // The summary, the cast and the staff as own tables
            wasm_bindgen_futures::spawn_local(async {
                if !d_tweaks_shared::settings::is_enabled("work-detail").await {
                    log("作品情報: 設定で無効");
                    return;
                }
                match features::work_detail::render() {
                    Ok(true) => {}
                    Ok(false) => log("作品情報: 対象外（形が想定と違う）"),
                    Err(err) => log(&format!("作品情報の描画に失敗: {err:?}")),
                }
            });

            // The episodes as an own section. The original Swiper is hidden.
            match features::work_hero::render_episodes() {
                Ok(true) => {}
                Ok(false) => log("エピソード: 対象外（一覧が見つからない）"),
                Err(err) => log(&format!("エピソードの描画に失敗: {err:?}")),
            }
        }
        _ if list.is_none() => log("このページには表示改造を入れていません"),
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::timestamp;

    #[test]
    fn formats_timestamps() {
        assert_eq!(timestamp(0.0), "0:00");
        assert_eq!(timestamp(9.9), "0:09");
        assert_eq!(timestamp(675.26), "11:15");
        // More than one hour (a film) also shows the hours
        assert_eq!(timestamp(3723.0), "1:02:03");
        // A negative value is not a failure
        assert_eq!(timestamp(-5.0), "0:00");
    }
}
