use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use crc32fast::Hasher;
use keyring::{Entry, Error as KeyringError};
use native_tls::TlsConnector;
use quick_xml::de::from_str;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};
use url::Url;
use zeroize::Zeroizing;
use tauri_specta::Event as _;

const PROWLARR_CREDENTIAL_SERVICE: &str = "com.soundtrackgeek.flipside.usenet.prowlarr";
const NEWS_CREDENTIAL_SERVICE: &str = "com.soundtrackgeek.flipside.usenet.news";
const PROWLARR_CREDENTIAL_ACCOUNT: &str = "api-key";
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetProfile {
    pub prowlarr_url: String,
    pub news_host: String,
    pub news_port: u16,
    pub use_tls: bool,
    pub username: String,
    pub download_directory: String,
    pub connections: u8,
}

impl UsenetProfile {
    fn suggested(download_directory: &Path) -> Self {
        Self {
            prowlarr_url: "http://127.0.0.1:9696".to_owned(),
            news_host: "news.newsgroup.ninja".to_owned(),
            news_port: 563,
            use_tls: true,
            username: "soundtrackgeek".to_owned(),
            download_directory: download_directory.to_string_lossy().into_owned(),
            connections: 8,
        }
    }

    fn validate(&self) -> Result<()> {
        let url = Url::parse(self.prowlarr_url.trim())
            .context("Enter a valid Prowlarr URL, such as http://127.0.0.1:9696.")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            bail!("Prowlarr must use an http:// or https:// URL.");
        }
        if !url.username().is_empty() || url.password().is_some() {
            bail!("Remove embedded credentials from the Prowlarr URL.");
        }
        if self.news_host.trim().is_empty()
            || self.news_host.len() > 255
            || self.news_host.chars().any(char::is_whitespace)
        {
            bail!("Enter a valid Usenet server hostname.");
        }
        if self.news_port == 0 {
            bail!("Enter a valid Usenet server port.");
        }
        if self.username.trim().is_empty() {
            bail!("Enter the username supplied by your Usenet provider.");
        }
        if self.download_directory.trim().is_empty() {
            bail!("Choose a Usenet download folder.");
        }
        if !(1..=50).contains(&self.connections) {
            bail!("Usenet connections must be between 1 and 50.");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveUsenetProfileRequest {
    pub profile: UsenetProfile,
    pub prowlarr_api_key: Option<String>,
    pub news_password: Option<String>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetBootstrap {
    pub profile: UsenetProfile,
    pub has_prowlarr_api_key: bool,
    pub has_news_password: bool,
    pub extractor_path: Option<String>,
    pub par2_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetConnectionTest {
    pub prowlarr_version: String,
    pub news_server: String,
    pub extractor_path: Option<String>,
    pub par2_path: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetSearchRequest {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetSearchResult {
    pub guid: String,
    pub title: String,
    pub indexer: String,
    pub size_bytes: u64,
    pub age_days: i64,
    pub grabs: Option<i64>,
    pub publish_date: Option<String>,
    pub download_url: String,
    pub info_url: Option<String>,
    pub categories: Vec<String>,
    pub match_score: u8,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetSearchResponse {
    pub query: String,
    pub results: Vec<UsenetSearchResult>,
    pub searched_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetDownloadRequest {
    pub guid: String,
    pub title: String,
    pub indexer: String,
    pub download_url: String,
    pub size_bytes: u64,
    pub expected_artist: String,
    pub expected_album: String,
    pub expected_year: Option<i32>,
    pub release_group_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UsenetTransferStatus {
    Queued,
    FetchingNzb,
    Downloading,
    Verifying,
    Repairing,
    Extracting,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetTransfer {
    pub id: String,
    pub guid: String,
    pub title: String,
    pub indexer: String,
    pub status: UsenetTransferStatus,
    pub progress_percent: u8,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub message: String,
    pub destination_path: Option<String>,
    pub error: Option<String>,
    pub release_group_id: Option<String>,
    /// The original request, kept so a finished download can be started again.
    #[serde(default)]
    pub request: Option<UsenetDownloadRequest>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UsenetTransferQueue {
    pub transfers: Vec<UsenetTransfer>,
    pub active_count: usize,
}

#[derive(Clone)]
pub struct UsenetManager {
    app: AppHandle,
    profile_path: PathBuf,
    queue_path: PathBuf,
    suggested_profile: UsenetProfile,
    queue: Arc<Mutex<UsenetTransferQueue>>,
    connection_slots: Arc<(Mutex<u8>, Condvar)>,
}

pub fn initialize(app: &AppHandle) -> Result<UsenetManager, String> {
    let config_directory = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let download_directory = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| config_directory.clone())
        .join("_Usenet");
    let queue_path = config_directory.join("usenet-transfers.json");
    let mut queue = load_json::<UsenetTransferQueue>(&queue_path)
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    for transfer in &mut queue.transfers {
        if !matches!(
            transfer.status,
            UsenetTransferStatus::Completed | UsenetTransferStatus::Failed
        ) {
            transfer.status = UsenetTransferStatus::Failed;
            transfer.error =
                Some("The app closed before this Usenet download finished.".to_owned());
            transfer.message = "Interrupted before completion".to_owned();
            transfer.updated_at = Utc::now().to_rfc3339();
        }
    }
    queue.active_count = 0;
    Ok(UsenetManager {
        app: app.clone(),
        profile_path: config_directory.join("usenet.json"),
        queue_path,
        suggested_profile: UsenetProfile::suggested(&download_directory),
        queue: Arc::new(Mutex::new(queue)),
        connection_slots: Arc::new((Mutex::new(0), Condvar::new())),
    })
}

impl UsenetManager {
    fn profile(&self) -> Result<UsenetProfile> {
        Ok(load_json(&self.profile_path)?.unwrap_or_else(|| self.suggested_profile.clone()))
    }

    fn bootstrap(&self) -> Result<UsenetBootstrap> {
        let profile = self.profile()?;
        Ok(UsenetBootstrap {
            has_prowlarr_api_key: credential_has(
                PROWLARR_CREDENTIAL_SERVICE,
                PROWLARR_CREDENTIAL_ACCOUNT,
            )?,
            has_news_password: !profile.username.trim().is_empty()
                && credential_has(NEWS_CREDENTIAL_SERVICE, profile.username.trim())?,
            extractor_path: find_unrar().map(|path| path.to_string_lossy().into_owned()),
            par2_path: find_par2().map(|path| path.to_string_lossy().into_owned()),
            profile,
        })
    }

    fn save_profile(&self, mut request: SaveUsenetProfileRequest) -> Result<UsenetBootstrap> {
        let previous_profile = self.profile()?;
        request.profile.prowlarr_url = request
            .profile
            .prowlarr_url
            .trim()
            .trim_end_matches('/')
            .to_owned();
        request.profile.news_host = request.profile.news_host.trim().to_owned();
        request.profile.username = request.profile.username.trim().to_owned();
        request.profile.download_directory = request.profile.download_directory.trim().to_owned();
        request.profile.validate()?;
        let supplies_api_key = request
            .prowlarr_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if !supplies_api_key
            && !credential_has(PROWLARR_CREDENTIAL_SERVICE, PROWLARR_CREDENTIAL_ACCOUNT)?
        {
            bail!("Enter the Prowlarr API key.");
        }
        let supplies_news_password = request
            .news_password
            .as_deref()
            .is_some_and(|value| !value.is_empty());
        if !supplies_news_password
            && !credential_has(NEWS_CREDENTIAL_SERVICE, request.profile.username.as_str())?
        {
            bail!("Enter the password for this Usenet username.");
        }
        if let Some(api_key) = request.prowlarr_api_key.take() {
            if !api_key.trim().is_empty() {
                credential_store(
                    PROWLARR_CREDENTIAL_SERVICE,
                    PROWLARR_CREDENTIAL_ACCOUNT,
                    api_key.trim(),
                )?;
            }
        }
        if let Some(password) = request.news_password.take() {
            if !password.is_empty() {
                credential_store(
                    NEWS_CREDENTIAL_SERVICE,
                    request.profile.username.as_str(),
                    &password,
                )?;
            }
        }
        if !credential_has(PROWLARR_CREDENTIAL_SERVICE, PROWLARR_CREDENTIAL_ACCOUNT)? {
            bail!("Enter the Prowlarr API key.");
        }
        if !credential_has(NEWS_CREDENTIAL_SERVICE, request.profile.username.as_str())? {
            bail!("Enter the password for this Usenet username.");
        }
        save_json(&self.profile_path, &request.profile)?;
        if previous_profile.username != request.profile.username
            && !previous_profile.username.trim().is_empty()
        {
            credential_delete(NEWS_CREDENTIAL_SERVICE, previous_profile.username.trim())?;
        }
        self.bootstrap()
    }

    fn reset(&self) -> Result<UsenetBootstrap> {
        let profile = self.profile()?;
        credential_delete(PROWLARR_CREDENTIAL_SERVICE, PROWLARR_CREDENTIAL_ACCOUNT)?;
        if !profile.username.trim().is_empty() {
            credential_delete(NEWS_CREDENTIAL_SERVICE, profile.username.trim())?;
        }
        if self.profile_path.exists() {
            fs::remove_file(&self.profile_path).context("Could not remove Usenet settings")?;
        }
        self.bootstrap()
    }

    fn credentials(
        &self,
        profile: &UsenetProfile,
    ) -> Result<(Zeroizing<String>, Zeroizing<String>)> {
        let api_key = credential_get(PROWLARR_CREDENTIAL_SERVICE, PROWLARR_CREDENTIAL_ACCOUNT)?
            .ok_or_else(|| anyhow!("Enter the Prowlarr API key in Settings → Providers."))?;
        let news_password = credential_get(NEWS_CREDENTIAL_SERVICE, profile.username.trim())?
            .ok_or_else(|| {
                anyhow!("Enter the Newsgroup Ninja password in Settings → Providers.")
            })?;
        Ok((api_key, news_password))
    }

    fn test_connections(&self) -> Result<UsenetConnectionTest> {
        let profile = self.profile()?;
        profile.validate()?;
        let (api_key, news_password) = self.credentials(&profile)?;
        let status: ProwlarrStatus =
            prowlarr_get_json(&profile, &api_key, "api/v1/system/status", &[])?;
        let mut connection = NntpConnection::connect(&profile)?;
        connection.authenticate(&profile.username, &news_password)?;
        let extractor = find_unrar().map(|path| path.to_string_lossy().into_owned());
        let par2 = find_par2().map(|path| path.to_string_lossy().into_owned());
        let message = match (extractor.is_some(), par2.is_some()) {
            (true, true) => {
                "Prowlarr search, Newsgroup Ninja authentication, PAR2 repair, and UnRAR are ready."
            }
            (true, false) => {
                "Search, NNTP authentication, and UnRAR work. Install par2cmdline-turbo to repair incomplete releases."
            }
            (false, true) => {
                "Search, NNTP authentication, and PAR2 repair work. Install UnRAR to unpack RAR releases automatically."
            }
            (false, false) => {
                "Search and NNTP authentication work. Install par2cmdline-turbo and UnRAR for repair and extraction."
            }
        }
        .to_owned();
        Ok(UsenetConnectionTest {
            prowlarr_version: status.version,
            news_server: format!("{}:{}", profile.news_host, profile.news_port),
            extractor_path: extractor,
            par2_path: par2,
            message,
        })
    }

    fn search(&self, request: UsenetSearchRequest) -> Result<UsenetSearchResponse> {
        let profile = self.profile()?;
        profile.validate()?;
        let (api_key, _) = self.credentials(&profile)?;
        let query = [request.artist.trim(), request.title.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if query.is_empty() {
            bail!("Enter an artist or album to search Usenet.");
        }
        let rows: Vec<ProwlarrSearchRow> = prowlarr_get_json(
            &profile,
            &api_key,
            "api/v1/search",
            &[
                ("query", query.as_str()),
                ("categories", "3000"),
                ("type", "search"),
            ],
        )?;
        let mut results = rows
            .into_iter()
            .filter(|row| row.protocol.eq_ignore_ascii_case("usenet"))
            .filter(|row| !row.download_url.trim().is_empty())
            .map(|row| UsenetSearchResult {
                match_score: release_match_score(
                    &row.title,
                    request.artist.as_str(),
                    request.title.as_str(),
                    request.year,
                ),
                guid: row.guid,
                title: row.title,
                indexer: row.indexer,
                size_bytes: row.size,
                age_days: row.age.unwrap_or(0),
                grabs: row.grabs,
                publish_date: row.publish_date,
                download_url: row.download_url,
                info_url: row.info_url.filter(|value| !value.trim().is_empty()),
                categories: row
                    .categories
                    .into_iter()
                    .map(|category| category.name)
                    .filter(|name| !name.trim().is_empty())
                    .collect(),
            })
            .collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .match_score
                .cmp(&left.match_score)
                .then_with(|| right.grabs.unwrap_or(0).cmp(&left.grabs.unwrap_or(0)))
                .then_with(|| left.age_days.cmp(&right.age_days))
        });
        results.truncate(request.limit.unwrap_or(30).clamp(1, 100));
        Ok(UsenetSearchResponse {
            query,
            results,
            searched_at: Utc::now().to_rfc3339(),
        })
    }

    fn snapshot(&self) -> UsenetTransferQueue {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn enqueue(&self, request: UsenetDownloadRequest) -> Result<UsenetTransferQueue> {
        let profile = self.profile()?;
        profile.validate()?;
        let _ = self.credentials(&profile)?;
        if request.guid.trim().is_empty() || request.download_url.trim().is_empty() {
            bail!("The selected Prowlarr result does not contain an NZB download link.");
        }
        if self.snapshot().transfers.iter().any(|transfer| {
            transfer.guid == request.guid
                && !matches!(
                    transfer.status,
                    UsenetTransferStatus::Completed | UsenetTransferStatus::Failed
                )
        }) {
            bail!("This Usenet release is already queued or downloading.");
        }
        let id = format!(
            "usenet-{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            self.snapshot().transfers.len() + 1
        );
        let now = Utc::now().to_rfc3339();
        {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue.transfers.push(UsenetTransfer {
                id: id.clone(),
                guid: request.guid.clone(),
                title: request.title.clone(),
                indexer: request.indexer.clone(),
                status: UsenetTransferStatus::Queued,
                progress_percent: 0,
                downloaded_bytes: 0,
                total_bytes: request.size_bytes,
                message: "Queued for Usenet download".to_owned(),
                destination_path: None,
                error: None,
                release_group_id: request.release_group_id.clone(),
                request: Some(request.clone()),
                created_at: now.clone(),
                updated_at: now,
            });
            queue.active_count += 1;
            self.persist_locked(&queue)?;
        }
        self.emit();
        let manager = self.clone();
        let worker_id = id.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("usenet-download-{id}"))
            .spawn(move || {
                if let Err(error) = manager.download_release(&worker_id, request) {
                    manager.finish_failed(&worker_id, error.to_string());
                }
            })
        {
            self.finish_failed(
                &id,
                format!("Could not start the Usenet download worker: {error}"),
            );
            bail!("Could not start the Usenet download worker: {error}");
        }
        Ok(self.snapshot())
    }

    fn retry(&self, id: &str) -> Result<UsenetTransferQueue> {
        let request = self
            .snapshot()
            .transfers
            .into_iter()
            .find(|transfer| transfer.id == id)
            .and_then(|transfer| transfer.request)
            .ok_or_else(|| anyhow!("This download was started by an older version and cannot be repeated; fetch the album again from its page."))?;
        self.enqueue(request)
    }

    fn clear_completed(&self) -> Result<UsenetTransferQueue> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut removed_guids = queue
            .transfers
            .iter()
            .filter(|transfer| {
                matches!(
                    transfer.status,
                    UsenetTransferStatus::Completed | UsenetTransferStatus::Failed
                )
            })
            .map(|transfer| transfer.guid.clone())
            .collect::<Vec<_>>();
        queue.transfers.retain(|transfer| {
            !matches!(
                transfer.status,
                UsenetTransferStatus::Completed | UsenetTransferStatus::Failed
            )
        });
        let remaining_guids = queue
            .transfers
            .iter()
            .map(|transfer| transfer.guid.clone())
            .collect::<Vec<_>>();
        self.persist_locked(&queue)?;
        drop(queue);
        removed_guids.sort();
        removed_guids.dedup();
        if let Ok(profile) = self.profile() {
            for guid in removed_guids {
                if !remaining_guids.contains(&guid) {
                    let stage = release_stage_path(&profile.download_directory, &guid);
                    if stage.exists() {
                        let _ = fs::remove_dir_all(stage);
                    }
                }
            }
        }
        self.emit();
        Ok(self.snapshot())
    }

    fn download_release(&self, id: &str, request: UsenetDownloadRequest) -> Result<()> {
        let profile = self.profile()?;
        let (api_key, news_password) = self.credentials(&profile)?;
        self.update(
            id,
            UsenetTransferStatus::FetchingNzb,
            1,
            0,
            "Fetching NZB from Prowlarr",
            None,
        )?;
        let nzb_xml = prowlarr_download(&profile, &api_key, &request.download_url)?;
        let nzb: Nzb = from_str(&nzb_xml).context("Prowlarr returned an invalid NZB document")?;
        if nzb.files.is_empty() {
            bail!("The NZB does not contain any files.");
        }
        let total_bytes = nzb
            .files
            .iter()
            .flat_map(|file| file.segments.segments.iter())
            .map(|segment| segment.bytes)
            .sum::<u64>()
            .max(1);
        let total_files = nzb.files.len();
        let mut payload_files = Vec::new();
        let mut optional_files = Vec::new();
        let mut par2_indexes = Vec::new();
        let mut par2_volumes = Vec::new();
        for file in nzb.files {
            match file.kind() {
                NzbFileKind::Payload => payload_files.push(file),
                NzbFileKind::Optional => optional_files.push(file),
                NzbFileKind::Par2Index => par2_indexes.push(file),
                NzbFileKind::Par2Volume => par2_volumes.push(file),
            }
        }
        par2_volumes.sort_by_key(|file| {
            par2_recovery_blocks(&file.suggested_filename()).unwrap_or(u32::MAX)
        });

        let stage = release_stage_path(&profile.download_directory, &request.guid);
        let files = stage.join("files");
        fs::create_dir_all(&files).context("Could not create the Usenet staging folder")?;

        let mut downloaded = 0u64;
        let mut ordinal = 0usize;
        let par2 = find_par2();
        let mut par2_reference = first_par2_file(&files)?;
        let par2_expected = !par2_indexes.is_empty() || !par2_volumes.is_empty();
        let mut critical_failures = Vec::new();
        let mut optional_failure_count = 0usize;
        let mut par2_repaired = false;
        let mut par2_verified = false;
        let mut last_par2_detail = None;

        if let (Some(executable), Some(reference)) = (par2.as_deref(), par2_reference.as_deref()) {
            self.update(
                id,
                UsenetTransferStatus::Verifying,
                2,
                0,
                "Checking preserved PAR2 recovery staging",
                None,
            )?;
            let verification = run_par2(executable, "v", reference, &files)?;
            par2_verified = verification.success;
            last_par2_detail = Some(verification.detail);
            if !par2_verified {
                let repair = run_par2(executable, "r", reference, &files)?;
                par2_repaired = repair.success;
                last_par2_detail = Some(repair.detail);
            }
            if par2_verified || par2_repaired {
                downloaded = total_bytes;
            }
        }

        if !par2_verified && !par2_repaired {
            for nzb_file in par2_indexes {
                ordinal += 1;
                let filename = nzb_file.suggested_filename();
                let report = self.download_release_file(
                    id,
                    &profile,
                    &news_password,
                    &files,
                    nzb_file,
                    UsenetTransferStatus::Downloading,
                    format!("Downloading PAR2 index · {}", compact_subject(&filename)),
                    total_bytes,
                    &mut downloaded,
                )?;
                if par2_reference.is_none() {
                    par2_reference = report.path.clone();
                }
            }

            for nzb_file in payload_files {
                ordinal += 1;
                let filename = nzb_file.suggested_filename();
                let report = self.download_release_file(
                    id,
                    &profile,
                    &news_password,
                    &files,
                    nzb_file,
                    UsenetTransferStatus::Downloading,
                    format!(
                        "Downloading file {ordinal}/{total_files} · {}",
                        compact_subject(&filename)
                    ),
                    total_bytes,
                    &mut downloaded,
                )?;
                if !report.complete {
                    critical_failures.push(format!(
                        "{}: {} ({} of {} segments unavailable)",
                        compact_subject(&filename),
                        report
                            .first_error
                            .as_deref()
                            .unwrap_or("incomplete Usenet file"),
                        report.failed_segments,
                        report.successful_segments + report.failed_segments
                    ));
                }
            }

            for nzb_file in optional_files {
                ordinal += 1;
                let filename = nzb_file.suggested_filename();
                let report = self.download_release_file(
                    id,
                    &profile,
                    &news_password,
                    &files,
                    nzb_file,
                    UsenetTransferStatus::Downloading,
                    format!(
                        "Downloading optional file {ordinal}/{total_files} · {}",
                        compact_subject(&filename)
                    ),
                    total_bytes,
                    &mut downloaded,
                )?;
                if !report.complete {
                    optional_failure_count += 1;
                }
            }
        }

        if !par2_verified && !par2_repaired && par2_reference.is_none() {
            par2_reference = first_par2_file(&files)?;
        }
        if !par2_verified && !par2_repaired {
            if let (Some(executable), Some(reference)) =
                (par2.as_deref(), par2_reference.as_deref())
            {
                self.update(
                    id,
                    UsenetTransferStatus::Verifying,
                    progress_percent(downloaded, total_bytes),
                    downloaded,
                    "Verifying downloaded files with PAR2",
                    None,
                )?;
                let verification = run_par2(executable, "v", reference, &files)?;
                par2_verified = verification.success;
                last_par2_detail = Some(verification.detail);
                if !par2_verified {
                    self.update(
                        id,
                        UsenetTransferStatus::Repairing,
                        progress_percent(downloaded, total_bytes),
                        downloaded,
                        "Repairing incomplete files with available PAR2 data",
                        None,
                    )?;
                    let repair = run_par2(executable, "r", reference, &files)?;
                    par2_repaired = repair.success;
                    last_par2_detail = Some(repair.detail);
                }
            }
        }

        for nzb_file in par2_volumes {
            if par2_verified || par2_repaired {
                break;
            }
            ordinal += 1;
            let filename = nzb_file.suggested_filename();
            let report = self.download_release_file(
                id,
                &profile,
                &news_password,
                &files,
                nzb_file,
                UsenetTransferStatus::Repairing,
                format!(
                    "Fetching PAR2 recovery data {ordinal}/{total_files} · {}",
                    compact_subject(&filename)
                ),
                total_bytes,
                &mut downloaded,
            )?;
            if par2_reference.is_none() {
                par2_reference = report.path.clone();
            }
            let (Some(executable), Some(reference)) = (par2.as_deref(), par2_reference.as_deref())
            else {
                continue;
            };
            self.update(
                id,
                UsenetTransferStatus::Repairing,
                progress_percent(downloaded, total_bytes),
                downloaded,
                "Repairing incomplete files with PAR2",
                None,
            )?;
            let repair = run_par2(executable, "r", reference, &files)?;
            par2_repaired = repair.success;
            last_par2_detail = Some(repair.detail);
        }

        if par2_expected && par2.is_some() && !par2_verified && !par2_repaired {
            let detail = last_par2_detail
                .as_deref()
                .unwrap_or("No usable PAR2 index or recovery volume was downloaded");
            bail!("PAR2 could not verify or repair this release: {detail}");
        }
        if !critical_failures.is_empty() && !par2_repaired {
            if par2.is_none() {
                bail!(
                    "The release has missing or corrupt payload segments, but par2.exe was not found. Staging was preserved for repair."
                );
            }
            if !par2_expected {
                bail!(
                    "The release has missing or corrupt payload segments and its NZB contains no PAR2 recovery data. {}",
                    critical_failures[0]
                );
            }
            bail!(
                "PAR2 recovery did not repair every payload file. {}",
                critical_failures[0]
            );
        }

        let payload = if let Some(archive) = first_rar_archive(&files)? {
            let extractor = find_unrar().ok_or_else(|| {
                anyhow!("This release is RAR-compressed. Install UnRAR and retry the download.")
            })?;
            self.update(
                id,
                UsenetTransferStatus::Extracting,
                99,
                total_bytes,
                "Verifying and unpacking the RAR release",
                None,
            )?;
            let extracted = stage.join("extracted");
            if extracted.exists() {
                fs::remove_dir_all(&extracted)
                    .context("Could not reset the Usenet extraction folder")?;
            }
            fs::create_dir_all(&extracted).context("Could not create the extraction folder")?;
            let output = Command::new(&extractor)
                .arg("x")
                .arg("-o+")
                .arg("-y")
                .arg(&archive)
                .arg(format!(
                    "{}{}",
                    extracted.display(),
                    std::path::MAIN_SEPARATOR
                ))
                .current_dir(&files)
                .output()
                .with_context(|| format!("Could not start {}", extractor.display()))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                bail!(
                    "UnRAR could not verify or unpack the release: {}",
                    stderr
                        .trim()
                        .lines()
                        .last()
                        .or_else(|| stdout.trim().lines().last())
                        .unwrap_or("archive error")
                );
            }
            select_payload_root(&extracted)?
        } else {
            remove_par2_files(&files)?;
            files.clone()
        };

        let destination_parent = PathBuf::from(&profile.download_directory);
        fs::create_dir_all(&destination_parent)
            .context("Could not create the Usenet download folder")?;
        let folder_name = album_folder_name(
            &request.expected_artist,
            &request.expected_album,
            request.expected_year,
            &request.title,
        );
        let destination = unique_destination(&destination_parent, &folder_name);
        move_payload(&payload, &destination)?;
        if stage.exists() {
            let _ = fs::remove_dir_all(&stage);
        }
        self.update(
            id,
            UsenetTransferStatus::Completed,
            100,
            total_bytes,
            if par2_repaired {
                "Usenet download repaired and completed"
            } else if optional_failure_count > 0 && !par2_verified {
                "Usenet download completed without unavailable optional metadata"
            } else {
                "Usenet download verified and completed"
            },
            Some(destination.to_string_lossy().into_owned()),
        )?;
        self.decrement_active();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn download_release_file(
        &self,
        id: &str,
        profile: &UsenetProfile,
        news_password: &str,
        directory: &Path,
        nzb_file: NzbFile,
        status: UsenetTransferStatus,
        message: String,
        total_bytes: u64,
        downloaded: &mut u64,
    ) -> Result<NzbFileDownload> {
        let file_bytes = nzb_file.encoded_bytes();
        self.update(
            id,
            status.clone(),
            progress_percent(*downloaded, total_bytes),
            *downloaded,
            &message,
            None,
        )?;
        let manager = self.clone();
        let transfer_id = id.to_owned();
        let start_bytes = *downloaded;
        let progress_status = status;
        let progress_message = message;
        let report = download_nzb_file(
            profile,
            news_password,
            directory,
            nzb_file,
            Arc::clone(&self.connection_slots),
            move |file_downloaded| {
                let current = start_bytes.saturating_add(file_downloaded.min(file_bytes));
                let _ = manager.update(
                    &transfer_id,
                    progress_status.clone(),
                    progress_percent(current, total_bytes),
                    current,
                    &progress_message,
                    None,
                );
            },
        )?;
        *downloaded = start_bytes.saturating_add(report.downloaded_source_bytes.min(file_bytes));
        Ok(report)
    }

    fn update(
        &self,
        id: &str,
        status: UsenetTransferStatus,
        percent: u8,
        downloaded_bytes: u64,
        message: &str,
        destination_path: Option<String>,
    ) -> Result<()> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(transfer) = queue
            .transfers
            .iter_mut()
            .find(|transfer| transfer.id == id)
        {
            transfer.status = status;
            transfer.progress_percent = percent.min(100);
            transfer.downloaded_bytes = downloaded_bytes;
            transfer.message = message.to_owned();
            transfer.destination_path =
                destination_path.or_else(|| transfer.destination_path.clone());
            transfer.updated_at = Utc::now().to_rfc3339();
        }
        self.persist_locked(&queue)?;
        drop(queue);
        self.emit();
        Ok(())
    }

    fn finish_failed(&self, id: &str, error: String) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(transfer) = queue
            .transfers
            .iter_mut()
            .find(|transfer| transfer.id == id)
        {
            transfer.status = UsenetTransferStatus::Failed;
            transfer.message = "Usenet download failed; recovery staging preserved".to_owned();
            transfer.error = Some(error);
            transfer.updated_at = Utc::now().to_rfc3339();
        }
        queue.active_count = queue.active_count.saturating_sub(1);
        let _ = self.persist_locked(&queue);
        drop(queue);
        self.emit();
    }

    fn decrement_active(&self) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.active_count = queue.active_count.saturating_sub(1);
        let _ = self.persist_locked(&queue);
        drop(queue);
        self.emit();
    }

    fn persist_locked(&self, queue: &UsenetTransferQueue) -> Result<()> {
        save_json(&self.queue_path, queue)
    }

    fn emit(&self) {
        let _ = crate::events::UsenetTransfersChanged(self.snapshot()).emit(&self.app);
    }
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_bootstrap(
    manager: State<'_, UsenetManager>,
) -> Result<UsenetBootstrap, String> {
    manager.bootstrap().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_save_profile(
    manager: State<'_, UsenetManager>,
    request: SaveUsenetProfileRequest,
) -> Result<UsenetBootstrap, String> {
    manager
        .save_profile(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_reset(manager: State<'_, UsenetManager>) -> Result<UsenetBootstrap, String> {
    manager.reset().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_test_connections(
    manager: State<'_, UsenetManager>,
) -> Result<UsenetConnectionTest, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.test_connections())
        .await
        .map_err(|error| format!("Usenet connection test task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_search(
    manager: State<'_, UsenetManager>,
    request: UsenetSearchRequest,
) -> Result<UsenetSearchResponse, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.search(request))
        .await
        .map_err(|error| format!("Usenet search task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_transfers_snapshot(
    manager: State<'_, UsenetManager>,
) -> Result<UsenetTransferQueue, String> {
    Ok(manager.snapshot())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_enqueue_download(
    manager: State<'_, UsenetManager>,
    request: UsenetDownloadRequest,
) -> Result<UsenetTransferQueue, String> {
    manager.enqueue(request).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_retry_download(
    manager: State<'_, UsenetManager>,
    id: String,
) -> Result<UsenetTransferQueue, String> {
    manager.retry(&id).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usenet_clear_completed(
    manager: State<'_, UsenetManager>,
) -> Result<UsenetTransferQueue, String> {
    manager.clear_completed().map_err(|error| error.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProwlarrStatus {
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProwlarrSearchRow {
    #[serde(default)]
    guid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    indexer: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    age: Option<i64>,
    #[serde(default)]
    grabs: Option<i64>,
    #[serde(default)]
    publish_date: Option<String>,
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    info_url: Option<String>,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    categories: Vec<ProwlarrCategory>,
}

#[derive(Deserialize)]
struct ProwlarrCategory {
    #[serde(default)]
    name: String,
}

fn prowlarr_get_json<T: for<'de> Deserialize<'de>>(
    profile: &UsenetProfile,
    api_key: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<T> {
    let mut url = base_url(profile)?
        .join(path)
        .context("Could not build the Prowlarr API URL")?;
    url.query_pairs_mut().extend_pairs(query.iter().copied());
    let response = crate::http::get(url.as_str())
        .set("X-Api-Key", api_key)
        .timeout(Duration::from_secs(45))
        .call()
        .map_err(describe_ureq_error)?;
    response
        .into_json::<T>()
        .context("Prowlarr returned an unexpected response")
}

fn prowlarr_download(profile: &UsenetProfile, api_key: &str, download_url: &str) -> Result<String> {
    let base = base_url(profile)?;
    let url = match Url::parse(download_url) {
        Ok(url) => url,
        Err(_) => base
            .join(download_url)
            .context("Prowlarr returned an invalid NZB link")?,
    };
    if url.scheme() != base.scheme()
        || url.host_str() != base.host_str()
        || url.port_or_known_default() != base.port_or_known_default()
    {
        bail!("Prowlarr returned an NZB link on a different server; the API key was not sent.");
    }
    let response = crate::http::get(url.as_str())
        .without_retries()
        .set("X-Api-Key", api_key)
        .timeout(Duration::from_secs(60))
        .call()
        .map_err(describe_ureq_error)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(32 * 1_024 * 1_024)
        .read_to_end(&mut bytes)
        .context("Could not read the NZB from Prowlarr")?;
    String::from_utf8(bytes).context("Prowlarr returned an NZB that is not UTF-8 XML")
}

fn base_url(profile: &UsenetProfile) -> Result<Url> {
    let mut value = profile.prowlarr_url.trim().to_owned();
    if !value.ends_with('/') {
        value.push('/');
    }
    Url::parse(&value).context("The Prowlarr URL is invalid")
}

fn describe_ureq_error(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Status(401, _) => anyhow!("Prowlarr rejected the API key."),
        ureq::Error::Status(code, _) => anyhow!("Prowlarr returned HTTP {code}."),
        ureq::Error::Transport(error) => anyhow!("Could not reach Prowlarr: {error}"),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename = "nzb")]
struct Nzb {
    #[serde(rename = "file", default)]
    files: Vec<NzbFile>,
}

#[derive(Debug, Deserialize)]
struct NzbFile {
    #[serde(rename = "@subject", default)]
    subject: String,
    segments: NzbSegments,
}

#[derive(Debug, Deserialize)]
struct NzbSegments {
    #[serde(rename = "segment", default)]
    segments: Vec<NzbSegment>,
}

#[derive(Clone, Debug, Deserialize)]
struct NzbSegment {
    #[serde(rename = "@bytes", default)]
    bytes: u64,
    #[serde(rename = "@number", default)]
    number: u32,
    #[serde(rename = "$text", default)]
    message_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NzbFileKind {
    Payload,
    Optional,
    Par2Index,
    Par2Volume,
}

impl NzbFile {
    fn suggested_filename(&self) -> String {
        subject_filename(&self.subject)
    }

    fn kind(&self) -> NzbFileKind {
        let filename = self.suggested_filename().to_lowercase();
        if filename.ends_with(".par2") {
            if par2_recovery_blocks(&filename).is_some() {
                NzbFileKind::Par2Volume
            } else {
                NzbFileKind::Par2Index
            }
        } else if Path::new(&filename)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension,
                    "nfo"
                        | "sfv"
                        | "srr"
                        | "nzb"
                        | "txt"
                        | "url"
                        | "m3u"
                        | "m3u8"
                        | "cue"
                        | "log"
                        | "jpg"
                        | "jpeg"
                        | "png"
                        | "gif"
                )
            })
        {
            NzbFileKind::Optional
        } else {
            NzbFileKind::Payload
        }
    }

    fn encoded_bytes(&self) -> u64 {
        self.segments
            .segments
            .iter()
            .map(|segment| segment.bytes)
            .sum()
    }
}

#[derive(Debug)]
struct NzbFileDownload {
    path: Option<PathBuf>,
    complete: bool,
    successful_segments: usize,
    failed_segments: usize,
    downloaded_source_bytes: u64,
    first_error: Option<String>,
}

#[derive(Debug)]
struct DecodedSegment {
    number: u32,
    filename: String,
    begin: Option<u64>,
    total_size: Option<u64>,
    bytes: Vec<u8>,
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

struct NntpConnection {
    stream: BufReader<Box<dyn ReadWrite + Send>>,
}

impl NntpConnection {
    fn connect(profile: &UsenetProfile) -> Result<Self> {
        let address = (profile.news_host.as_str(), profile.news_port)
            .to_socket_addrs()
            .context("Could not resolve the Usenet server")?
            .next()
            .ok_or_else(|| anyhow!("The Usenet server did not resolve to an address."))?;
        let tcp =
            TcpStream::connect_timeout(&address, Duration::from_secs(15)).with_context(|| {
                format!(
                    "Could not connect to {}:{}",
                    profile.news_host, profile.news_port
                )
            })?;
        tcp.set_read_timeout(Some(Duration::from_secs(45)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(20)))?;
        let stream: Box<dyn ReadWrite + Send> = if profile.use_tls {
            let connector = TlsConnector::new().context("Could not initialize TLS")?;
            Box::new(
                connector
                    .connect(profile.news_host.as_str(), tcp)
                    .context("The TLS connection to the Usenet server failed")?,
            )
        } else {
            Box::new(tcp)
        };
        let mut connection = Self {
            stream: BufReader::new(stream),
        };
        let greeting = connection.read_status()?;
        if !matches!(greeting.0, 200 | 201) {
            bail!("The Usenet server rejected the connection: {}", greeting.1);
        }
        Ok(connection)
    }

    fn authenticate(&mut self, username: &str, password: &str) -> Result<()> {
        let user = self.command(&format!("AUTHINFO USER {username}"))?;
        match user.0 {
            281 => Ok(()),
            381 => {
                let pass = self.command(&format!("AUTHINFO PASS {password}"))?;
                if pass.0 == 281 {
                    Ok(())
                } else {
                    bail!(
                        "Newsgroup Ninja rejected the username or password: {}",
                        pass.1
                    )
                }
            }
            _ => bail!("Newsgroup Ninja rejected the username: {}", user.1),
        }
    }

    fn body(&mut self, message_id: &str) -> Result<Vec<Vec<u8>>> {
        let message_id = message_id
            .trim()
            .trim_matches(|character| character == '<' || character == '>');
        let status = self.command(&format!("BODY <{message_id}>"))?;
        if status.0 != 222 {
            bail!("Article <{message_id}> is unavailable: {}", status.1);
        }
        let mut lines = Vec::new();
        let mut total = 0usize;
        loop {
            let mut line = Vec::new();
            let count = self
                .stream
                .read_until(b'\n', &mut line)
                .context("The NNTP article ended unexpectedly")?;
            if count == 0 {
                bail!("The NNTP article ended unexpectedly.");
            }
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            if line == b"." {
                break;
            }
            if line.starts_with(b"..") {
                line.remove(0);
            }
            total = total.saturating_add(line.len());
            if total > 16 * 1_024 * 1_024 {
                bail!("A Usenet article exceeded the safe 16 MB segment limit.");
            }
            lines.push(line);
        }
        Ok(lines)
    }

    fn command(&mut self, command: &str) -> Result<(u16, String)> {
        let stream = self.stream.get_mut();
        stream.write_all(command.as_bytes())?;
        stream.write_all(b"\r\n")?;
        stream.flush()?;
        self.read_status()
    }

    fn read_status(&mut self) -> Result<(u16, String)> {
        let mut line = String::new();
        self.stream
            .read_line(&mut line)
            .context("Could not read the NNTP response")?;
        let code = line
            .get(..3)
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| anyhow!("The Usenet server returned an invalid response."))?;
        Ok((code, line.trim().to_owned()))
    }
}

fn download_nzb_file(
    profile: &UsenetProfile,
    password: &str,
    stage: &Path,
    mut nzb_file: NzbFile,
    connection_slots: Arc<(Mutex<u8>, Condvar)>,
    on_progress: impl Fn(u64) + Send + Sync + 'static,
) -> Result<NzbFileDownload> {
    if nzb_file.segments.segments.is_empty() {
        bail!("An NZB file entry has no article segments.");
    }
    let suggested_filename = nzb_file.suggested_filename();
    nzb_file
        .segments
        .segments
        .sort_by_key(|segment| segment.number);
    let segments = Arc::new(nzb_file.segments.segments);
    let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (sender, receiver) = mpsc::channel();
    let workers = usize::from(profile.connections).min(segments.len()).max(1);
    for _ in 0..workers {
        let profile = profile.clone();
        let password = Zeroizing::new(password.to_owned());
        let segments = Arc::clone(&segments);
        let next = Arc::clone(&next);
        let sender = sender.clone();
        let connection_slots = Arc::clone(&connection_slots);
        thread::spawn(move || {
            let _connection_slot = ConnectionSlot::acquire(connection_slots, profile.connections);
            let mut connection =
                match NntpConnection::connect(&profile).and_then(|mut connection| {
                    connection.authenticate(&profile.username, &password)?;
                    Ok(connection)
                }) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        return;
                    }
                };
            loop {
                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(segment) = segments.get(index) else {
                    break;
                };
                let result = connection
                    .body(&segment.message_id)
                    .and_then(|lines| decode_yenc(segment.number, &lines));
                if sender.send(result).is_err() {
                    break;
                }
            }
        });
    }
    drop(sender);
    let temporary_path = stage.join(".assembling-usenet-file.part");
    let mut file = File::create(&temporary_path)
        .with_context(|| format!("Could not create {}", temporary_path.display()))?;
    let mut decoded_count = 0usize;
    let mut sequential_segments = Vec::new();
    let mut filename = None;
    let mut total_size = None;
    let mut has_positioned_segments = false;
    let mut downloaded = 0u64;
    let mut first_error: Option<String> = None;
    for result in receiver {
        match result {
            Ok(segment) => {
                if let Some(source) = segments
                    .iter()
                    .find(|source| source.number == segment.number)
                {
                    downloaded = downloaded.saturating_add(source.bytes);
                    on_progress(downloaded);
                }
                if filename.is_none() && !segment.filename.is_empty() {
                    filename = Some(segment.filename.clone());
                }
                if total_size.is_none() {
                    total_size = segment.total_size;
                }
                if let Some(begin) = segment.begin {
                    has_positioned_segments = true;
                    file.seek(SeekFrom::Start(begin.saturating_sub(1)))?;
                    file.write_all(&segment.bytes)?;
                } else {
                    sequential_segments.push(segment);
                }
                decoded_count += 1;
            }
            Err(error) => {
                first_error.get_or_insert_with(|| error.to_string());
            }
        };
    }
    let complete = decoded_count == segments.len();
    let sequential_partial = !complete && !sequential_segments.is_empty();
    if has_positioned_segments && !sequential_segments.is_empty() {
        bail!("A Usenet file mixed positioned and sequential yEnc segments.");
    }
    if !sequential_partial {
        sequential_segments.sort_by_key(|segment| segment.number);
        let mut sequential_offset = 0u64;
        for segment in sequential_segments {
            file.seek(SeekFrom::Start(sequential_offset))?;
            file.write_all(&segment.bytes)?;
            sequential_offset = sequential_offset.saturating_add(segment.bytes.len() as u64);
        }
    }
    if has_positioned_segments {
        if let Some(total_size) = total_size {
            file.set_len(total_size)?;
        }
    }
    file.flush()?;
    drop(file);
    let path = if decoded_count == 0 || sequential_partial {
        let _ = fs::remove_file(&temporary_path);
        None
    } else {
        let filename = sanitize_filename(filename.as_deref().unwrap_or(&suggested_filename));
        let path = stage.join(filename);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Could not replace {}", path.display()))?;
        }
        fs::rename(&temporary_path, &path)
            .with_context(|| format!("Could not finalize {}", path.display()))?;
        Some(path)
    };
    let failed_segments = segments.len().saturating_sub(decoded_count);
    if !complete && first_error.is_none() {
        first_error = Some(format!(
            "Only {decoded_count} of {} Usenet segments were downloaded.",
            segments.len()
        ));
    }
    if complete {
        first_error = None;
    }
    Ok(NzbFileDownload {
        path,
        complete,
        successful_segments: decoded_count,
        failed_segments,
        downloaded_source_bytes: downloaded,
        first_error,
    })
}

struct ConnectionSlot {
    slots: Arc<(Mutex<u8>, Condvar)>,
}

impl ConnectionSlot {
    fn acquire(slots: Arc<(Mutex<u8>, Condvar)>, maximum: u8) -> Self {
        let (lock, condition) = &*slots;
        let mut active = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while *active >= maximum.max(1) {
            active = condition
                .wait(active)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *active += 1;
        drop(active);
        Self { slots }
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let (lock, condition) = &*self.slots;
        let mut active = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *active = active.saturating_sub(1);
        condition.notify_one();
    }
}

fn decode_yenc(number: u32, lines: &[Vec<u8>]) -> Result<DecodedSegment> {
    let begin_index = lines
        .iter()
        .position(|line| line.starts_with(b"=ybegin "))
        .ok_or_else(|| anyhow!("Usenet segment {number} is not yEnc encoded."))?;
    let begin_line = String::from_utf8_lossy(&lines[begin_index]);
    let filename = begin_line
        .split_once(" name=")
        .map(|(_, name)| name.trim().to_owned())
        .unwrap_or_default();
    let total_size =
        yenc_attribute(&lines[begin_index], "size").and_then(|value| value.parse::<u64>().ok());
    let part_line = lines
        .get(begin_index + 1)
        .filter(|line| line.starts_with(b"=ypart "));
    let begin = part_line
        .and_then(|line| yenc_attribute(line, "begin"))
        .and_then(|value| value.parse().ok());
    let data_start = begin_index + 1 + usize::from(part_line.is_some());
    let end_index = lines[data_start..]
        .iter()
        .position(|line| line.starts_with(b"=yend "))
        .map(|index| index + data_start)
        .ok_or_else(|| anyhow!("Usenet segment {number} is missing its yEnc footer."))?;
    let mut output = Vec::new();
    let mut escaped = false;
    for line in &lines[data_start..end_index] {
        for &encoded in line {
            if escaped {
                output.push(encoded.wrapping_sub(64).wrapping_sub(42));
                escaped = false;
            } else if encoded == b'=' {
                escaped = true;
            } else {
                output.push(encoded.wrapping_sub(42));
            }
        }
    }
    if escaped {
        bail!("Usenet segment {number} ends in an incomplete yEnc escape.");
    }
    let footer = &lines[end_index];
    let expected_crc = if part_line.is_some() {
        yenc_attribute(footer, "pcrc32")
    } else {
        yenc_attribute(footer, "crc32")
    };
    if let Some(expected) = expected_crc {
        let mut hasher = Hasher::new();
        hasher.update(&output);
        let actual = format!("{:08x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            bail!("Usenet segment {number} failed its yEnc checksum.");
        }
    }
    Ok(DecodedSegment {
        number,
        filename,
        begin,
        total_size,
        bytes: output,
    })
}

fn yenc_attribute<'a>(line: &'a [u8], name: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(line).ok()?;
    text.split_ascii_whitespace()
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
}

fn subject_filename(subject: &str) -> String {
    let mut quote_indexes = subject.match_indices('"').map(|(index, _)| index);
    if let (Some(start), Some(end)) = (quote_indexes.next(), quote_indexes.next()) {
        let filename = subject[start + 1..end].trim();
        if !filename.is_empty() {
            return sanitize_filename(filename);
        }
    }
    sanitize_filename(subject)
}

fn par2_recovery_blocks(filename: &str) -> Option<u32> {
    let lowercase = filename.to_lowercase();
    let volume = lowercase.rsplit_once(".vol")?.1;
    let (range, extension) = volume.rsplit_once('.')?;
    if extension != "par2" {
        return None;
    }
    range.rsplit_once('+')?.1.parse().ok()
}

fn release_match_score(release: &str, artist: &str, album: &str, year: Option<i32>) -> u8 {
    let release = normalized_words(release);
    let artist_words = normalized_words(artist);
    let album_words = normalized_words(album);
    let wanted = artist_words
        .iter()
        .chain(album_words.iter())
        .collect::<Vec<_>>();
    if wanted.is_empty() {
        return 0;
    }
    let matches = wanted.iter().filter(|word| release.contains(word)).count();
    let mut score = ((matches * 90) / wanted.len()) as i32;
    if artist_words.iter().all(|word| release.contains(word)) {
        score += 5;
    }
    if album_words.iter().all(|word| release.contains(word)) {
        score += 5;
    }
    if year.is_some_and(|year| release.contains(&year.to_string())) {
        score += 3;
    }
    score.clamp(0, 100) as u8
}

fn normalized_words(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() > 1)
        .map(|word| word.to_lowercase())
        .collect()
}

fn progress_percent(downloaded: u64, total: u64) -> u8 {
    (((downloaded.min(total) as f64 / total.max(1) as f64) * 98.0).round() as u8).min(98)
}

fn compact_subject(subject: &str) -> String {
    let value = subject.trim();
    if value.chars().count() <= 70 {
        value.to_owned()
    } else {
        format!("{}…", value.chars().take(69).collect::<String>())
    }
}

fn sanitize_filename(value: &str) -> String {
    let source = value.trim().trim_matches('"');
    let mut output = source
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) || character.is_control()
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    output = output.trim().trim_end_matches('.').to_owned();
    if output.is_empty() {
        "usenet-file.bin".to_owned()
    } else {
        output
    }
}

fn release_stage_path(download_directory: &str, guid: &str) -> PathBuf {
    let digest = Sha256::digest(guid.trim().as_bytes());
    let key = hex::encode(&digest[..12]);
    PathBuf::from(download_directory)
        .join(".flipside-usenet")
        .join(key)
}

fn album_folder_name(artist: &str, album: &str, year: Option<i32>, fallback: &str) -> String {
    let base = if artist.trim().is_empty() || album.trim().is_empty() {
        fallback.to_owned()
    } else {
        format!("{} - {}", artist.trim(), album.trim())
    };
    let with_year = year.map(|year| format!("{base} ({year})")).unwrap_or(base);
    sanitize_filename(&with_year)
}

fn unique_destination(parent: &Path, name: &str) -> PathBuf {
    let first = parent.join(name);
    if !first.exists() {
        return first;
    }
    for number in 2..10_000 {
        let candidate = parent.join(format!("{name} ({number})"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{name} ({})", Utc::now().timestamp()))
}

fn first_rar_archive(directory: &Path) -> Result<Option<PathBuf>> {
    let mut candidates = fs::read_dir(directory)
        .context("Could not inspect downloaded Usenet files")?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("rar"))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if name.contains("part01.rar") || name.contains("part001.rar") {
            0
        } else if name.contains("part1.rar") {
            1
        } else {
            2
        }
    });
    Ok(candidates.into_iter().next())
}

fn first_par2_file(directory: &Path) -> Result<Option<PathBuf>> {
    let mut candidates = fs::read_dir(directory)
        .context("Could not inspect Usenet recovery staging")?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("par2"))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        path.file_name()
            .and_then(|value| value.to_str())
            .and_then(par2_recovery_blocks)
            .map(|blocks| (1, blocks))
            .unwrap_or((0, 0))
    });
    Ok(candidates.into_iter().next())
}

#[derive(Debug)]
struct Par2Outcome {
    success: bool,
    detail: String,
}

fn run_par2(
    executable: &Path,
    operation: &str,
    reference: &Path,
    directory: &Path,
) -> Result<Par2Outcome> {
    let mut command = Command::new(executable);
    command
        .arg(operation)
        .arg("-q")
        .arg(format!("-B{}", directory.display()));
    if operation == "r" {
        command.arg("-p");
    }
    let output = command
        .arg(reference)
        .current_dir(directory)
        .output()
        .with_context(|| format!("Could not start {}", executable.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .unwrap_or(if output.status.success() {
            "PAR2 verification completed"
        } else {
            "PAR2 verification or repair failed"
        })
        .chars()
        .take(500)
        .collect();
    Ok(Par2Outcome {
        success: output.status.success(),
        detail,
    })
}

fn remove_par2_files(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory).context("Could not inspect repaired Usenet files")? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("par2"))
        {
            fs::remove_file(&path)
                .with_context(|| format!("Could not remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn find_par2() -> Option<PathBuf> {
    let explicit = [
        PathBuf::from(r"C:\Tools\par2cmdline-turbo\par2.exe"),
        PathBuf::from(r"C:\Program Files\par2cmdline-turbo\par2.exe"),
    ];
    if let Some(path) = explicit.into_iter().find(|path| path.is_file()) {
        return Some(path);
    }
    let output = Command::new("where.exe").arg("par2.exe").output().ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .map(PathBuf::from)
    })?
}

fn find_unrar() -> Option<PathBuf> {
    let explicit = [
        PathBuf::from(r"C:\Tools\UnRAR\UnRAR.exe"),
        PathBuf::from(r"C:\Program Files\WinRAR\UnRAR.exe"),
    ];
    if let Some(path) = explicit.into_iter().find(|path| path.is_file()) {
        return Some(path);
    }
    let output = Command::new("where.exe").arg("unrar.exe").output().ok()?;
    output
        .status
        .success()
        .then(|| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .map(PathBuf::from)
        })
        .flatten()
}

fn select_payload_root(extracted: &Path) -> Result<PathBuf> {
    let entries = fs::read_dir(extracted)
        .context("Could not inspect extracted Usenet files")?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect::<Vec<_>>();
    if entries.len() == 1 && entries[0].is_dir() {
        Ok(entries[0].clone())
    } else {
        Ok(extracted.to_owned())
    }
}

fn move_payload(source: &Path, destination: &Path) -> Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_directory(source, destination)?;
            fs::remove_dir_all(source).context("Could not remove the completed staging folder")
        }
    }
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn credential_entry(service: &str, account: &str) -> Result<Entry> {
    Entry::new(service, account).context("Windows Credential Manager could not be opened")
}

fn credential_get(service: &str, account: &str) -> Result<Option<Zeroizing<String>>> {
    match credential_entry(service, account)?.get_password() {
        Ok(value) => Ok(Some(Zeroizing::new(value))),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(anyhow!(
            "Windows Credential Manager could not be read: {error}"
        )),
    }
}

fn credential_has(service: &str, account: &str) -> Result<bool> {
    Ok(credential_get(service, account)?.is_some())
}

fn credential_store(service: &str, account: &str, value: &str) -> Result<()> {
    credential_entry(service, account)?
        .set_password(value)
        .context("Windows Credential Manager could not store the credential")
}

/// Stores the Prowlarr API key when none exists yet (used when seeding from `.env`).
pub(crate) fn seed_prowlarr_key_if_missing(value: &str) -> Result<bool> {
    if value.trim().is_empty()
        || credential_has(PROWLARR_CREDENTIAL_SERVICE, PROWLARR_CREDENTIAL_ACCOUNT)?
    {
        return Ok(false);
    }
    credential_store(PROWLARR_CREDENTIAL_SERVICE, PROWLARR_CREDENTIAL_ACCOUNT, value.trim())?;
    Ok(true)
}

fn credential_delete(service: &str, account: &str) -> Result<()> {
    match credential_entry(service, account)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(anyhow!(
            "Windows Credential Manager could not remove the credential: {error}"
        )),
    }
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("Could not read {}", path.display()))?;
    serde_json::from_str(&contents)
        .map(Some)
        .with_context(|| format!("{} is not valid JSON", path.display()))
}

fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_yenc(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        for &byte in input {
            let encoded = byte.wrapping_add(42);
            if matches!(encoded, 0 | b'\n' | b'\r' | b'=') {
                output.push(b'=');
                output.push(encoded.wrapping_add(64));
            } else {
                output.push(encoded);
            }
        }
        output
    }

    #[test]
    fn decodes_and_checks_yenc_segments() {
        let input = b"FLAC\r\n=music";
        let mut hasher = Hasher::new();
        hasher.update(input);
        let lines = vec![
            b"=ybegin part=1 line=128 size=11 name=01 Track.flac".to_vec(),
            b"=ypart begin=1 end=11".to_vec(),
            encode_yenc(input),
            format!("=yend size=11 part=1 pcrc32={:08x}", hasher.finalize()).into_bytes(),
        ];
        let decoded = decode_yenc(1, &lines).expect("valid yEnc");
        assert_eq!(decoded.filename, "01 Track.flac");
        assert_eq!(decoded.begin, Some(1));
        assert_eq!(decoded.total_size, Some(11));
        assert_eq!(decoded.bytes, input);
    }

    #[test]
    fn parses_nzb_files_and_segments() {
        let xml = r#"<?xml version="1.0"?><nzb xmlns="http://www.newzbin.com/DTD/2003/nzb"><file subject="album.rar"><groups><group>alt.binaries.music</group></groups><segments><segment bytes="123" number="1">abc@example</segment></segments></file></nzb>"#;
        let nzb: Nzb = from_str(xml).expect("valid NZB");
        assert_eq!(nzb.files.len(), 1);
        assert_eq!(nzb.files[0].segments.segments[0].message_id, "abc@example");
    }

    #[test]
    fn classifies_payload_optional_and_par2_entries() {
        fn file(subject: &str) -> NzbFile {
            NzbFile {
                subject: subject.to_owned(),
                segments: NzbSegments { segments: vec![] },
            }
        }

        assert_eq!(
            file(r#"[1/4] - "album.part01.rar" yEnc"#).kind(),
            NzbFileKind::Payload
        );
        assert_eq!(
            file(r#"[2/4] - "album.nfo" yEnc"#).kind(),
            NzbFileKind::Optional
        );
        assert_eq!(
            file(r#"[3/4] - "album.par2" yEnc"#).kind(),
            NzbFileKind::Par2Index
        );
        assert_eq!(
            file(r#"[4/4] - "album.vol0+252.par2" yEnc"#).kind(),
            NzbFileKind::Par2Volume
        );
        assert_eq!(par2_recovery_blocks("album.vol0+252.par2"), Some(252));
        assert_eq!(
            subject_filename(r#"[1/1] - "Artist - Album.flac" yEnc"#),
            "Artist - Album.flac"
        );
    }

    #[test]
    fn installed_par2_repairs_a_corrupt_file() {
        let Some(executable) = find_par2() else {
            return;
        };
        let directory = tempfile::tempdir().expect("temporary PAR2 test directory");
        let payload_path = directory.path().join("payload.bin");
        let par2_path = directory.path().join("recovery.par2");
        let original = (0..256 * 1_024)
            .map(|index| ((index * 31) % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&payload_path, &original).expect("write original payload");
        let creation = Command::new(&executable)
            .arg("c")
            .arg("-q")
            .arg("-s1024")
            .arg("-r20")
            .arg(&par2_path)
            .arg(&payload_path)
            .current_dir(directory.path())
            .output()
            .expect("start PAR2 creation");
        assert!(creation.status.success(), "PAR2 creation failed");

        let mut corrupt = original.clone();
        corrupt[32_768..40_960].fill(0);
        fs::write(&payload_path, corrupt).expect("write corrupt payload");
        assert!(
            !run_par2(&executable, "v", &par2_path, directory.path())
                .expect("verify corrupt payload")
                .success
        );
        assert!(
            run_par2(&executable, "r", &par2_path, directory.path())
                .expect("repair corrupt payload")
                .success
        );
        assert_eq!(
            fs::read(&payload_path).expect("read repaired payload"),
            original
        );
        let remaining = fs::read_dir(directory.path())
            .expect("inspect repaired directory")
            .map(|entry| entry.expect("directory entry").file_name())
            .collect::<Vec<_>>();
        assert_eq!(remaining, vec![std::ffi::OsString::from("payload.bin")]);
    }

    #[test]
    fn scores_artist_album_and_year_matches() {
        let exact = release_match_score(
            "Michael.Stanley.Band-Heartland-Remastered-WEB-2016-FLAC",
            "Michael Stanley Band",
            "Heartland",
            Some(2016),
        );
        let unrelated = release_match_score(
            "Another.Artist-Other.Album",
            "Michael Stanley Band",
            "Heartland",
            Some(2016),
        );
        assert_eq!(exact, 100);
        assert!(unrelated < exact);
    }

    #[test]
    fn validates_newsgroup_ninja_defaults() {
        let mut profile = UsenetProfile::suggested(Path::new(r"C:\Downloads\Usenet"));
        profile.username = "listener".to_owned();
        profile.validate().expect("valid profile");
        assert_eq!(profile.prowlarr_url, "http://127.0.0.1:9696");
        assert_eq!(profile.news_host, "news.newsgroup.ninja");
        assert_eq!(profile.news_port, 563);
        assert!(profile.use_tls);
    }
}
