/*
 * d-tweaks / popup loader
 *
 * The popup of the toolbar icon uses the same WASM as the options page: `crates/options`
 * draws the same rows into `#settings`, and `body.compact` makes them smaller. So there
 * is no crate of its own; only the file that loads it differs.
 *
 * This file is separate from options-loader.js because the CSP of an extension page
 * (`script-src 'self'`) has no inline script, so every page needs a real file.
 */
import init from "./pkg-options/options.js";

// Read the .wasm without the cache (see wasm-loader.js)
init({
  module_or_path: fetch(chrome.runtime.getURL("pkg-options/options_bg.wasm"), {
    cache: "no-store",
  }),
}).catch((error) => {
  const reason =
    error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  console.error(`[d-tweaks/popup] wasm init failed: ${reason}`);
  const status = document.getElementById("status");
  if (status) {
    status.textContent = `設定の読み込みに失敗しました（${reason}）`;
  }
});
