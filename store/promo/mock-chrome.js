/*
 * d-tweaks / a `chrome` object for the preview of the settings page
 *
 * The settings UI is WASM, and it only runs on an extension page. The store image of that
 * page must show the real UI and not a drawing of it, so this file gives the four APIs
 * that `crates/options` uses and nothing else. Every value is the default of a new
 * installation, so the picture is what a user sees after the install.
 *
 * Only for `store/promo/build.sh`. It is not in the extension.
 */
window.chrome = {
  runtime: {
    getURL: (path) => `../../extension/${path}`,
    getManifest: () => ({ version: window.__DT_VERSION__ || "0.0.0" }),
    openOptionsPage: () => {},
  },
  tabs: { reload: () => {} },
  storage: {
    // The UI asks with the defaults as the argument and takes what comes back
    // MV3 gives a promise back, and that is the form the WASM awaits
    sync: {
      get: (defaults) => Promise.resolve(defaults || {}),
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
