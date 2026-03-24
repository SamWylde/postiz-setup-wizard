use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;

static UPDATING: AtomicBool = AtomicBool::new(false);

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
    if UPDATING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("Update already in progress".to_string());
    }

    let _ = app.emit("update-status", "checking");

    let updater = app.updater().map_err(|e| {
        UPDATING.store(false, Ordering::SeqCst);
        format!("Updater not available: {}", e)
    })?;

    let update = updater
        .check()
        .await
        .map_err(|e| {
            UPDATING.store(false, Ordering::SeqCst);
            format!("Failed to check for updates: {}", e)
        })?;

    let update = update.ok_or_else(|| {
        UPDATING.store(false, Ordering::SeqCst);
        "No update available".to_string()
    })?;

    let _ = app.emit("update-status", "downloading");

    // Track cumulative downloaded bytes (chunk_length is per-chunk, not cumulative)
    let downloaded = Arc::new(AtomicU64::new(0));
    let downloaded_clone = downloaded.clone();
    let app_handle = app.clone();

    let bytes = update
        .download(
            move |chunk_length, content_length| {
                let total = content_length.unwrap_or(0);
                let cumulative = downloaded_clone.fetch_add(chunk_length as u64, Ordering::Relaxed)
                    + chunk_length as u64;
                let percent = if total > 0 {
                    ((cumulative as f64 / total as f64) * 100.0).min(100.0) as u8
                } else {
                    0
                };
                let _ = app_handle.emit(
                    "update-progress",
                    UpdateProgress {
                        percent,
                        downloaded: cumulative,
                        total,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|e| {
            UPDATING.store(false, Ordering::SeqCst);
            let _ = app.emit("update-status", "error");
            format!("Failed to download update: {}", e)
        })?;

    // Emit installing status before install — on Windows, install() calls
    // process::exit(0) so nothing after it will execute.
    let _ = app.emit("update-status", "installing");

    update.install(bytes).map_err(|e| {
        UPDATING.store(false, Ordering::SeqCst);
        let _ = app.emit("update-status", "error");
        format!("Failed to install update: {}", e)
    })?;

    // This code only runs on platforms where install() doesn't exit the process.
    // On Windows NSIS/MSI, the process will have already exited above.
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}

#[tauri::command]
pub fn get_current_app_version(app: tauri::AppHandle) -> Result<String, String> {
    Ok(app.package_info().version.to_string())
}
