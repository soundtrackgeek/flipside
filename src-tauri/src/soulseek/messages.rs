use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;

pub const MESSAGES_EVENT: &str = "flipside://soulseek-messages";
const STORE_VERSION: u32 = 1;
const MAX_CONVERSATIONS: usize = 100;
const MAX_MESSAGES_PER_CONVERSATION: usize = 500;
pub const MAX_PRIVATE_MESSAGE_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MessageDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MessageDelivery {
    Received,
    Queued,
    #[default]
    Sent,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PrivateMessage {
    pub id: String,
    pub server_id: Option<u32>,
    pub username: String,
    pub body: String,
    pub direction: MessageDirection,
    pub sent_at_ms: u64,
    pub unread: bool,
    #[serde(default)]
    pub delivery: MessageDelivery,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PrivateConversation {
    pub username: String,
    pub messages: Vec<PrivateMessage>,
    pub unread_count: u32,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MessagesSnapshot {
    pub conversations: Vec<PrivateConversation>,
    pub unread_count: u32,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagesStore {
    version: u32,
    conversations: Vec<PrivateConversation>,
}

impl Default for MessagesStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            conversations: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct MessagesHub {
    app: AppHandle,
    path: PathBuf,
    store: Arc<RwLock<MessagesStore>>,
}

impl MessagesHub {
    pub fn new(app: AppHandle, path: PathBuf) -> Result<Self, MessagesError> {
        Ok(Self {
            app,
            path: path.clone(),
            store: Arc::new(RwLock::new(load_store(&path)?)),
        })
    }

    pub fn record_incoming(
        &self,
        server_id: u32,
        timestamp_seconds: u32,
        username: &str,
        body: &str,
    ) -> Result<MessagesSnapshot, MessagesError> {
        let username = valid_username(username).ok_or(MessagesError::InvalidUsername)?;
        let body = valid_message(body)?;
        let mut store = self
            .store
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if store.conversations.iter().any(|conversation| {
            conversation
                .messages
                .iter()
                .any(|message| message.server_id == Some(server_id))
        }) {
            return Ok(snapshot_from(&store));
        }
        let sent_at_ms = u64::from(timestamp_seconds).saturating_mul(1_000);
        let conversation = conversation_mut(&mut store, &username);
        conversation.messages.push(PrivateMessage {
            id: format!("server-{server_id}"),
            server_id: Some(server_id),
            username,
            body,
            direction: MessageDirection::Incoming,
            sent_at_ms,
            unread: true,
            delivery: MessageDelivery::Received,
            error: None,
        });
        conversation.unread_count = conversation.unread_count.saturating_add(1);
        conversation.updated_at_ms = sent_at_ms.max(timestamp_ms());
        trim_conversation(conversation);
        sort_and_trim(&mut store);
        persist(&self.path, &store)?;
        let snapshot = snapshot_from(&store);
        drop(store);
        self.publish(&snapshot);
        Ok(snapshot)
    }

    pub fn fail_queued(&self, error: &str) -> Result<MessagesSnapshot, MessagesError> {
        let mut store = self
            .store
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let error = clean_error(error);
        let mut changed = false;
        for conversation in &mut store.conversations {
            for message in &mut conversation.messages {
                if message.direction == MessageDirection::Outgoing
                    && message.delivery == MessageDelivery::Queued
                {
                    message.delivery = MessageDelivery::Failed;
                    message.error = Some(error.clone());
                    changed = true;
                }
            }
        }
        if changed {
            persist(&self.path, &store)?;
        }
        let snapshot = snapshot_from(&store);
        drop(store);
        if changed {
            self.publish(&snapshot);
        }
        Ok(snapshot)
    }

    fn publish(&self, snapshot: &MessagesSnapshot) {
        let _ = self.app.emit(MESSAGES_EVENT, snapshot);
    }
}

fn conversation_mut<'a>(
    store: &'a mut MessagesStore,
    username: &str,
) -> &'a mut PrivateConversation {
    let index = store
        .conversations
        .iter()
        .position(|conversation| conversation.username.eq_ignore_ascii_case(username))
        .unwrap_or_else(|| {
            store.conversations.push(PrivateConversation {
                username: username.to_owned(),
                messages: Vec::new(),
                unread_count: 0,
                updated_at_ms: timestamp_ms(),
            });
            store.conversations.len() - 1
        });
    &mut store.conversations[index]
}

fn trim_conversation(conversation: &mut PrivateConversation) {
    if conversation.messages.len() > MAX_MESSAGES_PER_CONVERSATION {
        let remove = conversation.messages.len() - MAX_MESSAGES_PER_CONVERSATION;
        conversation.messages.drain(..remove);
        conversation.unread_count = conversation
            .messages
            .iter()
            .filter(|message| message.unread)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
    }
}

fn sort_and_trim(store: &mut MessagesStore) {
    store
        .conversations
        .sort_by_key(|conversation| std::cmp::Reverse(conversation.updated_at_ms));
    store.conversations.truncate(MAX_CONVERSATIONS);
}

fn snapshot_from(store: &MessagesStore) -> MessagesSnapshot {
    MessagesSnapshot {
        conversations: store.conversations.clone(),
        unread_count: store
            .conversations
            .iter()
            .map(|conversation| conversation.unread_count)
            .fold(0_u32, u32::saturating_add),
        updated_at_ms: timestamp_ms(),
    }
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

pub fn valid_message(value: &str) -> Result<String, MessagesError> {
    let value = value.replace('\0', "").trim().to_owned();
    if value.is_empty() || value.len() > MAX_PRIVATE_MESSAGE_BYTES {
        return Err(MessagesError::InvalidMessage);
    }
    Ok(value)
}

fn clean_error(value: &str) -> String {
    value
        .replace(['\r', '\n', '\0'], " ")
        .chars()
        .take(500)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn load_store(path: &Path) -> Result<MessagesStore, MessagesError> {
    if !path.exists() {
        return Ok(MessagesStore::default());
    }
    let store: MessagesStore = serde_json::from_slice(&fs::read(path)?)?;
    if store.version != STORE_VERSION {
        return Err(MessagesError::UnsupportedStore);
    }
    Ok(store)
}

fn persist(path: &Path, store: &MessagesStore) -> Result<(), MessagesError> {
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
pub enum MessagesError {
    #[error("Choose a valid Soulseek username.")]
    InvalidUsername,
    #[error("Enter a private message between 1 and {MAX_PRIVATE_MESSAGE_BYTES} bytes.")]
    InvalidMessage,
    #[error("The private-message history was created by an unsupported Flipside version.")]
    UnsupportedStore,
    #[error("Could not read or save private messages: {0}")]
    Io(#[from] std::io::Error),
    #[error("Could not read or save private messages: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_messages_are_trimmed_and_bounded() {
        assert_eq!(valid_message("  hello  ").unwrap(), "hello");
        assert!(valid_message("  ").is_err());
        assert!(valid_message(&"x".repeat(MAX_PRIVATE_MESSAGE_BYTES + 1)).is_err());
        assert_eq!(clean_error("  first\r\nsecond\0  "), "first  second");
    }

    #[test]
    fn older_messages_default_to_sent_delivery() {
        let message: PrivateMessage = serde_json::from_str(
            r#"{"id":"local-1","serverId":null,"username":"listener","body":"hello","direction":"outgoing","sentAtMs":1,"unread":false}"#,
        )
        .unwrap();

        assert_eq!(message.delivery, MessageDelivery::Sent);
        assert!(message.error.is_none());
    }
}
