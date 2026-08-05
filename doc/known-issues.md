# Known issues

This document records the defects and the limits that are known but not
corrected. It uses Simplified Technical English (ASD-STE100).

Make an issue in the repository for each item when the repository is available.
Remove the item from this document at that time.

For the design decisions, read the [implementation notes](internals.md).

## 1. A work without an episode number can be lost

**Where:** `crates/background/src/niconico.rs`, `crates/background/src/matching.rs`

The search interface of nicovideo sends a maximum of 100 items for one request.
The code puts the episode number in the search words to make the result small
(read "Put the episode number in the search words" in the notes).

This method does not help a work that has no episode number, for example a film
or a single program. If more than 100 videos agree with the title, the correct
video can be outside of the window. The user sees no comments.

**Possible correction:** find if the search interface accepts a filter on the
channel (`filters[channelId]`). The official videos are on a channel, and a
filter on the channel makes the result much smaller. This is not tested.

**Effect if not corrected:** no comments for some works. The code shows no wrong
comments, because the match is strict.

## 2. The index of the comment cache is only safe in one service worker

**Where:** `crates/background/src/cache.rs`

The code keeps a copy of the index in the memory of the service worker and
changes the copy only in blocks that have no `await`. This makes two requests in
the same service worker safe.

Chrome runs one service worker for one extension, so this is sufficient today.
If Chrome runs more than one instance, or if another page writes the same key,
the copy and the storage can disagree. The result is a wrong count of the
entries, and the code can remove an entry too early.

**Effect if not corrected:** the cache holds less than 20 videos. The comments
are correct.

## 3. The `User-Agent` rule is not measured

**Where:** `extension/rules-nico.json`, `crates/background/src/niconico.rs`

The search interface of nicovideo requires the name of the application in the
`User-Agent`. A `fetch` cannot set that header (the browser forbids it), so a
`declarativeNetRequest` rule sets it for the one endpoint that this extension asks.

Three facts say that this works, but none of them is a measurement of the header:

- `user-agent` is in the list of headers that a rule may change.
- Chrome 111 and later apply the rules of one extension to the background requests
  of another extension, so a request of an extension does reach the rule matcher.
- The access test of Chromium accepts a URL whose origin is the origin of the
  extension itself, so the initiator of an own request should pass.

The rule cannot break the search: the same request with the `User-Agent` of the rule
and with the `User-Agent` of Chrome gives the same two candidates (measured with
curl).

**How to measure:** open the service worker in DevTools, ask for the comments of one
episode, and read the request headers of the search request in the Network panel. If
the header is the one of Chrome, the rule does nothing, and the `_context` parameter
stays the only identification.

## 4. The CSP rule replaces the complete header

**Where:** `extension/rules.json`

CSP has no operation that weakens one directive, so the rule must `set` the complete
header. The value is a copy of the CSP of the site with `'self'` in `frame-src`.

If the site adds a directive to its CSP, this copy removes it again for the users of
this extension. The rule is only active while the float player is on, which makes the
time small, but it does not remove the problem.

**Possible correction:** read the CSP of the answer and add `'self'` to the
`frame-src` of that value. `declarativeNetRequest` cannot do this (a rule is
static), so it needs another mechanism.

## 5. `let _ =` on the DOM operations

**Where:** all of `crates/core`

The code has about 60 places with `let _ =`. Almost all of them are operations
that only change the appearance: `set_attribute`, `class_list`, `set_property`,
`play` and `pause`.

This is a decision, not an omission. These operations fail only if the element
is not in the document, and then the correct action is to do nothing. A log for
each of them gives no information and makes noise.

The rule is:

- An operation that changes the appearance can use `let _ =`.
- An operation that gets data, that writes to the storage, or that draws the own
  UI must use `?` or must write to the log.

Make this rule automatic if a tool becomes available. `clippy` has no lint for
this difference today.
