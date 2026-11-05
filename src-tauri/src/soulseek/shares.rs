use super::protocol::{FolderFile, SharedFileListResponse};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
pub struct SharesTicket {
    pub connection_token: u32,
    pub username: String,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShareDirectorySummary {
    pub path: String,
    pub name: String,
    pub parent: Option<String>,
    pub depth: u32,
    pub file_count: u32,
    pub total_size_bytes: u64,
    pub is_private: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UserSharesOverview {
    pub username: String,
    pub directories: Vec<ShareDirectorySummary>,
    pub total_file_count: u32,
    pub total_size_bytes: u64,
    pub public_directory_count: u32,
    pub private_directory_count: u32,
    pub received_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShareFileSnapshot {
    pub remote_filename: String,
    pub directory: String,
    pub filename: String,
    pub size_bytes: u64,
    pub extension: String,
    pub bitrate: Option<u32>,
    pub duration_seconds: Option<u32>,
    pub vbr: Option<bool>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub is_private: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShareFolderSnapshot {
    pub username: String,
    pub directory: String,
    pub is_private: bool,
    pub files: Vec<ShareFileSnapshot>,
    pub total_size_bytes: u64,
}

struct PendingShares {
    ticket: SharesTicket,
    claimed: bool,
    response: oneshot::Sender<Result<UserSharesOverview, SharesError>>,
}

#[derive(Clone, Default)]
pub struct SharesHub {
    pending: Arc<Mutex<HashMap<u32, PendingShares>>>,
}

impl SharesHub {
    pub fn requesting_for_username(&self, username: &str) -> Option<SharesTicket> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .find(|pending| {
                !pending.claimed && pending.ticket.username.eq_ignore_ascii_case(username)
            })
            .map(|pending| pending.ticket.clone())
    }

    pub fn claim_peer(&self, connection_token: u32) -> Option<SharesTicket> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let request = pending.get_mut(&connection_token)?;
        if request.claimed {
            return None;
        }
        request.claimed = true;
        Some(request.ticket.clone())
    }

    pub fn resolve(
        &self,
        connection_token: u32,
        username: &str,
        response: SharedFileListResponse,
    ) -> bool {
        let pending = {
            let mut requests = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(request) = requests.get(&connection_token) else {
                return false;
            };
            if !request.ticket.username.eq_ignore_ascii_case(username) {
                return false;
            }
            requests.remove(&connection_token)
        };
        let Some(pending) = pending else {
            return false;
        };

        let received_at_ms = timestamp_ms();
        let mut folders = HashMap::new();
        for listing in response.directories {
            let directory = normalize_remote_path(&listing.directory);
            if directory.is_empty() {
                continue;
            }
            let mut seen = HashMap::<String, ShareFileSnapshot>::new();
            for file in listing.files {
                let snapshot = file_snapshot(&directory, file, listing.is_private);
                seen.entry(snapshot.remote_filename.to_ascii_lowercase())
                    .or_insert(snapshot);
            }
            let mut files: Vec<_> = seen.into_values().collect();
            files.sort_by(|left, right| {
                left.filename
                    .to_ascii_lowercase()
                    .cmp(&right.filename.to_ascii_lowercase())
            });
            let total_size_bytes = files
                .iter()
                .fold(0_u64, |total, file| total.saturating_add(file.size_bytes));
            folders.insert(
                folder_key(&directory, listing.is_private),
                ShareFolderSnapshot {
                    username: pending.ticket.username.clone(),
                    directory,
                    is_private: listing.is_private,
                    files,
                    total_size_bytes,
                },
            );
        }

        let mut directories: Vec<_> = folders
            .values()
            .map(|folder| ShareDirectorySummary {
                path: folder.directory.clone(),
                name: basename(&folder.directory),
                parent: parent_path(&folder.directory),
                depth: folder
                    .directory
                    .split('\\')
                    .count()
                    .try_into()
                    .unwrap_or(u32::MAX),
                file_count: folder.files.len().try_into().unwrap_or(u32::MAX),
                total_size_bytes: folder.total_size_bytes,
                is_private: folder.is_private,
            })
            .collect();
        directories.sort_by(|left, right| {
            left.path
                .to_ascii_lowercase()
                .cmp(&right.path.to_ascii_lowercase())
                .then(left.is_private.cmp(&right.is_private))
        });
        let total_file_count = directories.iter().fold(0_u32, |total, directory| {
            total.saturating_add(directory.file_count)
        });
        let total_size_bytes = directories.iter().fold(0_u64, |total, directory| {
            total.saturating_add(directory.total_size_bytes)
        });
        let public_directory_count = directories
            .iter()
            .filter(|directory| !directory.is_private)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let private_directory_count = directories
            .iter()
            .filter(|directory| directory.is_private)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let overview = UserSharesOverview {
            username: pending.ticket.username.clone(),
            directories,
            total_file_count,
            total_size_bytes,
            public_directory_count,
            private_directory_count,
            received_at_ms,
        };
        let _ = pending.response.send(Ok(overview));
        true
    }

    pub fn fail_connection(&self, connection_token: u32, message: String) -> bool {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&connection_token);
        if let Some(pending) = pending {
            let _ = pending.response.send(Err(SharesError::Request(message)));
            true
        } else {
            false
        }
    }

    pub fn connection_lost(&self) {
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for (_, request) in pending {
            let _ = request.response.send(Err(SharesError::Request(
                "The Soulseek connection was interrupted while browsing this user's shares."
                    .to_owned(),
            )));
        }
    }
}

fn file_snapshot(directory: &str, file: FolderFile, is_private: bool) -> ShareFileSnapshot {
    let normalized_file = normalize_remote_path(&file.filename);
    let filename = basename(&normalized_file);
    let remote_filename = if normalized_file
        .to_ascii_lowercase()
        .starts_with(&format!("{}\\", directory.to_ascii_lowercase()))
    {
        normalized_file
    } else {
        format!("{directory}\\{normalized_file}")
    };
    ShareFileSnapshot {
        remote_filename,
        directory: directory.to_owned(),
        filename,
        size_bytes: file.size_bytes,
        extension: file.extension,
        bitrate: file.bitrate,
        duration_seconds: file.duration_seconds,
        vbr: file.vbr,
        sample_rate: file.sample_rate,
        bit_depth: file.bit_depth,
        is_private,
    }
}

fn normalize_remote_path(value: &str) -> String {
    value.replace('/', "\\").trim_matches('\\').to_owned()
}

fn basename(value: &str) -> String {
    normalize_remote_path(value)
        .rsplit('\\')
        .next()
        .unwrap_or("Shared folder")
        .to_owned()
}

fn parent_path(value: &str) -> Option<String> {
    normalize_remote_path(value)
        .rsplit_once('\\')
        .map(|(parent, _)| parent.to_owned())
}

fn folder_key(directory: &str, is_private: bool) -> String {
    format!("{}\0{}", directory.to_ascii_lowercase(), is_private)
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[derive(Clone, Debug, Error)]
pub enum SharesError {
    #[error("{0}")]
    Request(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soulseek::protocol::{ShareListing, SharedFileListResponse};

    fn file(name: &str, size: u64) -> FolderFile {
        FolderFile {
            filename: name.to_owned(),
            size_bytes: size,
            extension: "flac".to_owned(),
            bitrate: Some(2_304),
            duration_seconds: Some(300),
            vbr: Some(false),
            sample_rate: Some(96_000),
            bit_depth: Some(24),
        }
    }

}
