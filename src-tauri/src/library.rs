//! Your music library, as Flipside sees it.
//!
//! Reads the Music Library SQLite database (what you own), the MusicBrainz cache and the
//! newer MusicBrainz overlay, all strictly read-only, into an in-memory index:
//!
//! * which albums you already own (so they are hidden everywhere in the app), and
//! * which **studio albums** (MusicBrainz primary type "Album" with no secondary types:
//!   no live, compilation, soundtrack, remix, EP or single) are missing from artists you own.
//!
//! Names are compared with the same identity rules Music Library uses for its Wish List,
//! so "The Beatles" / "Beatles" and "Discovery (Deluxe Edition)" / "Discovery" line up.

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Manager};

use crate::identity::{credit_keys, edition_title_key, loose_artist_key};

#[derive(Clone, Debug, Default)]
pub struct Paths {
    pub library: Option<PathBuf>,
    pub mb_cache: Option<PathBuf>,
    pub overlay: Option<PathBuf>,
    /// Flipside's own database (download receipts count as "owned" too).
    pub receipts: Option<PathBuf>,
}

pub struct OwnedArtist {
    pub name: String,
    pub albums: u32,
    /// Most common genre among this artist's albums in the library.
    pub genre: String,
}

pub struct MissingAlbum {
    pub rgid: String,
    pub artist_key: String,
    pub artist_mbid: String,
    pub title: String,
    pub year: Option<i32>,
}

#[derive(Default)]
pub struct Library {
    owned_albums: HashSet<(String, String)>,
    pub artists: HashMap<String, OwnedArtist>,
    /// MusicBrainz id -> owned artist key.
    pub mbid_to_key: HashMap<String, String>,
    /// Owned artist key -> MusicBrainz id.
    pub key_to_mbid: HashMap<String, String>,
    /// Release groups that are out of scope or have no official release.
    pub excluded: HashSet<String>,
    pub missing: Vec<MissingAlbum>,
    pub owned_album_count: usize,
    pub loaded_at: Option<SystemTime>,
}

impl Library {
    pub fn owns(&self, artist: &str, title: &str) -> bool {
        let title_key = edition_title_key(title);
        let mut keys = credit_keys(artist);
        keys.push(loose_artist_key(artist));
        keys.iter().any(|key| self.owned_albums.contains(&(key.clone(), title_key.clone())))
    }

    pub fn owns_artist(&self, artist_key: &str) -> bool {
        self.artists.contains_key(artist_key)
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static PATHS: RwLock<Option<Paths>> = RwLock::new(None);
static HIDE_OWNED: AtomicBool = AtomicBool::new(true);
static LIBRARY: RwLock<Option<Arc<Library>>> = RwLock::new(None);
static LOADING: AtomicBool = AtomicBool::new(false);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static LAST_STAMP_CHECK: Mutex<Option<(Instant, Option<SystemTime>)>> = Mutex::new(None);

/// MusicBrainz often leaves "best of" records untagged, so they look like studio albums. Unmistakable
/// compilation titles are dropped as well; the phrases are specific enough not to hit real album names.
pub(crate) fn looks_like_compilation(title: &str) -> bool {
    const PHRASES: [&str; 13] = [
        "greatest hits", "the best of", "very best", "best of the", "anthology", "the collection", "the essential",
        "essential collection", "ultimate collection", "complete collection", "hit singles", "retrospective",
        "all time greatest",
    ];
    let lower = title.to_lowercase();
    lower.starts_with("best of ") || PHRASES.iter().any(|phrase| lower.contains(phrase))
}

pub fn hide_owned() -> bool {
    HIDE_OWNED.load(Ordering::Relaxed)
}

pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().ok().and_then(|e| e.clone())
}

/// The loaded library, if any. Never blocks on a load; schedules a refresh when the files changed.
pub fn loaded() -> Option<Arc<Library>> {
    maybe_refresh();
    LIBRARY.read().ok().and_then(|l| l.clone())
}

/// Whether an album is owned and should be hidden (honours the "hide owned" setting).
pub fn should_hide(artist: &str, title: &str) -> bool {
    hide_owned() && loaded().is_some_and(|lib| lib.owns(artist, title))
}

fn resolve_paths(app: &AppHandle, settings: &crate::db::AppSettings) -> Paths {
    let library = if settings.library_db_path.is_empty() {
        app.path()
            .config_dir()
            .ok()
            .map(|dir| dir.join("com.local.musiclibrary").join("music-library.sqlite3"))
    } else {
        Some(PathBuf::from(&settings.library_db_path))
    }
    .filter(|p| p.is_file());

    // Music Library knows where its own MusicBrainz files are; reuse that unless overridden.
    let configured = |column: &str| -> Option<PathBuf> {
        let conn = open_ro(library.as_deref()?).ok()?;
        conn.query_row(&format!("SELECT {column} FROM app_settings LIMIT 1"), [], |r| r.get::<_, String>(0))
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
    };
    let pick = |explicit: &str, column: &str| -> Option<PathBuf> {
        let path = if explicit.is_empty() { configured(column)? } else { PathBuf::from(explicit) };
        path.is_file().then_some(path)
    };
    Paths {
        mb_cache: pick(&settings.musicbrainz_cache_path, "musicbrainz_cache_path"),
        overlay: pick(&settings.musicbrainz_overlay_path, "musicbrainz_overlay_sync_path"),
        receipts: crate::db::database_path(app).ok().filter(|p| p.is_file()),
        library,
    }
}

/// Applies the settings. Reloads the library when paths changed.
pub fn configure(app: &AppHandle, settings: &crate::db::AppSettings) {
    HIDE_OWNED.store(settings.hide_owned, Ordering::Relaxed);
    let paths = resolve_paths(app, settings);
    let changed = PATHS
        .read()
        .ok()
        .and_then(|p| p.clone())
        .map(|old| {
            old.library != paths.library || old.mb_cache != paths.mb_cache || old.overlay != paths.overlay
        })
        .unwrap_or(true);
    if let Ok(mut slot) = PATHS.write() {
        *slot = Some(paths);
    }
    if changed {
        reload_in_background();
    }
}

pub fn paths() -> Paths {
    PATHS.read().ok().and_then(|p| p.clone()).unwrap_or_default()
}

fn stamp(path: &Path) -> Option<SystemTime> {
    let main = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let wal = std::fs::metadata(format!("{}-wal", path.display())).and_then(|m| m.modified()).ok();
    main.max(wal)
}

/// Music Library keeps writing to its database; pick up changes at most once a minute.
fn maybe_refresh() {
    let Some(path) = paths().library else { return };
    let now = Instant::now();
    let Ok(mut check) = LAST_STAMP_CHECK.lock() else { return };
    if check.as_ref().is_some_and(|(at, _)| now.duration_since(*at) < Duration::from_secs(60)) {
        return;
    }
    let current = stamp(&path);
    let previous = check.as_ref().and_then(|(_, s)| *s);
    *check = Some((now, current));
    drop(check);
    let loaded = LIBRARY.read().map(|l| l.is_some()).unwrap_or(false);
    if loaded && previous.is_some() && current != previous {
        reload_in_background();
    }
}

pub fn reload_in_background() {
    if LOADING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let result = load(&paths());
        match result {
            Ok(library) => {
                if let Ok(mut slot) = LIBRARY.write() {
                    *slot = Some(Arc::new(library));
                }
                if let Ok(mut error) = LAST_ERROR.lock() {
                    *error = None;
                }
                crate::discovery::rebuild_index();
            }
            Err(error) => {
                if let Ok(mut slot) = LAST_ERROR.lock() {
                    *slot = Some(format!("{error:#}"));
                }
            }
        }
        LOADING.store(false, Ordering::SeqCst);
    });
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

pub(crate) fn open_ro(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("Could not open {} read-only", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn pairs(conn: &Connection, sql: &str) -> Vec<(String, String)> {
    let Ok(mut stmt) = conn.prepare(sql) else { return Vec::new() };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))) else {
        return Vec::new();
    };
    rows.flatten().filter_map(|(a, b)| Some((a?, b?))).collect()
}

fn strings(conn: &Connection, sql: &str) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(sql) else { return Vec::new() };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, Option<String>>(0)) else { return Vec::new() };
    rows.flatten().flatten().collect()
}

struct ReleaseGroupRow {
    artist_mbid: String,
    rgid: String,
    title: String,
    year: Option<i32>,
}

/// Studio albums only: primary type Album, no secondary types, officially released.
fn studio_albums(conn: &Connection, table: &str) -> Vec<ReleaseGroupRow> {
    let sql = format!(
        "SELECT artist_mbid, release_mbid, title, year FROM {table}
         WHERE type = 'Album' AND (secondary_types IS NULL OR secondary_types = '')
           AND (status IS NULL OR status = 'Official')"
    );
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<i64>>(3)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|(artist_mbid, rgid, title, year)| {
            Some(ReleaseGroupRow {
                artist_mbid: artist_mbid?,
                rgid: rgid?,
                title: title?,
                year: year.map(|y| y as i32).filter(|y| *y > 1900),
            })
        })
        .collect()
}

pub fn load(paths: &Paths) -> Result<Library> {
    let library_path = paths.library.as_ref().context(
        "Music Library's database was not found. Set its location in Settings > Your library.",
    )?;
    let lib = open_ro(library_path)?;
    let mut out = Library::default();

    // 1. What you own.
    let mut genre_counts: HashMap<String, HashMap<String, u32>> = HashMap::new();
    {
        let mut stmt = lib.prepare("SELECT album_artist_display, album, canonical_genre FROM albums")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))
        })?;
        for (artist, album, genre) in rows.flatten() {
            let (Some(artist), Some(album)) = (artist, album) else { continue };
            out.owned_album_count += 1;
            let title_key = edition_title_key(&album);
            let primary = loose_artist_key(&artist);
            let mut keys = credit_keys(&artist);
            if !keys.contains(&primary) {
                keys.push(primary.clone());
            }
            for key in keys {
                out.owned_albums.insert((key, title_key.clone()));
            }
            let entry = out.artists.entry(primary.clone()).or_insert_with(|| OwnedArtist {
                name: artist.clone(),
                albums: 0,
                genre: String::new(),
            });
            entry.albums += 1;
            if let Some(genre) = genre.filter(|g| !g.is_empty()) {
                *genre_counts.entry(primary).or_default().entry(genre).or_default() += 1;
            }
        }
    }
    for (key, counts) in genre_counts {
        if let (Some(artist), Some((genre, _))) = (out.artists.get_mut(&key), counts.into_iter().max_by_key(|(_, n)| *n)) {
            artist.genre = genre;
        }
    }
    // Albums downloaded but not imported yet still count as owned.
    for (artist, album) in pairs(&lib, "SELECT artist, album FROM deemix_downloads") {
        add_owned(&mut out, &artist, &album);
    }
    if let Some(receipts) = paths.receipts.as_ref().and_then(|p| open_ro(p).ok()) {
        for (artist, album) in pairs(&receipts, "SELECT artist, album FROM deemix_downloads") {
            add_owned(&mut out, &artist, &album);
        }
    }

    // 2. MusicBrainz ids for owned artists. Newest source wins: overlay, manual links, cached infos, name cache.
    let overlay = paths.overlay.as_ref().and_then(|p| open_ro(p).ok());
    let mb_cache = paths.mb_cache.as_ref().and_then(|p| open_ro(p).ok());
    let mut artist_mbid: HashMap<String, String> = HashMap::new();
    let mut remember = |name: &str, mbid: String| {
        artist_mbid.entry(loose_artist_key(name)).or_insert(mbid);
    };
    if let Some(overlay) = &overlay {
        for (name, mbid) in pairs(overlay, "SELECT display_artist, mbid FROM musicbrainz_artist_links WHERE ignored = 0") {
            remember(&name, mbid);
        }
    }
    for (name, mbid) in pairs(&lib, "SELECT display_artist, mbid FROM musicbrainz_artist_links WHERE ignored = 0") {
        remember(&name, mbid);
    }
    for (name, mbid) in pairs(&lib, "SELECT display_artist, mbid FROM musicbrainz_artist_infos WHERE mbid IS NOT NULL") {
        remember(&name, mbid);
    }
    if let Some(cache) = &mb_cache {
        for (name, mbid) in pairs(cache, "SELECT name, mbid FROM artist_cache WHERE mbid IS NOT NULL") {
            remember(&name, mbid);
        }
    }
    for (key, mbid) in artist_mbid {
        if out.artists.contains_key(&key) {
            out.mbid_to_key.insert(mbid.clone(), key.clone());
            out.key_to_mbid.insert(key, mbid);
        }
    }

    // 3. Things the user (or MusicBrainz) says are out of scope.
    let mut excluded: HashSet<String> = HashSet::new();
    for conn in std::iter::once(&lib).chain(overlay.iter()) {
        excluded.extend(strings(conn, "SELECT release_mbid FROM musicbrainz_release_decisions WHERE decision = 'not-in-scope'"));
        excluded.extend(strings(conn, "SELECT release_mbid FROM musicbrainz_release_status_cache WHERE has_official_release = 0"));
    }

    // 4. Studio albums missing from owned artists. Overlay is newest, then Music Library, then the big cache.
    let mut seen_groups: HashSet<String> = HashSet::new();
    let mut seen_titles: HashSet<(String, String)> = HashSet::new();
    let mut take = |rows: Vec<ReleaseGroupRow>, out: &mut Library| {
        for row in rows {
            let Some(artist_key) = out.mbid_to_key.get(&row.artist_mbid).cloned() else { continue };
            if excluded.contains(&row.rgid) || looks_like_compilation(&row.title) || !seen_groups.insert(row.rgid.clone()) {
                continue;
            }
            let title_key = edition_title_key(&row.title);
            let owned = out.artists.get(&artist_key).is_some_and(|a| out.owns(&a.name, &row.title));
            if owned || !seen_titles.insert((artist_key.clone(), title_key)) {
                continue;
            }
            out.missing.push(MissingAlbum {
                rgid: row.rgid,
                artist_key,
                artist_mbid: row.artist_mbid,
                title: row.title,
                year: row.year,
            });
        }
    };
    if let Some(overlay) = &overlay {
        take(studio_albums(overlay, "musicbrainz_artist_release_groups"), &mut out);
    }
    take(studio_albums(&lib, "musicbrainz_artist_release_groups"), &mut out);
    if let Some(cache) = &mb_cache {
        take(studio_albums(cache, "release_groups"), &mut out);
    }

    out.excluded = excluded;
    out.loaded_at = Some(SystemTime::now());
    Ok(out)
}

/// Release groups the local MusicBrainz data (overlay, Music Library, cache) holds for one artist:
/// `(release group id, title, year)`. With `studio_only`, only official studio albums are returned.
pub fn local_albums(mbid: &str, studio_only: bool) -> Vec<(String, String, Option<i32>)> {
    let paths = paths();
    let excluded = loaded().map(|l| l.excluded.clone()).unwrap_or_default();
    let filter = if studio_only {
        " AND type = 'Album' AND (secondary_types IS NULL OR secondary_types = '') AND (status IS NULL OR status = 'Official')"
    } else {
        ""
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (path, table) in [
        (paths.overlay, "musicbrainz_artist_release_groups"),
        (paths.library, "musicbrainz_artist_release_groups"),
        (paths.mb_cache, "release_groups"),
    ] {
        let Some(conn) = path.and_then(|p| open_ro(&p).ok()) else { continue };
        let sql = format!("SELECT release_mbid, title, year FROM {table} WHERE artist_mbid = ?1{filter}");
        let Ok(mut stmt) = conn.prepare(&sql) else { continue };
        let Ok(rows) = stmt.query_map([mbid], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?))
        }) else {
            continue;
        };
        for (rgid, title, year) in rows.flatten() {
            let (Some(rgid), Some(title)) = (rgid, title) else { continue };
            if (studio_only && (excluded.contains(&rgid) || looks_like_compilation(&title))) || !seen.insert(rgid.clone()) {
                continue;
            }
            out.push((rgid, title, year.map(|y| y as i32).filter(|y| *y > 1900)));
        }
    }
    out
}


fn add_owned(out: &mut Library, artist: &str, album: &str) {
    let title_key = edition_title_key(album);
    let mut keys = credit_keys(artist);
    keys.push(loose_artist_key(artist));
    for key in keys {
        out.owned_albums.insert((key, title_key.clone()));
    }
}

/// A small summary for Settings.
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatus {
    pub library_path: Option<String>,
    pub musicbrainz_cache_path: Option<String>,
    pub musicbrainz_overlay_path: Option<String>,
    pub loaded: bool,
    pub loading: bool,
    pub owned_albums: u32,
    pub owned_artists: u32,
    pub missing_albums: u32,
    pub error: Option<String>,
}

pub fn status() -> LibraryStatus {
    let paths = paths();
    let library = LIBRARY.read().ok().and_then(|l| l.clone());
    let show = |p: &Option<PathBuf>| p.as_ref().map(|p| p.to_string_lossy().into_owned());
    LibraryStatus {
        library_path: show(&paths.library),
        musicbrainz_cache_path: show(&paths.mb_cache),
        musicbrainz_overlay_path: show(&paths.overlay),
        loaded: library.is_some(),
        loading: LOADING.load(Ordering::SeqCst),
        owned_albums: library.as_ref().map(|l| l.owned_album_count as u32).unwrap_or(0),
        owned_artists: library.as_ref().map(|l| l.artists.len() as u32).unwrap_or(0),
        missing_albums: library.as_ref().map(|l| l.missing.len() as u32).unwrap_or(0),
        error: last_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compilation_titles_are_recognised() {
        for title in ["The Very Best of Janie Fricke", "Greatest Hits", "Best of Both Worlds", "Donna Fargo: All-Time Greatest Hits", "The Essential Dolly Parton", "Anthology"] {
            assert!(looks_like_compilation(title), "{title}");
        }
        for title in ["Discovery", "Bestial", "Homework", "Live and Let Die", "The Happiest Girl in the Whole U.S.A.", "Rest of Us"] {
            assert!(!looks_like_compilation(title), "{title}");
        }
    }

    #[test]
    fn owned_lookup_ignores_editions_and_the() {
        let mut library = Library::default();
        add_owned(&mut library, "The Beatles", "Abbey Road (Remastered)");
        assert!(library.owns("Beatles", "Abbey Road"));
        assert!(library.owns("The Beatles", "Abbey Road (Deluxe Edition)"));
        assert!(!library.owns("The Beatles", "Let It Be"));
    }

    /// Reads the real databases on this machine: `cargo test --lib live_library -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_library_index() {
        let appdata = std::env::var("APPDATA").expect("APPDATA");
        let library_path = PathBuf::from(appdata).join("com.local.musiclibrary").join("music-library.sqlite3");
        let conn = open_ro(&library_path).expect("library db");
        let configured = |column: &str| -> Option<PathBuf> {
            conn.query_row(&format!("SELECT {column} FROM app_settings LIMIT 1"), [], |r| r.get::<_, String>(0)).ok().map(PathBuf::from)
        };
        let paths = Paths {
            mb_cache: configured("musicbrainz_cache_path"),
            overlay: configured("musicbrainz_overlay_sync_path"),
            library: Some(library_path),
            receipts: None,
        };
        println!("paths: {paths:?}");
        let started = Instant::now();
        let lib = load(&paths).expect("load");
        println!(
            "loaded in {:?}: owned albums={} artists={} mbids={} missing studio albums={}",
            started.elapsed(), lib.owned_album_count, lib.artists.len(), lib.mbid_to_key.len(), lib.missing.len()
        );
        for m in lib.missing.iter().step_by(lib.missing.len().max(40) / 20).take(15) {
            let artist = &lib.artists[&m.artist_key];
            println!("  {} - {} ({:?}) [owns {} albums by them]", artist.name, m.title, m.year, artist.albums);
        }
        assert!(lib.owned_album_count > 1000);
        assert!(!lib.missing.is_empty());
        // Nothing missing may be something we own.
        assert!(lib.missing.iter().all(|m| !lib.owns(&lib.artists[&m.artist_key].name, &m.title)));
    }
}
