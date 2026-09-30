/*
 * d-tweaks / danmaku-worker
 *
 * Starts the WASM of the danmaku worker (crates/danmaku) and gives it every message.
 * All logic is in Rust.
 *
 * The content script starts this worker from a blob of the page origin, because a page
 * cannot start one from chrome-extension://. The blob imports this file, and the name
 * of the worker is the address of the extension.
 *
 * The first messages (the canvas and the comments) arrive before the WASM is ready, so
 * they wait in a queue.
 */

const base = self.name;
const queue = [];
self.onmessage = (event) => queue.push(event.data);

function fail(error) {
  const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  self.postMessage({ type: "error", message });
}

try {
  importScripts(`${base}pkg-danmaku/danmaku.js`);
  // Without the cache, for the same reason as wasm-loader.js: an old .wasm with a new
  // glue cannot resolve the hashed import names
  wasm_bindgen({
    module_or_path: fetch(`${base}pkg-danmaku/danmaku_bg.wasm`, { cache: "no-store" }),
  }).then(() => {
    self.onmessage = (event) => wasm_bindgen.on_message(event.data);
    for (const data of queue.splice(0)) {
      wasm_bindgen.on_message(data);
    }
  }, fail);
} catch (error) {
  fail(error);
}
