/*
 * d-tweaks / settings loader
 *
 * Starts the WASM of the settings UI. The options page and the popup both use this file:
 * `crates/options` draws the same rows into `#settings` on both, and `body.compact` makes
 * them smaller in the popup.
 *
 * The file exists because the CSP of an extension page is `script-src 'self'`, so a page
 * cannot have an inline script.
 *
 * A failure also appears on the page. Without that, no setting can be changed and the
 * reason is not visible.
 */
import init from "./pkg-options/options.js";

// The .wasm is read without the cache (the reason is in wasm-loader.js)
init({
  module_or_path: fetch(chrome.runtime.getURL("pkg-options/options_bg.wasm"), {
    cache: "no-store",
  }),
}).catch((error) => {
  const reason =
    error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  console.error(`[d-tweaks/settings] wasm init failed: ${reason}`);
  const status = document.getElementById("status");
  if (status) {
    status.textContent = `設定の読み込みに失敗しました（${reason}）`;
  }
});
