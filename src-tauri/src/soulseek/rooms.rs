use super::protocol::{RoomJoin, RoomListing, RoomMemberData, UserStats};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;

pub const ROOMS_EVENT: &str = "flipside://soulseek-rooms";
pub const MAX_ROOM_MESSAGE_BYTES: usize = 4 * 1024;
const STORE_VERSION: u32 = 1;
const MAX_STORED_ROOMS: usize = 64;
const MAX_MESSAGES_PER_ROOM: usize = 250;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RoomMessage {
    pub id: String,
    pub room: String,
    pub username: String,
    pub body: String,
    pub sent_at_ms: u64,
    pub own: bool,
    pub unread: bool,
    pub mention: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RoomMember {
    pub username: String,
    pub status: u32,
    pub average_speed: u32,
    pub upload_count: u32,
    pub shared_file_count: u32,
    pub shared_directory_count: u32,
    pub slots_free: bool,
    pub country_code: Option<String>,
}

impl From<RoomMemberData> for RoomMember {
    fn from(value: RoomMemberData) -> Self {
        Self {
            username: value.username,
            status: value.status,
            average_speed: value.average_speed,
            upload_count: value.upload_count,
            shared_file_count: value.shared_file_count,
            shared_directory_count: value.shared_directory_count,
            slots_free: value.slots_free,
            country_code: value.country_code,
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshot {
    pub name: String,
    pub user_count: u32,
    pub joined: bool,
    pub joining: bool,
    pub auto_join: bool,
    pub favorite: bool,
    pub unread_count: u32,
    pub mention_count: u32,
    pub last_message_at_ms: Option<u64>,
    pub messages: Vec<RoomMessage>,
    pub members: Vec<RoomMember>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RoomsSnapshot {
    pub rooms: Vec<RoomSnapshot>,
    pub connected: bool,
    pub unread_count: u32,
    pub mention_count: u32,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRoom {
    name: String,
    auto_join: bool,
    favorite: bool,
    messages: Vec<RoomMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoomsStore {
    version: u32,
    rooms: Vec<StoredRoom>,
}

impl Default for RoomsStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            rooms: Vec::new(),
        }
    }
}

#[derive(Default)]
struct RoomsRuntime {
    connected: bool,
    available: Vec<RoomListing>,
    joined: HashSet<String>,
    joining: HashSet<String>,
    members: HashMap<String, Vec<RoomMember>>,
}

struct RoomsState {
    store: RoomsStore,
    runtime: RoomsRuntime,
}

#[derive(Clone)]
pub struct RoomsHub {
    app: AppHandle,
    path: PathBuf,
    state: Arc<RwLock<RoomsState>>,
}

impl RoomsHub {
    pub fn new(app: AppHandle, path: PathBuf) -> Result<Self, RoomsError> {
        Ok(Self {
            app,
            path: path.clone(),
            state: Arc::new(RwLock::new(RoomsState {
                store: load_store(&path)?,
                runtime: RoomsRuntime::default(),
            })),
        })
    }

    pub fn desired_rooms(&self) -> Vec<String> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .store
            .rooms
            .iter()
            .filter(|room| room.auto_join)
            .map(|room| room.name.clone())
            .collect()
    }

    pub fn connected(&self) -> RoomsSnapshot {
        self.mutate_runtime(|state| {
            state.runtime.connected = true;
            state.runtime.joining = state
                .store
                .rooms
                .iter()
                .filter(|room| room.auto_join)
                .map(|room| room_key(&room.name))
                .collect();
        })
    }

    pub fn disconnected(&self) -> RoomsSnapshot {
        self.mutate_runtime(|state| {
            state.runtime.connected = false;
            state.runtime.joined.clear();
            state.runtime.joining.clear();
            state.runtime.members.clear();
        })
    }

    pub fn update_list(&self, rooms: Vec<RoomListing>) -> RoomsSnapshot {
        self.mutate_runtime(|state| state.runtime.available = rooms)
    }

    pub fn joined(&self, joined: RoomJoin) -> Result<RoomsSnapshot, RoomsError> {
        let room = valid_room_name(&joined.room)?;
        self.mutate_store(|state| {
            stored_room_mut(&mut state.store, &room).auto_join = true;
            let key = room_key(&room);
            state.runtime.joining.remove(&key);
            state.runtime.joined.insert(key.clone());
            state.runtime.members.insert(
                key,
                joined.members.into_iter().map(RoomMember::from).collect(),
            );
        })
    }

    pub fn left(&self, room: &str) -> Result<RoomsSnapshot, RoomsError> {
        let room = valid_room_name(room)?;
        Ok(self.mutate_runtime(|state| {
            let key = room_key(&room);
            state.runtime.joining.remove(&key);
            state.runtime.joined.remove(&key);
            state.runtime.members.remove(&key);
        }))
    }

    pub fn user_joined(
        &self,
        room: &str,
        member: RoomMemberData,
    ) -> Result<RoomsSnapshot, RoomsError> {
        let room = valid_room_name(room)?;
        Ok(self.mutate_runtime(|state| {
            let members = state.runtime.members.entry(room_key(&room)).or_default();
            members.retain(|item| !item.username.eq_ignore_ascii_case(&member.username));
            members.push(member.into());
            sort_members(members);
        }))
    }

    pub fn user_left(&self, room: &str, username: &str) -> Result<RoomsSnapshot, RoomsError> {
        let room = valid_room_name(room)?;
        Ok(self.mutate_runtime(|state| {
            if let Some(members) = state.runtime.members.get_mut(&room_key(&room)) {
                members.retain(|member| !member.username.eq_ignore_ascii_case(username));
            }
        }))
    }

    pub fn update_status(&self, username: &str, status: u32) -> RoomsSnapshot {
        self.mutate_runtime(|state| {
            for members in state.runtime.members.values_mut() {
                if let Some(member) = members
                    .iter_mut()
                    .find(|member| member.username.eq_ignore_ascii_case(username))
                {
                    member.status = status;
                }
            }
        })
    }

    pub fn update_stats(&self, stats: &UserStats) -> RoomsSnapshot {
        self.mutate_runtime(|state| {
            for members in state.runtime.members.values_mut() {
                if let Some(member) = members
                    .iter_mut()
                    .find(|member| member.username.eq_ignore_ascii_case(&stats.username))
                {
                    member.average_speed = stats.average_speed;
                    member.upload_count = stats.upload_count;
                    member.shared_file_count = stats.shared_file_count;
                    member.shared_directory_count = stats.shared_directory_count;
                }
            }
        })
    }

    pub fn record_message(
        &self,
        room: &str,
        username: &str,
        body: &str,
        own_username: &str,
    ) -> Result<RoomsSnapshot, RoomsError> {
        let room = valid_room_name(room)?;
        let username = valid_username(username)?;
        let body = valid_room_message(body)?;
        let own = username.eq_ignore_ascii_case(own_username);
        let mention = !own && contains_mention(&body, own_username);
        self.mutate_store(|state| {
            let stored = stored_room_mut(&mut state.store, &room);
            let sent_at_ms = timestamp_ms();
            let sequence = stored.messages.len();
            stored.messages.push(RoomMessage {
                id: format!("room-{sent_at_ms}-{sequence}"),
                room,
                username,
                body,
                sent_at_ms,
                own,
                unread: !own,
                mention,
            });
            trim_messages(stored);
        })
    }

    fn mutate_runtime(&self, update: impl FnOnce(&mut RoomsState)) -> RoomsSnapshot {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut state);
        let snapshot = snapshot_from(&state);
        drop(state);
        self.publish(&snapshot);
        snapshot
    }

    fn mutate_store(
        &self,
        update: impl FnOnce(&mut RoomsState),
    ) -> Result<RoomsSnapshot, RoomsError> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut state);
        trim_store(&mut state.store);
        persist(&self.path, &state.store)?;
        let snapshot = snapshot_from(&state);
        drop(state);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    fn publish(&self, snapshot: &RoomsSnapshot) {
        let _ = self.app.emit(ROOMS_EVENT, snapshot);
    }
}

fn snapshot_from(state: &RoomsState) -> RoomsSnapshot {
    let mut names = state
        .runtime
        .available
        .iter()
        .map(|room| room.name.clone())
        .chain(state.store.rooms.iter().map(|room| room.name.clone()))
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    let mut rooms = names
        .into_iter()
        .map(|name| {
            let key = room_key(&name);
            let stored = state
                .store
                .rooms
                .iter()
                .find(|room| room.name.eq_ignore_ascii_case(&name));
            let messages = stored.map_or_else(Vec::new, |room| room.messages.clone());
            let unread_count = messages.iter().filter(|message| message.unread).count() as u32;
            let mention_count = messages.iter().filter(|message| message.mention).count() as u32;
            let members = state.runtime.members.get(&key).cloned().unwrap_or_default();
            let listed_count = state
                .runtime
                .available
                .iter()
                .find(|room| room.name.eq_ignore_ascii_case(&name))
                .map_or(0, |room| room.user_count);
            RoomSnapshot {
                name,
                user_count: if members.is_empty() {
                    listed_count
                } else {
                    u32::try_from(members.len()).unwrap_or(u32::MAX)
                },
                joined: state.runtime.joined.contains(&key),
                joining: state.runtime.joining.contains(&key),
                auto_join: stored.is_some_and(|room| room.auto_join),
                favorite: stored.is_some_and(|room| room.favorite),
                unread_count,
                mention_count,
                last_message_at_ms: messages.last().map(|message| message.sent_at_ms),
                messages,
                members,
            }
        })
        .collect::<Vec<_>>();
    rooms.sort_by_key(|room| {
        (
            !room.joined,
            !room.favorite,
            std::cmp::Reverse(room.user_count),
            room.name.to_ascii_lowercase(),
        )
    });
    RoomsSnapshot {
        connected: state.runtime.connected,
        unread_count: rooms.iter().map(|room| room.unread_count).sum(),
        mention_count: rooms.iter().map(|room| room.mention_count).sum(),
        rooms,
        updated_at_ms: timestamp_ms(),
    }
}

fn stored_room_mut<'a>(store: &'a mut RoomsStore, name: &str) -> &'a mut StoredRoom {
    let index = store
        .rooms
        .iter()
        .position(|room| room.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| {
            store.rooms.push(StoredRoom {
                name: name.to_owned(),
                auto_join: false,
                favorite: false,
                messages: Vec::new(),
            });
            store.rooms.len() - 1
        });
    &mut store.rooms[index]
}

fn trim_messages(room: &mut StoredRoom) {
    if room.messages.len() > MAX_MESSAGES_PER_ROOM {
        room.messages
            .drain(..room.messages.len() - MAX_MESSAGES_PER_ROOM);
    }
}

fn trim_store(store: &mut RoomsStore) {
    for room in &mut store.rooms {
        trim_messages(room);
    }
    if store.rooms.len() > MAX_STORED_ROOMS {
        store.rooms.sort_by_key(|room| {
            (
                !room.auto_join,
                !room.favorite,
                std::cmp::Reverse(room.messages.last().map_or(0, |message| message.sent_at_ms)),
            )
        });
        store.rooms.truncate(MAX_STORED_ROOMS);
    }
}

fn sort_members(members: &mut [RoomMember]) {
    members.sort_by_key(|member| {
        (
            std::cmp::Reverse(member.status),
            member.username.to_ascii_lowercase(),
        )
    });
}

pub fn valid_room_name(value: &str) -> Result<String, RoomsError> {
    let value = value.trim();
    let valid = !value.is_empty()
        && value.len() <= 24
        && value.is_ascii()
        && !value.contains("  ")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ');
    valid
        .then(|| value.to_owned())
        .ok_or(RoomsError::InvalidRoom)
}

pub fn valid_room_message(value: &str) -> Result<String, RoomsError> {
    let value = value.replace('\0', "").trim().to_owned();
    if value.is_empty() || value.len() > MAX_ROOM_MESSAGE_BYTES {
        return Err(RoomsError::InvalidMessage);
    }
    Ok(value)
}

fn valid_username(value: &str) -> Result<String, RoomsError> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' '))
    .then(|| value.to_owned())
    .ok_or(RoomsError::InvalidUsername)
}

fn contains_mention(message: &str, username: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let username = username.to_ascii_lowercase();
    message.match_indices(&username).any(|(index, _)| {
        let before = index
            .checked_sub(1)
            .and_then(|value| message.as_bytes().get(value));
        let after = message.as_bytes().get(index + username.len());
        before.is_none_or(|byte| !byte.is_ascii_alphanumeric())
            && after.is_none_or(|byte| !byte.is_ascii_alphanumeric())
    })
}

fn room_key(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn load_store(path: &Path) -> Result<RoomsStore, RoomsError> {
    if !path.exists() {
        return Ok(RoomsStore::default());
    }
    let mut store: RoomsStore = serde_json::from_slice(&fs::read(path)?)?;
    if store.version != STORE_VERSION {
        return Err(RoomsError::UnsupportedStore);
    }
    trim_store(&mut store);
    Ok(store)
}

fn persist(path: &Path, store: &RoomsStore) -> Result<(), RoomsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(store)?)?;
    Ok(())
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
pub enum RoomsError {
    #[error("Enter a public room name using 1–24 ASCII characters and single spaces.")]
    InvalidRoom,
    #[error("Enter a room message between 1 and {MAX_ROOM_MESSAGE_BYTES} bytes.")]
    InvalidMessage,
    #[error("The room message contained an invalid username.")]
    InvalidUsername,
    #[error("The room history was created by an unsupported Flipside version.")]
    UnsupportedStore,
    #[error("Could not read or save room state: {0}")]
    Io(#[from] std::io::Error),
    #[error("Could not read or save room state: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_names_and_messages_are_bounded() {
        assert_eq!(valid_room_name("  indie  ").unwrap(), "indie");
        assert!(valid_room_name("").is_err());
        assert!(valid_room_name("two  spaces").is_err());
        assert!(valid_room_name(&"x".repeat(25)).is_err());
        assert!(valid_room_name("blå").is_err());
        assert_eq!(valid_room_message("  hello  ").unwrap(), "hello");
        assert!(valid_room_message(&"x".repeat(MAX_ROOM_MESSAGE_BYTES + 1)).is_err());
    }

    #[test]
    fn mentions_require_username_boundaries() {
        assert!(contains_mention("hello SignalLevel!", "signallevel"));
        assert!(contains_mention(
            "@signallevel this is ready",
            "SignalLevel"
        ));
        assert!(!contains_mention("signalleveler", "signallevel"));
    }

    #[test]
    fn room_history_is_bounded_without_dropping_join_preferences() {
        let mut store = RoomsStore::default();
        let room = stored_room_mut(&mut store, "ambient");
        room.auto_join = true;
        for index in 0..(MAX_MESSAGES_PER_ROOM + 5) {
            room.messages.push(RoomMessage {
                id: index.to_string(),
                room: "ambient".to_owned(),
                username: "listener".to_owned(),
                body: "signal".to_owned(),
                sent_at_ms: index as u64,
                own: false,
                unread: true,
                mention: false,
            });
        }
        trim_store(&mut store);
        assert_eq!(store.rooms[0].messages.len(), MAX_MESSAGES_PER_ROOM);
        assert!(store.rooms[0].auto_join);
    }
}
