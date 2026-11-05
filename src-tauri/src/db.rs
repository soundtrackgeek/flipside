//! Small persistence layer: a JSON settings file plus a SQLite file that holds
//! Deemix download receipts (so an album is never downloaded twice by accident) and
//! the discovery index (similar artists and the studio albums found for them).

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub deemix_download_path: String,
    pub deemix_download_quality: String,
    pub deemix_download_fallback: bool,
    pub deemix_download_organization: String,
    pub auto_update_check: bool,
    /// Music Library database (what you own). Empty means "auto-detect".
    pub library_db_path: String,
    /// MusicBrainz cache database. Empty means "use the path Music Library is configured with".
    pub musicbrainz_cache_path: String,
    /// MusicBrainz overlay (newer data synced through OneDrive). Empty means "use Music Library's".
    pub musicbrainz_overlay_path: String,
    /// Never show albums that are already in the library.
    pub hide_owned: bool,
    /// Genres hidden on the For you page. "scores" expands to Music Library's film/TV/game umbrella.
    pub excluded_genres: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            deemix_download_path: String::new(),
            deemix_download_quality: "mp3_320".to_owned(),
            deemix_download_fallback: true,
            deemix_download_organization: "flat_artist_album_year".to_owned(),
            auto_update_check: true,
            library_db_path: String::new(),
            musicbrainz_cache_path: String::new(),
            musicbrainz_overlay_path: String::new(),
            hide_owned: true,
            excluded_genres: Vec::new(),
        }
    }
}

impl AppSettings {
    fn normalized(mut self, app: &AppHandle) -> Self {
        self.deemix_download_path = self.deemix_download_path.trim().to_owned();
        if self.deemix_download_path.is_empty() {
            self.deemix_download_path = downloads_dir(app).join("_Deemix").to_string_lossy().into_owned();
        }
        if !matches!(self.deemix_download_quality.as_str(), "mp3_128" | "mp3_320" | "flac") {
            self.deemix_download_quality = "mp3_320".to_owned();
        }
        if !matches!(
            self.deemix_download_organization.as_str(),
            "flat_artist_album_year" | "artist_album_year_folders"
        ) {
            self.deemix_download_organization = "flat_artist_album_year".to_owned();
        }
        self.library_db_path = self.library_db_path.trim().to_owned();
        self.musicbrainz_cache_path = self.musicbrainz_cache_path.trim().to_owned();
        self.musicbrainz_overlay_path = self.musicbrainz_overlay_path.trim().to_owned();
        let mut seen = std::collections::HashSet::new();
        self.excluded_genres = self
            .excluded_genres
            .iter()
            .map(|g| g.trim().to_owned())
            .filter(|g| !g.is_empty() && seen.insert(g.to_lowercase()))
            .collect();
        self
    }
}

fn downloads_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn settings_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app.path().app_config_dir().context("no config directory")?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("settings.json"))
}

pub fn settings_for_app(app: &AppHandle) -> Result<AppSettings> {
    let path = settings_path(app)?;
    let stored: AppSettings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    Ok(stored.normalized(app))
}

pub fn save_settings_for_app(app: &AppHandle, settings: AppSettings) -> Result<AppSettings> {
    let settings = settings.normalized(app);
    std::fs::write(settings_path(app)?, serde_json::to_string_pretty(&settings)?)
        .context("Could not save settings")?;
    Ok(settings)
}

/// Where the receipts/discovery database lives.
pub fn database_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app.path().app_data_dir().context("no data directory")?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("flipside.sqlite3"))
}

/// Opens (and migrates) the receipts database.
pub fn open(app: &AppHandle) -> Result<(Connection, PathBuf)> {
    let path = database_path(app)?;
    let conn = Connection::open(&path).context("Could not open the Flipside database")?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok((conn, path))
}

pub fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS deemix_downloads (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            deezer_album_id TEXT NOT NULL,
            wish_list_item_id INTEGER,
            musicbrainz_release_group_id TEXT,
            artist TEXT NOT NULL,
            album TEXT NOT NULL,
            year INTEGER,
            quality TEXT NOT NULL,
            destination_path TEXT NOT NULL,
            cover_path TEXT,
            track_count INTEGER NOT NULL DEFAULT 0,
            completed_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'download',
            UNIQUE(deezer_album_id, destination_path)
        );
        CREATE INDEX IF NOT EXISTS idx_deemix_downloads_deezer_album
            ON deemix_downloads(deezer_album_id, completed_at DESC);

        -- Last.fm 'similar to' edges from artists in the library (seed) to artists outside it.
        CREATE TABLE IF NOT EXISTS disc_similar (
            seed_key TEXT NOT NULL,
            cand_key TEXT NOT NULL,
            cand_name TEXT NOT NULL,
            cand_mbid TEXT,
            score REAL NOT NULL,
            fetched_at TEXT NOT NULL,
            PRIMARY KEY (seed_key, cand_key)
        );
        -- Candidate artists, ranked by how strongly the library points at them.
        CREATE TABLE IF NOT EXISTS disc_artist (
            cand_key TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            mbid TEXT,
            score REAL NOT NULL DEFAULT 0,
            because TEXT NOT NULL DEFAULT '[]',
            tags TEXT NOT NULL DEFAULT '[]',
            source TEXT,
            fetched_at TEXT
        );
        -- Studio albums found for those artists (MusicBrainz first, Discogs as a fallback).
        CREATE TABLE IF NOT EXISTS disc_album (
            rgid TEXT PRIMARY KEY,
            cand_key TEXT NOT NULL,
            title TEXT NOT NULL,
            year INTEGER,
            genres TEXT NOT NULL DEFAULT '[]',
            cover TEXT,
            source TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_disc_album_artist ON disc_album(cand_key);
        CREATE TABLE IF NOT EXISTS disc_meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
        -- Cover art found through Deezer/Discogs when the Cover Art Archive has none (url NULL = looked, found nothing).
        CREATE TABLE IF NOT EXISTS disc_cover (rgid TEXT PRIMARY KEY, url TEXT, source TEXT, checked_at TEXT NOT NULL);
        ",
    )?;
    Ok(())
}
