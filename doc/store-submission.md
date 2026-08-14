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

## The upload runs in the workflow

`release.yml` sends the archive to the store after it made the tag and the GitHub release.
It **does not publish**: the listing (screenshots, category, the answers about the data) is
not in the API, so a human looks at the draft in the dashboard and presses the button.

Four secrets turn the step on. Without them the step does nothing, so a checkout without
them still releases to GitHub:

| Secret | Where it comes from |
|---|---|
| `CWS_EXTENSION_ID` | The 32 letters in the address of the item in the dashboard |
| `CWS_CLIENT_ID`, `CWS_CLIENT_SECRET` | An OAuth client of the type "Desktop app" in a Cloud project with the Chrome Web Store API on |
| `CWS_REFRESH_TOKEN` | `scripts/cws-refresh-token.py <the json of the client> --set-secrets` |

Two settings decide whether this works at all:

- The consent screen must be **In production**. In "Testing" a refresh token dies after
  seven days, and an account that is not a test user gets `access_denied`.
- The account must be the one that owns the item in the store. Another account can get a
  token and still not be allowed to upload.

### When the review says no

The item stays a draft, so the correction can go up under the **same version**:

1. Correct what the review named. If it is only the listing (a screenshot, a sentence), the
   dashboard is enough and nothing else is necessary.
2. If the archive must change, put the correction on main. The release workflow will not
   send it: the tag of that version exists, so it stops. Run **store-upload** from the
   Actions tab instead, with the tag to send (empty takes the newest release). It takes the
   archive of the GitHub release, so the bytes are the ones that were tested.

A version that is already **published** cannot go up a second time; the store refuses the
same version. Then the fourth part of the version is the way (`1.4.0.1`): write it into
`extension/manifest.json` and `Cargo.toml` by hand, on a branch, with a `[chore]` commit.
Nothing computes that number, because only a human knows that the same content must go out
one more time.

The first upload is by hand: the item does not exist yet, and the listing has to be filled
in once. After that every release goes through the workflow.

## Still by hand

- The listing: the screenshots and the promo images of `store/assets/<language>`, the
  category, and the answers about the data (read PRIVACY.md)
- The button that sends a draft to the review
