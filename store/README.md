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

Still missing: the settings page. `chrome-extension://` cannot be opened by the browser
automation, so that one capture has to come from the browser by hand
(`chrome-extension://<id>/options.html`).
