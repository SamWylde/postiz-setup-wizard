use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use tauri::{Emitter, State};

use super::silent_cmd;
use crate::state::SharedState;

#[derive(Debug, Serialize, Clone)]
pub struct PostizUpdateInfo {
    pub update_available: bool,
    pub local_digest: String,
    pub remote_digest: String,
    pub image: String,
}

/// Check if a newer Postiz Docker image is available by comparing local and remote digests.
#[tauri::command]
pub async fn check_postiz_update(
    _state: State<'_, SharedState>,
) -> Result<PostizUpdateInfo, String> {
    let image = "ghcr.io/gitroomhq/postiz-app:latest";

    // Get local image digest
    let local_output = silent_cmd("docker")
        .args([
            "image",
            "inspect",
            image,
            "--format",
            "{{index .RepoDigests 0}}",
        ])
        .output()
        .map_err(|e| format!("Failed to inspect local image: {}", e))?;

    let local_digest = if local_output.status.success() {
        let raw = String::from_utf8_lossy(&local_output.stdout).trim().to_string();
        // RepoDigests format: "ghcr.io/gitroomhq/postiz-app@sha256:abc123..."
        raw.split('@')
            .nth(1)
            .unwrap_or("unknown")
            .to_string()
    } else {
        "not-installed".to_string()
    };

    // Get remote image digest via docker manifest inspect
    let remote_output = silent_cmd("docker")
        .args(["manifest", "inspect", image, "--verbose"])
        .output()
        .map_err(|e| format!("Failed to check remote image: {}", e))?;

    let remote_digest = if remote_output.status.success() {
        let raw = String::from_utf8_lossy(&remote_output.stdout);
        // Try to extract digest from the verbose manifest output
        // Look for "digest": "sha256:..."
        extract_digest_from_manifest(&raw).unwrap_or_else(|| {
            // Fallback: use docker pull --dry-run approach by checking
            // if `docker pull` would download new layers
            "unknown".to_string()
        })
    } else {
        // Fallback: try `docker pull --dry-run` equivalent
        // We'll use `docker manifest inspect` without --verbose
        let simple_output = silent_cmd("docker")
            .args(["manifest", "inspect", image])
            .output()
            .ok();

        if let Some(out) = simple_output {
            if out.status.success() {
                let raw = String::from_utf8_lossy(&out.stdout);
                extract_digest_from_manifest(&raw).unwrap_or("unknown".to_string())
            } else {
                "unavailable".to_string()
            }
        } else {
            "unavailable".to_string()
        }
    };

    let update_available = !local_digest.is_empty()
        && !remote_digest.is_empty()
        && local_digest != "not-installed"
        && local_digest != "unknown"
        && remote_digest != "unknown"
        && remote_digest != "unavailable"
        && local_digest != remote_digest;

    Ok(PostizUpdateInfo {
        update_available,
        local_digest,
        remote_digest,
        image: image.to_string(),
    })
}

fn extract_digest_from_manifest(json: &str) -> Option<String> {
    // Try parsing as JSON to get the config digest
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json) {
        // Handle array format (--verbose)
        if let Some(arr) = val.as_array() {
            for entry in arr {
                if let Some(descriptor) = entry.get("Descriptor") {
                    if let Some(digest) = descriptor.get("digest").and_then(|d| d.as_str()) {
                        return Some(digest.to_string());
                    }
                }
            }
        }
        // Handle single object format
        if let Some(config) = val.get("config") {
            if let Some(digest) = config.get("digest").and_then(|d| d.as_str()) {
                return Some(digest.to_string());
            }
        }
        // Try top-level digest
        if let Some(digest) = val.get("digest").and_then(|d| d.as_str()) {
            return Some(digest.to_string());
        }
    }
    None
}

/// Upgrade the Postiz app container: snapshot current image for rollback, pull
/// the latest image, recreate only the postiz service, verify health, and
/// automatically roll back if the new version fails to start.
#[tauri::command]
pub async fn upgrade_postiz(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.install_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .ok_or("No install path configured")?
    };

    let path = std::path::PathBuf::from(&install_path);
    let image = "ghcr.io/gitroomhq/postiz-app:latest";
    let rollback_tag = "ghcr.io/gitroomhq/postiz-app:rollback";

    // Phase 0: Capture current image ID and create rollback tag
    let _ = app.emit("upgrade-progress", serde_json::json!({
        "phase": "snapshot",
        "message": "Saving current image as rollback snapshot..."
    }));

    let inspect_output = silent_cmd("docker")
        .args(["image", "inspect", image, "--format", "{{.Id}}"])
        .output()
        .map_err(|e| format!("Failed to inspect current image: {}", e))?;

    let current_image_id = if inspect_output.status.success() {
        String::from_utf8_lossy(&inspect_output.stdout).trim().to_string()
    } else {
        return Err("Cannot determine current Postiz image ID. Is Postiz installed?".to_string());
    };

    let tag_output = silent_cmd("docker")
        .args(["tag", &current_image_id, rollback_tag])
        .output()
        .map_err(|e| format!("Failed to tag rollback image: {}", e))?;

    if !tag_output.status.success() {
        let stderr = String::from_utf8_lossy(&tag_output.stderr);
        return Err(format!("Failed to create rollback tag: {}", stderr));
    }

    // Phase 1: Pull only the postiz service image
    let _ = app.emit("upgrade-progress", serde_json::json!({
        "phase": "pulling",
        "message": "Downloading updated Postiz app image..."
    }));

    let mut child = silent_cmd("docker")
        .args(["compose", "--env-file", "postiz.env", "pull", "postiz"])
        .current_dir(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            "Could not start Docker. Make sure Docker Desktop is running.".to_string()
        })?;

    // Stream stderr
    if let Some(stderr) = child.stderr.take() {
        let app_clone = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = app_clone.emit("upgrade-progress", serde_json::json!({
                    "phase": "pulling",
                    "message": line
                }));
            }
        });
    }

    // Stream stdout
    if let Some(stdout) = child.stdout.take() {
        let app_clone = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let _ = app_clone.emit("upgrade-progress", serde_json::json!({
                    "phase": "pulling",
                    "message": line
                }));
            }
        });
    }

    let status = child
        .wait()
        .map_err(|e| format!("Docker pull failed: {}", e))?;

    if !status.success() {
        // Clean up rollback tag on pull failure
        let _ = silent_cmd("docker").args(["rmi", rollback_tag]).output();
        return Err("Failed to pull updated Postiz image. Check your internet connection.".to_string());
    }

    // Phase 2: Recreate only the postiz container (no-deps keeps other services untouched)
    let _ = app.emit("upgrade-progress", serde_json::json!({
        "phase": "restarting",
        "message": "Recreating Postiz app container..."
    }));

    let up_output = silent_cmd("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d", "--no-deps", "postiz"])
        .current_dir(&path)
        .output()
        .map_err(|e| format!("Failed to recreate postiz container: {}", e))?;

    if !up_output.status.success() {
        let stderr = String::from_utf8_lossy(&up_output.stderr);
        // Attempt rollback
        let _ = app.emit("upgrade-progress", serde_json::json!({
            "phase": "rollback",
            "message": "Container recreation failed — rolling back..."
        }));
        let _ = silent_cmd("docker").args(["tag", rollback_tag, image]).output();
        let _ = silent_cmd("docker")
            .args(["compose", "--env-file", "postiz.env", "up", "-d", "--no-deps", "postiz"])
            .current_dir(&path)
            .output();
        let _ = silent_cmd("docker").args(["rmi", rollback_tag]).output();
        return Err(format!("Failed to start updated Postiz container (rolled back): {}", stderr));
    }

    // Phase 3: Health checks — poll for healthy + responding
    let _ = app.emit("upgrade-progress", serde_json::json!({
        "phase": "health-checks",
        "message": "Waiting for Postiz app to become healthy..."
    }));

    let port = state.lock().unwrap_or_else(|e| e.into_inner()).port;
    let client = reqwest::Client::new();

    let mut healthy = false;
    for attempt in 1..=40 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let _ = app.emit("upgrade-progress", serde_json::json!({
            "phase": "health-checks",
            "message": format!("Health check {}/40...", attempt)
        }));

        // Check all containers are still running
        let ps_output = silent_cmd("docker")
            .args(["compose", "ps", "--format", "json"])
            .current_dir(&path)
            .output();

        let all_healthy = if let Ok(ps_out) = ps_output {
            let stdout = String::from_utf8_lossy(&ps_out.stdout);
            let mut containers = Vec::new();
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    let container_state = val["State"].as_str().unwrap_or("unknown");
                    let health = val["Health"].as_str().unwrap_or("");
                    containers.push((container_state.to_string(), health.to_string()));
                }
            }
            !containers.is_empty()
                && containers
                    .iter()
                    .all(|(s, h)| s == "running" && (h.is_empty() || h == "healthy"))
        } else {
            false
        };

        let postiz_responding = client
            .get(format!("http://localhost:{}", port))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().is_redirection())
            .unwrap_or(false);

        if all_healthy && postiz_responding {
            healthy = true;
            break;
        }
    }

    if healthy {
        // Success — clean up rollback tag
        let _ = silent_cmd("docker").args(["rmi", rollback_tag]).output();
        let _ = app.emit("upgrade-progress", serde_json::json!({
            "phase": "complete",
            "message": "Upgrade complete! Postiz app is healthy."
        }));
        return Ok("Postiz app upgraded successfully.".to_string());
    }

    // Health checks failed — roll back
    let _ = app.emit("upgrade-progress", serde_json::json!({
        "phase": "rollback",
        "message": "Health checks failed — rolling back to previous version..."
    }));

    let _ = silent_cmd("docker").args(["tag", rollback_tag, image]).output();
    let _ = silent_cmd("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d", "--no-deps", "postiz"])
        .current_dir(&path)
        .output();
    let _ = silent_cmd("docker").args(["rmi", rollback_tag]).output();

    Err("Upgrade health checks timed out — rolled back to previous version. Check logs for details.".to_string())
}
