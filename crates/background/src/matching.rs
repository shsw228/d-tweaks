//! Selects the nicovideo video of a dAnime episode.
//!
//! The official channel uses the same text as dAnime. Measured with the snapshot
//! interface:
//!
//! | dAnime | Title on nicovideo |
//! |---|---|
//! | `作品A シーズン2` / `第363話` | `作品A シーズン2　第363話　「サブタイトル前／後」` |
//! | `「作品D」第二期` / `第十四回` | `「作品D」第二期　第十四回　サブタイトルD` |
//!
//! An episode number can be a kanji number (`第十四回`). A comparison of numbers alone
//! loses those, so the text of the label is compared first.
//!
//! # A label can be only a number
//!
//! `partDispNumber` of `WS010105` returns `第241話` for some works and `6` for others
//! (measured). Never use a bare number for a text match: `"6"` is inside `第16話` and
//! inside `シーズン6`, so it gives the comments of another episode.
//!
//! For the same reason, do not compare with every number of a title: a search for
//! `第2話` finds the `2` of `作品A シーズン2　第363話`. Only a number that is written
//! as an episode number counts (`第N話`, `第N回`, `#N`, `Episode N`, `N話`).
//!
//! One episode can have more than one video. Measured: two per episode, with and
//! without brackets around the subtitle. The one with more comments wins.

/// What the selection needs from a video of the search.
pub struct Candidate {
    pub content_id: String,
    pub title: String,
    /// `Some` for a channel, `None` for a video of a user.
    pub channel_id: Option<f64>,
    pub comment_count: f64,
    pub length_seconds: Option<f64>,
}

/// Remove the decoration that a search cannot use.
pub fn sanitize_title(title: &str) -> String {
    let mut s = to_half_width(title);

    // These brackets often hold the work title itself, so remove the brackets and
    // keep the text: 「作品D」第二期 becomes 作品D 第二期
    s = s.replace(['「', '」', '『', '』'], " ");

    // These hold a note, so remove the text also
    s = strip_bracketed(&s);

    // Remove a prefix for the kind of medium
    for prefix in [
        "劇場版",
        "映画",
        "TVアニメーション",
        "TVアニメ",
        "テレビアニメーション",
        "テレビアニメ",
        "劇場アニメ",
        "アニメ",
    ] {
        if let Some(rest) = s.trim_start().strip_prefix(prefix) {
            s = rest.to_string();
        }
    }

    s = strip_season_suffix(&s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Full-width letters, digits and some marks to half width.
fn to_half_width(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            'Ａ'..='Ｚ' | 'ａ'..='ｚ' | '０'..='９' => {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            }
            '：' => ':',
            '＆' => '&',
            '＃' => '#',
            '\u{3000}' | '\u{00A0}' => ' ',
            _ => c,
        })
        .collect()
}

/// Remove a note in brackets, with its text.
///
/// `「」` and `『』` are not here: a dAnime work title can be inside them, as in
/// `「作品D」第二期`, and the title would disappear.
fn strip_bracketed(input: &str) -> String {
    const PAIRS: [(char, char); 6] = [
        ('【', '】'),
        ('（', '）'),
        ('〈', '〉'),
        ('＜', '＞'),
        ('[', ']'),
        ('《', '》'),
    ];
    let mut out = String::with_capacity(input.len());
    let mut closing: Option<char> = None;
    for c in input.chars() {
        match closing {
            None => {
                if let Some((_, close)) = PAIRS.iter().find(|(open, _)| *open == c) {
                    closing = Some(*close);
                } else {
                    out.push(c);
                }
            }
            Some(close) if c == close => closing = None,
            Some(_) => {}
        }
    }
    // An open bracket without its pair: return the input, remove nothing
    if closing.is_some() {
        input.to_string()
    } else {
        out
    }
}

/// Remove a season at the end, such as `第2期` or `第二期`.
fn strip_season_suffix(input: &str) -> String {
    let trimmed = input.trim_end();
    for suffix in [
        "第一期",
        "第二期",
        "第三期",
        "第四期",
        "第1期",
        "第2期",
        "第3期",
        "第4期",
    ] {
        if let Some(rest) = trimmed.strip_suffix(suffix) {
            return rest.trim_end().to_string();
        }
    }
    trimmed.to_string()
}

/// Remove the differences of the spaces and the widths, for a comparison.
fn normalize_for_compare(input: &str) -> String {
    to_half_width(input)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Also remove the brackets and the separators, for a comparison of titles.
///
/// The same title can have other brackets:
///
/// | dAnime (after `sanitize_title`) | nicovideo |
/// |---|---|
/// | `作品D 第二期` | `「作品D」第二期　…` |
/// | `サブタイトル前／後` | `「サブタイトル前／後」` |
///
/// A text match on those fails although the titles agree, and then there are no
/// comments. Not used for the episode number, so a sequence of digits stays as it is.
fn normalize_loose(input: &str) -> String {
    const DROP: [char; 16] = [
        '「', '」', '『', '』', '（', '）', '(', ')', '【', '】', '〈', '〉', '《', '》', '[', ']',
    ];
    normalize_for_compare(input)
        .chars()
        .filter(|c| !DROP.contains(c))
        .collect()
}

/// The number of a label (`"第363話"` gives `Some(363)`). Not for a kanji number.
pub fn episode_number(label: &str) -> Option<u32> {
    let digits: String = to_half_width(label)
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Collect the numbers of a title that are written as an episode number.
///
/// Takes the 6 of `第6話`, `第6回`, `#6`, `Episode 6` and `6話`, but not the 4 of
/// `シーズン4` and not a number of a subtitle.
fn title_episode_numbers(title: &str) -> Vec<u32> {
    let normalized = normalize_for_compare(title).to_ascii_lowercase();
    let chars: Vec<char> = normalized.chars().collect();
    let mut numbers = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        let digits: String = chars[start..index].iter().collect();
        let Ok(number) = digits.parse::<u32>() else {
            continue;
        };

        // Is the text before it an episode marker?
        let before: String = chars[..start].iter().collect();
        let leads = before.ends_with('第')
            || before.ends_with('#')
            || before.ends_with("episode")
            || before.ends_with("ep")
            || before.ends_with("no.");
        // Is the text after it an episode marker?
        let trails = matches!(chars.get(index), Some('話') | Some('回'));
        if leads || trails {
            numbers.push(number);
        }
    }
    numbers
}

/// Is the candidate the same episode?
///
/// 1. A label that is written as an episode number (`第十四回`) is compared as text.
///    Only this finds a kanji number.
/// 2. If the label gives a number, compare it with the numbers of the title that are
///    written as an episode number.
///
/// A bare number is never used for a text match (see the head of this module).
pub fn title_matches_episode(title: &str, episode_label: &str) -> bool {
    let label_n = normalize_for_compare(episode_label);
    if label_n.is_empty() {
        return false;
    }

    // A label of only digits (`6`) is inside 第16話, so it is not a text match
    let bare_number = label_n.chars().all(|c| c.is_ascii_digit());
    if !bare_number && normalize_for_compare(title).contains(&label_n) {
        return true;
    }

    let Some(number) = episode_number(episode_label) else {
        return false;
    };
    title_episode_numbers(title).contains(&number)
}

/// The word for the season of a work title.
///
/// The search words have no `第2期` (with it, the official title can give zero
/// results), so candidates of another season arrive. A `第6話` exists in every season,
/// so a selection by the comment count can take the wrong season.
///
/// With this word the candidates can be filtered. `None` if there is no season.
pub fn season_token(work_title: &str) -> Option<String> {
    let normalized = normalize_for_compare(work_title);
    for token in [
        "第一期",
        "第二期",
        "第三期",
        "第四期",
        "第1期",
        "第2期",
        "第3期",
        "第4期",
    ] {
        if normalized.contains(token) {
            return Some(token.to_string());
        }
    }
    // `シーズン2` and `season2` include the number
    for prefix in ["シーズン", "season", "Season"] {
        if let Some(rest) = normalized.split(prefix).nth(1) {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some(format!("{prefix}{digits}"));
            }
        }
    }
    None
}

/// What to search for. Given to `pick`.
pub struct Want<'a> {
    /// The work title after `sanitize_title`.
    pub work_title: &'a str,
    /// `第363話`, `第十四回`. `None` for a work without episodes (a film).
    pub episode_label: Option<&'a str>,
    /// The episode title (`partTitle`), to confirm a candidate.
    pub episode_title: Option<&'a str>,
    /// The length on dAnime.
    pub duration_seconds: Option<f64>,
    /// `第二期`, `シーズン2`.
    pub season: Option<String>,
}

/// Accepted range of the length, as a ratio of the length on dAnime.
///
/// A ratio and not a difference in seconds: the official video can be some seconds
/// longer, because of a logo or a sponsor card at the start, but the videos to remove
/// are previews, which are much shorter (measured: 90 s against 1420 s).
///
/// A limit in seconds removes a correct video that has a sponsor card, and then there
/// are no comments. A ratio removes only what is much shorter.
const DURATION_MIN_RATIO: f64 = 0.7;
const DURATION_MAX_RATIO: f64 = 1.5;

/// Select one candidate. Without certainty, select nothing.
///
/// The official channel uses this form (measured):
///
/// ```text
/// 作品A シーズン2　第363話　「サブタイトル前／後」
/// 「作品D」第二期　第十四回　サブタイトルD
/// |- work title ----| |- number -| |- episode title ----|
/// ```
///
/// So the title must have the work title and the episode number in it. With an episode
/// title, a candidate that also has that is preferred: one episode can have two
/// videos that differ only in the brackets.
///
/// Without a match, this returns `None`. An earlier version selected by the length
/// when the number gave nothing, and the result was the comments of another episode.
/// No comments is better than the wrong comments, and the UI says "not found".
///
/// The one exception is a work without episode numbers (a film), where the work title
/// and the length decide.
pub fn pick<'a>(candidates: &'a [Candidate], want: &Want<'_>) -> Option<&'a Candidate> {
    // The brackets differ, so compare without them
    let work = normalize_loose(want.work_title);

    let mut best: Option<(u32, f64, &Candidate)> = None;
    for candidate in candidates {
        // Only a video of a channel
        if candidate.channel_id.is_none() {
            continue;
        }
        let title = normalize_loose(&candidate.title);

        // The work title must be in it
        if work.is_empty() || !title.contains(&work) {
            continue;
        }
        // With a season, the candidate must have that season
        if let Some(season) = &want.season
            && !title.contains(&normalize_loose(season))
        {
            continue;
        }
        // With an episode number, the candidate must be that episode
        match want.episode_label {
            Some(label) if !label.trim().is_empty() => {
                if !title_matches_episode(&candidate.title, label) {
                    continue;
                }
            }
            // Without a number, only the length can decide
            _ => {
                if want.duration_seconds.is_none() {
                    continue;
                }
            }
        }
        // Remove what is much shorter or much longer (a preview)
        if let Some(target) = want.duration_seconds
            && target > 0.0
        {
            let ok = candidate.length_seconds.is_some_and(|length| {
                length >= target * DURATION_MIN_RATIO && length <= target * DURATION_MAX_RATIO
            });
            if !ok {
                continue;
            }
        }

        // Every candidate here is the same episode. Prefer one with the episode title.
        let score = match want.episode_title {
            Some(episode_title) if title.contains(&normalize_loose(episode_title)) => 1,
            _ => 0,
        };
        let better = match best {
            None => true,
            Some((best_score, best_comments, _)) => {
                (score, candidate.comment_count) > (best_score, best_comments)
            }
        };
        if better {
            best = Some((score, candidate.comment_count, candidate));
        }
    }

    best.map(|(_, _, candidate)| candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search. The work title must already be through `sanitize_title`.
    fn want<'a>(
        work: &'a str,
        label: Option<&'a str>,
        title: Option<&'a str>,
        duration: Option<f64>,
    ) -> Want<'a> {
        Want {
            work_title: work,
            episode_label: label,
            episode_title: title,
            duration_seconds: duration,
            season: season_token(work),
        }
    }

    fn candidate(id: &str, title: &str, official: bool, comments: f64, len: f64) -> Candidate {
        Candidate {
            content_id: id.into(),
            title: title.into(),
            channel_id: if official { Some(1.0) } else { None },
            comment_count: comments,
            length_seconds: Some(len),
        }
    }

    #[test]
    fn never_matches_a_different_episode_from_a_bare_number() {
        // `partDispNumber` of WS010105 can be a bare number such as `6` (measured).
        // As a text match it is also inside 第16話, which is another episode.
        let title = "作品F　シーズン4　第16話　サブタイトルF1";
        assert!(!title_matches_episode(title, "6"));
        assert!(title_matches_episode(title, "16"));

        // The correct episode matches
        let sixth = "作品F　シーズン4　第6話　サブタイトルF2";
        assert!(title_matches_episode(sixth, "6"));
        assert!(title_matches_episode(sixth, "第6話"));
    }

    #[test]
    fn ignores_numbers_that_are_not_episode_numbers() {
        // A season number must not match
        let title = "作品A シーズン2　第363話　「サブタイトル前／後」";
        assert!(!title_matches_episode(title, "2"));
        assert!(!title_matches_episode(title, "第2話"));
        assert!(title_matches_episode(title, "第363話"));

        // A number of a subtitle must not match
        let sub = "作品名　第7話　3年目の夏";
        assert!(!title_matches_episode(sub, "3"));
        assert!(title_matches_episode(sub, "7"));
    }

    #[test]
    fn understands_other_episode_notations() {
        // A kanji number matches as text; it cannot become a number
        assert!(title_matches_episode(
            "「作品D」第二期　第十四回　サブタイトルD",
            "第十四回"
        ));
        // #6, Episode 12 and 6話 are also episode numbers
        assert!(title_matches_episode("作品名 #6 サブタイトル", "第6話"));
        assert!(title_matches_episode("Work Title Episode 12", "12"));
        assert!(title_matches_episode("作品名 6話 サブタイトル", "6"));
    }

    #[test]
    fn prefers_the_same_season() {
        // The search words have no 第二期, so candidates of another season arrive. The
        // comment count alone would take the wrong season, so the season filters.
        let candidates = [
            candidate("sm1", "作品D　第6話　サブタイトル", true, 9000.0, 1420.0),
            candidate(
                "sm2",
                "「作品D」第二期　第6話　サブタイトル",
                true,
                300.0,
                1420.0,
            ),
        ];
        let picked = pick(
            &candidates,
            &want("作品D 第二期", Some("第6話"), None, Some(1420.0)),
        )
        .unwrap();
        assert_eq!(picked.content_id, "sm2");

        // Without a season (the first one), both candidates pass: nothing can decide.
        // They are the same episode with the same length, so this does no harm. What
        // matters is that a known season is never wrong.
        let picked = pick(
            &candidates,
            &want("作品D", Some("第6話"), None, Some(1420.0)),
        )
        .unwrap();
        assert_eq!(picked.content_id, "sm1", "the one with more comments");
    }

    #[test]
    fn picks_the_right_episode_from_a_real_reply() {
        // dAnime (WS010105): work, number, subtitle and 1420 seconds.
        // nicovideo (snapshot interface): the values below are the real reply.
        let candidates = [
            candidate("so1", "作品E　第5話　サブタイトルE", true, 2079.0, 1420.0),
            candidate("so2", "作品E 第5話「サブタイトルE」", true, 62182.0, 1420.0),
            candidate(
                "so3",
                "作品E 第4話「サブタイトルE2」",
                true,
                69830.0,
                1420.0,
            ),
            candidate(
                "so4",
                "個人配信2026-07-21「雑談、『作品E』",
                true,
                0.0,
                4140.0,
            ),
        ];
        let picked = pick(
            &candidates,
            &want("作品E", Some("第5話"), Some("サブタイトルE"), Some(1420.0)),
        )
        .expect("episode 5 must be found");
        assert_eq!(
            picked.content_id, "so2",
            "the same episode: more comments wins"
        );
    }

    #[test]
    fn matches_across_bracket_differences() {
        // The same title with other brackets (real data). A failure here means: the
        // titles agree but there are no comments.
        let candidates = [candidate(
            "so1",
            "「作品D」第二期　第十四回　サブタイトルD",
            true,
            500.0,
            1420.0,
        )];
        let picked = pick(
            &candidates,
            &want(
                // `sanitize_title` removes these brackets
                "作品D 第二期",
                Some("第十四回"),
                Some("サブタイトルD"),
                Some(1420.0),
            ),
        )
        .unwrap();
        assert_eq!(picked.content_id, "so1");

        // The brackets of an episode title also differ
        let candidates = [candidate(
            "so2",
            "作品A シーズン2　第363話　「サブタイトル前／後」",
            true,
            500.0,
            93.0,
        )];
        let picked = pick(
            &candidates,
            &want(
                "作品A シーズン2",
                Some("第363話"),
                Some("サブタイトル前／後"),
                Some(93.0),
            ),
        )
        .unwrap();
        assert_eq!(picked.content_id, "so2");
    }

    #[test]
    fn gives_up_instead_of_showing_another_episode() {
        // If the episode is not on nicovideo, never take another episode. No comments
        // is better than the wrong comments.
        let candidates = [
            candidate(
                "so1",
                "作品A シーズン2　第362話　「サブタイトル362」",
                true,
                900.0,
                93.0,
            ),
            candidate(
                "so2",
                "作品A シーズン2　第364話　「サブタイトル364」",
                true,
                800.0,
                93.0,
            ),
        ];
        assert!(
            pick(
                &candidates,
                &want("作品A シーズン2", Some("第363話"), None, Some(93.0))
            )
            .is_none()
        );
    }

    #[test]
    fn requires_the_work_title_too() {
        // Another work with the same number is not a match
        let candidates = [candidate(
            "so1",
            "別のアニメ　第6話　サブタイトル",
            true,
            900.0,
            1420.0,
        )];
        assert!(
            pick(
                &candidates,
                &want("作品A シーズン2", Some("第6話"), None, Some(1420.0))
            )
            .is_none()
        );
    }

    #[test]
    fn drops_previews_by_length() {
        // A preview has the work title and the number in it, so the length removes it
        let candidates = [
            candidate("so1", "作品名　第6話予告", true, 5000.0, 90.0),
            candidate("so2", "作品名　第6話　サブタイトル", true, 100.0, 1420.0),
        ];
        let picked = pick(
            &candidates,
            &want("作品名", Some("第6話"), None, Some(1420.0)),
        )
        .unwrap();
        assert_eq!(picked.content_id, "so2");
    }

    #[test]
    fn keeps_uploads_that_are_a_little_longer() {
        // The nicovideo version can be longer (a logo, a sponsor card); keep it
        let candidates = [candidate(
            "so1",
            "作品名　第6話　サブタイトル",
            true,
            100.0,
            1480.0,
        )];
        assert!(
            pick(
                &candidates,
                &want("作品名", Some("第6話"), None, Some(1420.0))
            )
            .is_some()
        );
    }

    #[test]
    fn prefers_the_candidate_whose_subtitle_matches() {
        // Two videos of one episode, one with another subtitle (possibly another
        // episode). The episode title wins over the comment count.
        let candidates = [
            candidate(
                "so1",
                "作品名　第6話　ちがうサブタイトル",
                true,
                9000.0,
                1420.0,
            ),
            candidate(
                "so2",
                "作品名　第6話　「サブタイトルF2」",
                true,
                100.0,
                1420.0,
            ),
        ];
        let picked = pick(
            &candidates,
            &want(
                "作品名",
                Some("第6話"),
                Some("サブタイトルF2"),
                Some(1420.0),
            ),
        )
        .unwrap();
        assert_eq!(picked.content_id, "so2");
    }

    #[test]
    fn reads_season_tokens_from_work_titles() {
        assert_eq!(season_token("「作品D」第二期").as_deref(), Some("第二期"));
        assert_eq!(
            season_token("作品A シーズン2").as_deref(),
            Some("シーズン2")
        );
        assert_eq!(season_token("作品A"), None);
    }

    #[test]
    fn sanitizes_decorations_from_work_titles() {
        // All of these are real dAnime titles
        assert_eq!(sanitize_title("作品A シーズン2"), "作品A シーズン2");
        assert_eq!(sanitize_title("劇場版 作品B"), "作品B");
        assert_eq!(sanitize_title("ＷＯＲＫ　ＴＩＴＬＥ"), "WORK TITLE");
        // These marks are not brackets and stay; they are part of the title
        assert_eq!(
            sanitize_title("作品C 〜サブタイトルC〜"),
            "作品C 〜サブタイトルC〜"
        );
    }

    #[test]
    fn unwraps_quoted_title_but_drops_notes() {
        // These brackets hold the work title, so the text stays
        assert_eq!(sanitize_title("「作品D」第二期"), "作品D");
        // A note in brackets goes with its text
        assert_eq!(sanitize_title("作品G＜無修正Ver.＞"), "作品G");
        assert_eq!(sanitize_title("作品A（第2期）"), "作品A");
        // An open bracket without its pair returns the input
        assert_eq!(strip_bracketed("未対応（かっこ"), "未対応（かっこ");
    }

    #[test]
    fn parses_episode_numbers() {
        assert_eq!(episode_number("第363話"), Some(363));
        assert_eq!(episode_number("第５話"), Some(5));
        assert_eq!(episode_number("#12"), Some(12));
        assert_eq!(episode_number("＃05"), Some(5));
        assert_eq!(episode_number("本編"), None);
        // A kanji number cannot become a number, so the text match must come first
        assert_eq!(episode_number("第十五回"), None);
    }

    #[test]
    fn matches_kanji_numbered_episodes_by_string() {
        // Real data: a kanji number, so only the text match finds it
        let title = "「作品D」第二期　第十四回　サブタイトルD";
        assert!(title_matches_episode(title, "第十四回"));
        assert!(!title_matches_episode(title, "第十三回"));
    }

    #[test]
    fn matches_numeric_episodes_and_width_variants() {
        let title = "作品A シーズン2　第363話　「サブタイトル前／後」";
        assert!(title_matches_episode(title, "第363話"));
        assert!(!title_matches_episode(title, "第364話"));
        // Full-width label and full-width title
        assert!(title_matches_episode("作品A　第３６３話", "第363話"));
        // Another form still matches through the number
        assert!(title_matches_episode("作品A 第5話", "#5"));
    }

    #[test]
    fn picks_official_upload_with_most_comments() {
        let candidates = vec![
            // A video of a user is never selected, also with more comments
            candidate("sm1", "作品A 第363話 音MAD", false, 99999.0, 211.0),
            candidate(
                "so1",
                "作品A シーズン2　第363話　「サブタイトル前」",
                true,
                28.0,
                93.0,
            ),
            candidate(
                "so2",
                "作品A シーズン2 第363話「サブタイトル前」",
                true,
                51.0,
                93.0,
            ),
            candidate(
                "so3",
                "作品A シーズン2　第362話　「サブタイトル362」",
                true,
                900.0,
                94.0,
            ),
        ];
        let picked = pick(
            &candidates,
            &want("作品A シーズン2", Some("第363話"), None, None),
        )
        .unwrap();
        assert_eq!(
            picked.content_id, "so2",
            "the same episode: more comments wins"
        );
    }

    #[test]
    fn falls_back_to_duration_when_episode_label_absent() {
        let candidates = vec![
            candidate("so1", "劇場版 作品B 予告", true, 10.0, 90.0),
            candidate("so2", "劇場版 作品B 本編", true, 5.0, 6000.0),
        ];
        // A work without episode numbers is selected by the length
        let picked = pick(&candidates, &want("劇場版 作品B", None, None, Some(6000.0))).unwrap();
        assert_eq!(picked.content_id, "so2");
    }

    #[test]
    fn returns_none_when_nothing_official() {
        let candidates = vec![candidate("sm1", "作品A MAD", false, 100.0, 200.0)];
        assert!(pick(&candidates, &want("作品A", Some("第1話"), None, None)).is_none());
    }
}
