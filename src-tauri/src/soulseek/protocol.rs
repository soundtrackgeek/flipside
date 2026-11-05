use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::{
    io::{Read, Write},
    net::Ipv4Addr,
    string::FromUtf8Error,
};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

pub const LOGIN_CODE: u32 = 1;
pub const SET_WAIT_PORT_CODE: u32 = 2;
pub const GET_PEER_ADDRESS_CODE: u32 = 3;
pub const WATCH_USER_CODE: u32 = 5;
pub const USER_STATUS_CODE: u32 = 7;
pub const SAY_CHATROOM_CODE: u32 = 13;
pub const JOIN_ROOM_CODE: u32 = 14;
pub const LEAVE_ROOM_CODE: u32 = 15;
pub const USER_JOINED_ROOM_CODE: u32 = 16;
pub const USER_LEFT_ROOM_CODE: u32 = 17;
pub const CONNECT_TO_PEER_CODE: u32 = 18;
pub const MESSAGE_USER_CODE: u32 = 22;
pub const MESSAGE_ACKED_CODE: u32 = 23;
pub const FILE_SEARCH_CODE: u32 = 26;
pub const SET_STATUS_CODE: u32 = 28;
pub const SERVER_PING_CODE: u32 = 32;
pub const SHARED_COUNTS_CODE: u32 = 35;
pub const USER_STATS_CODE: u32 = 36;
pub const RELOGGED_CODE: u32 = 41;
pub const USER_INTERESTS_CODE: u32 = 57;
pub const ROOM_LIST_CODE: u32 = 64;
pub const HAVE_NO_PARENT_CODE: u32 = 71;
pub const EMBEDDED_MESSAGE_CODE: u32 = 93;
pub const ACCEPT_CHILDREN_CODE: u32 = 100;
pub const POSSIBLE_PARENTS_CODE: u32 = 102;
pub const BRANCH_LEVEL_CODE: u32 = 126;
pub const BRANCH_ROOT_CODE: u32 = 127;
pub const RESET_DISTRIBUTED_CODE: u32 = 130;
pub const CANT_CONNECT_TO_PEER_CODE: u32 = 1001;
pub const SHARED_FILE_LIST_REQUEST_CODE: u32 = 4;
pub const SHARED_FILE_LIST_RESPONSE_CODE: u32 = 5;
pub const FILE_SEARCH_RESPONSE_CODE: u32 = 9;
pub const USER_INFO_REQUEST_CODE: u32 = 15;
pub const USER_INFO_RESPONSE_CODE: u32 = 16;
pub const FOLDER_CONTENTS_REQUEST_CODE: u32 = 36;
pub const FOLDER_CONTENTS_RESPONSE_CODE: u32 = 37;
pub const TRANSFER_REQUEST_CODE: u32 = 40;
pub const TRANSFER_RESPONSE_CODE: u32 = 41;
pub const QUEUE_UPLOAD_CODE: u32 = 43;
pub const PLACE_IN_QUEUE_RESPONSE_CODE: u32 = 44;
pub const UPLOAD_FAILED_CODE: u32 = 46;
pub const UPLOAD_DENIED_CODE: u32 = 50;
pub const PLACE_IN_QUEUE_REQUEST_CODE: u32 = 51;
pub const DISTRIBUTED_SEARCH_CODE: u8 = 3;
pub const DISTRIBUTED_BRANCH_LEVEL_CODE: u8 = 4;
pub const DISTRIBUTED_BRANCH_ROOT_CODE: u8 = 5;
pub const EXPERIMENTAL_MAJOR_VERSION: u32 = 177;
pub const FOREVER_MINOR_VERSION: u32 = 3;
const MAX_MESSAGE_LENGTH: usize = 16 * 1024 * 1024;
const MAX_PEER_MESSAGE_LENGTH: usize = 64 * 1024 * 1024;
const MAX_PROFILE_MESSAGE_LENGTH: usize = 3 * 1024 * 1024;
const MAX_PEER_INIT_LENGTH: usize = 64 * 1024;
const MAX_DISTRIBUTED_MESSAGE_LENGTH: usize = 16 * 1024;
const MAX_POSSIBLE_PARENTS: usize = 10;
const MAX_NETWORK_USERNAME_BYTES: usize = 100;
const MAX_DISTRIBUTED_QUERY_BYTES: usize = 250;
const MAX_DECOMPRESSED_SEARCH_LENGTH: usize = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_FOLDER_LENGTH: usize = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_SHARE_LENGTH: usize = 256 * 1024 * 1024;
const MAX_RESULTS_PER_RESPONSE: usize = 20_000;
const MAX_FOLDERS_PER_RESPONSE: usize = 5_000;
const MAX_FILES_PER_FOLDER: usize = 20_000;
const MAX_FOLDER_FILES_TOTAL: usize = 50_000;
const MAX_SHARE_DIRECTORIES: usize = 100_000;
const MAX_SHARE_FILES_TOTAL: usize = 500_000;
const MAX_FILE_ATTRIBUTES: usize = 64;
const MAX_PROFILE_DESCRIPTION_BYTES: usize = 8 * 1024;
const MAX_PROFILE_PICTURE_BYTES: usize = 2 * 1024 * 1024;
const MAX_USER_INTERESTS: usize = 500;
const MAX_USER_INTEREST_BYTES: usize = 250;
const MAX_PRIVATE_MESSAGE_BYTES: usize = 8 * 1024;
const MAX_ROOM_NAME_BYTES: usize = 24;
const MAX_ROOM_MESSAGE_BYTES: usize = 4 * 1024;
const MAX_ROOM_LIST: usize = 5_000;
const MAX_ROOM_MEMBERS: usize = 5_000;

#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub code: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoginResponse {
    Accepted {
        greeting: String,
        supporter: bool,
    },
    Rejected {
        reason: String,
        detail: Option<String>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ConnectToPeer {
    pub username: String,
    pub connection_type: String,
    pub address: Ipv4Addr,
    pub port: u32,
    pub token: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PeerAddress {
    pub username: String,
    pub address: Ipv4Addr,
    pub port: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchedUser {
    pub username: String,
    pub exists: bool,
    pub status: u32,
    pub average_speed: u32,
    pub upload_count: u32,
    pub shared_file_count: u32,
    pub shared_directory_count: u32,
    pub country_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserStats {
    pub username: String,
    pub average_speed: u32,
    pub upload_count: u32,
    pub shared_file_count: u32,
    pub shared_directory_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomListing {
    pub name: String,
    pub user_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomMemberData {
    pub username: String,
    pub status: u32,
    pub average_speed: u32,
    pub upload_count: u32,
    pub shared_file_count: u32,
    pub shared_directory_count: u32,
    pub slots_free: bool,
    pub country_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomJoin {
    pub room: String,
    pub members: Vec<RoomMemberData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomChatMessage {
    pub room: String,
    pub username: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserInterests {
    pub username: String,
    pub likes: Vec<String>,
    pub hates: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserInfoResponse {
    pub description: String,
    pub picture: Option<Vec<u8>>,
    pub upload_slots: u32,
    pub queue_size: u32,
    pub slots_free: bool,
    pub upload_permission: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentCandidate {
    pub username: String,
    pub address: Ipv4Addr,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistributedFrame {
    pub code: u8,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistributedSearch {
    pub username: String,
    pub token: u32,
    pub query: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateMessage {
    pub id: u32,
    pub timestamp_seconds: u32,
    pub username: String,
    pub message: String,
    pub is_new: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PeerInit {
    PierceFirewall {
        token: u32,
    },
    Peer {
        username: String,
        connection_type: String,
        token: u32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct TransferRequest {
    pub direction: u32,
    pub token: u32,
    pub filename: String,
    pub size_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchFile {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResponse {
    pub username: String,
    pub token: u32,
    pub files: Vec<SearchFile>,
    pub slot_free: bool,
    pub average_speed: u32,
    pub queue_length: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderFile {
    pub filename: String,
    pub size_bytes: u64,
    pub extension: String,
    pub bitrate: Option<u32>,
    pub duration_seconds: Option<u32>,
    pub vbr: Option<bool>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderListing {
    pub directory: String,
    pub files: Vec<FolderFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderContentsResponse {
    pub token: u32,
    pub requested_folder: String,
    pub folders: Vec<FolderListing>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareListing {
    pub directory: String,
    pub files: Vec<FolderFile>,
    pub is_private: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedFileListResponse {
    pub directories: Vec<ShareListing>,
}

pub fn login_frame(username: &str, password: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    push_string(&mut payload, username);
    push_string(&mut payload, password);
    push_u32(&mut payload, EXPERIMENTAL_MAJOR_VERSION);
    let digest_input = Zeroizing::new(format!("{username}{password}"));
    push_string(
        &mut payload,
        &format!("{:x}", md5_legacy::compute(digest_input.as_bytes())),
    );
    push_u32(&mut payload, FOREVER_MINOR_VERSION);
    encode_message(LOGIN_CODE, &payload)
}

pub fn set_wait_port_frame(port: u16) -> Vec<u8> {
    encode_message(SET_WAIT_PORT_CODE, &u32::from(port).to_le_bytes())
}

pub fn watch_user_frame(username: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(username.len() + 4);
    push_string(&mut payload, username);
    encode_message(WATCH_USER_CODE, &payload)
}

pub fn message_acked_frame(id: u32) -> Vec<u8> {
    encode_message(MESSAGE_ACKED_CODE, &id.to_le_bytes())
}

pub fn room_list_frame() -> Vec<u8> {
    encode_message(ROOM_LIST_CODE, &[])
}

pub fn join_room_frame(room: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(room.len() + 8);
    push_string(&mut payload, room);
    push_u32(&mut payload, 0);
    encode_message(JOIN_ROOM_CODE, &payload)
}

pub fn have_no_parent_frame(has_no_parent: bool) -> Vec<u8> {
    encode_message(HAVE_NO_PARENT_CODE, &[u8::from(has_no_parent)])
}

pub fn accept_children_frame(accept: bool) -> Vec<u8> {
    encode_message(ACCEPT_CHILDREN_CODE, &[u8::from(accept)])
}

pub fn branch_level_frame(level: u32) -> Vec<u8> {
    encode_message(BRANCH_LEVEL_CODE, &level.to_le_bytes())
}

pub fn branch_root_frame(username: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(username.len() + 4);
    push_string(&mut payload, username);
    encode_message(BRANCH_ROOT_CODE, &payload)
}

pub fn file_search_frame(token: u32, query: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(query.len() + 8);
    push_u32(&mut payload, token);
    push_string(&mut payload, query);
    encode_message(FILE_SEARCH_CODE, &payload)
}

pub fn shared_file_list_request_frame() -> Vec<u8> {
    encode_message(SHARED_FILE_LIST_REQUEST_CODE, &[])
}

pub fn user_info_request_frame() -> Vec<u8> {
    encode_message(USER_INFO_REQUEST_CODE, &[])
}

pub fn user_info_response_frame(
    description: &str,
    picture: Option<&[u8]>,
    upload_slots: u32,
    queue_size: u32,
    slots_free: bool,
    upload_permission: u32,
) -> Result<Vec<u8>, ProtocolError> {
    if description.len() > MAX_PROFILE_DESCRIPTION_BYTES
        || picture.is_some_and(|bytes| bytes.len() > MAX_PROFILE_PICTURE_BYTES)
    {
        return Err(ProtocolError::InvalidUserInfo);
    }
    let mut payload = Vec::new();
    push_string(&mut payload, description);
    payload.push(u8::from(picture.is_some()));
    if let Some(picture) = picture {
        push_u32(&mut payload, checked_count(picture.len())?);
        payload.extend_from_slice(picture);
    }
    push_u32(&mut payload, upload_slots);
    push_u32(&mut payload, queue_size);
    payload.push(u8::from(slots_free));
    push_u32(&mut payload, upload_permission);
    Ok(encode_message(USER_INFO_RESPONSE_CODE, &payload))
}

pub fn folder_contents_request_frame(token: u32, folder: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(folder.len() + 8);
    push_u32(&mut payload, token);
    push_string(&mut payload, folder);
    encode_message(FOLDER_CONTENTS_REQUEST_CODE, &payload)
}

pub fn get_peer_address_frame(username: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(username.len() + 4);
    push_string(&mut payload, username);
    encode_message(GET_PEER_ADDRESS_CODE, &payload)
}

pub fn connect_to_peer_frame(token: u32, username: &str, connection_type: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(username.len() + connection_type.len() + 12);
    push_u32(&mut payload, token);
    push_string(&mut payload, username);
    push_string(&mut payload, connection_type);
    encode_message(CONNECT_TO_PEER_CODE, &payload)
}

pub fn peer_init_frame(username: &str, connection_type: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(username.len() + connection_type.len() + 13);
    push_string(&mut payload, username);
    push_string(&mut payload, connection_type);
    push_u32(&mut payload, 0);
    let mut frame = Vec::with_capacity(payload.len() + 5);
    push_u32(
        &mut frame,
        u32::try_from(payload.len() + 1).expect("peer init length fits in u32"),
    );
    frame.push(1);
    frame.extend(payload);
    frame
}

pub fn queue_upload_frame(filename: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(filename.len() + 4);
    push_string(&mut payload, filename);
    encode_message(QUEUE_UPLOAD_CODE, &payload)
}

pub fn place_in_queue_request_frame(filename: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(filename.len() + 4);
    push_string(&mut payload, filename);
    encode_message(PLACE_IN_QUEUE_REQUEST_CODE, &payload)
}

pub fn transfer_response_frame(token: u32, allowed: bool, reason: Option<&str>) -> Vec<u8> {
    let mut payload = Vec::new();
    push_u32(&mut payload, token);
    payload.push(u8::from(allowed));
    if !allowed {
        push_string(&mut payload, reason.unwrap_or("Cancelled"));
    }
    encode_message(TRANSFER_RESPONSE_CODE, &payload)
}

pub fn transfer_request_frame(token: u32, filename: &str, size_bytes: u64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(filename.len() + 20);
    push_u32(&mut payload, 1);
    push_u32(&mut payload, token);
    push_string(&mut payload, filename);
    push_u64(&mut payload, size_bytes);
    encode_message(TRANSFER_REQUEST_CODE, &payload)
}

pub fn place_in_queue_response_frame(filename: &str, position: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(filename.len() + 8);
    push_string(&mut payload, filename);
    push_u32(&mut payload, position);
    encode_message(PLACE_IN_QUEUE_RESPONSE_CODE, &payload)
}

pub fn upload_denied_frame(filename: &str, reason: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(filename.len() + reason.len() + 8);
    push_string(&mut payload, filename);
    push_string(&mut payload, reason);
    encode_message(UPLOAD_DENIED_CODE, &payload)
}

pub fn file_search_response_frame(
    username: &str,
    token: u32,
    files: &[SearchFile],
    slot_free: bool,
    average_speed: u32,
    queue_length: u32,
) -> Result<Vec<u8>, ProtocolError> {
    let mut payload = Vec::new();
    push_string(&mut payload, username);
    push_u32(&mut payload, token);
    push_search_files(&mut payload, files.iter().filter(|file| !file.is_private))?;
    payload.push(u8::from(slot_free));
    push_u32(&mut payload, average_speed);
    push_u32(&mut payload, queue_length);
    push_u32(&mut payload, 0);
    push_search_files(&mut payload, files.iter().filter(|file| file.is_private))?;
    encode_compressed_message(FILE_SEARCH_RESPONSE_CODE, &payload)
}

pub fn folder_contents_response_frame(
    token: u32,
    requested_folder: &str,
    folders: &[FolderListing],
) -> Result<Vec<u8>, ProtocolError> {
    let mut payload = Vec::new();
    push_u32(&mut payload, token);
    push_string(&mut payload, requested_folder);
    push_u32(&mut payload, checked_count(folders.len())?);
    for folder in folders {
        push_string(&mut payload, &folder.directory);
        push_u32(&mut payload, checked_count(folder.files.len())?);
        for file in &folder.files {
            push_folder_file(&mut payload, file)?;
        }
    }
    encode_compressed_message(FOLDER_CONTENTS_RESPONSE_CODE, &payload)
}

pub fn shared_file_list_response_frame(
    directories: &[ShareListing],
) -> Result<Vec<u8>, ProtocolError> {
    let mut payload = Vec::new();
    push_share_directories(
        &mut payload,
        directories.iter().filter(|directory| !directory.is_private),
    )?;
    push_u32(&mut payload, 0);
    push_share_directories(
        &mut payload,
        directories.iter().filter(|directory| directory.is_private),
    )?;
    encode_compressed_message(SHARED_FILE_LIST_RESPONSE_CODE, &payload)
}

pub fn cant_connect_to_peer_frame(token: u32, username: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(username.len() + 8);
    push_u32(&mut payload, token);
    push_string(&mut payload, username);
    encode_message(CANT_CONNECT_TO_PEER_CODE, &payload)
}

pub fn pierce_firewall_frame(token: u32) -> Vec<u8> {
    let mut frame = Vec::with_capacity(9);
    push_u32(&mut frame, 5);
    frame.push(0);
    push_u32(&mut frame, token);
    frame
}

pub fn set_online_frame() -> Vec<u8> {
    encode_message(SET_STATUS_CODE, &2_i32.to_le_bytes())
}

pub fn shared_counts_frame(directories: u32, files: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8);
    push_u32(&mut payload, directories);
    push_u32(&mut payload, files);
    encode_message(SHARED_COUNTS_CODE, &payload)
}

pub fn parse_server_search_request(frame: &Frame) -> Result<(String, u32, String), ProtocolError> {
    if frame.code != FILE_SEARCH_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: FILE_SEARCH_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    Ok((
        reader.read_string_lossy()?,
        reader.read_u32()?,
        reader.read_string_lossy()?,
    ))
}

pub fn parse_private_message(frame: &Frame) -> Result<PrivateMessage, ProtocolError> {
    if frame.code != MESSAGE_USER_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: MESSAGE_USER_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let id = reader.read_u32()?;
    let timestamp_seconds = reader.read_u32()?;
    let username = reader.read_string_lossy()?;
    let message = reader.read_string_lossy()?;
    let is_new = reader.read_bool()?;
    if username.is_empty()
        || username.len() > MAX_NETWORK_USERNAME_BYTES
        || message.is_empty()
        || message.len() > MAX_PRIVATE_MESSAGE_BYTES
    {
        return Err(ProtocolError::InvalidPrivateMessage);
    }
    Ok(PrivateMessage {
        id,
        timestamp_seconds,
        username,
        message,
        is_new,
    })
}

pub fn parse_possible_parents(frame: &Frame) -> Result<Vec<ParentCandidate>, ProtocolError> {
    if frame.code != POSSIBLE_PARENTS_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: POSSIBLE_PARENTS_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let count = reader.read_u32()? as usize;
    if count > MAX_POSSIBLE_PARENTS {
        return Err(ProtocolError::InvalidCount {
            kind: "possible distributed parents",
            count,
        });
    }

    let mut parents = Vec::with_capacity(count);
    for _ in 0..count {
        let username = reader.read_string_lossy()?;
        let address = reader.read_ipv4()?;
        let port = reader.read_u32()?;
        if username.is_empty()
            || username.len() > MAX_NETWORK_USERNAME_BYTES
            || port == 0
            || port > u16::MAX.into()
        {
            continue;
        }
        parents.push(ParentCandidate {
            username,
            address,
            port: port as u16,
        });
    }
    Ok(parents)
}

pub fn parse_distributed_search(
    frame: &DistributedFrame,
) -> Result<DistributedSearch, ProtocolError> {
    if frame.code != DISTRIBUTED_SEARCH_CODE {
        return Err(ProtocolError::UnexpectedDistributedCode {
            expected: DISTRIBUTED_SEARCH_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let identifier = reader.read_u32()?;
    if identifier != u32::from(b'1') {
        return Err(ProtocolError::InvalidDistributedIdentifier(identifier));
    }
    let username = reader.read_string_lossy()?;
    let token = reader.read_u32()?;
    let query = reader.read_string_lossy()?;
    if username.is_empty()
        || username.len() > MAX_NETWORK_USERNAME_BYTES
        || query.is_empty()
        || query.len() > MAX_DISTRIBUTED_QUERY_BYTES
    {
        return Err(ProtocolError::InvalidDistributedSearch);
    }
    Ok(DistributedSearch {
        username,
        token,
        query,
    })
}

pub fn parse_distributed_branch_level(frame: &DistributedFrame) -> Result<u32, ProtocolError> {
    if frame.code != DISTRIBUTED_BRANCH_LEVEL_CODE {
        return Err(ProtocolError::UnexpectedDistributedCode {
            expected: DISTRIBUTED_BRANCH_LEVEL_CODE,
            actual: frame.code,
        });
    }
    let level = PayloadReader::new(&frame.payload).read_i32()?;
    u32::try_from(level).map_err(|_| ProtocolError::InvalidDistributedBranchLevel(level))
}

pub fn parse_distributed_branch_root(frame: &DistributedFrame) -> Result<String, ProtocolError> {
    if frame.code != DISTRIBUTED_BRANCH_ROOT_CODE {
        return Err(ProtocolError::UnexpectedDistributedCode {
            expected: DISTRIBUTED_BRANCH_ROOT_CODE,
            actual: frame.code,
        });
    }
    let root = PayloadReader::new(&frame.payload).read_string_lossy()?;
    if root.is_empty() || root.len() > MAX_NETWORK_USERNAME_BYTES {
        return Err(ProtocolError::InvalidDistributedBranchRoot);
    }
    Ok(root)
}

pub fn parse_embedded_distributed_search(
    frame: &Frame,
) -> Result<DistributedSearch, ProtocolError> {
    if frame.code != EMBEDDED_MESSAGE_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: EMBEDDED_MESSAGE_CODE,
            actual: frame.code,
        });
    }
    let (&code, payload) = frame
        .payload
        .split_first()
        .ok_or(ProtocolError::TruncatedPayload)?;
    parse_distributed_search(&DistributedFrame {
        code,
        payload: payload.to_vec(),
    })
}

pub fn parse_folder_contents_request(frame: &Frame) -> Result<(u32, String), ProtocolError> {
    if frame.code != FOLDER_CONTENTS_REQUEST_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: FOLDER_CONTENTS_REQUEST_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    Ok((
        reader.read_u32()?,
        reader.read_string_lossy()?.replace('/', "\\"),
    ))
}

pub fn parse_transfer_response(
    frame: &Frame,
) -> Result<(u32, bool, Option<String>), ProtocolError> {
    if frame.code != TRANSFER_RESPONSE_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: TRANSFER_RESPONSE_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let token = reader.read_u32()?;
    let allowed = reader.read_bool()?;
    let reason = if allowed {
        None
    } else {
        Some(reader.read_string_lossy()?)
    };
    Ok((token, allowed, reason))
}

pub fn server_ping_frame() -> Vec<u8> {
    encode_message(SERVER_PING_CODE, &[])
}

pub fn parse_login_response(frame: &Frame) -> Result<LoginResponse, ProtocolError> {
    if frame.code != LOGIN_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: LOGIN_CODE,
            actual: frame.code,
        });
    }

    let mut reader = PayloadReader::new(&frame.payload);
    let accepted = reader.read_bool()?;
    if accepted {
        let greeting = reader.read_string()?;
        let _own_ip = reader.read_u32()?;
        let _password_hash = reader.read_string()?;
        let supporter = reader.read_bool()?;
        Ok(LoginResponse::Accepted {
            greeting,
            supporter,
        })
    } else {
        let reason = reader.read_string()?;
        let detail = if reason == "INVALIDUSERNAME" && reader.remaining() > 0 {
            Some(reader.read_string()?)
        } else {
            None
        };
        Ok(LoginResponse::Rejected { reason, detail })
    }
}

pub fn parse_connect_to_peer(frame: &Frame) -> Result<ConnectToPeer, ProtocolError> {
    if frame.code != CONNECT_TO_PEER_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: CONNECT_TO_PEER_CODE,
            actual: frame.code,
        });
    }

    let mut reader = PayloadReader::new(&frame.payload);
    let username = reader.read_string_lossy()?;
    let connection_type = reader.read_string_lossy()?;
    let address = reader.read_ipv4()?;
    let port = reader.read_u32()?;
    let token = reader.read_u32()?;
    let _privileged = reader.read_bool()?;
    let _obfuscation_type = reader.read_u32()?;
    let _obfuscated_port = reader.read_u32()?;

    Ok(ConnectToPeer {
        username,
        connection_type,
        address,
        port,
        token,
    })
}

pub fn parse_peer_address(frame: &Frame) -> Result<PeerAddress, ProtocolError> {
    if frame.code != GET_PEER_ADDRESS_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: GET_PEER_ADDRESS_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let username = reader.read_string_lossy()?;
    let address = reader.read_ipv4()?;
    let port = reader.read_u32()?;
    if reader.remaining() >= 4 {
        let _obfuscation_type = reader.read_u32()?;
    }
    if reader.remaining() >= 2 {
        let _obfuscated_port = reader.read_u16()?;
    }
    Ok(PeerAddress {
        username,
        address,
        port,
    })
}

pub fn parse_watch_user(frame: &Frame) -> Result<WatchedUser, ProtocolError> {
    if frame.code != WATCH_USER_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: WATCH_USER_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let username = read_network_username(&mut reader)?;
    let exists = reader.read_bool()?;
    if !exists {
        return Ok(WatchedUser {
            username,
            exists,
            status: 0,
            average_speed: 0,
            upload_count: 0,
            shared_file_count: 0,
            shared_directory_count: 0,
            country_code: None,
        });
    }
    let status = reader.read_u32()?;
    let average_speed = reader.read_u32()?;
    let upload_count = reader.read_u32()?;
    let _unknown = reader.read_u32()?;
    let shared_file_count = reader.read_u32()?;
    let shared_directory_count = reader.read_u32()?;
    let country_code = if status != 0 && reader.remaining() >= 4 {
        normalize_country_code(reader.read_string_lossy()?)
    } else {
        None
    };
    Ok(WatchedUser {
        username,
        exists,
        status,
        average_speed,
        upload_count,
        shared_file_count,
        shared_directory_count,
        country_code,
    })
}

pub fn parse_user_status(frame: &Frame) -> Result<(String, u32, bool), ProtocolError> {
    if frame.code != USER_STATUS_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: USER_STATUS_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    Ok((
        read_network_username(&mut reader)?,
        reader.read_u32()?,
        reader.read_bool()?,
    ))
}

pub fn parse_room_list(frame: &Frame) -> Result<Vec<RoomListing>, ProtocolError> {
    expect_code(frame, ROOM_LIST_CODE)?;
    let mut reader = PayloadReader::new(&frame.payload);
    let room_count = bounded_count(reader.read_u32()?, MAX_ROOM_LIST, "rooms")?;
    let mut names = Vec::with_capacity(room_count);
    for _ in 0..room_count {
        names.push(read_room_name(&mut reader)?);
    }
    let count_count = bounded_count(reader.read_u32()?, MAX_ROOM_LIST, "room counts")?;
    let mut counts = Vec::with_capacity(count_count);
    for _ in 0..count_count {
        counts.push(reader.read_u32()?);
    }
    Ok(names
        .into_iter()
        .enumerate()
        .map(|(index, name)| RoomListing {
            name,
            user_count: counts.get(index).copied().unwrap_or_default(),
        })
        .collect())
}

pub fn parse_join_room(frame: &Frame) -> Result<RoomJoin, ProtocolError> {
    expect_code(frame, JOIN_ROOM_CODE)?;
    let mut reader = PayloadReader::new(&frame.payload);
    let room = read_room_name(&mut reader)?;
    let user_count = bounded_count(reader.read_u32()?, MAX_ROOM_MEMBERS, "room users")?;
    let mut usernames = Vec::with_capacity(user_count);
    for _ in 0..user_count {
        usernames.push(read_network_username(&mut reader)?);
    }
    let status_count = bounded_count(reader.read_u32()?, MAX_ROOM_MEMBERS, "room statuses")?;
    let mut statuses = Vec::with_capacity(status_count);
    for _ in 0..status_count {
        statuses.push(reader.read_u32()?);
    }
    let stats_count = bounded_count(reader.read_u32()?, MAX_ROOM_MEMBERS, "room statistics")?;
    let mut statistics = Vec::with_capacity(stats_count);
    for _ in 0..stats_count {
        statistics.push((
            reader.read_u32()?,
            reader.read_u32()?,
            reader.read_u32()?,
            reader.read_u32()?,
            reader.read_u32()?,
        ));
    }
    let slots_count = bounded_count(reader.read_u32()?, MAX_ROOM_MEMBERS, "room slot states")?;
    let mut slots = Vec::with_capacity(slots_count);
    for _ in 0..slots_count {
        slots.push(reader.read_u32()? == 0);
    }
    let country_count = if reader.remaining() >= 4 {
        bounded_count(reader.read_u32()?, MAX_ROOM_MEMBERS, "room countries")?
    } else {
        0
    };
    let mut countries = Vec::with_capacity(country_count);
    for _ in 0..country_count {
        countries.push(normalize_country_code(reader.read_string_lossy()?));
    }
    let members = usernames
        .into_iter()
        .enumerate()
        .map(|(index, username)| {
            let (average_speed, upload_count, _unknown, shared_file_count, shared_directory_count) =
                statistics.get(index).copied().unwrap_or_default();
            RoomMemberData {
                username,
                status: statuses.get(index).copied().unwrap_or_default(),
                average_speed,
                upload_count,
                shared_file_count,
                shared_directory_count,
                slots_free: slots.get(index).copied().unwrap_or(false),
                country_code: countries.get(index).cloned().flatten(),
            }
        })
        .collect();
    Ok(RoomJoin { room, members })
}

pub fn parse_room_chat_message(frame: &Frame) -> Result<RoomChatMessage, ProtocolError> {
    expect_code(frame, SAY_CHATROOM_CODE)?;
    let mut reader = PayloadReader::new(&frame.payload);
    let room = read_room_name(&mut reader)?;
    let username = read_network_username(&mut reader)?;
    let message = reader.read_string_lossy()?;
    if message.is_empty() || message.len() > MAX_ROOM_MESSAGE_BYTES {
        return Err(ProtocolError::InvalidRoomMessage);
    }
    Ok(RoomChatMessage {
        room,
        username,
        message,
    })
}

pub fn parse_leave_room(frame: &Frame) -> Result<String, ProtocolError> {
    expect_code(frame, LEAVE_ROOM_CODE)?;
    read_room_name(&mut PayloadReader::new(&frame.payload))
}

pub fn parse_user_joined_room(frame: &Frame) -> Result<(String, RoomMemberData), ProtocolError> {
    expect_code(frame, USER_JOINED_ROOM_CODE)?;
    let mut reader = PayloadReader::new(&frame.payload);
    let room = read_room_name(&mut reader)?;
    let username = read_network_username(&mut reader)?;
    let status = reader.read_u32()?;
    let average_speed = reader.read_u32()?;
    let upload_count = reader.read_u32()?;
    let _unknown = reader.read_u32()?;
    let shared_file_count = reader.read_u32()?;
    let shared_directory_count = reader.read_u32()?;
    let slots_free = reader.read_u32()? == 0;
    let country_code = normalize_country_code(reader.read_string_lossy()?);
    Ok((
        room,
        RoomMemberData {
            username,
            status,
            average_speed,
            upload_count,
            shared_file_count,
            shared_directory_count,
            slots_free,
            country_code,
        },
    ))
}

pub fn parse_user_left_room(frame: &Frame) -> Result<(String, String), ProtocolError> {
    expect_code(frame, USER_LEFT_ROOM_CODE)?;
    let mut reader = PayloadReader::new(&frame.payload);
    Ok((
        read_room_name(&mut reader)?,
        read_network_username(&mut reader)?,
    ))
}

pub fn parse_user_stats(frame: &Frame) -> Result<UserStats, ProtocolError> {
    if frame.code != USER_STATS_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: USER_STATS_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let username = read_network_username(&mut reader)?;
    let average_speed = reader.read_u32()?;
    let upload_count = reader.read_u32()?;
    let _unknown = reader.read_u32()?;
    Ok(UserStats {
        username,
        average_speed,
        upload_count,
        shared_file_count: reader.read_u32()?,
        shared_directory_count: reader.read_u32()?,
    })
}

pub fn parse_user_interests(frame: &Frame) -> Result<UserInterests, ProtocolError> {
    if frame.code != USER_INTERESTS_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: USER_INTERESTS_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let username = read_network_username(&mut reader)?;
    let likes = read_interests(&mut reader, "liked interests")?;
    let hates = read_interests(&mut reader, "hated interests")?;
    Ok(UserInterests {
        username,
        likes,
        hates,
    })
}

pub fn parse_user_info_response(frame: &Frame) -> Result<UserInfoResponse, ProtocolError> {
    if frame.code != USER_INFO_RESPONSE_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: USER_INFO_RESPONSE_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let description = reader.read_string_lossy()?;
    if description.len() > MAX_PROFILE_DESCRIPTION_BYTES {
        return Err(ProtocolError::InvalidUserInfo);
    }
    let picture = if reader.read_bool()? {
        let length = reader.read_u32()? as usize;
        if length > MAX_PROFILE_PICTURE_BYTES {
            return Err(ProtocolError::InvalidUserInfo);
        }
        Some(reader.read_bytes(length)?.to_vec())
    } else {
        None
    };
    let upload_slots = reader.read_u32()?;
    let queue_size = reader.read_u32()?;
    let slots_free = reader.read_bool()?;
    let upload_permission = (reader.remaining() >= 4)
        .then(|| reader.read_u32())
        .transpose()?;
    Ok(UserInfoResponse {
        description,
        picture,
        upload_slots,
        queue_size,
        slots_free,
        upload_permission,
    })
}

fn read_network_username(reader: &mut PayloadReader<'_>) -> Result<String, ProtocolError> {
    let username = reader.read_string_lossy()?;
    if username.is_empty() || username.len() > MAX_NETWORK_USERNAME_BYTES {
        return Err(ProtocolError::InvalidUserData);
    }
    Ok(username)
}

fn read_room_name(reader: &mut PayloadReader<'_>) -> Result<String, ProtocolError> {
    let room = reader.read_string_lossy()?;
    let valid = !room.is_empty()
        && room.len() <= MAX_ROOM_NAME_BYTES
        && room.is_ascii()
        && !room.starts_with(' ')
        && !room.ends_with(' ')
        && !room.contains("  ")
        && room
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ');
    valid
        .then_some(room)
        .ok_or(ProtocolError::InvalidRoomMessage)
}

fn expect_code(frame: &Frame, expected: u32) -> Result<(), ProtocolError> {
    if frame.code != expected {
        return Err(ProtocolError::UnexpectedCode {
            expected,
            actual: frame.code,
        });
    }
    Ok(())
}

fn bounded_count(count: u32, maximum: usize, kind: &'static str) -> Result<usize, ProtocolError> {
    let count = count as usize;
    if count > maximum {
        return Err(ProtocolError::InvalidCount { kind, count });
    }
    Ok(count)
}

fn normalize_country_code(value: String) -> Option<String> {
    let value = value.trim().to_ascii_uppercase();
    (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_alphabetic())).then_some(value)
}

fn read_interests(
    reader: &mut PayloadReader<'_>,
    kind: &'static str,
) -> Result<Vec<String>, ProtocolError> {
    let count = reader.read_u32()? as usize;
    if count > MAX_USER_INTERESTS {
        return Err(ProtocolError::InvalidCount { kind, count });
    }
    let mut interests = Vec::with_capacity(count);
    for _ in 0..count {
        let interest = reader.read_string_lossy()?;
        if interest.is_empty() || interest.len() > MAX_USER_INTEREST_BYTES {
            return Err(ProtocolError::InvalidUserData);
        }
        interests.push(interest);
    }
    Ok(interests)
}

pub fn parse_cant_connect_token(frame: &Frame) -> Result<u32, ProtocolError> {
    if frame.code != CANT_CONNECT_TO_PEER_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: CANT_CONNECT_TO_PEER_CODE,
            actual: frame.code,
        });
    }
    PayloadReader::new(&frame.payload).read_u32()
}

pub fn parse_peer_init(code: u8, payload: &[u8]) -> Result<PeerInit, ProtocolError> {
    let mut reader = PayloadReader::new(payload);
    match code {
        0 => Ok(PeerInit::PierceFirewall {
            token: reader.read_u32()?,
        }),
        1 => {
            let username = reader.read_string_lossy()?;
            let connection_type = reader.read_string_lossy()?;
            let token = if reader.remaining() >= 4 {
                reader.read_u32()?
            } else {
                0
            };
            Ok(PeerInit::Peer {
                username,
                connection_type,
                token,
            })
        }
        actual => Err(ProtocolError::UnexpectedPeerInitCode(actual)),
    }
}

pub fn parse_transfer_request(frame: &Frame) -> Result<TransferRequest, ProtocolError> {
    if frame.code != TRANSFER_REQUEST_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: TRANSFER_REQUEST_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    let direction = reader.read_u32()?;
    let token = reader.read_u32()?;
    let filename = reader.read_string_lossy()?.replace('/', "\\");
    let size_bytes = if direction == 1 {
        Some(reader.read_u64()?)
    } else {
        None
    };
    Ok(TransferRequest {
        direction,
        token,
        filename,
        size_bytes,
    })
}

pub fn parse_queue_position(frame: &Frame) -> Result<(String, u32), ProtocolError> {
    if frame.code != PLACE_IN_QUEUE_RESPONSE_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: PLACE_IN_QUEUE_RESPONSE_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    Ok((
        reader.read_string_lossy()?.replace('/', "\\"),
        reader.read_u32()?,
    ))
}

pub fn parse_filename(frame: &Frame, expected_code: u32) -> Result<String, ProtocolError> {
    if frame.code != expected_code {
        return Err(ProtocolError::UnexpectedCode {
            expected: expected_code,
            actual: frame.code,
        });
    }
    Ok(PayloadReader::new(&frame.payload)
        .read_string_lossy()?
        .replace('/', "\\"))
}

pub fn parse_upload_denied(frame: &Frame) -> Result<(String, String), ProtocolError> {
    if frame.code != UPLOAD_DENIED_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: UPLOAD_DENIED_CODE,
            actual: frame.code,
        });
    }
    let mut reader = PayloadReader::new(&frame.payload);
    Ok((
        reader.read_string_lossy()?.replace('/', "\\"),
        reader.read_string_lossy()?,
    ))
}

pub fn parse_search_response(frame: &Frame) -> Result<SearchResponse, ProtocolError> {
    if frame.code != FILE_SEARCH_RESPONSE_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: FILE_SEARCH_RESPONSE_CODE,
            actual: frame.code,
        });
    }

    let decoder = ZlibDecoder::new(frame.payload.as_slice());
    let mut payload = Vec::new();
    decoder
        .take((MAX_DECOMPRESSED_SEARCH_LENGTH + 1) as u64)
        .read_to_end(&mut payload)?;
    if payload.len() > MAX_DECOMPRESSED_SEARCH_LENGTH {
        return Err(ProtocolError::DecompressedPayloadTooLarge);
    }

    let mut reader = PayloadReader::new(&payload);
    let username = reader.read_string_lossy()?;
    let token = reader.read_u32()?;
    let mut files = read_search_files(&mut reader, false)?;
    let slot_free = reader.read_bool()?;
    let average_speed = reader.read_u32()?;
    let queue_length = reader.read_u32()?;

    if reader.remaining() >= 4 {
        let _unknown = reader.read_u32()?;
    }
    if reader.remaining() >= 4 {
        files.extend(read_search_files(&mut reader, true)?);
    }

    Ok(SearchResponse {
        username,
        token,
        files,
        slot_free,
        average_speed,
        queue_length,
    })
}

pub fn parse_folder_contents_response(
    frame: &Frame,
) -> Result<FolderContentsResponse, ProtocolError> {
    if frame.code != FOLDER_CONTENTS_RESPONSE_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: FOLDER_CONTENTS_RESPONSE_CODE,
            actual: frame.code,
        });
    }

    let decoder = ZlibDecoder::new(frame.payload.as_slice());
    let mut payload = Vec::new();
    decoder
        .take((MAX_DECOMPRESSED_FOLDER_LENGTH + 1) as u64)
        .read_to_end(&mut payload)?;
    if payload.len() > MAX_DECOMPRESSED_FOLDER_LENGTH {
        return Err(ProtocolError::DecompressedPayloadTooLarge);
    }

    let mut reader = PayloadReader::new(&payload);
    let token = reader.read_u32()?;
    let requested_folder = reader.read_string_lossy()?.replace('/', "\\");
    let folder_count = reader.read_u32()? as usize;
    if folder_count > MAX_FOLDERS_PER_RESPONSE {
        return Err(ProtocolError::InvalidCount {
            kind: "folders",
            count: folder_count,
        });
    }

    let mut total_files = 0_usize;
    let mut folders = Vec::with_capacity(folder_count);
    for _ in 0..folder_count {
        let directory = reader.read_string_lossy()?.replace('/', "\\");
        let file_count = reader.read_u32()? as usize;
        if file_count > MAX_FILES_PER_FOLDER {
            return Err(ProtocolError::InvalidCount {
                kind: "folder files",
                count: file_count,
            });
        }
        total_files = total_files.saturating_add(file_count);
        if total_files > MAX_FOLDER_FILES_TOTAL {
            return Err(ProtocolError::InvalidCount {
                kind: "folder files",
                count: total_files,
            });
        }

        let mut files = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            let _code = reader.read_u8()?;
            let filename = reader.read_string_lossy()?.replace('/', "\\");
            let size_bytes = reader.read_u64()?;
            let supplied_extension = reader.read_string_lossy()?;
            let attribute_count = reader.read_u32()? as usize;
            if attribute_count > MAX_FILE_ATTRIBUTES {
                return Err(ProtocolError::InvalidCount {
                    kind: "file attributes",
                    count: attribute_count,
                });
            }

            let mut bitrate = None;
            let mut duration_seconds = None;
            let mut vbr = None;
            let mut sample_rate = None;
            let mut bit_depth = None;
            for _ in 0..attribute_count {
                let attribute = reader.read_u32()?;
                let value = reader.read_u32()?;
                match attribute {
                    0 => bitrate = Some(value),
                    1 => duration_seconds = Some(value),
                    2 => vbr = Some(value != 0),
                    4 => sample_rate = Some(value),
                    5 => bit_depth = Some(value),
                    _ => {}
                }
            }

            let extension = if supplied_extension.is_empty() {
                filename
                    .rsplit_once('.')
                    .map(|(_, extension)| extension.to_ascii_lowercase())
                    .unwrap_or_default()
            } else {
                supplied_extension.to_ascii_lowercase()
            };
            files.push(FolderFile {
                filename,
                size_bytes,
                extension,
                bitrate,
                duration_seconds,
                vbr,
                sample_rate,
                bit_depth,
            });
        }
        folders.push(FolderListing { directory, files });
    }

    Ok(FolderContentsResponse {
        token,
        requested_folder,
        folders,
    })
}

pub fn parse_shared_file_list_response(
    frame: &Frame,
) -> Result<SharedFileListResponse, ProtocolError> {
    if frame.code != SHARED_FILE_LIST_RESPONSE_CODE {
        return Err(ProtocolError::UnexpectedCode {
            expected: SHARED_FILE_LIST_RESPONSE_CODE,
            actual: frame.code,
        });
    }

    let decoder = ZlibDecoder::new(frame.payload.as_slice());
    let mut payload = Vec::new();
    decoder
        .take((MAX_DECOMPRESSED_SHARE_LENGTH + 1) as u64)
        .read_to_end(&mut payload)?;
    if payload.len() > MAX_DECOMPRESSED_SHARE_LENGTH {
        return Err(ProtocolError::DecompressedPayloadTooLarge);
    }

    let mut reader = PayloadReader::new(&payload);
    let mut total_files = 0_usize;
    let mut directories = read_share_directories(&mut reader, false, &mut total_files)?;
    let _unknown = reader.read_u32()?;
    directories.extend(read_share_directories(&mut reader, true, &mut total_files)?);
    Ok(SharedFileListResponse { directories })
}

fn read_share_directories(
    reader: &mut PayloadReader<'_>,
    is_private: bool,
    total_files: &mut usize,
) -> Result<Vec<ShareListing>, ProtocolError> {
    let count = reader.read_u32()? as usize;
    if count > MAX_SHARE_DIRECTORIES {
        return Err(ProtocolError::InvalidCount {
            kind: "shared directories",
            count,
        });
    }

    let mut directories = Vec::with_capacity(count);
    for _ in 0..count {
        let directory = reader.read_string_lossy()?.replace('/', "\\");
        let file_count = reader.read_u32()? as usize;
        *total_files = (*total_files).saturating_add(file_count);
        if *total_files > MAX_SHARE_FILES_TOTAL {
            return Err(ProtocolError::InvalidCount {
                kind: "shared files",
                count: *total_files,
            });
        }

        let mut files = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            files.push(read_folder_file(reader)?);
        }
        directories.push(ShareListing {
            directory,
            files,
            is_private,
        });
    }
    Ok(directories)
}

fn read_folder_file(reader: &mut PayloadReader<'_>) -> Result<FolderFile, ProtocolError> {
    let _code = reader.read_u8()?;
    let filename = reader.read_string_lossy()?.replace('/', "\\");
    let size_bytes = reader.read_u64()?;
    let supplied_extension = reader.read_string_lossy()?;
    let attribute_count = reader.read_u32()? as usize;
    if attribute_count > MAX_FILE_ATTRIBUTES {
        return Err(ProtocolError::InvalidCount {
            kind: "file attributes",
            count: attribute_count,
        });
    }

    let mut bitrate = None;
    let mut duration_seconds = None;
    let mut vbr = None;
    let mut sample_rate = None;
    let mut bit_depth = None;
    for _ in 0..attribute_count {
        let attribute = reader.read_u32()?;
        let value = reader.read_u32()?;
        match attribute {
            0 => bitrate = Some(value),
            1 => duration_seconds = Some(value),
            2 => vbr = Some(value != 0),
            4 => sample_rate = Some(value),
            5 => bit_depth = Some(value),
            _ => {}
        }
    }
    let extension = if supplied_extension.is_empty() {
        filename
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .unwrap_or_default()
    } else {
        supplied_extension.to_ascii_lowercase()
    };
    Ok(FolderFile {
        filename,
        size_bytes,
        extension,
        bitrate,
        duration_seconds,
        vbr,
        sample_rate,
        bit_depth,
    })
}

fn read_search_files(
    reader: &mut PayloadReader<'_>,
    is_private: bool,
) -> Result<Vec<SearchFile>, ProtocolError> {
    let count = reader.read_u32()? as usize;
    if count > MAX_RESULTS_PER_RESPONSE {
        return Err(ProtocolError::InvalidCount {
            kind: "search results",
            count,
        });
    }

    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let _code = reader.read_u8()?;
        let filename = reader.read_string_lossy()?.replace('/', "\\");
        let size_bytes = reader.read_u64()?;
        let supplied_extension = reader.read_string_lossy()?;
        let attribute_count = reader.read_u32()? as usize;
        if attribute_count > MAX_FILE_ATTRIBUTES {
            return Err(ProtocolError::InvalidCount {
                kind: "file attributes",
                count: attribute_count,
            });
        }

        let mut bitrate = None;
        let mut duration_seconds = None;
        let mut vbr = None;
        let mut sample_rate = None;
        let mut bit_depth = None;
        for _ in 0..attribute_count {
            let attribute = reader.read_u32()?;
            let value = reader.read_u32()?;
            match attribute {
                0 => bitrate = Some(value),
                1 => duration_seconds = Some(value),
                2 => vbr = Some(value != 0),
                4 => sample_rate = Some(value),
                5 => bit_depth = Some(value),
                _ => {}
            }
        }

        let extension = if supplied_extension.is_empty() {
            filename
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_ascii_lowercase())
                .unwrap_or_default()
        } else {
            supplied_extension.to_ascii_lowercase()
        };
        files.push(SearchFile {
            filename,
            size_bytes,
            extension,
            bitrate,
            duration_seconds,
            vbr,
            sample_rate,
            bit_depth,
            is_private,
        });
    }
    Ok(files)
}

pub async fn write_raw_frame<W>(writer: &mut W, frame: &[u8]) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(frame).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_frame<R>(reader: &mut R) -> Result<Frame, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    read_frame_with_limit(reader, MAX_MESSAGE_LENGTH).await
}

pub async fn read_peer_frame<R>(reader: &mut R) -> Result<Frame, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    read_frame_with_limit(reader, MAX_PEER_MESSAGE_LENGTH).await
}

pub async fn read_profile_frame<R>(reader: &mut R) -> Result<Frame, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    read_frame_with_limit(reader, MAX_PROFILE_MESSAGE_LENGTH).await
}

pub async fn read_distributed_frame<R>(reader: &mut R) -> Result<DistributedFrame, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let length = reader.read_u32_le().await? as usize;
    if !(1..=MAX_DISTRIBUTED_MESSAGE_LENGTH).contains(&length) {
        return Err(ProtocolError::InvalidLength(length));
    }
    let code = reader.read_u8().await?;
    let mut payload = vec![0; length - 1];
    reader.read_exact(&mut payload).await?;
    Ok(DistributedFrame { code, payload })
}

async fn read_frame_with_limit<R>(reader: &mut R, maximum: usize) -> Result<Frame, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let length = reader.read_u32_le().await? as usize;
    if !(4..=maximum).contains(&length) {
        return Err(ProtocolError::InvalidLength(length));
    }

    let code = reader.read_u32_le().await?;
    let mut payload = vec![0; length - 4];
    reader.read_exact(&mut payload).await?;
    Ok(Frame { code, payload })
}

pub async fn read_peer_init<R>(reader: &mut R) -> Result<PeerInit, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let length = reader.read_u32_le().await? as usize;
    if !(1..=MAX_PEER_INIT_LENGTH).contains(&length) {
        return Err(ProtocolError::InvalidLength(length));
    }
    let code = reader.read_u8().await?;
    let mut payload = vec![0; length - 1];
    reader.read_exact(&mut payload).await?;
    parse_peer_init(code, &payload)
}

fn encode_message(code: u32, payload: &[u8]) -> Vec<u8> {
    let message_length = 4_usize
        .checked_add(payload.len())
        .and_then(|length| u32::try_from(length).ok())
        .expect("Soulseek message length fits in u32");
    let mut frame = Vec::with_capacity(message_length as usize + 4);
    push_u32(&mut frame, message_length);
    push_u32(&mut frame, code);
    frame.extend_from_slice(payload);
    frame
}

fn encode_compressed_message(code: u32, payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(payload)?;
    Ok(encode_message(code, &encoder.finish()?))
}

fn checked_count(count: usize) -> Result<u32, ProtocolError> {
    u32::try_from(count).map_err(|_| ProtocolError::InvalidCount {
        kind: "encoded values",
        count,
    })
}

fn push_search_files<'a>(
    payload: &mut Vec<u8>,
    files: impl Iterator<Item = &'a SearchFile>,
) -> Result<(), ProtocolError> {
    let files: Vec<_> = files.collect();
    push_u32(payload, checked_count(files.len())?);
    for file in files {
        push_folder_file(
            payload,
            &FolderFile {
                filename: file.filename.clone(),
                size_bytes: file.size_bytes,
                extension: file.extension.clone(),
                bitrate: file.bitrate,
                duration_seconds: file.duration_seconds,
                vbr: file.vbr,
                sample_rate: file.sample_rate,
                bit_depth: file.bit_depth,
            },
        )?;
    }
    Ok(())
}

fn push_share_directories<'a>(
    payload: &mut Vec<u8>,
    directories: impl Iterator<Item = &'a ShareListing>,
) -> Result<(), ProtocolError> {
    let directories: Vec<_> = directories.collect();
    push_u32(payload, checked_count(directories.len())?);
    for directory in directories {
        push_string(payload, &directory.directory);
        push_u32(payload, checked_count(directory.files.len())?);
        for file in &directory.files {
            push_folder_file(payload, file)?;
        }
    }
    Ok(())
}

fn push_folder_file(payload: &mut Vec<u8>, file: &FolderFile) -> Result<(), ProtocolError> {
    payload.push(1);
    push_string(payload, &file.filename);
    push_u64(payload, file.size_bytes);
    push_string(payload, &file.extension);
    let attributes = [
        file.bitrate.map(|value| (0, value)),
        file.duration_seconds.map(|value| (1, value)),
        file.vbr.map(|value| (2, u32::from(value))),
        file.sample_rate.map(|value| (4, value)),
        file.bit_depth.map(|value| (5, value)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    push_u32(payload, checked_count(attributes.len())?);
    for (code, value) in attributes {
        push_u32(payload, code);
        push_u32(payload, value);
    }
    Ok(())
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(buffer: &mut Vec<u8>, value: u64) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_string(buffer: &mut Vec<u8>, value: &str) {
    let length = u32::try_from(value.len()).expect("Soulseek string length fits in u32");
    push_u32(buffer, length);
    buffer.extend_from_slice(value.as_bytes());
}

struct PayloadReader<'a> {
    payload: &'a [u8],
    position: usize,
}

impl<'a> PayloadReader<'a> {
    fn new(payload: &'a [u8]) -> Self {
        Self {
            payload,
            position: 0,
        }
    }

    fn remaining(&self) -> usize {
        self.payload.len().saturating_sub(self.position)
    }

    fn read_bool(&mut self) -> Result<bool, ProtocolError> {
        let value = *self
            .payload
            .get(self.position)
            .ok_or(ProtocolError::TruncatedPayload)?;
        self.position += 1;
        Ok(value != 0)
    }

    fn read_u8(&mut self) -> Result<u8, ProtocolError> {
        let value = *self
            .payload
            .get(self.position)
            .ok_or(ProtocolError::TruncatedPayload)?;
        self.position += 1;
        Ok(value)
    }

    fn read_u32(&mut self) -> Result<u32, ProtocolError> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes(
            bytes.try_into().expect("four byte slice"),
        ))
    }

    fn read_i32(&mut self) -> Result<i32, ProtocolError> {
        let bytes = self.read_bytes(4)?;
        Ok(i32::from_le_bytes(
            bytes.try_into().expect("four byte slice"),
        ))
    }

    fn read_u16(&mut self) -> Result<u16, ProtocolError> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes(
            bytes.try_into().expect("two byte slice"),
        ))
    }

    fn read_u64(&mut self) -> Result<u64, ProtocolError> {
        let bytes = self.read_bytes(8)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().expect("eight byte slice"),
        ))
    }

    fn read_string(&mut self) -> Result<String, ProtocolError> {
        let length = self.read_u32()? as usize;
        let bytes = self.read_bytes(length)?.to_vec();
        String::from_utf8(bytes).map_err(Into::into)
    }

    fn read_string_lossy(&mut self) -> Result<String, ProtocolError> {
        let length = self.read_u32()? as usize;
        Ok(String::from_utf8_lossy(self.read_bytes(length)?).into_owned())
    }

    fn read_ipv4(&mut self) -> Result<Ipv4Addr, ProtocolError> {
        let bytes = self.read_bytes(4)?;
        Ok(Ipv4Addr::new(bytes[3], bytes[2], bytes[1], bytes[0]))
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::TruncatedPayload)?;
        let bytes = self
            .payload
            .get(self.position..end)
            .ok_or(ProtocolError::TruncatedPayload)?;
        self.position = end;
        Ok(bytes)
    }
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Soulseek server message length {0} is invalid")]
    InvalidLength(usize),
    #[error("Soulseek message payload ended unexpectedly")]
    TruncatedPayload,
    #[error("Expected Soulseek message code {expected}, received {actual}")]
    UnexpectedCode { expected: u32, actual: u32 },
    #[error("Expected Soulseek distributed message code {expected}, received {actual}")]
    UnexpectedDistributedCode { expected: u8, actual: u8 },
    #[error("Soulseek distributed search identifier {0} is invalid")]
    InvalidDistributedIdentifier(u32),
    #[error("Soulseek distributed search fields are invalid")]
    InvalidDistributedSearch,
    #[error("Soulseek distributed branch level {0} is invalid")]
    InvalidDistributedBranchLevel(i32),
    #[error("Soulseek distributed branch root is invalid")]
    InvalidDistributedBranchRoot,
    #[error("Soulseek user data is invalid")]
    InvalidUserData,
    #[error("Soulseek user profile exceeds the safety limits")]
    InvalidUserInfo,
    #[error("Soulseek private-message fields are invalid")]
    InvalidPrivateMessage,
    #[error("Soulseek room or room-message fields are invalid")]
    InvalidRoomMessage,
    #[error("Unexpected Soulseek peer initialization code {0}")]
    UnexpectedPeerInitCode(u8),
    #[error("Soulseek {kind} count {count} exceeds the safety limit")]
    InvalidCount { kind: &'static str, count: usize },
    #[error("Decompressed Soulseek payload exceeds the safety limit")]
    DecompressedPayloadTooLarge,
    #[error("Soulseek message text is not valid UTF-8: {0}")]
    InvalidUtf8(#[from] FromUtf8Error),
    #[error("Soulseek socket error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    fn encoded_string(value: &str) -> Vec<u8> {
        let mut result = Vec::new();
        push_string(&mut result, value);
        result
    }

    fn decoded_frame(bytes: &[u8]) -> Frame {
        Frame {
            code: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            payload: bytes[8..].to_vec(),
        }
    }

    #[test]
    fn encodes_login_exactly_like_the_protocol_reference() {
        let frame = login_frame("username", "password");

        assert_eq!(u32::from_le_bytes(frame[0..4].try_into().unwrap()), 72);
        assert_eq!(
            u32::from_le_bytes(frame[4..8].try_into().unwrap()),
            LOGIN_CODE
        );
        assert!(frame
            .windows(32)
            .any(|window| window == b"d51c9a7e9353746a6020f9602d452929"));
        assert_eq!(
            u32::from_le_bytes(frame[frame.len() - 4..].try_into().unwrap()),
            FOREVER_MINOR_VERSION
        );
    }

    #[test]
    fn parses_bounded_public_room_directory_and_member_arrays() {
        let mut directory = 2_u32.to_le_bytes().to_vec();
        directory.extend(encoded_string("Lossless Listening"));
        directory.extend(encoded_string("Ambient / Drone"));
        directory.extend(2_u32.to_le_bytes());
        directory.extend(124_u32.to_le_bytes());
        directory.extend(61_u32.to_le_bytes());
        let rooms = parse_room_list(&Frame {
            code: ROOM_LIST_CODE,
            payload: directory,
        })
        .unwrap();
        assert_eq!(rooms[0].name, "Lossless Listening");
        assert_eq!(rooms[1].user_count, 61);

        let mut joined = encoded_string("Lossless Listening");
        joined.extend(1_u32.to_le_bytes());
        joined.extend(encoded_string("needle_drop"));
        joined.extend(1_u32.to_le_bytes());
        joined.extend(2_u32.to_le_bytes());
        joined.extend(1_u32.to_le_bytes());
        joined.extend(18_200_000_u32.to_le_bytes());
        joined.extend(4_u32.to_le_bytes());
        joined.extend(0_u32.to_le_bytes());
        joined.extend(184_210_u32.to_le_bytes());
        joined.extend(12_940_u32.to_le_bytes());
        joined.extend(1_u32.to_le_bytes());
        joined.extend(0_u32.to_le_bytes());
        joined.extend(1_u32.to_le_bytes());
        joined.extend(encoded_string("no"));
        let room = parse_join_room(&Frame {
            code: JOIN_ROOM_CODE,
            payload: joined,
        })
        .unwrap();
        assert_eq!(room.members.len(), 1);
        assert_eq!(room.members[0].country_code.as_deref(), Some("NO"));
        assert_eq!(room.members[0].shared_file_count, 184_210);
        assert!(room.members[0].slots_free);

        let oversized = (u32::try_from(MAX_ROOM_LIST).unwrap() + 1)
            .to_le_bytes()
            .to_vec();
        assert!(matches!(
            parse_room_list(&Frame {
                code: ROOM_LIST_CODE,
                payload: oversized,
            }),
            Err(ProtocolError::InvalidCount { .. })
        ));
    }

    #[test]
    fn parses_successful_login_response() {
        let mut payload = vec![1];
        payload.extend(encoded_string("Welcome to Soulseek"));
        payload.extend(0x01020304_u32.to_le_bytes());
        payload.extend(encoded_string("password-hash"));
        payload.push(1);

        assert_eq!(
            parse_login_response(&Frame {
                code: LOGIN_CODE,
                payload,
            })
            .unwrap(),
            LoginResponse::Accepted {
                greeting: "Welcome to Soulseek".to_owned(),
                supporter: true,
            }
        );
    }

    #[test]
    fn parses_rejected_login_response_with_detail() {
        let mut payload = vec![0];
        payload.extend(encoded_string("INVALIDUSERNAME"));
        payload.extend(encoded_string("Nick too long."));

        assert_eq!(
            parse_login_response(&Frame {
                code: LOGIN_CODE,
                payload,
            })
            .unwrap(),
            LoginResponse::Rejected {
                reason: "INVALIDUSERNAME".to_owned(),
                detail: Some("Nick too long.".to_owned()),
            }
        );
    }

    #[test]
    fn encodes_wait_port_and_file_search_frames() {
        let wait_port = set_wait_port_frame(48_123);
        assert_eq!(
            u32::from_le_bytes(wait_port[4..8].try_into().unwrap()),
            SET_WAIT_PORT_CODE
        );
        assert_eq!(
            u32::from_le_bytes(wait_port[8..12].try_into().unwrap()),
            48_123
        );

        let search = file_search_frame(91, "night geometry");
        assert_eq!(
            u32::from_le_bytes(search[4..8].try_into().unwrap()),
            FILE_SEARCH_CODE
        );
        assert_eq!(u32::from_le_bytes(search[8..12].try_into().unwrap()), 91);
        assert!(search.ends_with(b"night geometry"));
    }

    #[test]
    fn parses_people_presence_stats_and_country() {
        let mut watch_payload = encoded_string("audiophile92");
        watch_payload.push(1);
        watch_payload.extend(2_u32.to_le_bytes());
        watch_payload.extend(8_200_000_u32.to_le_bytes());
        watch_payload.extend(18_402_u32.to_le_bytes());
        watch_payload.extend(0_u32.to_le_bytes());
        watch_payload.extend(23_941_u32.to_le_bytes());
        watch_payload.extend(1_284_u32.to_le_bytes());
        watch_payload.extend(encoded_string("nl"));
        let watched = parse_watch_user(&Frame {
            code: WATCH_USER_CODE,
            payload: watch_payload,
        })
        .unwrap();
        assert_eq!(watched.username, "audiophile92");
        assert_eq!(watched.status, 2);
        assert_eq!(watched.average_speed, 8_200_000);
        assert_eq!(watched.shared_file_count, 23_941);
        assert_eq!(watched.country_code.as_deref(), Some("NL"));

        let mut status_payload = encoded_string("audiophile92");
        status_payload.extend(1_u32.to_le_bytes());
        status_payload.push(1);
        assert_eq!(
            parse_user_status(&Frame {
                code: USER_STATUS_CODE,
                payload: status_payload,
            })
            .unwrap(),
            ("audiophile92".to_owned(), 1, true)
        );

        let mut stats_payload = encoded_string("audiophile92");
        stats_payload.extend(8_200_000_u32.to_le_bytes());
        stats_payload.extend(18_402_u32.to_le_bytes());
        stats_payload.extend(0_u32.to_le_bytes());
        stats_payload.extend(23_941_u32.to_le_bytes());
        stats_payload.extend(1_284_u32.to_le_bytes());
        let stats = parse_user_stats(&Frame {
            code: USER_STATS_CODE,
            payload: stats_payload,
        })
        .unwrap();
        assert_eq!(stats.upload_count, 18_402);
        assert_eq!(stats.shared_directory_count, 1_284);
    }

    #[test]
    fn parses_bounded_user_interests() {
        let mut payload = encoded_string("resonant");
        payload.extend(2_u32.to_le_bytes());
        payload.extend(encoded_string("minimalism"));
        payload.extend(encoded_string("sound art"));
        payload.extend(1_u32.to_le_bytes());
        payload.extend(encoded_string("transcodes"));
        let interests = parse_user_interests(&Frame {
            code: USER_INTERESTS_CODE,
            payload,
        })
        .unwrap();
        assert_eq!(interests.likes, vec!["minimalism", "sound art"]);
        assert_eq!(interests.hates, vec!["transcodes"]);

        let mut oversized = encoded_string("resonant");
        oversized.extend(u32::try_from(MAX_USER_INTERESTS + 1).unwrap().to_le_bytes());
        assert!(matches!(
            parse_user_interests(&Frame {
                code: USER_INTERESTS_CODE,
                payload: oversized,
            }),
            Err(ProtocolError::InvalidCount { .. })
        ));
    }

    #[test]
    fn user_info_response_round_trips_and_rejects_oversized_content() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let frame =
            user_info_response_frame("Sharing careful rips.", Some(&png), 3, 2, true, 1).unwrap();
        let response = parse_user_info_response(&decoded_frame(&frame)).unwrap();
        assert_eq!(response.description, "Sharing careful rips.");
        assert_eq!(response.picture.as_deref(), Some(png.as_slice()));
        assert_eq!(response.upload_slots, 3);
        assert_eq!(response.queue_size, 2);
        assert!(response.slots_free);
        assert_eq!(response.upload_permission, Some(1));

        assert!(matches!(
            user_info_response_frame(
                &"x".repeat(MAX_PROFILE_DESCRIPTION_BYTES + 1),
                None,
                1,
                0,
                true,
                1,
            ),
            Err(ProtocolError::InvalidUserInfo)
        ));
        assert!(matches!(
            user_info_response_frame(
                "profile",
                Some(&vec![0; MAX_PROFILE_PICTURE_BYTES + 1]),
                1,
                0,
                true,
                1,
            ),
            Err(ProtocolError::InvalidUserInfo)
        ));
    }

    #[tokio::test]
    async fn profile_reader_rejects_frames_above_its_dedicated_limit() {
        let bytes = u32::try_from(MAX_PROFILE_MESSAGE_LENGTH + 1)
            .unwrap()
            .to_le_bytes();
        let mut source = bytes.as_slice();
        assert!(matches!(
            read_profile_frame(&mut source).await,
            Err(ProtocolError::InvalidLength(_))
        ));
    }

    #[test]
    fn encodes_distributed_topology_frames() {
        let no_parent = have_no_parent_frame(true);
        assert_eq!(
            u32::from_le_bytes(no_parent[4..8].try_into().unwrap()),
            HAVE_NO_PARENT_CODE
        );
        assert_eq!(no_parent[8], 1);

        let root = branch_root_frame("music_library_user");
        assert_eq!(
            u32::from_le_bytes(root[4..8].try_into().unwrap()),
            BRANCH_ROOT_CODE
        );
        assert!(root.ends_with(b"music_library_user"));

        let level = branch_level_frame(7);
        assert_eq!(
            u32::from_le_bytes(level[4..8].try_into().unwrap()),
            BRANCH_LEVEL_CODE
        );
        assert_eq!(u32::from_le_bytes(level[8..12].try_into().unwrap()), 7);
        assert_eq!(accept_children_frame(false)[8], 0);
    }

    #[test]
    fn parses_possible_distributed_parents_with_bounded_ports() {
        let mut payload = 2_u32.to_le_bytes().to_vec();
        payload.extend(encoded_string("relay-one"));
        payload.extend([1, 0, 0, 127]);
        payload.extend(48_123_u32.to_le_bytes());
        payload.extend(encoded_string("invalid-relay"));
        payload.extend([5, 4, 3, 2]);
        payload.extend(0_u32.to_le_bytes());

        assert_eq!(
            parse_possible_parents(&Frame {
                code: POSSIBLE_PARENTS_CODE,
                payload,
            })
            .unwrap(),
            vec![ParentCandidate {
                username: "relay-one".to_owned(),
                address: Ipv4Addr::new(127, 0, 0, 1),
                port: 48_123,
            }]
        );
    }

    #[test]
    fn parses_distributed_and_embedded_search_requests() {
        let mut payload = u32::from(b'1').to_le_bytes().to_vec();
        payload.extend(encoded_string("listener"));
        payload.extend(991_u32.to_le_bytes());
        payload.extend(encoded_string("night geometry -live"));
        let distributed = DistributedFrame {
            code: DISTRIBUTED_SEARCH_CODE,
            payload: payload.clone(),
        };
        let expected = DistributedSearch {
            username: "listener".to_owned(),
            token: 991,
            query: "night geometry -live".to_owned(),
        };
        assert_eq!(parse_distributed_search(&distributed).unwrap(), expected);

        let mut embedded = vec![DISTRIBUTED_SEARCH_CODE];
        embedded.extend(payload);
        assert_eq!(
            parse_embedded_distributed_search(&Frame {
                code: EMBEDDED_MESSAGE_CODE,
                payload: embedded,
            })
            .unwrap(),
            expected
        );
    }

    #[test]
    fn rejects_malformed_distributed_searches_and_branch_state() {
        let invalid_search = DistributedFrame {
            code: DISTRIBUTED_SEARCH_CODE,
            payload: 0_u32.to_le_bytes().to_vec(),
        };
        assert!(matches!(
            parse_distributed_search(&invalid_search),
            Err(ProtocolError::InvalidDistributedIdentifier(0))
        ));
        assert!(matches!(
            parse_distributed_branch_level(&DistributedFrame {
                code: DISTRIBUTED_BRANCH_LEVEL_CODE,
                payload: (-1_i32).to_le_bytes().to_vec(),
            }),
            Err(ProtocolError::InvalidDistributedBranchLevel(-1))
        ));
    }

    #[tokio::test]
    async fn reads_fragmented_distributed_frames() {
        let mut payload = encoded_string("root-user");
        let mut bytes = Vec::new();
        push_u32(&mut bytes, u32::try_from(payload.len() + 1).unwrap());
        bytes.push(DISTRIBUTED_BRANCH_ROOT_CODE);
        bytes.append(&mut payload);
        let (mut writer, mut reader) = tokio::io::duplex(16);
        let write_task = tokio::spawn(async move {
            for byte in bytes {
                writer.write_all(&[byte]).await.unwrap();
            }
        });
        let frame = read_distributed_frame(&mut reader).await.unwrap();
        assert_eq!(parse_distributed_branch_root(&frame).unwrap(), "root-user");
        write_task.await.unwrap();
    }

    #[test]
    fn parses_connect_to_peer_with_reversed_ipv4_bytes() {
        let mut payload = encoded_string("signalpath");
        payload.extend(encoded_string("P"));
        payload.extend([1, 0, 0, 127]);
        payload.extend(48_123_u32.to_le_bytes());
        payload.extend(77_u32.to_le_bytes());
        payload.push(0);
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());

        let request = parse_connect_to_peer(&Frame {
            code: CONNECT_TO_PEER_CODE,
            payload,
        })
        .unwrap();
        assert_eq!(request.username, "signalpath");
        assert_eq!(request.connection_type, "P");
        assert_eq!(request.address, Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(request.port, 48_123);
        assert_eq!(request.token, 77);
    }

    #[test]
    fn parses_peer_initialization_messages() {
        let mut peer_payload = encoded_string("audiophile92");
        peer_payload.extend(encoded_string("P"));
        peer_payload.extend(42_u32.to_le_bytes());

        assert_eq!(
            parse_peer_init(1, &peer_payload).unwrap(),
            PeerInit::Peer {
                username: "audiophile92".to_owned(),
                connection_type: "P".to_owned(),
                token: 42,
            }
        );
        assert_eq!(
            parse_peer_init(0, &42_u32.to_le_bytes()).unwrap(),
            PeerInit::PierceFirewall { token: 42 }
        );
    }

    #[test]
    fn encodes_download_queue_and_peer_connection_messages() {
        let connection = connect_to_peer_frame(73, "deepcrate", "P");
        assert_eq!(
            u32::from_le_bytes(connection[4..8].try_into().unwrap()),
            CONNECT_TO_PEER_CODE
        );
        assert_eq!(
            u32::from_le_bytes(connection[8..12].try_into().unwrap()),
            73
        );
        assert!(connection.windows(9).any(|bytes| bytes == b"deepcrate"));

        let queue = queue_upload_frame("Music\\Track.flac");
        assert_eq!(
            u32::from_le_bytes(queue[4..8].try_into().unwrap()),
            QUEUE_UPLOAD_CODE
        );
        assert!(queue.ends_with(b"Music\\Track.flac"));

        let peer_init = peer_init_frame("SignalLevel", "P");
        assert_eq!(peer_init[4], 1);
        assert!(peer_init.windows(11).any(|bytes| bytes == b"SignalLevel"));
        assert_eq!(
            u32::from_le_bytes(peer_init[peer_init.len() - 4..].try_into().unwrap()),
            0
        );
    }

    #[test]
    fn parses_peer_addresses_and_upload_requests() {
        let mut address_payload = encoded_string("deepcrate");
        address_payload.extend([5, 4, 3, 2]);
        address_payload.extend(22_334_u32.to_le_bytes());
        address_payload.extend(0_u32.to_le_bytes());
        address_payload.extend(0_u16.to_le_bytes());
        let address = parse_peer_address(&Frame {
            code: GET_PEER_ADDRESS_CODE,
            payload: address_payload,
        })
        .unwrap();
        assert_eq!(address.username, "deepcrate");
        assert_eq!(address.address, Ipv4Addr::new(2, 3, 4, 5));
        assert_eq!(address.port, 22_334);

        let mut request_payload = 1_u32.to_le_bytes().to_vec();
        request_payload.extend(881_u32.to_le_bytes());
        request_payload.extend(encoded_string("Music/Track.flac"));
        request_payload.extend(98_765_u64.to_le_bytes());
        let request = parse_transfer_request(&Frame {
            code: TRANSFER_REQUEST_CODE,
            payload: request_payload,
        })
        .unwrap();
        assert_eq!(request.direction, 1);
        assert_eq!(request.token, 881);
        assert_eq!(request.filename, "Music\\Track.flac");
        assert_eq!(request.size_bytes, Some(98_765));

        let accepted = transfer_response_frame(request.token, true, None);
        assert_eq!(accepted.len(), 13);
        assert_eq!(accepted[12], 1);
    }

    #[test]
    fn decompresses_and_parses_a_search_response() {
        let mut payload = encoded_string("audiophile92");
        payload.extend(739_u32.to_le_bytes());
        payload.extend(1_u32.to_le_bytes());
        payload.push(1);
        payload.extend(encoded_string("Music\\Night Geometry\\Thresholds.flac"));
        payload.extend(1_204_567_890_u64.to_le_bytes());
        payload.extend(encoded_string("flac"));
        payload.extend(3_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(2_304_u32.to_le_bytes());
        payload.extend(1_u32.to_le_bytes());
        payload.extend(321_u32.to_le_bytes());
        payload.extend(4_u32.to_le_bytes());
        payload.extend(96_000_u32.to_le_bytes());
        payload.push(1);
        payload.extend(8_200_000_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        let compressed = encoder.finish().unwrap();
        let response = parse_search_response(&Frame {
            code: FILE_SEARCH_RESPONSE_CODE,
            payload: compressed,
        })
        .unwrap();

        assert_eq!(response.username, "audiophile92");
        assert_eq!(response.token, 739);
        assert!(response.slot_free);
        assert_eq!(response.average_speed, 8_200_000);
        assert_eq!(response.files.len(), 1);
        assert_eq!(response.files[0].size_bytes, 1_204_567_890);
        assert_eq!(response.files[0].bitrate, Some(2_304));
        assert_eq!(response.files[0].duration_seconds, Some(321));
        assert_eq!(response.files[0].sample_rate, Some(96_000));
    }

    #[test]
    fn outgoing_search_and_share_responses_round_trip() {
        let search_file = SearchFile {
            filename: "Midnight Archive\\Burial\\04 Endorphin.flac".to_owned(),
            size_bytes: 31_800_000,
            extension: "flac".to_owned(),
            bitrate: Some(1_020),
            duration_seconds: Some(179),
            vbr: Some(false),
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            is_private: false,
        };
        let response = file_search_response_frame(
            "music_library_user",
            91,
            std::slice::from_ref(&search_file),
            true,
            2_800_000,
            2,
        )
        .unwrap();
        let decoded = parse_search_response(&decoded_frame(&response)).unwrap();
        assert_eq!(decoded.username, "music_library_user");
        assert_eq!(decoded.token, 91);
        assert_eq!(decoded.files, vec![search_file]);

        let listing = ShareListing {
            directory: "Midnight Archive\\Burial".to_owned(),
            files: vec![FolderFile {
                filename: "04 Endorphin.flac".to_owned(),
                size_bytes: 31_800_000,
                extension: "flac".to_owned(),
                bitrate: None,
                duration_seconds: None,
                vbr: None,
                sample_rate: None,
                bit_depth: None,
            }],
            is_private: false,
        };
        let response = shared_file_list_response_frame(std::slice::from_ref(&listing)).unwrap();
        assert_eq!(
            parse_shared_file_list_response(&decoded_frame(&response))
                .unwrap()
                .directories,
            vec![listing]
        );
    }

    #[test]
    fn encodes_folder_requests_and_parses_compressed_folder_contents() {
        let request = folder_contents_request_frame(812, "Music\\Night Geometry");
        assert_eq!(
            u32::from_le_bytes(request[4..8].try_into().unwrap()),
            FOLDER_CONTENTS_REQUEST_CODE
        );
        assert_eq!(u32::from_le_bytes(request[8..12].try_into().unwrap()), 812);
        assert!(request.ends_with(b"Music\\Night Geometry"));

        let mut payload = 812_u32.to_le_bytes().to_vec();
        payload.extend(encoded_string("Music\\Night Geometry"));
        payload.extend(1_u32.to_le_bytes());
        payload.extend(encoded_string("Music\\Night Geometry"));
        payload.extend(1_u32.to_le_bytes());
        payload.push(1);
        payload.extend(encoded_string("01 - Thresholds.flac"));
        payload.extend(112_400_000_u64.to_le_bytes());
        payload.extend(encoded_string(""));
        payload.extend(3_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(2_304_u32.to_le_bytes());
        payload.extend(4_u32.to_le_bytes());
        payload.extend(96_000_u32.to_le_bytes());
        payload.extend(5_u32.to_le_bytes());
        payload.extend(24_u32.to_le_bytes());

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        let response = parse_folder_contents_response(&Frame {
            code: FOLDER_CONTENTS_RESPONSE_CODE,
            payload: encoder.finish().unwrap(),
        })
        .unwrap();
        assert_eq!(response.token, 812);
        assert_eq!(response.requested_folder, "Music\\Night Geometry");
        assert_eq!(response.folders.len(), 1);
        assert_eq!(response.folders[0].files.len(), 1);
        assert_eq!(response.folders[0].files[0].extension, "flac");
        assert_eq!(response.folders[0].files[0].bit_depth, Some(24));
    }

    #[test]
    fn encodes_share_list_requests_and_parses_public_and_private_directories() {
        let request = shared_file_list_request_frame();
        assert_eq!(request.len(), 8);
        assert_eq!(
            u32::from_le_bytes(request[4..8].try_into().unwrap()),
            SHARED_FILE_LIST_REQUEST_CODE
        );

        let mut payload = 1_u32.to_le_bytes().to_vec();
        payload.extend(encoded_string("Music\\Night Geometry"));
        payload.extend(1_u32.to_le_bytes());
        payload.push(1);
        payload.extend(encoded_string("01 - Thresholds.flac"));
        payload.extend(112_400_000_u64.to_le_bytes());
        payload.extend(encoded_string(""));
        payload.extend(2_u32.to_le_bytes());
        payload.extend(4_u32.to_le_bytes());
        payload.extend(96_000_u32.to_le_bytes());
        payload.extend(5_u32.to_le_bytes());
        payload.extend(24_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(1_u32.to_le_bytes());
        payload.extend(encoded_string("Private Mixes"));
        payload.extend(0_u32.to_le_bytes());

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        let response = parse_shared_file_list_response(&Frame {
            code: SHARED_FILE_LIST_RESPONSE_CODE,
            payload: encoder.finish().unwrap(),
        })
        .unwrap();
        assert_eq!(response.directories.len(), 2);
        assert!(!response.directories[0].is_private);
        assert_eq!(response.directories[0].files[0].extension, "flac");
        assert_eq!(response.directories[0].files[0].bit_depth, Some(24));
        assert!(response.directories[1].is_private);
    }

    #[tokio::test]
    async fn reads_a_complete_server_frame() {
        let bytes = encode_message(69, &[1, 2, 3, 4]);
        let mut source = bytes.as_slice();

        assert_eq!(
            read_frame(&mut source).await.unwrap(),
            Frame {
                code: 69,
                payload: vec![1, 2, 3, 4],
            }
        );
    }
}
