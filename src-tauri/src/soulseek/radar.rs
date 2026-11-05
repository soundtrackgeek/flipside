use super::{protocol::SearchResponse, search::SearchResult};
use serde::{Serialize};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

pub const RADAR_EVENT: &str = "flipside://soulseek-radar";
const RADAR_SEARCH_TIMEOUT: Duration = Duration::from_secs(12);
const RADAR_SEARCH_COOLDOWN: Duration = Duration::from_secs(1);
const RADAR_RESULT_LIMIT: usize = 2_000;
const RADAR_EVENT_BATCH_SIZE: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum RadarState {
    Idle,
    Scanning,
    Completed,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum RadarAlbumState {
    Scanning,
    Completed,
    Error,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RadarAlbumScan {
    pub album_id: String,
    pub artist: String,
    pub title: String,
    pub first_release_date: String,
    pub cover_art_url: Option<String>,
    pub state: RadarAlbumState,
    pub result_count: u32,
    pub peer_count: u32,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub error: Option<String>,
}

impl RadarAlbumScan {
    fn query(&self) -> String {
        format!("{} {}", self.artist, self.title)
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RadarSnapshot {
    pub state: RadarState,
    pub albums: Vec<RadarAlbumScan>,
    pub active_album_id: Option<String>,
    pub completed_count: u32,
    pub total_count: u32,
    pub message: String,
    pub updated_at_ms: u64,
}

impl RadarSnapshot {
    fn idle() -> Self {
        Self {
            state: RadarState::Idle,
            albums: Vec::new(),
            active_album_id: None,
            completed_count: 0,
            total_count: 0,
            message: "Shelf Radar is ready.".to_owned(),
            updated_at_ms: timestamp_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RadarEvent {
    event: &'static str,
    snapshot: RadarSnapshot,
    album_id: Option<String>,
    results: Vec<SearchResult>,
}

struct ActiveRadarSearch {
    album_index: usize,
    token: u32,
    deadline: Instant,
    seen: HashSet<String>,
    peers: HashSet<String>,
    next_result_id: u64,
}

struct RadarRuntime {
    snapshot: RadarSnapshot,
    queue: VecDeque<usize>,
    active: Option<ActiveRadarSearch>,
    next_allowed_at: Option<Instant>,
}

impl RadarRuntime {
    fn new() -> Self {
        Self {
            snapshot: RadarSnapshot::idle(),
            queue: VecDeque::new(),
            active: None,
            next_allowed_at: None,
        }
    }

    fn start_next(&mut self, token: u32) -> Option<(String, String)> {
        if self.snapshot.state != RadarState::Scanning || self.active.is_some() {
            return None;
        }
        if self
            .next_allowed_at
            .is_some_and(|allowed| Instant::now() < allowed)
        {
            return None;
        }
        let Some(album_index) = self.queue.pop_front() else {
            self.snapshot.state = RadarState::Completed;
            self.snapshot.active_album_id = None;
            self.snapshot.message = format!(
                "Shelf Radar finished {} {}.",
                self.snapshot.completed_count,
                if self.snapshot.completed_count == 1 {
                    "album"
                } else {
                    "albums"
                }
            );
            self.snapshot.updated_at_ms = timestamp_ms();
            return None;
        };
        let album = &mut self.snapshot.albums[album_index];
        album.state = RadarAlbumState::Scanning;
        album.started_at_ms = Some(timestamp_ms());
        album.finished_at_ms = None;
        album.error = None;
        let album_id = album.album_id.clone();
        let query = album.query();
        self.snapshot.active_album_id = Some(album_id.clone());
        self.snapshot.message = format!("Listening for {}…", album.title);
        self.snapshot.updated_at_ms = timestamp_ms();
        self.active = Some(ActiveRadarSearch {
            album_index,
            token,
            deadline: Instant::now() + RADAR_SEARCH_TIMEOUT,
            seen: HashSet::new(),
            peers: HashSet::new(),
            next_result_id: 0,
        });
        Some((album_id, query))
    }

    fn record(&mut self, response: &SearchResponse) -> Option<(String, Vec<SearchResult>)> {
        let active = self.active.as_mut()?;
        if response.token != active.token {
            return None;
        }
        active.peers.insert(response.username.clone());
        let album_id = self.snapshot.albums[active.album_index].album_id.clone();
        let mut accepted = Vec::new();
        for file in &response.files {
            if active.seen.len() >= RADAR_RESULT_LIMIT {
                break;
            }
            let deduplication_key = format!(
                "{}\u{0}{}\u{0}{}",
                response.username, file.filename, file.size_bytes
            );
            if !active.seen.insert(deduplication_key) {
                continue;
            }
            active.next_result_id += 1;
            accepted.push(SearchResult {
                id: format!("radar:{}:{}", response.token, active.next_result_id),
                token: response.token,
                username: response.username.clone(),
                filename: file.filename.clone(),
                size_bytes: file.size_bytes,
                extension: file.extension.clone(),
                bitrate: file.bitrate,
                duration_seconds: file.duration_seconds,
                vbr: file.vbr,
                sample_rate: file.sample_rate,
                bit_depth: file.bit_depth,
                slot_free: response.slot_free,
                average_speed: response.average_speed,
                queue_length: response.queue_length,
                is_private: file.is_private,
            });
        }
        let album = &mut self.snapshot.albums[active.album_index];
        album.result_count = active.seen.len().try_into().unwrap_or(u32::MAX);
        album.peer_count = active.peers.len().try_into().unwrap_or(u32::MAX);
        self.snapshot.message = format!("{} answered for {}…", album.peer_count, album.title);
        self.snapshot.updated_at_ms = timestamp_ms();
        Some((album_id, accepted))
    }

    fn expire_if_due(&mut self) -> Option<String> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| Instant::now() < active.deadline)
        {
            return None;
        }
        self.finish_active(None)
    }

    fn fail_active(&mut self, message: String) -> Option<String> {
        self.finish_active(Some(message))
    }

    fn finish_active(&mut self, error: Option<String>) -> Option<String> {
        let active = self.active.take()?;
        let album = &mut self.snapshot.albums[active.album_index];
        let album_id = album.album_id.clone();
        album.finished_at_ms = Some(timestamp_ms());
        if let Some(message) = error {
            album.state = RadarAlbumState::Error;
            album.error = Some(message);
        } else {
            album.state = RadarAlbumState::Completed;
            self.snapshot.completed_count = self.snapshot.completed_count.saturating_add(1);
        }
        self.snapshot.active_album_id = None;
        self.snapshot.updated_at_ms = timestamp_ms();
        self.next_allowed_at = Some(Instant::now() + RADAR_SEARCH_COOLDOWN);
        if self.queue.is_empty() {
            self.snapshot.state = if album.state == RadarAlbumState::Error {
                RadarState::Error
            } else {
                RadarState::Completed
            };
            self.snapshot.message = if album.state == RadarAlbumState::Error {
                "Shelf Radar stopped after a search error.".to_owned()
            } else {
                format!(
                    "Shelf Radar finished {} albums.",
                    self.snapshot.completed_count
                )
            };
        } else {
            self.snapshot.message = "Waiting briefly before the next album…".to_owned();
        }
        Some(album_id)
    }

    fn connection_lost(&mut self) -> Option<RadarSnapshot> {
        if self.snapshot.state != RadarState::Scanning {
            return None;
        }
        if let Some(active) = self.active.take() {
            let album = &mut self.snapshot.albums[active.album_index];
            album.state = RadarAlbumState::Error;
            album.error = Some("The Soulseek connection was interrupted.".to_owned());
            album.finished_at_ms = Some(timestamp_ms());
        }
        for index in self.queue.drain(..) {
            let album = &mut self.snapshot.albums[index];
            album.state = RadarAlbumState::Error;
            album.error = Some("Reconnect before scanning this album.".to_owned());
        }
        self.snapshot.state = RadarState::Error;
        self.snapshot.active_album_id = None;
        self.snapshot.message = "Shelf Radar lost the Soulseek connection.".to_owned();
        self.snapshot.updated_at_ms = timestamp_ms();
        Some(self.snapshot.clone())
    }
}

#[derive(Clone)]
pub struct RadarHub {
    app: AppHandle,
    runtime: Arc<Mutex<RadarRuntime>>,
}

impl RadarHub {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            runtime: Arc::new(Mutex::new(RadarRuntime::new())),
        }
    }

    pub fn start_next(&self, token: u32) -> Option<(u32, String)> {
        let (album_id, query, snapshot) = {
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (album_id, query) = runtime.start_next(token)?;
            (album_id, query, runtime.snapshot.clone())
        };
        self.emit("albumStarted", snapshot, Some(album_id), Vec::new());
        Some((token, query))
    }

    pub fn record(&self, response: &SearchResponse) {
        let Some((album_id, results, snapshot)) = ({
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            runtime
                .record(response)
                .map(|(album_id, results)| (album_id, results, runtime.snapshot.clone()))
        }) else {
            return;
        };
        for batch in results.chunks(RADAR_EVENT_BATCH_SIZE) {
            self.emit(
                "results",
                snapshot.clone(),
                Some(album_id.clone()),
                batch.to_vec(),
            );
        }
    }

    pub fn expire_if_due(&self) {
        let completed = {
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            runtime
                .expire_if_due()
                .map(|album_id| (album_id, runtime.snapshot.clone()))
        };
        if let Some((album_id, snapshot)) = completed {
            self.emit("albumCompleted", snapshot, Some(album_id), Vec::new());
        }
    }

    pub fn fail_active(&self, message: impl Into<String>) {
        let failed = {
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            runtime
                .fail_active(message.into())
                .map(|album_id| (album_id, runtime.snapshot.clone()))
        };
        if let Some((album_id, snapshot)) = failed {
            self.emit("error", snapshot, Some(album_id), Vec::new());
        }
    }

    pub fn connection_lost(&self) {
        let snapshot = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .connection_lost();
        if let Some(snapshot) = snapshot {
            self.emit("error", snapshot, None, Vec::new());
        }
    }

    fn emit(
        &self,
        event: &'static str,
        snapshot: RadarSnapshot,
        album_id: Option<String>,
        results: Vec<SearchResult>,
    ) {
        let _ = self.app.emit(
            RADAR_EVENT,
            RadarEvent {
                event,
                snapshot,
                album_id,
                results,
            },
        );
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


