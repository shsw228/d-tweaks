# Store assets

Everything that the listing needs, and how it is made.

| Directory | What |
|---|---|
| `screenshots/` | The captures of the real pages (`raw-*.jpg`), as they came out of the browser |
| `promo/` | `promo.html` (one file for every image) and `build.sh` |
| `listing/` | The words of the listing: the description in both languages and the answers of the submission form, ready to paste |
| `assets/ja`, `assets/en` | What goes to the store, one set per language. Made by `build.sh`; do not edit by hand |

```sh
store/promo/build.sh    # bakes assets/ with the Chrome of this machine
```

The store keeps one listing per language (title, description, screenshots, promo images), so
every image exists in both. The captures are the same in both: the site is Japanese. The
words come from `FRAMES` and `BRAND` in `make.py`.

The store accepts a screenshot of 1280x800 (or 640x400), a small tile of 440x280 and a
marquee of 1400x560. `promo.html` draws one of them at a time (`?frame=`), so a change of
the words is a change of text and never of an image editor. The captures are inlined as
base64, so the file works from `file://` in headless Chrome.

The picture of a video can be black in a capture: Chrome blanks protected video while a
screen recording runs (see `player_modal`). That is not a defect.

## The icon of the listing

The dashboard asks for the icon of the store as its own file, so `assets/store-icon-128.png`
is that file: the picture of `extension/icons/icon128.png` at 96px in a 128px frame, with
16px of transparent margin around it. That margin is what the guidelines of the store ask
for, and it makes the icon the same size as the others in a list.

`assets/store-icon-128-full.png` is the same picture without the margin (what the toolbar
uses). Take that one if the frame should be filled.

Both come from `extension/icons/icon128.png`, so a new icon means making them again:

```sh
uv run --with pillow python3 -c "
from PIL import Image
src = Image.open('extension/icons/icon128.png').convert('RGBA')
art = src.resize((96, 96), Image.LANCZOS)
canvas = Image.new('RGBA', (128, 128), (0, 0, 0, 0))
canvas.paste(art, (16, 16), art)
canvas.save('store/assets/store-icon-128.png')
src.save('store/assets/store-icon-128-full.png')
"
```

## The settings page

`chrome-extension://` cannot be opened outside the browser, so that page is rendered from
the **real UI**: `settings-preview.html` loads the same WASM and CSS as the extension, and
`mock-chrome.js` answers the four `chrome` APIs that `crates/options` uses with the
defaults of a new installation. The picture is the real page, not a drawing of it.

It needs a local server (an ES module and a `.wasm` are loaded), which `build.sh` starts
and stops. `?group=` opens one card, so the picture is not the group with a single row.

`mock-chrome.js` and `settings-preview.html` are only for this build. They are not in the
extension and not in the archive (`just package` copies `extension/` only).
