use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::time::Instant;
use tauri::{Emitter, State};

use super::{sanitize_log_line, silent_cmd};
use crate::state::SharedState;

#[derive(Clone, Serialize)]
struct PullProgress {
    total_layers: usize,
    completed_layers: usize,
    message: String,
    completed_services: Vec<String>,
}

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

/// Run a docker compose command with PID tracking so it can be cancelled via
/// `cancel_docker_operation`. Replaces bare `.output()` calls that block
/// indefinitely with no way for the user to abort.
pub(super) fn run_docker_compose(
    args: &[&str],
    install_path: &std::path::Path,
    state: &SharedState,
) -> Result<std::process::Output, String> {
    // Bail out early if a cancel was already requested (catches the window
    // between sequential docker commands in multi-step operations).
    if state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .docker_op_cancelled
    {
        return Err("Operation cancelled.".to_string());
    }

    let child = silent_cmd("docker")
        .args(args)
        .current_dir(install_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start docker: {}", e))?;

    let pid = child.id();
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .docker_child_pid = Some(pid);

    let result = child.wait_with_output().map_err(|e| {
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .docker_child_pid = None;
        format!("Docker command failed: {}", e)
    });

    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .docker_child_pid = None;
    result
}

#[tauri::command]
pub async fn start_stack(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = std::path::PathBuf::from(&path);

    // Clear cancel flag at the start of this operation
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .docker_op_cancelled = false;

    let _ = app.emit(
        "docker-progress",
        "Downloading Docker images (this may take several minutes)...",
    );

    // Pull images with streaming progress
    let mut child = silent_cmd("docker")
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

    // Stream stderr for progress, parsing pull progress lines
    if let Some(stderr) = child.stderr.take() {
        let app_clone = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            // Matches layer progress lines like "9e81dfb60284 Extracting 37B"
            let layer_re = Regex::new(
                r"^\s*([0-9a-f]{12})\s+(Downloading|Extracting|Verifying Checksum|Download complete|Pull complete|Already exists|Waiting)"
            ).unwrap();
            let pulled_re = Regex::new(r"^\s*(?:\[\+\]\s+)?(\S+)\s+Pulled").unwrap();
            let mut completed_services: Vec<String> = Vec::new();

            // Track layer states: hash -> current action
            let mut layer_states: HashMap<String, String> = HashMap::new();
            // Track last emit time per hash for throttling
            let mut last_emit_time: HashMap<String, Instant> = HashMap::new();
            let throttle_duration = std::time::Duration::from_secs(1);

            for line in reader.lines().map_while(Result::ok) {
                if let Some(caps) = layer_re.captures(&line) {
                    let hash = caps[1].to_string();
                    let action = caps[2].to_string();

                    let previous_action = layer_states.get(&hash).cloned();
                    let is_new_hash = previous_action.is_none();
                    let is_terminal = action == "Pull complete" || action == "Already exists";
                    let action_changed = previous_action.as_deref() != Some(&action);

                    // Update layer state
                    layer_states.insert(hash.clone(), action.clone());

                    // Emit to docker-log only for: new hash, state transitions to terminal
                    if is_new_hash || is_terminal {
                        let _ = app_clone.emit("docker-log", sanitize_log_line(&line));
                    }

                    // Throttle per-hash progress emissions (for non-terminal lines)
                    if !is_terminal {
                        let now = Instant::now();
                        let should_emit = match last_emit_time.get(&hash) {
                            Some(last) => now.duration_since(*last) >= throttle_duration,
                            None => true,
                        };
                        if should_emit {
                            last_emit_time.insert(hash.clone(), now);
                        }
                    }

                    // Compute and emit structured pull progress
                    let total_layers = layer_states.len();
                    let completed_layers = layer_states.values()
                        .filter(|a| *a == "Pull complete" || *a == "Already exists")
                        .count();

                    // Build action summary
                    let mut action_counts: HashMap<&str, usize> = HashMap::new();
                    for a in layer_states.values() {
                        if a != "Pull complete" && a != "Already exists" {
                            *action_counts.entry(a.as_str()).or_insert(0) += 1;
                        }
                    }
                    let summary_parts: Vec<String> = action_counts.iter()
                        .map(|(a, c)| format!("{} {} layers", a, c))
                        .collect();
                    let summary = if summary_parts.is_empty() {
                        String::new()
                    } else {
                        summary_parts.join(", ")
                    };

                    // Determine the dominant current action for the progress message
                    let progress_msg = if completed_layers == total_layers {
                        format!("All {} layers complete", total_layers)
                    } else if action_counts.get("Extracting").copied().unwrap_or(0) > 0 {
                        format!("Extracting layers ({}/{} complete)...", completed_layers, total_layers)
                    } else {
                        format!("Pulling layers ({}/{} complete)...", completed_layers, total_layers)
                    };

                    // Only emit structured progress on action changes or terminal states
                    if action_changed || is_terminal {
                        let _ = app_clone.emit("docker-progress", &progress_msg);
                        let _ = app_clone.emit("docker-pull-progress", PullProgress {
                            total_layers,
                            completed_layers,
                            message: if summary.is_empty() {
                                progress_msg.clone()
                            } else {
                                format!("{} ({})", progress_msg, summary)
                            },
                            completed_services: completed_services.clone(),
                        });
                    }
                } else {
                    // Non-layer line: always emit to log
                    let _ = app_clone.emit("docker-log", sanitize_log_line(&line));

                    // Parse "servicename Pulled" lines
                    if let Some(caps) = pulled_re.captures(&line) {
                        let service = caps[1].to_string();
                        if !completed_services.contains(&service) {
                            completed_services.push(service);
                        }
                    }
                }
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

    let _ = app.emit("docker-progress", "Cleaning up any stale containers...");

    // Clean up stale containers from a previous failed run to avoid
    // "container name already in use" errors on retry/reinstall.
    let _ = silent_cmd("docker")
        .args(["compose", "--env-file", "postiz.env", "down", "--remove-orphans"])
        .current_dir(&install_path)
        .output();

    let _ = app.emit("docker-progress", "Starting Postiz services...");

    // Start the stack (cancellable via cancel_docker_operation)
    let output = run_docker_compose(
        &["compose", "--env-file", "postiz.env", "up", "-d"],
        &install_path,
        &state,
    )
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

    let output = silent_cmd("docker")
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

    let output = silent_cmd("docker")
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

    let output = silent_cmd("docker")
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
        let _ = silent_cmd("taskkill")
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

    let output = silent_cmd("docker")
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

    // Clear cancel flag at the start of this multi-step operation
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .docker_op_cancelled = false;

    let _ = app.emit("docker-progress", "Restarting Postiz...");

    // Run docker compose down (cancellable)
    let output = run_docker_compose(
        &["compose", "--env-file", "postiz.env", "down"],
        &install_path,
        &state,
    )
    .map_err(|e| format!("Failed to stop stack: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit("docker-progress", format!("Warning: docker compose down had issues: {}", stderr));
    }

    // Run docker compose up (cancellable — run_docker_compose checks the flag)
    let _ = app.emit("docker-progress", "Starting Postiz services...");
    let output = run_docker_compose(
        &["compose", "--env-file", "postiz.env", "up", "-d"],
        &install_path,
        &state,
    )
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
        // Check cancel flag each iteration so the poll exits promptly
        if state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .docker_op_cancelled
        {
            return Err("Operation cancelled.".to_string());
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let _ = app.emit(
            "docker-progress",
            format!("Health check attempt {}/40...", attempt),
        );

        // Check container status
        let ps_output = silent_cmd("docker")
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

#[tauri::command]
pub fn cancel_docker_operation(state: State<SharedState>) -> Result<String, String> {
    let pid = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.docker_child_pid
    };

    // Set the cancelled flag so in-flight multi-step operations bail out
    // between steps (e.g. after `down` finishes but before `up` starts,
    // or during health-check polling).
    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.docker_op_cancelled = true;
    }

    if let Some(pid) = pid {
        let _ = silent_cmd("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();

        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .docker_child_pid = None;

        Ok("Docker operation cancelled.".to_string())
    } else {
        Ok("No active Docker operation to cancel.".to_string())
    }
}
