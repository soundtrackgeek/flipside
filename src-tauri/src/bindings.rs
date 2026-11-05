//! Generated TypeScript bindings (tauri-specta).
//!
//! Every Tauri command is listed once here. tauri-specta both dispatches the calls
//! and exports their types to `src/bindings.ts`, so a command cannot exist without a
//! generated TypeScript signature.

use std::path::Path;

use tauri::ipc::Invoke;
use tauri::Wry;
use tauri_specta::{Builder, ErrorHandlingMode};

use super::{
    catalog_home, catalog_genre, catalog_search, catalog_album, catalog_artist,
    catalog_artist_enrichment, catalog_album_enrichment, get_settings, save_settings,
    keys_status, keys_save, keys_delete, get_deemix_credential_status, save_deemix_arl,
    delete_deemix_arl, test_deemix_connection, search_deemix_albums, download_deemix_album,
    preflight_deemix_album_download, get_app_update_status, check_app_update,
    install_app_update, catalog_resolve_album, discovery_query, discovery_status, discovery_refresh,
    library_status, library_reload, soulseek, usenet,
};

macro_rules! specta_commands {
    ($($b:ident $(:: $p:ident)*),* $(,)?) => {
        tauri_specta::collect_commands![$($b $(:: $p)*),*]
    };
}

fn builder() -> Builder<Wry> {
    let commands = specta_commands![
        catalog_home,
        catalog_genre,
        catalog_search,
        catalog_album,
        catalog_artist,
        catalog_artist_enrichment,
        catalog_album_enrichment,
        catalog_resolve_album,
        discovery_query,
        discovery_status,
        discovery_refresh,
        library_status,
        library_reload,
        get_settings,
        save_settings,
        keys_status,
        keys_save,
        keys_delete,
        get_deemix_credential_status,
        save_deemix_arl,
        delete_deemix_arl,
        test_deemix_connection,
        search_deemix_albums,
        download_deemix_album,
        preflight_deemix_album_download,
        get_app_update_status,
        check_app_update,
        install_app_update,
        usenet::usenet_bootstrap,
        usenet::usenet_save_profile,
        usenet::usenet_reset,
        usenet::usenet_test_connections,
        usenet::usenet_search,
        usenet::usenet_transfers_snapshot,
        usenet::usenet_enqueue_download,
        usenet::usenet_clear_completed,
        soulseek::local_shares_snapshot,
        soulseek::local_shares_add,
        soulseek::local_shares_remove,
        soulseek::local_shares_set_enabled,
        soulseek::local_shares_rescan,
        soulseek::local_shares_set_upload_slots,
        soulseek::uploads_snapshot,
        soulseek::upload_cancel,
        soulseek::upload_clear_finished,
        soulseek::transfers_snapshot,
        soulseek::transfer_set_max_concurrent_downloads,
        soulseek::transfer_enqueue_release,
        soulseek::transfer_pause_release,
        soulseek::transfer_resume_release,
        soulseek::transfer_cancel_release,
        soulseek::transfer_clear_completed,
        soulseek::transfer_reveal_release_path,
        soulseek::connection_bootstrap,
        soulseek::connection_save_profile,
        soulseek::connection_connect,
        soulseek::connection_disconnect,
        soulseek::connection_reset,
        soulseek::search_snapshot,
        soulseek::search_start,
        soulseek::search_stop,
        soulseek::search_close
    ];
    Builder::<Wry>::new()
        .commands(commands)
        .events(crate::events::collect())
        // Command errors reject the promise.
        .error_handling(ErrorHandlingMode::Throw)
        // Row ids and counts stay far below 2^53.
        .dangerously_cast_bigints_to_number()
}

pub fn export(path: &Path) -> Result<(), String> {
    builder()
        .export(specta_typescript::Typescript::default(), path)
        .map_err(|error| error.to_string())
}

/// Regenerates the bindings in debug builds so `tauri dev` always serves types that
/// match the Rust commands. Only rewrites the file when the content changed.
#[cfg(debug_assertions)]
pub fn export_in_debug(path: &Path) {
    let temporary = std::env::temp_dir().join("flipside-bindings-dev.ts");
    let result = export(&temporary).and_then(|()| {
        let generated = std::fs::read_to_string(&temporary).map_err(|error| error.to_string())?;
        let current = std::fs::read_to_string(path).unwrap_or_default();
        let normalize = |text: &str| text.replace("\r\n", "\n");
        if normalize(&generated) != normalize(&current) {
            std::fs::write(path, generated).map_err(|error| error.to_string())?;
        }
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("Could not refresh {}: {error}", path.display());
    }
}

fn shared() -> &'static Builder<Wry> {
    static BUILDER: std::sync::OnceLock<Builder<Wry>> = std::sync::OnceLock::new();
    BUILDER.get_or_init(builder)
}

pub fn invoke_handler() -> impl Fn(Invoke<Wry>) -> bool + Send + Sync + 'static {
    shared().invoke_handler()
}

pub fn mount_events(app: &tauri::App) {
    shared().mount_events(app);
}
