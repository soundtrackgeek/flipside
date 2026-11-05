//! Typed events the backend pushes to the UI (exported to `src/bindings.ts`).

use crate::{
    deemix_download::DeemixAlbumDownloadProgress,
    soulseek::{
        ConnectionSnapshot, LocalSharesSnapshot, SearchEvent, TransferQueueSnapshot,
        UploadQueueSnapshot,
    },
    updater::{InstallProgress, UpdateSnapshot},
    usenet::UsenetTransferQueue,
};

macro_rules! wire_events {
    ($($name:ident($payload:ty) = $wire:literal;)*) => {
        $(
            /// Serializes exactly as its payload.
            #[derive(Clone, serde::Serialize, specta::Type)]
            pub struct $name(pub $payload);

            impl tauri_specta::Event for $name {
                const NAME: &'static str = $wire;
            }
        )*

        pub fn collect() -> tauri_specta::Events {
            tauri_specta::collect_events![$($name),*]
        }
    };
}

wire_events! {
    AppUpdateChecked(UpdateSnapshot) = "app-update-checked";
    AppUpdateInstallProgress(InstallProgress) = "app-update-install-progress";
    DeemixDownloadProgress(DeemixAlbumDownloadProgress) = "deemix-download-progress";
    UsenetTransfersChanged(UsenetTransferQueue) = "flipside://usenet-transfers";
    SoulseekConnectionChanged(ConnectionSnapshot) = "flipside://soulseek-connection";
    SoulseekTransfersChanged(TransferQueueSnapshot) = "flipside://soulseek-transfers";
    SoulseekLocalSharesChanged(LocalSharesSnapshot) = "flipside://soulseek-local-shares";
    SoulseekUploadsChanged(UploadQueueSnapshot) = "flipside://soulseek-uploads";
    SoulseekSearchProgress(SearchEvent) = "flipside://soulseek-search";
}
