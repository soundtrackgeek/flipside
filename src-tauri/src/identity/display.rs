//! Display level: the catalog's own spelling of a value.

/// Typographic dashes the catalog treats as `-` in artist names
/// (hyphen, non-breaking hyphen, figure dash, en dash, em dash, minus).
const DASHES: [char; 6] = [
    '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2212}',
];

/// Trim, lowercase, and collapse whitespace. Accents and punctuation are kept,
/// so "Björk" and "Bjork" stay different. Used for genres, album titles in
/// import history, and folder-sync track matching.
pub(crate) fn display_key(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replace typographic dashes with `-`.
fn fold_dashes(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if DASHES.contains(&character) {
                '-'
            } else {
                character
            }
        })
        .collect()
}

/// [`display_key`] with typographic dashes folded. May be empty.
pub(crate) fn artist_text_key(value: &str) -> String {
    display_key(&fold_dashes(value))
}

/// The catalog artist key: Artists summaries, artist filters, the MusicBrainz
/// overlay, and portraits are grouped by it. Empty names become `unknown`.
/// [`artist_key_sql`] computes the same key inside SQLite.
pub(crate) fn artist_key(value: &str) -> String {
    let key = artist_text_key(value);
    if key.is_empty() {
        "unknown".to_string()
    } else {
        key
    }
}

/// SQL twin of [`artist_key`] for `field`.
///
/// The text of this expression is part of the schema: `idx_albums_artist_key`
/// indexes exactly this expression, and SQLite only uses an expression index
/// when a query repeats the expression verbatim. It may only call built-in
/// functions and `unicode_lower`, because older builds open newer databases
/// (the catalog is synced between PCs) and must still be able to write rows.
///
/// Known difference: SQL `TRIM` only removes ASCII spaces and does not
/// collapse inner runs, so names with doubled or non-ASCII whitespace get a
/// different key in SQL than in Rust. The golden tests pin this.
pub(crate) fn artist_key_sql(field: &str) -> String {
    format!(
        "COALESCE(NULLIF(TRIM(unicode_lower({})), ''), 'unknown')",
        fold_dashes_sql(field)
    )
}

/// Album artist identity, with the legacy name key as a compatibility fallback.
/// Both albums and tracks publish the same assignment column.
pub(crate) fn catalog_artist_key_sql(field: &str) -> String {
    let assignment = field.replace("album_artist_display", "artist_identity_key");
    format!("COALESCE(NULLIF({assignment}, ''), {})", artist_key_sql(field))
}

/// SQL twin of [`fold_dashes`] for `field`, with NULL read as empty.
pub(crate) fn fold_dashes_sql(field: &str) -> String {
    DASHES
        .iter()
        .fold(format!("COALESCE({field}, '')"), |expression, dash| {
            format!("REPLACE({expression}, char({}), '-')", u32::from(*dash))
        })
}
