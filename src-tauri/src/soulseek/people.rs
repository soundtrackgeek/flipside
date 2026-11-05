use super::protocol::{UserInfoResponse, UserInterests, UserStats, WatchedUser};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::sync::oneshot;

pub const PEOPLE_EVENT: &str = "flipside://soulseek-people";
const STORE_VERSION: u32 = 1;
const MAX_RECENT: usize = 40;
const MAX_RUNTIME_PROFILES: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PersonStatus {
    Unknown,
    Offline,
    Away,
    Online,
}

impl PersonStatus {
    fn from_protocol(value: u32) -> Self {
        match value {
            0 => Self::Offline,
            1 => Self::Away,
            2 => Self::Online,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ProfileState {
    Idle,
    Loading,
    Ready,
    Unavailable,
    Error,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonProfile {
    pub username: String,
    pub status: PersonStatus,
    pub profile_state: ProfileState,
    pub country_code: Option<String>,
    pub description: Option<String>,
    pub picture_data_url: Option<String>,
    pub average_speed: u32,
    pub upload_count: u32,
    pub shared_file_count: u32,
    pub shared_directory_count: u32,
    pub upload_slots: Option<u32>,
    pub queue_size: Option<u32>,
    pub slots_free: Option<bool>,
    pub upload_permission: Option<u32>,
    pub likes: Vec<String>,
    pub hates: Vec<String>,
    pub privileged: bool,
    pub favorite: bool,
    pub blocked: bool,
    pub ignored: bool,
    pub error: Option<String>,
    pub last_seen_at_ms: Option<u64>,
    pub last_interaction_at_ms: u64,
    pub updated_at_ms: u64,
}

impl PersonProfile {
    fn new(
        username: String,
        favorite: bool,
        blocked: bool,
        ignored: bool,
        interacted_at_ms: u64,
    ) -> Self {
        Self {
            username,
            status: PersonStatus::Unknown,
            profile_state: ProfileState::Idle,
            country_code: None,
            description: None,
            picture_data_url: None,
            average_speed: 0,
            upload_count: 0,
            shared_file_count: 0,
            shared_directory_count: 0,
            upload_slots: None,
            queue_size: None,
            slots_free: None,
            upload_permission: None,
            likes: Vec::new(),
            hates: Vec::new(),
            privileged: false,
            favorite,
            blocked,
            ignored,
            error: None,
            last_seen_at_ms: None,
            last_interaction_at_ms: interacted_at_ms,
            updated_at_ms: timestamp_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PeopleSnapshot {
    pub users: Vec<PersonProfile>,
    pub favorite_count: u32,
    pub online_favorite_count: u32,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug)]
pub struct ProfileTicket {
    pub connection_token: u32,
    pub username: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentPerson {
    username: String,
    interacted_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PeopleStore {
    version: u32,
    favorites: Vec<String>,
    blocked: Vec<String>,
    #[serde(default)]
    ignored: Vec<String>,
    recent: Vec<RecentPerson>,
}

impl Default for PeopleStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            favorites: Vec::new(),
            blocked: Vec::new(),
            ignored: Vec::new(),
            recent: Vec::new(),
        }
    }
}

struct PendingProfile {
    ticket: ProfileTicket,
    claimed: bool,
    response: oneshot::Sender<Result<PersonProfile, PeopleError>>,
}

#[derive(Default)]
struct PeopleRuntime {
    profiles: HashMap<String, PersonProfile>,
    pending: HashMap<u32, PendingProfile>,
    watched: HashSet<String>,
}

#[derive(Clone)]
pub struct PeopleHub {
    app: AppHandle,
    path: PathBuf,
    store: Arc<RwLock<PeopleStore>>,
    runtime: Arc<Mutex<PeopleRuntime>>,
}

impl PeopleHub {
    pub fn new(app: AppHandle, path: PathBuf) -> Result<Self, PeopleError> {
        let store = load_store(&path)?;
        let hub = Self {
            app,
            path,
            store: Arc::new(RwLock::new(store)),
            runtime: Arc::new(Mutex::new(PeopleRuntime::default())),
        };
        hub.populate_saved_people();
        Ok(hub)
    }

    pub fn snapshot(&self) -> PeopleSnapshot {
        let store = self
            .store
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ensure_saved_profiles(&store, &mut runtime);
        let mut users: Vec<_> = runtime.profiles.values().cloned().collect();
        users.sort_by(|left, right| {
            right
                .favorite
                .cmp(&left.favorite)
                .then(status_rank(right.status).cmp(&status_rank(left.status)))
                .then(
                    right
                        .last_interaction_at_ms
                        .cmp(&left.last_interaction_at_ms),
                )
                .then_with(|| {
                    left.username
                        .to_ascii_lowercase()
                        .cmp(&right.username.to_ascii_lowercase())
                })
        });
        let favorite_count = users
            .iter()
            .filter(|profile| profile.favorite)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let online_favorite_count = users
            .iter()
            .filter(|profile| profile.favorite && profile.status == PersonStatus::Online)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        PeopleSnapshot {
            users,
            favorite_count,
            online_favorite_count,
            updated_at_ms: timestamp_ms(),
        }
    }

    pub fn observe(&self, username: &str) -> bool {
        let Some(username) = valid_username(username) else {
            return false;
        };
        let store = self
            .store
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = person_key(&username);
        if runtime.profiles.contains_key(&key) {
            return false;
        }
        if runtime.profiles.len() >= MAX_RUNTIME_PROFILES {
            return false;
        }
        let favorite = contains_username(&store.favorites, &username);
        let blocked = contains_username(&store.blocked, &username);
        let ignored = contains_username(&store.ignored, &username);
        runtime.profiles.insert(
            key,
            PersonProfile::new(username, favorite, blocked, ignored, timestamp_ms()),
        );
        drop(runtime);
        drop(store);
        self.publish();
        true
    }

    pub fn remember(&self, username: &str) -> Result<(), PeopleError> {
        let username = valid_username(username).ok_or(PeopleError::InvalidUsername)?;
        let interacted_at_ms = timestamp_ms();
        self.observe(&username);
        {
            let mut store = self
                .store
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            store
                .recent
                .retain(|recent| !recent.username.eq_ignore_ascii_case(&username));
            store.recent.insert(
                0,
                RecentPerson {
                    username: username.clone(),
                    interacted_at_ms,
                },
            );
            store.recent.truncate(MAX_RECENT);
        }
        if let Some(profile) = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .profiles
            .get_mut(&person_key(&username))
        {
            profile.last_interaction_at_ms = interacted_at_ms;
            profile.updated_at_ms = interacted_at_ms;
        }
        self.persist()?;
        self.publish();
        Ok(())
    }

    pub fn is_ignored(&self, username: &str) -> bool {
        contains_username(
            &self
                .store
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .ignored,
            username,
        )
    }

    pub fn is_blocked(&self, username: &str) -> bool {
        contains_username(
            &self
                .store
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .blocked,
            username,
        )
    }

    pub fn requesting_for_username(&self, username: &str) -> Option<ProfileTicket> {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending
            .values()
            .find(|pending| {
                !pending.claimed && pending.ticket.username.eq_ignore_ascii_case(username)
            })
            .map(|pending| pending.ticket.clone())
    }

    pub fn claim_profile(&self, connection_token: u32) -> Option<ProfileTicket> {
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = runtime.pending.get_mut(&connection_token)?;
        if pending.claimed {
            return None;
        }
        pending.claimed = true;
        Some(pending.ticket.clone())
    }

    pub fn resolve_user_info(
        &self,
        connection_token: u32,
        username: &str,
        response: UserInfoResponse,
    ) -> bool {
        let pending = {
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(pending) = runtime.pending.get(&connection_token) else {
                return false;
            };
            if !pending.ticket.username.eq_ignore_ascii_case(username) {
                return false;
            }
            runtime.pending.remove(&connection_token)
        };
        let Some(pending) = pending else {
            return false;
        };
        let profile = {
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(profile) = runtime.profiles.get_mut(&person_key(username)) else {
                return false;
            };
            profile.description = clean_description(response.description);
            profile.picture_data_url = response.picture.as_deref().and_then(picture_data_url);
            profile.upload_slots = Some(response.upload_slots);
            profile.queue_size = Some(response.queue_size);
            profile.slots_free = Some(response.slots_free);
            profile.upload_permission = response.upload_permission;
            profile.profile_state = ProfileState::Ready;
            profile.error = None;
            profile.updated_at_ms = timestamp_ms();
            profile.clone()
        };
        let _ = pending.response.send(Ok(profile));
        self.publish();
        true
    }

    pub fn update_watch(&self, watched: WatchedUser) {
        self.observe(&watched.username);
        if let Some(profile) = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .profiles
            .get_mut(&person_key(&watched.username))
        {
            profile.status = if watched.exists {
                PersonStatus::from_protocol(watched.status)
            } else {
                PersonStatus::Offline
            };
            profile.average_speed = watched.average_speed;
            profile.upload_count = watched.upload_count;
            profile.shared_file_count = watched.shared_file_count;
            profile.shared_directory_count = watched.shared_directory_count;
            profile.country_code = watched.country_code;
            if matches!(profile.status, PersonStatus::Online | PersonStatus::Away) {
                profile.last_seen_at_ms = Some(timestamp_ms());
            }
            if !watched.exists {
                profile.profile_state = ProfileState::Unavailable;
                profile.error = Some("Soulseek does not recognize this username.".to_owned());
            }
            profile.updated_at_ms = timestamp_ms();
        }
        self.publish();
    }

    pub fn update_status(&self, username: &str, status: u32, privileged: bool) {
        self.observe(username);
        if let Some(profile) = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .profiles
            .get_mut(&person_key(username))
        {
            profile.status = PersonStatus::from_protocol(status);
            profile.privileged = privileged;
            if matches!(profile.status, PersonStatus::Online | PersonStatus::Away) {
                profile.last_seen_at_ms = Some(timestamp_ms());
            }
            profile.updated_at_ms = timestamp_ms();
        }
        self.publish();
    }

    pub fn update_stats(&self, stats: UserStats) {
        self.observe(&stats.username);
        if let Some(profile) = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .profiles
            .get_mut(&person_key(&stats.username))
        {
            profile.average_speed = stats.average_speed;
            profile.upload_count = stats.upload_count;
            profile.shared_file_count = stats.shared_file_count;
            profile.shared_directory_count = stats.shared_directory_count;
            profile.updated_at_ms = timestamp_ms();
        }
        self.publish();
    }

    pub fn update_interests(&self, interests: UserInterests) {
        self.observe(&interests.username);
        if let Some(profile) = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .profiles
            .get_mut(&person_key(&interests.username))
        {
            profile.likes = interests.likes;
            profile.hates = interests.hates;
            profile.updated_at_ms = timestamp_ms();
        }
        self.publish();
    }

    pub fn mark_watched(&self, username: &str) -> bool {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .watched
            .insert(person_key(username))
    }

    pub fn saved_users_to_watch(&self) -> Vec<String> {
        self.store
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .favorites
            .clone()
    }

    pub fn fail_profile(&self, connection_token: u32, message: String) -> bool {
        let pending = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending
            .remove(&connection_token);
        if let Some(pending) = pending {
            if let Some(profile) = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .profiles
                .get_mut(&person_key(&pending.ticket.username))
            {
                profile.profile_state = ProfileState::Error;
                profile.error = Some(message.clone());
                profile.updated_at_ms = timestamp_ms();
            }
            let _ = pending.response.send(Err(PeopleError::Request(message)));
            self.publish();
            true
        } else {
            false
        }
    }

    pub fn connection_lost(&self) {
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.watched.clear();
        let pending = std::mem::take(&mut runtime.pending);
        for profile in runtime.profiles.values_mut() {
            profile.status = PersonStatus::Unknown;
            if profile.profile_state == ProfileState::Loading {
                profile.profile_state = ProfileState::Error;
                profile.error = Some("The Soulseek connection changed.".to_owned());
            }
            profile.updated_at_ms = timestamp_ms();
        }
        drop(runtime);
        for pending in pending.into_values() {
            let _ = pending.response.send(Err(PeopleError::Unavailable));
        }
        self.publish();
    }

    fn populate_saved_people(&self) {
        let store = self
            .store
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ensure_saved_profiles(&store, &mut runtime);
    }

    fn persist(&self) -> Result<(), PeopleError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let store = self
            .store
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        fs::write(&self.path, serde_json::to_vec_pretty(&*store)?)?;
        Ok(())
    }

    fn publish(&self) {
        let _ = self.app.emit(PEOPLE_EVENT, self.snapshot());
    }
}

fn ensure_saved_profiles(store: &PeopleStore, runtime: &mut PeopleRuntime) {
    for username in store
        .favorites
        .iter()
        .chain(store.blocked.iter())
        .chain(store.ignored.iter())
        .chain(store.recent.iter().map(|recent| &recent.username))
    {
        let key = person_key(username);
        if runtime.profiles.contains_key(&key) {
            continue;
        }
        if runtime.profiles.len() >= MAX_RUNTIME_PROFILES {
            break;
        }
        let recent_at = store
            .recent
            .iter()
            .find(|recent| recent.username.eq_ignore_ascii_case(username))
            .map(|recent| recent.interacted_at_ms)
            .unwrap_or_default();
        runtime.profiles.insert(
            key,
            PersonProfile::new(
                username.clone(),
                contains_username(&store.favorites, username),
                contains_username(&store.blocked, username),
                contains_username(&store.ignored, username),
                recent_at,
            ),
        );
    }
}

fn load_store(path: &Path) -> Result<PeopleStore, PeopleError> {
    if !path.exists() {
        return Ok(PeopleStore::default());
    }
    let store: PeopleStore = serde_json::from_slice(&fs::read(path)?)?;
    if store.version != STORE_VERSION {
        return Err(PeopleError::UnsupportedStore);
    }
    Ok(store)
}

fn valid_username(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' '))
    .then(|| value.to_owned())
}

fn contains_username(values: &[String], username: &str) -> bool {
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(username))
}

fn person_key(username: &str) -> String {
    username.trim().to_ascii_lowercase()
}

fn status_rank(status: PersonStatus) -> u8 {
    match status {
        PersonStatus::Online => 3,
        PersonStatus::Away => 2,
        PersonStatus::Offline => 1,
        PersonStatus::Unknown => 0,
    }
}

fn clean_description(value: String) -> Option<String> {
    let value = value.replace('\0', "").trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn picture_data_url(bytes: &[u8]) -> Option<String> {
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return None;
    };
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
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
pub enum PeopleError {
    #[error("Choose a valid Soulseek username.")]
    InvalidUsername,
    #[error("Connect to Soulseek before opening a live user profile.")]
    Unavailable,
    #[error("{0}")]
    Request(String),
    #[error("The people configuration was created by an unsupported Flipside version.")]
    UnsupportedStore,
    #[error("Could not read or save people data: {0}")]
    Io(#[from] std::io::Error),
    #[error("Could not read or save people data: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_profile_picture_accepts_only_safe_raster_formats() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(picture_data_url(&png)
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(picture_data_url(b"<svg onload=alert(1)>").is_none());
    }

    #[test]
    fn usernames_are_bounded_and_ascii() {
        assert_eq!(
            valid_username("  midnight-listener  ").as_deref(),
            Some("midnight-listener")
        );
        assert!(valid_username("").is_none());
        assert!(valid_username("listener\nname").is_none());
        assert!(valid_username(&"x".repeat(101)).is_none());
    }

    #[test]
    fn persisted_people_cannot_overfill_the_runtime_cache() {
        let store = PeopleStore {
            version: STORE_VERSION,
            favorites: (0..MAX_RUNTIME_PROFILES + 20)
                .map(|index| format!("listener-{index}"))
                .collect(),
            blocked: Vec::new(),
            ignored: Vec::new(),
            recent: Vec::new(),
        };
        let mut runtime = PeopleRuntime::default();

        ensure_saved_profiles(&store, &mut runtime);

        assert_eq!(runtime.profiles.len(), MAX_RUNTIME_PROFILES);
    }

    #[test]
    fn older_people_store_defaults_to_no_ignored_users() {
        let store: PeopleStore =
            serde_json::from_str(r#"{"version":1,"favorites":[],"blocked":[],"recent":[]}"#)
                .unwrap();

        assert!(store.ignored.is_empty());
    }
}
