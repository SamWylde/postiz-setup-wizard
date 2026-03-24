use std::fs;
use std::path::PathBuf;
use tauri::State;

use crate::state::{ResumeState, SharedState, TunnelProvider};

fn default_resume_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Postiz")
}

fn get_resume_path(install_path: Option<&str>) -> PathBuf {
    if let Some(path) = install_path {
        PathBuf::from(path).join("install-state.json")
    } else {
        default_resume_dir().join("install-state.json")
    }
}

/// Write a pointer file at the default location so we can find custom install paths on restart.
/// Contains just the path to the real install directory.
/// Uses atomic temp-file-then-rename to avoid corruption on crash/power loss.
fn write_install_pointer(install_path: &str) -> Result<(), String> {
    let default_dir = default_resume_dir();
    let pointer_path = default_dir.join("install-pointer.json");

    fs::create_dir_all(&default_dir)
        .map_err(|e| format!("Failed to create pointer directory: {}", e))?;
    let pointer = serde_json::json!({ "install_path": install_path });
    let content = serde_json::to_string_pretty(&pointer)
        .map_err(|e| format!("Failed to serialize pointer: {}", e))?;
    let tmp = pointer_path.with_extension("tmp");
    fs::write(&tmp, &content)
        .map_err(|e| format!("Failed to write pointer temp file: {}", e))?;
    fs::rename(&tmp, &pointer_path)
        .map_err(|e| format!("Failed to rename pointer temp file: {}", e))?;
    Ok(())
}

/// Read the pointer file to discover a custom install path.
fn read_install_pointer() -> Option<String> {
    let pointer_path = default_resume_dir().join("install-pointer.json");
    let contents = fs::read_to_string(&pointer_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&contents).ok()?;
    val["install_path"].as_str().map(|s| s.to_string())
}

#[tauri::command]
pub fn load_resume_state(
    install_path: Option<String>,
    state: State<SharedState>,
) -> Result<Option<ResumeState>, String> {
    // Try the provided path first
    let path = get_resume_path(install_path.as_deref());

    let resume_path = if path.exists() {
        path
    } else if install_path.is_none() {
        // No explicit path given and default doesn't exist — check pointer file
        if let Some(pointer_path) = read_install_pointer() {
            let p = PathBuf::from(&pointer_path).join("install-state.json");
            if p.exists() {
                p
            } else {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }
    } else {
        return Ok(None);
    };

    let contents =
        fs::read_to_string(&resume_path).map_err(|e| format!("Failed to read resume state: {}", e))?;

    let resume: ResumeState =
        serde_json::from_str(&contents).map_err(|e| format!("Failed to parse resume state: {}", e))?;

    // Restore Rust AppState from resume data so tray menu and save work correctly
    {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref ip) = resume.install_path {
            app_state.install_path = Some(PathBuf::from(ip));
        }
        app_state.port = resume.port;
        app_state.local_url = Some(format!("http://localhost:{}", resume.port));
        // Do NOT restore tunnel_url — it must be verified as alive first.
        // The frontend will check via get_tunnel_status after load.
        app_state.current_step = resume.current_step;
        for p in &resume.providers_configured {
            app_state.providers_configured.insert(p.clone());
        }
        for p in &resume.providers_stale {
            app_state.stale_providers.insert(p.clone());
        }
        app_state.tunnel_mode = resume.tunnel_mode.clone();
        app_state.tunnel_provider = TunnelProvider::from_str_loose(&resume.tunnel_provider);
        app_state.permanent_domain = resume.permanent_domain.clone();
        app_state.reboot_pending = resume.reboot_pending_for.clone();
        app_state.transfer_review_pending = resume.transfer_review_pending;
    }

    Ok(Some(resume))
}

#[tauri::command]
pub fn save_resume_state(state: State<SharedState>) -> Result<String, String> {
    let app_state = state.lock().unwrap_or_else(|e| e.into_inner());

    let install_path = app_state
        .install_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    let resume = ResumeState {
        version: 1,
        current_step: app_state.current_step,
        install_path: install_path.clone(),
        port: app_state.port,
        tunnel_url: app_state.tunnel_url.clone(),
        tunnel_mode: app_state.tunnel_mode.clone(),
        permanent_domain: app_state.permanent_domain.clone(),
        providers_configured: app_state.providers_configured.iter().cloned().collect(),
        providers_stale: app_state.stale_providers.iter().cloned().collect(),
        reboot_pending_for: app_state.reboot_pending.clone(),
        transfer_review_pending: app_state.transfer_review_pending,
        tunnel_provider: app_state.tunnel_provider.as_str().to_string(),
        last_updated: chrono::Utc::now().to_rfc3339(),
    };

    let path = get_resume_path(install_path.as_deref());

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let json = serde_json::to_string_pretty(&resume)
        .map_err(|e| format!("Failed to serialize state: {}", e))?;

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &json).map_err(|e| format!("Failed to write resume state temp file: {}", e))?;
    fs::rename(&tmp, &path).map_err(|e| format!("Failed to rename resume state temp file: {}", e))?;

    // If using a custom path, write a pointer at the default location
    if let Some(ref ip) = install_path {
        let default_path_str = default_resume_dir().to_string_lossy().to_string();
        if !ip.starts_with(&default_path_str) {
            write_install_pointer(ip)?;
        }
    }

    Ok("Resume state saved.".to_string())
}

#[tauri::command]
pub fn update_step(step: usize, state: State<SharedState>) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.current_step = step;
    Ok(())
}

/// Sync frontend tunnel config to Rust state so snapshot/resume captures it correctly.
#[tauri::command]
pub fn sync_tunnel_config(
    tunnel_mode: String,
    permanent_domain: Option<String>,
    tunnel_provider: Option<String>,
    state: State<SharedState>,
) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.tunnel_mode = tunnel_mode;
    app_state.permanent_domain = permanent_domain;
    if let Some(ref provider) = tunnel_provider {
        app_state.tunnel_provider = TunnelProvider::from_str_loose(provider);
    }
    Ok(())
}

/// Sync frontend provider status to Rust state so save_resume_state captures it correctly.
#[tauri::command]
pub fn sync_provider_status(
    configured: Vec<String>,
    stale: Vec<String>,
    state: State<SharedState>,
) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.providers_configured = configured.into_iter().collect();
    app_state.stale_providers = stale.into_iter().collect();
    Ok(())
}

/// Atomically clear `transfer_review_pending` and persist resume state to disk.
/// This avoids the race where a separate `save_resume_state` call could persist
/// the old `true` value before the flag clear takes effect.
#[tauri::command]
pub fn clear_transfer_review_and_save(state: State<SharedState>) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.transfer_review_pending = false;

    // Build and persist resume state while still holding the lock
    let install_path = app_state
        .install_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    let resume = ResumeState {
        version: 1,
        current_step: app_state.current_step,
        install_path: install_path.clone(),
        port: app_state.port,
        tunnel_url: app_state.tunnel_url.clone(),
        tunnel_mode: app_state.tunnel_mode.clone(),
        permanent_domain: app_state.permanent_domain.clone(),
        providers_configured: app_state.providers_configured.iter().cloned().collect(),
        providers_stale: app_state.stale_providers.iter().cloned().collect(),
        reboot_pending_for: app_state.reboot_pending.clone(),
        transfer_review_pending: false,
        tunnel_provider: app_state.tunnel_provider.as_str().to_string(),
        last_updated: chrono::Utc::now().to_rfc3339(),
    };

    // Drop the lock before doing I/O
    drop(app_state);

    let path = get_resume_path(install_path.as_deref());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let json = serde_json::to_string_pretty(&resume)
        .map_err(|e| format!("Failed to serialize state: {}", e))?;

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &json).map_err(|e| format!("Failed to write resume state temp file: {}", e))?;
    fs::rename(&tmp, &path).map_err(|e| format!("Failed to rename resume state temp file: {}", e))?;

    // If using a custom path, write a pointer at the default location
    if let Some(ref ip) = install_path {
        let default_path_str = default_resume_dir().to_string_lossy().to_string();
        if !ip.starts_with(&default_path_str) {
            write_install_pointer(ip)?;
        }
    }

    Ok(())
}
