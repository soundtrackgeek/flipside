//! Golden corpus and property tests for every identity level.
use super::*;
use rusqlite::Connection;
use unicode_normalization::UnicodeNormalization;

const GOLDEN_CORPUS: &str = include_str!("golden_corpus.csv");

#[derive(serde::Deserialize)]
struct CorpusRow {
    level: String,
    left: String,
    right: String,
    relation: String,
    note: String,
}

fn corpus() -> Vec<CorpusRow> {
    csv::Reader::from_reader(GOLDEN_CORPUS.as_bytes())
        .deserialize()
        .collect::<Result<_, _>>()
        .expect("golden corpus parses")
}

fn key_at(level: &str, value: &str) -> String {
    match level {
        "display" => display_key(value),
        "artist" => artist_key(value),
        "strict" => strict_key(value),
        "loose" => loose_key(value),
        "loose_artist" => loose_artist_key(value),
        "credit" => credit_keys(value).join(" | "),
        "chart_artist" => loose_key(strip_chart_country_suffix(value)),
        "edition_title" => edition_title_key(value),
        other => panic!("unknown corpus level {other}"),
    }
}

#[test]
fn golden_corpus_matches_every_level() {
    let rows = corpus();
    assert!(rows.len() > 50, "corpus lost rows");
    let mut failures = Vec::new();
    for row in &rows {
        let left = key_at(&row.level, &row.left);
        let passed = match row.relation.as_str() {
            "->" => left == row.right,
            "=" => left == key_at(&row.level, &row.right),
            "!=" => left != key_at(&row.level, &row.right),
            other => panic!("unknown corpus relation {other}"),
        };
        if !passed {
            failures.push(format!(
                "{} {:?} {} {:?}: got {:?} vs {:?} ({})",
                row.level,
                row.left,
                row.relation,
                row.right,
                left,
                key_at(&row.level, &row.right),
                row.note
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The SQL text is indexed by `idx_albums_artist_key`; any change stops
/// SQLite from using the index and must come with a migration.
#[test]
fn artist_key_sql_text_is_the_indexed_expression() {
    assert_eq!(
        artist_key_sql("album_artist_display"),
        "COALESCE(NULLIF(TRIM(unicode_lower(REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(\
         COALESCE(album_artist_display, ''), char(8208), '-'), char(8209), '-'), \
         char(8210), '-'), char(8211), '-'), char(8212), '-'), char(8722), '-'))), ''), 'unknown')"
    );
}

/// Characters that have broken matching before: accents in both composed and
/// decomposed form, Nordic letters, ligatures, typographic punctuation,
/// connectors, brackets, and unusual whitespace.
const ALPHABET: &[&str] = &[
    "a",
    "B",
    "z",
    "Q",
    "1",
    "9",
    "é",
    "e\u{301}",
    "Ö",
    "ø",
    "Ø",
    "æ",
    "Å",
    "ß",
    "ẞ",
    "ł",
    "İ",
    "ﬁ",
    "Ｂ",
    "²",
    "&",
    "/",
    "+",
    "!",
    "'",
    "’",
    "-",
    "‐",
    "–",
    "—",
    "−",
    ".",
    ",",
    "(",
    ")",
    "[",
    "]",
    " ",
    " ",
    " ",
    "  ",
    "\t",
    "\u{a0}",
    "the ",
    "The ",
    " and ",
    " feat. ",
    " x ",
    " with ",
    "Remastered",
    " - ",
    "Live",
    "Deluxe Edition",
    "[NO]",
];

/// Deterministic xorshift strings so failures reproduce without a seed.
fn generated_names(count: usize) -> Vec<String> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..count)
        .map(|_| {
            let length = (next() % 12) as usize;
            (0..length)
                .map(|_| ALPHABET[(next() % ALPHABET.len() as u64) as usize])
                .collect()
        })
        .collect()
}

/// `loose_artist_key` drops one leading "the", so a key that still starts with
/// "the " after that (from "The The Beatles") loses another on a second pass.
/// The rule is a stored-key contract shared with Aurora; keep it, but only
/// ever apply the level to raw names.
fn starts_with_repeated_article(key: &str) -> bool {
    key.starts_with("the ")
}

#[test]
fn every_level_is_idempotent() {
    for name in generated_names(4_000) {
        for level in [
            "display",
            "artist",
            "strict",
            "loose",
            "loose_artist",
            "edition_title",
        ] {
            let once = key_at(level, &name);
            if level == "loose_artist" && starts_with_repeated_article(&once) {
                continue;
            }
            assert_eq!(key_at(level, &once), once, "{level} {name:?}");
        }
        for credit in credit_keys(&name) {
            if !starts_with_repeated_article(&credit) {
                assert_eq!(loose_artist_key(&credit), credit, "credit {name:?}");
            }
        }
    }
}

#[test]
fn loose_keys_are_lowercase_words_separated_by_single_spaces() {
    for name in generated_names(4_000) {
        let key = loose_key(&name);
        assert_eq!(key.trim(), key, "{name:?}");
        assert!(!key.contains("  "), "{name:?} -> {key:?}");
        assert!(
            key.chars()
                .all(|c| c == ' ' || (c.is_alphanumeric() && !c.is_uppercase())),
            "{name:?} -> {key:?}"
        );
    }
}

#[test]
fn looser_levels_never_split_what_stricter_levels_join() {
    for name in generated_names(4_000) {
        let loose = loose_key(&name);
        assert_eq!(loose_key(&display_key(&name)), loose, "display {name:?}");
        assert_eq!(loose_key(&artist_text_key(&name)), loose, "artist {name:?}");
        // NFKC can turn one character into several ("ﬁ" -> "fi"); loose_key
        // keeps compatibility characters as they are, so skip those names.
        if name.nfkc().eq(name.nfc()) {
            assert_eq!(loose_key(&strict_key(&name)), loose, "strict {name:?}");
        }
        let title = edition_title_key(&name);
        assert!(loose.starts_with(&title), "title {name:?} -> {title:?}");
    }
}

