# d-tweaks

[![build](https://github.com/shsw228/d-tweaks/actions/workflows/build.yml/badge.svg)](https://github.com/shsw228/d-tweaks/actions/workflows/build.yml)
[![version](https://img.shields.io/github/manifest-json/v/shsw228/d-tweaks?filename=extension%2Fmanifest.json&label=version)](extension/manifest.json)
[![manifest](https://img.shields.io/badge/manifest-v3-4285f4)](extension/manifest.json)
[![license](https://img.shields.io/github/license/shsw228/d-tweaks)](LICENSE)

A Chrome extension that rebuilds the PC web interface of dアニメストア, a Japanese
anime streaming service.

It reads the DOM of the site, draws its own UI for the lists, the top page and the work
pages, and plays an episode in a window on the same page instead of a navigation. The
logic is Rust compiled to WebAssembly.

The UI of the extension is in Japanese, because the service is.

## What it changes

| Page | Change |
|---|---|
| Top | 15 horizontal strips become one screen: a large showcase of the ranking, the episodes of today, and one grid with chips for the rest |
| Lists (continue, favorites, history, complete, all works) | Two fixed columns in 860px become a grid over the full width. The paging becomes an infinite scroll |
| Work page | The head becomes a full-width hero, the episodes become a grid of every episode, and the summary, the cast and the staff become tables |
| Search | The search link of the header opens a float search that does not leave the page. Results arrive while you type. Cmd-K, Ctrl-K and `/` also open it |
| Playback | An episode plays in a float window over the list. The control bar and the head bar are outside of the video, and the chapters give a "skip to the main story" button |
| Comments | The comments of the same episode come from the official channel on nicovideo and are drawn over the video and in a list |

Every feature has its own switch, and one more switch stops everything without a change
in `chrome://extensions`. With a feature off, the site looks as it always does.

## Requirements

- Chrome (Manifest V3)
- The PC web of the service (`https://animestore.docomo.ne.jp/animestore/`)
- The book and goods pages (`/book/`, `/ec/`) are out of scope

## Use

There is no package to install, so build it.

```sh
brew install rustup just binaryen wasm-pack
rustup default stable
rustup target add wasm32-unknown-unknown
# Put $(brew --prefix rustup)/bin in the PATH
```

```sh
just build
```

1. Open `chrome://extensions`.
2. Turn on the developer mode.
3. Select **the `extension/` directory** with "load unpacked".
4. Open the site.

After a build, reload the extension and then the page. Chrome reads the content script
one time, when it loads the extension, so a new build needs that reload.

### Settings

The toolbar icon opens a popup. The same rows are on the options page.

- **Master switch**: is the extension on?
- **Features**: one switch for each of the 11 features
- **Details**: remove the rentals from the search, the skip button, the keyboard
  shortcuts of the search
- **Lists**: the draw rate and the duration of the comments, the default sort of the
  search, the minimum width of a card, the resolution of the thumbnails

The popup also has "reload the page" and "remove the comment cache".

### Commands

```sh
just build   # release build (opt-level=z and wasm-opt -Oz)
just dev     # debug build
just test    # unit tests
just check   # clippy and fmt --check
just package # the archive for the store, in dist/
just clean
```

## How it works

```
crates/
  shared/       the settings tables and the bindings for the chrome API
  core/         content script: reads the DOM and draws the own UI
  background/   service worker: registers the CSS, asks nicovideo
  options/      the settings UI, for the options page and the popup
extension/      the directory that Chrome loads
  styles/       layout, applied at document_start
```

Three layers:

| Layer | What | When | Task |
|---|---|---|---|
| 0 | `extension/styles/*.css` | `document_start`, registered by the service worker | Layout. Only CSS can be ready before the first paint |
| 1 | `crates/core` (WASM) | Loaded at `document_start`, runs at `DOMContentLoaded` | Reads the DOM, draws the own UI |
| 2 | `crates/background` (WASM) | Always | Registers the CSS, talks to nicovideo |

The rules that the code follows:

- **Read the data and build own elements. Do not correct the DOM with CSS.** The site
  puts the same information in a different element on each page, and CSS needs one
  exception per page.
- **Fail safe.** The original DOM is hidden and never removed, and only after the own UI
  is ready. Without the WASM, the user sees the normal site.
- **Keep the traffic low.** Each position asks for the image size that it needs, a
  prefetch reads one half screen ahead, and an answer that arrives twice is kept. Nothing
  runs in the background.
- **Never touch the DRM data.** The playback interface returns `laUrl`, `contentUrls`,
  `oneTimeKey`, `viewOneTimeToken` and `castContentUri`. The code does not read them.

[doc/internals.md](doc/internals.md) has the decisions in the code and the reason for
each of them. [doc/known-issues.md](doc/known-issues.md) has the defects that are known
and not corrected.

## CI and releases

`.github/workflows/build.yml` runs `just check`, `just test` and `just package` on every
push and pull request, so a clean checkout must build. The archive stays as an artifact
of the run. The workflow installs a pinned `wasm-opt`, because the copy inside wasm-pack
can be older than the flags that the crates use.

Nobody types a version number. `.github/workflows/release-pr.yml` reads the commits since
the last tag with git-cliff, decides the next version, writes it into
`extension/manifest.json` (and into Cargo), writes `CHANGELOG.md`, and keeps a
**"Release x.y.z" pull request** open.

That pull request is the only decision: merge it and `.github/workflows/release.yml` sees
a version without a tag, builds, makes the tag, and publishes a GitHub release with the
archive. Leave it and the changes wait for the next one.

The trigger of the release is a push to main and not a tag, because a tag that a workflow
pushes with the default token does not start another workflow.

An upload to the store is not automatic.

The next version comes from the kinds of the commits:

| Part | When |
|---|---|
| major | A commit with `[feat!]` or with `BREAKING CHANGE` in its body |
| minor | A commit with `[feat]` |
| patch | Everything else |
| fourth part | Manual. The same content needs one more upload (an answer to a store review). |

A Chrome version is one to four integers (each 0 to 65535, no leading zero). A suffix
such as `-rc.1` is not valid. The store never accepts the same version twice.

## Limits

- The extension reads the HTML, the CSS and the interfaces of the site, so **a change of
  the site breaks it**.
- Comments exist only for the works and the episodes that are on the official channel.
  The match uses the work title, the season, the episode number and the length, and it
  **shows nothing when it is not certain**, so that the comments of another episode never
  appear.
- Playback on the same page needs `'self'` in the `frame-src` of the site, which this
  extension adds. That change is active **only while the float player is on** and goes
  away with the feature or with the master switch (`extension/rules.json`).

## Disclaimer

**This is a personal project. It has no relation to NTT DOCOMO, INC. or to nicovideo, and
neither of them endorses it.** All product names and service names belong to their
owners.

It has no feature that downloads or redistributes a video, and it never reads the DRM
data of the playback interface.

The site can change at any time, and then this extension stops working. Use it at your
own risk.

## License

[MIT](LICENSE)
