use super::{
    credentials::CredentialVault,
    diagnostics::{DiagnosticEntry, Diagnostics},
    distributed::{DistributedHub, DistributedSnapshot, RequestAdmission},
    downloads::{
        DownloadPlan, EnqueueReleaseRequest, TransferError, TransferHub,         TransferQueueSnapshot, TransferTicket,
    },
    folders::{FolderError, FolderHub, FolderTicket},
    local_shares::{
        LocalSharesError, LocalSharesHub, LocalSharesSnapshot, SearchResponseOrigin,
        SearchResponseTicket,
    },
    messages::{MessagesError, MessagesHub, },
    people::{PeopleError, PeopleHub, ProfileTicket},
    protocol::{
        accept_children_frame, branch_level_frame, branch_root_frame, cant_connect_to_peer_frame,
        connect_to_peer_frame, file_search_frame, file_search_response_frame,
        folder_contents_request_frame, folder_contents_response_frame, get_peer_address_frame,
        have_no_parent_frame, join_room_frame, login_frame, message_acked_frame,
        parse_cant_connect_token, parse_connect_to_peer,
        parse_distributed_branch_level, parse_distributed_branch_root, parse_distributed_search,
        parse_embedded_distributed_search, parse_filename, parse_folder_contents_request,
        parse_folder_contents_response, parse_join_room, parse_leave_room, parse_login_response,
        parse_peer_address, parse_possible_parents, parse_private_message, parse_queue_position,
        parse_room_chat_message, parse_room_list, parse_search_response,
        parse_server_search_request, parse_shared_file_list_response, parse_transfer_request,
        parse_transfer_response, parse_upload_denied, parse_user_info_response,
        parse_user_interests, parse_user_joined_room, parse_user_left_room, parse_user_stats,
        parse_user_status, parse_watch_user, peer_init_frame, pierce_firewall_frame,
        place_in_queue_request_frame, place_in_queue_response_frame, queue_upload_frame,
        read_distributed_frame, read_frame, read_peer_frame, read_peer_init, read_profile_frame,
        room_list_frame, server_ping_frame, set_online_frame,
        set_wait_port_frame, shared_counts_frame, shared_file_list_request_frame,
        shared_file_list_response_frame, transfer_request_frame, transfer_response_frame,
        upload_denied_frame, user_info_request_frame, user_info_response_frame,
        watch_user_frame, write_raw_frame, ConnectToPeer,
        DistributedFrame, DistributedSearch, Frame, LoginResponse, ParentCandidate, PeerAddress,
        PeerInit, ProtocolError, CANT_CONNECT_TO_PEER_CODE, CONNECT_TO_PEER_CODE,
        DISTRIBUTED_BRANCH_LEVEL_CODE, DISTRIBUTED_BRANCH_ROOT_CODE, DISTRIBUTED_SEARCH_CODE,
        EMBEDDED_MESSAGE_CODE, FILE_SEARCH_CODE, FILE_SEARCH_RESPONSE_CODE,
        FOLDER_CONTENTS_REQUEST_CODE, FOLDER_CONTENTS_RESPONSE_CODE, GET_PEER_ADDRESS_CODE,
        JOIN_ROOM_CODE, LEAVE_ROOM_CODE, MESSAGE_USER_CODE, PLACE_IN_QUEUE_REQUEST_CODE,
        PLACE_IN_QUEUE_RESPONSE_CODE, POSSIBLE_PARENTS_CODE, QUEUE_UPLOAD_CODE, RELOGGED_CODE,
        RESET_DISTRIBUTED_CODE, ROOM_LIST_CODE, SAY_CHATROOM_CODE, SHARED_FILE_LIST_REQUEST_CODE,
        SHARED_FILE_LIST_RESPONSE_CODE, TRANSFER_REQUEST_CODE, UPLOAD_DENIED_CODE,
        UPLOAD_FAILED_CODE, USER_INFO_REQUEST_CODE, USER_INFO_RESPONSE_CODE, USER_INTERESTS_CODE,
        USER_JOINED_ROOM_CODE, USER_LEFT_ROOM_CODE, USER_STATS_CODE, USER_STATUS_CODE,
        WATCH_USER_CODE,
    },
    radar::RadarHub,
    rooms::{RoomsError, RoomsHub, },
    search::{SearchHub, SearchSnapshot, SearchState},
    settings::{ConnectionProfile, SettingsStore},
    shares::{
        SharesError, SharesHub, SharesTicket,
            },
    uploads::{UploadError, UploadHub, UploadQueueSnapshot, UploadTicket},
    wanted::{
        WantedError,
        WantedHub, },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::SeekFrom,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, };
use thiserror::Error;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Semaphore},
    time::timeout,
};
use zeroize::Zeroizing;
use tauri_specta::Event as _;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(12);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(60);
const PEER_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const PEER_MESSAGE_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_CONCURRENT_PEERS: usize = 32;
const SERVER_FRAME_QUEUE_SIZE: usize = 64;
const MAX_SEARCH_QUERY_BYTES: usize = 250;
const MAX_SEARCH_USERNAME_BYTES: usize = 100;
const TRANSFER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const PEER_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const FILE_BUFFER_SIZE: usize = 128 * 1024;
const DISTRIBUTED_EVENT_QUEUE_SIZE: usize = 256;
const DISTRIBUTED_PARENT_IDLE_TIMEOUT: Duration = Duration::from_secs(3 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Unconfigured,
    Offline,
    Connecting,
    Authenticating,
    Online,
    Reconnecting,
    Error,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub state: ConnectionState,
    pub username: Option<String>,
    pub server: Option<String>,
    pub message: String,
    pub attempt: u32,
    pub connected_at_ms: Option<u64>,
    pub retry_in_seconds: Option<u64>,
    pub updated_at_ms: u64,
}

impl ConnectionSnapshot {
    fn unconfigured() -> Self {
        Self {
            state: ConnectionState::Unconfigured,
            username: None,
            server: None,
            message: "Add your Soulseek account to get started.".to_owned(),
            attempt: 0,
            connected_at_ms: None,
            retry_in_seconds: None,
            updated_at_ms: timestamp_ms(),
        }
    }

    fn offline(profile: &ConnectionProfile) -> Self {
        Self {
            state: ConnectionState::Offline,
            username: Some(profile.username.clone()),
            server: Some(server_label(profile)),
            message: "Ready to connect.".to_owned(),
            attempt: 0,
            connected_at_ms: None,
            retry_in_seconds: None,
            updated_at_ms: timestamp_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionBootstrap {
    pub profile: Option<ConnectionProfile>,
    pub suggested_profile: ConnectionProfile,
    pub has_password: bool,
    pub snapshot: ConnectionSnapshot,
    pub diagnostics_path: String,
    pub diagnostics: Vec<DiagnosticEntry>,
    pub search_network: DistributedSnapshot,
}

#[derive(Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnectionRequest {
    pub profile: ConnectionProfile,
    pub password: Option<String>,
}

struct ActiveTask {
    generation: u64,
    handle: tauri::async_runtime::JoinHandle<()>,
}

struct AbortOnDrop(tauri::async_runtime::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct DistributedCandidateTask {
    branch_level: Option<u32>,
    branch_root: Option<String>,
    _task: AbortOnDrop,
}

enum DistributedPeerEvent {
    Frame { id: u64, frame: DistributedFrame },
    Closed { id: u64 },
}

struct DistributedCoordinator {
    next_id: u64,
    candidates: HashMap<u64, DistributedCandidateTask>,
    active_parent: Option<u64>,
    server_is_parent: bool,
}

impl DistributedCoordinator {
    fn new() -> Self {
        Self {
            next_id: 1,
            candidates: HashMap::new(),
            active_parent: None,
            server_is_parent: false,
        }
    }

    fn start_candidates(
        &mut self,
        parents: Vec<ParentCandidate>,
        own_username: &str,
        event_sender: &mpsc::Sender<DistributedPeerEvent>,
    ) {
        if self.active_parent.is_some() || self.server_is_parent || !self.candidates.is_empty() {
            return;
        }
        self.candidates.clear();
        let mut usernames = std::collections::HashSet::new();
        for parent in parents {
            if parent.username.eq_ignore_ascii_case(own_username)
                || !usernames.insert(parent.username.to_ascii_lowercase())
            {
                continue;
            }
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            let task = spawn_distributed_parent_candidate(
                id,
                parent.clone(),
                own_username.to_owned(),
                event_sender.clone(),
            );
            self.candidates.insert(
                id,
                DistributedCandidateTask {
                    branch_level: None,
                    branch_root: None,
                    _task: task,
                },
            );
        }
    }

    fn update_branch_level(&mut self, id: u64, level: u32) -> Option<u32> {
        let candidate = self.candidates.get_mut(&id)?;
        candidate.branch_level = Some(level);
        (self.active_parent == Some(id)).then_some(level.saturating_add(1))
    }

    fn update_branch_root(&mut self, id: u64, root: String) -> Option<String> {
        let candidate = self.candidates.get_mut(&id)?;
        candidate.branch_root = Some(root.clone());
        (self.active_parent == Some(id)).then_some(root)
    }

    fn adopt(&mut self, id: u64) -> Option<(u32, String)> {
        if self.active_parent.is_some() || self.server_is_parent {
            return None;
        }
        let candidate = self.candidates.get(&id)?;
        let level = candidate.branch_level?.saturating_add(1);
        let root = candidate.branch_root.clone()?;
        let candidate = self.candidates.remove(&id)?;
        self.candidates.clear();
        self.candidates.insert(id, candidate);
        self.active_parent = Some(id);
        Some((level, root))
    }

    fn accepts_search_from(&self, id: u64) -> bool {
        self.active_parent == Some(id)
    }

    fn close(&mut self, id: u64) -> bool {
        self.candidates.remove(&id);
        if self.active_parent == Some(id) {
            self.active_parent = None;
            return true;
        }
        false
    }

    fn become_branch_root(&mut self) -> bool {
        let changed = !self.server_is_parent;
        self.candidates.clear();
        self.active_parent = None;
        self.server_is_parent = true;
        changed
    }

    fn reset(&mut self) {
        self.candidates.clear();
        self.active_parent = None;
        self.server_is_parent = false;
    }
}

enum ConnectionCommand {
    StartSearch {
        token: u32,
        query: String,
    },
    RefreshRooms,
    JoinRoom {
        room: String,
    },
    WatchPerson {
        username: String,
    },
    PeerConnectionFailed {
        token: u32,
        username: String,
    },
    ScheduleDownloads,
    ScheduleUploads,
    OpenUploadFile {
        id: String,
    },
    RefreshSharedCounts,
}

#[derive(Clone)]
struct PeerServices {
    search: SearchHub,
    wanted: WantedHub,
    radar: RadarHub,
    folders: FolderHub,
    shares: SharesHub,
    people: PeopleHub,
    transfers: TransferHub,
    local_shares: LocalSharesHub,
    uploads: UploadHub,
    distributed: DistributedHub,
    own_username: String,
    command_sender: mpsc::UnboundedSender<ConnectionCommand>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PeerMessagePurpose {
    General,
    Transfer,
    Folder,
    Shares(u32),
    Profile(u32),
}

#[derive(Clone)]
pub struct ConnectionManager {
    app: AppHandle,
    settings: SettingsStore,
    vault: CredentialVault,
    diagnostics: Diagnostics,
    suggested_profile: ConnectionProfile,
    snapshot: Arc<RwLock<ConnectionSnapshot>>,
    task: Arc<Mutex<Option<ActiveTask>>>,
    command_sender: Arc<Mutex<Option<mpsc::UnboundedSender<ConnectionCommand>>>>,
    generation: Arc<AtomicU64>,
    next_search_token: Arc<AtomicU32>,
    next_connection_token: Arc<AtomicU32>,
    search: SearchHub,
    wanted: WantedHub,
    radar: RadarHub,
    folders: FolderHub,
    shares: SharesHub,
    people: PeopleHub,
    messages: MessagesHub,
    rooms: RoomsHub,
    transfers: TransferHub,
    local_shares: LocalSharesHub,
    uploads: UploadHub,
    distributed: DistributedHub,
}

pub struct ConnectionPaths {
    pub settings: PathBuf,
    pub transfers: PathBuf,
    pub sharing: PathBuf,
    pub people: PathBuf,
    pub messages: PathBuf,
    pub rooms: PathBuf,
    pub wanted: PathBuf,
    pub diagnostics: PathBuf,
}

impl ConnectionManager {
    pub fn new(
        app: AppHandle,
        paths: ConnectionPaths,
        download_directory: PathBuf,
    ) -> Result<Self, ConnectionServiceError> {
        let settings = SettingsStore::new(paths.settings);
        let diagnostics = Diagnostics::new(paths.diagnostics)?;
        let profile = settings.load()?;
        let snapshot = profile
            .as_ref()
            .map(ConnectionSnapshot::offline)
            .unwrap_or_else(ConnectionSnapshot::unconfigured);
        let search = SearchHub::new(app.clone());
        let transfers = TransferHub::new(app.clone(), paths.transfers)?;
        let local_shares = LocalSharesHub::new(app.clone(), paths.sharing)?;
        let uploads = UploadHub::new(app.clone());
        let distributed = DistributedHub::new(app.clone());
        let people = PeopleHub::new(app.clone(), paths.people)?;
        let messages = MessagesHub::new(app.clone(), paths.messages)?;
        let rooms = RoomsHub::new(app.clone(), paths.rooms)?;
        let wanted = WantedHub::new(app.clone(), paths.wanted)?;
        let radar = RadarHub::new(app.clone());

        Ok(Self {
            app,
            settings,
            vault: CredentialVault::default(),
            diagnostics,
            suggested_profile: ConnectionProfile::suggested(&download_directory),
            snapshot: Arc::new(RwLock::new(snapshot)),
            task: Arc::new(Mutex::new(None)),
            command_sender: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            next_search_token: Arc::new(AtomicU32::new((timestamp_ms() as u32).max(1))),
            next_connection_token: Arc::new(AtomicU32::new(
                (timestamp_ms() as u32).wrapping_add(0x4000).max(1),
            )),
            search,
            wanted,
            radar,
            folders: FolderHub::default(),
            shares: SharesHub::default(),
            people,
            messages,
            rooms,
            transfers,
            local_shares,
            uploads,
            distributed,
        })
    }

    pub fn bootstrap(&self) -> Result<ConnectionBootstrap, ConnectionServiceError> {
        let profile = self.settings.load()?;
        let has_password = match profile.as_ref() {
            Some(profile) => self.vault.has(&profile.username)?,
            None => false,
        };

        Ok(ConnectionBootstrap {
            profile,
            suggested_profile: self.suggested_profile.clone(),
            has_password,
            snapshot: self.current_snapshot(),
            diagnostics_path: self.diagnostics.path().to_string_lossy().into_owned(),
            diagnostics: self.diagnostics.recent(),
            search_network: self.distributed.snapshot(),
        })
    }

    pub fn save_profile(
        &self,
        request: SaveConnectionRequest,
    ) -> Result<ConnectionBootstrap, ConnectionServiceError> {
        request.profile.validate()?;
        let previous = self.settings.load()?;
        let username_changed = previous
            .as_ref()
            .is_some_and(|profile| profile.username != request.profile.username);

        let password = match request.password.filter(|password| !password.is_empty()) {
            Some(password) => Some(Zeroizing::new(password)),
            None if username_changed => None,
            None => self.vault.get(&request.profile.username)?,
        }
        .ok_or(ConnectionServiceError::MissingPassword)?;

        self.vault.store(
            &request.profile.username,
            password.to_string(),
            request.profile.remember_password,
        )?;
        self.settings.save(&request.profile)?;

        if let Some(previous) = previous {
            if previous.username != request.profile.username {
                self.vault.forget(&previous.username)?;
            }
        }

        self.stop_active_task();
        self.diagnostics.record(
            "info",
            "profile_saved",
            "Soulseek connection settings were saved.",
        );
        self.publish(ConnectionSnapshot::offline(&request.profile));
        self.bootstrap()
    }

    pub fn connect(&self) -> Result<ConnectionSnapshot, ConnectionServiceError> {
        let profile = self
            .settings
            .load()?
            .ok_or(ConnectionServiceError::NotConfigured)?;
        let password = self
            .vault
            .get(&profile.username)?
            .ok_or(ConnectionServiceError::MissingPassword)?;

        self.stop_active_task();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let manager = self.clone();
        let handle = tauri::async_runtime::spawn(async move {
            manager
                .run_connection_loop(profile, password, generation)
                .await;
            manager.clear_task(generation);
        });
        *self
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(ActiveTask { generation, handle });

        Ok(self.current_snapshot())
    }

    pub fn disconnect(&self) -> Result<ConnectionSnapshot, ConnectionServiceError> {
        self.stop_active_task();
        self.rooms.disconnected();
        let snapshot = match self.settings.load()? {
            Some(profile) => ConnectionSnapshot::offline(&profile),
            None => ConnectionSnapshot::unconfigured(),
        };
        self.diagnostics
            .record("info", "disconnected", "Disconnected by the user.");
        self.publish(snapshot.clone());
        Ok(snapshot)
    }

    pub fn reset(&self) -> Result<ConnectionBootstrap, ConnectionServiceError> {
        self.stop_active_task();
        self.rooms.disconnected();
        if let Some(profile) = self.settings.load()? {
            self.vault.forget(&profile.username)?;
        }
        self.settings.delete()?;
        self.diagnostics.record(
            "info",
            "profile_removed",
            "Soulseek account settings and stored credentials were removed.",
        );
        self.publish(ConnectionSnapshot::unconfigured());
        self.bootstrap()
    }

    pub fn current_searches(&self) -> Vec<SearchSnapshot> {
        self.search.snapshots()
    }

    pub fn current_transfers(&self) -> TransferQueueSnapshot {
        self.transfers.snapshot()
    }

    pub fn set_max_concurrent_downloads(
        &self,
        max_concurrent_downloads: u8,
    ) -> Result<TransferQueueSnapshot, ConnectionServiceError> {
        let snapshot = self
            .transfers
            .set_max_concurrent_downloads(max_concurrent_downloads)?;
        self.schedule_downloads();
        Ok(snapshot)
    }

    pub fn current_local_shares(&self) -> LocalSharesSnapshot {
        self.local_shares.snapshot()
    }

    pub fn add_local_share(
        &self,
        path: &str,
    ) -> Result<LocalSharesSnapshot, ConnectionServiceError> {
        let snapshot = self.local_shares.add_root(path)?;
        self.refresh_shared_counts();
        Ok(snapshot)
    }

    pub fn remove_local_share(
        &self,
        id: &str,
    ) -> Result<LocalSharesSnapshot, ConnectionServiceError> {
        let snapshot = self.local_shares.remove_root(id)?;
        self.refresh_shared_counts();
        Ok(snapshot)
    }

    pub fn set_local_share_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<LocalSharesSnapshot, ConnectionServiceError> {
        let snapshot = self.local_shares.set_enabled(id, enabled)?;
        self.refresh_shared_counts();
        Ok(snapshot)
    }

    pub fn rescan_local_shares(&self) -> Result<LocalSharesSnapshot, ConnectionServiceError> {
        let snapshot = self.local_shares.scan()?;
        self.refresh_shared_counts();
        Ok(snapshot)
    }

    pub fn set_upload_slots(
        &self,
        upload_slots: u8,
    ) -> Result<LocalSharesSnapshot, ConnectionServiceError> {
        let snapshot = self.local_shares.set_upload_slots(upload_slots)?;
        self.schedule_uploads();
        Ok(snapshot)
    }

    pub fn current_uploads(&self) -> UploadQueueSnapshot {
        self.uploads.snapshot()
    }

    pub fn cancel_upload(&self, id: &str) -> Result<UploadQueueSnapshot, ConnectionServiceError> {
        let snapshot = self.uploads.cancel(id)?;
        self.schedule_uploads();
        Ok(snapshot)
    }

    pub fn clear_finished_uploads(&self) -> UploadQueueSnapshot {
        self.uploads.clear_finished()
    }

    pub fn enqueue_release(
        &self,
        request: EnqueueReleaseRequest,
    ) -> Result<TransferQueueSnapshot, ConnectionServiceError> {
        let profile = self
            .settings
            .load()?
            .ok_or(ConnectionServiceError::NotConfigured)?;
        self.people.remember(&request.username)?;
        let snapshot = self
            .transfers
            .enqueue_release(request, Path::new(&profile.download_directory))?;
        self.schedule_downloads();
        Ok(snapshot)
    }

    pub fn pause_release(
        &self,
        release_id: &str,
    ) -> Result<TransferQueueSnapshot, ConnectionServiceError> {
        let snapshot = self.transfers.pause_release(release_id)?;
        self.schedule_downloads();
        Ok(snapshot)
    }

    pub fn resume_release(
        &self,
        release_id: &str,
    ) -> Result<TransferQueueSnapshot, ConnectionServiceError> {
        let snapshot = self.transfers.resume_release(release_id)?;
        self.schedule_downloads();
        Ok(snapshot)
    }

    pub fn cancel_release(
        &self,
        release_id: &str,
    ) -> Result<TransferQueueSnapshot, ConnectionServiceError> {
        let snapshot = self.transfers.cancel_release(release_id)?;
        self.schedule_downloads();
        Ok(snapshot)
    }

    pub fn clear_completed_transfers(
        &self,
    ) -> Result<TransferQueueSnapshot, ConnectionServiceError> {
        Ok(self.transfers.clear_completed()?)
    }

    pub fn reveal_release_path(&self, release_id: &str) -> Result<String, ConnectionServiceError> {
        Ok(self.transfers.reveal_release_path(release_id)?)
    }

    fn schedule_downloads(&self) {
        if self.current_snapshot().state != ConnectionState::Online {
            return;
        }
        if let Some(sender) = self
            .command_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            let _ = sender.send(ConnectionCommand::ScheduleDownloads);
        }
    }

    fn schedule_uploads(&self) {
        if self.current_snapshot().state != ConnectionState::Online {
            return;
        }
        if let Some(sender) = self
            .command_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            let _ = sender.send(ConnectionCommand::ScheduleUploads);
        }
    }

    fn refresh_shared_counts(&self) {
        if let Some(sender) = self
            .command_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            let _ = sender.send(ConnectionCommand::RefreshSharedCounts);
        }
    }

    fn peer_services(
        &self,
        own_username: &str,
        command_sender: &mpsc::UnboundedSender<ConnectionCommand>,
    ) -> PeerServices {
        PeerServices {
            search: self.search.clone(),
            wanted: self.wanted.clone(),
            radar: self.radar.clone(),
            folders: self.folders.clone(),
            shares: self.shares.clone(),
            people: self.people.clone(),
            transfers: self.transfers.clone(),
            local_shares: self.local_shares.clone(),
            uploads: self.uploads.clone(),
            distributed: self.distributed.clone(),
            own_username: own_username.to_owned(),
            command_sender: command_sender.clone(),
        }
    }

    fn take_connection_token(&self) -> u32 {
        loop {
            let token = self.next_connection_token.fetch_add(1, Ordering::SeqCst);
            if token != 0 {
                return token;
            }
        }
    }

    pub fn start_search(
        &self,
        client_id: String,
        query: String,
    ) -> Result<SearchSnapshot, ConnectionServiceError> {
        let query = query.trim().to_owned();
        let client_id = client_id.trim().to_owned();
        if query.is_empty() {
            return Err(ConnectionServiceError::InvalidSearch(
                "Enter something to search for.".to_owned(),
            ));
        }
        if query.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(ConnectionServiceError::InvalidSearch(format!(
                "Search queries can be at most {MAX_SEARCH_QUERY_BYTES} bytes."
            )));
        }
        if client_id.is_empty()
            || client_id.len() > 64
            || !client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ConnectionServiceError::InvalidSearch(
                "The search session identifier is invalid.".to_owned(),
            ));
        }
        if self.current_snapshot().state != ConnectionState::Online {
            return Err(ConnectionServiceError::SearchUnavailable);
        }

        let sender = self
            .command_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or(ConnectionServiceError::SearchUnavailable)?;
        let mut token = self.next_search_token.fetch_add(1, Ordering::SeqCst);
        if token == 0 {
            token = self.next_search_token.fetch_add(1, Ordering::SeqCst);
        }

        let snapshot = self
            .search
            .start(token, client_id.clone(), query.clone())
            .map_err(ConnectionServiceError::InvalidSearch)?;
        if sender
            .send(ConnectionCommand::StartSearch { token, query })
            .is_err()
        {
            self.search.fail(
                &client_id,
                "The Soulseek connection changed before the search could start.",
            );
            return Err(ConnectionServiceError::SearchUnavailable);
        }
        self.diagnostics.record(
            "info",
            "search_started",
            "A live Soulseek search was started.",
        );
        Ok(snapshot)
    }

    pub fn stop_search(&self, client_id: &str) -> Option<SearchSnapshot> {
        let snapshot = self.search.stop(client_id);
        if snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.state == SearchState::Stopped)
        {
            self.diagnostics
                .record("info", "search_stopped", "The live search was stopped.");
        }
        snapshot
    }

    pub fn close_search(&self, client_id: &str) -> bool {
        self.search.close(client_id)
    }

    pub fn current_snapshot(&self) -> ConnectionSnapshot {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    async fn announce_no_distributed_parent<W>(
        &self,
        writer: &mut W,
        own_username: &str,
    ) -> Result<(), ConnectionFailure>
    where
        W: AsyncWrite + Unpin,
    {
        for frame in [
            have_no_parent_frame(true),
            branch_root_frame(own_username),
            branch_level_frame(0),
            accept_children_frame(false),
        ] {
            write_raw_frame(writer, &frame).await.map_err(|error| {
                ConnectionFailure::retryable(
                    format!("The global-search network could not be initialized: {error}"),
                    "distributed_setup_failed",
                )
            })?;
        }
        Ok(())
    }

    async fn announce_distributed_parent<W>(
        &self,
        writer: &mut W,
        branch_root: &str,
        branch_level: u32,
    ) -> Result<(), ConnectionFailure>
    where
        W: AsyncWrite + Unpin,
    {
        for frame in [
            have_no_parent_frame(false),
            branch_root_frame(branch_root),
            branch_level_frame(branch_level),
            accept_children_frame(false),
        ] {
            write_raw_frame(writer, &frame).await.map_err(|error| {
                ConnectionFailure::retryable(
                    format!("The global-search parent could not be announced: {error}"),
                    "distributed_parent_failed",
                )
            })?;
        }
        Ok(())
    }

    async fn answer_search_request<W>(
        &self,
        writer: &mut W,
        own_username: &str,
        request: DistributedSearch,
        origin: SearchResponseOrigin,
    ) -> Result<(), ConnectionFailure>
    where
        W: AsyncWrite + Unpin,
    {
        if request.username.is_empty()
            || request.username.len() > MAX_SEARCH_USERNAME_BYTES
            || request.query.is_empty()
            || request.query.len() > MAX_SEARCH_QUERY_BYTES
            || (origin == SearchResponseOrigin::Distributed
                && request.username.eq_ignore_ascii_case(own_username))
        {
            return Ok(());
        }
        if self.people.is_ignored(&request.username) {
            return Ok(());
        }
        if origin == SearchResponseOrigin::Distributed
            && self
                .distributed
                .admit_request(&request.username, request.token)
                != RequestAdmission::Allowed
        {
            return Ok(());
        }

        let connection_token = self.take_connection_token();
        let Some(ticket) = self.local_shares.queue_search_response(
            connection_token,
            &request.username,
            request.token,
            &request.query,
            origin,
        ) else {
            return Ok(());
        };
        if origin == SearchResponseOrigin::Distributed {
            self.distributed.record_match();
        }

        write_raw_frame(
            writer,
            &connect_to_peer_frame(connection_token, &ticket.username, "P"),
        )
        .await
        .map_err(|error| {
            ConnectionFailure::retryable(
                format!("The search-result peer request could not be sent: {error}"),
                "search_response_failed",
            )
        })?;
        write_raw_frame(writer, &get_peer_address_frame(&ticket.username))
            .await
            .map_err(|error| {
                ConnectionFailure::retryable(
                    format!("The search-result address request could not be sent: {error}"),
                    "search_response_failed",
                )
            })?;
        Ok(())
    }

    async fn run_connection_loop(
        &self,
        profile: ConnectionProfile,
        password: Zeroizing<String>,
        generation: u64,
    ) {
        let mut attempt = 0;
        loop {
            if self.generation.load(Ordering::SeqCst) != generation {
                return;
            }

            attempt += 1;
            let state = if attempt == 1 {
                ConnectionState::Connecting
            } else {
                ConnectionState::Reconnecting
            };
            self.publish(ConnectionSnapshot {
                state,
                username: Some(profile.username.clone()),
                server: Some(server_label(&profile)),
                message: if attempt == 1 {
                    "Connecting to the Soulseek network…".to_owned()
                } else {
                    format!("Reconnect attempt {attempt}…")
                },
                attempt,
                connected_at_ms: None,
                retry_in_seconds: None,
                updated_at_ms: timestamp_ms(),
            });
            self.diagnostics.record(
                "info",
                if attempt == 1 {
                    "connect_started"
                } else {
                    "reconnect_started"
                },
                &format!("Connecting to {}.", server_label(&profile)),
            );

            let outcome = self
                .connect_once(&profile, password.as_str(), attempt)
                .await;
            self.clear_command_sender();
            let _ = self.messages.fail_queued(
                "The Soulseek connection was interrupted before this message was sent.",
            );
            self.rooms.disconnected();
            self.transfers.connection_lost();
            self.uploads.connection_lost();
            self.folders.connection_lost();
            self.shares.connection_lost();
            self.local_shares.connection_lost();
            self.people.connection_lost();
            self.distributed.offline();
            self.wanted.connection_lost();
            self.radar.connection_lost();
            self.search
                .fail_all("Search stopped because the Soulseek connection was interrupted.");

            match outcome {
                Ok(()) => {
                    let failure = ConnectionFailure::retryable(
                        "The Soulseek server closed the connection.",
                        "socket_closed",
                    );
                    if !self.wait_to_retry(&profile, attempt, &failure).await {
                        return;
                    }
                }
                Err(failure) if failure.retryable => {
                    if !self.wait_to_retry(&profile, attempt, &failure).await {
                        return;
                    }
                }
                Err(failure) => {
                    self.diagnostics
                        .record("error", failure.event, &failure.message);
                    self.publish(ConnectionSnapshot {
                        state: ConnectionState::Error,
                        username: Some(profile.username.clone()),
                        server: Some(server_label(&profile)),
                        message: failure.message,
                        attempt,
                        connected_at_ms: None,
                        retry_in_seconds: None,
                        updated_at_ms: timestamp_ms(),
                    });
                    return;
                }
            }
        }
    }

    async fn connect_once(
        &self,
        profile: &ConnectionProfile,
        password: &str,
        attempt: u32,
    ) -> Result<(), ConnectionFailure> {
        let address = (profile.server_host.as_str(), profile.server_port);
        let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
            .await
            .map_err(|_| {
                ConnectionFailure::retryable(
                    "The Soulseek server did not respond in time.",
                    "connect_timeout",
                )
            })?
            .map_err(|error| {
                ConnectionFailure::retryable(
                    format!("Could not reach the Soulseek server: {error}"),
                    "connect_failed",
                )
            })?;
        let _ = stream.set_nodelay(true);

        self.publish(ConnectionSnapshot {
            state: ConnectionState::Authenticating,
            username: Some(profile.username.clone()),
            server: Some(server_label(profile)),
            message: "Signing in to Soulseek…".to_owned(),
            attempt,
            connected_at_ms: None,
            retry_in_seconds: None,
            updated_at_ms: timestamp_ms(),
        });

        let login = Zeroizing::new(login_frame(&profile.username, password));
        write_raw_frame(&mut stream, login.as_slice())
            .await
            .map_err(|error| {
                ConnectionFailure::retryable(
                    format!("Could not send the Soulseek login: {error}"),
                    "login_send_failed",
                )
            })?;
        let response = timeout(LOGIN_TIMEOUT, read_frame(&mut stream))
            .await
            .map_err(|_| {
                ConnectionFailure::retryable(
                    "The Soulseek server did not answer the login request.",
                    "login_timeout",
                )
            })?
            .map_err(|error| {
                ConnectionFailure::retryable(
                    format!("Could not read the Soulseek login response: {error}"),
                    "login_read_failed",
                )
            })?;

        match parse_login_response(&response).map_err(|error| {
            ConnectionFailure::fatal(
                format!("The Soulseek server sent an unexpected login response: {error}"),
                "login_protocol_error",
            )
        })? {
            LoginResponse::Accepted { .. } => {}
            LoginResponse::Rejected { reason, detail } => {
                return Err(rejection_failure(&reason, detail.as_deref()));
            }
        }

        let listener = TcpListener::bind(("0.0.0.0", 0)).await.map_err(|error| {
            ConnectionFailure::retryable(
                format!("Flipside could not open a Soulseek peer listening port: {error}"),
                "listen_failed",
            )
        })?;
        let listen_port = listener
            .local_addr()
            .map_err(|error| {
                ConnectionFailure::retryable(
                    format!(
                        "Flipside could not inspect its Soulseek peer listening port: {error}"
                    ),
                    "listen_failed",
                )
            })?
            .port();
        write_raw_frame(&mut stream, &set_wait_port_frame(listen_port))
            .await
            .map_err(|error| {
                ConnectionFailure::retryable(
                    format!("The Soulseek peer listener could not be announced: {error}"),
                    "session_setup_failed",
                )
            })?;
        write_raw_frame(&mut stream, &set_online_frame())
            .await
            .map_err(|error| {
                ConnectionFailure::retryable(
                    format!("The Soulseek session could not be initialized: {error}"),
                    "session_setup_failed",
                )
            })?;
        let (shared_directories, shared_files) = self.local_shares.counts();
        write_raw_frame(
            &mut stream,
            &shared_counts_frame(shared_directories, shared_files),
        )
        .await
        .map_err(|error| {
            ConnectionFailure::retryable(
                format!("The Soulseek session could not be initialized: {error}"),
                "session_setup_failed",
            )
        })?;

        // `read_frame` uses `read_exact`, which is not cancellation-safe. Keep it
        // out of the busy select loop so timer/listener/command branches cannot
        // discard a partially consumed server frame and desynchronize the socket.
        let (server_reader, mut server_writer) = stream.into_split();
        let (server_frame_sender, mut server_frame_receiver) =
            mpsc::channel(SERVER_FRAME_QUEUE_SIZE);
        let _server_reader_task = AbortOnDrop(tauri::async_runtime::spawn(forward_server_frames(
            server_reader,
            server_frame_sender,
        )));

        let (command_sender, mut command_receiver) = mpsc::unbounded_channel();
        *self
            .command_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(command_sender.clone());

        let (distributed_event_sender, mut distributed_event_receiver) =
            mpsc::channel(DISTRIBUTED_EVENT_QUEUE_SIZE);
        let mut distributed_coordinator = DistributedCoordinator::new();
        self.announce_no_distributed_parent(&mut server_writer, &profile.username)
            .await?;
        self.distributed.begin_discovery();
        self.diagnostics.record(
            "info",
            "distributed_discovery",
            "Looking for a global-search relay.",
        );

        let connected_at_ms = timestamp_ms();
        self.diagnostics.record(
            "info",
            "connected",
            "Authenticated with the Soulseek server.",
        );
        self.publish(ConnectionSnapshot {
            state: ConnectionState::Online,
            username: Some(profile.username.clone()),
            server: Some(server_label(profile)),
            message: "Network online".to_owned(),
            attempt,
            connected_at_ms: Some(connected_at_ms),
            retry_in_seconds: None,
            updated_at_ms: connected_at_ms,
        });
        self.rooms.connected();
        let _ = command_sender.send(ConnectionCommand::RefreshRooms);
        for room in self.rooms.desired_rooms() {
            let _ = command_sender.send(ConnectionCommand::JoinRoom { room });
        }
        let _ = command_sender.send(ConnectionCommand::ScheduleDownloads);
        let _ = command_sender.send(ConnectionCommand::ScheduleUploads);
        for username in self.people.saved_users_to_watch() {
            let _ = command_sender.send(ConnectionCommand::WatchPerson { username });
        }

        let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
        let mut search_tick = tokio::time::interval(Duration::from_millis(250));
        let mut wanted_tick = tokio::time::interval(Duration::from_secs(5));
        let mut radar_tick = tokio::time::interval(Duration::from_millis(500));
        let mut download_retry_tick = tokio::time::interval(Duration::from_secs(1));
        let peer_limit = Arc::new(Semaphore::new(MAX_CONCURRENT_PEERS));
        keepalive.tick().await;
        search_tick.tick().await;
        wanted_tick.tick().await;
        radar_tick.tick().await;
        download_retry_tick.tick().await;
        loop {
            tokio::select! {
                _ = download_retry_tick.tick() => {
                    let _ = command_sender.send(ConnectionCommand::ScheduleDownloads);
                }
                _ = keepalive.tick() => {
                    write_raw_frame(&mut server_writer, &server_ping_frame()).await.map_err(|error| {
                        ConnectionFailure::retryable(
                            format!("The Soulseek keepalive failed: {error}"),
                            "keepalive_failed",
                        )
                    })?;
                }
                frame = server_frame_receiver.recv() => {
                    let frame = match frame {
                        Some(Ok(frame)) => frame,
                        Some(Err(error)) => {
                            return Err(ConnectionFailure::retryable(
                                format!("The Soulseek connection was interrupted: {error}"),
                                "socket_interrupted",
                            ));
                        }
                        None => {
                            return Err(ConnectionFailure::retryable(
                                "The Soulseek server reader stopped unexpectedly.",
                                "socket_interrupted",
                            ));
                        }
                    };
                    if frame.code == RELOGGED_CODE {
                        return Err(ConnectionFailure::fatal(
                            "This account signed in from another Soulseek client.",
                            "relogged",
                        ));
                    }
                    if frame.code == ROOM_LIST_CODE {
                        if let Ok(rooms) = parse_room_list(&frame) {
                            self.rooms.update_list(rooms);
                        }
                    } else if frame.code == JOIN_ROOM_CODE {
                        if let Ok(room) = parse_join_room(&frame) {
                            let _ = self.rooms.joined(room);
                        }
                    } else if frame.code == LEAVE_ROOM_CODE {
                        if let Ok(room) = parse_leave_room(&frame) {
                            let _ = self.rooms.left(&room);
                        }
                    } else if frame.code == USER_JOINED_ROOM_CODE {
                        if let Ok((room, member)) = parse_user_joined_room(&frame) {
                            let _ = self.rooms.user_joined(&room, member);
                        }
                    } else if frame.code == USER_LEFT_ROOM_CODE {
                        if let Ok((room, username)) = parse_user_left_room(&frame) {
                            let _ = self.rooms.user_left(&room, &username);
                        }
                    } else if frame.code == SAY_CHATROOM_CODE {
                        if let Ok(message) = parse_room_chat_message(&frame) {
                            if !self.people.is_ignored(&message.username) {
                                let _ = self.rooms.record_message(
                                    &message.room,
                                    &message.username,
                                    &message.message,
                                    &profile.username,
                                );
                            }
                        }
                    } else if frame.code == MESSAGE_USER_CODE {
                        if let Ok(message) = parse_private_message(&frame) {
                            if !self.people.is_ignored(&message.username) {
                                self.people.remember(&message.username).map_err(|error| {
                                    ConnectionFailure::retryable(
                                        format!("The incoming-message sender could not be saved: {error}"),
                                        "message_store_failed",
                                    )
                                })?;
                                self.messages.record_incoming(
                                    message.id,
                                    message.timestamp_seconds,
                                    &message.username,
                                    &message.message,
                                ).map_err(|error| ConnectionFailure::retryable(
                                    format!("The incoming private message could not be saved: {error}"),
                                    "message_store_failed",
                                ))?;
                            }
                            write_raw_frame(&mut server_writer, &message_acked_frame(message.id))
                                .await
                                .map_err(|error| ConnectionFailure::retryable(
                                    format!("The private-message acknowledgement failed: {error}"),
                                    "message_ack_failed",
                                ))?;
                        }
                    } else if frame.code == FILE_SEARCH_CODE {
                        if let Ok((username, search_token, query)) = parse_server_search_request(&frame) {
                            self.answer_search_request(
                                &mut server_writer,
                                &profile.username,
                                DistributedSearch { username, token: search_token, query },
                                SearchResponseOrigin::Server,
                            ).await?;
                        }
                    } else if frame.code == WATCH_USER_CODE {
                        if let Ok(watched) = parse_watch_user(&frame) {
                            self.people.update_watch(watched);
                        }
                    } else if frame.code == USER_STATUS_CODE {
                        if let Ok((username, status, privileged)) = parse_user_status(&frame) {
                            self.rooms.update_status(&username, status);
                            self.people.update_status(&username, status, privileged);
                        }
                    } else if frame.code == USER_STATS_CODE {
                        if let Ok(stats) = parse_user_stats(&frame) {
                            self.rooms.update_stats(&stats);
                            self.people.update_stats(stats);
                        }
                    } else if frame.code == USER_INTERESTS_CODE {
                        if let Ok(interests) = parse_user_interests(&frame) {
                            self.people.update_interests(interests);
                        }
                    } else if frame.code == POSSIBLE_PARENTS_CODE {
                        if let Ok(parents) = parse_possible_parents(&frame) {
                            distributed_coordinator.start_candidates(
                                parents,
                                &profile.username,
                                &distributed_event_sender,
                            );
                        }
                    } else if frame.code == EMBEDDED_MESSAGE_CODE {
                        if let Ok(request) = parse_embedded_distributed_search(&frame) {
                            if distributed_coordinator.become_branch_root() {
                                self.distributed.branch_root();
                                self.diagnostics.record(
                                    "info",
                                    "distributed_branch_root",
                                    "Global search connected in branch-root mode.",
                                );
                            }
                            self.answer_search_request(
                                &mut server_writer,
                                &profile.username,
                                request,
                                SearchResponseOrigin::Distributed,
                            ).await?;
                        }
                    } else if frame.code == RESET_DISTRIBUTED_CODE {
                        distributed_coordinator.reset();
                        self.announce_no_distributed_parent(
                            &mut server_writer,
                            &profile.username,
                        ).await?;
                        self.distributed.rediscovering();
                        self.diagnostics.record(
                            "warn",
                            "distributed_reset",
                            "The server reset global search; finding a new relay.",
                        );
                    } else if frame.code == CONNECT_TO_PEER_CODE {
                        if let Ok(request) = parse_connect_to_peer(&frame) {
                            spawn_indirect_peer(
                                request,
                                self.peer_services(&profile.username, &command_sender),
                                peer_limit.clone(),
                            );
                        }
                    } else if frame.code == GET_PEER_ADDRESS_CODE {
                        if let Ok(address) = parse_peer_address(&frame) {
                            if let Some(ticket) = self.uploads.requesting_file_for_username(&address.username) {
                                spawn_outbound_upload_file_peer(
                                    address,
                                    ticket,
                                    profile.username.clone(),
                                    self.peer_services(&profile.username, &command_sender),
                                    peer_limit.clone(),
                                );
                            } else if let Some(ticket) = self.uploads.requesting_control_for_username(&address.username) {
                                spawn_outbound_upload_control_peer(
                                    address,
                                    ticket,
                                    profile.username.clone(),
                                    self.peer_services(&profile.username, &command_sender),
                                    peer_limit.clone(),
                                );
                            } else if let Some(ticket) = self.local_shares.requesting_search_for_username(&address.username) {
                                spawn_outbound_search_response_peer(
                                    address,
                                    ticket,
                                    profile.username.clone(),
                                    self.peer_services(&profile.username, &command_sender),
                                    peer_limit.clone(),
                                );
                            } else if let Some(ticket) = self.people.requesting_for_username(&address.username) {
                                spawn_outbound_profile_peer(
                                    address,
                                    ticket,
                                    profile.username.clone(),
                                    self.peer_services(&profile.username, &command_sender),
                                    peer_limit.clone(),
                                );
                            } else if let Some(ticket) = self.folders.requesting_for_username(&address.username) {
                                spawn_outbound_folder_peer(
                                    address,
                                    ticket,
                                    profile.username.clone(),
                                    self.peer_services(&profile.username, &command_sender),
                                    peer_limit.clone(),
                                );
                            } else if let Some(ticket) = self.shares.requesting_for_username(&address.username) {
                                spawn_outbound_shares_peer(
                                    address,
                                    ticket,
                                    profile.username.clone(),
                                    self.peer_services(&profile.username, &command_sender),
                                    peer_limit.clone(),
                                );
                            } else if let Some(ticket) = self
                                .transfers
                                .requesting_for_username(&address.username)
                            {
                                spawn_outbound_download_peer(
                                    address,
                                    ticket,
                                    profile.username.clone(),
                                    self.peer_services(&profile.username, &command_sender),
                                    peer_limit.clone(),
                                );
                            }
                        }
                    } else if frame.code == CANT_CONNECT_TO_PEER_CODE {
                        if let Ok(token) = parse_cant_connect_token(&frame) {
                            if self.transfers.retry_connection(
                                token,
                                "The source could not establish a peer connection.".to_owned(),
                            ) {
                                let _ = command_sender.send(ConnectionCommand::ScheduleDownloads);
                            }
                            self.folders.fail_connection(
                                token,
                                "The source could not establish a peer connection.".to_owned(),
                            );
                            self.shares.fail_connection(
                                token,
                                "The source could not establish a peer connection.".to_owned(),
                            );
                            self.local_shares.fail_search(token);
                            self.people.fail_profile(
                                token,
                                "The user could not establish a profile connection.".to_owned(),
                            );
                            if self.uploads.fail_connection(
                                token,
                                "The downloader could not establish a peer connection.".to_owned(),
                            ) {
                                let _ = command_sender.send(ConnectionCommand::ScheduleUploads);
                            }
                        }
                    }
                }
                Some(event) = distributed_event_receiver.recv() => {
                    match event {
                        DistributedPeerEvent::Frame { id, frame } => match frame.code {
                            DISTRIBUTED_BRANCH_LEVEL_CODE => {
                                if let Ok(level) = parse_distributed_branch_level(&frame) {
                                    if let Some(own_level) =
                                        distributed_coordinator.update_branch_level(id, level)
                                    {
                                        write_raw_frame(
                                            &mut server_writer,
                                            &branch_level_frame(own_level),
                                        ).await.map_err(|error| ConnectionFailure::retryable(
                                            format!("The global-search branch update failed: {error}"),
                                            "distributed_parent_failed",
                                        ))?;
                                        self.distributed.connected(own_level);
                                    }
                                }
                            }
                            DISTRIBUTED_BRANCH_ROOT_CODE => {
                                if let Ok(root) = parse_distributed_branch_root(&frame) {
                                    if let Some(own_root) =
                                        distributed_coordinator.update_branch_root(id, root)
                                    {
                                        write_raw_frame(
                                            &mut server_writer,
                                            &branch_root_frame(&own_root),
                                        ).await.map_err(|error| ConnectionFailure::retryable(
                                            format!("The global-search branch update failed: {error}"),
                                            "distributed_parent_failed",
                                        ))?;
                                    }
                                }
                            }
                            DISTRIBUTED_SEARCH_CODE => {
                                if let Ok(request) = parse_distributed_search(&frame) {
                                    if !distributed_coordinator.accepts_search_from(id) {
                                        if let Some((branch_level, branch_root)) =
                                            distributed_coordinator.adopt(id)
                                        {
                                            self.announce_distributed_parent(
                                                &mut server_writer,
                                                &branch_root,
                                                branch_level,
                                            ).await?;
                                            self.distributed.connected(branch_level);
                                            self.diagnostics.record(
                                                "info",
                                                "distributed_connected",
                                                "Connected to the global-search network.",
                                            );
                                        }
                                    }
                                    if distributed_coordinator.accepts_search_from(id) {
                                        self.answer_search_request(
                                            &mut server_writer,
                                            &profile.username,
                                            request,
                                            SearchResponseOrigin::Distributed,
                                        ).await?;
                                    }
                                }
                            }
                            _ => {}
                        },
                        DistributedPeerEvent::Closed { id } => {
                            if distributed_coordinator.close(id) {
                                self.announce_no_distributed_parent(
                                    &mut server_writer,
                                    &profile.username,
                                ).await?;
                                self.distributed.rediscovering();
                                self.diagnostics.record(
                                    "warn",
                                    "distributed_parent_lost",
                                    "The global-search relay disconnected; finding another.",
                                );
                            }
                        }
                    }
                }
                accepted = listener.accept() => {
                    let (peer_stream, _) = accepted.map_err(|error| {
                        ConnectionFailure::retryable(
                            format!("The Soulseek peer listener stopped: {error}"),
                            "listen_failed",
                        )
                    })?;
                    spawn_direct_peer(
                        peer_stream,
                        self.peer_services(&profile.username, &command_sender),
                        peer_limit.clone(),
                    );
                }
                command = command_receiver.recv() => {
                    match command {
                        Some(ConnectionCommand::StartSearch { token, query }) => {
                            write_raw_frame(&mut server_writer, &file_search_frame(token, &query))
                                .await
                                .map_err(|error| {
                                    ConnectionFailure::retryable(
                                        format!("The live search could not be sent: {error}"),
                                        "search_send_failed",
                                    )
                                })?;
                        }
                        Some(ConnectionCommand::RefreshRooms) => {
                            write_raw_frame(&mut server_writer, &room_list_frame())
                                .await
                                .map_err(|error| ConnectionFailure::retryable(
                                    format!("The public room directory could not be refreshed: {error}"),
                                    "room_list_failed",
                                ))?;
                        }
                        Some(ConnectionCommand::JoinRoom { room }) => {
                            write_raw_frame(&mut server_writer, &join_room_frame(&room))
                                .await
                                .map_err(|error| ConnectionFailure::retryable(
                                    format!("The public room could not be joined: {error}"),
                                    "room_join_failed",
                                ))?;
                        }
                        Some(ConnectionCommand::WatchPerson { username }) => {
                            if self.people.mark_watched(&username) {
                                write_raw_frame(&mut server_writer, &watch_user_frame(&username))
                                    .await
                                    .map_err(|error| ConnectionFailure::retryable(
                                        format!("The user presence request could not be sent: {error}"),
                                        "people_watch_failed",
                                    ))?;
                            }
                        }
                        Some(ConnectionCommand::PeerConnectionFailed { token, username }) => {
                            write_raw_frame(
                                &mut server_writer,
                                &cant_connect_to_peer_frame(token, &username),
                            )
                            .await
                            .map_err(|error| {
                                ConnectionFailure::retryable(
                                    format!("The peer connection response could not be sent: {error}"),
                                    "peer_response_failed",
                                )
                                })?;
                        }
                        Some(ConnectionCommand::ScheduleDownloads) => {
                            loop {
                                let token = self.take_connection_token();
                                let Some(ticket) = self.transfers.activate_next(token) else {
                                    break;
                                };
                                write_raw_frame(
                                    &mut server_writer,
                                    &connect_to_peer_frame(
                                        ticket.connection_token,
                                        &ticket.username,
                                        "P",
                                    ),
                                )
                                .await
                                .map_err(|error| {
                                    ConnectionFailure::retryable(
                                        format!("The download peer request could not be sent: {error}"),
                                        "download_request_failed",
                                    )
                                })?;
                                write_raw_frame(
                                    &mut server_writer,
                                    &get_peer_address_frame(&ticket.username),
                                )
                                .await
                                .map_err(|error| {
                                    ConnectionFailure::retryable(
                                        format!("The source address request could not be sent: {error}"),
                                        "download_address_failed",
                                    )
                                })?;
                                spawn_transfer_request_timeout(
                                    ticket,
                                    self.transfers.clone(),
                                    command_sender.clone(),
                                );
                            }
                        }
                        Some(ConnectionCommand::ScheduleUploads) => {
                            loop {
                                let token = self.take_connection_token();
                                let Some(ticket) = self.uploads.activate_next(
                                    token,
                                    self.local_shares.upload_slots(),
                                ) else {
                                    break;
                                };
                                write_raw_frame(
                                    &mut server_writer,
                                    &connect_to_peer_frame(
                                        ticket.connection_token,
                                        &ticket.username,
                                        "P",
                                    ),
                                ).await.map_err(|error| ConnectionFailure::retryable(
                                    format!("The upload peer request could not be sent: {error}"),
                                    "upload_request_failed",
                                ))?;
                                write_raw_frame(
                                    &mut server_writer,
                                    &get_peer_address_frame(&ticket.username),
                                ).await.map_err(|error| ConnectionFailure::retryable(
                                    format!("The downloader address request could not be sent: {error}"),
                                    "upload_address_failed",
                                ))?;
                                spawn_upload_request_timeout(
                                    ticket,
                                    self.uploads.clone(),
                                    command_sender.clone(),
                                );
                            }
                        }
                        Some(ConnectionCommand::OpenUploadFile { id }) => {
                            let connection_token = self.take_connection_token();
                            if let Some(ticket) = self.uploads.prepare_file_connection(
                                &id,
                                connection_token,
                            ) {
                                write_raw_frame(
                                    &mut server_writer,
                                    &connect_to_peer_frame(
                                        ticket.connection_token,
                                        &ticket.username,
                                        "F",
                                    ),
                                ).await.map_err(|error| ConnectionFailure::retryable(
                                    format!("The upload file connection could not be sent: {error}"),
                                    "upload_file_request_failed",
                                ))?;
                                write_raw_frame(
                                    &mut server_writer,
                                    &get_peer_address_frame(&ticket.username),
                                ).await.map_err(|error| ConnectionFailure::retryable(
                                    format!("The downloader file address request could not be sent: {error}"),
                                    "upload_file_address_failed",
                                ))?;
                            }
                        }
                        Some(ConnectionCommand::RefreshSharedCounts) => {
                            let (directories, files) = self.local_shares.counts();
                            write_raw_frame(
                                &mut server_writer,
                                &shared_counts_frame(directories, files),
                            ).await.map_err(|error| ConnectionFailure::retryable(
                                format!("Updated shared counts could not be sent: {error}"),
                                "shared_counts_failed",
                            ))?;
                        }
                        None => {
                            return Err(ConnectionFailure::retryable(
                                "The Soulseek command channel closed.",
                                "command_channel_closed",
                            ));
                        }
                    }
                }
                _ = search_tick.tick() => {
                    self.search.expire_if_due();
                    self.wanted.expire_if_due();
                    self.radar.expire_if_due();
                }
                _ = radar_tick.tick() => {
                    let mut token = self.next_search_token.fetch_add(1, Ordering::SeqCst);
                    if token == 0 {
                        token = self.next_search_token.fetch_add(1, Ordering::SeqCst);
                    }
                    if let Some((token, query)) = self.radar.start_next(token) {
                        if let Err(error) = write_raw_frame(&mut server_writer, &file_search_frame(token, &query)).await {
                            self.radar.fail_active(format!("The Shelf Radar search could not be sent: {error}"));
                        }
                    }
                }
                _ = wanted_tick.tick() => {
                    let mut token = self.next_search_token.fetch_add(1, Ordering::SeqCst);
                    if token == 0 {
                        token = self.next_search_token.fetch_add(1, Ordering::SeqCst);
                    }
                    if let Some(query) = self.wanted.start_due(token) {
                        write_raw_frame(&mut server_writer, &file_search_frame(token, &query))
                            .await
                            .map_err(|error| ConnectionFailure::retryable(
                                format!("The Wanted check could not be sent: {error}"),
                                "wanted_search_send_failed",
                            ))?;
                    }
                }
            }
        }
    }

    async fn wait_to_retry(
        &self,
        profile: &ConnectionProfile,
        attempt: u32,
        failure: &ConnectionFailure,
    ) -> bool {
        let delay = retry_delay(attempt);
        self.diagnostics
            .record("warn", failure.event, &failure.message);
        self.publish(ConnectionSnapshot {
            state: ConnectionState::Reconnecting,
            username: Some(profile.username.clone()),
            server: Some(server_label(profile)),
            message: failure.message.clone(),
            attempt,
            connected_at_ms: None,
            retry_in_seconds: Some(delay.as_secs()),
            updated_at_ms: timestamp_ms(),
        });
        tokio::time::sleep(delay).await;
        true
    }

    fn publish(&self, snapshot: ConnectionSnapshot) {
        *self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot.clone();
        let _ = crate::events::SoulseekConnectionChanged(snapshot).emit(&self.app);
    }

    fn stop_active_task(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.clear_command_sender();
        self.search.stop_all();
        self.transfers.connection_lost();
        self.folders.connection_lost();
        self.shares.connection_lost();
        self.local_shares.connection_lost();
        self.people.connection_lost();
        self.distributed.offline();
        self.wanted.connection_lost();
        self.radar.connection_lost();
        if let Some(active) = self
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            active.handle.abort();
        }
    }

    fn clear_command_sender(&self) {
        self.command_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    fn clear_task(&self, generation: u64) {
        let mut task = self
            .task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if task
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            *task = None;
        }
    }
}

async fn forward_server_frames<R>(mut reader: R, sender: mpsc::Sender<Result<Frame, ProtocolError>>)
where
    R: AsyncRead + Unpin,
{
    loop {
        let frame = read_frame(&mut reader).await;
        let terminal = frame.is_err();
        if sender.send(frame).await.is_err() || terminal {
            return;
        }
    }
}

fn spawn_distributed_parent_candidate(
    id: u64,
    parent: ParentCandidate,
    own_username: String,
    event_sender: mpsc::Sender<DistributedPeerEvent>,
) -> AbortOnDrop {
    AbortOnDrop(tauri::async_runtime::spawn(async move {
        let _ = run_distributed_parent_candidate(id, &parent, &own_username, event_sender.clone())
            .await;
        let _ = event_sender.send(DistributedPeerEvent::Closed { id }).await;
    }))
}

async fn run_distributed_parent_candidate(
    id: u64,
    parent: &ParentCandidate,
    own_username: &str,
    event_sender: mpsc::Sender<DistributedPeerEvent>,
) -> Result<(), ProtocolError> {
    let mut stream = timeout(
        PEER_CONNECT_TIMEOUT,
        TcpStream::connect((parent.address, parent.port)),
    )
    .await
    .map_err(|_| peer_timeout_error())??;
    let _ = stream.set_nodelay(true);
    write_raw_frame(&mut stream, &peer_init_frame(own_username, "D")).await?;

    loop {
        let frame = timeout(
            DISTRIBUTED_PARENT_IDLE_TIMEOUT,
            read_distributed_frame(&mut stream),
        )
        .await
        .map_err(|_| peer_timeout_error())??;
        if event_sender
            .send(DistributedPeerEvent::Frame { id, frame })
            .await
            .is_err()
        {
            return Ok(());
        }
    }
}

fn spawn_direct_peer(stream: TcpStream, services: PeerServices, limit: Arc<Semaphore>) {
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let _ = handle_direct_peer(stream, services).await;
    });
}

fn spawn_indirect_peer(request: ConnectToPeer, services: PeerServices, limit: Arc<Semaphore>) {
    if !matches!(request.connection_type.as_str(), "P" | "F")
        || request.port == 0
        || request.port > u16::MAX.into()
    {
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        if handle_indirect_peer(&request, services.clone())
            .await
            .is_err()
        {
            services.local_shares.fail_search(request.token);
            if services.uploads.fail_connection(
                request.token,
                "The downloader peer connection failed.".to_owned(),
            ) {
                let _ = services
                    .command_sender
                    .send(ConnectionCommand::ScheduleUploads);
            }
            services.folders.fail_connection(
                request.token,
                "The source peer connection failed before the folder could be read.".to_owned(),
            );
            services.shares.fail_connection(
                request.token,
                "The source peer connection failed before the share list could be read.".to_owned(),
            );
            let _ = services
                .command_sender
                .send(ConnectionCommand::PeerConnectionFailed {
                    token: request.token,
                    username: request.username,
                });
        }
    });
}

async fn handle_direct_peer(
    mut stream: TcpStream,
    services: PeerServices,
) -> Result<(), super::protocol::ProtocolError> {
    let _ = stream.set_nodelay(true);
    let init = timeout(PEER_MESSAGE_TIMEOUT, read_peer_init(&mut stream))
        .await
        .map_err(|_| peer_timeout_error())??;
    match init {
        PeerInit::PierceFirewall { token } => {
            if let Some(ticket) = services.uploads.claim_file(token) {
                stream.write_u32_le(ticket.transfer_token).await?;
                stream.flush().await?;
                spawn_file_upload(stream, ticket, services.uploads, services.command_sender);
            } else if let Some(ticket) = services.uploads.claim_control(token) {
                negotiate_upload_on_peer(&mut stream, ticket, services).await?;
            } else if let Some(ticket) = services.local_shares.claim_search(token) {
                send_search_response_on_peer(&mut stream, ticket, &services).await?;
            } else if let Some(ticket) = services.people.claim_profile(token) {
                request_profile_on_peer(&mut stream, ticket, services).await?;
            } else if let Some(ticket) = services.transfers.claim_peer(token) {
                if let Err(error) =
                    queue_download_on_peer(&mut stream, ticket.clone(), services.clone()).await
                {
                    if services.transfers.retry_id(
                        &ticket.id,
                        format!("The source connection ended before the file was queued: {error}"),
                    ) {
                        let _ = services
                            .command_sender
                            .send(ConnectionCommand::ScheduleDownloads);
                    }
                }
            } else if let Some(ticket) = services.folders.claim_peer(token) {
                if let Err(error) =
                    browse_folder_on_peer(&mut stream, ticket.clone(), services.clone()).await
                {
                    services.folders.fail_connection(
                        ticket.connection_token,
                        format!("The folder connection ended before the source answered: {error}"),
                    );
                }
            } else if let Some(ticket) = services.shares.claim_peer(token) {
                if let Err(error) =
                    browse_shares_on_peer(&mut stream, ticket.clone(), services.clone()).await
                {
                    services.shares.fail_connection(
                        ticket.connection_token,
                        format!(
                            "The share-list connection ended before the source answered: {error}"
                        ),
                    );
                }
            }
            Ok(())
        }
        PeerInit::Peer {
            username,
            connection_type,
            ..
        } if connection_type == "P" => {
            handle_peer_messages(
                &mut stream,
                &username,
                services,
                PeerMessagePurpose::General,
            )
            .await
        }
        PeerInit::Peer {
            connection_type, ..
        } if connection_type == "F" => {
            let transfer_token = timeout(PEER_MESSAGE_TIMEOUT, stream.read_u32_le())
                .await
                .map_err(|_| peer_timeout_error())??;
            spawn_file_download(
                stream,
                transfer_token,
                services.transfers,
                services.command_sender,
            );
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn handle_indirect_peer(
    request: &ConnectToPeer,
    services: PeerServices,
) -> Result<(), super::protocol::ProtocolError> {
    let mut stream = timeout(
        PEER_CONNECT_TIMEOUT,
        TcpStream::connect((request.address, request.port as u16)),
    )
    .await
    .map_err(|_| peer_timeout_error())??;
    let _ = stream.set_nodelay(true);
    write_raw_frame(&mut stream, &pierce_firewall_frame(request.token)).await?;
    if request.connection_type == "F" {
        if let Some(ticket) = services.uploads.claim_file(request.token) {
            stream.write_u32_le(ticket.transfer_token).await?;
            stream.flush().await?;
            spawn_file_upload(stream, ticket, services.uploads, services.command_sender);
            return Ok(());
        }
        let transfer_token = timeout(PEER_MESSAGE_TIMEOUT, stream.read_u32_le())
            .await
            .map_err(|_| peer_timeout_error())??;
        spawn_file_download(
            stream,
            transfer_token,
            services.transfers,
            services.command_sender,
        );
        return Ok(());
    }
    if let Some(ticket) = services.uploads.claim_control(request.token) {
        return negotiate_upload_on_peer(&mut stream, ticket, services).await;
    }
    if let Some(ticket) = services.local_shares.claim_search(request.token) {
        return send_search_response_on_peer(&mut stream, ticket, &services).await;
    }
    if let Some(ticket) = services.people.claim_profile(request.token) {
        return request_profile_on_peer(&mut stream, ticket, services).await;
    }
    if let Some(ticket) = services.folders.claim_peer(request.token) {
        return browse_folder_on_peer(&mut stream, ticket, services).await;
    }
    if let Some(ticket) = services.shares.claim_peer(request.token) {
        return browse_shares_on_peer(&mut stream, ticket, services).await;
    }
    handle_peer_messages(
        &mut stream,
        &request.username,
        services,
        PeerMessagePurpose::General,
    )
    .await
}

fn spawn_outbound_search_response_peer(
    address: PeerAddress,
    ticket: SearchResponseTicket,
    own_username: String,
    services: PeerServices,
    limit: Arc<Semaphore>,
) {
    if address.port == 0 || address.port > u16::MAX.into() {
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let connection_token = ticket.connection_token;
        let Ok(Ok(mut stream)) = timeout(
            PEER_CONNECT_TIMEOUT,
            TcpStream::connect((address.address, address.port as u16)),
        )
        .await
        else {
            services.local_shares.fail_search(connection_token);
            return;
        };
        let _ = stream.set_nodelay(true);
        if write_raw_frame(&mut stream, &peer_init_frame(&own_username, "P"))
            .await
            .is_err()
        {
            services.local_shares.fail_search(connection_token);
            return;
        }
        let Some(claimed) = services.local_shares.claim_search(ticket.connection_token) else {
            return;
        };
        let _ = send_search_response_on_peer(&mut stream, claimed, &services).await;
    });
}

fn spawn_outbound_upload_control_peer(
    address: PeerAddress,
    ticket: UploadTicket,
    own_username: String,
    services: PeerServices,
    limit: Arc<Semaphore>,
) {
    if address.port == 0 || address.port > u16::MAX.into() {
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let result = async {
            let mut stream = timeout(
                PEER_CONNECT_TIMEOUT,
                TcpStream::connect((address.address, address.port as u16)),
            )
            .await
            .map_err(|_| peer_timeout_error())??;
            let _ = stream.set_nodelay(true);
            write_raw_frame(&mut stream, &peer_init_frame(&own_username, "P")).await?;
            let claimed = services
                .uploads
                .claim_control(ticket.connection_token)
                .ok_or_else(peer_timeout_error)?;
            negotiate_upload_on_peer(&mut stream, claimed, services.clone()).await
        }
        .await;
        if let Err(error) = result {
            if services.uploads.fail_connection(
                ticket.connection_token,
                format!("The downloader did not accept the upload: {error}"),
            ) {
                let _ = services
                    .command_sender
                    .send(ConnectionCommand::ScheduleUploads);
            }
        }
    });
}

fn spawn_outbound_upload_file_peer(
    address: PeerAddress,
    ticket: UploadTicket,
    own_username: String,
    services: PeerServices,
    limit: Arc<Semaphore>,
) {
    if address.port == 0 || address.port > u16::MAX.into() {
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let Ok(Ok(mut stream)) = timeout(
            PEER_CONNECT_TIMEOUT,
            TcpStream::connect((address.address, address.port as u16)),
        )
        .await
        else {
            return;
        };
        let _ = stream.set_nodelay(true);
        if write_raw_frame(&mut stream, &peer_init_frame(&own_username, "F"))
            .await
            .is_err()
        {
            return;
        }
        let Some(claimed) = services.uploads.claim_file(ticket.connection_token) else {
            return;
        };
        if stream.write_u32_le(claimed.transfer_token).await.is_err()
            || stream.flush().await.is_err()
        {
            services.uploads.fail_id(
                &claimed.id,
                "The upload file connection could not be started.".to_owned(),
            );
            let _ = services
                .command_sender
                .send(ConnectionCommand::ScheduleUploads);
            return;
        }
        spawn_file_upload(stream, claimed, services.uploads, services.command_sender);
    });
}

async fn send_search_response_on_peer(
    stream: &mut TcpStream,
    ticket: SearchResponseTicket,
    services: &PeerServices,
) -> Result<(), ProtocolError> {
    let frame = file_search_response_frame(
        &services.own_username,
        ticket.search_token,
        &ticket.files,
        services
            .uploads
            .has_free_slot(services.local_shares.upload_slots()),
        0,
        services.uploads.queued_count(),
    )?;
    write_raw_frame(stream, &frame).await?;
    if ticket.origin == SearchResponseOrigin::Distributed {
        services.distributed.record_answered();
    }
    Ok(())
}

async fn negotiate_upload_on_peer(
    stream: &mut TcpStream,
    ticket: UploadTicket,
    services: PeerServices,
) -> Result<(), ProtocolError> {
    write_raw_frame(
        stream,
        &transfer_request_frame(
            ticket.transfer_token,
            &ticket.remote_filename,
            ticket.size_bytes,
        ),
    )
    .await?;
    let frame = timeout(PEER_MESSAGE_TIMEOUT, read_peer_frame(stream))
        .await
        .map_err(|_| peer_timeout_error())??;
    let (token, allowed, reason) = parse_transfer_response(&frame)?;
    if token != ticket.transfer_token || !allowed {
        services.uploads.fail_id(
            &ticket.id,
            reason.unwrap_or_else(|| "The downloader declined the upload.".to_owned()),
        );
        let _ = services
            .command_sender
            .send(ConnectionCommand::ScheduleUploads);
        return Ok(());
    }
    let _ = services
        .command_sender
        .send(ConnectionCommand::OpenUploadFile { id: ticket.id });
    Ok(())
}

fn spawn_outbound_download_peer(
    address: PeerAddress,
    ticket: TransferTicket,
    own_username: String,
    services: PeerServices,
    limit: Arc<Semaphore>,
) {
    if address.port == 0 || address.port > u16::MAX.into() {
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let PeerServices {
            search,
            wanted,
            radar,
            folders,
            shares,
            people,
            transfers,
            local_shares,
            uploads,
            distributed,
            own_username: service_username,
            command_sender,
        } = services;
        let _permit = permit;
        let Ok(Ok(mut stream)) = timeout(
            PEER_CONNECT_TIMEOUT,
            TcpStream::connect((address.address, address.port as u16)),
        )
        .await
        else {
            return;
        };
        let _ = stream.set_nodelay(true);
        if write_raw_frame(&mut stream, &peer_init_frame(&own_username, "P"))
            .await
            .is_err()
        {
            return;
        }
        let Some(claimed) = transfers.claim_peer(ticket.connection_token) else {
            return;
        };
        if let Err(error) = queue_download_on_peer(
            &mut stream,
            claimed.clone(),
            PeerServices {
                search,
                wanted,
                radar,
                folders,
                shares,
                people,
                transfers: transfers.clone(),
                local_shares,
                uploads,
                distributed,
                own_username: service_username,
                command_sender: command_sender.clone(),
            },
        )
        .await
        {
            if transfers.retry_id(
                &claimed.id,
                format!("The source connection ended before the file was queued: {error}"),
            ) {
                let _ = command_sender.send(ConnectionCommand::ScheduleDownloads);
            }
        }
    });
}

fn spawn_outbound_folder_peer(
    address: PeerAddress,
    ticket: FolderTicket,
    own_username: String,
    services: PeerServices,
    limit: Arc<Semaphore>,
) {
    if address.port == 0 || address.port > u16::MAX.into() {
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let PeerServices {
            search,
            wanted,
            radar,
            folders,
            shares,
            people,
            transfers,
            local_shares,
            uploads,
            distributed,
            own_username: service_username,
            command_sender,
        } = services;
        let _permit = permit;
        let Ok(Ok(mut stream)) = timeout(
            PEER_CONNECT_TIMEOUT,
            TcpStream::connect((address.address, address.port as u16)),
        )
        .await
        else {
            return;
        };
        let _ = stream.set_nodelay(true);
        if write_raw_frame(&mut stream, &peer_init_frame(&own_username, "P"))
            .await
            .is_err()
        {
            return;
        }
        let Some(claimed) = folders.claim_peer(ticket.connection_token) else {
            return;
        };
        if let Err(error) = browse_folder_on_peer(
            &mut stream,
            claimed.clone(),
            PeerServices {
                search,
                wanted,
                radar,
                folders: folders.clone(),
                shares,
                people,
                transfers,
                local_shares,
                uploads,
                distributed,
                own_username: service_username,
                command_sender,
            },
        )
        .await
        {
            folders.fail_connection(
                claimed.connection_token,
                format!("The folder connection ended before the source answered: {error}"),
            );
        }
    });
}

fn spawn_outbound_shares_peer(
    address: PeerAddress,
    ticket: SharesTicket,
    own_username: String,
    services: PeerServices,
    limit: Arc<Semaphore>,
) {
    if address.port == 0 || address.port > u16::MAX.into() {
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let Ok(Ok(mut stream)) = timeout(
            PEER_CONNECT_TIMEOUT,
            TcpStream::connect((address.address, address.port as u16)),
        )
        .await
        else {
            return;
        };
        let _ = stream.set_nodelay(true);
        if write_raw_frame(&mut stream, &peer_init_frame(&own_username, "P"))
            .await
            .is_err()
        {
            return;
        }
        let Some(claimed) = services.shares.claim_peer(ticket.connection_token) else {
            return;
        };
        if let Err(error) =
            browse_shares_on_peer(&mut stream, claimed.clone(), services.clone()).await
        {
            services.shares.fail_connection(
                claimed.connection_token,
                format!("The share-list connection ended before the source answered: {error}"),
            );
        }
    });
}

fn spawn_outbound_profile_peer(
    address: PeerAddress,
    ticket: ProfileTicket,
    own_username: String,
    services: PeerServices,
    limit: Arc<Semaphore>,
) {
    if address.port == 0 || address.port > u16::MAX.into() {
        services.people.fail_profile(
            ticket.connection_token,
            "The user is not accepting profile connections.".to_owned(),
        );
        return;
    }
    let Ok(permit) = limit.try_acquire_owned() else {
        services.people.fail_profile(
            ticket.connection_token,
            "Flipside is handling too many Soulseek peer connections to open this profile."
                .to_owned(),
        );
        return;
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let result = async {
            let mut stream = timeout(
                PEER_CONNECT_TIMEOUT,
                TcpStream::connect((address.address, address.port as u16)),
            )
            .await
            .map_err(|_| peer_timeout_error())??;
            let _ = stream.set_nodelay(true);
            write_raw_frame(&mut stream, &peer_init_frame(&own_username, "P")).await?;
            let claimed = services
                .people
                .claim_profile(ticket.connection_token)
                .ok_or_else(peer_timeout_error)?;
            request_profile_on_peer(&mut stream, claimed, services.clone()).await
        }
        .await;
        if let Err(error) = result {
            services.people.fail_profile(
                ticket.connection_token,
                format!("The user profile connection ended before a response arrived: {error}"),
            );
        }
    });
}

async fn queue_download_on_peer(
    stream: &mut TcpStream,
    ticket: TransferTicket,
    services: PeerServices,
) -> Result<(), super::protocol::ProtocolError> {
    write_raw_frame(stream, &queue_upload_frame(&ticket.remote_filename)).await?;
    write_raw_frame(
        stream,
        &place_in_queue_request_frame(&ticket.remote_filename),
    )
    .await?;
    handle_peer_messages(
        stream,
        &ticket.username,
        services,
        PeerMessagePurpose::Transfer,
    )
    .await
}

async fn browse_folder_on_peer(
    stream: &mut TcpStream,
    ticket: FolderTicket,
    services: PeerServices,
) -> Result<(), super::protocol::ProtocolError> {
    write_raw_frame(
        stream,
        &folder_contents_request_frame(ticket.folder_token, &ticket.folder),
    )
    .await?;
    handle_peer_messages(
        stream,
        &ticket.username,
        services,
        PeerMessagePurpose::Folder,
    )
    .await
}

async fn browse_shares_on_peer(
    stream: &mut TcpStream,
    ticket: SharesTicket,
    services: PeerServices,
) -> Result<(), super::protocol::ProtocolError> {
    write_raw_frame(stream, &shared_file_list_request_frame()).await?;
    handle_peer_messages(
        stream,
        &ticket.username,
        services,
        PeerMessagePurpose::Shares(ticket.connection_token),
    )
    .await
}

async fn request_profile_on_peer(
    stream: &mut TcpStream,
    ticket: ProfileTicket,
    services: PeerServices,
) -> Result<(), super::protocol::ProtocolError> {
    write_raw_frame(stream, &user_info_request_frame()).await?;
    handle_peer_messages(
        stream,
        &ticket.username,
        services,
        PeerMessagePurpose::Profile(ticket.connection_token),
    )
    .await
}

async fn handle_peer_messages(
    stream: &mut TcpStream,
    username: &str,
    services: PeerServices,
    purpose: PeerMessagePurpose,
) -> Result<(), super::protocol::ProtocolError> {
    if !username.eq_ignore_ascii_case(&services.own_username) && services.people.observe(username) {
        let _ = services
            .command_sender
            .send(ConnectionCommand::WatchPerson {
                username: username.to_owned(),
            });
    }
    loop {
        let frame_result = match purpose {
            PeerMessagePurpose::Profile(_) => {
                timeout(PEER_IDLE_TIMEOUT, read_profile_frame(stream)).await
            }
            _ => timeout(PEER_IDLE_TIMEOUT, read_peer_frame(stream)).await,
        };
        let expects_response = matches!(
            purpose,
            PeerMessagePurpose::Folder
                | PeerMessagePurpose::Shares(_)
                | PeerMessagePurpose::Profile(_)
        );
        let frame = match frame_result {
            Ok(Ok(frame)) => frame,
            Ok(Err(ProtocolError::Io(error)))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                if expects_response {
                    return Err(error.into());
                }
                return Ok(());
            }
            Ok(Err(error)) => return Err(error),
            Err(_) if expects_response => return Err(peer_timeout_error()),
            Err(_) => return Ok(()),
        };
        match frame.code {
            USER_INFO_REQUEST_CODE => {
                let response = user_info_response_frame(
                    "Sharing music with Flipside.",
                    None,
                    u32::try_from(services.local_shares.upload_slots()).unwrap_or(u32::MAX),
                    services.uploads.queued_count(),
                    services
                        .uploads
                        .has_free_slot(services.local_shares.upload_slots()),
                    1,
                )?;
                write_raw_frame(stream, &response).await?;
            }
            USER_INFO_RESPONSE_CODE => {
                if let PeerMessagePurpose::Profile(connection_token) = purpose {
                    services.people.resolve_user_info(
                        connection_token,
                        username,
                        parse_user_info_response(&frame)?,
                    );
                    return Ok(());
                }
            }
            SHARED_FILE_LIST_REQUEST_CODE => {
                let shares = if services.people.is_blocked(username) {
                    Vec::new()
                } else {
                    services.local_shares.share_list()
                };
                let response = shared_file_list_response_frame(&shares)?;
                write_raw_frame(stream, &response).await?;
            }
            FOLDER_CONTENTS_REQUEST_CODE => {
                let (token, folder) = parse_folder_contents_request(&frame)?;
                let folders = if services.people.is_blocked(username) {
                    Vec::new()
                } else {
                    services.local_shares.folder_list(&folder)
                };
                let response = folder_contents_response_frame(token, &folder, &folders)?;
                write_raw_frame(stream, &response).await?;
            }
            QUEUE_UPLOAD_CODE => {
                let filename = parse_filename(&frame, QUEUE_UPLOAD_CODE)?;
                if services.people.is_blocked(username) {
                    write_raw_frame(
                        stream,
                        &upload_denied_frame(&filename, "Banned by the sharing user."),
                    )
                    .await?;
                } else if let Some(file) = services.local_shares.resolve_file(&filename) {
                    match services.uploads.enqueue(username, file) {
                        Ok(_) => {
                            let _ = services
                                .command_sender
                                .send(ConnectionCommand::ScheduleUploads);
                        }
                        Err(error) => {
                            write_raw_frame(
                                stream,
                                &upload_denied_frame(&filename, &error.to_string()),
                            )
                            .await?;
                        }
                    }
                } else {
                    write_raw_frame(
                        stream,
                        &upload_denied_frame(&filename, "File is not shared."),
                    )
                    .await?;
                }
            }
            PLACE_IN_QUEUE_REQUEST_CODE => {
                let filename = parse_filename(&frame, PLACE_IN_QUEUE_REQUEST_CODE)?;
                if services.people.is_blocked(username) {
                    write_raw_frame(
                        stream,
                        &upload_denied_frame(&filename, "Banned by the sharing user."),
                    )
                    .await?;
                } else if let Some(position) = services.uploads.queue_position(username, &filename)
                {
                    write_raw_frame(stream, &place_in_queue_response_frame(&filename, position))
                        .await?;
                } else {
                    write_raw_frame(
                        stream,
                        &upload_denied_frame(&filename, "File is not queued."),
                    )
                    .await?;
                }
            }
            FILE_SEARCH_RESPONSE_CODE => {
                let response = parse_search_response(&frame)?;
                if services.people.is_ignored(&response.username) {
                    if purpose == PeerMessagePurpose::General {
                        return Ok(());
                    }
                    continue;
                }
                if services.people.observe(&response.username) {
                    let _ = services
                        .command_sender
                        .send(ConnectionCommand::WatchPerson {
                            username: response.username.clone(),
                        });
                }
                services.wanted.record(&response);
                services.radar.record(&response);
                services.search.record(response);
                if purpose == PeerMessagePurpose::General {
                    return Ok(());
                }
            }
            FOLDER_CONTENTS_RESPONSE_CODE
                if services
                    .folders
                    .resolve(username, parse_folder_contents_response(&frame)?) =>
            {
                return Ok(())
            }
            FOLDER_CONTENTS_RESPONSE_CODE => {}
            SHARED_FILE_LIST_RESPONSE_CODE => {
                if let PeerMessagePurpose::Shares(connection_token) = purpose {
                    services.shares.resolve(
                        connection_token,
                        username,
                        parse_shared_file_list_response(&frame)?,
                    );
                    return Ok(());
                }
            }
            TRANSFER_REQUEST_CODE => {
                let request = parse_transfer_request(&frame)?;
                let accepted = request.direction == 1
                    && request.size_bytes.is_some_and(|size| {
                        services
                            .transfers
                            .accept_upload_request(username, &request.filename, request.token, size)
                            .is_some()
                    });
                write_raw_frame(
                    stream,
                    &transfer_response_frame(
                        request.token,
                        accepted,
                        (!accepted).then_some("Cancelled"),
                    ),
                )
                .await?;
                if !accepted
                    && services.transfers.fail_for_filename(
                        username,
                        &request.filename,
                        "The source reported a different file size than the search result."
                            .to_owned(),
                    )
                {
                    let _ = services
                        .command_sender
                        .send(ConnectionCommand::ScheduleDownloads);
                }
            }
            PLACE_IN_QUEUE_RESPONSE_CODE => {
                let (filename, position) = parse_queue_position(&frame)?;
                services
                    .transfers
                    .set_queue_position(username, &filename, position);
            }
            UPLOAD_FAILED_CODE => {
                let filename = parse_filename(&frame, UPLOAD_FAILED_CODE)?;
                if services.transfers.retry_for_filename(
                    username,
                    &filename,
                    "The source stopped the upload before the file completed.".to_owned(),
                ) {
                    let _ = services
                        .command_sender
                        .send(ConnectionCommand::ScheduleDownloads);
                }
            }
            UPLOAD_DENIED_CODE => {
                let (filename, reason) = parse_upload_denied(&frame)?;
                if services.transfers.fail_for_filename(
                    username,
                    &filename,
                    format!("The source declined the download: {reason}"),
                ) {
                    let _ = services
                        .command_sender
                        .send(ConnectionCommand::ScheduleDownloads);
                }
            }
            _ => {}
        }
    }
}

fn spawn_transfer_request_timeout(
    ticket: TransferTicket,
    transfers: TransferHub,
    command_sender: mpsc::UnboundedSender<ConnectionCommand>,
) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(TRANSFER_REQUEST_TIMEOUT).await;
        if transfers.retry_connection(
            ticket.connection_token,
            "The source did not answer the peer connection request.".to_owned(),
        ) {
            let _ = command_sender.send(ConnectionCommand::ScheduleDownloads);
        }
    });
}

fn spawn_upload_request_timeout(
    ticket: UploadTicket,
    uploads: UploadHub,
    command_sender: mpsc::UnboundedSender<ConnectionCommand>,
) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(TRANSFER_REQUEST_TIMEOUT).await;
        if uploads.fail_connection(
            ticket.connection_token,
            "The downloader did not answer the upload connection request.".to_owned(),
        ) {
            let _ = command_sender.send(ConnectionCommand::ScheduleUploads);
        }
    });
}

fn spawn_file_download(
    stream: TcpStream,
    transfer_token: u32,
    transfers: TransferHub,
    command_sender: mpsc::UnboundedSender<ConnectionCommand>,
) {
    let plan = match transfers.begin_file(transfer_token) {
        Ok(plan) => plan,
        Err(error) => {
            if transfers.fail_transfer_token(transfer_token, error.to_string()) {
                let _ = command_sender.send(ConnectionCommand::ScheduleDownloads);
            }
            return;
        }
    };
    let id = plan.id.clone();
    let cancellation = Arc::new(AtomicBool::new(false));
    transfers.register_task(id.clone(), cancellation.clone());
    let task_transfers = transfers.clone();
    let task_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = receive_file(stream, &plan, task_transfers.clone(), cancellation).await;
        if let Err(message) = outcome {
            if retryable_download_error(&message) {
                task_transfers.retry_id(&task_id, message);
            } else {
                task_transfers.fail_id(&task_id, message);
            }
        }
        task_transfers.unregister_task(&task_id);
        let _ = command_sender.send(ConnectionCommand::ScheduleDownloads);
    });
}

fn spawn_file_upload(
    stream: TcpStream,
    ticket: UploadTicket,
    uploads: UploadHub,
    command_sender: mpsc::UnboundedSender<ConnectionCommand>,
) {
    let plan = match uploads.begin_file(&ticket.id) {
        Ok(plan) => plan,
        Err(error) => {
            uploads.fail_id(&ticket.id, error.to_string());
            let _ = command_sender.send(ConnectionCommand::ScheduleUploads);
            return;
        }
    };
    let cancellation = Arc::new(AtomicBool::new(false));
    uploads.register_task(plan.id.clone(), cancellation.clone());
    let task_uploads = uploads.clone();
    tauri::async_runtime::spawn(async move {
        let result = send_file(stream, &plan, task_uploads.clone(), cancellation).await;
        if let Err(message) = result {
            task_uploads.fail_id(&plan.id, message);
        }
        task_uploads.unregister_task(&plan.id);
        let _ = command_sender.send(ConnectionCommand::ScheduleUploads);
    });
}

async fn send_file(
    mut stream: TcpStream,
    plan: &UploadTicket,
    uploads: UploadHub,
    cancellation: Arc<AtomicBool>,
) -> Result<(), String> {
    let offset = timeout(PEER_MESSAGE_TIMEOUT, stream.read_u64_le())
        .await
        .map_err(|_| "The downloader did not send a resume offset in time.".to_owned())?
        .map_err(|error| format!("The upload resume offset could not be read: {error}"))?;
    if offset > plan.size_bytes {
        return Err("The downloader requested an invalid resume offset.".to_owned());
    }
    let mut file = File::open(&plan.local_path)
        .await
        .map_err(|error| format!("Flipside could not open the shared file: {error}"))?;
    file.seek(SeekFrom::Start(offset))
        .await
        .map_err(|error| format!("Flipside could not seek the shared file: {error}"))?;

    let mut transferred = offset;
    let mut last_bytes = transferred;
    let mut last_update = Instant::now();
    let mut buffer = vec![0_u8; FILE_BUFFER_SIZE];
    while transferred < plan.size_bytes {
        if cancellation.load(Ordering::SeqCst) {
            return Ok(());
        }
        let remaining = plan.size_bytes - transferred;
        let capacity = usize::try_from(remaining.min(FILE_BUFFER_SIZE as u64))
            .expect("bounded file read size fits usize");
        let count = file
            .read(&mut buffer[..capacity])
            .await
            .map_err(|error| format!("Flipside could not read the shared file: {error}"))?;
        if count == 0 {
            return Err("The shared file ended before its indexed size.".to_owned());
        }
        stream
            .write_all(&buffer[..count])
            .await
            .map_err(|error| format!("The downloader connection was interrupted: {error}"))?;
        transferred += count as u64;
        let elapsed = last_update.elapsed();
        if elapsed >= Duration::from_millis(250) || transferred == plan.size_bytes {
            let millis = elapsed.as_millis().max(1) as u64;
            let speed = transferred.saturating_sub(last_bytes).saturating_mul(1_000) / millis;
            uploads.update_progress(&plan.id, transferred, speed);
            last_bytes = transferred;
            last_update = Instant::now();
        }
    }
    stream
        .flush()
        .await
        .map_err(|error| format!("The upload could not be finished: {error}"))?;
    if !cancellation.load(Ordering::SeqCst) {
        uploads.complete(&plan.id);
    }
    Ok(())
}

async fn receive_file(
    mut stream: TcpStream,
    plan: &DownloadPlan,
    transfers: TransferHub,
    cancellation: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&plan.partial_path)
        .await
        .map_err(|error| format!("Flipside could not open the partial file: {error}"))?;
    stream
        .write_u64_le(plan.offset)
        .await
        .map_err(|error| format!("The resume offset could not be sent: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("The source connection could not be started: {error}"))?;

    let mut transferred = plan.offset;
    let mut last_bytes = transferred;
    let mut last_update = Instant::now();
    let mut buffer = vec![0_u8; FILE_BUFFER_SIZE];
    while transferred < plan.size_bytes {
        if cancellation.load(Ordering::SeqCst) {
            file.flush().await.map_err(|error| {
                format!("Flipside could not flush the paused file: {error}")
            })?;
            file.sync_data().await.map_err(|error| {
                format!("Flipside could not secure the paused file: {error}")
            })?;
            return Ok(());
        }
        let remaining = plan.size_bytes - transferred;
        let capacity = usize::try_from(remaining.min(FILE_BUFFER_SIZE as u64))
            .expect("bounded file read size fits usize");
        let count = tokio::select! {
            result = stream.read(&mut buffer[..capacity]) => result
                .map_err(|error| format!("The source connection was interrupted: {error}"))?,
            _ = tokio::time::sleep(Duration::from_millis(100)) => continue,
        };
        if count == 0 {
            return Err("The source closed the connection before the file completed.".to_owned());
        }
        file.write_all(&buffer[..count])
            .await
            .map_err(|error| format!("Flipside could not write the partial file: {error}"))?;
        transferred += count as u64;
        let elapsed = last_update.elapsed();
        if elapsed >= Duration::from_millis(250) || transferred == plan.size_bytes {
            let millis = elapsed.as_millis().max(1) as u64;
            let speed = transferred.saturating_sub(last_bytes).saturating_mul(1_000) / millis;
            transfers.update_progress(&plan.id, transferred, speed);
            last_bytes = transferred;
            last_update = Instant::now();
        }
    }
    if cancellation.load(Ordering::SeqCst) {
        return Ok(());
    }
    file.flush()
        .await
        .map_err(|error| format!("Flipside could not finish the partial file: {error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("Flipside could not secure the completed file: {error}"))?;
    drop(file);

    if plan.final_path.exists() {
        return Err("A file appeared at the final download path before completion.".to_owned());
    }
    tokio::fs::rename(&plan.partial_path, &plan.final_path)
        .await
        .map_err(|error| {
            format!("Flipside could not finalize the downloaded file: {error}")
        })?;
    transfers
        .complete(&plan.id)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn peer_timeout_error() -> super::protocol::ProtocolError {
    super::protocol::ProtocolError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "Soulseek peer timed out",
    ))
}

fn retryable_download_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("source connection was interrupted")
        || normalized.contains("source closed the connection")
        || normalized.contains("timed out")
}

fn server_label(profile: &ConnectionProfile) -> String {
    format!("{}:{}", profile.server_host, profile.server_port)
}

fn retry_delay(attempt: u32) -> Duration {
    let seconds = 2_u64.saturating_pow(attempt).min(30);
    Duration::from_secs(seconds)
}

fn rejection_failure(reason: &str, detail: Option<&str>) -> ConnectionFailure {
    let normalized = reason.to_ascii_uppercase();
    let (message, retryable) = match normalized.as_str() {
        "INVALIDPASS" => (
            "That Soulseek username or password was not accepted.".to_owned(),
            false,
        ),
        "EMPTYPASSWORD" => ("Enter your Soulseek password.".to_owned(), false),
        "INVALIDUSERNAME" => detail
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .map(|message| (message, false))
            .unwrap_or_else(|| ("That Soulseek username is not valid.".to_owned(), false)),
        "INVALIDVERSION" => (
            "This version of Flipside is not accepted by the Soulseek server. Check for an update."
                .to_owned(),
            false,
        ),
        "SVRFULL" => (
            "The Soulseek server is full. Flipside will try again.".to_owned(),
            true,
        ),
        "SVRPRIVATE" => (
            "The Soulseek server is currently private.".to_owned(),
            false,
        ),
        _ => (
            format!("The Soulseek server rejected the login ({reason})."),
            false,
        ),
    };
    if retryable {
        ConnectionFailure::retryable(message, "login_rejected")
    } else {
        ConnectionFailure::fatal(message, "login_rejected")
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

#[derive(Debug)]
struct ConnectionFailure {
    message: String,
    event: &'static str,
    retryable: bool,
}

impl ConnectionFailure {
    fn retryable(message: impl Into<String>, event: &'static str) -> Self {
        Self {
            message: message.into(),
            event,
            retryable: true,
        }
    }

    fn fatal(message: impl Into<String>, event: &'static str) -> Self {
        Self {
            message: message.into(),
            event,
            retryable: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConnectionServiceError {
    #[error("{0}")]
    Settings(#[from] super::settings::SettingsError),
    #[error("{0}")]
    Credentials(#[from] super::credentials::CredentialError),
    #[error("{0}")]
    Transfer(#[from] TransferError),
    #[error("{0}")]
    Folder(#[from] FolderError),
    #[error("{0}")]
    Shares(#[from] SharesError),
    #[error("{0}")]
    LocalShares(#[from] LocalSharesError),
    #[error("{0}")]
    Upload(#[from] UploadError),
    #[error("{0}")]
    People(#[from] PeopleError),
    #[error("{0}")]
    Messages(#[from] MessagesError),
    #[error("{0}")]
    Rooms(#[from] RoomsError),
    #[error("{0}")]
    Wanted(#[from] WantedError),
    #[error("Add your Soulseek account before connecting.")]
    NotConfigured,
    #[error("Enter your Soulseek password.")]
    MissingPassword,
    #[error("Connect to Soulseek before starting a live search.")]
    SearchUnavailable,
    #[error("{0}")]
    InvalidSearch(String),
    #[error("Could not initialize connection diagnostics: {0}")]
    Diagnostics(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn reconnect_backoff_caps_at_thirty_seconds() {
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(2), Duration::from_secs(4));
        assert_eq!(retry_delay(6), Duration::from_secs(30));
        assert_eq!(retry_delay(30), Duration::from_secs(30));
    }

    #[test]
    fn rejection_messages_are_safe_and_actionable() {
        let invalid_password = rejection_failure("INVALIDPASS", None);
        assert!(!invalid_password.retryable);
        assert!(invalid_password.message.contains("username or password"));

        let invalid_username = rejection_failure("INVALIDUSERNAME", Some("Name unavailable."));
        assert_eq!(invalid_username.message, "Name unavailable.");
    }

    #[tokio::test]
    async fn server_frame_pump_preserves_fragmented_frames_while_other_events_fire() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let (sender, mut receiver) = mpsc::channel(2);
        let pump = tokio::spawn(forward_server_frames(reader, sender));
        let first = server_ping_frame();
        let second = set_online_frame();

        let fragmented_writer = tokio::spawn(async move {
            for byte in first.into_iter().chain(second) {
                writer.write_all(&[byte]).await.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        let mut competing_tick = tokio::time::interval(Duration::from_millis(1));
        let mut competing_events = 0;
        let mut received = Vec::new();
        while received.len() < 2 {
            tokio::select! {
                biased;
                _ = competing_tick.tick(), if competing_events == 0 => competing_events += 1,
                frame = receiver.recv() => received.push(frame.unwrap().unwrap()),
            }
        }

        assert!(competing_events > 0);
        assert_eq!(received[0].code, super::super::protocol::SERVER_PING_CODE);
        assert_eq!(received[1].code, super::super::protocol::SET_STATUS_CODE);
        fragmented_writer.await.unwrap();
        pump.abort();
    }

    #[tokio::test]
    async fn distributed_coordinator_adopts_only_a_candidate_with_branch_state() {
        let mut coordinator = DistributedCoordinator::new();
        coordinator.candidates.insert(
            7,
            DistributedCandidateTask {
                branch_level: None,
                branch_root: None,
                _task: AbortOnDrop(tauri::async_runtime::spawn(std::future::pending())),
            },
        );
        coordinator.candidates.insert(
            8,
            DistributedCandidateTask {
                branch_level: Some(3),
                branch_root: Some("branch-root".to_owned()),
                _task: AbortOnDrop(tauri::async_runtime::spawn(std::future::pending())),
            },
        );

        assert_eq!(coordinator.adopt(7), None);
        assert_eq!(coordinator.adopt(8), Some((4, "branch-root".to_owned())));
        assert!(coordinator.accepts_search_from(8));
        assert_eq!(coordinator.candidates.len(), 1);
        assert!(coordinator.close(8));
    }

    #[tokio::test]
    async fn distributed_parent_connection_sends_d_init_and_streams_frames() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (event_sender, mut event_receiver) = mpsc::channel(4);
        let parent = ParentCandidate {
            username: "relay-user".to_owned(),
            address: "127.0.0.1".parse().unwrap(),
            port: address.port(),
        };
        let candidate_task = spawn_distributed_parent_candidate(
            17,
            parent,
            "flipside-user".to_owned(),
            event_sender,
        );

        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert_eq!(
                read_peer_init(&mut stream).await.unwrap(),
                PeerInit::Peer {
                    username: "flipside-user".to_owned(),
                    connection_type: "D".to_owned(),
                    token: 0,
                }
            );
            let mut payload = Vec::new();
            payload.extend(11_u32.to_le_bytes());
            payload.extend(b"branch-root");
            let mut frame = Vec::new();
            frame.extend(u32::try_from(payload.len() + 1).unwrap().to_le_bytes());
            frame.push(DISTRIBUTED_BRANCH_ROOT_CODE);
            frame.extend(payload);
            stream.write_all(&frame).await.unwrap();
        });

        let event = timeout(Duration::from_secs(2), event_receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let DistributedPeerEvent::Frame { id, frame } = event else {
            panic!("expected a distributed frame");
        };
        assert_eq!(id, 17);
        assert_eq!(
            parse_distributed_branch_root(&frame).unwrap(),
            "branch-root"
        );
        peer.await.unwrap();
        drop(candidate_task);
    }
}
