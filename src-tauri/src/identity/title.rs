//! Title level: one work across its reissues.
use super::loose_key;

/// Words that mark a reissue or packaging of the same recordings.
const EDITION_WORDS: [&str; 11] = [
    "anniversary",
    "bonus",
    "deluxe",
    "edition",
    "expanded",
    "reissue",
    "reissued",
    "remaster",
    "remastered",
    "remastering",
    "remasters",
];

/// Words that mark different recordings; a decoration naming one is kept.
const DISTINCT_WORDS: [&str; 13] = [
    "acoustic",
    "demo",
    "demos",
    "edit",
    "instrumental",
    "instrumentals",
    "karaoke",
    "live",
    "mix",
    "remix",
    "remixes",
    "session",
    "sessions",
];

/// [`loose_key`] of a title without trailing reissue decorations:
/// "OK Computer (Remastered)" and "Song - 2011 Remaster" become "ok computer"
/// and "song". Live, demo, remix, and similar versions keep their decoration
/// ("Abbey Road (2019 Mix)") because they are different recordings. Used to
/// decide whether an album is already owned; stored identities keep the full
/// [`loose_key`].
pub(crate) fn edition_title_key(value: &str) -> String {
    let mut title = value.trim();
    while let Some(base) = without_edition_suffix(title) {
        title = base;
    }
    let key = loose_key(title);
    if key.is_empty() {
        loose_key(value)
    } else {
        key
    }
}

/// `title` before its last bracketed group or ` - ` suffix, when that suffix
/// is only a reissue decoration.
fn without_edition_suffix(title: &str) -> Option<&str> {
    let (base, decoration) = split_bracket_suffix(title).or_else(|| split_dash_suffix(title))?;
    let base = base.trim_end();
    (!base.is_empty() && is_edition_decoration(decoration)).then_some(base)
}

fn split_bracket_suffix(title: &str) -> Option<(&str, &str)> {
    let close = title.chars().next_back()?;
    let open = match close {
        ')' => '(',
        ']' => '[',
        _ => return None,
    };
    let mut depth = 0_usize;
    for (index, character) in title.char_indices().rev() {
        if character == close {
            depth += 1;
        } else if character == open {
            depth -= 1;
            if depth == 0 {
                return Some((&title[..index], &title[index + 1..title.len() - 1]));
            }
        }
    }
    None
}

fn split_dash_suffix(title: &str) -> Option<(&str, &str)> {
    [" - ", " \u{2013} ", " \u{2014} "]
        .iter()
        .filter_map(|separator| {
            title
                .rfind(separator)
                .map(|index| (index, &title[index + separator.len()..]))
        })
        .max_by_key(|(index, _)| *index)
        .map(|(index, decoration)| (&title[..index], decoration))
}

fn is_edition_decoration(decoration: &str) -> bool {
    let key = loose_key(decoration);
    let words = key.split(' ').collect::<Vec<_>>();
    words.iter().any(|word| EDITION_WORDS.contains(word))
        && !words.iter().any(|word| DISTINCT_WORDS.contains(word))
}
