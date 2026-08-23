/*
 * d-tweaks / a `chrome` object for the preview of the settings page
 *
 * The settings UI is WASM, and it only runs on an extension page. The store image of that
 * page must show the real UI and not a drawing of it, so this file gives the APIs that
 * `crates/options` uses and nothing else. Every value is the default of a new
 * installation, so the picture is what a user sees after the install.
 *
 * `popup-preview.html` uses the same file for the list of new episodes. That list comes
 * from the service worker over `runtime.sendMessage`, so the answer of `dt/watch-list` is
 * here, with a few entries that show every state of a row.
 *
 * Only for `store/promo/build.sh`. It is not in the extension.
 */
/*
 * The reply of the service worker for the list of new episodes.
 *
 * `at` is relative to the moment of the capture, so the row says "5 分前" and not a date.
 * The set covers the three states that a row can have: unread, read, and the end of a
 * watch. `thumb` is the still that the interface gives with the episode, so a row of the
 * preview has a picture in the same place as a real one.
 */
const NOW = Date.now();
const MINUTE = 60 * 1000;
const NEWS = [
  {
    workId: "26609", workTitle: "葬送のフリーレン",
    partId: "26609007", number: "第7話", title: "おとぎ話のようなもの",
    thumb: "https://cs1.animestore.docomo.ne.jp/anime/2/66/09/8kZG9w/007/MVtRVg/26609007_1_2.png?1697698889857",
    at: NOW - 4 * MINUTE, seen: false, ended: false,
  },
  {
    workId: "25152", workTitle: "ワンピース ワノ国編",
    partId: "25152007", number: "#898", title: "真打ち！魔術師ホーキンス登場",
    thumb: "https://cs1.animestore.docomo.ne.jp/anime/2/51/52/1SDiew/007/TsaQVA/25152007_1_2.png?1637117189000",
    at: NOW - 3 * 60 * MINUTE, seen: false, ended: false,
  },
  {
    workId: "26610", workTitle: "薬屋のひとりごと",
    partId: "26610007", number: "第7話", title: "里帰り",
    thumb: "https://cs1.animestore.docomo.ne.jp/anime/2/66/10/nlUB1g/007/IClWxA/26610007_1_2.png?1700190004776",
    at: NOW - 26 * 60 * MINUTE, seen: true, ended: false,
  },
  {
    workId: "25767", workTitle: "機動戦士ガンダム 水星の魔女",
    partId: "", number: null, title: null, thumb: null,
    at: NOW - 3 * 24 * 60 * MINUTE, seen: true, ended: true,
  },
];

/*
 * The works that are watched. `nextAt` shows the back-off: a work that gives nothing is
 * asked less often, and the row of the popup says when the next request goes out.
 */
const HOUR = 60 * MINUTE;
const WORKS = [
  { workId: "26609", title: "葬送のフリーレン", tail: "26609028",
    checkedAt: NOW - 10 * MINUTE, nextAt: NOW + 5 * HOUR, misses: 0, lastNewAt: NOW - 4 * MINUTE },
  { workId: "25152", title: "ワンピース ワノ国編", tail: "25152197",
    checkedAt: NOW - 3 * HOUR, nextAt: NOW + 3 * HOUR, misses: 0, lastNewAt: NOW - 3 * HOUR },
  { workId: "26610", title: "薬屋のひとりごと", tail: "26610024",
    checkedAt: NOW - 6 * HOUR, nextAt: NOW + 18 * HOUR, misses: 2, lastNewAt: NOW - 26 * HOUR },
  { workId: "27565", title: "機動戦士ガンダムSEED FREEDOM", tail: "27565001",
    checkedAt: NOW - 20 * HOUR, nextAt: NOW + 24 * HOUR, misses: 6, lastNewAt: NOW - 22 * 24 * HOUR },
];

window.chrome = {
  runtime: {
    getURL: (path) => `../../extension/${path}`,
    getManifest: () => ({ version: window.__DT_VERSION__ || "0.0.0" }),
    openOptionsPage: () => {},
    // Only the messages that the popup sends
    sendMessage: (message) => {
      switch (message && message.type) {
        case "dt/watch-list":
          return Promise.resolve({ ok: true, news: NEWS, workList: WORKS, enabled: true });
        case "dt/watch-seen":
        case "dt/watch-clear":
        case "dt/watch-check":
        case "dt/watch-remove":
          return Promise.resolve({ ok: true });
        default:
          return Promise.resolve(undefined);
      }
    },
  },
  tabs: { reload: () => {}, create: () => {} },
  storage: {
    // The UI asks with the defaults as the argument and takes what comes back
    // MV3 gives a promise back, and that is the form the WASM awaits
    sync: {
      // `?lang=` forces the language of the UI, so both listings get their own picture
      get: (defaults) => {
        const want = new URLSearchParams(location.search).get("lang");
        return Promise.resolve(
          want ? { ...(defaults || {}), "ui-lang": want } : defaults || {},
        );
      },
      set: () => Promise.resolve(),
    },
    local: {
      get: () => Promise.resolve({}),
      set: () => Promise.resolve(),
      remove: () => Promise.resolve(),
    },
    onChanged: { addListener: () => {} },
  },
};
