//! The "store" side of Flipside: browsing and searching music metadata.
//!
//! Deezer's public API (no key needed) powers charts, new releases, search and
//! discographies. Last.fm, Fanart.tv and Discogs enrich artist and album pages when
//! their keys are configured. Responses are normalised into small typed structs and
//! cached in memory so navigation feels instant.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::identity::{edition_title_key, loose_artist_key};
use crate::{keys, library};

const DEEZER: &str = "https://api.deezer.com";
const CACHE_TTL: Duration = Duration::from_secs(600);

type Cache = HashMap<String, (Instant, Value)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

pub(crate) fn cached_get(url: &str, query: &[(&str, &str)]) -> Result<Value> {
    let key = format!(
        "{url}?{}",
        query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
    );
    if let Some((at, value)) = CACHE
        .lock()
        .ok()
        .and_then(|c| c.as_ref().and_then(|m| m.get(&key).cloned()))
    {
        if at.elapsed() < CACHE_TTL {
            return Ok(value);
        }
    }
    // MusicBrainz and Discogs reject anonymous clients: always identify ourselves.
    let mut request = crate::http::get(url)
        .timeout(Duration::from_secs(12))
        .set("User-Agent", crate::http::USER_AGENT);
    for (name, value) in query {
        request = request.query(name, value);
    }
    let value: Value = request
        .call()
        .map_err(|error| anyhow!("Request to {} failed: {error}", host(url)))?
        .into_json()
        .context("The provider returned an unreadable response")?;
    if let Ok(mut cache) = CACHE.lock() {
        let map = cache.get_or_insert_with(HashMap::new);
        if map.len() > 400 {
            map.clear();
        }
        map.insert(key, (Instant::now(), value.clone()));
    }
    Ok(value)
}

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn deezer(path: &str, query: &[(&str, &str)]) -> Result<Value> {
    let value = cached_get(&format!("{DEEZER}{path}"), query)?;
    if let Some(error) = value.get("error") {
        let message = error.get("message").and_then(Value::as_str).unwrap_or("Deezer error");
        return Err(anyhow!("Deezer: {message}"));
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AlbumCard {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub cover: String,
    pub cover_large: String,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    pub kind: String,
    pub track_count: Option<u32>,
    pub explicit: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtistCard {
    pub id: String,
    pub name: String,
    pub picture: String,
    pub fans: Option<u32>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackRow {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration_seconds: u32,
    pub disc: u32,
    pub position: u32,
    pub explicit: bool,
    pub album_id: Option<String>,
    pub album_title: Option<String>,
    pub cover: Option<String>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Genre {
    pub id: String,
    pub name: String,
    pub picture: String,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HomeFeed {
    pub picks: Vec<AlbumCard>,
    pub new_releases: Vec<AlbumCard>,
    pub chart_albums: Vec<AlbumCard>,
    pub top_artists: Vec<ArtistCard>,
    pub top_tracks: Vec<TrackRow>,
    pub genres: Vec<Genre>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub query: String,
    pub albums: Vec<AlbumCard>,
    pub artists: Vec<ArtistCard>,
    pub tracks: Vec<TrackRow>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDetail {
    pub album: AlbumCard,
    pub label: Option<String>,
    pub genres: Vec<String>,
    pub duration_seconds: u32,
    pub fans: Option<u32>,
    pub tracks: Vec<TrackRow>,
    pub upc: Option<String>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtistDetail {
    pub artist: ArtistCard,
    pub albums: Vec<AlbumCard>,
    pub related: Vec<ArtistCard>,
    pub top_tracks: Vec<TrackRow>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtistEnrichment {
    pub background: Option<String>,
    pub thumb: Option<String>,
    pub bio: Option<String>,
    pub tags: Vec<String>,
    pub similar: Vec<String>,
    pub listeners: Option<f64>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AlbumEnrichment {
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub listeners: Option<f64>,
    pub label: Option<String>,
    pub styles: Vec<String>,
    pub country: Option<String>,
    pub discogs_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Normalisers
// ---------------------------------------------------------------------------

fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn id_of(value: &Value) -> String {
    text(value, "id")
}

fn year_of(date: &str) -> Option<i32> {
    date.get(..4).and_then(|y| y.parse().ok()).filter(|y| *y > 1900)
}

fn first_non_empty(v: &Value, keys: &[&str]) -> String {
    keys.iter().map(|k| text(v, k)).find(|s| !s.is_empty()).unwrap_or_default()
}

fn album_card(v: &Value, fallback_artist: Option<&Value>) -> Option<AlbumCard> {
    let id = id_of(v);
    if id.is_empty() {
        return None;
    }
    let artist = v.get("artist").or(fallback_artist);
    let release_date = Some(text(v, "release_date")).filter(|d| !d.is_empty());
    let cover = first_non_empty(v, &["cover_big", "cover_medium", "cover"]);
    let cover_large = first_non_empty(v, &["cover_xl", "cover_big", "cover_medium"]);
    Some(AlbumCard {
        id,
        title: text(v, "title"),
        artist: artist.map(|a| text(a, "name")).unwrap_or_default(),
        artist_id: artist.map(id_of).filter(|s| !s.is_empty()),
        cover,
        cover_large,
        year: release_date.as_deref().and_then(year_of),
        release_date,
        kind: text(v, "record_type"),
        track_count: v.get("nb_tracks").and_then(Value::as_u64).map(|n| n as u32),
        explicit: v.get("explicit_lyrics").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn items(v: Option<&Value>) -> &[Value] {
    v.and_then(|v| v.get("data").or(Some(v)))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Lists never show albums you already own (when "hide owned" is on).
fn album_cards(v: Option<&Value>, fallback_artist: Option<&Value>) -> Vec<AlbumCard> {
    items(v)
        .iter()
        .filter_map(|x| album_card(x, fallback_artist))
        .filter(|a| !library::should_hide(&a.artist, &a.title))
        .collect()
}

fn artist_card(v: &Value) -> Option<ArtistCard> {
    let id = id_of(v);
    if id.is_empty() {
        return None;
    }
    Some(ArtistCard {
        id,
        name: text(v, "name"),
        picture: first_non_empty(v, &["picture_xl", "picture_big", "picture_medium"]),
        fans: v.get("nb_fan").and_then(Value::as_u64).map(|n| n as u32),
    })
}

fn artist_cards(v: Option<&Value>) -> Vec<ArtistCard> {
    items(v).iter().filter_map(artist_card).collect()
}

fn track_row(v: &Value) -> Option<TrackRow> {
    let id = id_of(v);
    if id.is_empty() {
        return None;
    }
    let album = v.get("album");
    Some(TrackRow {
        id,
        title: text(v, "title"),
        artist: v.get("artist").map(|a| text(a, "name")).unwrap_or_default(),
        duration_seconds: v.get("duration").and_then(Value::as_u64).unwrap_or(0) as u32,
        disc: v.get("disk_number").and_then(Value::as_u64).unwrap_or(1) as u32,
        position: v.get("track_position").and_then(Value::as_u64).unwrap_or(0) as u32,
        explicit: v.get("explicit_lyrics").and_then(Value::as_bool).unwrap_or(false),
        album_id: album.map(id_of).filter(|s| !s.is_empty()),
        album_title: album.map(|a| text(a, "title")).filter(|s| !s.is_empty()),
        cover: album.map(|a| text(a, "cover_medium")).filter(|s| !s.is_empty()),
    })
}

fn track_rows(v: Option<&Value>) -> Vec<TrackRow> {
    items(v)
        .iter()
        .filter_map(track_row)
        .filter(|t| t.album_title.as_deref().is_none_or(|album| !library::should_hide(&t.artist, album)))
        .collect()
}

fn clean_id(id: &str) -> Result<&str> {
    if !id.is_empty() && id.len() < 24 && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(anyhow!("Invalid id"))
    }
}

// ---------------------------------------------------------------------------
// Deezer-backed queries
// ---------------------------------------------------------------------------

type Joined = std::thread::Result<Result<Value>>;

fn joined(result: Joined, what: &str) -> Result<Value> {
    result.map_err(|_| anyhow!("{what} task failed"))?
}

pub fn home() -> Result<HomeFeed> {
    // Independent calls run side by side so the first screen is quick.
    let (chart, releases, picks, genres) = std::thread::scope(|scope| {
        let chart = scope.spawn(|| deezer("/chart", &[("limit", "40")]));
        let releases = scope.spawn(|| deezer("/editorial/0/releases", &[("limit", "40")]));
        let picks = scope.spawn(|| deezer("/editorial/0/selection", &[]));
        let genres = scope.spawn(|| deezer("/genre", &[]));
        (chart.join(), releases.join(), picks.join(), genres.join())
    });
    let chart = joined(chart, "chart")?;
    let releases = joined(releases, "releases").ok();
    let picks = joined(picks, "picks").ok();
    let genres = joined(genres, "genre").ok();
    let genres = items(genres.as_ref())
        .iter()
        .filter(|g| id_of(g) != "0")
        .map(|g| Genre { id: id_of(g), name: text(g, "name"), picture: text(g, "picture_medium") })
        .collect();
    Ok(HomeFeed {
        picks: album_cards(picks.as_ref(), None),
        new_releases: album_cards(releases.as_ref(), None),
        chart_albums: album_cards(chart.get("albums"), None),
        top_artists: artist_cards(chart.get("artists")),
        top_tracks: track_rows(chart.get("tracks")),
        genres,
    })
}

pub fn genre_albums(id: &str) -> Result<Vec<AlbumCard>> {
    let id = clean_id(id)?;
    let value = deezer(&format!("/chart/{id}/albums"), &[("limit", "60")])?;
    Ok(album_cards(Some(&value), None))
}

pub fn search(query: &str) -> Result<SearchResults> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchResults { query: String::new(), albums: vec![], artists: vec![], tracks: vec![] });
    }
    let (albums, artists, tracks) = std::thread::scope(|scope| {
        let albums = scope.spawn(|| deezer("/search/album", &[("q", query), ("limit", "48")]));
        let artists = scope.spawn(|| deezer("/search/artist", &[("q", query), ("limit", "12")]));
        let tracks = scope.spawn(|| deezer("/search/track", &[("q", query), ("limit", "12")]));
        (albums.join(), artists.join(), tracks.join())
    });
    let albums = joined(albums, "album search")?;
    let artists = joined(artists, "artist search").ok();
    let tracks = joined(tracks, "track search").ok();
    Ok(SearchResults {
        query: query.to_owned(),
        albums: album_cards(Some(&albums), None),
        artists: artist_cards(artists.as_ref()),
        tracks: track_rows(tracks.as_ref()),
    })
}

pub fn album(id: &str) -> Result<AlbumDetail> {
    let id = clean_id(id)?;
    let v = deezer(&format!("/album/{id}"), &[])?;
    let card = album_card(&v, None).context("Album not found")?;
    let tracks_value = deezer(&format!("/album/{id}/tracks"), &[("limit", "200")]).ok();
    let mut tracks = track_rows(tracks_value.as_ref().or_else(|| v.get("tracks")));
    for (index, track) in tracks.iter_mut().enumerate() {
        if track.position == 0 {
            track.position = (index + 1) as u32;
        }
        track.album_id = Some(card.id.clone());
    }
    Ok(AlbumDetail {
        label: Some(text(&v, "label")).filter(|s| !s.is_empty()),
        genres: items(v.get("genres"))
            .iter()
            .map(|x| text(x, "name"))
            .filter(|s| !s.is_empty())
            .collect(),
        duration_seconds: v.get("duration").and_then(Value::as_u64).unwrap_or(0) as u32,
        fans: v.get("fans").and_then(Value::as_u64).map(|n| n as u32),
        upc: Some(text(&v, "upc")).filter(|s| !s.is_empty()),
        album: card,
        tracks,
    })
}

pub fn artist(id: &str) -> Result<ArtistDetail> {
    let id = clean_id(id)?;
    let (info, albums, related, top) = std::thread::scope(|scope| {
        let info = scope.spawn(|| deezer(&format!("/artist/{id}"), &[]));
        let albums = scope.spawn(|| deezer(&format!("/artist/{id}/albums"), &[("limit", "120")]));
        let related = scope.spawn(|| deezer(&format!("/artist/{id}/related"), &[("limit", "14")]));
        let top = scope.spawn(|| deezer(&format!("/artist/{id}/top"), &[("limit", "10")]));
        (info.join(), albums.join(), related.join(), top.join())
    });
    let info = joined(info, "artist")?;
    let artist = artist_card(&info).context("Artist not found")?;
    let albums = joined(albums, "albums").ok();
    let related = joined(related, "related").ok();
    let top = joined(top, "top tracks").ok();
    let mut cards = album_cards(albums.as_ref(), Some(&info));
    // Newest first; Deezer's own order is arbitrary.
    cards.sort_by(|a, b| b.release_date.cmp(&a.release_date));
    Ok(ArtistDetail {
        artist,
        albums: cards,
        related: artist_cards(related.as_ref()),
        top_tracks: track_rows(top.as_ref()),
    })
}

// ---------------------------------------------------------------------------
// Enrichment (Last.fm, Fanart.tv, Discogs) — all optional
// ---------------------------------------------------------------------------

const LASTFM: &str = "https://ws.audioscrobbler.com/2.0/";

fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&quot;", "\"").replace("&amp;", "&").replace("&#39;", "'").trim().to_owned()
}

pub(crate) fn lastfm_json(method: &str, params: &[(&str, &str)]) -> Option<Value> {
    lastfm(method, params)
}

/// Finds the Deezer album matching a MusicBrainz/Discogs release so it can be opened and downloaded.
pub fn resolve_album(artist: &str, title: &str) -> Result<Option<AlbumCard>> {
    let wanted_artist = loose_artist_key(artist);
    let wanted_title = edition_title_key(title);
    let queries = [
        format!("artist:\"{}\" album:\"{}\"", artist.replace('"', ""), title.replace('"', "")),
        format!("{artist} {title}"),
    ];
    for query in queries {
        let value = deezer("/search/album", &[("q", query.as_str()), ("limit", "20")])?;
        let found = items(Some(&value))
            .iter()
            .filter_map(|x| album_card(x, None))
            .find(|a| loose_artist_key(&a.artist) == wanted_artist && edition_title_key(&a.title) == wanted_title);
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

fn lastfm(method: &str, params: &[(&str, &str)]) -> Option<Value> {
    let key = keys::get("lastfm_key")?;
    let mut query = vec![
        ("method", method),
        ("api_key", key.as_str()),
        ("format", "json"),
        ("autocorrect", "1"),
    ];
    query.extend_from_slice(params);
    let value = cached_get(LASTFM, &query).ok()?;
    if value.get("error").is_some() {
        None
    } else {
        Some(value)
    }
}

fn tag_names(v: Option<&Value>) -> Vec<String> {
    v.and_then(|t| t.get("tag"))
        .and_then(Value::as_array)
        .map(|t| t.iter().map(|x| text(x, "name")).filter(|s| !s.is_empty()).take(6).collect())
        .unwrap_or_default()
}

fn summary_of(node: Option<&Value>, key: &str) -> Option<String> {
    node.and_then(|n| n.get(key))
        .map(|b| text(b, "summary"))
        .map(|s| strip_html(s.split("<a href").next().unwrap_or(&s)))
        .filter(|s| !s.is_empty())
}

pub fn artist_enrichment(name: &str) -> Result<ArtistEnrichment> {
    let info = lastfm("artist.getInfo", &[("artist", name)]);
    let artist = info.as_ref().and_then(|i| i.get("artist"));
    let mbid = artist.map(|a| text(a, "mbid")).filter(|s| !s.is_empty());
    let (background, thumb) = match (keys::get("fanart_key"), mbid) {
        (Some(key), Some(mbid)) => {
            let url = format!("https://webservice.fanart.tv/v3/music/{mbid}");
            match cached_get(&url, &[("api_key", key.as_str())]) {
                Ok(v) => {
                    let first = |k: &str| {
                        v.get(k)
                            .and_then(Value::as_array)
                            .and_then(|a| a.first())
                            .map(|x| text(x, "url"))
                            .filter(|s| !s.is_empty())
                    };
                    (first("artistbackground"), first("artistthumb"))
                }
                Err(_) => (None, None),
            }
        }
        _ => (None, None),
    };
    Ok(ArtistEnrichment {
        background,
        thumb,
        bio: summary_of(artist, "bio"),
        tags: tag_names(artist.and_then(|a| a.get("tags"))),
        similar: artist
            .and_then(|a| a.get("similar"))
            .and_then(|s| s.get("artist"))
            .and_then(Value::as_array)
            .map(|s| s.iter().map(|x| text(x, "name")).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default(),
        listeners: artist
            .and_then(|a| a.get("stats"))
            .and_then(|s| text(s, "listeners").parse().ok()),
    })
}

pub fn album_enrichment(artist: &str, title: &str) -> Result<AlbumEnrichment> {
    let info = lastfm("album.getInfo", &[("artist", artist), ("album", title)]);
    let album = info.as_ref().and_then(|i| i.get("album"));
    let mut out = AlbumEnrichment {
        summary: summary_of(album, "wiki"),
        tags: tag_names(album.and_then(|a| a.get("tags"))),
        listeners: album.and_then(|a| text(a, "listeners").parse().ok()),
        label: None,
        styles: vec![],
        country: None,
        discogs_url: None,
    };

    if let (Some(key), Some(secret)) = (keys::get("discogs_key"), keys::get("discogs_secret")) {
        let found = cached_get(
            "https://api.discogs.com/database/search",
            &[
                ("type", "master"),
                ("artist", artist),
                ("release_title", title),
                ("per_page", "1"),
                ("key", key.as_str()),
                ("secret", secret.as_str()),
            ],
        )
        .ok()
        .and_then(|v| v.get("results").and_then(Value::as_array).and_then(|a| a.first().cloned()));
        if let Some(hit) = found {
            let list = |k: &str| -> Vec<String> {
                hit.get(k)
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default()
            };
            out.styles = list("style");
            out.label = list("label").into_iter().next();
            out.country = Some(text(&hit, "country")).filter(|s| !s.is_empty());
            out.discogs_url = Some(text(&hit, "uri"))
                .filter(|s| !s.is_empty())
                .map(|u| format!("https://www.discogs.com{u}"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_parses_only_plausible_values() {
        assert_eq!(year_of("2019-04-05"), Some(2019));
        assert_eq!(year_of("0000-00-00"), None);
        assert_eq!(year_of(""), None);
    }

    #[test]
    fn ids_must_be_numeric() {
        assert!(clean_id("302127").is_ok());
        assert!(clean_id("../etc").is_err());
        assert!(clean_id("").is_err());
    }

    #[test]
    fn html_is_stripped_from_bios() {
        assert_eq!(strip_html("A <b>band</b> &amp; friends"), "A band & friends");
    }

    // Live smoke tests against the public Deezer API: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_home_search_album_artist() {
        let home = home().expect("home feed");
        assert!(!home.chart_albums.is_empty(), "chart albums");
        assert!(!home.genres.is_empty(), "genres");
        println!("picks={} releases={} chart={} artists={}", home.picks.len(), home.new_releases.len(), home.chart_albums.len(), home.top_artists.len());

        let found = search("daft punk discovery").expect("search");
        assert!(!found.albums.is_empty());
        let detail = album("302127").expect("album");
        assert_eq!(detail.album.title, "Discovery");
        assert_eq!(detail.tracks.len(), 14);
        assert_eq!(detail.tracks[0].position, 1);
        let artist = artist("27").expect("artist");
        assert!(!artist.albums.is_empty());
        println!("artist {} albums={} related={}", artist.artist.name, artist.albums.len(), artist.related.len());
    }
}
