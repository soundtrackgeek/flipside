

mod credentials;
mod diagnostics;
mod distributed;
mod downloads;
mod folders;
mod local_shares;
mod messages;
mod people;
mod protocol;
mod radar;
mod rooms;
mod search;
mod service;
mod settings;
mod shares;
pub(crate) mod soundcheck;
mod uploads;
mod wanted;

use search::SearchSnapshot;
use service::{ConnectionBootstrap, ConnectionManager, ConnectionPaths, SaveConnectionRequest};
use tauri::{AppHandle, Manager, State};

// Payload types of the typed UI events (declared in `crate::events`).
pub(crate) use downloads::TransferQueueSnapshot;
pub(crate) use local_shares::LocalSharesSnapshot;
pub(crate) use search::SearchEvent;
pub(crate) use service::{ConnectionManager as BridgeConnectionManager, ConnectionSnapshot, ConnectionState as BridgeConnectionState};
pub(crate) use uploads::UploadQueueSnapshot;

pub fn initialize(app: &AppHandle) -> Result<ConnectionManager, String> {
    let config_directory = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let download_directory = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| config_directory.clone())
        .join("_SoulSeek");

    let manager = ConnectionManager::new(
        app.clone(),
        ConnectionPaths {
            settings: config_directory.join("connection.json"),
            transfers: config_directory.join("transfers.json"),
            sharing: config_directory.join("sharing.json"),
            people: config_directory.join("people.json"),
            messages: config_directory.join("messages.json"),
            rooms: config_directory.join("rooms.json"),
            wanted: config_directory.join("wanted.json"),
            diagnostics: config_directory.join("logs").join("connection.log"),
        },
        download_directory,
    )
    .map_err(|error| error.to_string())?;

    if manager.bootstrap().is_ok_and(|bootstrap| {
        bootstrap.has_password
            && bootstrap
                .profile
                .is_some_and(|profile| profile.auto_connect)
    }) {
        let _ = manager.connect();
    }

    Ok(manager)
}

#[tauri::command]
#[specta::specta]
pub async fn local_shares_snapshot(
    manager: State<'_, ConnectionManager>,
) -> Result<local_shares::LocalSharesSnapshot, String> {
    Ok(manager.current_local_shares())
}

#[tauri::command]
#[specta::specta]
pub async fn local_shares_add(
    manager: State<'_, ConnectionManager>,
    path: String,
) -> Result<local_shares::LocalSharesSnapshot, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.add_local_share(&path))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn local_shares_remove(
    manager: State<'_, ConnectionManager>,
    id: String,
) -> Result<local_shares::LocalSharesSnapshot, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.remove_local_share(&id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn local_shares_set_enabled(
    manager: State<'_, ConnectionManager>,
    id: String,
    enabled: bool,
) -> Result<local_shares::LocalSharesSnapshot, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.set_local_share_enabled(&id, enabled))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn local_shares_rescan(
    manager: State<'_, ConnectionManager>,
) -> Result<local_shares::LocalSharesSnapshot, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.rescan_local_shares())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn local_shares_set_upload_slots(
    manager: State<'_, ConnectionManager>,
    upload_slots: u8,
) -> Result<local_shares::LocalSharesSnapshot, String> {
    manager
        .set_upload_slots(upload_slots)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn uploads_snapshot(
    manager: State<'_, ConnectionManager>,
) -> Result<uploads::UploadQueueSnapshot, String> {
    Ok(manager.current_uploads())
}

#[tauri::command]
#[specta::specta]
pub async fn upload_cancel(
    manager: State<'_, ConnectionManager>,
    id: String,
) -> Result<uploads::UploadQueueSnapshot, String> {
    manager
        .cancel_upload(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn upload_clear_finished(
    manager: State<'_, ConnectionManager>,
) -> Result<uploads::UploadQueueSnapshot, String> {
    Ok(manager.clear_finished_uploads())
}

#[tauri::command]
#[specta::specta]
pub async fn transfers_snapshot(
    manager: State<'_, ConnectionManager>,
) -> Result<downloads::TransferQueueSnapshot, String> {
    Ok(manager.current_transfers())
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_set_max_concurrent_downloads(
    manager: State<'_, ConnectionManager>,
    max_concurrent_downloads: u8,
) -> Result<downloads::TransferQueueSnapshot, String> {
    manager
        .set_max_concurrent_downloads(max_concurrent_downloads)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_enqueue_release(
    manager: State<'_, ConnectionManager>,
    request: downloads::EnqueueReleaseRequest,
) -> Result<downloads::TransferQueueSnapshot, String> {
    manager
        .enqueue_release(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_pause_release(
    manager: State<'_, ConnectionManager>,
    release_id: String,
) -> Result<downloads::TransferQueueSnapshot, String> {
    manager
        .pause_release(&release_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_resume_release(
    manager: State<'_, ConnectionManager>,
    release_id: String,
) -> Result<downloads::TransferQueueSnapshot, String> {
    manager
        .resume_release(&release_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_cancel_release(
    manager: State<'_, ConnectionManager>,
    release_id: String,
) -> Result<downloads::TransferQueueSnapshot, String> {
    manager
        .cancel_release(&release_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_clear_completed(
    manager: State<'_, ConnectionManager>,
) -> Result<downloads::TransferQueueSnapshot, String> {
    manager
        .clear_completed_transfers()
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_reveal_release_path(
    manager: State<'_, ConnectionManager>,
    release_id: String,
) -> Result<String, String> {
    manager
        .reveal_release_path(&release_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn connection_bootstrap(
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionBootstrap, String> {
    manager.bootstrap().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn connection_save_profile(
    manager: State<'_, ConnectionManager>,
    request: SaveConnectionRequest,
) -> Result<ConnectionBootstrap, String> {
    manager
        .save_profile(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn connection_connect(
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionSnapshot, String> {
    manager.connect().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn connection_disconnect(
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionSnapshot, String> {
    manager.disconnect().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn connection_reset(
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionBootstrap, String> {
    manager.reset().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn search_snapshot(
    manager: State<'_, ConnectionManager>,
) -> Result<Vec<SearchSnapshot>, String> {
    Ok(manager.current_searches())
}

#[tauri::command]
#[specta::specta]
pub async fn search_start(
    manager: State<'_, ConnectionManager>,
    client_id: String,
    query: String,
) -> Result<SearchSnapshot, String> {
    manager
        .start_search(client_id, query)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn search_stop(
    manager: State<'_, ConnectionManager>,
    client_id: String,
) -> Result<Option<SearchSnapshot>, String> {
    Ok(manager.stop_search(&client_id))
}

#[tauri::command]
#[specta::specta]
pub async fn search_close(
    manager: State<'_, ConnectionManager>,
    client_id: String,
) -> Result<bool, String> {
    Ok(manager.close_search(&client_id))
}
