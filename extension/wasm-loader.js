/*
 * d-tweaks / wasm-loader
 *
 * WASM cannot start itself: the generated code of wasm-bindgen (the global
 * `wasm_bindgen` of pkg/core.js) must be called one time. That is why this file is
 * JavaScript. All logic is in Rust (crates/core).
 *
 * A content script reads the .wasm through a chrome-extension:// URL, so it is in
 * web_accessible_resources of the manifest.
 *
 * On a failure, say what happened and what to do. There are two causes, and a reload of
 * the page corrects both, but the raw exception does not separate them:
 *
 *   1. LinkError (an import is not callable): this core.js and this core_bg.wasm are from
 *      two builds. The import names have a hash in them, so one new file cannot resolve.
 *      This happens when a page loads during a build (the justfile also renames the
 *      files, which makes that window small).
 *
 *   2. The extension was reloaded: the content script of an open tab continues, but the
 *      extension that it came from is replaced, so the .wasm is not readable.
 *      `chrome.runtime.id` is then undefined.
 */

function describe(error) {
  return error instanceof Error ? `${error.name}: ${error.message}` : String(error);
}

async function boot() {
  try {
    /*
     * Read the .wasm without the cache.
     *
     * core.js is a content script and is read again after every build, but the .wasm is a
     * fetch of chrome-extension:// and the browser keeps it. An old .wasm with a new
     * core.js cannot resolve the hashed import names. There are two forms of the same
     * cause:
     *
     *   LinkError: ... "__wbg_set_onload_...": function import requires a callable
     *   TypeError: wasm.__wasm_bindgen_func_elem_1525 is not a function
     *
     * The second form appears when a closure is called and not at the start, so the
     * failure looks like "only some features do not work".
     */
    await wasm_bindgen({
      module_or_path: fetch(chrome.runtime.getURL("pkg/core_bg.wasm"), {
        cache: "no-store",
      }),
    });
  } catch (error) {
    // After a reload of the extension, getURL works but the fetch fails
    if (!chrome.runtime?.id) {
      console.warn(
        "[d-tweaks] 拡張が再読み込みされたため、このページでは動きません。" +
          "ページを再読み込みしてください。",
      );
      return;
    }
    // The hashed import names do not agree: the glue and the .wasm are from two builds
    if (error instanceof WebAssembly.LinkError) {
      console.warn(
        "[d-tweaks] core.js と core_bg.wasm が別ビルドです" +
          "（ビルド中にページを読み込むと起きます）。ページを再読み込みしてください。\n" +
          describe(error),
      );
      return;
    }
    console.error(
      `[d-tweaks] wasm init failed: ${describe(error)}` +
        "（拡張は有効なので、extension/pkg が古くないか just build で確認）",
    );
  }
}

boot();
