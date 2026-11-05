//! Small persistence layer: a JSON settings file plus a SQLite file that holds
//! Deemix download receipts (so an album is never downloaded twice by accident).

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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            deemix_download_path: String::new(),
            deemix_download_quality: "mp3_320".to_owned(),
            deemix_download_fallback: true,
            deemix_download_organization: "flat_artist_album_year".to_owned(),
            auto_update_check: true,
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

/// Opens (and migrates) the receipts database.
pub fn open(app: &AppHandle) -> Result<(Connection, PathBuf)> {
    let dir = app.path().app_data_dir().context("no data directory")?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("flipside.sqlite3");
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
        ",
    )?;
    Ok(())
}
