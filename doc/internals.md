# Implementation notes

This document records the decisions in the code and the reason for each of them.
It uses Simplified Technical English (ASD-STE100).

It does not describe the site. The selectors and the addresses that the extension uses
are in the code, and the site can change them at any time.

For installation and settings, read the [README](../README.md). For the defects
that are known but not corrected, read [known-issues.md](known-issues.md).

## Layers

The extension has three layers.

| Layer | Code | Time | Task |
|---|---|---|---|
| 0 | `extension/styles/*.css` | `document_start` | Layout |
| 1 | `crates/core` (WASM) | `document_start`, then run at `DOMContentLoaded` | Read the DOM and draw the own UI |
| 2 | `crates/background` (WASM) | Always | Register the CSS. Get the comments |

Put the layout in layer 0. Only CSS can be ready before the first paint.

### Crates

| Crate | Task |
|---|---|
| `shared` | Settings tables. Bindings for the `chrome` API |
| `core` | Content script |
| `background` | Service worker |
| `options` | Settings UI for the options page and the toolbar popup |

### Hand-written JavaScript

WASM cannot start itself. Four files are JavaScript. All other logic is Rust.

| File | Task |
|---|---|
| `wasm-loader.js` | Start the WASM of the content script |
| `sw.js` | Add the service worker listeners |
| `options-loader.js` | Start the WASM of the options page |
| `popup-loader.js` | Start the WASM of the popup |

The service worker must add the listeners in a synchronous step. If the code
waits for the WASM, the worker loses the `onInstalled` event.

The CSP of the extension pages is `script-src 'self'`. An inline `<script>` does
not run. Put the start code in a file.

## Rules

1. **Read the data. Do not correct the DOM with CSS.** The site puts the work
   title in a different element on each page. A CSS-only solution needs one
   exception for each page.
2. **Fail safe.** Hide the original DOM only after the own UI is ready. If the
   WASM does not run, the user sees the normal site.
3. **Keep the traffic low.** Select the image size for each position. Read ahead
   for one half screen. Keep the last answer. Nothing runs in the background.
4. **Do not touch the DRM data.** Do not read `laUrl`, `contentUrls`,
   `oneTimeKey`, `viewOneTimeToken` or `castContentUri`.
5. **Do not open a new tab.** The cards use a normal `<a href>`.
6. **Change a header only for a feature that is on.** A header rule of
   `declarativeNetRequest` is declared as disabled, and the service worker enables
   it with its feature.

## Registration of the CSS

`manifest.json` has no `css` entry. The service worker registers the CSS of the
enabled features with `chrome.scripting.registerContentScripts`, at
`document_start`.

`chrome.storage` is asynchronous. A static CSS entry shows the layout of a
disabled feature for a short time. A dynamic registration prevents this effect.

The service worker registers again after `onInstalled`, `onStartup` and
`storage.onChanged`.

### Synchronous test for the master switch

The content script must know the master switch before the first paint. The
service worker registers `styles/enabled.css` only when the switch is on. That
file sets `--dt-enabled`. The content script reads the property with
`getComputedStyle`. This test is synchronous.

If the property is absent, the content script reads `chrome.storage` and then
starts.

### Header rules follow the feature

Two static rulesets of `declarativeNetRequest` change a header of a request:

| Ruleset | File | Change | Feature |
|---|---|---|---|
| `csp` | `extension/rules.json` | Adds `'self'` to the `frame-src` of the site | `player-modal` |
| `nico-ua` | `extension/rules-nico.json` | Puts the name of the extension in the `User-Agent` of the search request, which the interface of nicovideo requires | `comments` |

Both are declared as `"enabled": false`. The service worker enables one only while
its feature is on (`sync_rulesets`), so an installation alone changes no header,
and the master switch stops both.

Chrome keeps the enabled state across sessions but **not across an update of the
extension**: an update uses the state of the manifest again. `onInstalled` also
arrives on an update, and it runs the same synchronisation, so the state returns.

The `csp` rule replaces the complete header, because CSP has no operation that
weakens one directive. So the value in `rules.json` is a copy of the CSP of the
site with `'self'` added. **Read the CSP of the site again after a change of the
site.** If the site adds a directive, this copy removes it again.

## Timing

The content script loads at `document_start` and runs at `DOMContentLoaded`.

`document_idle` runs after `load`, and the first paint of the site comes before that,
so the original page was visible for a moment. The WASM itself needs only a few
milliseconds, so an early load and a wait for the DOM is faster than a late load.

At `document_start` the `document.body` is absent, so `start` waits for the
`DOMContentLoaded` event.

## Images

The site keeps the same image in more than one size; the size is the last number of
the file name. `card_view::resize_thumb` changes that number.

Two rules:

- **Select the size for the position.** A list card, a card of the top page and the
  hero of a work page need different sizes. A large image everywhere multiplies the
  traffic of a page with tens of cards. The user can also select "no change".
- **Never change the `src` of an image that is already on the screen.** These images
  have no `Cache-Control`, so the browser asks again (measured). The showcase of the
  top page makes one `<img>` for each item, keeps it, and changes only the opacity.

## Site structure

The parse layer holds every assumption about the DOM of the site. Two of those
assumptions are also in a CSS file, and they must not drift:

- The list container. `list-grid.css` and `LIST_SELECTOR` in `crates/core/src/lib.rs`
  must use the same condition. A different condition gives a grid of original cards.
- The marks that hide the original DOM (`dt-rendered`, `dt-top-rendered`,
  `dt-hero-rendered`, `dt-episodes-rendered`). The CSS hides an element only when the
  own UI carries the mark.

The site is not an SPA, but some pages insert their cards with JavaScript after the
first paint. A `MutationObserver` (lists) and a poll (top page) catch those.

The page kind comes from the path (`crates/core/src/page.rs`). The site also puts a
class on `<html>`, but its JavaScript adds that later, so layer 0 cannot use it.

## Comments

The site page cannot get data from `nicovideo.jp`. The browser stops the request
(CORS). The content script sends the work title, the episode number, the episode
title and the length to the service worker. The service worker gets the
comments.

### Select a video only with certainty

The service worker searches nicovideo for the work title and the episode, and
`matching::pick` accepts a candidate only when every one of these is true:

1. The video belongs to a channel. A video of a user is never a candidate.
2. The title has the work title.
3. The title has the season, if the work title has one.
4. The title has the episode number.
5. The length is 0.7 to 1.5 times the length of the episode.

Without a candidate, the extension shows no comments. **A wrong episode is worse than
no comments.**

Two details that cost a defect each:

- Compare the titles without the brackets. The two sites write the same title with
  different brackets, and a plain text match then fails although the titles agree.
- Compare the length as a ratio, not as a difference in seconds. The upload of a
  channel can be some seconds longer, and a preview is much shorter.

### Do not use a bare number as a text match

An episode number can arrive as a bare `6`. The text `6` is also inside `第16話` and
inside `シーズン6`, so a text match gives another episode. Give the number the form
`第6話` first, and compare only the numbers of a title that are written as an episode
number (`第N話`, `第N回`, `#N`, `Episode N`, `N話`). An earlier version compared every
number of the title and matched a season.

### Remove a "not found" on an update

The map keeps a "not found" for one day, so a work that nicovideo does not have gives
one search and not one per open.

`onInstalled` removes those entries. An update often changes the match, and a "not
found" of the old logic hides the correction for a day. This happened in a real
session: an episode said "not found" while the interface returned the correct video.

### Give the cache a version

`storage.local` keeps the map from the episode to the video for 30 days. It
keeps a "not found" result for one day.

**Change the version in the key when you change the match logic.** If you forget
this step, the old result stays and the correction has no effect.

Two examples from the tests:

- An episode that is not on the channel: an old version selected episode 1 of the
  same work, because that video has the most comments. The wrong map stayed for
  30 days.
- An episode that was correct, but a "not found" result of an earlier version
  stayed for one day.

The popup has a button to remove the cache.

### Keep the index of the cache in the memory

The comment blobs have an index that gives the sequence of the writes. The old
code read the index from the storage, changed it, and wrote it back. Each of
these steps is asynchronous. If two tabs ask for comments at the same time, one
of the two changes is lost. The result is a blob that no index gives, and that
blob stays until the storage is full.

The code now keeps a copy of the index in the memory of the service worker:

- `load_index` reads the storage one time only.
- `edit_index` changes the copy in a block that has **no `await`**. This block
  cannot be interrupted, so no change is lost.
- `save_index` writes the copy, not a local list.

The options page can remove the cache. That page writes to the storage directly,
so the service worker must forget the copy. `sw.js` sends
`on_comment_cache_cleared` when the index key is removed.

## Traps

These problems are difficult to find. Each one cost time.

### `dyn_into` fails for objects of an iframe

The float player uses an iframe of the same origin. The `<video>` element and
the `requestVideoFrameCallback` function belong to the realm of the iframe.
`instanceof` is false, so `dyn_into` fails.

Use `unchecked_into` for the element. Use `is_function` for the function.
`is_function` uses `typeof`, so the realm has no effect.

### The site CSS wins for links

The site has this rule:

```css
a:link, a:active, a:visited { color: #333 }
```

The specificity is (0,1,1). A selector like `.dt-head__work` has (0,1,0) and
loses. The text becomes dark grey on a black background.

Add `a.` or a parent class to the selector. Selectors with the
`html:not(.dt-off)` prefix have (0,2,1) and win.

### One own file can win against another own file

`list-grid.css` has `html:not(.dt-off) .dt-card { position: absolute }` for the
list. A selector `.dt-search__grid > .dt-card` has a lower specificity. The
cards keep the absolute position and the grid gets a height of 0.

Give the own container rules the `html:not(.dt-off)` prefix.

### A modal dialog makes the page inert

The site opens an advertisement with `<dialog class="popup-wrapper" open>`. A
modal dialog makes the other elements inert.

| State | Result of `elementFromPoint` |
|---|---|
| Open | `div.popup-overlay` |
| `display: none` | `html`. The elements below do not get the click |
| `close()` | The correct element |

Use `close()`. The CSS only prevents the advertisement for the first moment.

### The site loses the last position

The player of the site sends the last position with `sendBeacon` in the
`beforeunload` event. The browser does not send `beforeunload` when the code
removes the iframe.

Send the iframe to `about:blank`, wait 300 ms and then remove the iframe. A test
showed a `resumePoint` of 0 after 744 seconds of playback with the direct
removal.

### `document.styleSheets` has no content script CSS

The list of `document.styleSheets` does not include the CSS of a content script.
A test with this list gives a false negative.

### A custom property is absent when the feature is off

Each CSS file registers only with its feature. A custom property from one file
is absent when the user disables that feature. Always give a fallback:
`var(--dt-page-pad, clamp(16px, 2.5vw, 64px))`.

### Do not add the height of a bar twice

A first version put the film strip with `position: absolute` and moved the text
with `bottom: calc(var(--dt-rail-height) + 20px)`. The two values use `clamp`
and do not stay together. The text went behind the strip.

Use a flex column with `justify-content: flex-end`. The order of the elements
gives the position.

### Do not put a shared number in a mutable global

The danmaku code has one number that the collision test, the lane assignment and
the drawing must all use: the seconds for one comment to cross the screen. An
earlier version kept it in a mutable `thread_local` and `start` wrote to it
before the lane assignment. Nothing in the types shows this sequence. If a
function reads the value before `start` writes it, the comments overlap.

The functions now take the value as a parameter. The tests give the value
directly, so a test can use a different value without a global.

Use the same rule for the state of the float search. The four values that
describe the progress (generation, count, total, busy) are one `Copy` structure
in one `Cell`. Separate cells make it possible to increase the generation and to
forget to clear the count.

### Give one function to the caller, not two

`card_view` had `set_thumb_size` and `refresh_thumbs`. A caller that used only
the first one put a new setting in the memory but left the old images on the
page. The two functions are now one function, `apply_thumb_size`.

### `wasm-pack` writes two files

`wasm-pack` writes the `.js` file and the `.wasm` file one after the other. A
page load between the two writes gets a mixed pair. The import names have a
hash, so the pair does not link:

```text
LinkError: ... "__wbg_set_onload_...": function import requires a callable
```

The `justfile` builds in a temporary directory and renames the directory. The
loaders get the `.wasm` file with `cache: "no-store"`.

A build gives the same error a second way, and this one is normal: Chrome reads the
content script (`pkg/core.js`) one time, when it loads the unpacked extension, but
the loader gets the `.wasm` file at every page load. After a build, the old glue and
the new binary meet, and the import of the new binding does not resolve. **Reload the
extension after a build.** The message of the loader names this cause.

## Kill switch

All CSS rules start with `html:not(.dt-off)`. Each file also has a rule to hide
the own elements:

```css
html.dt-off .dt-card { display: none !important }
```

Without this rule, the own elements stay without a style. The result looks worse
than the normal site.

The content script adds `dt-off` when the master switch is off. The content
script also listens to `chrome.storage.onChanged`, so an open tab returns to the
normal site immediately.

## Packaging

`just package` makes `dist/d-tweaks-<version>.zip` for the Chrome Web Store.

Three kinds of file must not be in that archive:

- `_metadata/`: Chrome makes it when it loads the unpacked extension. A name that
  starts with `_` is reserved, and the upload fails with it.
- `__MACOSX/` and `.DS_Store`: macOS puts them into an archive.
- `*.d.ts`: the type declarations of wasm-pack. The extension does not need them.

The WASM is compiled, so a reviewer cannot read it. Give the address of the source
and the build command (`just build`) with the submission.

## Tests

The tests use the real values from the site. Examples:

- The thumbnail size numbers
- The name patterns of the official channel (with and without brackets, with a
  season, with a kanji episode number)
- The chapter lists with `avant` and with a 3-second part
- The broken episode label `...sode 12` from the site

Run `just test`. The tests do not need a browser.
