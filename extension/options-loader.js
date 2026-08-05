/*
 * d-tweaks / options loader
 *
 * The CSP of an MV3 extension page is `script-src 'self'`, so an inline <script> in
 * options.html is blocked. This file starts the WASM instead. The logic is in
 * crates/options.
 *
 * A failure is also shown on the page: without it, no setting can be changed, and "load
 * failed" alone does not say what to correct.
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
  console.error(`[d-tweaks/options] wasm init failed: ${reason}`);
  const status = document.getElementById("status");
  if (status) {
    status.textContent = `設定 UI の読み込みに失敗しました（${reason}）`;
  }
});
