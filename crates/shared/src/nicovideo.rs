//! The id of a video of nicovideo, from what the user gives.
//!
//! The content script reads what the user pastes into the float player, and the service
//! worker tests the value again before it puts it into an address. Both sides use this
//! one function, so they cannot disagree about what is accepted.

/// A video id has at most this many digits. `1173108780` has ten.
const MAX_DIGITS: usize = 12;

/// The id of a video, from an address or from the id itself. `None` for everything else.
///
/// These forms are accepted:
///
/// ```text
/// https://www.nicovideo.jp/watch/so46649112?ref=…
/// https://sp.nicovideo.jp/watch/sm9
/// https://nico.ms/so46649112
/// so46649112
/// ```
pub fn video_id_from(input: &str) -> Option<String> {
    // The query and the fragment are not part of the id
    let text = input.trim().split(['?', '#']).next()?;
    // The id is the last part of the path. A trailing slash gives an empty part.
    let candidate = text
        .trim_end_matches('/')
        .rsplit('/')
        .next()?
        .to_ascii_lowercase();
    is_video_id(&candidate).then_some(candidate)
}

/// `sm9`, `so12345`, `1173108780`: up to two letters and then digits.
fn is_video_id(text: &str) -> bool {
    let digits = text.trim_start_matches(|c: char| c.is_ascii_lowercase());
    // Both are ASCII here, so the difference of the lengths is the number of letters
    let letters = text.len() - digits.len();
    letters <= 2
        && !digits.is_empty()
        && digits.len() <= MAX_DIGITS
        && digits.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::video_id_from;

    /// A real id of an episode on a channel. Every test uses the same one.
    const SAMPLE: &str = "so46649112";

    #[test]
    fn reads_the_address_of_a_video() {
        assert_eq!(
            video_id_from("https://www.nicovideo.jp/watch/so46649112").as_deref(),
            Some(SAMPLE)
        );
        // A share address has a query
        assert_eq!(
            video_id_from("https://www.nicovideo.jp/watch/so46649112?ref=share&cp_in=watch")
                .as_deref(),
            Some(SAMPLE)
        );
        assert_eq!(
            video_id_from("https://sp.nicovideo.jp/watch/sm9").as_deref(),
            Some("sm9")
        );
        // The short address of the site
        assert_eq!(
            video_id_from("https://nico.ms/so46649112").as_deref(),
            Some(SAMPLE)
        );
        // An old video has only digits
        assert_eq!(
            video_id_from("https://www.nicovideo.jp/watch/1173108780").as_deref(),
            Some("1173108780")
        );
        // A trailing slash and a fragment
        assert_eq!(
            video_id_from("https://www.nicovideo.jp/watch/so46649112/#comment").as_deref(),
            Some(SAMPLE)
        );
    }

    #[test]
    fn reads_the_id_alone() {
        assert_eq!(video_id_from("so46649112").as_deref(), Some(SAMPLE));
        assert_eq!(video_id_from("  sm9  ").as_deref(), Some("sm9"));
        // A copy from the address bar can have capitals
        assert_eq!(video_id_from("SO46649112").as_deref(), Some(SAMPLE));
    }

    #[test]
    fn refuses_what_is_not_a_video() {
        assert_eq!(video_id_from(""), None);
        assert_eq!(video_id_from("   "), None);
        // A page that is not a video
        assert_eq!(video_id_from("https://www.nicovideo.jp/"), None);
        assert_eq!(
            video_id_from("https://www.nicovideo.jp/user/12345/video"),
            None
        );
        // A word, and a number that is too long
        assert_eq!(video_id_from("watch"), None);
        assert_eq!(video_id_from("so"), None);
        assert_eq!(video_id_from("1234567890123"), None);
        // More than two letters is not an id of nicovideo
        assert_eq!(video_id_from("abcd1234"), None);
    }
}
