//! The keys of the comment cache in `storage.local`.
//!
//! The background and the options page both use them: the background reads and writes,
//! and the options page has the button that removes them.

/// The map from an episode to a video, without the version.
pub const VIDEO_ROOT: &str = "dt:vid:";

/// The map from an episode to a video, with the version.
///
/// Increase the version after every change of the match logic (`matching`). The key then
/// changes, the old results are not read, and the new rules run. If you forget this step,
/// the correction has no effect.
///
/// Two examples, both from real use:
///
/// - An episode that is not on nicovideo: an old version had a fallback of "if the
///   episode number gives nothing, select by the length", which selected episode 1 (the
///   most comments), and that stayed for 30 days.
/// - Another episode was correct, but a "not found" of an earlier version stayed for one
///   day.
pub const VIDEO_PREFIX: &str = "dt:vid:v2:";

/// The video that the user gave for an episode, by its address.
///
/// It is not under `VIDEO_ROOT`. That root goes away with a new version of the match
/// (`drop_stale_video_entries`), and the choice of a user is not a result of the match:
/// the user gives an address when the search of nicovideo cannot find the video.
///
/// The entry has no age. Only the user removes it, with the button of the float player.
pub const PIN_PREFIX: &str = "dt:pin:";

/// The comments. The key is the video id, so a change of the match logic does not make
/// them wrong.
pub const COMMENT_PREFIX: &str = "dt:cmt:";
/// The index of the comments, in write order.
pub const COMMENT_INDEX: &str = "dt:cmtidx";
