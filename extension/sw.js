/*
 * d-tweaks / service worker
 *
 * An MV3 service worker must add its event listeners synchronously. The WASM init is
 * asynchronous, so a wait for it before the listeners would lose onInstalled. This file
 * only:
 *
 *   1. starts the init (without a wait),
 *   2. adds the listeners synchronously,
 *   3. waits for the init inside a listener and calls the WASM.
 *
 * The logic is in crates/background.
 */
import init, {
  on_comment_cache_cleared,
  on_installed,
  on_message,
  on_settings_changed,
  on_startup,
} from "./pkg-background/background.js";

// Key of the index of the comment cache (keep it the same as in cache_keys.rs)
const COMMENT_INDEX_KEY = "dt:cmtidx";

/*
 * Read the .wasm without the cache. The glue (background.js) is read again after every
 * build, but the browser can keep the .wasm, and two builds together cannot resolve the
 * hashed import names (see wasm-loader.js).
 */
const ready = init({
  module_or_path: fetch(chrome.runtime.getURL("pkg-background/background_bg.wasm"), {
    cache: "no-store",
  }),
});

chrome.runtime.onInstalled.addListener(() => {
  ready.then(on_installed);
});

chrome.runtime.onStartup.addListener(() => {
  ready.then(on_startup);
});

chrome.storage.onChanged.addListener((changes, areaName) => {
  if (areaName === "sync") {
    ready.then(on_settings_changed);
    return;
  }
  /*
   * The "remove the comment cache" button of the settings page writes to storage.local
   * directly, so the copy of the index in the service worker no longer agrees with it.
   * Only a remove of the index key discards that copy; a write of the service worker has
   * a newValue and does not pass this test.
   */
  if (
    areaName === "local" &&
    COMMENT_INDEX_KEY in changes &&
    changes[COMMENT_INDEX_KEY].newValue === undefined
  ) {
    ready.then(on_comment_cache_cleared);
  }
});

/*
 * A request for comments from a content script.
 *
 * The reply is asynchronous, so return true to keep the channel open. A throw here would
 * leave the sender without a reply, so every path calls sendResponse.
 */
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  ready
    .then(() => on_message(message, sender))
    .then(sendResponse)
    .catch((error) => sendResponse({ ok: false, error: String(error) }));
  return true;
});
