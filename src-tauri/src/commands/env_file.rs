use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tauri::{Emitter, State};

use crate::state::SharedState;

fn parse_env_file(contents: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

fn write_env_file(path: &PathBuf, updates: &HashMap<String, String>) -> Result<(), String> {
    let contents = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read env file: {}", e))?;

    // Create backup
    let backup_path = path.with_extension("env.bak");
    fs::copy(path, &backup_path).ok(); // Best effort backup

    let mut new_lines = Vec::new();
    let mut updated_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            new_lines.push(line.to_string());
            continue;
        }

        if let Some((key, _)) = trimmed.split_once('=') {
            let key = key.trim();
            if let Some(new_value) = updates.get(key) {
                new_lines.push(format!("{}={}", key, new_value));
                updated_keys.insert(key.to_string());
            } else {
                new_lines.push(line.to_string());
            }
        } else {
            new_lines.push(line.to_string());
        }
    }

    // Append any new keys that weren't in the original file
    for (key, value) in updates {
        if !updated_keys.contains(key) {
            new_lines.push(format!("{}={}", key, value));
        }
    }

    fs::write(path, new_lines.join("\n"))
        .map_err(|e| format!("Failed to write env file: {}", e))?;

    Ok(())
}

#[tauri::command]
pub fn stage_provider_config(
    provider: String,
    entries: HashMap<String, String>,
    state: State<SharedState>,
) -> Result<String, String> {
    let mut app_state = state.lock().map_err(|e| format!("State lock failed: {}", e))?;

    // Stage the changes (don't write to disk yet)
    // Provider is NOT marked as configured until apply succeeds.
    for (key, value) in entries {
        app_state.pending_env_changes.insert(key, value);
    }

    Ok(format!("Provider {} config staged.", provider))
}

#[tauri::command]
pub fn apply_provider_changes(path: String, state: State<SharedState>) -> Result<String, String> {
    let install_path = PathBuf::from(&path);
    let env_path = install_path.join("postiz.env");

    let pending = {
        let app_state = state.lock().map_err(|e| format!("State lock failed: {}", e))?;
        app_state.pending_env_changes.clone()
    };

    if pending.is_empty() {
        return Ok("No pending changes to apply.".to_string());
    }

    // Write first, only clear on success
    write_env_file(&env_path, &pending)?;

    // Now that the write succeeded, clear pending changes
    if let Ok(mut app_state) = state.lock() {
        app_state.pending_env_changes.clear();
    }

    Ok(format!("Applied {} env changes.", pending.len()))
}

#[tauri::command]
pub fn update_base_urls(path: String, base_url: String) -> Result<String, String> {
    let install_path = PathBuf::from(&path);
    let env_path = install_path.join("postiz.env");

    let mut updates = HashMap::new();
    updates.insert("MAIN_URL".to_string(), base_url.clone());
    updates.insert("FRONTEND_URL".to_string(), base_url.clone());
    updates.insert(
        "NEXT_PUBLIC_BACKEND_URL".to_string(),
        format!("{}/api", base_url),
    );

    write_env_file(&env_path, &updates)?;

    Ok(format!("Base URLs updated to {}", base_url))
}

#[tauri::command]
pub fn read_env_value(path: String, key: String) -> Result<Option<String>, String> {
    let env_path = PathBuf::from(&path).join("postiz.env");
    let contents = fs::read_to_string(&env_path)
        .map_err(|e| format!("Failed to read env file: {}", e))?;

    let map = parse_env_file(&contents);
    Ok(map.get(&key).cloned())
}

#[tauri::command]
pub async fn apply_config_transaction(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = PathBuf::from(&path);
    let env_path = install_path.join("postiz.env");

    // Get pending changes from state (clone, don't clear yet)
    let (pending, port) = {
        let app_state = state
            .lock()
            .map_err(|e| format!("State lock failed: {}", e))?;
        (app_state.pending_env_changes.clone(), app_state.port)
    };

    if pending.is_empty() {
        return Ok("No pending changes".to_string());
    }

    let _ = app.emit(
        "docker-progress",
        format!("Applying {} config changes...", pending.len()),
    );

    // Verify the env file exists
    if !env_path.exists() {
        return Err(format!("Env file not found: {}", env_path.display()));
    }

    // Create backup
    let backup_path = env_path.with_extension("env.bak");
    fs::copy(&env_path, &backup_path)
        .map_err(|e| format!("Failed to create backup: {}", e))?;

    let _ = app.emit("docker-progress", "Backup created. Writing new config...");

    // Write new values (same merge logic as write_env_file)
    write_env_file(&env_path, &pending)?;

    let _ = app.emit("docker-progress", "Config written. Restarting stack...");

    // Restart stack: docker compose down
    let output = Command::new("docker")
        .args(["compose", "down"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to stop stack: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit(
            "docker-progress",
            format!("Warning: docker compose down had issues: {}", stderr),
        );
    }

    // docker compose up
    let _ = app.emit("docker-progress", "Starting services with new config...");
    let output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to start stack: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit(
            "docker-progress",
            format!("Failed to start stack: {}. Rolling back...", stderr),
        );
        // Rollback: stop failed containers, restore config, restart with old config
        fs::copy(&backup_path, &env_path).ok();
        let _ = Command::new("docker")
            .args(["compose", "down"])
            .current_dir(&install_path)
            .output();
        let _ = Command::new("docker")
            .args(["compose", "--env-file", "postiz.env", "up", "-d"])
            .current_dir(&install_path)
            .output();
        return Err("Config changes caused startup failure. Changes have been rolled back.".to_string());
    }

    // Wait 15 seconds for services to start
    let _ = app.emit("docker-progress", "Waiting 15 seconds for services to start...");
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    // Check health
    let _ = app.emit("docker-progress", "Checking service health...");
    let client = reqwest::Client::new();
    let healthy = client
        .get(format!("http://localhost:{}", port))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map(|r| r.status().is_success() || r.status().is_redirection())
        .unwrap_or(false);

    if healthy {
        // Clear pending changes from state
        if let Ok(mut app_state) = state.lock() {
            app_state.pending_env_changes.clear();
        }
        let _ = app.emit("docker-progress", "Config applied successfully!");
        Ok(format!(
            "Applied {} config changes successfully.",
            pending.len()
        ))
    } else {
        // ROLLBACK
        let _ = app.emit("docker-progress", "Health check failed. Rolling back config...");
        fs::copy(&backup_path, &env_path).ok();

        // Restart stack with old config
        let _ = Command::new("docker")
            .args(["compose", "down"])
            .current_dir(&install_path)
            .output();

        let _ = Command::new("docker")
            .args(["compose", "--env-file", "postiz.env", "up", "-d"])
            .current_dir(&install_path)
            .output();

        Err("Config changes caused health check failure. Changes have been rolled back.".to_string())
    }
}
