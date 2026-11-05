use super::soundcheck::{inspect_file, SoundcheckResult, };
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, };
use thiserror::Error;
use tauri_specta::Event as _;

const STORE_VERSION: u32 = 1;
const MAX_AUTOMATIC_RETRIES: u32 = 3;
const DEFAULT_MAX_CONCURRENT_DOWNLOADS: u8 = 3;
const MAX_CONCURRENT_DOWNLOADS: u8 = 6;
const DEFAULT_RELAY_SUGGESTION_MINUTES: u32 = 10;
const RELAY_SUGGESTION_MINUTES: [u32; 6] = [0, 5, 10, 20, 30, 60];

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TransferSafetyState {
    #[default]
    Running,
    Draining,
    PausedForRestart,
}

impl TransferSafetyState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Draining,
            2 => Self::PausedForRestart,
            _ => Self::Running,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Draining => 1,
            Self::PausedForRestart => 2,
        }
    }
}

fn default_max_concurrent_downloads() -> u8 {
    DEFAULT_MAX_CONCURRENT_DOWNLOADS
}

fn default_relay_suggestion_minutes() -> u32 {
    DEFAULT_RELAY_SUGGESTION_MINUTES
}

fn default_soundcheck_enabled() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Queued,
    Retrying,
    Requesting,
    RemotelyQueued,
    Connecting,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum VerificationStatus {
    #[default]
    Pending,
    Verified,
    Missing,
    SizeMismatch,
}

#[derive(Clone, Debug, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAlternativeFile {
    pub title: String,
    pub remote_filename: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAlternativeSource {
    pub username: String,
    pub remote_folder: String,
    pub files: Vec<ReleaseAlternativeFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PatchRepairSnapshot {
    pub reason: String,
    pub original_username: Option<String>,
    pub original_remote_filename: Option<String>,
    pub requested_at_ms: u64,
    #[serde(default)]
    pub repaired_at_ms: Option<u64>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl TransferStatus {
    fn occupies_slot(self) -> bool {
        matches!(
            self,
            Self::Requesting | Self::RemotelyQueued | Self::Connecting | Self::Downloading
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferSnapshot {
    pub id: String,
    #[serde(default)]
    pub release_id: Option<String>,
    #[serde(default)]
    pub release_title: Option<String>,
    #[serde(default)]
    pub release_folder: Option<String>,
    #[serde(default)]
    pub file_index: Option<u32>,
    #[serde(default)]
    pub file_count: Option<u32>,
    #[serde(default)]
    pub expected_track_count: Option<u32>,
    #[serde(default)]
    pub release_group_id: Option<String>,
    pub title: String,
    pub username: String,
    pub remote_filename: String,
    pub size_bytes: u64,
    pub transferred_bytes: u64,
    pub speed_bytes_per_second: u64,
    pub eta_seconds: Option<u64>,
    pub status: TransferStatus,
    pub queue_position: Option<u32>,
    pub local_path: String,
    pub error: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub retry_at_ms: Option<u64>,
    #[serde(default)]
    pub waiting_since_ms: Option<u64>,
    #[serde(default)]
    pub verification_status: VerificationStatus,
    #[serde(default)]
    pub verification_message: Option<String>,
    #[serde(default)]
    pub verified_at_ms: Option<u64>,
    #[serde(default)]
    pub filed_at_ms: Option<u64>,
    #[serde(default)]
    pub soundcheck: Option<SoundcheckResult>,
    #[serde(default)]
    pub patch_repair: Option<PatchRepairSnapshot>,
    #[serde(default)]
    pub alternative_sources: Vec<ReleaseAlternativeSource>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(skip)]
    connection_token: Option<u32>,
    #[serde(skip)]
    transfer_token: Option<u32>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferQueueSnapshot {
    pub transfers: Vec<TransferSnapshot>,
    pub active_count: usize,
    pub max_concurrent_downloads: u8,
    pub relay_suggestion_minutes: u32,
    pub soundcheck_enabled: bool,
    pub safety_state: TransferSafetyState,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueReleaseFileRequest {
    pub title: String,
    pub remote_filename: String,
    pub size_bytes: u64,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueReleaseRequest {
    pub title: String,
    pub username: String,
    pub remote_folder: String,
    pub files: Vec<EnqueueReleaseFileRequest>,
    #[serde(default)]
    pub expected_track_count: Option<u32>,
    #[serde(default)]
    pub release_group_id: Option<String>,
    #[serde(default)]
    pub alternatives: Vec<ReleaseAlternativeSource>,
}

#[derive(Clone, Debug)]
pub struct TransferTicket {
    pub id: String,
    pub username: String,
    pub remote_filename: String,
    pub connection_token: u32,
}

#[derive(Clone, Debug)]
pub struct DownloadPlan {
    pub id: String,
    pub partial_path: PathBuf,
    pub final_path: PathBuf,
    pub size_bytes: u64,
    pub offset: u64,
}

#[derive(Serialize, Deserialize)]
struct TransferStore {
    version: u32,
    #[serde(default = "default_max_concurrent_downloads")]
    max_concurrent_downloads: u8,
    #[serde(default = "default_relay_suggestion_minutes")]
    relay_suggestion_minutes: u32,
    #[serde(default = "default_soundcheck_enabled")]
    soundcheck_enabled: bool,
    transfers: Vec<TransferSnapshot>,
}

#[derive(Clone)]
pub struct TransferHub {
    app: AppHandle,
    path: PathBuf,
    transfers: Arc<RwLock<Vec<TransferSnapshot>>>,
    max_concurrent_downloads: Arc<AtomicU8>,
    relay_suggestion_minutes: Arc<AtomicU32>,
    soundcheck_enabled: Arc<AtomicBool>,
    safety_state: Arc<AtomicU8>,
    tasks: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    next_id: Arc<AtomicU64>,
}

impl TransferHub {
    pub fn new(app: AppHandle, path: PathBuf) -> Result<Self, TransferError> {
        let store = load_store(&path)?;
        let mut transfers = store.transfers;
        for transfer in &mut transfers {
            reconcile_transfer_after_restart(transfer);
            if store.soundcheck_enabled
                && transfer.status == TransferStatus::Completed
                && transfer.verification_status == VerificationStatus::Verified
                && transfer.soundcheck.is_none()
            {
                refresh_soundcheck(transfer, false);
            }
        }

        let hub = Self {
            app,
            path,
            transfers: Arc::new(RwLock::new(transfers)),
            max_concurrent_downloads: Arc::new(AtomicU8::new(store.max_concurrent_downloads)),
            relay_suggestion_minutes: Arc::new(AtomicU32::new(store.relay_suggestion_minutes)),
            soundcheck_enabled: Arc::new(AtomicBool::new(store.soundcheck_enabled)),
            safety_state: Arc::new(AtomicU8::new(TransferSafetyState::Running.as_u8())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(timestamp_ms())),
        };
        hub.persist()?;
        Ok(hub)
    }

    pub fn snapshot(&self) -> TransferQueueSnapshot {
        let transfers = self
            .transfers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let active_count = transfers
            .iter()
            .filter(|transfer| transfer.status.occupies_slot())
            .count();
        TransferQueueSnapshot {
            transfers,
            active_count,
            max_concurrent_downloads: self.max_concurrent_downloads.load(Ordering::SeqCst),
            relay_suggestion_minutes: self.relay_suggestion_minutes.load(Ordering::SeqCst),
            soundcheck_enabled: self.soundcheck_enabled.load(Ordering::SeqCst),
            safety_state: self.safety_state(),
        }
    }

    pub fn safety_state(&self) -> TransferSafetyState {
        TransferSafetyState::from_u8(self.safety_state.load(Ordering::SeqCst))
    }

    pub fn set_max_concurrent_downloads(
        &self,
        max_concurrent_downloads: u8,
    ) -> Result<TransferQueueSnapshot, TransferError> {
        if !(1..=MAX_CONCURRENT_DOWNLOADS).contains(&max_concurrent_downloads) {
            return Err(TransferError::InvalidConcurrentDownloads);
        }
        self.max_concurrent_downloads
            .store(max_concurrent_downloads, Ordering::SeqCst);
        self.persist_and_publish()?;
        Ok(self.snapshot())
    }

    pub fn enqueue_release(
        &self,
        request: EnqueueReleaseRequest,
        download_directory: &Path,
    ) -> Result<TransferQueueSnapshot, TransferError> {
        validate_release_request(&request)?;
        let mut transfers = self
            .transfers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if unfinished_release_uses_source(&transfers, &request) {
            return Err(TransferError::DuplicateReleaseSource);
        }
        let existing = transfers.clone();
        fs::create_dir_all(download_directory)?;

        let now = timestamp_ms();
        let id_number = self.next_id.fetch_add(1, Ordering::SeqCst);
        let release_id = format!("release-{now}-{id_number}");
        let release_title = request.title.trim().to_owned();
        let release_directory =
            unique_release_directory(download_directory, &release_title, &existing);
        fs::create_dir_all(&release_directory)?;

        let file_count = u32::try_from(request.files.len()).unwrap_or(u32::MAX);
        let alternatives = request.alternatives.clone();
        let mut release_transfers = Vec::with_capacity(request.files.len());
        for (index, file) in request.files.into_iter().enumerate() {
            let local_path = unique_target_path(
                &release_directory,
                &file.remote_filename,
                &existing
                    .iter()
                    .chain(release_transfers.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            let file_id = self.next_id.fetch_add(1, Ordering::SeqCst);
            release_transfers.push(TransferSnapshot {
                id: format!("download-{now}-{file_id}"),
                release_id: Some(release_id.clone()),
                release_title: Some(release_title.clone()),
                release_folder: Some(release_directory.to_string_lossy().into_owned()),
                file_index: Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
                file_count: Some(file_count),
                expected_track_count: request.expected_track_count,
                release_group_id: request.release_group_id.clone(),
                title: file.title.trim().to_owned(),
                username: request.username.clone(),
                remote_filename: normalize_remote_filename(&file.remote_filename),
                size_bytes: file.size_bytes,
                transferred_bytes: 0,
                speed_bytes_per_second: 0,
                eta_seconds: None,
                status: TransferStatus::Queued,
                queue_position: None,
                local_path: local_path.to_string_lossy().into_owned(),
                error: None,
                retry_count: 0,
                retry_at_ms: None,
                waiting_since_ms: None,
                verification_status: VerificationStatus::Pending,
                verification_message: None,
                verified_at_ms: None,
                filed_at_ms: None,
                soundcheck: None,
                patch_repair: None,
                alternative_sources: alternatives.clone(),
                created_at_ms: now.saturating_add(index as u64),
                updated_at_ms: now,
                connection_token: None,
                transfer_token: None,
            });
        }
        transfers.extend(release_transfers);
        drop(transfers);
        self.persist_and_publish()?;
        Ok(self.snapshot())
    }

    pub fn pause_release(&self, release_id: &str) -> Result<TransferQueueSnapshot, TransferError> {
        self.mutate_release(release_id, |transfer| {
            if transfer.status != TransferStatus::Completed {
                transfer.status = TransferStatus::Paused;
                reset_runtime_state(transfer);
            }
        })
    }

    pub fn resume_release(&self, release_id: &str) -> Result<TransferQueueSnapshot, TransferError> {
        self.mutate_release(release_id, |transfer| {
            if matches!(
                transfer.status,
                TransferStatus::Paused | TransferStatus::Failed | TransferStatus::Retrying
            ) {
                refresh_partial_size(transfer);
                transfer.status = TransferStatus::Queued;
                transfer.error = None;
                transfer.retry_count = 0;
                transfer.retry_at_ms = None;
                transfer.verification_status = VerificationStatus::Pending;
                transfer.verification_message = None;
                transfer.verified_at_ms = None;
                transfer.soundcheck = None;
                reset_runtime_state(transfer);
            }
        })
    }

    pub fn cancel_release(&self, release_id: &str) -> Result<TransferQueueSnapshot, TransferError> {
        let ids: Vec<String> = self
            .transfers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|transfer| transfer.release_id.as_deref() == Some(release_id))
            .map(|transfer| transfer.id.clone())
            .collect();
        if ids.is_empty() {
            return Err(TransferError::ReleaseNotFound);
        }
        for id in &ids {
            self.abort(id);
        }
        let removed = {
            let mut transfers = self
                .transfers
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut removed = Vec::new();
            transfers.retain(|transfer| {
                if transfer.release_id.as_deref() == Some(release_id) {
                    removed.push(transfer.clone());
                    false
                } else {
                    true
                }
            });
            removed
        };
        for transfer in removed {
            if transfer.status != TransferStatus::Completed {
                schedule_partial_cleanup(partial_path(Path::new(&transfer.local_path)));
            }
        }
        self.persist_and_publish()?;
        Ok(self.snapshot())
    }

    pub fn verify_release(&self, release_id: &str) -> Result<TransferQueueSnapshot, TransferError> {
        let mut found = false;
        self.mutate(|transfers| {
            for transfer in transfers
                .iter_mut()
                .filter(|transfer| transfer.release_id.as_deref() == Some(release_id))
            {
                found = true;
                if transfer.status == TransferStatus::Completed {
                    refresh_verification(transfer);
                }
            }
        })?;
        if !found {
            return Err(TransferError::ReleaseNotFound);
        }
        Ok(self.snapshot())
    }

    pub fn clear_completed(&self) -> Result<TransferQueueSnapshot, TransferError> {
        self.mutate(|transfers| {
            let completed_release_ids: Vec<String> = transfers
                .iter()
                .filter_map(|transfer| transfer.release_id.clone())
                .filter(|release_id| {
                    transfers
                        .iter()
                        .filter(|transfer| {
                            transfer.release_id.as_deref() == Some(release_id.as_str())
                        })
                        .all(|transfer| transfer.status == TransferStatus::Completed)
                })
                .collect();
            transfers.retain(|transfer| {
                if transfer.release_id.is_none() {
                    return transfer.status != TransferStatus::Completed;
                }
                !transfer.release_id.as_ref().is_some_and(|release_id| {
                    completed_release_ids
                        .iter()
                        .any(|completed| completed == release_id)
                })
            });
        })?;
        Ok(self.snapshot())
    }

    pub fn reveal_release_path(&self, release_id: &str) -> Result<String, TransferError> {
        self.verify_release(release_id)?;
        self.transfers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|transfer| transfer.release_id.as_deref() == Some(release_id))
            .and_then(|transfer| {
                transfer.release_folder.clone().or_else(|| {
                    Path::new(&transfer.local_path)
                        .parent()
                        .map(|path| path.to_string_lossy().into_owned())
                })
            })
            .ok_or(TransferError::ReleaseNotFound)
    }

    pub fn activate_next(&self, connection_token: u32) -> Option<TransferTicket> {
        if self.safety_state() != TransferSafetyState::Running {
            return None;
        }
        let ticket = {
            let mut transfers = self
                .transfers
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let now = timestamp_ms();
            let index = next_download_index(
                &transfers,
                self.max_concurrent_downloads.load(Ordering::SeqCst),
                now,
            )?;
            let transfer = &mut transfers[index];
            transfer.status = TransferStatus::Requesting;
            transfer.connection_token = Some(connection_token);
            transfer.transfer_token = None;
            transfer.queue_position = None;
            transfer.error = None;
            transfer.retry_at_ms = None;
            transfer.waiting_since_ms = Some(now);
            transfer.updated_at_ms = now;
            TransferTicket {
                id: transfer.id.clone(),
                username: transfer.username.clone(),
                remote_filename: transfer.remote_filename.clone(),
                connection_token,
            }
        };
        let _ = self.persist_and_publish();
        Some(ticket)
    }

    pub fn requesting_for_username(&self, username: &str) -> Option<TransferTicket> {
        self.transfers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|transfer| {
                transfer.status == TransferStatus::Requesting
                    && transfer.username.eq_ignore_ascii_case(username)
            })
            .and_then(|transfer| {
                Some(TransferTicket {
                    id: transfer.id.clone(),
                    username: transfer.username.clone(),
                    remote_filename: transfer.remote_filename.clone(),
                    connection_token: transfer.connection_token?,
                })
            })
    }

    pub fn claim_peer(&self, connection_token: u32) -> Option<TransferTicket> {
        let ticket = {
            let mut transfers = self
                .transfers
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let transfer = transfers.iter_mut().find(|transfer| {
                transfer.status == TransferStatus::Requesting
                    && transfer.connection_token == Some(connection_token)
            })?;
            transfer.status = TransferStatus::RemotelyQueued;
            transfer.updated_at_ms = timestamp_ms();
            TransferTicket {
                id: transfer.id.clone(),
                username: transfer.username.clone(),
                remote_filename: transfer.remote_filename.clone(),
                connection_token,
            }
        };
        let _ = self.persist_and_publish();
        Some(ticket)
    }

    pub fn accept_upload_request(
        &self,
        username: &str,
        remote_filename: &str,
        transfer_token: u32,
        size_bytes: u64,
    ) -> Option<String> {
        let id = {
            let mut transfers = self
                .transfers
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let normalized = normalize_remote_filename(remote_filename);
            let transfer = transfers.iter_mut().find(|transfer| {
                transfer.status == TransferStatus::RemotelyQueued
                    && transfer.username.eq_ignore_ascii_case(username)
                    && transfer.remote_filename.eq_ignore_ascii_case(&normalized)
                    && transfer.size_bytes == size_bytes
            })?;
            transfer.status = TransferStatus::Connecting;
            transfer.transfer_token = Some(transfer_token);
            transfer.queue_position = None;
            transfer.updated_at_ms = timestamp_ms();
            transfer.id.clone()
        };
        let _ = self.persist_and_publish();
        Some(id)
    }

    pub fn set_queue_position(&self, username: &str, filename: &str, position: u32) {
        let normalized = normalize_remote_filename(filename);
        let _ = self.mutate(|transfers| {
            if let Some(transfer) = transfers.iter_mut().find(|transfer| {
                transfer.status == TransferStatus::RemotelyQueued
                    && transfer.username.eq_ignore_ascii_case(username)
                    && transfer.remote_filename.eq_ignore_ascii_case(&normalized)
            }) {
                transfer.queue_position = Some(position);
                transfer.updated_at_ms = timestamp_ms();
            }
        });
    }

    pub fn fail_for_filename(&self, username: &str, filename: &str, message: String) -> bool {
        let normalized = normalize_remote_filename(filename);
        self.fail_matching(message, |transfer| {
            transfer.username.eq_ignore_ascii_case(username)
                && transfer.remote_filename.eq_ignore_ascii_case(&normalized)
                && transfer.status.occupies_slot()
        })
    }

    pub fn retry_for_filename(&self, username: &str, filename: &str, message: String) -> bool {
        let normalized = normalize_remote_filename(filename);
        self.retry_matching(message, |transfer| {
            transfer.username.eq_ignore_ascii_case(username)
                && transfer.remote_filename.eq_ignore_ascii_case(&normalized)
                && transfer.status.occupies_slot()
        })
    }

    pub fn retry_connection(&self, connection_token: u32, message: String) -> bool {
        self.retry_matching(message, |transfer| {
            transfer.connection_token == Some(connection_token)
                && transfer.status == TransferStatus::Requesting
        })
    }

    pub fn fail_id(&self, id: &str, message: String) -> bool {
        self.fail_matching(message, |transfer| transfer.id == id)
    }

    pub fn retry_id(&self, id: &str, message: String) -> bool {
        self.retry_matching(message, |transfer| transfer.id == id)
    }

    pub fn fail_transfer_token(&self, transfer_token: u32, message: String) -> bool {
        self.fail_matching(message, |transfer| {
            transfer.transfer_token == Some(transfer_token)
                && matches!(
                    transfer.status,
                    TransferStatus::Connecting | TransferStatus::Downloading
                )
        })
    }

    pub fn begin_file(&self, transfer_token: u32) -> Result<DownloadPlan, TransferError> {
        let plan = {
            let mut transfers = self
                .transfers
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let transfer = transfers
                .iter_mut()
                .find(|transfer| {
                    transfer.status == TransferStatus::Connecting
                        && transfer.transfer_token == Some(transfer_token)
                })
                .ok_or(TransferError::UnexpectedFileConnection)?;
            let final_path = PathBuf::from(&transfer.local_path);
            let partial_path = partial_path(&final_path);
            let offset = match fs::metadata(&partial_path) {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => return Err(error.into()),
            };
            if offset > transfer.size_bytes {
                return Err(TransferError::PartialTooLarge);
            }
            transfer.status = TransferStatus::Downloading;
            transfer.transferred_bytes = offset;
            transfer.speed_bytes_per_second = 0;
            transfer.eta_seconds = None;
            transfer.waiting_since_ms = None;
            transfer.updated_at_ms = timestamp_ms();
            DownloadPlan {
                id: transfer.id.clone(),
                partial_path,
                final_path,
                size_bytes: transfer.size_bytes,
                offset,
            }
        };
        self.persist_and_publish()?;
        Ok(plan)
    }

    pub fn update_progress(&self, id: &str, transferred: u64, speed: u64) {
        let _ = self.mutate(|transfers| {
            if let Some(transfer) = transfers.iter_mut().find(|transfer| transfer.id == id) {
                if transfer.status == TransferStatus::Downloading {
                    transfer.transferred_bytes = transferred.min(transfer.size_bytes);
                    transfer.speed_bytes_per_second = speed;
                    transfer.eta_seconds = (speed > 0).then(|| {
                        transfer
                            .size_bytes
                            .saturating_sub(transfer.transferred_bytes)
                            .div_ceil(speed)
                    });
                    transfer.updated_at_ms = timestamp_ms();
                }
            }
        });
    }

    pub fn complete(&self, id: &str) -> Result<(), TransferError> {
        let mut found = false;
        self.mutate(|transfers| {
            if let Some(transfer) = transfers.iter_mut().find(|transfer| transfer.id == id) {
                found = true;
                transfer.status = TransferStatus::Completed;
                transfer.transferred_bytes = transfer.size_bytes;
                transfer.speed_bytes_per_second = 0;
                transfer.eta_seconds = Some(0);
                transfer.queue_position = None;
                transfer.error = None;
                transfer.retry_count = 0;
                transfer.retry_at_ms = None;
                transfer.waiting_since_ms = None;
                transfer.connection_token = None;
                transfer.transfer_token = None;
                transfer.updated_at_ms = timestamp_ms();
                refresh_verification(transfer);
                let patch_repair = transfer.patch_repair.is_some();
                if patch_repair || self.soundcheck_enabled.load(Ordering::SeqCst) {
                    refresh_soundcheck(transfer, patch_repair);
                }
                if let Some(repair) = transfer.patch_repair.as_mut() {
                    repair.repaired_at_ms = Some(timestamp_ms());
                }
            }
        })?;
        if !found {
            return Err(TransferError::NotFound);
        }
        Ok(())
    }

    pub fn connection_lost(&self) {
        let ids: Vec<String> = self
            .transfers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|transfer| transfer.status.occupies_slot())
            .map(|transfer| transfer.id.clone())
            .collect();
        for id in ids {
            self.abort(&id);
        }
        let _ = self.mutate(|transfers| {
            for transfer in transfers {
                if transfer.status.occupies_slot() {
                    refresh_partial_size(transfer);
                    transfer.status = TransferStatus::Queued;
                    transfer.speed_bytes_per_second = 0;
                    transfer.eta_seconds = None;
                    transfer.queue_position = None;
                    transfer.error = None;
                    transfer.retry_at_ms = None;
                    transfer.waiting_since_ms = None;
                    transfer.connection_token = None;
                    transfer.transfer_token = None;
                    transfer.updated_at_ms = timestamp_ms();
                }
            }
        });
    }

    pub fn register_task(&self, id: String, cancellation: Arc<AtomicBool>) {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, cancellation);
    }

    pub fn unregister_task(&self, id: &str) {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }

    fn abort(&self, id: &str) {
        if let Some(cancellation) = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
        {
            cancellation.store(true, Ordering::SeqCst);
        }
    }

    fn mutate_release(
        &self,
        release_id: &str,
        mut mutation: impl FnMut(&mut TransferSnapshot),
    ) -> Result<TransferQueueSnapshot, TransferError> {
        let ids: Vec<String> = self
            .transfers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|transfer| transfer.release_id.as_deref() == Some(release_id))
            .map(|transfer| transfer.id.clone())
            .collect();
        if ids.is_empty() {
            return Err(TransferError::ReleaseNotFound);
        }
        for id in ids {
            self.abort(&id);
        }
        self.mutate(|transfers| {
            for transfer in transfers
                .iter_mut()
                .filter(|transfer| transfer.release_id.as_deref() == Some(release_id))
            {
                mutation(transfer);
                transfer.updated_at_ms = timestamp_ms();
            }
        })?;
        Ok(self.snapshot())
    }

    fn fail_matching(&self, message: String, matches: impl Fn(&TransferSnapshot) -> bool) -> bool {
        let mut changed = false;
        let _ = self.mutate(|transfers| {
            if let Some(transfer) = transfers.iter_mut().find(|transfer| matches(transfer)) {
                refresh_partial_size(transfer);
                transfer.status = TransferStatus::Failed;
                transfer.speed_bytes_per_second = 0;
                transfer.eta_seconds = None;
                transfer.queue_position = None;
                transfer.error = Some(message);
                transfer.retry_at_ms = None;
                transfer.waiting_since_ms = None;
                transfer.connection_token = None;
                transfer.transfer_token = None;
                transfer.updated_at_ms = timestamp_ms();
                changed = true;
            }
        });
        changed
    }

    fn retry_matching(&self, message: String, matches: impl Fn(&TransferSnapshot) -> bool) -> bool {
        let mut changed = false;
        let _ = self.mutate(|transfers| {
            if let Some(transfer) = transfers.iter_mut().find(|transfer| matches(transfer)) {
                refresh_partial_size(transfer);
                transfer.retry_count = transfer.retry_count.saturating_add(1);
                transfer.speed_bytes_per_second = 0;
                transfer.eta_seconds = None;
                transfer.queue_position = None;
                transfer.connection_token = None;
                transfer.transfer_token = None;
                transfer.waiting_since_ms = None;
                transfer.updated_at_ms = timestamp_ms();
                if transfer.retry_count <= MAX_AUTOMATIC_RETRIES {
                    transfer.status = TransferStatus::Retrying;
                    transfer.retry_at_ms = Some(
                        transfer
                            .updated_at_ms
                            .saturating_add(automatic_retry_delay_ms(transfer.retry_count)),
                    );
                    transfer.error = Some(message);
                } else {
                    transfer.status = TransferStatus::Failed;
                    transfer.retry_at_ms = None;
                    transfer.error = Some(format!(
                        "Automatic recovery stopped after {MAX_AUTOMATIC_RETRIES} tries. {message}"
                    ));
                }
                changed = true;
            }
        });
        changed
    }

    fn mutate(
        &self,
        mutation: impl FnOnce(&mut Vec<TransferSnapshot>),
    ) -> Result<bool, TransferError> {
        {
            let mut transfers = self
                .transfers
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let before = transfers.len();
            mutation(&mut transfers);
            if transfers.len() == before && transfers.is_empty() {
                return Ok(false);
            }
        }
        self.persist_and_publish()?;
        Ok(true)
    }

    fn persist_and_publish(&self) -> Result<(), TransferError> {
        self.persist()?;
        let _ = crate::events::SoulseekTransfersChanged(self.snapshot()).emit(&self.app);
        Ok(())
    }

    fn persist(&self) -> Result<(), TransferError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let store = TransferStore {
            version: STORE_VERSION,
            max_concurrent_downloads: self.max_concurrent_downloads.load(Ordering::SeqCst),
            relay_suggestion_minutes: self.relay_suggestion_minutes.load(Ordering::SeqCst),
            soundcheck_enabled: self.soundcheck_enabled.load(Ordering::SeqCst),
            transfers: self
                .transfers
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        };
        fs::write(&self.path, serde_json::to_vec_pretty(&store)?)?;
        Ok(())
    }
}

fn schedule_partial_cleanup(path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        for _ in 0..20 {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(_) => return,
            }
        }
    });
}

fn reconcile_transfer_after_restart(transfer: &mut TransferSnapshot) {
    let final_path = Path::new(&transfer.local_path);
    let final_file_complete = transfer.status != TransferStatus::Completed
        && fs::metadata(final_path)
            .map(|metadata| metadata.is_file() && metadata.len() == transfer.size_bytes)
            .unwrap_or(false);
    if final_file_complete {
        transfer.status = TransferStatus::Completed;
        transfer.transferred_bytes = transfer.size_bytes;
        transfer.speed_bytes_per_second = 0;
        transfer.eta_seconds = Some(0);
        transfer.error = None;
        transfer.retry_at_ms = None;
        transfer.waiting_since_ms = None;
        transfer.connection_token = None;
        transfer.transfer_token = None;
        refresh_verification(transfer);
        return;
    }

    if transfer.status.occupies_slot() {
        transfer.status = TransferStatus::Queued;
        transfer.speed_bytes_per_second = 0;
        transfer.eta_seconds = None;
        transfer.error = None;
        transfer.connection_token = None;
        transfer.transfer_token = None;
    }
    refresh_partial_size(transfer);
    if transfer.status == TransferStatus::Completed {
        refresh_verification(transfer);
    }
}

fn validate_release_request(request: &EnqueueReleaseRequest) -> Result<(), TransferError> {
    if request.title.trim().is_empty()
        || request.username.trim().is_empty()
        || request.remote_folder.trim().is_empty()
        || request.files.is_empty()
        || request.files.len() > 5_000
        || request
            .expected_track_count
            .is_some_and(|count| count == 0 || count > 5_000)
        || request.release_group_id.as_deref().is_some_and(|id| {
            id.len() != 36
                || id
                    .chars()
                    .enumerate()
                    .any(|(index, character)| match index {
                        8 | 13 | 18 | 23 => character != '-',
                        _ => !character.is_ascii_hexdigit(),
                    })
        })
        || request.files.iter().any(|file| {
            file.title.trim().is_empty()
                || file.remote_filename.trim().is_empty()
                || file.size_bytes == 0
        })
        || request.alternatives.len() > 12
        || request.alternatives.iter().any(|source| {
            source.username.trim().is_empty()
                || source.remote_folder.trim().is_empty()
                || source.files.is_empty()
                || source.files.len() > 5_000
                || source.files.iter().any(|file| {
                    file.title.trim().is_empty()
                        || file.remote_filename.trim().is_empty()
                        || file.size_bytes == 0
                })
        })
    {
        return Err(TransferError::InvalidReleaseRequest);
    }
    Ok(())
}

fn unfinished_release_uses_source(
    transfers: &[TransferSnapshot],
    request: &EnqueueReleaseRequest,
) -> bool {
    let requested_folder = normalize_remote_folder(&request.remote_folder);
    let unfinished_releases: HashSet<&str> = transfers
        .iter()
        .filter(|transfer| transfer.status != TransferStatus::Completed)
        .filter_map(|transfer| transfer.release_id.as_deref())
        .collect();

    transfers.iter().any(|transfer| {
        transfer
            .release_id
            .as_deref()
            .is_some_and(|release_id| unfinished_releases.contains(release_id))
            && transfer.username.eq_ignore_ascii_case(&request.username)
            && remote_parent(&transfer.remote_filename).eq_ignore_ascii_case(&requested_folder)
    })
}

fn reset_runtime_state(transfer: &mut TransferSnapshot) {
    transfer.speed_bytes_per_second = 0;
    transfer.eta_seconds = None;
    transfer.queue_position = None;
    transfer.connection_token = None;
    transfer.transfer_token = None;
    transfer.waiting_since_ms = None;
}

fn next_download_index(
    transfers: &[TransferSnapshot],
    max_concurrent_downloads: u8,
    now: u64,
) -> Option<usize> {
    let active_count = transfers
        .iter()
        .filter(|transfer| transfer.status.occupies_slot())
        .count();
    if active_count >= usize::from(max_concurrent_downloads) {
        return None;
    }
    let active_usernames = transfers
        .iter()
        .filter(|transfer| transfer.status.occupies_slot())
        .map(|transfer| transfer.username.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    transfers.iter().position(|transfer| {
        !active_usernames.contains(&transfer.username.to_ascii_lowercase())
            && (transfer.status == TransferStatus::Queued
                || (transfer.status == TransferStatus::Retrying
                    && transfer.retry_at_ms.is_none_or(|retry_at| retry_at <= now)))
    })
}

fn load_store(path: &Path) -> Result<TransferStore, TransferError> {
    if !path.exists() {
        return Ok(TransferStore {
            version: STORE_VERSION,
            max_concurrent_downloads: DEFAULT_MAX_CONCURRENT_DOWNLOADS,
            relay_suggestion_minutes: DEFAULT_RELAY_SUGGESTION_MINUTES,
            soundcheck_enabled: true,
            transfers: Vec::new(),
        });
    }
    let store = serde_json::from_slice::<TransferStore>(&fs::read(path)?)?;
    if store.version != STORE_VERSION {
        return Err(TransferError::UnsupportedStoreVersion(store.version));
    }
    if !(1..=MAX_CONCURRENT_DOWNLOADS).contains(&store.max_concurrent_downloads) {
        return Err(TransferError::InvalidConcurrentDownloads);
    }
    if !RELAY_SUGGESTION_MINUTES.contains(&store.relay_suggestion_minutes) {
        return Err(TransferError::InvalidRelaySuggestionMinutes);
    }
    Ok(store)
}

fn refresh_partial_size(transfer: &mut TransferSnapshot) {
    if transfer.status == TransferStatus::Completed {
        transfer.transferred_bytes = transfer.size_bytes;
        return;
    }
    transfer.transferred_bytes = fs::metadata(partial_path(Path::new(&transfer.local_path)))
        .map(|metadata| metadata.len().min(transfer.size_bytes))
        .unwrap_or(0);
}

fn refresh_verification(transfer: &mut TransferSnapshot) {
    if transfer.status != TransferStatus::Completed {
        transfer.verification_status = VerificationStatus::Pending;
        transfer.verification_message = None;
        transfer.verified_at_ms = None;
        transfer.soundcheck = None;
        return;
    }
    let path = Path::new(&transfer.local_path);
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() == transfer.size_bytes => {
            transfer.verification_status = VerificationStatus::Verified;
            transfer.verification_message = None;
            transfer.verified_at_ms = Some(timestamp_ms());
        }
        Ok(metadata) => {
            transfer.verification_status = VerificationStatus::SizeMismatch;
            transfer.verification_message = Some(format!(
                "Expected {} bytes, but the completed file contains {} bytes.",
                transfer.size_bytes,
                metadata.len()
            ));
            transfer.verified_at_ms = Some(timestamp_ms());
            transfer.soundcheck = None;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            transfer.verification_status = VerificationStatus::Missing;
            transfer.verification_message =
                Some("The completed file is no longer present in the download folder.".to_owned());
            transfer.verified_at_ms = Some(timestamp_ms());
            transfer.soundcheck = None;
        }
        Err(error) => {
            transfer.verification_status = VerificationStatus::SizeMismatch;
            transfer.verification_message = Some(format!(
                "Flipside could not inspect the completed file: {error}"
            ));
            transfer.verified_at_ms = Some(timestamp_ms());
            transfer.soundcheck = None;
        }
    }
}

fn refresh_soundcheck(transfer: &mut TransferSnapshot, deep: bool) {
    transfer.soundcheck = if transfer.status == TransferStatus::Completed
        && transfer.verification_status == VerificationStatus::Verified
    {
        inspect_file(Path::new(&transfer.local_path), deep, timestamp_ms())
    } else {
        None
    };
}

fn automatic_retry_delay_ms(attempt: u32) -> u64 {
    match attempt {
        0 | 1 => 5_000,
        2 => 15_000,
        _ => 45_000,
    }
}

fn partial_path(final_path: &Path) -> PathBuf {
    let mut value = final_path.as_os_str().to_os_string();
    value.push(".part");
    PathBuf::from(value)
}

fn unique_target_path(
    directory: &Path,
    remote_filename: &str,
    transfers: &[TransferSnapshot],
) -> PathBuf {
    let safe_name = safe_basename(remote_filename);
    let candidate = directory.join(&safe_name);
    if path_available(&candidate, transfers) {
        return candidate;
    }

    let path = Path::new(&safe_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2..10_000 {
        let name = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = directory.join(name);
        if path_available(&candidate, transfers) {
            return candidate;
        }
    }
    directory.join(format!("download-{}.bin", timestamp_ms()))
}

fn unique_release_directory(
    directory: &Path,
    title: &str,
    transfers: &[TransferSnapshot],
) -> PathBuf {
    let safe_title = safe_path_segment(title);
    for index in 1..10_000 {
        let name = if index == 1 {
            safe_title.clone()
        } else {
            format!("{safe_title} ({index})")
        };
        let candidate = directory.join(name);
        let used = candidate.exists()
            || transfers.iter().any(|transfer| {
                transfer.release_folder.as_ref().is_some_and(|folder| {
                    Path::new(folder)
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&candidate.to_string_lossy())
                })
            });
        if !used {
            return candidate;
        }
    }
    directory.join(format!("release-{}", timestamp_ms()))
}

fn path_available(candidate: &Path, transfers: &[TransferSnapshot]) -> bool {
    !candidate.exists()
        && !partial_path(candidate).exists()
        && !transfers.iter().any(|transfer| {
            Path::new(&transfer.local_path)
                .to_string_lossy()
                .eq_ignore_ascii_case(&candidate.to_string_lossy())
        })
}

fn normalize_remote_filename(value: &str) -> String {
    value.replace('/', "\\")
}

fn normalize_remote_folder(value: &str) -> String {
    normalize_remote_filename(value)
        .trim_matches('\\')
        .trim_end_matches('\\')
        .to_owned()
}

fn remote_parent(remote_filename: &str) -> String {
    let normalized = normalize_remote_filename(remote_filename);
    normalized
        .rsplit_once('\\')
        .map(|(folder, _)| normalize_remote_folder(folder))
        .unwrap_or_default()
}

fn safe_basename(remote_filename: &str) -> String {
    let normalized = normalize_remote_filename(remote_filename);
    let raw = normalized
        .split('\\')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or("download");
    let mut name: String = raw
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    name = name.trim().trim_end_matches(['.', ' ']).to_owned();
    if name.is_empty() || name == "." || name == ".." {
        name = "download".to_owned();
    }
    let stem = Path::new(&name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(&name)
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        name.insert(0, '_');
    }
    if name.chars().count() > 180 {
        name = name.chars().take(180).collect();
        name = name.trim_end_matches(['.', ' ']).to_owned();
    }
    name
}

fn safe_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim().trim_end_matches(['.', ' ']);
    let sanitized = if sanitized.is_empty() {
        "Release"
    } else {
        sanitized
    };
    sanitized.chars().take(120).collect()
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("That transfer is no longer in the queue.")]
    NotFound,
    #[error("That release is no longer in the queue.")]
    ReleaseNotFound,
    #[error("That exact listener and folder are already in the release queue.")]
    DuplicateReleaseSource,
    #[error("Choose between 1 and 6 simultaneous download users.")]
    InvalidConcurrentDownloads,
    #[error("Choose Off, 5, 10, 20, 30, or 60 minutes for Signal Relay suggestions.")]
    InvalidRelaySuggestionMinutes,
    #[error("Choose at least one valid file before downloading the release.")]
    InvalidReleaseRequest,
    #[error("The incoming Soulseek file connection did not match the active download.")]
    UnexpectedFileConnection,
    #[error(
        "The partial file is larger than the expected Soulseek file. Remove it before retrying."
    )]
    PartialTooLarge,
    #[error("Transfer data uses an unsupported format version ({0}).")]
    UnsupportedStoreVersion(u32),
    #[error("Could not read or save transfer data: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transfer data is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_transfer(
        id: &str,
        release_id: &str,
        username: &str,
        remote_filename: &str,
        status: TransferStatus,
        file_index: u32,
    ) -> TransferSnapshot {
        TransferSnapshot {
            id: id.to_owned(),
            release_id: Some(release_id.to_owned()),
            release_title: Some(release_id.to_owned()),
            release_folder: Some(format!("C:\\Downloads\\{release_id}")),
            file_index: Some(file_index),
            file_count: Some(2),
            expected_track_count: Some(2),
            release_group_id: None,
            title: remote_filename.to_owned(),
            username: username.to_owned(),
            remote_filename: remote_filename.to_owned(),
            size_bytes: 100,
            transferred_bytes: 0,
            speed_bytes_per_second: 0,
            eta_seconds: None,
            status,
            queue_position: None,
            local_path: format!("C:\\Downloads\\{release_id}\\{id}.flac"),
            error: None,
            retry_count: 0,
            retry_at_ms: None,
            waiting_since_ms: None,
            verification_status: VerificationStatus::Pending,
            verification_message: None,
            verified_at_ms: None,
            filed_at_ms: None,
            soundcheck: None,
            patch_repair: None,
            alternative_sources: Vec::new(),
            created_at_ms: u64::from(file_index),
            updated_at_ms: u64::from(file_index),
            connection_token: None,
            transfer_token: None,
        }
    }

    #[test]
    fn remote_paths_are_reduced_to_safe_local_filenames() {
        assert_eq!(safe_basename("Music\\Artist\\Track.flac"), "Track.flac");
        assert_eq!(safe_basename("../../CON.mp3"), "_CON.mp3");
        assert_eq!(safe_basename("folder\\bad<name>?.wav"), "bad_name__.wav");
        assert_eq!(safe_basename("folder\\..."), "download");
    }

    #[test]
    fn collision_paths_never_overwrite_existing_files_or_queue_entries() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("Track.flac"), b"existing").unwrap();
        let transfers = vec![TransferSnapshot {
            id: "one".to_owned(),
            release_id: None,
            release_title: None,
            release_folder: None,
            file_index: None,
            file_count: None,
            expected_track_count: None,
            release_group_id: None,
            title: "Track".to_owned(),
            username: "peer".to_owned(),
            remote_filename: "Track.flac".to_owned(),
            size_bytes: 10,
            transferred_bytes: 0,
            speed_bytes_per_second: 0,
            eta_seconds: None,
            status: TransferStatus::Queued,
            queue_position: None,
            local_path: directory
                .path()
                .join("Track (2).flac")
                .to_string_lossy()
                .into_owned(),
            error: None,
            retry_count: 0,
            retry_at_ms: None,
            waiting_since_ms: None,
            verification_status: VerificationStatus::Pending,
            verification_message: None,
            verified_at_ms: None,
            filed_at_ms: None,
            soundcheck: None,
            patch_repair: None,
            alternative_sources: Vec::new(),
            created_at_ms: 0,
            updated_at_ms: 0,
            connection_token: None,
            transfer_token: None,
        }];

        assert_eq!(
            unique_target_path(directory.path(), "Track.flac", &transfers),
            directory.path().join("Track (3).flac")
        );
    }

    #[test]
    fn persisted_transfers_keep_resume_progress_without_runtime_tokens() {
        let directory = tempfile::tempdir().unwrap();
        let final_path = directory.path().join("Track.flac");
        fs::write(partial_path(&final_path), vec![7_u8; 37]).unwrap();
        let mut transfer = TransferSnapshot {
            id: "resume-me".to_owned(),
            release_id: None,
            release_title: None,
            release_folder: None,
            file_index: None,
            file_count: None,
            expected_track_count: None,
            release_group_id: None,
            title: "Track.flac".to_owned(),
            username: "peer".to_owned(),
            remote_filename: "Music\\Track.flac".to_owned(),
            size_bytes: 100,
            transferred_bytes: 0,
            speed_bytes_per_second: 0,
            eta_seconds: None,
            status: TransferStatus::Queued,
            queue_position: None,
            local_path: final_path.to_string_lossy().into_owned(),
            error: None,
            retry_count: 0,
            retry_at_ms: None,
            waiting_since_ms: None,
            verification_status: VerificationStatus::Pending,
            verification_message: None,
            verified_at_ms: None,
            filed_at_ms: None,
            soundcheck: None,
            patch_repair: None,
            alternative_sources: Vec::new(),
            created_at_ms: 1,
            updated_at_ms: 2,
            connection_token: Some(77),
            transfer_token: Some(88),
        };
        refresh_partial_size(&mut transfer);
        assert_eq!(transfer.transferred_bytes, 37);

        let store_path = directory.path().join("transfers.json");
        let serialized = serde_json::to_vec_pretty(&TransferStore {
            version: STORE_VERSION,
            max_concurrent_downloads: DEFAULT_MAX_CONCURRENT_DOWNLOADS,
            relay_suggestion_minutes: DEFAULT_RELAY_SUGGESTION_MINUTES,
            soundcheck_enabled: true,
            transfers: vec![transfer],
        })
        .unwrap();
        fs::write(&store_path, &serialized).unwrap();
        let raw = String::from_utf8(serialized).unwrap();
        assert!(!raw.contains("connectionToken"));
        assert!(!raw.contains("transferToken"));
        assert_eq!(
            load_store(&store_path).unwrap().transfers[0].transferred_bytes,
            37
        );
    }

    #[test]
    fn restart_reconciles_a_finalized_file_before_resuming_the_queue() {
        let directory = tempfile::tempdir().unwrap();
        let final_path = directory.path().join("finished.flac");
        fs::write(&final_path, vec![4_u8; 100]).unwrap();
        let mut transfer = release_transfer(
            "renamed",
            "release-renamed",
            "listener",
            "Music\\Album\\finished.flac",
            TransferStatus::Downloading,
            1,
        );
        transfer.local_path = final_path.to_string_lossy().into_owned();
        transfer.transferred_bytes = 80;

        reconcile_transfer_after_restart(&mut transfer);

        assert_eq!(transfer.status, TransferStatus::Completed);
        assert_eq!(transfer.transferred_bytes, 100);
        assert_eq!(transfer.verification_status, VerificationStatus::Verified);
    }

    #[test]
    fn release_requests_validate_every_file_and_use_safe_unique_folders() {
        let request = EnqueueReleaseRequest {
            title: "Night: Geometry".to_owned(),
            username: "source".to_owned(),
            remote_folder: "Music\\Night Geometry".to_owned(),
            files: vec![EnqueueReleaseFileRequest {
                title: "01 - Thresholds.flac".to_owned(),
                remote_filename: "Music\\Night Geometry\\01 - Thresholds.flac".to_owned(),
                size_bytes: 112_400_000,
            }],
            expected_track_count: Some(1),
            release_group_id: None,
            alternatives: Vec::new(),
        };
        assert!(validate_release_request(&request).is_ok());

        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("Night_ Geometry")).unwrap();
        assert_eq!(
            unique_release_directory(directory.path(), &request.title, &[]),
            directory.path().join("Night_ Geometry (2)")
        );
    }

    #[test]
    fn exact_unfinished_release_sources_are_not_queued_twice() {
        let request = EnqueueReleaseRequest {
            title: "Hysteria".to_owned(),
            username: "listener".to_owned(),
            remote_folder: "Music/Hysteria".to_owned(),
            files: vec![EnqueueReleaseFileRequest {
                title: "01 - Women.flac".to_owned(),
                remote_filename: "Music\\Hysteria\\01 - Women.flac".to_owned(),
                size_bytes: 100,
            }],
            expected_track_count: Some(1),
            release_group_id: None,
            alternatives: Vec::new(),
        };
        let queued = release_transfer(
            "queued",
            "release-one",
            "Listener",
            "Music\\Hysteria\\01 - Women.flac",
            TransferStatus::Queued,
            1,
        );
        assert!(unfinished_release_uses_source(
            std::slice::from_ref(&queued),
            &request,
        ));

        let mut completed = queued;
        completed.status = TransferStatus::Completed;
        assert!(!unfinished_release_uses_source(&[completed], &request));

        let other_user = EnqueueReleaseRequest {
            username: "another-listener".to_owned(),
            ..request
        };
        assert!(!unfinished_release_uses_source(
            &[release_transfer(
                "queued",
                "release-one",
                "listener",
                "Music\\Hysteria\\01 - Women.flac",
                TransferStatus::Queued,
                1,
            )],
            &other_user,
        ));
    }

    #[test]
    fn download_lanes_skip_busy_users_and_respect_the_configured_limit() {
        let transfers = vec![
            release_transfer(
                "active-a",
                "release-a",
                "listener-a",
                "A\\01.flac",
                TransferStatus::RemotelyQueued,
                1,
            ),
            release_transfer(
                "same-user",
                "release-b",
                "LISTENER-A",
                "B\\01.flac",
                TransferStatus::Queued,
                1,
            ),
            release_transfer(
                "other-user",
                "release-c",
                "listener-b",
                "C\\01.flac",
                TransferStatus::Queued,
                1,
            ),
        ];
        assert_eq!(next_download_index(&transfers, 3, timestamp_ms()), Some(2));
        assert_eq!(next_download_index(&transfers, 1, timestamp_ms()), None);
    }

    #[test]
    fn completed_files_are_verified_without_modifying_their_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("01.flac");
        fs::write(&path, vec![9_u8; 100]).unwrap();
        let mut transfer = release_transfer(
            "verified",
            "release-verified",
            "listener",
            "Music\\Album\\01.flac",
            TransferStatus::Completed,
            1,
        );
        transfer.local_path = path.to_string_lossy().into_owned();
        refresh_verification(&mut transfer);
        assert_eq!(transfer.verification_status, VerificationStatus::Verified);
        assert_eq!(fs::read(&path).unwrap(), vec![9_u8; 100]);

        fs::remove_file(&path).unwrap();
        refresh_verification(&mut transfer);
        assert_eq!(transfer.verification_status, VerificationStatus::Missing);

        fs::write(&path, vec![9_u8; 60]).unwrap();
        refresh_verification(&mut transfer);
        assert_eq!(
            transfer.verification_status,
            VerificationStatus::SizeMismatch
        );
    }

    #[test]
    fn transfer_store_defaults_signal_relay_to_ten_minutes() {
        let value = serde_json::json!({
            "version": STORE_VERSION,
            "max_concurrent_downloads": 3,
            "transfers": []
        });
        let store: TransferStore = serde_json::from_value(value).unwrap();
        assert_eq!(store.relay_suggestion_minutes, 10);
        assert!(RELAY_SUGGESTION_MINUTES.contains(&0));
        assert!(RELAY_SUGGESTION_MINUTES.contains(&30));
    }

    #[test]
    fn automatic_recovery_uses_three_bounded_delays() {
        assert_eq!(automatic_retry_delay_ms(1), 5_000);
        assert_eq!(automatic_retry_delay_ms(2), 15_000);
        assert_eq!(automatic_retry_delay_ms(3), 45_000);
        assert_eq!(MAX_AUTOMATIC_RETRIES, 3);
    }
}
