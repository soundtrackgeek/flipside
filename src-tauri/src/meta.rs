//! Artist and album facts that the Deezer catalog gets wrong or does not have:
//!
//! * the artist's own studio albums with their **original** release year (MusicBrainz, local data first),
//! * the original release year of any album (Deezer only knows re-release dates),
//! * a biography from Wikipedia.

use anyhow::Result;
use rusqlite::params;
use serde_json::Value;
use std::collections::HashSet;

use crate::covers::titles_match;
use crate::discovery::{self, DiscoveryAlbum};
use crate::identity::{edition_title_key, loose_artist_key};
use crate::{catalog, library};

const FOUND_TTL_DAYS: i64 = 365;
const MISSING_TTL_DAYS: i64 = 30;

fn age_days(timestamp: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_days())
        .unwrap_or(i64::MAX)
}

/// Values go inside a quoted Lucene phrase, where punctuation is literal: only quotes and backslashes
/// would break out of it.
fn lucene_clean(value: &str) -> String {
    value
        .chars()
        .map(|c| if c == '"' || c == '\\' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Studio albums of one artist
// ---------------------------------------------------------------------------

/// The artist's studio albums, newest first. Uses the local MusicBrainz data when it knows the artist
/// and asks MusicBrainz otherwise. Albums already in the library are left out when "hide owned" is on.
pub fn artist_albums(name: &str) -> Result<Vec<DiscoveryAlbum>> {
    let key = loose_artist_key(name);
    let lib = library::loaded();
    let mbid = lib
        .as_ref()
        .and_then(|l| l.key_to_mbid.get(&key).cloned())
        .or_else(|| discovery::known_mbid(&key));
    let mbid = match mbid {
        Some(mbid) => mbid,
        None => match discovery::musicbrainz_artist(name)? {
            Some(mbid) => mbid,
            None => return Ok(Vec::new()),
        },
    };
    let library_genre = lib
        .as_ref()
        .and_then(|l| l.artists.get(&key))
        .map(|a| a.genre.clone())
        .filter(|g| !g.is_empty());

    // (id, title, year, genres)
    let mut rows: Vec<(String, String, Option<i32>, Vec<String>)> = library::local_albums(&mbid, true)
        .into_iter()
        .map(|(id, title, year)| (id, title, year, library_genre.iter().cloned().collect()))
        .collect();
    if rows.is_empty() {
        rows = discovery::musicbrainz_studio_albums(&mbid)?
            .into_iter()
            .map(|a| (a.id, a.title, a.year, a.genres))
            .collect();
    }

    let mut seen = HashSet::new();
    let mut albums: Vec<DiscoveryAlbum> = rows
        .into_iter()
        .filter(|(_, title, _, _)| !library::looks_like_compilation(title))
        .filter(|(_, title, _, _)| seen.insert(edition_title_key(title)))
        .filter(|(_, title, _, _)| !library::should_hide(name, title))
        .map(|(id, title, year, genres)| DiscoveryAlbum {
            cover: discovery::caa(&id),
            id,
            title,
            artist: name.to_owned(),
            artist_mbid: Some(mbid.clone()),
            year,
            genres,
            similar: false,
            because: Vec::new(),
        })
        .collect();
    albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.title.cmp(&b.title)));
    Ok(albums)
}

// ---------------------------------------------------------------------------
// Original release year
// ---------------------------------------------------------------------------

/// First release year of an album: local MusicBrainz data, then a cached answer, then MusicBrainz itself.
pub fn original_year(artist: &str, title: &str) -> Result<Option<i32>> {
    let artist_key = loose_artist_key(artist);

    if let Some(mbid) = library::loaded().and_then(|l| l.key_to_mbid.get(&artist_key).cloned()) {
        let year = library::local_albums(&mbid, false)
            .into_iter()
            .filter(|(_, found, _)| titles_match(found, title))
            .filter_map(|(_, _, year)| year)
            .min();
        if year.is_some() {
            return Ok(year);
        }
    }

    let cache_key = format!("{artist_key}|{}", edition_title_key(title));
    let conn = discovery::flip().ok();
    if let Some(conn) = &conn {
        let cached: Option<(Option<i64>, String)> = conn
            .query_row("SELECT year, checked_at FROM disc_year WHERE k = ?1", [&cache_key], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok();
        if let Some((year, checked_at)) = cached {
            let limit = if year.is_some() { FOUND_TTL_DAYS } else { MISSING_TTL_DAYS };
            if age_days(&checked_at) < limit {
                return Ok(year.map(|y| y as i32));
            }
        }
    }

    let year = musicbrainz_year(artist, title)?;
    if let Some(conn) = &conn {
        let _ = conn.execute(
            "INSERT INTO disc_year (k, year, checked_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(k) DO UPDATE SET year = excluded.year, checked_at = excluded.checked_at",
            params![cache_key, year, chrono::Utc::now().to_rfc3339()],
        );
    }
    Ok(year)
}

fn musicbrainz_year(artist: &str, title: &str) -> Result<Option<i32>> {
    let query = format!("releasegroup:\"{}\" AND artist:\"{}\"", lucene_clean(title), lucene_clean(artist));
    let value = catalog::cached_get(
        "https://musicbrainz.org/ws/2/release-group",
        &[("query", query.as_str()), ("limit", "6"), ("fmt", "json")],
    )?;
    let wanted = loose_artist_key(artist);
    Ok(value
        .get("release-groups")
        .and_then(Value::as_array)
        .and_then(|groups| {
            groups
                .iter()
                .filter(|g| g.get("score").and_then(Value::as_i64).unwrap_or(0) >= 80)
                .filter(|g| titles_match(g.get("title").and_then(Value::as_str).unwrap_or_default(), title))
                .filter(|g| {
                    g.get("artist-credit")
                        .and_then(Value::as_array)
                        .is_some_and(|credits| {
                            credits.iter().any(|c| {
                                let name = c.get("name").and_then(Value::as_str).unwrap_or_default();
                                let artist_name = c.pointer("/artist/name").and_then(Value::as_str).unwrap_or_default();
                                loose_artist_key(name) == wanted || loose_artist_key(artist_name) == wanted
                            })
                        })
                })
                .filter_map(|g| g.get("first-release-date").and_then(Value::as_str)?.get(..4)?.parse::<i32>().ok())
                .filter(|y| *y > 1900)
                .min()
        }))
}

// ---------------------------------------------------------------------------
// Wikipedia biography
// ---------------------------------------------------------------------------

const MUSIC_WORDS: [&str; 22] = [
    "singer", "musician", "band", "rapper", "songwriter", "composer", "duo", "group", "vocalist", "guitarist",
    "pianist", "drummer", "producer", "dj", "trio", "quartet", "orchestra", "ensemble", "bassist", "saxophonist",
    "violinist", "recording artist",
];

/// `(text, page url)` of the artist's Wikipedia summary, when the page is clearly about a musician.
pub fn wikipedia_bio(name: &str) -> Option<(String, String)> {
    let wanted = loose_artist_key(name);
    let candidates = [
        name.to_owned(),
        format!("{name} (singer)"),
        format!("{name} (band)"),
        format!("{name} (musician)"),
        format!("{name} (rapper)"),
        format!("{name} (group)"),
    ];
    for title in candidates {
        let slug = percent_encoding::utf8_percent_encode(&title.replace(' ', "_"), percent_encoding::NON_ALPHANUMERIC).to_string();
        let Ok(page) = catalog::cached_get(&format!("https://en.wikipedia.org/api/rest_v1/page/summary/{slug}"), &[]) else {
            continue;
        };
        if page.get("type").and_then(Value::as_str) != Some("standard") {
            continue;
        }
        let page_title = page.get("title").and_then(Value::as_str).unwrap_or_default();
        let bare = page_title.split(" (").next().unwrap_or(page_title);
        if loose_artist_key(bare) != wanted {
            continue;
        }
        let description = page.get("description").and_then(Value::as_str).unwrap_or_default().to_lowercase();
        let is_musician = MUSIC_WORDS.iter().any(|w| description.split(|c: char| !c.is_alphanumeric()).any(|word| word == *w))
            || title.ends_with("(singer)")
            || title.ends_with("(band)")
            || title.ends_with("(musician)")
            || title.ends_with("(rapper)")
            || title.ends_with("(group)");
        let extract = page.get("extract").and_then(Value::as_str).unwrap_or_default().trim();
        if !is_musician || extract.is_empty() {
            continue;
        }
        let url = page
            .pointer("/content_urls/desktop/page")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("https://en.wikipedia.org/wiki/{slug}"));
        return Some((extract.to_owned(), url));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lucene_input_is_neutralised() {
        assert_eq!(lucene_clean("AC/DC: \"Back\" (Live)"), "AC/DC: Back (Live)");
        assert_eq!(lucene_clean("a\\b"), "a b");
    }

    /// Live: `cargo test --lib live_meta -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_meta() {
        let year = original_year("Donna Fargo", "The Happiest Girl In The Whole U.S.A.").expect("year");
        println!("original year: {year:?}");
        assert_eq!(year, Some(1972), "Deezer says 2008; MusicBrainz knows the original");

        let bio = wikipedia_bio("Donna Fargo").expect("wikipedia");
        println!("wikipedia: {} ... {}", &bio.0[..80.min(bio.0.len())], bio.1);
        assert!(bio.0.contains("country"));
        assert!(wikipedia_bio("Zzyzx Qwerty Band 12345").is_none());

        let albums = artist_albums("Donna Fargo").expect("albums");
        for a in &albums {
            println!("  {:?} {}", a.year, a.title);
        }
        assert!(albums.iter().any(|a| a.title.contains("Happiest Girl") && a.year == Some(1972)));
        assert!(!albums.iter().any(|a| a.title == "Country Queens"), "other artists' albums must not appear");
    }
}
