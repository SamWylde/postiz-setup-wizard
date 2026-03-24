use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tauri::{Emitter, State};

use crate::state::SharedState;

use super::sanitize_log_line;

#[derive(Debug, Serialize, Clone)]
pub struct ContainerInfo {
    pub name: String,
    pub state: String,
    pub status: String,
    pub health: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct StackStatus {
    pub containers: Vec<ContainerInfo>,
    pub all_healthy: bool,
    pub postiz_responding: bool,
}

#[tauri::command]
pub async fn start_stack(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = std::path::PathBuf::from(&path);

    let _ = app.emit(
        "docker-progress",
        "Downloading Docker images (this may take several minutes)...",
    );

    // Pull images with streaming progress
    let mut child = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "pull"])
        .current_dir(&install_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            "Could not start Docker. Make sure Docker Desktop is running and try again.".to_string()
        })?;

    // Store PID for cancel support
    let pid = child.id();
    state.lock().unwrap_or_else(|e| e.into_inner()).docker_child_pid = Some(pid);

    // Stream stderr for progress
    if let Some(stderr) = child.stderr.take() {
        let app_clone = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = app_clone.emit("docker-log", sanitize_log_line(&line));
            }
        });
    }

    // Stream stdout too
    if let Some(stdout) = child.stdout.take() {
        let app_clone = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let _ = app_clone.emit("docker-log", sanitize_log_line(&line));
            }
        });
    }

    let status = child.wait().map_err(|e| {
        state.lock().unwrap_or_else(|e| e.into_inner()).docker_child_pid = None;
        format!("Docker pull failed: {}", e)
    })?;

    state.lock().unwrap_or_else(|e| e.into_inner()).docker_child_pid = None;

    if !status.success() {
        return Err(
            "Failed to download Docker images. Make sure Docker Desktop is running and you have an internet connection."
                .to_string(),
        );
    }

    let _ = app.emit("docker-progress", "Starting Postiz services...");

    // Start the stack
    let output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d"])
        .current_dir(&install_path)
        .output()
        .map_err(|_| {
            "Could not start Docker services. Make sure Docker Desktop is running.".to_string()
        })?;

    if output.status.success() {
        let _ = app.emit(
            "docker-progress",
            "Services started. Waiting for health checks...",
        );
        Ok("Stack started successfully.".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit("docker-log", sanitize_log_line(&format!("Start error: {}", stderr)));
        Err(
            "Failed to start Postiz services. See Technical Details for more information."
                .to_string(),
        )
    }
}

#[tauri::command]
pub async fn get_stack_status(
    path: String,
    state: State<'_, SharedState>,
) -> Result<StackStatus, String> {
    let install_path = std::path::PathBuf::from(&path);

    let output = Command::new("docker")
        .args(["compose", "ps", "--format", "json"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to get container status: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut containers = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let name = val["Name"].as_str().unwrap_or("unknown").to_string();
            let container_state = val["State"].as_str().unwrap_or("unknown").to_string();
            let status = val["Status"].as_str().unwrap_or("unknown").to_string();
            let health = val["Health"].as_str().unwrap_or("").to_string();

            containers.push(ContainerInfo {
                name,
                state: container_state,
                status,
                health,
            });
        }
    }

    let all_healthy = !containers.is_empty()
        && containers.iter().all(|c| {
            c.state == "running" && (c.health.is_empty() || c.health == "healthy")
        });

    let port = state.lock().unwrap_or_else(|e| e.into_inner()).port;
    let postiz_responding = reqwest::Client::new()
        .get(format!("http://localhost:{}", port))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map(|r| r.status().is_success() || r.status().is_redirection())
        .unwrap_or(false);

    Ok(StackStatus {
        containers,
        all_healthy,
        postiz_responding,
    })
}

/// Inner function callable from both Tauri commands and tray menu handlers
pub fn stop_stack_inner(path: &str) -> Result<String, String> {
    let install_path = std::path::PathBuf::from(path);

    let output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "down"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to stop stack: {}", e))?;

    if output.status.success() {
        Ok("Stack stopped.".to_string())
    } else {
        Err(format!(
            "Failed to stop stack: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[tauri::command]
pub async fn stop_stack(path: String) -> Result<String, String> {
    stop_stack_inner(&path)
}

/// Inner function callable from both Tauri commands and tray menu handlers
pub async fn repair_stack_inner(path: &str, app: &tauri::AppHandle) -> Result<String, String> {
    let _ = app.emit("docker-progress", "Restarting Postiz...");
    let _ = stop_stack_inner(path);

    let install_path = std::path::PathBuf::from(path);
    let _ = app.emit("docker-progress", "Starting Postiz services...");

    let output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to start stack: {}", e))?;

    if output.status.success() {
        let _ = app.emit("docker-progress", "Services restarted.");
        Ok("Stack restarted successfully.".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to restart: {}", stderr))
    }
}

#[tauri::command]
pub async fn repair_stack(path: String, app: tauri::AppHandle) -> Result<String, String> {
    repair_stack_inner(&path, &app).await
}

#[tauri::command]
pub fn cancel_install(state: State<SharedState>) -> Result<String, String> {
    let pid = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.docker_child_pid
    };

    if let Some(pid) = pid {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();

        state.lock().unwrap_or_else(|e| e.into_inner()).docker_child_pid = None;

        Ok("Installation cancelled.".to_string())
    } else {
        Ok("No active installation to cancel.".to_string())
    }
}

#[tauri::command]
pub async fn get_docker_logs(path: String) -> Result<Vec<String>, String> {
    let install_path = std::path::PathBuf::from(&path);

    let output = Command::new("docker")
        .args(["compose", "logs", "--tail", "100"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to get logs: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().map(|l| l.to_string()).collect())
}

#[tauri::command]
pub async fn restart_and_verify(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = std::path::PathBuf::from(&path);

    let _ = app.emit("docker-progress", "Restarting Postiz...");

    // Run docker compose down
    let output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "down"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to stop stack: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit("docker-progress", format!("Warning: docker compose down had issues: {}", stderr));
    }

    // Run docker compose up
    let _ = app.emit("docker-progress", "Starting Postiz services...");
    let output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d"])
        .current_dir(&install_path)
        .output()
        .map_err(|e| format!("Failed to start stack: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to start stack: {}", stderr));
    }

    let _ = app.emit("docker-progress", "Services started. Polling health checks...");

    // Get port from state
    let port = state.lock().unwrap_or_else(|e| e.into_inner()).port;

    // Poll health (up to 40 attempts, 3 seconds apart)
    let client = reqwest::Client::new();
    for attempt in 1..=40 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let _ = app.emit(
            "docker-progress",
            format!("Health check attempt {}/40...", attempt),
        );

        // Check container status
        let ps_output = Command::new("docker")
            .args(["compose", "ps", "--format", "json"])
            .current_dir(&install_path)
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
                && containers.iter().all(|(s, h)| {
                    s == "running" && (h.is_empty() || h == "healthy")
                })
        } else {
            false
        };

        // Check if Postiz is responding
        let postiz_responding = client
            .get(format!("http://localhost:{}", port))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().is_redirection())
            .unwrap_or(false);

        if all_healthy && postiz_responding {
            let _ = app.emit("docker-progress", "All services healthy and responding!");
            return Ok("Stack restarted and verified successfully.".to_string());
        }
    }

    Err("Health check timed out after 40 attempts (2 minutes). Services may still be starting.".to_string())
}
