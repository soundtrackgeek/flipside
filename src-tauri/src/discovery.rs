//! "For you": studio albums you don't own, from two sources.
//!
//! * **Missing** - studio albums by artists already in your library (straight from the
//!   MusicBrainz data in your own databases; no network needed).
//! * **Similar** - studio albums by artists Last.fm says are similar to yours but that you do
//!   not have. A background crawl finds the artists (Last.fm), then their albums (MusicBrainz,
//!   with Discogs as a fallback) and stores them in Flipside's own database.
//!
//! Only studio albums are ever listed: no live albums, compilations, soundtracks, remixes,
//! EPs or singles. Everything you already own is excluded.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use tauri::AppHandle;
use tauri_specta::Event as _;

use crate::identity::{edition_title_key, loose_artist_key, loose_key};
use crate::{catalog, keys, library};

static APP: OnceLock<AppHandle> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static CONTINUOUS: AtomicBool = AtomicBool::new(false);
static STATS: Mutex<Stats> = Mutex::new(Stats { seeds_done: 0, seeds_total: 0, artists_done: 0, artists_total: 0 });
/// Library artists Last.fm could not answer for in this session; not retried until the next launch.
static SKIPPED: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
struct Stats {
    seeds_done: u32,
    seeds_total: u32,
    artists_done: u32,
    artists_total: u32,
}
static INDEX: RwLock<Option<Arc<Index>>> = RwLock::new(None);
static PROGRESS: Mutex<Option<DiscoveryProgress>> = Mutex::new(None);

const MAX_CANDIDATES: usize = 300;
const LIVE_SEED_LIMIT: usize = 150;
const ALBUMS_STALE_DAYS: i64 = 45;
const SIMILAR_STALE_DAYS: i64 = 60;
const CRAWL_EVERY_DAYS: i64 = 7;

pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryProgress {
    pub running: bool,
    pub stage: String,
    pub done: u32,
    pub total: u32,
    pub message: String,
    /// Your artists whose similar artists are known, out of all that are worth asking about.
    pub seeds_done: u32,
    pub seeds_total: u32,
    /// Similar artists whose albums have been looked up, out of all candidates found so far.
    pub artists_done: u32,
    pub artists_total: u32,
    pub continuous: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryStatus {
    pub progress: DiscoveryProgress,
    pub library: library::LibraryStatus,
    pub last_crawl: Option<String>,
    pub similar_artists: u32,
    pub lastfm_configured: bool,
}

#[derive(Clone, Debug, Default, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscoveryQuery {
    /// "all", "missing" (artists you own) or "similar" (artists you don't).
    pub source: String,
    pub decade: Option<i32>,
    pub year_from: Option<i32>,
    pub year_to: Option<i32>,
    pub genre: Option<String>,
    /// Genres to leave out. "scores" expands to the film/TV/game score genres.
    pub exclude_genres: Vec<String>,
    pub text: String,
    /// "affinity", "newest", "oldest" or "artist".
    pub sort: String,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_mbid: Option<String>,
    pub year: Option<i32>,
    pub genres: Vec<String>,
    pub similar: bool,
    pub because: Vec<String>,
    pub cover: String,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Facet {
    pub key: String,
    pub label: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPage {
    pub total: u32,
    pub items: Vec<DiscoveryAlbum>,
    pub decades: Vec<Facet>,
    pub genres: Vec<Facet>,
    pub missing_total: u32,
    pub similar_total: u32,
}

// ---------------------------------------------------------------------------
// In-memory index
// ---------------------------------------------------------------------------

struct Item {
    view: DiscoveryAlbum,
    artist_lc: String,
    hay: String,
    genres_lc: Vec<String>,
    affinity: f32,
}

#[derive(Default)]
struct Index {
    items: Vec<Item>,
}

pub(crate) fn caa(rgid: &str) -> String {
    format!("https://coverartarchive.org/release-group/{rgid}/front-500")
}

pub(crate) fn title_case(value: &str) -> String {
    value
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn make_item(view: DiscoveryAlbum, affinity: f32) -> Item {
    Item {
        artist_lc: view.artist.to_lowercase(),
        hay: format!("{} {}", view.artist, view.title).to_lowercase(),
        genres_lc: view.genres.iter().map(|g| g.to_lowercase()).collect(),
        affinity,
        view,
    }
}

/// Rebuilds the searchable index from the library and the discovery tables.
pub fn rebuild_index() {
    let Some(lib) = library::loaded() else { return };
    let mut items: Vec<Item> = Vec::with_capacity(lib.missing.len() + 2048);

    let max_albums = lib.artists.values().map(|a| a.albums).max().unwrap_or(1).max(1) as f32;
    for missing in &lib.missing {
        let Some(artist) = lib.artists.get(&missing.artist_key) else { continue };
        let affinity = ((1.0 + artist.albums as f32).ln() / (1.0 + max_albums).ln()) * 100.0;
        items.push(make_item(
            DiscoveryAlbum {
                id: missing.rgid.clone(),
                title: missing.title.clone(),
                artist: artist.name.clone(),
                artist_mbid: Some(missing.artist_mbid.clone()),
                year: missing.year,
                genres: if artist.genre.is_empty() { vec![] } else { vec![artist.genre.clone()] },
                similar: false,
                because: vec![],
                cover: caa(&missing.rgid),
            },
            affinity,
        ));
    }

    if let Ok(conn) = flip() {
        let sql = "SELECT a.cand_key, a.name, a.mbid, a.score, a.because, a.tags,
                          b.rgid, b.title, b.year, b.genres, b.cover
                   FROM disc_album b JOIN disc_artist a ON a.cand_key = b.cand_key";
        if let Ok(mut stmt) = conn.prepare(sql) {
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, Option<String>>(10)?,
                ))
            });
            if let Ok(rows) = rows {
                for (cand_key, name, mbid, score, because, tags, rgid, title, year, genres, cover) in rows.flatten() {
                    // The library may have grown since the crawl.
                    if lib.owns_artist(&cand_key) || lib.owns(&name, &title) {
                        continue;
                    }
                    let mut genre_list: Vec<String> = serde_json::from_str(&genres).unwrap_or_default();
                    if genre_list.is_empty() {
                        genre_list = serde_json::from_str(&tags).unwrap_or_default();
                    }
                    items.push(make_item(
                        DiscoveryAlbum {
                            cover: cover.filter(|c| !c.is_empty()).unwrap_or_else(|| caa(&rgid)),
                            id: rgid,
                            title,
                            artist: name,
                            artist_mbid: mbid,
                            year: year.map(|y| y as i32),
                            genres: genre_list,
                            similar: true,
                            because: serde_json::from_str(&because).unwrap_or_default(),
                        },
                        score as f32,
                    ));
                }
            }
        }
    }

    if let Ok(mut slot) = INDEX.write() {
        *slot = Some(Arc::new(Index { items }));
    }
}

fn index() -> Arc<Index> {
    INDEX.read().ok().and_then(|i| i.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/// Music Library's `scores` umbrella: film, TV, animation, anime and video-game score genres.
const SCORE_GENRE_GROUP: [&str; 13] = [
    "action", "animation", "comedy", "documentary", "drama", "fantasy", "horror", "sci fi", "thriller", "tv",
    "video game", "western", "anime",
];

/// Case, hyphen and spacing insensitive genre key ("Sci-Fi" == "sci fi").
fn genre_key(value: &str) -> String {
    value.to_lowercase().replace(['-', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The set of genre keys to hide, with the `scores` umbrella expanded.
fn expand_excluded(items: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for item in items {
        let key = genre_key(item);
        if key.is_empty() {
            continue;
        }
        if key == "score" || key == "scores" {
            out.extend(SCORE_GENRE_GROUP.iter().map(|g| (*g).to_owned()));
        } else {
            out.insert(key);
        }
    }
    out
}

fn matches(
    item: &Item,
    q: &DiscoveryQuery,
    skip_genre: bool,
    skip_year: bool,
    text: &[String],
    excluded: &HashSet<String>,
) -> bool {
    if !excluded.is_empty() && item.genres_lc.iter().any(|g| excluded.contains(&genre_key(g))) {
        return false;
    }
    if !skip_year {
        let year = item.view.year;
        if let Some(decade) = q.decade {
            if !year.is_some_and(|y| y / 10 * 10 == decade) {
                return false;
            }
        }
        if let Some(from) = q.year_from {
            if !year.is_some_and(|y| y >= from) {
                return false;
            }
        }
        if let Some(to) = q.year_to {
            if !year.is_some_and(|y| y <= to) {
                return false;
            }
        }
    }
    if !skip_genre {
        if let Some(genre) = q.genre.as_deref().filter(|g| !g.is_empty()) {
            let wanted = genre.to_lowercase();
            if !item.genres_lc.iter().any(|g| *g == wanted) {
                return false;
            }
        }
    }
    text.iter().all(|word| item.hay.contains(word.as_str()))
}

pub fn query(q: &DiscoveryQuery) -> DiscoveryPage {
    let index = index();
    let text: Vec<String> = q.text.to_lowercase().split_whitespace().map(str::to_owned).collect();
    let limit = if q.limit == 0 { 60 } else { q.limit.min(200) } as usize;
    let excluded = expand_excluded(&q.exclude_genres);

    // Genre facets ignore the genre filter, decade facets ignore the year filters, so you can switch.
    let mut genre_counts: HashMap<String, (String, u32)> = HashMap::new();
    let mut decade_counts: HashMap<i32, u32> = HashMap::new();
    let (mut missing_total, mut similar_total) = (0_u32, 0_u32);
    let mut hits: Vec<&Item> = Vec::new();

    for item in &index.items {
        let source_ok = match q.source.as_str() {
            "missing" => !item.view.similar,
            "similar" => item.view.similar,
            _ => true,
        };
        if source_ok {
            if matches(item, q, true, false, &text, &excluded) {
                for (lc, label) in item.genres_lc.iter().zip(&item.view.genres) {
                    genre_counts.entry(lc.clone()).or_insert_with(|| (label.clone(), 0)).1 += 1;
                }
            }
            if matches(item, q, false, true, &text, &excluded) {
                if let Some(year) = item.view.year {
                    *decade_counts.entry(year / 10 * 10).or_default() += 1;
                }
            }
        }
        if matches(item, q, false, false, &text, &excluded) {
            if item.view.similar {
                similar_total += 1;
            } else {
                missing_total += 1;
            }
            if source_ok {
                hits.push(item);
            }
        }
    }

    match q.sort.as_str() {
        "newest" => hits.sort_by(|a, b| b.view.year.cmp(&a.view.year).then_with(|| a.artist_lc.cmp(&b.artist_lc))),
        "oldest" => hits.sort_by(|a, b| {
            a.view.year.unwrap_or(9999).cmp(&b.view.year.unwrap_or(9999)).then_with(|| a.artist_lc.cmp(&b.artist_lc))
        }),
        "artist" => hits.sort_by(|a, b| a.artist_lc.cmp(&b.artist_lc).then_with(|| a.view.year.cmp(&b.view.year))),
        _ => {
            hits.sort_by(|a, b| {
                b.affinity
                    .partial_cmp(&a.affinity)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.artist_lc.cmp(&b.artist_lc))
                    .then_with(|| b.view.year.cmp(&a.view.year))
            });
            // Round-robin across artists: everyone's best album first, then everyone's second, ...
            // so one prolific artist cannot fill the whole first page.
            let mut seen: HashMap<&str, u32> = HashMap::new();
            let mut ranked: Vec<(u32, usize, &Item)> = hits
                .iter()
                .enumerate()
                .map(|(position, item)| {
                    let turn = seen.entry(item.artist_lc.as_str()).or_default();
                    let entry = (*turn, position, *item);
                    *turn += 1;
                    entry
                })
                .collect();
            ranked.sort_by_key(|(turn, position, _)| (*turn, *position));
            hits = ranked.into_iter().map(|(_, _, item)| item).collect();
        }
    }

    let total = hits.len() as u32;
    let items = hits.into_iter().skip(q.offset as usize).take(limit).map(|i| i.view.clone()).collect();

    let mut genres: Vec<Facet> = genre_counts
        .into_iter()
        .map(|(key, (label, count))| Facet { key, label, count })
        .collect();
    genres.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    genres.truncate(60);
    let mut decades: Vec<Facet> = decade_counts
        .into_iter()
        .map(|(decade, count)| Facet { key: decade.to_string(), label: format!("{decade}s"), count })
        .collect();
    decades.sort_by(|a, b| a.key.cmp(&b.key));

    DiscoveryPage { total, items, decades, genres, missing_total, similar_total }
}

// ---------------------------------------------------------------------------
// Status and progress
// ---------------------------------------------------------------------------

pub(crate) fn flip() -> Result<Connection> {
    let app = APP.get().context("not initialised")?;
    Ok(crate::db::open(app)?.0)
}

fn meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT v FROM disc_meta WHERE k = ?1", [key], |r| r.get(0)).ok()
}

fn set_meta(conn: &Connection, key: &str, value: &str) {
    let _ = conn.execute(
        "INSERT INTO disc_meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![key, value],
    );
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn age_days(timestamp: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_days())
        .unwrap_or(i64::MAX)
}

fn set_stats(change: impl FnOnce(&mut Stats)) {
    if let Ok(mut stats) = STATS.lock() {
        change(&mut stats);
    }
}

fn progress(stage: &str, done: u32, total: u32, message: impl Into<String>, running: bool) {
    let stats = STATS.lock().map(|s| *s).unwrap_or(Stats { seeds_done: 0, seeds_total: 0, artists_done: 0, artists_total: 0 });
    let p = DiscoveryProgress {
        running,
        stage: stage.to_owned(),
        done,
        total,
        message: message.into(),
        seeds_done: stats.seeds_done,
        seeds_total: stats.seeds_total,
        artists_done: stats.artists_done,
        artists_total: stats.artists_total,
        continuous: running && CONTINUOUS.load(Ordering::SeqCst),
    };
    if let Ok(mut slot) = PROGRESS.lock() {
        *slot = Some(p.clone());
    }
    if let Some(app) = APP.get() {
        let _ = crate::events::DiscoveryProgressChanged(p).emit(app);
    }
}

pub fn status() -> DiscoveryStatus {
    let conn = flip().ok();
    let mut current = PROGRESS.lock().ok().and_then(|p| p.clone()).unwrap_or_default();
    if let Ok(stats) = STATS.lock() {
        current.seeds_done = stats.seeds_done;
        current.seeds_total = stats.seeds_total;
        current.artists_done = stats.artists_done;
        current.artists_total = stats.artists_total;
    }
    if current.seeds_total == 0 && !RUNNING.load(Ordering::SeqCst) {
        if let (Some(lib), Some(conn)) = (library::loaded(), conn.as_ref()) {
            let mut covered: HashSet<String> = HashSet::new();
            if let Some(path) = library::paths().library {
                if let Ok(src) = library::open_ro(&path) {
                    if let Ok(mut stmt) = src.prepare("SELECT DISTINCT artist_key FROM lastfm_similar_artists") {
                        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                            covered.extend(rows.flatten().map(|k| loose_artist_key(&k)));
                        }
                    }
                }
            }
            if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT seed_key FROM disc_similar") {
                if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                    covered.extend(rows.flatten());
                }
            }
            let seeds = lib.artists.iter().filter(|(k, a)| a.albums >= 2 && k.as_str() != "various artists");
            let (mut total, mut done) = (0u32, 0u32);
            for (key, _) in seeds {
                total += 1;
                if covered.contains(key) {
                    done += 1;
                }
            }
            let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u32;
            set_stats(|s| {
                s.seeds_total = total;
                s.seeds_done = done;
                s.artists_done = count("SELECT COUNT(*) FROM disc_artist WHERE fetched_at IS NOT NULL");
                s.artists_total = count("SELECT COUNT(*) FROM disc_artist");
            });
            current.seeds_total = total;
            current.seeds_done = done;
            current.artists_done = STATS.lock().map(|s| s.artists_done).unwrap_or(0);
            current.artists_total = STATS.lock().map(|s| s.artists_total).unwrap_or(0);
        }
    }
    DiscoveryStatus {
        progress: current,
        library: library::status(),
        last_crawl: conn.as_ref().and_then(|c| meta(c, "last_crawl")),
        similar_artists: conn
            .as_ref()
            .and_then(|c| c.query_row("SELECT COUNT(*) FROM disc_artist WHERE fetched_at IS NOT NULL", [], |r| r.get::<_, i64>(0)).ok())
            .unwrap_or(0) as u32,
        lastfm_configured: keys::get("lastfm_key").is_some(),
    }
}

// ---------------------------------------------------------------------------
// Crawl
// ---------------------------------------------------------------------------

/// Starts the background crawl unless one is already running.
///
/// `continuous` keeps going round after round (each round asks Last.fm about up to 150 more of your
/// artists and looks up the albums of up to 300 new similar artists) until everything is checked or
/// [`stop_refresh`] is called. Last.fm's rate limit is enforced by the HTTP layer.
pub fn start_refresh(continuous: bool) -> bool {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return false;
    }
    STOP.store(false, Ordering::SeqCst);
    CONTINUOUS.store(continuous, Ordering::SeqCst);
    std::thread::spawn(move || {
        let mut errors = 0;
        let message = loop {
            let result = crawl();
            rebuild_index();
            match result {
                Ok(true) if continuous && !STOP.load(Ordering::SeqCst) => {
                    errors = 0;
                    continue;
                }
                Ok(_) if STOP.load(Ordering::SeqCst) => break "Stopped. Progress is saved; press Find more to continue.".to_owned(),
                Ok(true) => break "Checked another batch.".to_owned(),
                Ok(false) => break "Everything has been checked.".to_owned(),
                // A provider outage must not end a continuous run: wait and try the round again,
                // and only give up after several rounds in a row fail.
                Err(error) if continuous && errors < 5 => {
                    errors += 1;
                    progress(
                        "albums",
                        0,
                        0,
                        format!("A provider is not responding ({error:#}). Retrying in a minute ({errors}/5)..."),
                        true,
                    );
                    if !interruptible_sleep(Duration::from_secs(60)) {
                        break "Stopped. Progress is saved; press Find more to continue.".to_owned();
                    }
                }
                Err(error) => break format!("Discovery stopped: {error:#}"),
            }
        };
        CONTINUOUS.store(false, Ordering::SeqCst);
        progress("done", 0, 0, message, false);
        RUNNING.store(false, Ordering::SeqCst);
    });
    true
}

/// Sleeps in short slices so Stop stays responsive. Returns false when Stop was requested.
fn interruptible_sleep(total: Duration) -> bool {
    let end = std::time::Instant::now() + total;
    while std::time::Instant::now() < end {
        if STOP.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    !STOP.load(Ordering::SeqCst)
}

/// Asks a running crawl to stop after the artist it is on.
pub fn stop_refresh() {
    STOP.store(true, Ordering::SeqCst);
}

/// Called at launch: refresh in the background when the data is a week old or missing.
pub fn refresh_if_stale() {
    let stale = flip()
        .ok()
        .and_then(|c| meta(&c, "last_crawl"))
        .map(|t| age_days(&t) >= CRAWL_EVERY_DAYS)
        .unwrap_or(true);
    if stale && keys::get("lastfm_key").is_some() {
        start_refresh(false);
    }
}

struct Candidate {
    key: String,
    name: String,
    mbid: Option<String>,
    score: f64,
    because: Vec<(f64, String)>,
}

/// One pass. Returns whether it found anything left to do (false: everything is checked).
fn crawl() -> Result<bool> {
    progress("library", 0, 0, "Reading your library...", true);
    let mut waited = 0;
    let lib = loop {
        if let Some(lib) = library::loaded() {
            break lib;
        }
        if let Some(error) = library::last_error() {
            anyhow::bail!(error);
        }
        waited += 1;
        if waited > 240 {
            anyhow::bail!("Timed out waiting for the library to load");
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    rebuild_index();
    let conn = flip()?;

    // 1. Edges: artists in your library -> similar artists you don't have.
    let mut covered: HashSet<String> = HashSet::new();
    let mut edges: Vec<(String, String, Option<String>, f64)> = Vec::new(); // seed key, name, mbid, match

    if let Some(path) = library::paths().library {
        if let Ok(src) = library::open_ro(&path) {
            if let Ok(mut stmt) = src.prepare(
                "SELECT artist_key, similar_artist_name, similar_artist_mbid, match_score FROM lastfm_similar_artists",
            ) {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, f64>(3)?))
                }) {
                    for (seed, name, mbid, score) in rows.flatten() {
                        let seed = loose_artist_key(&seed);
                        covered.insert(seed.clone());
                        edges.push((seed, name, mbid, score));
                    }
                }
            }
        }
    }
    {
        let mut stmt = conn.prepare("SELECT seed_key, cand_name, cand_mbid, score, fetched_at FROM disc_similar")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, f64>(3)?, r.get::<_, String>(4)?))
        })?;
        for (seed, name, mbid, score, fetched) in rows.flatten() {
            if age_days(&fetched) < SIMILAR_STALE_DAYS {
                covered.insert(seed.clone());
                edges.push((seed, name, mbid, score));
            }
        }
    }

    let is_seed = |k: &String, a: &library::OwnedArtist| a.albums >= 2 && k != "various artists";
    let seeds_total = lib.artists.iter().filter(|(k, a)| is_seed(k, a)).count() as u32;
    let mut seeds_done = lib.artists.iter().filter(|(k, a)| is_seed(k, a) && covered.contains(*k)).count() as u32;
    set_stats(|s| {
        s.seeds_total = seeds_total;
        s.seeds_done = seeds_done;
    });
    let mut work = false;

    // Seeds your cache doesn't cover yet: your biggest artists first.
    if keys::get("lastfm_key").is_some() {
        let skipped: Vec<String> = SKIPPED.lock().map(|s| s.clone()).unwrap_or_default();
        let mut seeds: Vec<(&String, &library::OwnedArtist)> =
            lib.artists.iter().filter(|(k, a)| is_seed(k, a) && !covered.contains(*k) && !skipped.contains(*k)).collect();
        seeds.sort_by(|a, b| b.1.albums.cmp(&a.1.albums).then_with(|| a.0.cmp(b.0)));
        seeds.truncate(LIVE_SEED_LIMIT);
        let total = seeds.len() as u32;
        work |= total > 0;
        for (index, (key, artist)) in seeds.iter().enumerate() {
            if STOP.load(Ordering::SeqCst) {
                return Ok(true);
            }
            progress("similar", index as u32, total, format!("Asking Last.fm about {}", artist.name), true);
            let Some(response) = catalog::lastfm_json("artist.getSimilar", &[("artist", artist.name.as_str()), ("limit", "30")]) else {
                if let Ok(mut list) = SKIPPED.lock() {
                    list.push((*key).clone());
                }
                continue;
            };
            let list = response.pointer("/similarartists/artist").and_then(Value::as_array).cloned().unwrap_or_default();
            let tx = conn.unchecked_transaction()?;
            for entry in list {
                let name = entry.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
                if name.is_empty() {
                    continue;
                }
                let mbid = entry.get("mbid").and_then(Value::as_str).filter(|m| !m.is_empty()).map(str::to_owned);
                let score = entry.get("match").and_then(Value::as_str).and_then(|m| m.parse::<f64>().ok()).unwrap_or(0.0);
                tx.execute(
                    "INSERT INTO disc_similar (seed_key, cand_key, cand_name, cand_mbid, score, fetched_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(seed_key, cand_key) DO UPDATE SET cand_mbid = excluded.cand_mbid, score = excluded.score, fetched_at = excluded.fetched_at",
                    params![key, loose_artist_key(&name), name, mbid, score, now()],
                )?;
                edges.push(((*key).clone(), name, mbid, score));
            }
            // Marks the artist as asked, even when Last.fm had no similar artists for it.
            tx.execute(
                "INSERT INTO disc_similar (seed_key, cand_key, cand_name, cand_mbid, score, fetched_at)
                 VALUES (?1, '', '', NULL, 0, ?2)
                 ON CONFLICT(seed_key, cand_key) DO UPDATE SET fetched_at = excluded.fetched_at",
                params![key, now()],
            )?;
            tx.commit()?;
            seeds_done += 1;
            set_stats(|s| s.seeds_done = seeds_done);
        }
    }

    // 2. Rank candidates: strong matches to artists you own a lot of count most.
    let mut candidates: HashMap<String, Candidate> = HashMap::new();
    for (seed, name, mbid, score) in edges {
        let cand_key = loose_artist_key(&name);
        if cand_key.is_empty() || cand_key == "various artists" || cand_key == seed {
            continue;
        }
        if lib.owns_artist(&cand_key) || mbid.as_ref().is_some_and(|m| lib.mbid_to_key.contains_key(m)) {
            continue;
        }
        let Some(seed_artist) = lib.artists.get(&seed) else { continue };
        let weight = 1.0 + (1.0 + seed_artist.albums as f64).ln();
        let entry = candidates.entry(cand_key.clone()).or_insert_with(|| Candidate {
            key: cand_key,
            name: name.clone(),
            mbid: None,
            score: 0.0,
            because: Vec::new(),
        });
        entry.score += score * weight;
        entry.because.push((score * weight, seed_artist.name.clone()));
        if entry.mbid.is_none() {
            entry.mbid = mbid;
        }
    }
    let pool = candidates.len() as u32;
    let mut ranked: Vec<Candidate> = candidates.into_values().collect();
    ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.key.cmp(&b.key)));
    // Each pass looks at MAX_CANDIDATES more than the ones already explored.
    let explored = conn
        .query_row("SELECT COUNT(*) FROM disc_artist WHERE fetched_at IS NOT NULL", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as usize;
    ranked.truncate(explored + MAX_CANDIDATES);
    let mut artists_done = explored as u32;
    set_stats(|s| {
        s.artists_total = pool;
        s.artists_done = artists_done;
    });
    let top = ranked.first().map(|c| c.score).unwrap_or(1.0).max(f64::EPSILON);
    {
        let tx = conn.unchecked_transaction()?;
        for candidate in &mut ranked {
            candidate.because.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            let because: Vec<String> = candidate.because.iter().map(|(_, n)| n.clone()).take(3).collect();
            tx.execute(
                "INSERT INTO disc_artist (cand_key, name, mbid, score, because) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(cand_key) DO UPDATE SET name = excluded.name,
                    mbid = COALESCE(disc_artist.mbid, excluded.mbid), score = excluded.score, because = excluded.because",
                params![candidate.key, candidate.name, candidate.mbid, candidate.score / top * 100.0, serde_json::to_string(&because)?],
            )?;
        }
        tx.commit()?;
    }

    // 3. Albums for each candidate, best candidates first.
    let ranked_keys: HashSet<&str> = ranked.iter().map(|c| c.key.as_str()).collect();
    let skipped: Vec<String> = SKIPPED.lock().map(|s| s.clone()).unwrap_or_default();
    let todo: Vec<(String, String, Option<String>)> = {
        let mut stmt = conn.prepare("SELECT cand_key, name, mbid, fetched_at FROM disc_artist ORDER BY score DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?))
        })?;
        rows.flatten()
            .filter(|(key, _, _, fetched)| {
                ranked_keys.contains(key.as_str()) && !skipped.contains(key)
                    && fetched.as_deref().map(|t| age_days(t) >= ALBUMS_STALE_DAYS).unwrap_or(true)
            })
            .map(|(k, n, m, _)| (k, n, m))
            .collect()
    };
    let total = todo.len() as u32;
    work |= total > 0;
    let mut failures = 0;
    // Artists that failed in a row. They are only skipped for the session once a later artist
    // succeeds (a one-off bad artist); during an outage they stay eligible for the next round.
    let mut pending_skips: Vec<String> = Vec::new();
    for (index, (key, name, known_mbid)) in todo.into_iter().enumerate() {
        if STOP.load(Ordering::SeqCst) {
            rebuild_index();
            return Ok(true);
        }
        progress("albums", index as u32, total, format!("Finding albums by {name}"), true);
        match fetch_artist_albums(&conn, &lib, &key, &name, known_mbid) {
            Ok(()) => {
                failures = 0;
                if let Ok(mut list) = SKIPPED.lock() {
                    list.append(&mut pending_skips);
                }
                pending_skips.clear();
                artists_done += 1;
                set_stats(|s| s.artists_done = artists_done);
            }
            Err(error) => {
                pending_skips.push(key.clone());
                failures += 1;
                if failures >= 6 {
                    return Err(error.context("MusicBrainz keeps failing"));
                }
                // The shared HTTP gate opens a ~20s circuit breaker after repeated 429/503s, which
                // makes every request fail instantly. Wait it out instead of burning through artists.
                progress("albums", index as u32, total, "MusicBrainz is busy, waiting a moment...", true);
                if !interruptible_sleep(Duration::from_secs(25)) {
                    rebuild_index();
                    return Ok(true);
                }
            }
        }
        if index % 8 == 7 {
            rebuild_index();
        }
    }
    set_meta(&conn, "last_crawl", &now());
    Ok(work)
}

fn fetch_artist_albums(
    conn: &Connection,
    lib: &library::Library,
    key: &str,
    name: &str,
    known_mbid: Option<String>,
) -> Result<()> {
    let mbid = match known_mbid {
        Some(m) => Some(m),
        None => musicbrainz_artist(name)?,
    };
    // MusicBrainz says this is an artist you already own: nothing to discover.
    if mbid.as_ref().is_some_and(|m| lib.mbid_to_key.contains_key(m)) {
        conn.execute("UPDATE disc_artist SET mbid = ?2, fetched_at = ?3, source = 'owned' WHERE cand_key = ?1", params![key, mbid, now()])?;
        return Ok(());
    }

    let mut source = "musicbrainz";
    let mut albums = match &mbid {
        Some(mbid) => musicbrainz_studio_albums(mbid)?,
        None => Vec::new(),
    };
    if albums.is_empty() {
        let found = discogs_studio_albums(name, key);
        if !found.is_empty() {
            albums = found;
            source = "discogs";
        }
    }
    let mut tags: Vec<String> = Vec::new();
    if albums.iter().all(|a| a.genres.is_empty()) {
        tags = lastfm_tags(name);
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM disc_album WHERE cand_key = ?1", [key])?;
    let mut seen: HashSet<String> = HashSet::new();
    for album in albums {
        if !seen.insert(edition_title_key(&album.title)) {
            continue;
        }
        tx.execute(
            "INSERT OR REPLACE INTO disc_album (rgid, cand_key, title, year, genres, cover, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![album.id, key, album.title, album.year, serde_json::to_string(&album.genres)?, album.cover, source],
        )?;
    }
    tx.execute(
        "UPDATE disc_artist SET mbid = COALESCE(?2, mbid), tags = ?3, source = ?4, fetched_at = ?5 WHERE cand_key = ?1",
        params![key, mbid, serde_json::to_string(&tags)?, source, now()],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) struct FoundAlbum {
    pub id: String,
    pub title: String,
    pub year: Option<i32>,
    pub genres: Vec<String>,
    pub cover: Option<String>,
}

fn string_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub(crate) fn musicbrainz_artist(name: &str) -> Result<Option<String>> {
    let query = format!("artist:\"{}\"", name.replace('"', ""));
    let value = catalog::cached_get(
        "https://musicbrainz.org/ws/2/artist",
        &[("query", query.as_str()), ("limit", "3"), ("fmt", "json")],
    )?;
    let wanted = loose_artist_key(name);
    Ok(value
        .get("artists")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter().find(|a| {
                a.get("score").and_then(Value::as_i64).unwrap_or(0) >= 90 && loose_artist_key(string_at(a, "name")) == wanted
            })
        })
        .map(|a| string_at(a, "id").to_owned())
        .filter(|id| !id.is_empty()))
}

/// Studio albums only: primary type Album and *no* secondary types.
pub(crate) fn musicbrainz_studio_albums(mbid: &str) -> Result<Vec<FoundAlbum>> {
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let offset_text = offset.to_string();
        let value = catalog::cached_get(
            "https://musicbrainz.org/ws/2/release-group",
            &[
                ("artist", mbid),
                ("type", "album"),
                ("inc", "genres"),
                ("limit", "100"),
                ("offset", offset_text.as_str()),
                ("fmt", "json"),
            ],
        )?;
        let groups = value.get("release-groups").and_then(Value::as_array).cloned().unwrap_or_default();
        for group in &groups {
            let studio = string_at(group, "primary-type") == "Album"
                && group.get("secondary-types").and_then(Value::as_array).is_none_or(|s| s.is_empty());
            if !studio {
                continue;
            }
            let mut genres: Vec<(i64, String)> = group
                .get("genres")
                .and_then(Value::as_array)
                .map(|g| g.iter().map(|x| (x.get("count").and_then(Value::as_i64).unwrap_or(0), title_case(string_at(x, "name")))).collect())
                .unwrap_or_default();
            genres.sort_by(|a, b| b.0.cmp(&a.0));
            out.push(FoundAlbum {
                id: string_at(group, "id").to_owned(),
                title: string_at(group, "title").to_owned(),
                year: string_at(group, "first-release-date").get(..4).and_then(|y| y.parse().ok()).filter(|y| *y > 1900),
                genres: genres.into_iter().map(|(_, g)| g).filter(|g| !g.is_empty()).take(3).collect(),
                cover: None,
            });
        }
        let count = value.get("release-group-count").and_then(Value::as_u64).unwrap_or(0) as usize;
        offset += 100;
        if groups.is_empty() || offset >= count || offset >= 300 {
            break;
        }
    }
    out.retain(|a| !a.id.is_empty() && !a.title.is_empty() && !library::looks_like_compilation(&a.title));
    // MusicBrainz also lists bootlegs and unreleased material as plain "Album": keep only release
    // groups that have at least one official release (Music Library's cache applies the same rule).
    if !out.is_empty() {
        let official = musicbrainz_official_groups(mbid)?;
        out.retain(|a| official.contains(&a.id));
    }
    Ok(out)
}

/// Release-group ids of an artist's official album releases.
fn musicbrainz_official_groups(mbid: &str) -> Result<HashSet<String>> {
    let mut groups = HashSet::new();
    let mut offset = 0;
    loop {
        let offset_text = offset.to_string();
        let value = catalog::cached_get(
            "https://musicbrainz.org/ws/2/release",
            &[
                ("artist", mbid),
                ("status", "official"),
                ("type", "album"),
                ("inc", "release-groups"),
                ("limit", "100"),
                ("offset", offset_text.as_str()),
                ("fmt", "json"),
            ],
        )?;
        let releases = value.get("releases").and_then(Value::as_array).cloned().unwrap_or_default();
        for release in &releases {
            if let Some(id) = release.pointer("/release-group/id").and_then(Value::as_str) {
                groups.insert(id.to_owned());
            }
        }
        let count = value.get("release-count").and_then(Value::as_u64).unwrap_or(0) as usize;
        offset += 100;
        if releases.is_empty() || offset >= count || offset >= 600 {
            break;
        }
    }
    Ok(groups)
}

/// Discogs has no album type, so obvious compilations and live records are filtered by title.
fn discogs_studio_albums(name: &str, key: &str) -> Vec<FoundAlbum> {
    let (Some(consumer_key), Some(secret)) = (keys::get("discogs_key"), keys::get("discogs_secret")) else {
        return Vec::new();
    };
    let Ok(value) = catalog::cached_get(
        "https://api.discogs.com/database/search",
        &[
            ("type", "master"),
            ("artist", name),
            ("per_page", "60"),
            ("key", consumer_key.as_str()),
            ("secret", secret.as_str()),
        ],
    ) else {
        return Vec::new();
    };
    const NOT_STUDIO: [&str; 14] = [
        "greatest hits", "best of", "live", "collection", "anthology", "compilation", "remix", "unplugged",
        "sessions", "essential", "soundtrack", "hits", "singles", "demos",
    ];
    value
        .get("results")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|r| {
                    let (artist, title) = string_at(r, "title").split_once(" - ")?;
                    if loose_artist_key(artist) != key {
                        return None;
                    }
                    let lower = loose_key(title);
                    if NOT_STUDIO.iter().any(|w| lower.split(' ').any(|word| word == *w) || lower.contains(&format!("{w} "))) {
                        return None;
                    }
                    let strings = |k: &str| -> Vec<String> {
                        r.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(title_case)).collect()).unwrap_or_default()
                    };
                    let mut genres = strings("style");
                    genres.extend(strings("genre"));
                    genres.truncate(3);
                    let id = r.get("master_id").and_then(Value::as_i64).or_else(|| r.get("id").and_then(Value::as_i64))?;
                    Some(FoundAlbum {
                        id: format!("discogs:{id}"),
                        title: title.trim().to_owned(),
                        year: string_at(r, "year").parse().ok().filter(|y| *y > 1900),
                        genres,
                        cover: Some(string_at(r, "cover_image").to_owned()).filter(|c| !c.is_empty() && !c.contains("spacer.gif")),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn lastfm_tags(name: &str) -> Vec<String> {
    const NOISE: [&str; 8] = ["seen live", "favorites", "favourites", "favorite", "albums i own", "male vocalists", "female vocalists", "all"];
    catalog::lastfm_json("artist.getTopTags", &[("artist", name)])
        .and_then(|v| v.pointer("/toptags/tag").and_then(Value::as_array).cloned())
        .map(|tags| {
            tags.iter()
                .map(|t| string_at(t, "name").to_lowercase())
                .filter(|t| !t.is_empty() && !NOISE.contains(&t.as_str()) && t.parse::<u32>().is_err())
                .take(3)
                .map(|t| title_case(&t))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(genres: &[&str]) -> Item {
        make_item(
            DiscoveryAlbum {
                id: "x".into(), title: "T".into(), artist: "A".into(), artist_mbid: None, year: Some(1999),
                genres: genres.iter().map(|g| (*g).to_owned()).collect(), similar: false, because: vec![], cover: String::new(),
            },
            1.0,
        )
    }

    #[test]
    fn scores_umbrella_and_plain_genres_are_excluded() {
        let excluded = expand_excluded(&["scores".to_owned(), "Soundtrack".to_owned(), " synthwave ".to_owned()]);
        let q = DiscoveryQuery::default();
        let allowed = |genres: &[&str]| matches(&item(genres), &q, false, false, &[], &excluded);
        assert!(!allowed(&["TV"]));
        assert!(!allowed(&["Video Game"]));
        assert!(!allowed(&["Sci-Fi"]), "hyphen and case must not matter");
        assert!(!allowed(&["Soundtrack"]));
        assert!(!allowed(&["Synthwave"]));
        assert!(!allowed(&["Rock", "Drama"]), "any excluded genre hides the album");
        assert!(allowed(&["Rock"]));
        assert!(allowed(&[]), "albums without a genre are not excluded");
        assert!(excluded.contains("thriller") && excluded.contains("anime"));
    }

    #[test]
    fn title_case_keeps_inner_caps() {
        assert_eq!(title_case("synth-pop"), "Synth-pop");
        assert_eq!(title_case("hard rock"), "Hard Rock");
    }

    /// Live network smoke test: `cargo test --lib live_crawl_pieces -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_crawl_pieces() {
        // Daft Punk: studio albums only, so no "Alive 1997", "Musique Vol. 1", "Human After All (remixes)" etc.
        let albums = musicbrainz_studio_albums("056e4f3e-d505-4dad-8ec1-d04f521cbb56").expect("musicbrainz");
        let titles: Vec<_> = albums.iter().map(|a| format!("{} ({:?}) {:?}", a.title, a.year, a.genres)).collect();
        println!("daft punk studio albums: {titles:#?}");
        assert!(albums.iter().any(|a| a.title == "Discovery"));
        assert!(!albums.iter().any(|a| a.title.to_lowercase().contains("alive")), "live albums must be excluded");
        assert!(albums.len() <= 5, "only official studio albums expected, got {}", albums.len());

        let found = musicbrainz_artist("Kip Winger").expect("artist search");
        println!("kip winger mbid: {found:?}");
        assert!(found.is_some());

        let discogs = discogs_studio_albums("Kip Winger", &loose_artist_key("Kip Winger"));
        println!("discogs kip winger: {:?}", discogs.iter().map(|a| (&a.title, a.year, &a.genres)).collect::<Vec<_>>());

        let tags = lastfm_tags("Kip Winger");
        println!("lastfm tags: {tags:?}");
        let similar = catalog::lastfm_json("artist.getSimilar", &[("artist", "Kip Winger"), ("limit", "5")]);
        println!("similar ok: {}", similar.is_some());
        assert!(similar.is_some(), "Last.fm key should be configured");
    }
}

/// MusicBrainz id of a candidate artist the crawl already resolved.
pub(crate) fn known_mbid(cand_key: &str) -> Option<String> {
    flip()
        .ok()?
        .query_row("SELECT mbid FROM disc_artist WHERE cand_key = ?1 AND mbid IS NOT NULL", [cand_key], |r| r.get(0))
        .ok()
}
