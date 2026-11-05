use anyhow::{anyhow, bail, Context, Result};
use blowfish::Blowfish;
use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use chrono::Utc;
use cookie::Cookie;
use cookie_store::CookieStore;
use id3::frame::{ExtendedText, InvolvedPeopleList, InvolvedPeopleListItem, Picture, PictureType};
use id3::{Tag, TagLike, Version};
use md5::{Digest, Md5};
use metaflac::block::PictureType as FlacPictureType;
use metaflac::Tag as FlacTag;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
#[cfg(not(test))]
use tauri::{AppHandle, };
use url::Url;
use zeroize::Zeroizing;
use tauri_specta::Event as _;

const DEEZER_GATEWAY_URL: &str = "https://www.deezer.com/ajax/gw-light.php";
const DEEZER_MEDIA_URL: &str = "https://media.deezer.com/v1/get_url";
const DEEMIX_USER_AGENT: &str = crate::http::BROWSER_USER_AGENT;
const STREAM_CHUNK_SIZE: usize = 2048;
const MAX_ARTWORK_BYTES: u64 = 20 * 1024 * 1024;
const MAX_ALBUM_TRACKS: usize = 500;
const MAX_MEDIA_FALLBACKS: usize = 8;
const MAX_PATH_SEGMENT_CHARS: usize = 96;
static DOWNLOAD_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeemixAlbumDownloadRequest {
    pub album_id: String,
    pub request_id: String,
    #[serde(default)]
    pub wish_list_item_id: Option<i64>,
    #[serde(default)]
    pub musicbrainz_release_group_id: Option<String>,
    #[serde(default)]
    pub expected_artist: String,
    #[serde(default)]
    pub expected_album: String,
    pub expected_year: Option<i32>,
    #[serde(default)]
    pub allow_duplicate: bool,
}

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeemixAlbumDownloadPreflightRequest {
    pub album_id: String,
    #[serde(default)]
    pub wish_list_item_id: Option<i64>,
    #[serde(default)]
    pub musicbrainz_release_group_id: Option<String>,
    pub artist: String,
    pub album: String,
    pub year: Option<i32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeemixAlbumDownloadPreflight {
    pub already_downloaded: bool,
    pub destination_path: Option<String>,
    pub downloaded_at: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeemixAlbumDownloadProgress {
    pub request_id: String,
    pub album_id: String,
    #[specta(type = crate::wire_enums::DeemixAlbumDownloadPhase)]
    pub phase: String,
    pub message: String,
    pub current_track: Option<String>,
    pub completed_tracks: usize,
    pub total_tracks: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeemixAlbumDownloadSummary {
    pub request_id: String,
    pub album_id: String,
    pub artist: String,
    pub album: String,
    pub year: Option<i32>,
    #[specta(type = crate::wire_enums::DeemixDownloadQuality)]
    pub quality: String,
    pub destination_path: String,
    pub cover_path: Option<String>,
    pub warning: Option<String>,
    pub track_count: usize,
    pub completed_at: String,
}

#[derive(Debug, Clone)]
struct KnownDownload {
    destination_path: String,
    downloaded_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DownloadQuality {
    Mp3_128,
    Mp3_320,
    Flac,
}

impl DownloadQuality {
    fn from_setting(value: &str) -> Self {
        match value {
            "mp3_128" => Self::Mp3_128,
            "flac" => Self::Flac,
            _ => Self::Mp3_320,
        }
    }

    fn setting_value(self) -> &'static str {
        match self {
            Self::Mp3_128 => "mp3_128",
            Self::Mp3_320 => "mp3_320",
            Self::Flac => "flac",
        }
    }

    fn deezer_format(self) -> &'static str {
        match self {
            Self::Mp3_128 => "MP3_128",
            Self::Mp3_320 => "MP3_320",
            Self::Flac => "FLAC",
        }
    }

    fn filesize_field(self) -> &'static str {
        match self {
            Self::Mp3_128 => "FILESIZE_MP3_128",
            Self::Mp3_320 => "FILESIZE_MP3_320",
            Self::Flac => "FILESIZE_FLAC",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Mp3_128 => "MP3 128 kbps",
            Self::Mp3_320 => "MP3 320 kbps",
            Self::Flac => "FLAC",
        }
    }

    fn file_extension(self) -> &'static str {
        match self {
            Self::Mp3_128 | Self::Mp3_320 => "mp3",
            Self::Flac => "flac",
        }
    }

    fn fallback_candidates(self, allow_fallback: bool) -> Vec<Self> {
        if !allow_fallback {
            return vec![self];
        }
        match self {
            Self::Flac => vec![Self::Flac, Self::Mp3_320, Self::Mp3_128],
            Self::Mp3_320 => vec![Self::Mp3_320, Self::Mp3_128],
            Self::Mp3_128 => vec![Self::Mp3_128],
        }
    }
}

struct AuthenticatedSession {
    agent: ureq::Agent,
    api_token: Zeroizing<String>,
    license_token: Zeroizing<String>,
    can_stream_hq: bool,
    can_stream_lossless: bool,
}

#[derive(Debug, Clone)]
struct AlbumMetadata {
    id: String,
    title: String,
    artist: String,
    album_artists: Vec<String>,
    release_date: Option<String>,
    year: Option<i32>,
    label: Option<String>,
    genres: Vec<String>,
    upc: Option<String>,
    copyright: Option<String>,
    cover_url: Option<String>,
    disc_total: u32,
    tracks: Vec<TrackMetadata>,
}

#[derive(Debug, Clone)]
struct TrackMetadata {
    id: String,
    media_id: String,
    title: String,
    artists: Vec<String>,
    track_number: u32,
    disc_number: u32,
    duration_seconds: u32,
    isrc: Option<String>,
    explicit: bool,
    copyright: Option<String>,
    composers: Vec<String>,
    involved_people: Vec<(String, String)>,
    track_token: Zeroizing<String>,
    track_token_expires_at: Option<u64>,
    fallback_id: Option<String>,
    download_quality: DownloadQuality,
}

#[derive(Debug, Clone)]
struct Artwork {
    bytes: Vec<u8>,
    mime_type: String,
    extension: &'static str,
}

fn known_download_for_connection(
    conn: &Connection,
    album_id: &str,
    wish_list_item_id: Option<i64>,
    musicbrainz_release_group_id: Option<&str>,
) -> Result<Option<KnownDownload>> {
    let mut statement = conn.prepare(
        "
        SELECT destination_path, completed_at
        FROM deemix_downloads
        WHERE deezer_album_id = ?1
           OR (?2 IS NOT NULL AND musicbrainz_release_group_id = ?2)
           OR (?2 IS NULL AND ?3 IS NOT NULL AND wish_list_item_id = ?3)
        ORDER BY completed_at DESC, id DESC
        ",
    )?;
    let rows = statement.query_map(
        params![album_id, musicbrainz_release_group_id, wish_list_item_id],
        |row| {
            Ok(KnownDownload {
                destination_path: row.get(0)?,
                downloaded_at: row.get(1)?,
            })
        },
    )?;
    for row in rows {
        let download = row?;
        if Path::new(&download.destination_path).is_dir() {
            return Ok(Some(download));
        }
    }
    Ok(None)
}

fn record_download_receipt(
    conn: &Connection,
    input: &DeemixAlbumDownloadPreflightRequest,
    quality: &str,
    destination_path: &Path,
    cover_path: Option<&Path>,
    track_count: usize,
    completed_at: &str,
    source: &str,
) -> Result<()> {
    conn.execute(
        "
        INSERT INTO deemix_downloads (
            deezer_album_id, wish_list_item_id, musicbrainz_release_group_id,
            artist, album, year, quality, destination_path, cover_path,
            track_count, completed_at, source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(deezer_album_id, destination_path) DO UPDATE SET
            wish_list_item_id = COALESCE(excluded.wish_list_item_id, deemix_downloads.wish_list_item_id),
            musicbrainz_release_group_id = COALESCE(excluded.musicbrainz_release_group_id, deemix_downloads.musicbrainz_release_group_id),
            artist = excluded.artist,
            album = excluded.album,
            year = excluded.year,
            quality = excluded.quality,
            cover_path = COALESCE(excluded.cover_path, deemix_downloads.cover_path),
            track_count = MAX(excluded.track_count, deemix_downloads.track_count),
            completed_at = excluded.completed_at,
            source = excluded.source
        ",
        params![
            &input.album_id,
            input.wish_list_item_id,
            input.musicbrainz_release_group_id.as_deref(),
            &input.artist,
            &input.album,
            input.year,
            quality,
            destination_path.to_string_lossy().as_ref(),
            cover_path.map(|path| path.to_string_lossy().into_owned()),
            track_count,
            completed_at,
            source,
        ],
    )
    .context("Could not record the Deemix download receipt")?;
    Ok(())
}

fn preflight_for_connection(
    conn: &Connection,
    root: &Path,
    organization: &str,
    quality: &str,
    input: &DeemixAlbumDownloadPreflightRequest,
    record_detected: bool,
) -> Result<DeemixAlbumDownloadPreflight> {
    if let Some(download) = known_download_for_connection(
        conn,
        &input.album_id,
        input.wish_list_item_id,
        input.musicbrainz_release_group_id.as_deref(),
    )? {
        return Ok(DeemixAlbumDownloadPreflight {
            already_downloaded: true,
            message: format!(
                "This album is already in the configured download folder: {}",
                download.destination_path
            ),
            destination_path: Some(download.destination_path),
            downloaded_at: Some(download.downloaded_at),
        });
    }

    let expected =
        destination_path_from_parts(root, &input.artist, &input.album, input.year, organization)?;
    if expected.is_dir() {
        let detected_at = Utc::now().to_rfc3339();
        if record_detected {
            record_download_receipt(
                conn,
                input,
                quality,
                &expected,
                None,
                0,
                &detected_at,
                "detected",
            )?;
        }
        return Ok(DeemixAlbumDownloadPreflight {
            already_downloaded: true,
            message: format!(
                "This album is already in the configured download folder: {}",
                expected.display()
            ),
            destination_path: Some(expected.to_string_lossy().into_owned()),
            downloaded_at: Some(detected_at),
        });
    }

    Ok(DeemixAlbumDownloadPreflight {
        already_downloaded: false,
        destination_path: None,
        downloaded_at: None,
        message: "This album is not currently in the configured download folder.".to_string(),
    })
}

#[cfg(not(test))]
pub fn preflight_album_for_app(
    app: &AppHandle,
    mut input: DeemixAlbumDownloadPreflightRequest,
) -> Result<DeemixAlbumDownloadPreflight> {
    validate_preflight_request(&mut input)?;
    let settings = crate::db::settings_for_app(app)?;
    let root = validate_download_root(&settings.deemix_download_path)?;
    let (conn, _) = crate::db::open(app)?;
    preflight_for_connection(
        &conn,
        &root,
        &settings.deemix_download_organization,
        &settings.deemix_download_quality,
        &input,
        true,
    )
}

#[cfg(not(test))]
pub fn download_album_for_app(
    app: &AppHandle,
    mut input: DeemixAlbumDownloadRequest,
) -> Result<DeemixAlbumDownloadSummary> {
    validate_request(&mut input)?;
    let _download_guard = DOWNLOAD_LOCK
        .lock()
        .map_err(|_| anyhow!("The Deemix download queue is unavailable."))?;
    let settings = crate::db::settings_for_app(app)?;
    let quality = DownloadQuality::from_setting(&settings.deemix_download_quality);
    let root = validate_download_root(&settings.deemix_download_path)?;
    let preflight_input = preflight_request_from_download(&input);
    let (preflight_conn, _) = crate::db::open(app)?;
    let preflight = preflight_for_connection(
        &preflight_conn,
        &root,
        &settings.deemix_download_organization,
        quality.setting_value(),
        &preflight_input,
        true,
    )?;
    drop(preflight_conn);
    if preflight.already_downloaded && !input.allow_duplicate {
        bail!(format!(
            "{} Use Download another copy if you intentionally want a second folder.",
            preflight.message
        ));
    }

    emit_progress(
        app,
        &input,
        "metadata",
        "Validating Deezer and loading album metadata…",
        None,
        0,
        0,
    );

    let mut session = authenticate()?;
    let quality_candidates =
        account_quality_candidates(&session, quality, settings.deemix_download_fallback)?;
    let mut album = fetch_album(&mut session, &input.album_id, &quality_candidates)?;
    let base_destination = destination_path(&root, &album, &settings.deemix_download_organization)?;
    let destination = if input.allow_duplicate {
        next_available_destination(&base_destination)?
    } else {
        base_destination
    };
    if destination.exists() {
        bail!(
            "The destination album folder already exists: {}. Existing files were not changed.",
            destination.display()
        );
    }

    let stage_name = format!(".flipside-download-{}-{}", album.id, input.request_id);
    let stage_path = root.join(stage_name);
    if stage_path.exists() {
        bail!("A staging folder for this download already exists. Try the download again.");
    }
    fs::create_dir(&stage_path).context("Could not create the Deemix staging folder")?;

    let result = download_album_to_stage(
        app,
        &input,
        &mut session,
        &mut album,
        &quality_candidates,
        &stage_path,
    )
        .and_then(|(cover_name, warning)| {
            if destination.exists() {
                bail!(
                    "The destination album folder was created by another process. Existing files were not changed."
                );
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .context("Could not create the configured album folder structure")?;
            }
            fs::rename(&stage_path, &destination)
                .context("Could not finalize the downloaded album folder")?;
            let cover_path = cover_name
                .as_deref()
                .map(|name| destination.join(name).to_string_lossy().into_owned());
            Ok(DeemixAlbumDownloadSummary {
                request_id: input.request_id.clone(),
                album_id: album.id.clone(),
                artist: album.artist.clone(),
                album: album.title.clone(),
                year: album.year,
                quality: quality.setting_value().to_string(),
                destination_path: destination.to_string_lossy().into_owned(),
                cover_path,
                warning,
                track_count: album.tracks.len(),
                completed_at: Utc::now().to_rfc3339(),
            })
        });

    match result {
        Ok(summary) => {
            let receipt_input = DeemixAlbumDownloadPreflightRequest {
                album_id: summary.album_id.clone(),
                wish_list_item_id: input.wish_list_item_id,
                musicbrainz_release_group_id: input.musicbrainz_release_group_id.clone(),
                artist: summary.artist.clone(),
                album: summary.album.clone(),
                year: summary.year,
            };
            let (conn, _) = crate::db::open(app)?;
            record_download_receipt(
                &conn,
                &receipt_input,
                &summary.quality,
                Path::new(&summary.destination_path),
                summary.cover_path.as_deref().map(Path::new),
                summary.track_count,
                &summary.completed_at,
                "download",
            )?;
            emit_progress(
                app,
                &input,
                "complete",
                &if summary.warning.is_some() {
                    format!(
                        "Downloaded and tagged {} tracks without album artwork: {}.",
                        summary.track_count,
                        quality_breakdown(&album)
                    )
                } else {
                    format!(
                        "Downloaded and tagged {} tracks: {}.",
                        summary.track_count,
                        quality_breakdown(&album)
                    )
                },
                None,
                summary.track_count,
                summary.track_count,
            );
            Ok(summary)
        }
        Err(error) => {
            if stage_path.exists() {
                let _ = fs::remove_dir_all(&stage_path);
            }
            emit_progress(
                app,
                &input,
                "failed",
                "The album download failed; staged files were removed.",
                None,
                0,
                album.tracks.len(),
            );
            Err(error)
        }
    }
}

#[cfg(not(test))]
fn download_album_to_stage(
    app: &AppHandle,
    input: &DeemixAlbumDownloadRequest,
    session: &mut AuthenticatedSession,
    album: &mut AlbumMetadata,
    quality_candidates: &[DownloadQuality],
    stage_path: &Path,
) -> Result<(Option<String>, Option<String>)> {
    emit_progress(
        app,
        input,
        "artwork",
        "Downloading album artwork…",
        None,
        0,
        album.tracks.len(),
    );
    let (artwork, cover_name, artwork_warning) = match album.cover_url.as_deref() {
        Some(cover_url) => match download_artwork(cover_url) {
            Ok(artwork) => {
                let cover_name = format!("cover.{}", artwork.extension);
                fs::write(stage_path.join(&cover_name), &artwork.bytes)
                    .context("Could not save the album cover")?;
                (Some(artwork), Some(cover_name), None)
            }
            Err(error) => {
                let warning = format!(
                    "Downloaded without artwork because Deezer artwork was unavailable: {error}"
                );
                emit_progress(
                    app,
                    input,
                    "artwork",
                    "Album artwork is unavailable; continuing with the audio download.",
                    None,
                    0,
                    album.tracks.len(),
                );
                (None, None, Some(warning))
            }
        },
        None => {
            let warning =
                "Downloaded without artwork because Deezer did not provide an album image."
                    .to_string();
            emit_progress(
                app,
                input,
                "artwork",
                "Deezer did not provide album artwork; continuing with the audio download.",
                None,
                0,
                album.tracks.len(),
            );
            (None, None, Some(warning))
        }
    };

    let mut used_names = HashSet::new();
    for index in 0..album.tracks.len() {
        let completed = index;
        let track_title = album.tracks[index].title.clone();
        emit_progress(
            app,
            input,
            "downloading",
            &format!("Downloading track {} of {}…", index + 1, album.tracks.len()),
            Some(track_title),
            completed,
            album.tracks.len(),
        );
        let (media_url, media_id) = {
            let track = &mut album.tracks[index];
            let media_url = authorize_track_media(session, track, quality_candidates)?;
            (media_url, track.media_id.clone())
        };
        let track = &album.tracks[index];
        let file_name = unique_track_filename(track, album.disc_total, &mut used_names);
        let final_path = stage_path.join(&file_name);
        let part_path = stage_path.join(format!("{file_name}.part"));
        download_and_decrypt_track(&media_url, &media_id, &part_path)
            .with_context(|| format!("Could not download {}", track.title))?;

        emit_progress(
            app,
            input,
            "tagging",
            &format!("Tagging track {} of {}…", index + 1, album.tracks.len()),
            Some(track.title.clone()),
            completed,
            album.tracks.len(),
        );
        write_audio_tags(&part_path, album, track, artwork.as_ref())
            .with_context(|| format!("Could not tag {}", track.title))?;
        fs::rename(&part_path, &final_path)
            .with_context(|| format!("Could not finalize {}", file_name))?;
    }
    Ok((cover_name, artwork_warning))
}

#[cfg(not(test))]
#[allow(clippy::too_many_arguments)]
fn emit_progress(
    app: &AppHandle,
    input: &DeemixAlbumDownloadRequest,
    phase: &str,
    message: &str,
    current_track: Option<String>,
    completed_tracks: usize,
    total_tracks: usize,
) {
    let _ = crate::events::DeemixDownloadProgress(DeemixAlbumDownloadProgress {
            request_id: input.request_id.clone(),
            album_id: input.album_id.clone(),
            phase: phase.to_string(),
            message: message.to_string(),
            current_track,
            completed_tracks,
            total_tracks,
        }).emit(app);
}

fn validate_request(input: &mut DeemixAlbumDownloadRequest) -> Result<()> {
    let mut preflight = preflight_request_from_download(input);
    validate_preflight_request(&mut preflight)?;
    input.album_id = preflight.album_id;
    input.wish_list_item_id = preflight.wish_list_item_id;
    input.musicbrainz_release_group_id = preflight.musicbrainz_release_group_id;
    input.expected_artist = preflight.artist;
    input.expected_album = preflight.album;
    input.expected_year = preflight.year;
    input.request_id = input.request_id.trim().to_string();
    if input.request_id.is_empty()
        || input.request_id.len() > 64
        || !input
            .request_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("The Deemix download request ID is invalid.");
    }
    Ok(())
}

fn preflight_request_from_download(
    input: &DeemixAlbumDownloadRequest,
) -> DeemixAlbumDownloadPreflightRequest {
    DeemixAlbumDownloadPreflightRequest {
        album_id: input.album_id.clone(),
        wish_list_item_id: input.wish_list_item_id,
        musicbrainz_release_group_id: input.musicbrainz_release_group_id.clone(),
        artist: input.expected_artist.clone(),
        album: input.expected_album.clone(),
        year: input.expected_year,
    }
}

fn validate_preflight_request(input: &mut DeemixAlbumDownloadPreflightRequest) -> Result<()> {
    input.album_id = input.album_id.trim().to_string();
    if input.album_id.is_empty()
        || input.album_id.len() > 20
        || !input
            .album_id
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        bail!("The selected Deezer album ID is invalid.");
    }
    if input.wish_list_item_id.is_some_and(|id| id <= 0) {
        bail!("The Wish List download context is invalid.");
    }
    input.musicbrainz_release_group_id = input
        .musicbrainz_release_group_id
        .take()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    if input
        .musicbrainz_release_group_id
        .as_deref()
        .is_some_and(|value| !is_uuid(value))
    {
        bail!("The MusicBrainz release-group context is invalid.");
    }
    input.artist = input.artist.trim().chars().take(300).collect();
    input.album = input.album.trim().chars().take(300).collect();
    if input.artist.is_empty() || input.album.is_empty() {
        bail!("The selected album needs an artist and title for the download preflight.");
    }
    if input
        .year
        .is_some_and(|year| !(1000..=3000).contains(&year))
    {
        bail!("The selected album year is outside the supported range.");
    }
    Ok(())
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, character)| {
            matches!(index, 8 | 13 | 18 | 23) && character == '-'
                || !matches!(index, 8 | 13 | 18 | 23) && character.is_ascii_hexdigit()
        })
}

fn account_quality_candidates(
    session: &AuthenticatedSession,
    preferred: DownloadQuality,
    allow_fallback: bool,
) -> Result<Vec<DownloadQuality>> {
    let candidates = preferred
        .fallback_candidates(allow_fallback)
        .into_iter()
        .filter(|quality| match quality {
            DownloadQuality::Flac => session.can_stream_lossless,
            DownloadQuality::Mp3_320 => session.can_stream_hq,
            DownloadQuality::Mp3_128 => true,
        })
        .collect::<Vec<_>>();
    if !candidates.is_empty() {
        return Ok(candidates);
    }
    match preferred {
        DownloadQuality::Flac => bail!(
            "This Deezer account does not report lossless streaming. Enable quality fallback or reconnect an account with lossless access."
        ),
        DownloadQuality::Mp3_320 => bail!(
            "This Deezer account does not report high-quality streaming. Enable quality fallback or reconnect an account with HQ access."
        ),
        DownloadQuality::Mp3_128 => bail!("This Deezer account cannot stream MP3 128 kbps."),
    }
}

fn quality_breakdown(album: &AlbumMetadata) -> String {
    [
        DownloadQuality::Flac,
        DownloadQuality::Mp3_320,
        DownloadQuality::Mp3_128,
    ]
    .into_iter()
    .filter_map(|quality| {
        let count = album
            .tracks
            .iter()
            .filter(|track| track.download_quality == quality)
            .count();
        (count > 0).then(|| format!("{count} as {}", quality.display_name()))
    })
    .collect::<Vec<_>>()
    .join(", ")
}

#[cfg(not(test))]
fn validate_download_root(value: &str) -> Result<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("Choose a Deemix download folder in Settings > Providers first.");
    }
    let root = PathBuf::from(trimmed);
    if !root.is_absolute() {
        bail!("The configured Deemix download folder must be an absolute path.");
    }
    if !root.is_dir() {
        bail!("The configured Deemix download folder does not exist or is not a folder.");
    }
    root.canonicalize()
        .context("Could not resolve the configured Deemix download folder")
}

fn authenticate() -> Result<AuthenticatedSession> {
    let arl = crate::deemix::stored_arl_for_download()?;
    let agent = authenticated_api_agent(&arl)?;
    let payload = gateway_request(&agent, "null", "deezer.getUserData", json!({}))?;
    if has_api_error(payload.get("error")) {
        bail!(gateway_failure_message(
            "deezer.getUserData",
            payload.get("error"),
            false
        ));
    }
    let user_data = gateway_results(&payload)?;
    let user_id = scalar_string(user_data.pointer("/USER/USER_ID")).unwrap_or_default();
    if user_id.is_empty() || user_id == "0" {
        bail!("The stored Deezer ARL is invalid or expired.");
    }
    let api_token = required_string(user_data.get("checkForm"), "Deezer API token")?;
    let license_token = required_string(
        user_data.pointer("/USER/OPTIONS/license_token"),
        "Deezer media license",
    )?;
    let options = user_data.pointer("/USER/OPTIONS").unwrap_or(&Value::Null);
    let can_stream_hq =
        flexible_bool(options.get("web_hq")) || flexible_bool(options.get("mobile_hq"));
    let can_stream_lossless =
        flexible_bool(options.get("web_lossless")) || flexible_bool(options.get("mobile_lossless"));
    Ok(AuthenticatedSession {
        agent,
        api_token: Zeroizing::new(api_token),
        license_token: Zeroizing::new(license_token),
        can_stream_hq,
        can_stream_lossless,
    })
}

fn gateway_request(
    agent: &ureq::Agent,
    api_token: &str,
    method: &str,
    args: Value,
) -> Result<Value> {
    gateway_request_at(agent, DEEZER_GATEWAY_URL, api_token, method, args)
}

fn gateway_request_at(
    agent: &ureq::Agent,
    endpoint: &str,
    api_token: &str,
    method: &str,
    args: Value,
) -> Result<Value> {
    let response = agent
        .post(endpoint)
        .query("api_version", "1.0")
        .query("api_token", api_token)
        .query("input", "3")
        .query("method", method)
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .set("User-Agent", DEEMIX_USER_AGENT)
        .send_json(args)
        .map_err(|error| network_error(error, "Deezer gateway"))?;
    response
        .into_json::<Value>()
        .context("Deezer gateway returned an unreadable response")
}

fn gateway_results(payload: &Value) -> Result<Value> {
    payload
        .get("results")
        .cloned()
        .context("Deezer returned no result for the authenticated metadata request")
}

fn gateway_call(session: &mut AuthenticatedSession, method: &str, args: Value) -> Result<Value> {
    gateway_call_at(session, DEEZER_GATEWAY_URL, method, args)
}

fn gateway_call_at(
    session: &mut AuthenticatedSession,
    endpoint: &str,
    method: &str,
    args: Value,
) -> Result<Value> {
    let mut refreshed_token = false;
    loop {
        let payload = gateway_request_at(
            &session.agent,
            endpoint,
            &session.api_token,
            method,
            args.clone(),
        )?;
        let error = payload.get("error");
        if !has_api_error(error) {
            return gateway_results(&payload);
        }
        if !refreshed_token && gateway_requires_token_refresh(error) {
            refresh_authenticated_session_at(session, endpoint)?;
            refreshed_token = true;
            continue;
        }
        bail!(gateway_failure_message(method, error, refreshed_token));
    }
}

fn refresh_authenticated_session_at(
    session: &mut AuthenticatedSession,
    endpoint: &str,
) -> Result<()> {
    let payload = gateway_request_at(
        &session.agent,
        endpoint,
        "null",
        "deezer.getUserData",
        json!({}),
    )?;
    if has_api_error(payload.get("error")) {
        bail!(gateway_failure_message(
            "deezer.getUserData",
            payload.get("error"),
            true
        ));
    }
    let user_data = gateway_results(&payload)?;
    let user_id = scalar_string(user_data.pointer("/USER/USER_ID")).unwrap_or_default();
    if user_id.is_empty() || user_id == "0" {
        bail!("The stored Deezer ARL is invalid or expired. Reconnect it in Settings > Providers.");
    }
    session.api_token = Zeroizing::new(required_string(
        user_data.get("checkForm"),
        "refreshed Deezer API token",
    )?);
    session.license_token = Zeroizing::new(required_string(
        user_data.pointer("/USER/OPTIONS/license_token"),
        "refreshed Deezer media license",
    )?);
    let options = user_data.pointer("/USER/OPTIONS").unwrap_or(&Value::Null);
    session.can_stream_hq =
        flexible_bool(options.get("web_hq")) || flexible_bool(options.get("mobile_hq"));
    session.can_stream_lossless =
        flexible_bool(options.get("web_lossless")) || flexible_bool(options.get("mobile_lossless"));
    Ok(())
}

fn fetch_album(
    session: &mut AuthenticatedSession,
    album_id: &str,
    quality_candidates: &[DownloadQuality],
) -> Result<AlbumMetadata> {
    let url = format!("https://api.deezer.com/album/{album_id}");
    let response = crate::http::get(&url)
        .timeout(Duration::from_secs(45))
        .set("Accept", "application/json")
        .call()
        .map_err(|error| network_error(error, "Deezer album metadata"))?;
    let public_album = response
        .into_json::<Value>()
        .context("Deezer returned unreadable album metadata")?;
    if public_album
        .get("error")
        .is_some_and(|value| !value.is_null())
    {
        bail!("Deezer could not load the selected album.");
    }
    let gateway_album = gateway_call(session, "album.getData", json!({ "ALB_ID": album_id }))?;
    let gateway_tracks = gateway_call(
        session,
        "song.getListByAlbum",
        json!({ "ALB_ID": album_id, "nb": -1 }),
    )?;
    parse_album_metadata(
        album_id,
        &public_album,
        &gateway_album,
        &gateway_tracks,
        quality_candidates,
    )
}

fn parse_album_metadata(
    requested_id: &str,
    public_album: &Value,
    gateway_album: &Value,
    gateway_tracks: &Value,
    quality_candidates: &[DownloadQuality],
) -> Result<AlbumMetadata> {
    quality_candidates
        .first()
        .context("No Deezer download quality is available for this account")?;
    let id = required_string(public_album.get("id"), "album ID")?;
    if id != requested_id {
        bail!("Deezer returned metadata for a different album.");
    }
    let title = required_trimmed(public_album.get("title"), "album title")?;
    let artist = required_trimmed(public_album.pointer("/artist/name"), "album artist")?;
    let mut album_artists = public_album
        .pointer("/contributors")
        .and_then(Value::as_array)
        .map(|contributors| {
            contributors
                .iter()
                .filter(|contributor| {
                    contributor
                        .get("role")
                        .and_then(Value::as_str)
                        .is_none_or(|role| role.eq_ignore_ascii_case("main"))
                })
                .filter_map(|contributor| string_value(contributor.get("name")))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    album_artists.dedup();
    if album_artists.is_empty() {
        album_artists.push(artist.clone());
    }
    let release_date = first_nonempty([
        string_value(gateway_album.get("ORIGINAL_RELEASE_DATE")),
        string_value(gateway_album.get("PHYSICAL_RELEASE_DATE")),
        string_value(public_album.get("release_date")),
    ]);
    let year = release_date.as_deref().and_then(parse_year);
    let label = first_nonempty([
        string_value(public_album.get("label")),
        string_value(gateway_album.get("LABEL_NAME")),
    ]);
    let upc = first_nonempty([
        string_value(public_album.get("upc")),
        string_value(gateway_album.get("UPC")),
    ]);
    let mut genres = public_album
        .pointer("/genres/data")
        .and_then(Value::as_array)
        .map(|values| string_list_from_objects(Some(values), "name"))
        .unwrap_or_default();
    genres.sort();
    genres.dedup();
    let album_copyright = string_value(gateway_album.get("COPYRIGHT"));
    let cover_url = first_nonempty([
        string_value(public_album.get("cover_xl")),
        string_value(public_album.get("cover_big")),
    ]);

    let gw_values = gateway_tracks
        .get("data")
        .and_then(Value::as_array)
        .context("Deezer returned no downloadable tracks for this album")?;
    if gw_values.is_empty() || gw_values.len() > MAX_ALBUM_TRACKS {
        bail!("The selected album has an unsupported number of tracks.");
    }
    let public_by_id = public_album
        .pointer("/tracks/data")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|track| scalar_string(track.get("id")).map(|id| (id, track)))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut tracks = Vec::with_capacity(gw_values.len());
    for (index, gateway_track) in gw_values.iter().enumerate() {
        let track_id = required_string(gateway_track.get("SNG_ID"), "track ID")?;
        let public_track = public_by_id.get(&track_id).copied().unwrap_or(&Value::Null);
        let version = string_value(gateway_track.get("VERSION")).unwrap_or_default();
        let mut track_title = first_nonempty([
            string_value(gateway_track.get("SNG_TITLE")),
            string_value(public_track.get("title")),
        ])
        .with_context(|| format!("Deezer omitted the title for track {track_id}"))?;
        if !version.is_empty() && !track_title.contains(&version) {
            track_title = format!("{} {}", track_title.trim(), version.trim());
        }
        let mut artists = string_list_from_gateway_artists(gateway_track.get("ARTISTS"));
        if artists.is_empty() {
            if let Some(name) = string_value(public_track.pointer("/artist/name")) {
                artists.push(name);
            }
        }
        if artists.is_empty() {
            artists.push(artist.clone());
        }
        let track_number = scalar_u32(gateway_track.get("TRACK_NUMBER"))
            .unwrap_or_else(|| u32::try_from(index + 1).unwrap_or(1))
            .max(1);
        let disc_number = scalar_u32(gateway_track.get("DISK_NUMBER"))
            .unwrap_or(1)
            .max(1);
        let track_token = required_string(gateway_track.get("TRACK_TOKEN"), "track token")?;
        let track_token_expires_at = scalar_u64(gateway_track.get("TRACK_TOKEN_EXPIRE"));
        let fallback_id = gateway_fallback_id(gateway_track, &track_id);
        let download_quality = select_download_quality(gateway_track, quality_candidates);
        let Some(download_quality) = download_quality else {
            let message = unavailable_track_message(&track_title, quality_candidates);
            bail!(message);
        };
        let (composers, involved_people) = contributor_tags(gateway_track.get("SNG_CONTRIBUTORS"));
        tracks.push(TrackMetadata {
            media_id: track_id.clone(),
            id: track_id,
            title: track_title,
            artists,
            track_number,
            disc_number,
            duration_seconds: scalar_u32(gateway_track.get("DURATION"))
                .or_else(|| scalar_u32(public_track.get("duration")))
                .unwrap_or(0),
            isrc: string_value(gateway_track.get("ISRC")),
            explicit: flexible_bool(gateway_track.get("EXPLICIT_LYRICS"))
                || flexible_bool(public_track.get("explicit_lyrics")),
            copyright: string_value(gateway_track.get("COPYRIGHT"))
                .or_else(|| album_copyright.clone()),
            composers,
            involved_people,
            track_token: Zeroizing::new(track_token),
            track_token_expires_at,
            fallback_id,
            download_quality,
        });
    }
    tracks.sort_by_key(|track| (track.disc_number, track.track_number));
    let disc_total = scalar_u32(gateway_album.get("NUMBER_DISK"))
        .unwrap_or_else(|| {
            tracks
                .iter()
                .map(|track| track.disc_number)
                .max()
                .unwrap_or(1)
        })
        .max(1);

    Ok(AlbumMetadata {
        id,
        title,
        artist,
        album_artists,
        release_date,
        year,
        label,
        genres,
        upc,
        copyright: album_copyright,
        cover_url,
        disc_total,
        tracks,
    })
}

fn select_download_quality(
    gateway_track: &Value,
    quality_candidates: &[DownloadQuality],
) -> Option<DownloadQuality> {
    quality_candidates
        .iter()
        .copied()
        .find(|quality| scalar_u64(gateway_track.get(quality.filesize_field())).unwrap_or(0) > 0)
}

fn gateway_fallback_id(gateway_track: &Value, current_id: &str) -> Option<String> {
    string_value(gateway_track.pointer("/FALLBACK/SNG_ID")).filter(|fallback_id| {
        fallback_id != current_id
            && fallback_id
                .chars()
                .all(|character| character.is_ascii_digit())
    })
}

fn accepted_quality_names(quality_candidates: &[DownloadQuality]) -> String {
    quality_candidates
        .iter()
        .map(|quality| quality.display_name())
        .collect::<Vec<_>>()
        .join(", then ")
}

fn unavailable_track_message(track_title: &str, quality_candidates: &[DownloadQuality]) -> String {
    let checked_qualities = accepted_quality_names(quality_candidates);
    if quality_candidates.len() > 1 {
        format!(
            "{track_title} has no full Deezer audio file at any accepted quality. Quality fallback checked {checked_qualities}. The album was not downloaded because every track must be available."
        )
    } else {
        format!(
            "{track_title} has no full Deezer audio file at the accepted quality: {checked_qualities}. The album was not downloaded because every track must be available."
        )
    }
}

fn update_track_media_source(
    track: &mut TrackMetadata,
    gateway_track: &Value,
    quality_candidates: &[DownloadQuality],
) -> Result<()> {
    let media_id = required_string(gateway_track.get("SNG_ID"), "fallback track ID")?;
    if !media_id.chars().all(|character| character.is_ascii_digit()) {
        bail!("Deezer returned an invalid fallback track ID.");
    }
    let track_token = required_string(gateway_track.get("TRACK_TOKEN"), "refreshed track token")?;
    let download_quality = select_download_quality(gateway_track, quality_candidates)
        .with_context(|| {
            format!(
                "{} has no playable source at the accepted qualities: {}.",
                track.title,
                accepted_quality_names(quality_candidates)
            )
        })?;
    track.media_id = media_id.clone();
    track.track_token = Zeroizing::new(track_token);
    track.track_token_expires_at = scalar_u64(gateway_track.get("TRACK_TOKEN_EXPIRE"));
    track.fallback_id = gateway_fallback_id(gateway_track, &media_id);
    track.download_quality = download_quality;
    Ok(())
}

fn refresh_track_media_source_at(
    session: &mut AuthenticatedSession,
    track: &mut TrackMetadata,
    source_id: &str,
    quality_candidates: &[DownloadQuality],
    gateway_endpoint: &str,
) -> Result<()> {
    let gateway_track = gateway_call_at(
        session,
        gateway_endpoint,
        "song.getData",
        json!({ "SNG_ID": source_id }),
    )
    .with_context(|| format!("Could not refresh the media source for {}", track.title))?;
    update_track_media_source(track, &gateway_track, quality_candidates)
}

fn track_token_is_expired(track: &TrackMetadata, now_unix: u64) -> bool {
    track
        .track_token_expires_at
        .is_some_and(|expires_at| expires_at <= now_unix.saturating_add(30))
}

fn request_media_url_from_endpoint(
    session: &AuthenticatedSession,
    track: &TrackMetadata,
    quality: DownloadQuality,
    media_endpoint: &str,
) -> Result<Option<String>> {
    let response = session
        .agent
        .post(media_endpoint)
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .set("User-Agent", DEEMIX_USER_AGENT)
        .send_json(json!({
            "license_token": session.license_token.as_str(),
            "media": [{
                "type": "FULL",
                "formats": [{ "cipher": "BF_CBC_STRIPE", "format": quality.deezer_format() }]
            }],
            "track_tokens": [track.track_token.as_str()]
        }))
        .map_err(|error| network_error(error, "Deezer media authorization"))?;
    let payload = response
        .into_json::<Value>()
        .context("Deezer returned an unreadable media authorization response")?;
    if has_api_error(payload.pointer("/data/0/errors")) {
        return Ok(None);
    }
    let Some(media_url) = string_value(payload.pointer("/data/0/media/0/sources/0/url")) else {
        return Ok(None);
    };
    validate_deezer_cdn_url(&media_url, false)?;
    Ok(Some(media_url))
}

fn authorize_track_media(
    session: &mut AuthenticatedSession,
    track: &mut TrackMetadata,
    quality_candidates: &[DownloadQuality],
) -> Result<String> {
    authorize_track_media_at(
        session,
        track,
        quality_candidates,
        DEEZER_GATEWAY_URL,
        DEEZER_MEDIA_URL,
        Utc::now().timestamp().max(0) as u64,
    )
}

fn authorize_track_media_at(
    session: &mut AuthenticatedSession,
    track: &mut TrackMetadata,
    quality_candidates: &[DownloadQuality],
    gateway_endpoint: &str,
    media_endpoint: &str,
    now_unix: u64,
) -> Result<String> {
    let mut refreshed_current_source = false;
    if track_token_is_expired(track, now_unix) {
        let source_id = track.media_id.clone();
        refresh_track_media_source_at(
            session,
            track,
            &source_id,
            quality_candidates,
            gateway_endpoint,
        )?;
        refreshed_current_source = true;
    }

    let mut visited_sources = HashSet::from([track.media_id.clone()]);
    let mut fallback_count = 0usize;
    loop {
        if let Some(media_url) =
            request_media_url_from_endpoint(session, track, track.download_quality, media_endpoint)?
        {
            return Ok(media_url);
        }

        if let Some(fallback_id) = track.fallback_id.clone().filter(|fallback_id| {
            fallback_count < MAX_MEDIA_FALLBACKS && visited_sources.insert(fallback_id.clone())
        }) {
            fallback_count += 1;
            refresh_track_media_source_at(
                session,
                track,
                &fallback_id,
                quality_candidates,
                gateway_endpoint,
            )?;
            refreshed_current_source = true;
            continue;
        }

        if !refreshed_current_source {
            let source_id = track.media_id.clone();
            refresh_track_media_source_at(
                session,
                track,
                &source_id,
                quality_candidates,
                gateway_endpoint,
            )?;
            refreshed_current_source = true;
            continue;
        }

        bail!(
            "Deezer could not provide a playable source for {} at the accepted qualities: {}. The current track token and Deezer fallback were both checked.",
            track.title,
            accepted_quality_names(quality_candidates)
        );
    }
}

fn download_and_decrypt_track(url: &str, track_id: &str, path: &Path) -> Result<()> {
    validate_deezer_cdn_url(url, false)?;
    let response = download_agent()
        .get(url)
        .set("User-Agent", DEEMIX_USER_AGENT)
        .call()
        .map_err(|error| network_error(error, "Deezer audio download"))?;
    let expected = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok());
    let mut reader = response.into_reader();
    let mut file = File::create(path).context("Could not create the staged audio file")?;
    let written = decrypt_stream(&mut reader, &mut file, track_id)?;
    file.flush()
        .context("Could not flush the staged audio file")?;
    if written < 1024 {
        bail!("Deezer returned an empty or incomplete audio file.");
    }
    if expected.is_some_and(|length| {
        written > length || length.saturating_sub(written) > STREAM_CHUNK_SIZE as u64
    }) {
        bail!("Deezer returned an incomplete audio stream.");
    }
    Ok(())
}

fn decrypt_stream(reader: &mut dyn Read, writer: &mut dyn Write, track_id: &str) -> Result<u64> {
    let key = blowfish_key(track_id);
    let mut chunk_index = 0usize;
    let mut written = 0u64;
    let mut is_start = true;
    loop {
        let mut buffer = [0u8; STREAM_CHUNK_SIZE];
        let mut filled = 0usize;
        while filled < buffer.len() {
            let count = reader
                .read(&mut buffer[filled..])
                .context("Could not read the encrypted Deezer stream")?;
            if count == 0 {
                break;
            }
            filled += count;
        }
        if filled == 0 {
            break;
        }
        if chunk_index % 3 == 0 && filled == STREAM_CHUNK_SIZE {
            decrypt_chunk(&mut buffer, &key)?;
        }
        let start = if is_start && buffer[0] == 0 && (filled < 8 || &buffer[4..8] != b"ftyp") {
            buffer[..filled]
                .iter()
                .position(|byte| *byte != 0)
                .unwrap_or(filled)
        } else {
            0
        };
        is_start = false;
        writer
            .write_all(&buffer[start..filled])
            .context("Could not write the decrypted Deezer stream")?;
        written += (filled - start) as u64;
        chunk_index += 1;
        if filled < STREAM_CHUNK_SIZE {
            break;
        }
    }
    Ok(written)
}

fn blowfish_key(track_id: &str) -> [u8; 16] {
    let digest = Md5::digest(track_id.as_bytes());
    let hex_digest = hex::encode(digest);
    let secret = b"g4el58wc0zvf9na1";
    let bytes = hex_digest.as_bytes();
    let mut key = [0u8; 16];
    for index in 0..16 {
        key[index] = bytes[index] ^ bytes[index + 16] ^ secret[index];
    }
    key
}

fn decrypt_chunk(chunk: &mut [u8], key: &[u8; 16]) -> Result<()> {
    let iv = [0u8, 1, 2, 3, 4, 5, 6, 7];
    cbc::Decryptor::<Blowfish>::new_from_slices(key, &iv)
        .map_err(|_| anyhow!("Could not initialize Deezer stream decryption"))?
        .decrypt_padded_mut::<NoPadding>(chunk)
        .map_err(|_| anyhow!("Could not decrypt the Deezer audio stream"))?;
    Ok(())
}

fn download_artwork(url: &str) -> Result<Artwork> {
    validate_deezer_cdn_url(url, true)?;
    let response = download_agent()
        .get(url)
        .set("Accept", "image/jpeg,image/png")
        .set("User-Agent", DEEMIX_USER_AGENT)
        .call()
        .map_err(|error| network_error(error, "Deezer album artwork"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_ARTWORK_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Could not read the Deezer album artwork")?;
    if bytes.len() as u64 > MAX_ARTWORK_BYTES {
        bail!("The Deezer album artwork exceeded the safe size limit.");
    }
    let (mime_type, extension) = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("image/jpeg", "jpg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        ("image/png", "png")
    } else {
        bail!("Deezer returned album artwork in an unsupported format.");
    };
    Ok(Artwork {
        bytes,
        mime_type: mime_type.to_string(),
        extension,
    })
}

fn write_audio_tags(
    path: &Path,
    album: &AlbumMetadata,
    track: &TrackMetadata,
    artwork: Option<&Artwork>,
) -> Result<()> {
    match track.download_quality {
        DownloadQuality::Mp3_128 | DownloadQuality::Mp3_320 => {
            write_mp3_tags(path, album, track, artwork)
        }
        DownloadQuality::Flac => write_flac_tags(path, album, track, artwork),
    }
}

fn write_mp3_tags(
    path: &Path,
    album: &AlbumMetadata,
    track: &TrackMetadata,
    artwork: Option<&Artwork>,
) -> Result<()> {
    let mut tag = Tag::new();
    tag.set_title(&track.title);
    tag.set_artist(track.artists.join("; "));
    tag.set_album(&album.title);
    tag.set_album_artist(album.album_artists.join("; "));
    tag.set_track(track.track_number);
    tag.set_total_tracks(u32::try_from(album.tracks.len()).unwrap_or(u32::MAX));
    tag.set_disc(track.disc_number);
    tag.set_total_discs(album.disc_total);
    if !album.genres.is_empty() {
        tag.set_genre(album.genres.join("; "));
    }
    if let Some(year) = album.year {
        tag.set_year(year);
    }
    if let Some(date) = &album.release_date {
        tag.set_text("TDRC", date);
    }
    if let Some(label) = &album.label {
        tag.set_text("TPUB", label);
    }
    if let Some(isrc) = &track.isrc {
        tag.set_text("TSRC", isrc);
    }
    if let Some(copyright) = track.copyright.as_ref().or(album.copyright.as_ref()) {
        tag.set_text("TCOP", copyright);
    }
    if track.duration_seconds > 0 {
        tag.set_text(
            "TLEN",
            (u64::from(track.duration_seconds) * 1000).to_string(),
        );
    }
    if !track.composers.is_empty() {
        tag.set_text("TCOM", track.composers.join("; "));
    }
    if !track.involved_people.is_empty() {
        tag.add_frame(InvolvedPeopleList {
            items: track
                .involved_people
                .iter()
                .map(|(role, name)| InvolvedPeopleListItem {
                    involvement: role.clone(),
                    involvee: name.clone(),
                })
                .collect(),
        });
    }
    if let Some(upc) = &album.upc {
        tag.add_frame(ExtendedText {
            description: "BARCODE".to_string(),
            value: upc.clone(),
        });
    }
    tag.add_frame(ExtendedText {
        description: "ITUNESADVISORY".to_string(),
        value: if track.explicit { "1" } else { "0" }.to_string(),
    });
    tag.add_frame(ExtendedText {
        description: "SOURCE".to_string(),
        value: "Deezer".to_string(),
    });
    tag.add_frame(ExtendedText {
        description: "SOURCEID".to_string(),
        value: track.id.clone(),
    });
    if let Some(artwork) = artwork {
        tag.add_frame(Picture {
            mime_type: artwork.mime_type.clone(),
            picture_type: PictureType::CoverFront,
            description: "cover".to_string(),
            data: artwork.bytes.clone(),
        });
    }
    tag.write_to_path(path, Version::Id3v24)
        .context("Could not write ID3 tags")?;
    Ok(())
}

fn write_flac_tags(
    path: &Path,
    album: &AlbumMetadata,
    track: &TrackMetadata,
    artwork: Option<&Artwork>,
) -> Result<()> {
    let mut tag = FlacTag::read_from_path(path).context("Could not read FLAC metadata")?;
    tag.set_vorbis("TITLE", vec![track.title.clone()]);
    tag.set_vorbis("ARTIST", track.artists.clone());
    tag.set_vorbis("ALBUM", vec![album.title.clone()]);
    tag.set_vorbis("ALBUMARTIST", album.album_artists.clone());
    tag.set_vorbis("TRACKNUMBER", vec![track.track_number.to_string()]);
    tag.set_vorbis("TRACKTOTAL", vec![album.tracks.len().to_string()]);
    tag.set_vorbis("DISCNUMBER", vec![track.disc_number.to_string()]);
    tag.set_vorbis("DISCTOTAL", vec![album.disc_total.to_string()]);
    if !album.genres.is_empty() {
        tag.set_vorbis("GENRE", album.genres.clone());
    }
    if let Some(date) = &album.release_date {
        tag.set_vorbis("DATE", vec![date.clone()]);
    } else if let Some(year) = album.year {
        tag.set_vorbis("DATE", vec![year.to_string()]);
    }
    if let Some(label) = &album.label {
        tag.set_vorbis("LABEL", vec![label.clone()]);
    }
    if let Some(isrc) = &track.isrc {
        tag.set_vorbis("ISRC", vec![isrc.clone()]);
    }
    if let Some(copyright) = track.copyright.as_ref().or(album.copyright.as_ref()) {
        tag.set_vorbis("COPYRIGHT", vec![copyright.clone()]);
    }
    if !track.composers.is_empty() {
        tag.set_vorbis("COMPOSER", track.composers.clone());
    }
    let mut people_by_role = HashMap::<String, Vec<String>>::new();
    for (role, name) in &track.involved_people {
        let key = role
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if !key.is_empty() {
            people_by_role.entry(key).or_default().push(name.clone());
        }
    }
    for (role, names) in people_by_role {
        tag.set_vorbis(role, names);
    }
    if let Some(upc) = &album.upc {
        tag.set_vorbis("BARCODE", vec![upc.clone()]);
    }
    tag.set_vorbis(
        "ITUNESADVISORY",
        vec![if track.explicit { "1" } else { "0" }.to_string()],
    );
    tag.set_vorbis("SOURCE", vec!["Deezer".to_string()]);
    tag.set_vorbis("SOURCEID", vec![track.id.clone()]);
    if let Some(artwork) = artwork {
        tag.add_picture(
            artwork.mime_type.clone(),
            FlacPictureType::CoverFront,
            artwork.bytes.clone(),
        );
    }
    tag.save().context("Could not write FLAC tags")?;
    Ok(())
}

fn destination_path(root: &Path, album: &AlbumMetadata, organization: &str) -> Result<PathBuf> {
    destination_path_from_parts(root, &album.artist, &album.title, album.year, organization)
}

fn destination_path_from_parts(
    root: &Path,
    artist: &str,
    album: &str,
    year: Option<i32>,
    organization: &str,
) -> Result<PathBuf> {
    let artist = safe_windows_segment(artist);
    let album_name = safe_windows_segment(album);
    let year_suffix = year.map(|year| format!(" ({year})")).unwrap_or_default();
    let destination = if organization == "artist_album_year_folders" {
        root.join(artist).join(format!("{album_name}{year_suffix}"))
    } else {
        root.join(format!("{artist} - {album_name}{year_suffix}"))
    };
    if destination.to_string_lossy().chars().count() > 245 {
        bail!("The generated album path is too long for reliable Windows file handling.");
    }
    Ok(destination)
}

fn next_available_destination(base: &Path) -> Result<PathBuf> {
    if !base.exists() {
        return Ok(base.to_path_buf());
    }
    let parent = base
        .parent()
        .context("The generated album destination has no parent folder")?;
    let name = base
        .file_name()
        .and_then(|value| value.to_str())
        .context("The generated album destination name is invalid")?;
    for copy_number in 2..=999 {
        let candidate = parent.join(format!("{name} [{copy_number}]"));
        if candidate.to_string_lossy().chars().count() > 245 {
            bail!("The generated duplicate album path is too long for reliable Windows file handling.");
        }
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("Could not choose a free folder name for another copy of this album.")
}

fn unique_track_filename(
    track: &TrackMetadata,
    disc_total: u32,
    used_names: &mut HashSet<String>,
) -> String {
    let title = safe_windows_segment(&track.title);
    let prefix = if disc_total > 1 {
        format!("{}-{:02}", track.disc_number, track.track_number)
    } else {
        format!("{:02}", track.track_number)
    };
    let extension = track.download_quality.file_extension();
    let mut file_name = format!("{prefix} {title}.{extension}");
    let key = file_name.to_lowercase();
    if !used_names.insert(key) {
        file_name = format!("{prefix} {title} [{}].{extension}", track.id);
        used_names.insert(file_name.to_lowercase());
    }
    file_name
}

fn safe_windows_segment(value: &str) -> String {
    let mut normalized = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches([' ', '.'])
        .chars()
        .take(MAX_PATH_SEGMENT_CHARS)
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_string();
    if normalized.is_empty() {
        normalized = "Unknown".to_string();
    }
    let stem = normalized
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        normalized.push('_');
    }
    normalized
}

fn validate_deezer_cdn_url(value: &str, artwork: bool) -> Result<()> {
    let parsed = Url::parse(value).context("Deezer returned an invalid CDN URL")?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("Deezer returned an unsafe CDN URL.");
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if !(host == "dzcdn.net" || host.ends_with(".dzcdn.net")) {
        bail!("Deezer returned an unexpected CDN host.");
    }
    if artwork && !parsed.path().to_ascii_lowercase().contains("/images/") {
        bail!("Deezer returned an unexpected artwork URL.");
    }
    Ok(())
}

fn authenticated_api_agent(arl: &str) -> Result<ureq::Agent> {
    let deezer_url = Url::parse("https://www.deezer.com/")
        .context("Could not initialize the authenticated Deezer session")?;
    let arl_cookie = Cookie::build(("arl", arl.to_string()))
        .domain(".deezer.com")
        .path("/")
        .secure(true)
        .http_only(true)
        .build();
    let mut cookie_store = CookieStore::default();
    cookie_store
        .insert_raw(&arl_cookie, &deezer_url)
        .context("Could not initialize the authenticated Deezer cookie jar")?;
    Ok(ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .redirects(0)
        .cookie_store(cookie_store)
        .build())
}

fn download_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(120))
        .timeout_write(Duration::from_secs(30))
        .redirects(0)
        .build()
}

fn network_error(error: ureq::Error, operation: &str) -> anyhow::Error {
    match error {
        ureq::Error::Status(status, _) => anyhow!("{operation} failed ({status})."),
        ureq::Error::Transport(_) => anyhow!("Could not reach Deezer for {operation}."),
    }
}

fn has_api_error(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::Object(values)) => !values.is_empty(),
        Some(Value::String(value)) => !value.trim().is_empty(),
        Some(_) => true,
    }
}

fn gateway_requires_token_refresh(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let normalized = serde_json::to_string(value)
        .unwrap_or_default()
        .to_ascii_lowercase();
    normalized.contains("invalid api token")
        || normalized.contains("invalid csrf token")
        || normalized.contains("valid_token_required")
}

fn gateway_is_rate_limited(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let normalized = serde_json::to_string(value)
        .unwrap_or_default()
        .to_ascii_lowercase();
    normalized.contains("rate limit")
        || normalized.contains("too many request")
        || normalized.contains("quota")
}

fn gateway_failure_message(method: &str, error: Option<&Value>, refreshed_token: bool) -> String {
    if gateway_requires_token_refresh(error) {
        return if refreshed_token {
            "Deezer rejected the refreshed authenticated session. Reconnect the ARL in Settings > Providers and try again."
                .to_string()
        } else {
            "Deezer rejected the authenticated session. Reconnect the ARL in Settings > Providers and try again."
                .to_string()
        };
    }
    if gateway_is_rate_limited(error) {
        return "Deezer temporarily rate-limited the album metadata request. Try again in a moment."
            .to_string();
    }
    match method {
        "deezer.getUserData" => {
            "Deezer could not validate the stored ARL session. Reconnect it in Settings > Providers."
                .to_string()
        }
        "album.getData" => {
            "Deezer could not load authenticated album details for this release.".to_string()
        }
        "song.getListByAlbum" => {
            "Deezer could not load the authenticated track list for this release.".to_string()
        }
        _ => "Deezer rejected the authenticated metadata request.".to_string(),
    }
}

fn scalar_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    scalar_string(value)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "0")
}

fn required_string(value: Option<&Value>, field: &str) -> Result<String> {
    string_value(value).with_context(|| format!("Deezer omitted the {field}"))
}

fn required_trimmed(value: Option<&Value>, field: &str) -> Result<String> {
    required_string(value, field).map(|value| value.chars().take(500).collect())
}

fn scalar_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn scalar_u32(value: Option<&Value>) -> Option<u32> {
    scalar_u64(value).and_then(|value| u32::try_from(value).ok())
}

fn flexible_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().is_some_and(|value| value != 0),
        Some(Value::String(value)) => matches!(value.as_str(), "1" | "true" | "TRUE"),
        _ => false,
    }
}

fn parse_year(value: &str) -> Option<i32> {
    let year = value.get(0..4)?.parse::<i32>().ok()?;
    (1000..=3000).contains(&year).then_some(year)
}

fn first_nonempty<const N: usize>(values: [Option<String>; N]) -> Option<String> {
    values.into_iter().flatten().find(|value| !value.is_empty())
}

fn string_list_from_objects(values: Option<&Vec<Value>>, field: &str) -> Vec<String> {
    let mut result = Vec::new();
    for value in values.into_iter().flatten() {
        if let Some(text) = string_value(value.get(field)) {
            if !result.iter().any(|existing| existing == &text) {
                result.push(text);
            }
        }
    }
    result
}

fn string_list_from_gateway_artists(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| string_list_from_objects(Some(values), "ART_NAME"))
        .unwrap_or_default()
}

fn contributor_tags(value: Option<&Value>) -> (Vec<String>, Vec<(String, String)>) {
    let Some(contributors) = value.and_then(Value::as_object) else {
        return (Vec::new(), Vec::new());
    };
    let mut composers = Vec::new();
    let mut involved = Vec::new();
    for (role, names) in contributors {
        let normalized_role = role.trim().to_ascii_lowercase();
        let names = names
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if normalized_role == "composer" {
            for name in &names {
                if !composers.contains(name) {
                    composers.push(name.clone());
                }
            }
        }
        if matches!(
            normalized_role.as_str(),
            "author" | "engineer" | "mixer" | "producer" | "writer"
        ) {
            for name in names {
                involved.push((normalized_role.clone(), name));
            }
        }
    }
    (composers, involved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};
    use std::io::Cursor;
    use std::net::TcpListener;
    use std::thread;

    fn read_complete_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = stream.read(&mut buffer).expect("read mock request");
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            let Some(header_end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            if request.len() >= header_end + content_length {
                break;
            }
        }
        request
    }

    #[test]
    fn validates_ids_and_rejects_staging_path_characters() {
        let mut valid = DeemixAlbumDownloadRequest {
            album_id: " 240766 ".to_string(),
            request_id: "request-123_abc".to_string(),
            wish_list_item_id: Some(12),
            musicbrainz_release_group_id: Some("f3cd5c58-4f20-3c84-8d7f-cf12e6ba4ec8".to_string()),
            expected_artist: "Helmet".to_string(),
            expected_album: "Meantime".to_string(),
            expected_year: Some(1992),
            allow_duplicate: false,
        };
        validate_request(&mut valid).expect("valid request");
        assert_eq!(valid.album_id, "240766");
        let mut invalid = DeemixAlbumDownloadRequest {
            album_id: "240766".to_string(),
            request_id: "../escape".to_string(),
            wish_list_item_id: None,
            musicbrainz_release_group_id: None,
            expected_artist: "Helmet".to_string(),
            expected_album: "Meantime".to_string(),
            expected_year: Some(1992),
            allow_duplicate: false,
        };
        assert!(validate_request(&mut invalid).is_err());
    }

    #[test]
    fn authenticated_agent_seeds_the_arl_cookie() {
        let arl = "ab".repeat(48);
        let agent = authenticated_api_agent(&arl).expect("authenticated agent");
        let url = Url::parse(DEEZER_GATEWAY_URL).expect("gateway URL");
        let cookies = agent.cookie_store();
        let stored_arl = cookies
            .get_request_values(&url)
            .find_map(|(name, value)| (name == "arl").then_some(value));
        assert_eq!(stored_arl, Some(arl.as_str()));
    }

    #[test]
    fn shared_cookie_session_refreshes_an_invalid_csrf_token_once() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Deezer gateway");
        let endpoint = format!(
            "http://{}/gw-light.php",
            listener.local_addr().expect("address")
        );
        let server = thread::spawn(move || {
            let mut observed_requests = Vec::new();
            for request_index in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept mock request");
                let request = read_complete_http_request(&mut stream);
                let request = String::from_utf8_lossy(&request);
                let request_line = request.lines().next().unwrap_or_default().to_string();
                let cookie_header = request
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("cookie:"))
                    .unwrap_or_default()
                    .to_string();

                let (body, set_cookie) = match request_index {
                    0 => (
                        r#"{"error":{"VALID_TOKEN_REQUIRED":"Invalid CSRF token"}}"#,
                        "Set-Cookie: sid=session-123; Path=/; HttpOnly\r\n",
                    ),
                    1 => (
                        r#"{"error":[],"results":{"checkForm":"fresh-token","USER":{"USER_ID":"123","OPTIONS":{"license_token":"fresh-license","web_hq":true}}}}"#,
                        "",
                    ),
                    _ => (r#"{"error":[],"results":{"ALB_ID":"42"}}"#, ""),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                    body.len(),
                    set_cookie,
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write mock response");
                observed_requests.push((request_line, cookie_header));
            }
            observed_requests
        });

        let endpoint_url = Url::parse(&endpoint).expect("mock endpoint URL");
        let mut cookie_store = CookieStore::default();
        cookie_store
            .insert_raw(
                &Cookie::build(("arl", "test-arl")).path("/").build(),
                &endpoint_url,
            )
            .expect("seed mock ARL");
        let mut session = AuthenticatedSession {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(5))
                .redirects(0)
                .cookie_store(cookie_store)
                .build(),
            api_token: Zeroizing::new("stale-token".to_string()),
            license_token: Zeroizing::new("stale-license".to_string()),
            can_stream_hq: false,
            can_stream_lossless: false,
        };
        let result = gateway_call_at(
            &mut session,
            &endpoint,
            "album.getData",
            json!({ "ALB_ID": "42" }),
        )
        .expect("retry authenticated metadata request");
        assert_eq!(result.get("ALB_ID"), Some(&json!("42")));
        assert_eq!(session.api_token.as_str(), "fresh-token");
        assert_eq!(session.license_token.as_str(), "fresh-license");
        assert!(session.can_stream_hq);
        let observed_requests = server.join().expect("mock server");
        assert_eq!(observed_requests.len(), 3);
        assert!(observed_requests
            .iter()
            .all(|(_, cookie)| cookie.contains("arl=test-arl")));
        assert!(observed_requests[0].0.contains("method=album.getData"));
        assert!(observed_requests[0].0.contains("api_token=stale-token"));
        assert!(observed_requests[1].0.contains("method=deezer.getUserData"));
        assert!(observed_requests[1].0.contains("api_token=null"));
        assert!(observed_requests[1].1.contains("sid=session-123"));
        assert!(observed_requests[2].0.contains("method=album.getData"));
        assert!(observed_requests[2].0.contains("api_token=fresh-token"));
        assert!(observed_requests[2].1.contains("sid=session-123"));
    }

    #[test]
    fn gateway_errors_are_classified_without_echoing_provider_payloads() {
        let error = json!({ "VALID_TOKEN_REQUIRED": "Invalid CSRF token" });
        assert!(gateway_requires_token_refresh(Some(&error)));
        let message = gateway_failure_message("album.getData", Some(&error), true);
        assert!(message.contains("refreshed authenticated session"));
        assert!(!message.contains("VALID_TOKEN_REQUIRED"));
        assert!(!message.contains("Invalid CSRF token"));

        let rate_limit = json!({ "GATEWAY_ERROR": "Too many requests" });
        assert!(gateway_is_rate_limited(Some(&rate_limit)));
        assert!(
            gateway_failure_message("album.getData", Some(&rate_limit), false)
                .contains("temporarily rate-limited")
        );
    }

    #[test]
    #[ignore = "requires a configured Windows Credential Manager ARL and makes live Deezer metadata requests"]
    fn live_authenticated_album_metadata_session() {
        let mut session = authenticate().expect("authenticate the configured Deezer ARL");
        let album = fetch_album(&mut session, "240766", &[DownloadQuality::Mp3_320])
            .expect("load authenticated album metadata");
        assert_eq!(album.id, "240766");
        assert!(!album.tracks.is_empty());
    }

    #[test]
    fn sanitizes_windows_names_and_reserved_devices() {
        assert_eq!(safe_windows_segment("AC/DC: Live?"), "AC_DC_ Live_");
        assert_eq!(safe_windows_segment("CON"), "CON_");
        assert_eq!(safe_windows_segment("  Album...  "), "Album");
    }

    #[test]
    fn builds_flat_and_nested_album_destinations() {
        let album = fixture_album();
        assert_eq!(
            destination_path(Path::new(r"D:\Downloads"), &album, "flat_artist_album_year")
                .expect("flat"),
            Path::new(r"D:\Downloads").join("Helmet - Meantime (1992)")
        );
        assert_eq!(
            destination_path(
                Path::new(r"D:\Downloads"),
                &album,
                "artist_album_year_folders"
            )
            .expect("nested"),
            Path::new(r"D:\Downloads")
                .join("Helmet")
                .join("Meantime (1992)")
        );
    }

    #[test]
    fn preflight_detects_and_records_an_existing_album_folder() {
        let root = std::env::temp_dir().join(format!(
            "flipside-deemix-preflight-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("create preflight root");
        let expected = destination_path_from_parts(
            &root,
            "Helmet",
            "Meantime",
            Some(1992),
            "flat_artist_album_year",
        )
        .expect("expected destination");
        fs::create_dir_all(&expected).expect("create existing album folder");

        let conn = Connection::open_in_memory().expect("open database");
        crate::db::configure(&conn).expect("configure database");
        crate::db::migrate(&conn).expect("migrate database");
        let input = DeemixAlbumDownloadPreflightRequest {
            album_id: "240766".to_string(),
            wish_list_item_id: Some(7),
            musicbrainz_release_group_id: Some("f3cd5c58-4f20-3c84-8d7f-cf12e6ba4ec8".to_string()),
            artist: "Helmet".to_string(),
            album: "Meantime".to_string(),
            year: Some(1992),
        };
        let result = preflight_for_connection(
            &conn,
            &root,
            "flat_artist_album_year",
            "mp3_320",
            &input,
            true,
        )
        .expect("run preflight");

        assert!(result.already_downloaded);
        assert_eq!(result.destination_path.as_deref(), expected.to_str());
        let receipt_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM deemix_downloads", [], |row| {
                row.get(0)
            })
            .expect("count detected receipt");
        assert_eq!(receipt_count, 1);
        fs::remove_dir_all(&root).expect("remove preflight root");
    }

    #[test]
    fn chooses_a_numbered_folder_for_another_copy() {
        let root = std::env::temp_dir().join(format!(
            "flipside-deemix-duplicate-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("create duplicate root");
        let base = root.join("Helmet - Meantime (1992)");
        fs::create_dir_all(&base).expect("create base album folder");
        fs::create_dir_all(root.join("Helmet - Meantime (1992) [2]"))
            .expect("create second album folder");

        assert_eq!(
            next_available_destination(&base).expect("duplicate destination"),
            root.join("Helmet - Meantime (1992) [3]")
        );
        fs::remove_dir_all(&root).expect("remove duplicate root");
    }

    #[test]
    fn artist_wish_receipts_do_not_mark_other_release_groups_downloaded() {
        let root = std::env::temp_dir().join(format!(
            "flipside-deemix-artist-receipt-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let downloaded = root.join("Pet Shop Boys - Please (1986)");
        fs::create_dir_all(&downloaded).expect("create downloaded album folder");
        let conn = Connection::open_in_memory().expect("open database");
        crate::db::configure(&conn).expect("configure database");
        crate::db::migrate(&conn).expect("migrate database");
        conn.execute(
            "
            INSERT INTO deemix_downloads (
                deezer_album_id, wish_list_item_id, musicbrainz_release_group_id,
                artist, album, year, quality, destination_path, track_count,
                completed_at, source
            ) VALUES (?1, 9, ?2, 'Pet Shop Boys', 'Please', 1986, 'mp3_320', ?3, 11,
                '2026-07-26T12:00:00Z', 'download')
            ",
            params![
                "101",
                "00000000-0000-4000-8000-000000000001",
                downloaded.to_string_lossy().as_ref()
            ],
        )
        .expect("insert artist album receipt");

        let other = known_download_for_connection(
            &conn,
            "102",
            Some(9),
            Some("00000000-0000-4000-8000-000000000002"),
        )
        .expect("look up other release group");
        assert!(other.is_none());
        fs::remove_dir_all(&root).expect("remove artist receipt root");
    }

    #[test]
    fn parses_download_and_tag_metadata() {
        let public = json!({
            "id": 240766,
            "title": "Meantime",
            "artist": { "name": "Helmet" },
            "contributors": [{ "name": "Helmet", "role": "Main" }],
            "release_date": "1992-01-01",
            "label": "Interscope",
            "upc": "606949216221",
            "cover_xl": "https://cdn-images.dzcdn.net/images/cover/hash/1000x1000.jpg",
            "genres": { "data": [{ "name": "Alternative" }, { "name": "Metal" }] },
            "tracks": { "data": [{
                "id": 2445707,
                "title": "In The Meantime",
                "duration": 188,
                "explicit_lyrics": false,
                "artist": { "name": "Helmet" }
            }] }
        });
        let gateway_album = json!({
            "NUMBER_DISK": 1,
            "COPYRIGHT": "1992 Interscope",
            "ORIGINAL_RELEASE_DATE": "1992-06-23"
        });
        let gateway_tracks = json!({ "data": [{
            "SNG_ID": 2445707,
            "SNG_TITLE": "In The Meantime",
            "TRACK_NUMBER": 1,
            "DISK_NUMBER": 1,
            "DURATION": 188,
            "ISRC": "USIR19200541",
            "TRACK_TOKEN": "token",
            "TRACK_TOKEN_EXPIRE": 4102444800u64,
            "FILESIZE_MP3_320": 123456,
            "FALLBACK": { "SNG_ID": 2445708 },
            "ARTISTS": [{ "ART_NAME": "Helmet" }],
            "SNG_CONTRIBUTORS": {
                "composer": ["Page Hamilton"],
                "producer": ["Steve Albini"]
            }
        }] });
        let album = parse_album_metadata(
            "240766",
            &public,
            &gateway_album,
            &gateway_tracks,
            &[DownloadQuality::Mp3_320],
        )
        .expect("album metadata");
        assert_eq!(album.year, Some(1992));
        assert_eq!(album.release_date.as_deref(), Some("1992-06-23"));
        assert_eq!(album.label.as_deref(), Some("Interscope"));
        assert_eq!(album.genres, vec!["Alternative", "Metal"]);
        assert_eq!(album.tracks[0].isrc.as_deref(), Some("USIR19200541"));
        assert_eq!(album.tracks[0].media_id, "2445707");
        assert_eq!(album.tracks[0].track_token_expires_at, Some(4_102_444_800));
        assert_eq!(album.tracks[0].fallback_id.as_deref(), Some("2445708"));
        assert_eq!(album.tracks[0].download_quality, DownloadQuality::Mp3_320);
        assert_eq!(album.tracks[0].composers, vec!["Page Hamilton"]);
        assert_eq!(
            album.tracks[0].involved_people,
            vec![("producer".to_string(), "Steve Albini".to_string())]
        );

        let mut public_without_cover = public.clone();
        public_without_cover
            .as_object_mut()
            .expect("public album object")
            .remove("cover_xl");
        let album_without_cover = parse_album_metadata(
            "240766",
            &public_without_cover,
            &gateway_album,
            &gateway_tracks,
            &[DownloadQuality::Mp3_320],
        )
        .expect("album metadata without optional artwork");
        assert!(album_without_cover.cover_url.is_none());
    }

    #[test]
    fn quality_fallback_uses_the_best_available_lower_format() {
        assert_eq!(
            DownloadQuality::Flac.fallback_candidates(true),
            vec![
                DownloadQuality::Flac,
                DownloadQuality::Mp3_320,
                DownloadQuality::Mp3_128
            ]
        );
        assert_eq!(
            DownloadQuality::Mp3_320.fallback_candidates(true),
            vec![DownloadQuality::Mp3_320, DownloadQuality::Mp3_128]
        );
        assert_eq!(
            DownloadQuality::Flac.fallback_candidates(false),
            vec![DownloadQuality::Flac]
        );

        let gateway_track = json!({
            "FILESIZE_FLAC": 0,
            "FILESIZE_MP3_320": 0,
            "FILESIZE_MP3_128": 12345
        });
        assert_eq!(
            select_download_quality(
                &gateway_track,
                &DownloadQuality::Flac.fallback_candidates(true)
            ),
            Some(DownloadQuality::Mp3_128)
        );
        assert_eq!(
            select_download_quality(
                &gateway_track,
                &DownloadQuality::Flac.fallback_candidates(false)
            ),
            None
        );
        assert_eq!(
            unavailable_track_message(
                "Flyin' Shoes",
                &[DownloadQuality::Mp3_320, DownloadQuality::Mp3_128]
            ),
            "Flyin' Shoes has no full Deezer audio file at any accepted quality. Quality fallback checked MP3 320 kbps, then MP3 128 kbps. The album was not downloaded because every track must be available."
        );
        assert_eq!(
            unavailable_track_message("Flyin' Shoes", &[DownloadQuality::Mp3_320]),
            "Flyin' Shoes has no full Deezer audio file at the accepted quality: MP3 320 kbps. The album was not downloaded because every track must be available."
        );
    }

    #[test]
    fn media_authorization_follows_the_deezer_track_fallback() {
        let media_listener = TcpListener::bind("127.0.0.1:0").expect("bind mock media endpoint");
        let media_endpoint = format!(
            "http://{}",
            media_listener.local_addr().expect("media endpoint address")
        );
        let media_server = thread::spawn(move || {
            let bodies = [
                r#"{"data":[{"errors":[{"code":2002}]}]}"#,
                r#"{"data":[{"media":[{"sources":[{"url":"https://e-cdns-proxy-a.dzcdn.net/mobile/1/fallback"}]}]}]}"#,
            ];
            for body in bodies {
                let (mut stream, _) = media_listener.accept().expect("accept media request");
                let _ = read_complete_http_request(&mut stream);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write media response");
            }
        });

        let gateway_listener = TcpListener::bind("127.0.0.1:0").expect("bind mock track gateway");
        let gateway_endpoint = format!(
            "http://{}",
            gateway_listener
                .local_addr()
                .expect("track gateway address")
        );
        let gateway_server = thread::spawn(move || {
            let (mut stream, _) = gateway_listener
                .accept()
                .expect("accept fallback metadata request");
            let _ = read_complete_http_request(&mut stream);
            let body = r#"{"error":[],"results":{"SNG_ID":2445708,"TRACK_TOKEN":"fallback-token","TRACK_TOKEN_EXPIRE":4102444800,"FILESIZE_MP3_320":654321}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write fallback metadata response");
        });

        let mut session = AuthenticatedSession {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(5))
                .redirects(0)
                .build(),
            api_token: Zeroizing::new("api-token".to_string()),
            license_token: Zeroizing::new("license-token".to_string()),
            can_stream_hq: true,
            can_stream_lossless: false,
        };
        let mut track = fixture_track(DownloadQuality::Mp3_320);
        track.fallback_id = Some("2445708".to_string());

        let media_url = authorize_track_media_at(
            &mut session,
            &mut track,
            &[DownloadQuality::Mp3_320],
            &gateway_endpoint,
            &media_endpoint,
            1_800_000_000,
        )
        .expect("authorize fallback media");

        assert_eq!(
            media_url,
            "https://e-cdns-proxy-a.dzcdn.net/mobile/1/fallback"
        );
        assert_eq!(track.id, "2445707");
        assert_eq!(track.media_id, "2445708");
        assert_eq!(track.track_token.as_str(), "fallback-token");
        assert_eq!(track.download_quality, DownloadQuality::Mp3_320);
        media_server.join().expect("mock media server");
        gateway_server.join().expect("mock gateway server");
    }

    #[test]
    fn expiring_track_tokens_are_refreshed_before_authorization() {
        let mut track = fixture_track(DownloadQuality::Mp3_320);
        track.track_token_expires_at = Some(1_000);
        assert!(track_token_is_expired(&track, 971));
        assert!(!track_token_is_expired(&track, 969));
        assert!(!track_token_is_expired(
            &fixture_track(DownloadQuality::Mp3_320),
            u64::MAX
        ));
    }

    #[test]
    fn decrypts_every_third_deezer_stream_chunk() {
        let track_id = "2445707";
        let key = blowfish_key(track_id);
        let iv = [0u8, 1, 2, 3, 4, 5, 6, 7];
        let plaintext = (0..(STREAM_CHUNK_SIZE * 4 + 73))
            .map(|index| ((index % 250) + 1) as u8)
            .collect::<Vec<_>>();
        let mut encrypted = plaintext.clone();
        for chunk_index in [0usize, 3usize] {
            let start = chunk_index * STREAM_CHUNK_SIZE;
            let end = start + STREAM_CHUNK_SIZE;
            cbc::Encryptor::<Blowfish>::new_from_slices(&key, &iv)
                .expect("cipher")
                .encrypt_padded_mut::<NoPadding>(&mut encrypted[start..end], STREAM_CHUNK_SIZE)
                .expect("encrypt");
        }
        let mut reader = Cursor::new(encrypted);
        let mut decrypted = Vec::new();
        let written = decrypt_stream(&mut reader, &mut decrypted, track_id).expect("decrypt");
        assert_eq!(written as usize, plaintext.len());
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn writes_complete_mp3_tags_and_embedded_cover() {
        let mut album = fixture_album();
        let track = fixture_track(DownloadQuality::Mp3_320);
        album.tracks.push(track.clone());
        let artwork = Artwork {
            bytes: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mime_type: "image/jpeg".to_string(),
            extension: "jpg",
        };
        let path = std::env::temp_dir().join(format!(
            "flipside-deemix-tag-test-{}-{}.mp3",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0, 0, 0, 0]).expect("seed mp3");
        write_mp3_tags(&path, &album, &track, Some(&artwork)).expect("write tags");

        let tag = Tag::read_from_path(&path).expect("read tags");
        assert_eq!(tag.title(), Some("In The Meantime"));
        assert_eq!(tag.artist(), Some("Helmet"));
        assert_eq!(tag.album(), Some("Meantime"));
        assert_eq!(tag.album_artist(), Some("Helmet"));
        assert_eq!(tag.track(), Some(1));
        assert_eq!(tag.total_tracks(), Some(1));
        assert_eq!(tag.disc(), Some(1));
        assert_eq!(tag.year(), Some(1992));
        assert_eq!(
            tag.get("TPUB").and_then(|frame| frame.content().text()),
            Some("Interscope")
        );
        assert_eq!(
            tag.get("TSRC").and_then(|frame| frame.content().text()),
            Some("USIR19200541")
        );
        assert_eq!(tag.pictures().count(), 1);
        fs::remove_file(&path).expect("remove tag test file");
    }

    #[test]
    fn writes_complete_mp3_tags_without_optional_artwork() {
        let mut album = fixture_album();
        album.cover_url = None;
        let track = fixture_track(DownloadQuality::Mp3_320);
        album.tracks.push(track.clone());
        let path = std::env::temp_dir().join(format!(
            "flipside-deemix-no-cover-tag-test-{}-{}.mp3",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0, 0, 0, 0]).expect("seed mp3");

        write_mp3_tags(&path, &album, &track, None).expect("write tags without artwork");

        let tag = Tag::read_from_path(&path).expect("read tags without artwork");
        assert_eq!(tag.album(), Some("Meantime"));
        assert_eq!(tag.pictures().count(), 0);
        fs::remove_file(&path).expect("remove no-cover tag test file");
    }

    #[test]
    fn writes_complete_flac_tags_and_embedded_cover() {
        let mut album = fixture_album();
        let track = fixture_track(DownloadQuality::Flac);
        album.tracks.push(track.clone());
        let artwork = Artwork {
            bytes: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mime_type: "image/jpeg".to_string(),
            extension: "jpg",
        };
        let path = std::env::temp_dir().join(format!(
            "flipside-deemix-tag-test-{}-{}.flac",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let mut minimal_flac = b"fLaC".to_vec();
        minimal_flac.extend_from_slice(&[0x80, 0x00, 0x00, 0x22]);
        minimal_flac.extend_from_slice(&[0; 34]);
        fs::write(&path, minimal_flac).expect("seed FLAC");
        write_flac_tags(&path, &album, &track, Some(&artwork)).expect("write FLAC tags");

        let tag = FlacTag::read_from_path(&path).expect("read FLAC tags");
        assert_eq!(
            tag.get_vorbis("TITLE")
                .expect("title tag")
                .collect::<Vec<_>>(),
            vec!["In The Meantime"]
        );
        assert_eq!(
            tag.get_vorbis("ALBUMARTIST")
                .expect("album artist tag")
                .collect::<Vec<_>>(),
            vec!["Helmet"]
        );
        assert_eq!(tag.pictures().count(), 1);
        fs::remove_file(&path).expect("remove FLAC tag test file");
    }

    fn fixture_track(download_quality: DownloadQuality) -> TrackMetadata {
        TrackMetadata {
            id: "2445707".to_string(),
            media_id: "2445707".to_string(),
            title: "In The Meantime".to_string(),
            artists: vec!["Helmet".to_string()],
            track_number: 1,
            disc_number: 1,
            duration_seconds: 188,
            isrc: Some("USIR19200541".to_string()),
            explicit: false,
            copyright: Some("1992 Interscope".to_string()),
            composers: vec!["Page Hamilton".to_string()],
            involved_people: vec![("producer".to_string(), "Steve Albini".to_string())],
            track_token: Zeroizing::new("token".to_string()),
            track_token_expires_at: None,
            fallback_id: None,
            download_quality,
        }
    }

    fn fixture_album() -> AlbumMetadata {
        AlbumMetadata {
            id: "240766".to_string(),
            title: "Meantime".to_string(),
            artist: "Helmet".to_string(),
            album_artists: vec!["Helmet".to_string()],
            release_date: Some("1992-06-23".to_string()),
            year: Some(1992),
            label: Some("Interscope".to_string()),
            genres: vec!["Alternative".to_string()],
            upc: Some("606949216221".to_string()),
            copyright: Some("1992 Interscope".to_string()),
            cover_url: Some(
                "https://cdn-images.dzcdn.net/images/cover/hash/1000x1000.jpg".to_string(),
            ),
            disc_total: 1,
            tracks: Vec::new(),
        }
    }
}
