//! The features and their settings.
//!
//! The `FEATURES` table is the only source:
//!
//! - The service worker registers the CSS of the features that are on, at
//!   `document_start`.
//! - The options page builds one row per feature.
//! - The content script tests a feature before it starts it.
//!
//! A new feature is one new line here and appears in all three places.

use std::cell::RefCell;

use js_sys::Object;
use wasm_bindgen::JsValue;

use crate::chrome;

thread_local! {
    /// The settings that were read.
    ///
    /// `chrome.storage.sync.get` is asynchronous and every call is a round trip. Almost 20
    /// places read a setting, and the start ran more than ten of them. Some work must
    /// happen before the first paint, so those round trips are worth removing (the time
    /// to the first paint is 55ms).
    ///
    /// One page (or one start of the service worker) reads one time and then uses this.
    /// Call `invalidate` after a change, else this returns the old values.
    static CACHED: RefCell<Option<Object>> = const { RefCell::new(None) };
}

/// Discard the copy. Call it after a change of the settings.
pub fn invalidate() {
    CACHED.with_borrow_mut(|cache| *cache = None);
}

/// Read the settings, or return the copy.
async fn stored() -> Option<Object> {
    if let Some(cached) = CACHED.with_borrow(|cache| cache.clone()) {
        return Some(cached);
    }
    let fresh = chrome::storage_get(&defaults().ok()?).await.ok()?;
    CACHED.with_borrow_mut(|cache| *cache = Some(fresh.clone()));
    Some(fresh)
}

/// Where the content script runs. Keep it the same as in manifest.json.
///
/// The manifest says `run_at: document_start` (see `start` of `crates/core`).
pub const MATCHES: &[&str] = &["https://animestore.docomo.ne.jp/animestore/*"];

/// The books and the goods have another layout, so they are out of scope.
pub const EXCLUDE_MATCHES: &[&str] = &[
    "https://animestore.docomo.ne.jp/animestore/book/*",
    "https://animestore.docomo.ne.jp/animestore/ec/*",
];

/// Prefix of the ids of the dynamic registrations, so no other extension is touched.
pub const SCRIPT_ID_PREFIX: &str = "dt-";

/// Feature that plays an episode in a float window. It needs `RULESET_CSP`.
pub const PLAYER_MODAL: &str = "player-modal";
/// Feature that draws the comments. It needs `RULESET_NICO_UA`.
pub const COMMENTS: &str = "comments";

/// Ruleset that adds `'self'` to the `frame-src` of the site, so the player page can be in
/// an iframe. It changes a security header, so it is declared as disabled and only
/// `PLAYER_MODAL` enables it (see `extension/rules.json`).
pub const RULESET_CSP: &str = "csp";
/// Ruleset that puts the name of this extension into the `User-Agent` of the search
/// request to nicovideo, which the interface requires. Only `COMMENTS` enables it.
pub const RULESET_NICO_UA: &str = "nico-ua";

/// Is the extension enabled? This stops it without a change in `chrome://extensions`.
///
/// With `false`:
///
/// - The service worker removes every registration, so the next page gets no CSS.
/// - The content script puts `dt-off` on `<html>` and starts nothing.
///
/// Every CSS file tests `dt-off`, so a tab that is already open also goes back.
pub const ENABLED: &str = "enabled";
pub const ENABLED_DEFAULT: bool = true;

/// CSS that tells the content script, synchronously, that the extension is enabled.
///
/// Registered only when it is enabled (see `extension/styles/enabled.css`).
pub const ENABLED_CSS: &str = "styles/enabled.css";
/// The variable that this CSS defines.
pub const ENABLED_VAR: &str = "--dt-enabled";

pub struct FeatureDef {
    /// Key in the storage. Also the id of the registration.
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// CSS to register at `document_start`.
    pub css: &'static [&'static str],
}

pub const FEATURES: &[FeatureDef] = &[
    FeatureDef {
        id: "list-grid",
        label: "一覧を全幅グリッドで表示する",
        description: "860px 固定 2 列の一覧（続きから見る / 気になる / 視聴履歴 / コンプリート / 全アニメ一覧）を、画面幅いっぱいのグリッドにします。",
        css: &["styles/list-grid.css"],
    },
    FeatureDef {
        id: "episode-grid",
        label: "エピソードを全話並べる",
        description: "作品ページのエピソード一覧を、12 話ずつの横送りから全話のグリッドに変えます。",
        css: &["styles/episode-grid.css"],
    },
    FeatureDef {
        id: "no-promo",
        label: "ポップアップ広告を出さない",
        description: "クーポン等の全面ポップアップ（サイトの dialog.popup-wrapper）を閉じます。隠すだけではページ全体が押せないままになるので、閉じるところまでやります。",
        css: &["styles/no-promo.css"],
    },
    FeatureDef {
        id: "top-page",
        label: "トップページを組み替える",
        description: "15 本ある横スクロールの帯（中身の幅は 4000〜10000px）を、自前の 1 画面に組み替えます。注目 1 作品のヒーロー・今日更新・順位つきランキング・チップで切り替える「さがす」の 4 段構成で、縦 4700px → 1372px になります。",
        css: &["styles/top-page.css"],
    },
    FeatureDef {
        id: "work-hero",
        label: "作品ページの見出しを全幅にする",
        description: "キービジュアル・タイトル・操作を全幅のヒーローにまとめます。見出しの高さが 563px から 320px になり、エピソード一覧が早く出ます。",
        css: &["styles/work-hero.css"],
    },
    FeatureDef {
        id: "work-detail",
        label: "作品情報を表にまとめる",
        description: "あらすじ・ジャンル・キャスト・スタッフの折りたたみをやめ、役名と担当者を対にした表で並べます。",
        css: &["styles/work-detail.css"],
    },
    FeatureDef {
        id: "infinite-scroll",
        label: "一覧を無限スクロールにする",
        description: "マイページ系の一覧でページ送りを隠し、スクロールで次のページを継ぎ足します。",
        css: &["styles/infinite-scroll.css"],
    },
    FeatureDef {
        id: "player-modal",
        label: "同じ画面内のウィンドウで再生する",
        description: "再生ボタンでページ遷移せず、一覧の上に重ねて再生します。Esc または背景クリックで閉じます。",
        css: &["styles/player-modal.css"],
    },
    FeatureDef {
        id: "search-overlay",
        label: "「さがす」をフロート検索にする",
        description: "ヘッダの「さがす」でページ遷移せず、打ちながら結果が出るフロートを重ねます（⌘K / Ctrl+K、/ でも開きます）。",
        css: &["styles/search-overlay.css"],
    },
    FeatureDef {
        id: "comments",
        label: "ニコニコのコメントを表示する",
        description: "フロート再生中に、ニコニコ動画の公式配信から同じ話のコメントを取ってきて重ねます。作品名と話数で動画を突き合わせるため、見つからないこともあります。",
        // The CSS is in player-modal.css; this only appears inside the float
        css: &[],
    },
    FeatureDef {
        id: "debug-view",
        label: "動画情報のデバッグ表示を出す",
        description: "フロート再生の右下に、配信フレームレート・コマ番号・解像度・先読み・フレーム落ちなどを重ねます。",
        css: &[],
    },
];

/// A setting that has no CSS of its own: a detail of the behaviour.
pub struct SwitchDef {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default: bool,
}

/// Remove the rentals from the search. `search_overlay` uses the same key.
pub const SEARCH_NO_RENTAL: &str = "search-no-rental";
/// Are the keyboard shortcuts of the float search on?
pub const SEARCH_SHORTCUTS: &str = "search-shortcuts";
/// Is the skip button of the float player on?
pub const PLAYER_SKIP: &str = "player-skip";

pub const SWITCHES: &[SwitchDef] = &[
    SwitchDef {
        id: SEARCH_NO_RENTAL,
        label: "検索でレンタル作品を除く",
        description: "見放題だけを出します。除外は API（vodTypeList）に任せるので、件数表示も除いた後の数になります。フロート検索のトグルと同じ設定です。",
        default: true,
    },
    SwitchDef {
        id: PLAYER_SKIP,
        label: "本編へのスキップボタンを出す",
        description: "配信データの章立てから、映像の右下に「本編へスキップ」を出します（判定はサイトのプレイヤーと同じ規則）。エンディングでは「次の話へ」になります。アバン（本編相当）は飛ばさず、自動スキップもしません。",
        default: true,
    },
    SwitchDef {
        id: SEARCH_SHORTCUTS,
        label: "キーボードでフロート検索を開く",
        description: "⌘K（Ctrl+K）と / で検索を開きます。切ってもヘッダの「さがす」からは開けます。",
        default: true,
    },
];

/// A setting with a list of values. Every value is a string, also a number.
///
/// This is a table of its own, so the options page and the popup can build a column of
/// switches and a column of lists.
pub struct ChoiceDef {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// `(value in the storage, text on the screen)`
    pub options: &'static [(&'static str, &'static str)],
    pub default: &'static str,
}

pub const CHOICES: &[ChoiceDef] = &[
    ChoiceDef {
        id: DANMAKU_FPS_KEY,
        label: "弾幕の描画レート",
        description: "上げるほど滑らかに流れますが、そのぶん CPU を使います。コマ数の表示は映像基準の 24fps 固定です。",
        options: &[
            ("24", "24 fps"),
            ("30", "30 fps"),
            ("48", "48 fps"),
            ("60", "60 fps"),
        ],
        default: "30",
    },
    ChoiceDef {
        id: DANMAKU_DURATION_KEY,
        label: "弾幕が流れきる時間",
        description: "コメントが画面を横切るのにかける秒数です。長くすると読みやすくなり、そのぶん同時に出る本数が増えます。",
        options: &[
            ("3", "3 秒（速い）"),
            ("4", "4 秒"),
            ("6", "6 秒"),
            ("8", "8 秒（ゆっくり）"),
        ],
        default: "4",
    },
    ChoiceDef {
        id: SEARCH_SORT_KEY,
        label: "検索の既定の並び",
        description: "フロート検索を開いたときの並び順です。開いた後はその場でも変えられます。",
        // The same values as the search page of the site (#listsort)
        options: &[
            ("4", "関連度順"),
            ("1", "再生数順"),
            ("7", "気になる登録数順"),
            ("3", "リリース順"),
            ("6", "製作年度順"),
            ("2", "５０音順"),
        ],
        default: "4",
    },
    ChoiceDef {
        id: CARD_MIN_WIDTH_KEY,
        label: "一覧カードの下限幅",
        description: "この幅を下回らない範囲で列数が決まります。狭くすると 1 画面に多く並び、広くすると 1 枚が大きくなります。",
        options: &[
            ("auto", "画面に合わせる（既定）"),
            ("180px", "180px（多く並べる）"),
            ("220px", "220px"),
            ("260px", "260px"),
            ("320px", "320px（大きく見る）"),
        ],
        default: "auto",
    },
    ChoiceDef {
        id: THUMB_SIZE_KEY,
        label: "サムネイルの解像度",
        description: "全幅グリッドではサイトが配る画像（288px など）が引き伸ばされてぼやけるので、既定では 640px 版に差し替えます。通信量を増やしたくないときは「そのまま」を選んでください。",
        // The site also has 1280 and 1920, but they are not options here: they are 12.5
        // and 22 times the bytes of the default of the site, and a card is at most 320px
        // wide, so they look the same.
        options: &[
            ("1", "640×360（既定）"),
            (THUMB_SIZE_OFF, "そのまま（通信量が最小）"),
        ],
        default: "1",
    },
];

/// Draw rate of the danmaku, in fps.
///
/// The frame counter uses the fps of the video, so this changes only how smooth the
/// comments move.
pub const DANMAKU_FPS_KEY: &str = "danmaku-fps";
pub const DANMAKU_FPS_DEFAULT: f64 = 30.0;
pub const DANMAKU_FPS_MIN: f64 = 24.0;
pub const DANMAKU_FPS_MAX: f64 = 60.0;
/// Seconds for one comment to cross the screen.
pub const DANMAKU_DURATION_KEY: &str = "danmaku-duration";
pub const DANMAKU_DURATION_DEFAULT: f64 = 4.0;
pub const DANMAKU_DURATION_MIN: f64 = 2.0;
pub const DANMAKU_DURATION_MAX: f64 = 12.0;
/// Default sort of the float search (`sortKey`).
pub const SEARCH_SORT_KEY: &str = "search-sort";
/// Minimum width of a card (`--dt-card-min` in the CSS). `auto` writes nothing.
pub const CARD_MIN_WIDTH_KEY: &str = "card-min-width";
/// Size of a thumbnail.
pub const THUMB_SIZE_KEY: &str = "thumb-size";
/// The value that keeps the image of the site.
pub const THUMB_SIZE_OFF: &str = "off";

/// The id of the registration of a feature.
pub fn script_id(feature_id: &str) -> String {
    format!("{SCRIPT_ID_PREFIX}{feature_id}")
}

fn choice_def(id: &str) -> Option<&'static ChoiceDef> {
    CHOICES.iter().find(|choice| choice.id == id)
}

fn switch_def(id: &str) -> Option<&'static SwitchDef> {
    SWITCHES.iter().find(|switch| switch.id == id)
}

/// The defaults as a JS object, for `chrome.storage.sync.get`.
///
/// Every feature is on. The switches and the lists are also here, so a setting that was
/// never written returns its default.
pub fn defaults() -> Result<Object, JsValue> {
    let mut entries: Vec<(&str, JsValue)> = FEATURES
        .iter()
        .map(|f| (f.id, JsValue::from_bool(true)))
        .collect();
    entries.push((ENABLED, JsValue::from_bool(ENABLED_DEFAULT)));
    for switch in SWITCHES {
        entries.push((switch.id, JsValue::from_bool(switch.default)));
    }
    for choice in CHOICES {
        entries.push((choice.id, JsValue::from_str(choice.default)));
    }
    chrome::object_from(&entries)
}

/// Read a list setting. An absent or invalid value gives the default.
///
/// A number in the storage is also read as a string: the draw rate of the danmaku was a
/// number in an earlier version, and it must still work.
pub async fn choice(id: &str) -> String {
    let fallback = choice_def(id).map(|c| c.default).unwrap_or_default();
    let Some(stored) = stored().await else {
        return fallback.to_string();
    };
    let Ok(value) = js_sys::Reflect::get(&stored, &JsValue::from_str(id)) else {
        return fallback.to_string();
    };
    if let Some(text) = value.as_string() {
        return text;
    }
    if let Some(number) = value.as_f64().filter(|n| n.is_finite()) {
        // 24.0 becomes "24", the form of the values of the list
        return format!("{number}");
    }
    fallback.to_string()
}

/// Write a list setting. The copy is then old, so it is discarded.
pub async fn save_choice(id: &str, value: &str) -> Result<(), JsValue> {
    let items = chrome::object_from(&[(id, JsValue::from_str(value))])?;
    invalidate();
    chrome::storage_set(&items).await
}

/// Read a list setting as a number. A value outside the range is clamped.
async fn choice_number(id: &str, fallback: f64, min: f64, max: f64) -> f64 {
    choice(id)
        .await
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(min, max))
        .unwrap_or(fallback)
}

/// Draw rate of the danmaku.
pub async fn danmaku_fps() -> f64 {
    choice_number(
        DANMAKU_FPS_KEY,
        DANMAKU_FPS_DEFAULT,
        DANMAKU_FPS_MIN,
        DANMAKU_FPS_MAX,
    )
    .await
}

/// Seconds for one comment to cross the screen.
pub async fn danmaku_duration() -> f64 {
    choice_number(
        DANMAKU_DURATION_KEY,
        DANMAKU_DURATION_DEFAULT,
        DANMAKU_DURATION_MIN,
        DANMAKU_DURATION_MAX,
    )
    .await
}

/// Is a switch on? An unreadable value gives the default.
pub async fn switch_enabled(id: &str) -> bool {
    let fallback = switch_def(id).map(|s| s.default).unwrap_or(true);
    let Some(stored) = stored().await else {
        return fallback;
    };
    js_sys::Reflect::get(&stored, &JsValue::from_str(id))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(fallback)
}

/// Is the extension enabled? An unreadable value counts as enabled.
pub async fn is_extension_enabled() -> bool {
    let Some(stored) = stored().await else {
        return ENABLED_DEFAULT;
    };
    js_sys::Reflect::get(&stored, &JsValue::from_str(ENABLED))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(ENABLED_DEFAULT)
}

/// All settings. The options page and the popup build their UI from one read.
pub struct Snapshot {
    /// Is the extension enabled?
    pub enabled: bool,
    /// In the order of `FEATURES`.
    pub features: Vec<bool>,
    /// In the order of `SWITCHES`.
    pub switches: Vec<bool>,
    /// In the order of `CHOICES`.
    pub choices: Vec<String>,
}

/// Read every setting in one call.
///
/// One `chrome.storage` call per row would be one round trip per row.
pub async fn snapshot() -> Result<Snapshot, JsValue> {
    let stored = stored()
        .await
        .ok_or_else(|| JsValue::from_str("settings unavailable"))?;
    let read = |id: &str| js_sys::Reflect::get(&stored, &JsValue::from_str(id)).ok();

    let features = FEATURES
        .iter()
        .map(|f| read(f.id).and_then(|v| v.as_bool()).unwrap_or(true))
        .collect();
    let switches = SWITCHES
        .iter()
        .map(|s| read(s.id).and_then(|v| v.as_bool()).unwrap_or(s.default))
        .collect();
    let choices = CHOICES
        .iter()
        .map(|c| {
            read(c.id)
                .and_then(|v| {
                    v.as_string()
                        // A setting that was a number in an earlier version must still
                        // work (the draw rate of the danmaku)
                        .or_else(|| v.as_f64().filter(|n| n.is_finite()).map(|n| format!("{n}")))
                })
                .unwrap_or_else(|| c.default.to_string())
        })
        .collect();

    Ok(Snapshot {
        enabled: read(ENABLED)
            .and_then(|v| v.as_bool())
            .unwrap_or(ENABLED_DEFAULT),
        features,
        switches,
        choices,
    })
}

/// The features that are on, in the order of `FEATURES`.
pub async fn load() -> Result<Vec<bool>, JsValue> {
    let stored = stored()
        .await
        .ok_or_else(|| JsValue::from_str("settings unavailable"))?;
    let mut enabled = Vec::with_capacity(FEATURES.len());
    for feature in FEATURES {
        let value = js_sys::Reflect::get(&stored, &JsValue::from_str(feature.id))?;
        // An absent or invalid value counts as on; the default is on
        enabled.push(value.as_bool().unwrap_or(true));
    }
    Ok(enabled)
}

/// Write the state of one feature. The copy is then old, so it is discarded.
pub async fn save_one(feature_id: &str, value: bool) -> Result<(), JsValue> {
    let items = chrome::object_from(&[(feature_id, JsValue::from_bool(value))])?;
    invalidate();
    chrome::storage_set(&items).await
}

/// Is the feature with this id on?
pub async fn is_enabled(feature_id: &str) -> bool {
    let Ok(enabled) = load().await else {
        // Without a readable setting, the default is on
        return true;
    };
    FEATURES
        .iter()
        .zip(enabled)
        .find(|(f, _)| f.id == feature_id)
        .map(|(_, on)| on)
        .unwrap_or(true)
}
