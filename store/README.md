# Store assets

Everything that the listing needs, and how it is made.

| Directory | What |
|---|---|
| `screenshots/` | The captures of the real pages (`raw-*.jpg`), as they came out of the browser |
| `promo/` | `promo.html` (one file for every image) and `build.sh` |
| `assets/` | What goes to the store. Made by `build.sh`; do not edit by hand |

```sh
store/promo/build.sh    # bakes assets/ with the Chrome of this machine
```

The store accepts a screenshot of 1280x800 (or 640x400), a small tile of 440x280 and a
marquee of 1400x560. `promo.html` draws one of them at a time (`?frame=`), so a change of
the words is a change of text and never of an image editor. The captures are inlined as
base64, so the file works from `file://` in headless Chrome.

The picture of a video can be black in a capture: Chrome blanks protected video while a
screen recording runs (see `player_modal`). That is not a defect.

## The settings page

`chrome-extension://` cannot be opened outside the browser, so that page is rendered from
the **real UI**: `settings-preview.html` loads the same WASM and CSS as the extension, and
`mock-chrome.js` answers the four `chrome` APIs that `crates/options` uses with the
defaults of a new installation. The picture is the real page, not a drawing of it.

It needs a local server (an ES module and a `.wasm` are loaded), which `build.sh` starts
and stops. `?group=` opens one card, so the picture is not the group with a single row.

`mock-chrome.js` and `settings-preview.html` are only for this build. They are not in the
extension and not in the archive (`just package` copies `extension/` only).
