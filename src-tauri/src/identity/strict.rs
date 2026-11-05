//! Strict level: provider names where punctuation still matters.
use unicode_normalization::UnicodeNormalization;

/// Compatibility-normalize (NFKC), lowercase, unify typographic dashes and
/// apostrophes, and collapse whitespace. Accents and punctuation are kept, so
/// "Song (Live)" and "Song - Live" stay different. Used for Last.fm cache keys
/// and biography candidates; callers that need a fallback use
/// [`super::loose_key`] next.
pub(crate) fn strict_key(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|character| match character {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
