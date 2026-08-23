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

**Way out for the user:** the side column of the float player has a field for the
address of a video. The comments of that video then come, and the choice stays for that
episode (read "The user can give the video" in the notes). This does not correct the
search; it removes the need for it in one episode.

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

## 3. The CSP rule replaces the complete header

**Where:** `extension/rules.json`

CSP has no operation that weakens one directive, so the rule must `set` the complete
header. The value is a copy of the CSP of the site with `'self'` in `frame-src`.

If the site adds a directive to its CSP, this copy removes it again for the users of
this extension. The rule is only active while the float player is on, which makes the
time small, but it does not remove the problem.

**Possible correction:** read the CSP of the answer and add `'self'` to the
`frame-src` of that value. `declarativeNetRequest` cannot do this (a rule is
static), so it needs another mechanism.

## 4. `let _ =` on the DOM operations

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

## 5. An episode that the site inserts before the last known one is not reported

**Where:** `crates/background/src/watch.rs`

The check for a new episode keeps the last episode of the chain of the site and asks
`nextPartId` of that episode. This finds every episode that the site puts **after** it.

The site also inserts a part between two episodes (a digest between episode 7 and episode
8 was measured; that part is not in the chain, so it was not inserted into it, but the site
can do it). A part that arrives before the last known one is not in that answer, so the
check does not report it.

**Possible correction:** read the whole chain from the first episode at a long interval (a
week). That is one request per episode of the work, so it is not free.

**Effect if not corrected:** one episode has no entry in the list. Nothing wrong is
reported.

**Way out for the user:** open the work page. The content script sends the whole list that
the page holds, so the state is read again and what was added appears (read "A visit of the
page is a check" in the notes). If the tail itself is gone from that list, the episodes
between it and the end of the list cannot be read; the log says so, and the state is
repaired for the next episode.

## 6. The success code of the episode interface of a `C` part is not measured

**Where:** `crates/background/src/watch.rs`

A `partId` can start with `C`, and the site then asks `WS040101` and not `WS030101`
(`partid.indexOf("C") === 0` in its `common.js`). The code makes the same choice.

`WS040101` has the same fields as `WS030101`, `nextPartId` among them (measured), and it
answers `resultCd: "4"` for an id that belongs to the other interface. So the two are one
shape. What is not measured is the answer for a real `C` part: no work with such an id was
available, so it is not certain that its success code is also `"1"`.

**Effect if not corrected:** a work whose last episode has a `C` id gives no new episode
from the periodic check. The code reads every code that is not `"1"` as "does not exist", so
it reports nothing wrong, and a visit of the work page still finds the episodes.

**Possible correction:** measure one answer of `WS040101` for a real `C` part and accept its
code.
