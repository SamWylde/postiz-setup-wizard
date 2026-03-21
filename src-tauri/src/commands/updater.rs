use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub body: Option<String>,
}

#[derive(Clone, Serialize)]
struct UpdateProgress {
    percent: u8,
    downloaded: u64,
    total: u64,
}

#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current_version = app.package_info().version.to_string();

    let _ = app.emit("update-status", "checking");

    let updater = app.updater().map_err(|e| format!("Updater not available: {}", e))?;

    let update = updater
        .check()
        .await
        .map_err(|e| {
            let _ = app.emit("update-status", "error");
            format!("Failed to check for updates: {}", e)
        })?;

    match update {
        Some(update) => {
            let _ = app.emit("update-status", "available");
            Ok(UpdateInfo {
                available: true,
                current_version,
                latest_version: update.version.clone(),
                body: update.body.clone(),
            })
        }
        None => {
            let _ = app.emit("update-status", "idle");
            Ok(UpdateInfo {
                available: false,
                current_version: current_version.clone(),
                latest_version: current_version,
                body: None,
            })
        }
    }
}

#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let _ = app.emit("update-status", "checking");

    let updater = app.updater().map_err(|e| format!("Updater not available: {}", e))?;

    let update = updater
        .check()
        .await
        .map_err(|e| format!("Failed to check for updates: {}", e))?;

    let update = update.ok_or("No update available")?;

    let _ = app.emit("update-status", "downloading");

    let app_handle = app.clone();
    update
        .download_and_install(
            move |chunk_length, content_length| {
                let total = content_length.unwrap_or(0);
                let percent = if total > 0 {
                    ((chunk_length as f64 / total as f64) * 100.0) as u8
                } else {
                    0
                };
                let _ = app_handle.emit(
                    "update-progress",
                    UpdateProgress {
                        percent,
                        downloaded: chunk_length as u64,
                        total,
                    },
                );
            },
            || {
                // Download finished
            },
        )
        .await
        .map_err(|e| {
            let _ = app.emit("update-status", "error");
            format!("Failed to install update: {}", e)
        })?;

    let _ = app.emit("update-status", "completed");

    // Restart the app after a brief delay
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        app_handle.restart();
    });

    Ok(())
}

#[tauri::command]
pub fn get_current_app_version(app: tauri::AppHandle) -> Result<String, String> {
    Ok(app.package_info().version.to_string())
}
