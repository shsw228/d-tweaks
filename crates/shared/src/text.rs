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

/// The word of `key` with `{name}` filled in.
///
/// A sentence with a number cannot be cut into pieces: the order of the pieces differs
/// between the two languages. So the whole sentence is one word with holes in it.
pub fn t_fill(key: &str, values: &[(&str, &str)]) -> String {
    let mut text = t(key).to_string();
    for (name, value) in values {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
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
    // --- The card of a list ---
    ("card.watched", "視聴済", "Watched"),
    // --- The head bar and the skip button of the float player ---
    ("meta.skip", "本編へスキップ ▶︎", "Skip to the main story ▶︎"),
    ("meta.next", "次の話へ ▶︎", "Next episode ▶︎"),
    ("meta.main", "本編", "Main story"),
    ("meta.resume", "前回", "Last time"),
    ("meta.latest", "最新話", "Latest"),
    // --- The float window ---
    (
        "modal.csp",
        "サイトの CSP（frame-src）によって、このページ内での再生がブロックされました。",
        "The CSP of the site (frame-src) blocked the playback on this page.",
    ),
    (
        "modal.csp.hint",
        "この機能を ON にすると frame-src を緩める規則が有効になります。切り替えた直後はページを再読み込みしてください。",
        "Turning this feature on enables the rule that opens frame-src. Reload the page after the change.",
    ),
    (
        "modal.open_tab",
        "このタブでプレイヤーを開く",
        "Open the player in this tab",
    ),
    (
        "modal.resizer.title",
        "ドラッグで幅を変更 / ダブルクリックで開閉",
        "Drag to change the width, double click to open or close",
    ),
    (
        "comments.setup_failed",
        "コメント: 準備に失敗しました",
        "Comments: the start failed",
    ),
    (
        "comments.error",
        "コメントを取得できません: {message}",
        "The comments did not arrive: {message}",
    ),
    ("comments.none", "コメントなし", "No comments"),
    (
        "comments.count",
        "{work} {label}／コメント {count} 件（{video}{mark}）",
        "{work} {label} — {count} comments ({video}{mark})",
    ),
    (
        "offset.step.title",
        "コメントを {step} 秒ずらす",
        "Move the comments by {step} seconds",
    ),
    // --- The infinite scroll ---
    (
        "scroll.first",
        "全 {total} ページ / 1 ページ目まで表示",
        "{total} pages, showing page 1",
    ),
    (
        "scroll.loading",
        "{page} ページ目を読み込み中…",
        "Loading page {page}…",
    ),
    ("scroll.all", "すべて表示しました", "Everything is shown"),
    (
        "scroll.progress",
        "{page} / {total} ページまで表示",
        "Showing page {page} of {total}",
    ),
    (
        "scroll.failed",
        "自動読み込みに失敗しました。ページ送りを使ってください。",
        "The next page did not arrive. Use the paging of the site.",
    ),
    // --- The float search ---
    (
        "search.hint",
        "作品名を入れると、打ちながら結果が出ます",
        "Type a title; the results arrive while you type",
    ),
    ("search.label", "作品を検索", "Search for a work"),
    ("search.placeholder", "作品名で検索", "Search by title"),
    ("search.sort.label", "並び替え", "Order"),
    ("search.no_rental", "レンタルを除く", "No rentals"),
    (
        "search.short",
        "もう少し入れてください（2 文字から）",
        "A little more, please (two letters or more)",
    ),
    ("search.running", "検索中…", "Searching…"),
    (
        "search.rental_hint",
        "（レンタルを除いています）",
        " (the rentals are left out)",
    ),
    (
        "search.empty",
        "「{word}」に一致する作品はありません{hint}",
        "Nothing matches \"{word}\"{hint}",
    ),
    ("search.failed", "検索できませんでした", "The search failed"),
    ("search.count", "{total} 件", "{total} works"),
    (
        "search.count.loaded",
        "{total} 件中 {loaded} 件",
        "{loaded} of {total} works",
    ),
    ("search.more", "読み込み中…", "Loading…"),
    (
        "search.more.failed",
        "続きを読み込めませんでした",
        "The next page did not arrive",
    ),
    ("badge.favorite", "気になる", "Favorite"),
    ("badge.watching", "視聴中", "Watching"),
    ("badge.rental", "レンタル", "Rental"),
    // --- The own top page ---
    ("top.all", "すべて見る", "See all"),
    ("top.watch", "この作品を見る", "Watch this work"),
    (
        "top.work.label",
        "この作品のページへ",
        "To the page of this work",
    ),
    (
        "top.more",
        "もっと見る（あと {count} 件）",
        "More ({count} left)",
    ),
    ("top.browse", "さがす", "Find"),
    (
        "top.list.all",
        "この一覧をすべて見る",
        "See all of this list",
    ),
    ("top.ranking", "デイリーランキング", "Daily ranking"),
    ("top.rank", "{label} {rank}位", "{label} no. {rank}"),
    ("top.other", "おすすめ", "Recommended"),
    // --- The work page ---
    ("work.summary", "あらすじ", "Summary"),
    ("work.genre", "ジャンル", "Genres"),
    ("work.other", "その他", "Other"),
    ("work.cast", "キャスト", "Cast"),
    ("work.staff", "スタッフ", "Staff"),
    ("work.year", "製作年", "Year"),
    ("work.mylist", "マイリスト", "My list"),
    ("work.favorite", "気になる", "Favorite"),
    // --- The debug view ---
    ("debug.fps", "配信フレームレート", "Source frame rate"),
    ("debug.frame", "コマ番号", "Frame"),
    ("debug.size", "解像度", "Resolution"),
    ("debug.time", "再生位置", "Position"),
    ("debug.buffer", "先読み", "Buffered"),
    ("debug.state", "再生状態", "State"),
    ("debug.ready", "読み込み", "Ready"),
    ("debug.frames", "フレーム数", "Frames"),
    ("debug.decode", "復号時間", "Decode"),
    ("debug.draw", "弾幕描画", "Overlay"),
    ("debug.comments", "コメント", "Comments"),
    ("debug.length", "尺差", "Length"),
    ("debug.canvas", "描画面", "Canvas"),
    ("debug.source", "取得元", "From"),
    ("debug.video", "動画", "Video"),
    ("debug.measuring", "測定中", "Measuring"),
    ("debug.seconds", "{n} 秒", "{n} s"),
    ("debug.ready.0", "未取得", "Nothing"),
    ("debug.ready.1", "メタデータのみ", "Metadata only"),
    ("debug.ready.2", "現在位置のみ", "Current position only"),
    ("debug.ready.3", "先まで再生可", "Can play ahead"),
    ("debug.ready.4", "十分", "Enough"),
    ("debug.net.0", "空", "Empty"),
    ("debug.net.1", "待機", "Idle"),
    ("debug.net.2", "読み込み中", "Loading"),
    ("debug.net.3", "ソースなし", "No source"),
    ("debug.unknown", "不明", "Unknown"),
    ("debug.playing", "再生中", "Playing"),
    ("debug.paused", "停止中", "Paused"),
    (
        "debug.rate",
        "{state} / 速度 {rate}x",
        "{state} / rate {rate}x",
    ),
    (
        "debug.frames.value",
        "復号 {total} / 落ち {dropped} / 破損 {corrupted}",
        "decoded {total} / dropped {dropped} / corrupt {corrupted}",
    ),
    (
        "debug.presented",
        "表示 {presented} / {quality}",
        "presented {presented} / {quality}",
    ),
    (
        "debug.draw.value",
        "設定 {set} fps / 実測 {measured}",
        "set {set} fps / measured {measured}",
    ),
    (
        "debug.time.value",
        "{current} / {duration} 秒（残り {remaining}）",
        "{current} / {duration} s (left {remaining})",
    ),
    (
        "debug.length.value",
        "{diff}s（ニコ {nico} / 配信 {site}）",
        "{diff}s (nicovideo {nico} / site {site})",
    ),
    (
        "debug.comments.value",
        "取得 {got} / 対象 {target} / 表示中 {now}",
        "got {got} / drawn {target} / on screen {now}",
    ),
    (
        "debug.canvas.value",
        "{lanes} 段 / {w}×{h} CSS px @{ratio}x",
        "{lanes} lanes / {w}x{h} CSS px @{ratio}x",
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
