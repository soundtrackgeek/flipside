use chrono::Utc;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;
use tauri_specta::Event as _;

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current_version: String,
    version: String,
    date: Option<String>,
    notes: Option<String>,
}

#[derive(Clone, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    checked_at: Option<String>,
    info: Option<UpdateInfo>,
    error: Option<String>,
}

#[derive(Default)]
pub struct UpdateState {
    operation: Mutex<()>,
    update: Mutex<Option<Update>>,
    snapshot: Mutex<UpdateSnapshot>,
}

pub async fn snapshot(app: &AppHandle) -> UpdateSnapshot {
    app.state::<UpdateState>().snapshot.lock().await.clone()
}

pub async fn check(app: &AppHandle) -> Result<UpdateSnapshot, String> {
    let state = app.state::<UpdateState>();
    let _operation = state.operation.lock().await;
    let result = match app
        .updater_builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(updater) => updater.check().await.map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    let mut snapshot = state.snapshot.lock().await;
    snapshot.checked_at = Some(Utc::now().to_rfc3339());
    match result {
        Ok(update) => {
            snapshot.info = update.as_ref().map(|update| UpdateInfo {
                current_version: update.current_version.clone(),
                version: update.version.clone(),
                date: update
                    .date
                    .and_then(|date| {
                        chrono::DateTime::from_timestamp(date.unix_timestamp(), date.nanosecond())
                    })
                    .map(|date| date.to_rfc3339()),
                notes: update.body.clone(),
            });
            snapshot.error = None;
            *state.update.lock().await = update;
        }
        Err(error) => {
            // Keep an already discovered update installable across transient failures.
            snapshot.error = Some(error.to_string());
        }
    }
    let snapshot = snapshot.clone();
    let _ = crate::events::AppUpdateChecked(snapshot.clone()).emit(app);
    Ok(snapshot)
}

pub async fn check_quietly(app: &AppHandle) {
    let _ = check(app).await;
}

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    #[specta(type = crate::wire_enums::AppUpdateInstallPhase)]
    phase: &'static str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: Option<f64>,
}

pub async fn install(app: &AppHandle, version: String) -> Result<(), String> {
    let state = app.state::<UpdateState>();
    let _operation = state.operation.lock().await;
    let update = state
        .update
        .lock()
        .await
        .clone()
        .filter(|update| update.version == version)
        .ok_or("The available update changed. Check for updates again before installing.")?;
    let mut downloaded = 0;
    let mut total = None;
    let bytes = update
        .download(
            |chunk, length| {
                downloaded += chunk as u64;
                total = length;
                let _ = crate::events::AppUpdateInstallProgress(InstallProgress {
                        phase: "downloading",
                        downloaded_bytes: downloaded,
                        total_bytes: length,
                        percent: length
                            .filter(|length| *length > 0)
                            .map(|length| (downloaded as f64 / length as f64 * 100.0).min(100.0)),
                    }).emit(app);
            },
            || {},
        )
        .await
        .map_err(|error| error.to_string())?;
    let _ = crate::events::AppUpdateInstallProgress(InstallProgress {
            phase: "installing",
            downloaded_bytes: downloaded,
            total_bytes: total,
            percent: Some(100.0),
        }).emit(app);
    update.install(bytes).map_err(|error| error.to_string())
}
