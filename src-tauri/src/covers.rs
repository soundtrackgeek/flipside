//! Cover art fallbacks for albums that come from MusicBrainz.
//!
//! The Cover Art Archive (loaded directly by the UI) has no picture for many older and
//! obscure releases. When it fails the UI asks here, and we try, in order:
//!
//! 1. Deezer (fast, no key, large images), then
//! 2. Discogs (often has art for obscure titles; needs the Discogs keys).
//!
//! Answers, including "nothing found", are cached in Flipside's database so each album is
//! only looked up once.

use anyhow::Result;
use rusqlite::params;
use serde_json::Value;

use crate::identity::{edition_title_key, loose_artist_key};
use crate::{catalog, keys};

const FOUND_TTL_DAYS: i64 = 180;
const MISSING_TTL_DAYS: i64 = 14;

/// Two titles are "the same album" when their edition-free keys match or one is a prefix of the other
/// (Discogs and MusicBrainz differ on subtitles and "Vol." suffixes).
fn titles_match(found: &str, wanted: &str) -> bool {
    let (a, b) = (edition_title_key(found), edition_title_key(wanted));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || (a.len().min(b.len()) >= 8 && (a.starts_with(&b) || b.starts_with(&a)))
}

fn from_deezer(artist: &str, title: &str) -> Option<String> {
    catalog::resolve_album(artist, title)
        .ok()
        .flatten()
        .map(|album| album.cover_large)
        .filter(|url| !url.is_empty())
}

fn from_discogs(artist: &str, title: &str) -> Option<String> {
    let (key, secret) = (keys::get("discogs_key")?, keys::get("discogs_secret")?);
    let value = catalog::cached_get(
        "https://api.discogs.com/database/search",
        &[
            ("type", "master"),
            ("artist", artist),
            ("release_title", title),
            ("per_page", "6"),
            ("key", key.as_str()),
            ("secret", secret.as_str()),
        ],
    )
    .ok()?;
    let wanted_artist = loose_artist_key(artist);
    value.get("results")?.as_array()?.iter().find_map(|result| {
        let name = result.get("title").and_then(Value::as_str)?;
        let (found_artist, found_title) = name.split_once(" - ")?;
        if loose_artist_key(found_artist) != wanted_artist || !titles_match(found_title, title) {
            return None;
        }
        ["cover_image", "thumb"]
            .iter()
            .filter_map(|field| result.get(*field).and_then(Value::as_str))
            .find(|url| !url.is_empty() && !url.contains("spacer.gif"))
            .map(str::to_owned)
    })
}

/// The uncached chain. Returns the image URL and which provider supplied it.
pub fn search(artist: &str, title: &str) -> Option<(String, &'static str)> {
    from_deezer(artist, title)
        .map(|url| (url, "deezer"))
        .or_else(|| from_discogs(artist, title).map(|url| (url, "discogs")))
}

fn age_days(timestamp: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_days())
        .unwrap_or(i64::MAX)
}

/// Cached lookup used by the UI.
pub fn lookup(rgid: &str, artist: &str, title: &str) -> Result<Option<String>> {
    let conn = crate::discovery::flip()?;
    let cached: Option<(Option<String>, String)> = conn
        .query_row("SELECT url, checked_at FROM disc_cover WHERE rgid = ?1", [rgid], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok();
    if let Some((url, checked_at)) = cached {
        let limit = if url.is_some() { FOUND_TTL_DAYS } else { MISSING_TTL_DAYS };
        if age_days(&checked_at) < limit {
            return Ok(url);
        }
    }
    let found = search(artist, title);
    conn.execute(
        "INSERT INTO disc_cover (rgid, url, source, checked_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(rgid) DO UPDATE SET url = excluded.url, source = excluded.source, checked_at = excluded.checked_at",
        params![rgid, found.as_ref().map(|(u, _)| u), found.as_ref().map(|(_, s)| *s), chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(found.map(|(url, _)| url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_tolerate_subtitles_and_editions() {
        assert!(titles_match("Discovery", "Discovery (Deluxe Edition)"));
        assert!(titles_match("My New Celebrity Is You", "My New Celebrity Is You - Vol. III"));
        assert!(!titles_match("Homework", "Discovery"));
        assert!(!titles_match("", "Discovery"));
    }

    /// Live: `cargo test --lib live_covers -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_covers() {
        // Deezer has it.
        let kayak = search("Kayak", "Close to the Fire");
        println!("kayak: {kayak:?}");
        assert!(kayak.is_some());
        // Obscure: found through the Discogs fallback.
        let dearie = search("Blossom Dearie", "My New Celebrity Is You - Vol. III");
        println!("dearie: {dearie:?}");
        assert!(dearie.is_some());
        // Nonsense must not match anything.
        assert!(search("Zzyzx Qwerty Band", "Definitely Not An Album 12345").is_none());
    }
}
