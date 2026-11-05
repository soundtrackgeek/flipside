use super::local_shares::IndexedFile;
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, };
use thiserror::Error;
use tauri_specta::Event as _;

const MAX_UPLOAD_QUEUE: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UploadStatus {
    Queued,
    Connecting,
    Uploading,
    Completed,
    Failed,
    Cancelled,
}

impl UploadStatus {
    fn occupies_slot(self) -> bool {
        matches!(self, Self::Connecting | Self::Uploading)
    }

    fn finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UploadSnapshot {
    pub id: String,
    pub username: String,
    pub remote_filename: String,
    pub filename: String,
    pub size_bytes: u64,
    pub transferred_bytes: u64,
    pub speed_bytes_per_second: u64,
    pub eta_seconds: Option<u64>,
    pub status: UploadStatus,
    pub queue_position: Option<u32>,
    pub error: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(skip)]
    local_path: PathBuf,
    #[serde(skip)]
    connection_token: Option<u32>,
    #[serde(skip)]
    file_connection_token: Option<u32>,
    #[serde(skip)]
    transfer_token: Option<u32>,
    #[serde(skip)]
    control_claimed: bool,
    #[serde(skip)]
    file_claimed: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UploadQueueSnapshot {
    pub uploads: Vec<UploadSnapshot>,
    pub active_count: usize,
    pub queued_count: usize,
    pub session_uploaded_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct UploadTicket {
    pub id: String,
    pub username: String,
    pub remote_filename: String,
    pub size_bytes: u64,
    pub local_path: PathBuf,
    pub connection_token: u32,
    pub transfer_token: u32,
}

#[derive(Clone)]
pub struct UploadHub {
    app: AppHandle,
    uploads: Arc<RwLock<Vec<UploadSnapshot>>>,
    tasks: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    next_id: Arc<AtomicU64>,
    next_transfer_token: Arc<AtomicU32>,
}

impl UploadHub {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            uploads: Arc::new(RwLock::new(Vec::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(timestamp_ms())),
            next_transfer_token: Arc::new(AtomicU32::new(
                (timestamp_ms() as u32).wrapping_add(0x6000).max(1),
            )),
        }
    }

    pub fn snapshot(&self) -> UploadQueueSnapshot {
        let uploads = self
            .uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        UploadQueueSnapshot {
            active_count: uploads
                .iter()
                .filter(|upload| upload.status.occupies_slot())
                .count(),
            queued_count: uploads
                .iter()
                .filter(|upload| upload.status == UploadStatus::Queued)
                .count(),
            session_uploaded_bytes: uploads
                .iter()
                .filter(|upload| upload.status == UploadStatus::Completed)
                .map(|upload| upload.size_bytes)
                .sum(),
            uploads,
        }
    }

    pub fn has_free_slot(&self, slots: usize) -> bool {
        self.uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|upload| upload.status.occupies_slot())
            .count()
            < slots
    }

    pub fn queued_count(&self) -> u32 {
        self.uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|upload| upload.status == UploadStatus::Queued)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    pub fn enqueue(
        &self,
        username: &str,
        file: IndexedFile,
    ) -> Result<UploadQueueSnapshot, UploadError> {
        if username.trim().is_empty() {
            return Err(UploadError::InvalidPeer);
        }
        {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if uploads.iter().any(|upload| {
                !upload.status.finished()
                    && upload.username.eq_ignore_ascii_case(username)
                    && upload
                        .remote_filename
                        .eq_ignore_ascii_case(&file.remote_filename)
            }) {
                return Ok(snapshot_from(&uploads));
            }
            if uploads
                .iter()
                .filter(|upload| !upload.status.finished())
                .count()
                >= MAX_UPLOAD_QUEUE
            {
                return Err(UploadError::QueueFull);
            }
            let now = timestamp_ms();
            let id_number = self.next_id.fetch_add(1, Ordering::SeqCst);
            uploads.push(UploadSnapshot {
                id: format!("upload-{now}-{id_number}"),
                username: username.to_owned(),
                remote_filename: file.remote_filename,
                filename: file.filename,
                size_bytes: file.size_bytes,
                transferred_bytes: 0,
                speed_bytes_per_second: 0,
                eta_seconds: None,
                status: UploadStatus::Queued,
                queue_position: None,
                error: None,
                created_at_ms: now,
                updated_at_ms: now,
                local_path: file.local_path,
                connection_token: None,
                file_connection_token: None,
                transfer_token: None,
                control_claimed: false,
                file_claimed: false,
            });
            refresh_queue_positions(&mut uploads);
        }
        self.publish();
        Ok(self.snapshot())
    }

    pub fn queue_position(&self, username: &str, remote_filename: &str) -> Option<u32> {
        self.uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|upload| {
                !upload.status.finished()
                    && upload.username.eq_ignore_ascii_case(username)
                    && upload.remote_filename.eq_ignore_ascii_case(remote_filename)
            })
            .map(|upload| upload.queue_position.unwrap_or(0))
    }

    pub fn activate_next(&self, connection_token: u32, slots: usize) -> Option<UploadTicket> {
        let ticket = {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if uploads
                .iter()
                .filter(|upload| upload.status.occupies_slot())
                .count()
                >= slots
            {
                return None;
            }
            let upload = uploads
                .iter_mut()
                .find(|upload| upload.status == UploadStatus::Queued)?;
            let transfer_token = self.take_transfer_token();
            upload.status = UploadStatus::Connecting;
            upload.connection_token = Some(connection_token);
            upload.transfer_token = Some(transfer_token);
            upload.control_claimed = false;
            upload.file_claimed = false;
            upload.queue_position = None;
            upload.error = None;
            upload.updated_at_ms = timestamp_ms();
            let ticket = ticket_from(upload, connection_token, transfer_token);
            refresh_queue_positions(&mut uploads);
            ticket
        };
        self.publish();
        Some(ticket)
    }

    pub fn requesting_control_for_username(&self, username: &str) -> Option<UploadTicket> {
        self.uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|upload| {
                upload.status == UploadStatus::Connecting
                    && !upload.control_claimed
                    && upload.file_connection_token.is_none()
                    && upload.username.eq_ignore_ascii_case(username)
            })
            .and_then(|upload| {
                Some(ticket_from(
                    upload,
                    upload.connection_token?,
                    upload.transfer_token?,
                ))
            })
    }

    pub fn claim_control(&self, connection_token: u32) -> Option<UploadTicket> {
        let ticket = {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let upload = uploads.iter_mut().find(|upload| {
                upload.status == UploadStatus::Connecting
                    && !upload.control_claimed
                    && upload.connection_token == Some(connection_token)
            })?;
            upload.control_claimed = true;
            upload.updated_at_ms = timestamp_ms();
            let transfer_token = upload.transfer_token?;
            ticket_from(upload, connection_token, transfer_token)
        };
        self.publish();
        Some(ticket)
    }

    pub fn prepare_file_connection(&self, id: &str, connection_token: u32) -> Option<UploadTicket> {
        let ticket = {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let upload = uploads
                .iter_mut()
                .find(|upload| upload.id == id && upload.status == UploadStatus::Connecting)?;
            upload.file_connection_token = Some(connection_token);
            upload.file_claimed = false;
            upload.updated_at_ms = timestamp_ms();
            let transfer_token = upload.transfer_token?;
            ticket_from(upload, connection_token, transfer_token)
        };
        self.publish();
        Some(ticket)
    }

    pub fn requesting_file_for_username(&self, username: &str) -> Option<UploadTicket> {
        self.uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|upload| {
                upload.status == UploadStatus::Connecting
                    && !upload.file_claimed
                    && upload.file_connection_token.is_some()
                    && upload.username.eq_ignore_ascii_case(username)
            })
            .and_then(|upload| {
                Some(ticket_from(
                    upload,
                    upload.file_connection_token?,
                    upload.transfer_token?,
                ))
            })
    }

    pub fn claim_file(&self, connection_token: u32) -> Option<UploadTicket> {
        let ticket = {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let upload = uploads.iter_mut().find(|upload| {
                upload.status == UploadStatus::Connecting
                    && !upload.file_claimed
                    && upload.file_connection_token == Some(connection_token)
            })?;
            upload.file_claimed = true;
            upload.updated_at_ms = timestamp_ms();
            let transfer_token = upload.transfer_token?;
            ticket_from(upload, connection_token, transfer_token)
        };
        self.publish();
        Some(ticket)
    }

    pub fn begin_file(&self, id: &str) -> Result<UploadTicket, UploadError> {
        let ticket = {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let upload = uploads
                .iter_mut()
                .find(|upload| upload.id == id && upload.status == UploadStatus::Connecting)
                .ok_or(UploadError::NotFound)?;
            let metadata = fs::metadata(&upload.local_path)?;
            if !metadata.is_file() || metadata.len() != upload.size_bytes {
                return Err(UploadError::ChangedFile);
            }
            upload.status = UploadStatus::Uploading;
            upload.speed_bytes_per_second = 0;
            upload.eta_seconds = None;
            upload.updated_at_ms = timestamp_ms();
            let connection_token = upload.file_connection_token.unwrap_or_default();
            let transfer_token = upload.transfer_token.ok_or(UploadError::NotFound)?;
            ticket_from(upload, connection_token, transfer_token)
        };
        self.publish();
        Ok(ticket)
    }

    pub fn update_progress(&self, id: &str, transferred: u64, speed: u64) {
        {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(upload) = uploads.iter_mut().find(|upload| upload.id == id) {
                if upload.status == UploadStatus::Uploading {
                    upload.transferred_bytes = transferred.min(upload.size_bytes);
                    upload.speed_bytes_per_second = speed;
                    upload.eta_seconds = (speed > 0).then(|| {
                        upload
                            .size_bytes
                            .saturating_sub(upload.transferred_bytes)
                            .div_ceil(speed)
                    });
                    upload.updated_at_ms = timestamp_ms();
                }
            }
        }
        self.publish();
    }

    pub fn complete(&self, id: &str) -> bool {
        self.finish(id, UploadStatus::Completed, None)
    }

    pub fn fail_id(&self, id: &str, message: String) -> bool {
        self.finish(id, UploadStatus::Failed, Some(message))
    }

    pub fn fail_connection(&self, connection_token: u32, message: String) -> bool {
        let id = self
            .uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|upload| {
                upload.status == UploadStatus::Connecting
                    && (upload.connection_token == Some(connection_token)
                        || upload.file_connection_token == Some(connection_token))
            })
            .map(|upload| upload.id.clone());
        id.is_some_and(|id| self.fail_id(&id, message))
    }

    pub fn cancel(&self, id: &str) -> Result<UploadQueueSnapshot, UploadError> {
        let finished = self
            .uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|upload| upload.id == id)
            .map(|upload| upload.status.finished())
            .ok_or(UploadError::NotFound)?;
        if finished {
            self.uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retain(|upload| upload.id != id);
            self.publish();
            return Ok(self.snapshot());
        }
        if !self.finish(id, UploadStatus::Cancelled, None) {
            return Err(UploadError::NotFound);
        }
        Ok(self.snapshot())
    }

    pub fn clear_finished(&self) -> UploadQueueSnapshot {
        {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            uploads.retain(|upload| !upload.status.finished());
            refresh_queue_positions(&mut uploads);
        }
        self.publish();
        self.snapshot()
    }

    pub fn connection_lost(&self) {
        let active_ids: Vec<String> = self
            .uploads
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|upload| upload.status.occupies_slot())
            .map(|upload| upload.id.clone())
            .collect();
        for id in active_ids {
            self.abort(&id);
        }
        {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for upload in uploads
                .iter_mut()
                .filter(|upload| upload.status.occupies_slot())
            {
                upload.status = UploadStatus::Queued;
                upload.transferred_bytes = 0;
                upload.speed_bytes_per_second = 0;
                upload.eta_seconds = None;
                upload.connection_token = None;
                upload.file_connection_token = None;
                upload.transfer_token = None;
                upload.control_claimed = false;
                upload.file_claimed = false;
                upload.updated_at_ms = timestamp_ms();
            }
            refresh_queue_positions(&mut uploads);
        }
        self.publish();
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

    fn take_transfer_token(&self) -> u32 {
        loop {
            let token = self.next_transfer_token.fetch_add(1, Ordering::SeqCst);
            if token != 0 {
                return token;
            }
        }
    }

    fn finish(&self, id: &str, status: UploadStatus, error: Option<String>) -> bool {
        self.abort(id);
        let changed = {
            let mut uploads = self
                .uploads
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(upload) = uploads.iter_mut().find(|upload| upload.id == id) else {
                return false;
            };
            if upload.status.finished() && status != UploadStatus::Cancelled {
                return false;
            }
            upload.status = status;
            if status == UploadStatus::Completed {
                upload.transferred_bytes = upload.size_bytes;
                upload.eta_seconds = Some(0);
            } else {
                upload.eta_seconds = None;
            }
            upload.speed_bytes_per_second = 0;
            upload.queue_position = None;
            upload.error = error;
            upload.connection_token = None;
            upload.file_connection_token = None;
            upload.transfer_token = None;
            upload.updated_at_ms = timestamp_ms();
            refresh_queue_positions(&mut uploads);
            true
        };
        if changed {
            self.publish();
        }
        changed
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

    fn publish(&self) {
        let _ = crate::events::SoulseekUploadsChanged(self.snapshot()).emit(&self.app);
    }
}

fn ticket_from(
    upload: &UploadSnapshot,
    connection_token: u32,
    transfer_token: u32,
) -> UploadTicket {
    UploadTicket {
        id: upload.id.clone(),
        username: upload.username.clone(),
        remote_filename: upload.remote_filename.clone(),
        size_bytes: upload.size_bytes,
        local_path: upload.local_path.clone(),
        connection_token,
        transfer_token,
    }
}

fn refresh_queue_positions(uploads: &mut [UploadSnapshot]) {
    let mut position = 1_u32;
    for upload in uploads {
        if upload.status == UploadStatus::Queued {
            upload.queue_position = Some(position);
            position = position.saturating_add(1);
        } else if upload.status.occupies_slot() {
            upload.queue_position = Some(0);
        } else {
            upload.queue_position = None;
        }
    }
}

fn snapshot_from(uploads: &[UploadSnapshot]) -> UploadQueueSnapshot {
    UploadQueueSnapshot {
        uploads: uploads.to_vec(),
        active_count: uploads
            .iter()
            .filter(|upload| upload.status.occupies_slot())
            .count(),
        queued_count: uploads
            .iter()
            .filter(|upload| upload.status == UploadStatus::Queued)
            .count(),
        session_uploaded_bytes: uploads
            .iter()
            .filter(|upload| upload.status == UploadStatus::Completed)
            .map(|upload| upload.size_bytes)
            .sum(),
    }
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
pub enum UploadError {
    #[error("The requesting Soulseek user is not valid.")]
    InvalidPeer,
    #[error("The upload queue is full. Try again later.")]
    QueueFull,
    #[error("That upload is no longer available.")]
    NotFound,
    #[error("The shared file changed after it was indexed. Rescan your shares.")]
    ChangedFile,
    #[error("Flipside could not read the shared file: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_positions_ignore_finished_uploads() {
        let mut uploads = vec![
            UploadSnapshot {
                id: "done".to_owned(),
                username: "one".to_owned(),
                remote_filename: "Music\\done.flac".to_owned(),
                filename: "done.flac".to_owned(),
                size_bytes: 1,
                transferred_bytes: 1,
                speed_bytes_per_second: 0,
                eta_seconds: Some(0),
                status: UploadStatus::Completed,
                queue_position: None,
                error: None,
                created_at_ms: 0,
                updated_at_ms: 0,
                local_path: PathBuf::new(),
                connection_token: None,
                file_connection_token: None,
                transfer_token: None,
                control_claimed: false,
                file_claimed: false,
            },
            UploadSnapshot {
                id: "queued".to_owned(),
                username: "two".to_owned(),
                remote_filename: "Music\\queued.flac".to_owned(),
                filename: "queued.flac".to_owned(),
                size_bytes: 1,
                transferred_bytes: 0,
                speed_bytes_per_second: 0,
                eta_seconds: None,
                status: UploadStatus::Queued,
                queue_position: None,
                error: None,
                created_at_ms: 0,
                updated_at_ms: 0,
                local_path: PathBuf::new(),
                connection_token: None,
                file_connection_token: None,
                transfer_token: None,
                control_claimed: false,
                file_claimed: false,
            },
        ];
        refresh_queue_positions(&mut uploads);
        assert_eq!(uploads[0].queue_position, None);
        assert_eq!(uploads[1].queue_position, Some(1));
    }
}
