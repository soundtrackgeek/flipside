//! Loose level: names as printed by different sources.
//!
//! These rules are shared with Aurora, which keeps a copy under its old names
//! (`chart_identity::text_key`, `artist_group_key`, `main_performers`). Keep
//! both copies byte-identical.
use unicode_normalization::UnicodeNormalization;

/// Fold case, accents, Nordic and ligature letters, punctuation, and `&`.
/// Stored chart, Wish List, and Artist Completion keys depend on this
/// contract; changing it requires rebuilding every stored key in the catalog.
pub(crate) fn loose_key(value: &str) -> String {
    let lowercased = value.replace('&', " and ").to_lowercase();
    let folded = lowercased
        .nfd()
        .filter(|character| !unicode_normalization::char::is_combining_mark(*character))
        .fold(String::new(), |mut normalized, character| {
            match character {
                'æ' => normalized.push_str("ae"),
                'œ' => normalized.push_str("oe"),
                'ø' => normalized.push('o'),
                'ð' => normalized.push('d'),
                'þ' => normalized.push_str("th"),
                'ł' => normalized.push('l'),
                'ß' => normalized.push_str("ss"),
                _ => normalized.push(character),
            }
            normalized
        });

    folded
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Group printed spellings of one artist credit. Archives print the same duo
/// with "&", "and", "/" or "+", and bands with or without a leading "The".
/// Different names (ELO vs Electric Light Orchestra) need explicit aliases.
pub(crate) fn loose_artist_key(value: &str) -> String {
    let key = loose_key(value);
    let mut words = key
        .split(' ')
        .filter(|word| *word != "and")
        .collect::<Vec<_>>();
    if words.len() > 1 && words[0] == "the" {
        words.remove(0);
    }
    if words.is_empty() {
        return key;
    }
    words.join(" ")
}

/// Main performers of a credit, lead first, as group keys. Charts and tags
/// print collaborations differently ("JOHN LENNON" vs "John Lennon & Yoko
/// Ono", "DAVE/ TEMS"), so matching compares performers. Guests after "feat.",
/// "with", "vs" or "x", or in parentheses, are left out: remakes often feature
/// the original artist ("N-Trance feat. Rod Stewart", "Kygo x Tina Turner").
pub(crate) fn credit_keys(value: &str) -> Vec<String> {
    const GUEST_JOINS: [&str; 8] = [
        "with",
        "feat",
        "featuring",
        "ft",
        "vs",
        "versus",
        "x",
        "duet",
    ];
    let mut performers = Vec::new();
    let mut guests = false;
    let mut depth = 0_usize;
    let mut segment = String::new();
    for character in value.chars().chain(std::iter::once(',')) {
        if !matches!(
            character,
            '&' | '/' | '+' | ',' | ';' | '(' | ')' | '[' | ']'
        ) {
            segment.push(character);
            continue;
        }
        let mut guest = guests || depth > 0;
        let words = segment.split_whitespace().collect::<Vec<_>>();
        let mut start = 0;
        for (index, word) in words.iter().enumerate() {
            let join = word.trim_end_matches('.').to_lowercase();
            let guest_join = GUEST_JOINS.contains(&join.as_str());
            // A trailing word belongs to the name: "Lil Nas X", "Rodney O".
            if (guest_join || join == "and") && index + 1 < words.len() {
                push_credit_key(&words[start..index], guest, &mut performers);
                start = index + 1;
                if guest_join {
                    guest = true;
                    guests |= depth == 0;
                }
            }
        }
        push_credit_key(&words[start..], guest, &mut performers);
        segment.clear();
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    performers
}

fn push_credit_key(words: &[&str], guest: bool, performers: &mut Vec<String>) {
    let performer = loose_artist_key(&words.join(" "));
    if !guest && !performer.is_empty() && !performers.contains(&performer) {
        performers.push(performer);
    }
}

/// Remove the ` [NO]` marker VG-lista and Norsktoppen print after Norwegian
/// artists ("a-ha [NO]"). Other bracketed text is part of the name.
pub(crate) fn strip_chart_country_suffix(artist: &str) -> &str {
    let artist = artist.trim();
    let suffix_start = artist.len().saturating_sub(4);
    if artist
        .get(suffix_start..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case("[no]"))
    {
        artist[..suffix_start].trim_end()
    } else {
        artist
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_keys_keep_co_leads_and_drop_guests() {
        for (value, expected) in [
            ("John Lennon & Yoko Ono", &["john lennon", "yoko ono"][..]),
            (
                "Julio Iglesias and Diana Ross",
                &["julio iglesias", "diana ross"],
            ),
            ("DAVE/ TEMS", &["dave", "tems"]),
            ("DJ Sammy & Yanou feat. Do", &["dj sammy", "yanou"]),
            ("DAVE STEWART WITH BARBARA GASKIN", &["dave stewart"]),
            ("P!nk feat. Nate Ruess", &["p nk"]),
            (
                "DJ Khaled featuring Jay-Z, Future & Beyoncé",
                &["dj khaled"],
            ),
            ("N-TRANCE FT ROD STEWART", &["n trance"]),
            ("Kygo x Whitney Houston", &["kygo"]),
            ("Mark Ronson (featuring Bruno Mars)", &["mark ronson"]),
            ("(MC SAR &) THE REAL MCCOY", &["real mccoy"]),
            (
                "Frank Sinatra with Harry James And His Orchestra",
                &["frank sinatra"],
            ),
            ("Lil Nas X", &["lil nas x"]),
            ("The Beatles", &["beatles"]),
            (
                "Tom Petty and the Heartbreakers",
                &["tom petty", "heartbreakers"],
            ),
            ("  ", &[]),
        ] {
            assert_eq!(credit_keys(value), expected, "{value}");
        }
    }

    #[test]
    fn loose_keys_fold_case_accents_punctuation_and_ampersands() {
        for (value, expected) in [
            ("  BÉYONCÉ & JAY-Z  ", "beyonce and jay z"),
            ("Æ Œ Ø Ð Þ Ł ß", "ae oe o d th l ss"),
            ("Don't  Stop… (Live)", "don t stop live"),
            ("P!nk", "p nk"),
            ("", ""),
        ] {
            assert_eq!(loose_key(value), expected, "{value}");
        }
    }

    #[test]
    fn loose_artist_keys_merge_printed_credit_variants_only() {
        for spellings in [
            &[
                "Daryl Hall & John Oates",
                "Daryl Hall John Oates",
                "Daryl Hall / John Oates",
                "DARYL HALL AND JOHN OATES",
            ][..],
            &["Prince And The Revolution", "Prince & The Revolution"],
            &["The Clovers", "Clovers", "CLOVERS"],
            &["Queen & David Bowie", "Queen + David Bowie"],
            &["Beyoncé", "BEYONCE"],
        ] {
            let expected = loose_artist_key(spellings[0]);
            for spelling in spellings {
                assert_eq!(loose_artist_key(spelling), expected, "{spelling}");
            }
        }
        for (left, right) in [
            ("ELO", "Electric Light Orchestra"),
            (
                "Electric Light Orchestra",
                "Electric Light Orchestra Part Two",
            ),
            ("P!nk", "P!nk featuring Nate Ruess"),
            ("Pink", "P!nk"),
        ] {
            assert_ne!(loose_artist_key(left), loose_artist_key(right));
        }
        assert_eq!(loose_artist_key("The The"), "the");
        assert_eq!(loose_artist_key("And"), "and");
        assert_eq!(loose_artist_key(""), "");
    }
}
