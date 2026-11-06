//! Flipside: a visual store for music with Usenet, Deezer and Soulseek downloads.
#![allow(dead_code, unused_imports)] // Provider modules are ported whole; not every API is surfaced in the UI yet.

mod catalog;
mod covers;
mod db;
mod discovery;
mod deemix;
mod deemix_download;
mod events;
mod http;
mod identity;
mod keys;
mod meta;
mod library;
mod soulseek;
mod updater;
mod usenet;
mod wire_enums;

use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_window_state::StateFlags;

/// Runs a blocking provider call off the async runtime and flattens the errors to strings.
#[cfg(not(test))]
async fn blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("Background task failed: {error}"))?
        .map_err(|error| format!("{error:#}"))
}

// --- Catalog (the store front) -------------------------------------------------

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_home() -> Result<catalog::HomeFeed, String> {
    blocking(catalog::home).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_genre(id: String) -> Result<Vec<catalog::AlbumCard>, String> {
    blocking(move || catalog::genre_albums(&id)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_search(query: String) -> Result<catalog::SearchResults, String> {
    blocking(move || catalog::search(&query)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_album(id: String) -> Result<catalog::AlbumDetail, String> {
    blocking(move || catalog::album(&id)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_artist(id: String) -> Result<catalog::ArtistDetail, String> {
    blocking(move || catalog::artist(&id)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_artist_enrichment(name: String) -> Result<catalog::ArtistEnrichment, String> {
    blocking(move || catalog::artist_enrichment(&name)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_album_enrichment(
    artist: String,
    title: String,
) -> Result<catalog::AlbumEnrichment, String> {
    blocking(move || catalog::album_enrichment(&artist, &title)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_resolve_album(artist: String, title: String) -> Result<Option<catalog::AlbumCard>, String> {
    blocking(move || catalog::resolve_album(&artist, &title)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_cover(rgid: String, artist: String, title: String) -> Result<Option<String>, String> {
    blocking(move || covers::lookup(&rgid, &artist, &title)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_artist_albums(name: String) -> Result<Vec<discovery::DiscoveryAlbum>, String> {
    blocking(move || meta::artist_albums(&name)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_original_year(artist: String, title: String) -> Result<Option<i32>, String> {
    blocking(move || meta::original_year(&artist, &title)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn catalog_resolve_artist(name: String) -> Result<Option<catalog::ArtistCard>, String> {
    blocking(move || catalog::resolve_artist(&name)).await
}

// --- For you: library-aware discovery -------------------------------------------

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn discovery_query(query: discovery::DiscoveryQuery) -> Result<discovery::DiscoveryPage, String> {
    blocking(move || Ok(discovery::query(&query))).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn discovery_status() -> Result<discovery::DiscoveryStatus, String> {
    blocking(|| Ok(discovery::status())).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn discovery_refresh(continuous: bool) -> Result<bool, String> {
    blocking(move || Ok(discovery::start_refresh(continuous))).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn discovery_stop() -> Result<(), String> {
    blocking(|| {
        discovery::stop_refresh();
        Ok(())
    })
    .await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn library_status() -> Result<library::LibraryStatus, String> {
    blocking(|| Ok(library::status())).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn library_reload(app: AppHandle) -> Result<library::LibraryStatus, String> {
    blocking(move || {
        let settings = db::settings_for_app(&app)?;
        library::configure(&app, &settings);
        library::reload_in_background();
        Ok(library::status())
    })
    .await
}

// --- Settings and provider keys -------------------------------------------------

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn get_settings(app: AppHandle) -> Result<db::AppSettings, String> {
    blocking(move || db::settings_for_app(&app)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn save_settings(app: AppHandle, settings: db::AppSettings) -> Result<db::AppSettings, String> {
    blocking(move || {
        let saved = db::save_settings_for_app(&app, settings)?;
        library::configure(&app, &saved);
        Ok(saved)
    })
    .await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn keys_status() -> Result<Vec<keys::KeyStatus>, String> {
    blocking(|| Ok(keys::status())).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn keys_save(id: String, value: String) -> Result<Vec<keys::KeyStatus>, String> {
    blocking(move || keys::save(&id, &value)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn keys_delete(id: String) -> Result<Vec<keys::KeyStatus>, String> {
    blocking(move || keys::delete(&id)).await
}

// --- Deemix (Deezer) -------------------------------------------------------------

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn get_deemix_credential_status() -> Result<deemix::DeemixCredentialStatus, String> {
    blocking(deemix::credential_status).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn save_deemix_arl(arl: String) -> Result<deemix::DeemixConnectionTest, String> {
    blocking(move || deemix::save_arl(arl)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn delete_deemix_arl() -> Result<deemix::DeemixCredentialStatus, String> {
    blocking(deemix::delete_arl).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn test_deemix_connection() -> Result<deemix::DeemixConnectionTest, String> {
    blocking(deemix::test_connection).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn search_deemix_albums(
    input: deemix::DeemixAlbumSearchRequest,
) -> Result<deemix::DeemixAlbumSearchResponse, String> {
    blocking(move || deemix::search_albums(input)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn download_deemix_album(
    app: AppHandle,
    input: deemix_download::DeemixAlbumDownloadRequest,
) -> Result<deemix_download::DeemixAlbumDownloadSummary, String> {
    blocking(move || deemix_download::download_album_for_app(&app, input)).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn preflight_deemix_album_download(
    app: AppHandle,
    input: deemix_download::DeemixAlbumDownloadPreflightRequest,
) -> Result<deemix_download::DeemixAlbumDownloadPreflight, String> {
    blocking(move || deemix_download::preflight_album_for_app(&app, input)).await
}

// --- App updates -----------------------------------------------------------------

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn get_app_update_status(app: AppHandle) -> updater::UpdateSnapshot {
    updater::snapshot(&app).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn check_app_update(app: AppHandle) -> Result<updater::UpdateSnapshot, String> {
    updater::check(&app).await
}

#[cfg(not(test))]
#[tauri::command]
#[specta::specta]
async fn install_app_update(app: AppHandle, version: String) -> Result<(), String> {
    updater::install(&app, version).await
}

/// Checks for an update shortly after launch and then every 30 minutes.
#[cfg(not(test))]
fn start_update_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        loop {
            let enabled = db::settings_for_app(&app)
                .map(|settings| settings.auto_update_check)
                .unwrap_or(true);
            if enabled {
                updater::check_quietly(&app).await;
            }
            tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        }
    });
}

// --- Entry points ----------------------------------------------------------------

pub fn run_cli_from_args() -> bool {
    #[cfg(not(test))]
    {
        let args = std::env::args_os().collect::<Vec<_>>();
        if args.get(1).and_then(|value| value.to_str()) == Some("--export-bindings") {
            let path = args
                .get(2)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("src/bindings.ts"));
            match bindings::export(&path) {
                Ok(()) => {
                    println!("Wrote TypeScript bindings to {}", path.display());
                    std::process::exit(0);
                }
                Err(error) => {
                    eprintln!("Could not export TypeScript bindings: {error}");
                    std::process::exit(1);
                }
            }
        }
    }
    false
}

#[cfg(not(test))]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .setup(|app| {
            bindings::mount_events(app);
            #[cfg(debug_assertions)]
            bindings::export_in_debug(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bindings.ts"),
            );
            keys::seed_from_env();
            // Creates the database, then loads the library index and the discovery crawl in the background.
            db::open(app.handle())?;
            discovery::init(app.handle());
            library::configure(app.handle(), &db::settings_for_app(app.handle())?);
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(20));
                discovery::refresh_if_stale();
            });
            app.manage(updater::UpdateState::default());
            app.manage(soulseek::initialize(app.handle())?);
            app.manage(usenet::initialize(app.handle())?);
            start_update_loop(app.handle().clone());
            Ok(())
        })
        .invoke_handler(bindings::invoke_handler())
        .run(tauri::generate_context!())
        .expect("failed to run Flipside");
}

// Declared last: tauri-specta command macros are textually scoped.
#[cfg(not(test))]
mod bindings;
