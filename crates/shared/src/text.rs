//! The words of the own UI, in Japanese and in English.
//!
//! # Why not `chrome.i18n`
//!
//! `chrome.i18n` takes the language of the browser and nothing else: an extension cannot
//! change it, and Chrome has no setting for one extension. The service is Japanese, so a
//! user of it can have an English browser and still want Japanese words (or the other way
//! round). So the language is a setting of this extension (`settings::UI_LANG`), and
//! `_locales/` stays for what only Chrome can show: the name and the description in the
//! store and on the extensions page.
//!
//! # One table, two columns
//!
//! `WORDS` holds every word of the own UI. A key that is not in the table gives the key
//! itself, which is visible at once, and a missing English word gives the Japanese one, so
//! a half-translated table still works.
//!
//! The language is read one time and kept, because `t` is called from the drawing code,
//! which is not async.

use std::cell::Cell;

/// The language of the own UI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ja,
    En,
}

thread_local! {
    /// The language of this page. `Ja` until `init` runs.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::Ja) };
}

/// Set the language for this page. Call it before anything is drawn.
pub fn init(lang: Lang) {
    CURRENT.set(lang);
}

pub fn current() -> Lang {
    CURRENT.get()
}

/// The language of a setting value: `ja`, `en`, or `auto` from the browser.
pub fn resolve(setting: &str, browser_language: Option<&str>) -> Lang {
    match setting {
        "ja" => Lang::Ja,
        "en" => Lang::En,
        // `auto`: the language of the browser. Japanese is the language of the service, so
        // anything that is not English gives Japanese.
        _ => match browser_language {
            Some(tag) if tag.to_ascii_lowercase().starts_with("en") => Lang::En,
            _ => Lang::Ja,
        },
    }
}

/// The word of `key` in the language of this page.
pub fn t(key: &str) -> &'static str {
    let lang = current();
    for (id, ja, en) in WORDS {
        if *id == key {
            return match lang {
                Lang::Ja => ja,
                // An empty English word means "not translated yet"
                Lang::En if !en.is_empty() => en,
                Lang::En => ja,
            };
        }
    }
    // The key itself, so a word that is missing from the table is visible
    Box::leak(key.to_string().into_boxed_str())
}

/// `(key, 日本語, English)`.
pub const WORDS: &[(&str, &str, &str)] = &[
    // --- The control bar of the float player ---
    ("bar.prev", "前話", "Prev"),
    ("bar.prev.title", "前の話へ", "Previous episode"),
    ("bar.play.title", "再生 / 一時停止", "Play / pause"),
    ("bar.next", "次話", "Next"),
    ("bar.next.title", "次の話へ", "Next episode"),
    ("bar.back30.title", "30 秒戻る", "Back 30 seconds"),
    ("bar.back10.title", "10 秒戻る", "Back 10 seconds"),
    ("bar.forward10.title", "10 秒進む", "Forward 10 seconds"),
    ("bar.forward30.title", "30 秒進む", "Forward 30 seconds"),
    ("bar.seek.title", "再生位置", "Position"),
    (
        "bar.speed.title",
        "再生速度を切り替える",
        "Change the play rate",
    ),
    ("bar.mute", "音", "Mute"),
    ("bar.mute.title", "ミュート", "Mute"),
    ("bar.volume.title", "音量", "Volume"),
    ("bar.danmaku", "弾幕", "Overlay"),
    (
        "bar.danmaku.title",
        "コメントの表示を切り替える",
        "Show or hide the comments over the video",
    ),
    ("bar.list", "一覧", "List"),
    (
        "bar.list.title",
        "コメント一覧の開閉",
        "Open or close the comment list",
    ),
    ("bar.native", "サイトUI", "Site UI"),
    (
        "bar.native.title",
        "サイト本来のコントロールを出す（画質などの設定はこちらから）",
        "Show the controls of the site (the picture quality is there)",
    ),
    ("bar.fullscreen", "最大化", "Fill"),
    (
        "bar.fullscreen.title",
        "ブラウザいっぱいに広げる（Esc で戻る）",
        "Fill the browser (Esc goes back)",
    ),
    // --- The comment column ---
    (
        "comments.searching",
        "コメントを検索中…",
        "Looking for the comments…",
    ),
    (
        "comments.loading",
        "指定された動画を読み込み中…",
        "Loading the video you gave…",
    ),
    (
        "comments.off",
        "コメント: 設定で無効",
        "Comments: off in the settings",
    ),
    (
        "comments.no_work",
        "コメント: 作品名が取れませんでした",
        "Comments: the work title is not readable",
    ),
    (
        "comments.not_found",
        "ニコニコに該当する公式配信が見つかりません。動画の URL を入れると、その動画のコメントを出します。",
        "No official video on nicovideo for this episode. Put the address of a video in the field to use that one.",
    ),
    (
        "comments.failed",
        "コメントを取得できませんでした",
        "The comments did not arrive",
    ),
    (
        "comments.no_reply",
        "コメントを取得できません（拡張の再読み込みが必要かもしれません）",
        "The comments did not arrive (the extension may need a reload)",
    ),
    ("pin.placeholder", "動画の URL", "Address of a video"),
    (
        "pin.title",
        "この話に使うニコニコ動画を指定します",
        "The video of nicovideo to use for this episode",
    ),
    ("pin.load", "読込", "Load"),
    ("pin.auto", "自動", "Auto"),
    (
        "pin.auto.title",
        "指定をやめて自動で探し直す",
        "Forget it and search again",
    ),
    (
        "pin.bad_url",
        "動画の URL を読み取れません（例: https://www.nicovideo.jp/watch/so46649112）",
        "That is not the address of a video (for example https://www.nicovideo.jp/watch/so46649112)",
    ),
    ("pin.mark", "指定", "given"),
    (
        "hist.title",
        "コメントの多いところ",
        "Where the comments are",
    ),
    ("offset.label", "コメント位置", "Comment offset"),
    (
        "offset.title",
        "既定は先頭揃え（0）。ニコニコ版の尺が長いぶんは末尾の広告なので、通常はずらさなくてよい",
        "The default is 0. The nicovideo version is longer at the end, so this rarely needs a change",
    ),
    (
        "offset.reset.title",
        "クリックで 0 に戻す",
        "Click to go back to 0",
    ),
    // --- The settings UI ---
    (
        "options.saved",
        "保存しました。開いているタブは再読み込みしてください。",
        "Saved. Reload the tabs that are open.",
    ),
    (
        "options.failed",
        "保存に失敗しました。",
        "The setting was not saved.",
    ),
    (
        "options.lead",
        "切り替えると即座に保存されます。すでに開いているタブには再読み込み後に反映されます。",
        "A change is saved at once. A tab that is already open needs a reload.",
    ),
    (
        "options.lead.compact",
        "切り替えは即座に保存されます。",
        "A change is saved at once.",
    ),
    ("options.nav.label", "設定の種類", "Groups of settings"),
    (
        "options.master",
        "この拡張を有効にする",
        "Turn this extension on",
    ),
    (
        "options.master.desc",
        "切ると、すべての表示改造を止めてサイト本来の見た目に戻します（Chrome の拡張機能そのものは切りません）。開いているタブはその場で戻り、こちらの UI を出し直すにはページの再読み込みが必要です。",
        "With this off, every change stops and the site looks as it always does (the extension itself stays on in Chrome). A tab that is open goes back at once; to get this UI again, reload the page.",
    ),
    ("popup.reload", "ページを再読み込み", "Reload the page"),
    (
        "popup.options",
        "説明つきで開く",
        "Open with the descriptions",
    ),
    (
        "popup.clear",
        "コメントの控えを消す",
        "Remove the comment cache",
    ),
    (
        "popup.cleared.none",
        "消すものがありませんでした。",
        "There was nothing to remove.",
    ),
    (
        "popup.cleared.failed",
        "控えを消せませんでした。",
        "The cache was not removed.",
    ),
    // --- The values of the lists (`CHOICES`) ---
    ("opt.auto", "自動", "Auto"),
    ("opt.ja", "日本語", "日本語"),
    ("opt.en", "English", "English"),
    ("opt.fps", "{n} fps", "{n} fps"),
    ("opt.sec.fast", "3 秒（速い）", "3 s (fast)"),
    ("opt.sec.4", "4 秒", "4 s"),
    ("opt.sec.6", "6 秒", "6 s"),
    ("opt.sec.slow", "8 秒（ゆっくり）", "8 s (slow)"),
    ("opt.sort.relevance", "関連度順", "By relevance"),
    ("opt.sort.plays", "再生数順", "By plays"),
    ("opt.sort.favorites", "気になる登録数順", "By favorites"),
    ("opt.sort.release", "リリース順", "By release"),
    ("opt.sort.year", "製作年度順", "By year"),
    ("opt.sort.kana", "５０音順", "By name"),
    (
        "opt.width.auto",
        "画面に合わせる（既定）",
        "Fit the screen (default)",
    ),
    (
        "opt.width.180",
        "180px（多く並べる）",
        "180px (many at once)",
    ),
    ("opt.width.220", "220px", "220px"),
    ("opt.width.260", "260px", "260px"),
    ("opt.width.320", "320px（大きく見る）", "320px (large)"),
    ("opt.thumb.640", "640×360（既定）", "640x360 (default)"),
    (
        "opt.thumb.off",
        "そのまま（通信量が最小）",
        "As it is (least traffic)",
    ),
    // --- The groups of the settings (`settings::GROUPS`) ---
    ("group.サイト全体", "サイト全体", "Whole site"),
    ("group.トップページ", "トップページ", "Top page"),
    ("group.一覧", "一覧", "Lists"),
    ("group.作品ページ", "作品ページ", "Work page"),
    ("group.検索", "検索", "Search"),
    ("group.再生", "再生", "Playback"),
    ("group.コメント", "コメント", "Comments"),
];

#[cfg(test)]
mod tests {
    use super::{Lang, WORDS, init, resolve, t};

    #[test]
    fn reads_the_language_of_the_setting_and_of_the_browser() {
        assert_eq!(resolve("ja", Some("en-US")), Lang::Ja);
        assert_eq!(resolve("en", Some("ja")), Lang::En);
        // `auto` follows the browser, and everything that is not English is Japanese
        assert_eq!(resolve("auto", Some("en-GB")), Lang::En);
        assert_eq!(resolve("auto", Some("ja-JP")), Lang::Ja);
        assert_eq!(resolve("auto", Some("de")), Lang::Ja);
        assert_eq!(resolve("auto", None), Lang::Ja);
    }

    #[test]
    fn gives_the_word_of_the_language() {
        init(Lang::Ja);
        assert_eq!(t("bar.prev"), "前話");
        init(Lang::En);
        assert_eq!(t("bar.prev"), "Prev");
        // A key that is not in the table is visible as itself
        assert_eq!(t("nothing.here"), "nothing.here");
        init(Lang::Ja);
    }

    #[test]
    fn every_key_appears_one_time() {
        let mut keys: Vec<&str> = WORDS.iter().map(|(key, _, _)| *key).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "a key is in the table twice");
    }
}
