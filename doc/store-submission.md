# Store submission

What the Chrome Web Store form asks for, with the answer of this extension. Keep it
together with the code: the form asks the same questions at every update.

## Single purpose

To change how the PC web of dアニメストア is drawn: the lists, the top page, the work
pages, the search and the playback of one episode on the same page.

## Permissions

| Item | Why |
|---|---|
| `storage` | The settings and the comment cache. Nothing leaves the browser |
| `scripting` | The CSS of a feature is registered at `document_start` only while that feature is on. A static CSS entry would show the layout of a disabled feature until `chrome.storage` answers |
| `declarativeNetRequestWithHostAccess` | Two header rules, each one active only with its feature (see below) |
| `https://animestore.docomo.ne.jp/*` | The site that this extension changes |
| `https://*.nicovideo.jp/*` | The comments of the same episode. The host of the comment server comes from the reply (`nvComment.server`), so one fixed host is not enough |

## The two header rules

Both are declared as `"enabled": false` in the manifest, and the service worker enables
one only while its feature is on (`sync_rulesets`), so an installation alone changes no
header.

- `rules.json` adds `'self'` to the `frame-src` of the site, so the player page of the
  site can be in an iframe on the same page. CSP has no operation that weakens one
  directive, so the rule must `set` the complete header; the value is a copy of the CSP of
  the site with `'self'` added. Only the main frame of that site, and only while the float
  player is on.
- `rules-nico.json` puts the name of this extension into the `User-Agent` of **the request
  of the extension itself** to the search interface of nicovideo, which that interface
  requires. No request of the user is changed.

## Code readability

The logic is Rust compiled to WebAssembly, so a reviewer cannot read the binary. Nothing
is obfuscated and no code is loaded from the network. Give this with the submission:

- Source: https://github.com/shsw228/d-tweaks (the tag of the version)
- Build: `just build` (the commands are in the README)

## Data

Read [PRIVACY.md](../PRIVACY.md). Nothing is collected, nothing is sent, and the DRM
fields of the playback interface are never read.

## Still by hand

- Screenshots, 1280x800
- The icons must not come from the logo of the service or of NTT DOCOMO
