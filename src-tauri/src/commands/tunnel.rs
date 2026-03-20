use regex::Regex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tauri::{Emitter, State};

use crate::state::SharedState;

#[derive(Debug, serde::Serialize, Clone)]
pub struct TunnelStatus {
    pub status: String, // "running", "starting", "stopped", "error"
    pub url: Option<String>,
}

/// Check if a process with the given PID is still alive.
/// Uses tasklist with PID filter in CSV format for reliable parsing.
/// When no process matches, tasklist outputs an "INFO:" line.
fn is_pid_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH", "/FO", "CSV"])
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // CSV format quotes each field: "name.exe","12345","Console","1","12,345 K"
            // Check for the PID as a quoted field to avoid false positives from
            // process names or memory values that happen to contain the PID digits.
            !stdout.contains("INFO:") && stdout.contains(&format!("\"{}\"", pid))
        })
        .unwrap_or(false)
}

#[tauri::command]
pub async fn start_tunnel(
    port: u16,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let _ = app.emit("tunnel-status", "starting");

    // Stop any existing tunnel before starting a new one to prevent orphaned processes
    {
        let existing_pid = state
            .lock()
            .ok()
            .and_then(|s| s.tunnel_pid);
        if let Some(pid) = existing_pid {
            if is_pid_alive(pid) {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output();
            }
        }
        if let Ok(mut app_state) = state.lock() {
            app_state.tunnel_url = None;
            app_state.tunnel_pid = None;
        }
    }

    // Set up isolated cloudflared home directory so user's existing config doesn't interfere
    let cloudflared_home = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("PostizWizard")
        .join("cloudflared");

    std::fs::create_dir_all(&cloudflared_home)
        .map_err(|e| format!("Failed to create cloudflared home: {}", e))?;

    let mut child = Command::new("cloudflared")
        .args(["tunnel", "--url", &format!("http://localhost:{}", port)])
        .env("CLOUDFLARED_HOME", &cloudflared_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start cloudflared: {}", e))?;

    // Store PID for targeted cleanup
    let pid = child.id();
    if let Ok(mut app_state) = state.lock() {
        app_state.tunnel_pid = Some(pid);
        app_state.tunnel_url = None; // Will be set once URL is captured
    }

    // Read stderr in a thread to capture the tunnel URL
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;
    let app_clone = app.clone();
    let state_url = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let state_url_clone = state_url.clone();

    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let url_re = Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();

        for line in reader.lines() {
            if let Ok(line) = line {
                // Try to find tunnel URL
                if let Some(mat) = url_re.find(&line) {
                    let url = mat.as_str().to_string();
                    let _ = app_clone.emit("tunnel-url", url.clone());
                    let _ = app_clone.emit("tunnel-status", "running");
                    if let Ok(mut u) = state_url_clone.lock() {
                        *u = Some(url);
                    }
                }
                let _ = app_clone.emit("tunnel-log", line);
            }
        }
        // Tunnel process ended
        let _ = app_clone.emit("tunnel-status", "stopped");
    });

    // Wait briefly for the URL to be captured
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let captured_url = state_url.lock().ok().and_then(|u| u.clone());
    if let Some(ref url) = captured_url {
        if let Ok(mut app_state) = state.lock() {
            app_state.tunnel_url = Some(url.clone());
        }
        Ok(url.clone())
    } else {
        // Give it more time
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let captured_url = state_url.lock().ok().and_then(|u| u.clone());
        match captured_url {
            Some(url) => {
                if let Ok(mut app_state) = state.lock() {
                    app_state.tunnel_url = Some(url.clone());
                }
                Ok(url)
            }
            None => {
                // URL capture failed — kill the orphaned process to avoid leaks
                if let Ok(s) = state.lock() {
                    if let Some(pid) = s.tunnel_pid {
                        let _ = Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/T", "/F"])
                            .output();
                    }
                }
                if let Ok(mut app_state) = state.lock() {
                    app_state.tunnel_pid = None;
                }
                Err("Tunnel started but URL not captured. The process has been stopped — please try again.".to_string())
            },
        }
    }
}

#[tauri::command]
pub async fn stop_tunnel(state: State<'_, SharedState>) -> Result<String, String> {
    // Kill only the cloudflared process we spawned, by PID
    let pid = {
        let s = state
            .lock()
            .map_err(|e| format!("State lock failed: {}", e))?;
        s.tunnel_pid
    };

    if let Some(pid) = pid {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }

    if let Ok(mut app_state) = state.lock() {
        app_state.tunnel_url = None;
        app_state.tunnel_pid = None;
    }

    Ok("Tunnel stopped.".to_string())
}

#[tauri::command]
pub fn get_tunnel_status(state: State<SharedState>) -> Result<TunnelStatus, String> {
    let app_state = state.lock().map_err(|e| format!("State lock failed: {}", e))?;

    let pid = app_state.tunnel_pid;
    let url = app_state.tunnel_url.clone();

    // Only report running if we have a PID and it's still alive
    let alive = pid.map(|p| is_pid_alive(p)).unwrap_or(false);

    let status = if alive && url.is_some() {
        "running".to_string()
    } else if alive {
        "starting".to_string()
    } else {
        "stopped".to_string()
    };

    // If the process died, return no URL
    let effective_url = if alive { url } else { None };

    Ok(TunnelStatus {
        status,
        url: effective_url,
    })
}

#[tauri::command]
pub async fn reconnect_tunnel(
    port: u16,
    install_path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let _ = app.emit("tunnel-status", "starting");

    // Stop existing tunnel if one is running
    let existing_pid = {
        let s = state
            .lock()
            .map_err(|e| format!("State lock failed: {}", e))?;
        s.tunnel_pid
    };

    if let Some(pid) = existing_pid {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }

    // Clear tunnel state
    if let Ok(mut app_state) = state.lock() {
        app_state.tunnel_url = None;
        app_state.tunnel_pid = None;
    }

    // Set up isolated cloudflared home directory
    let cloudflared_home = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("PostizWizard")
        .join("cloudflared");

    std::fs::create_dir_all(&cloudflared_home)
        .map_err(|e| format!("Failed to create cloudflared home: {}", e))?;

    // Start a new tunnel
    let mut child = Command::new("cloudflared")
        .args(["tunnel", "--url", &format!("http://localhost:{}", port)])
        .env("CLOUDFLARED_HOME", &cloudflared_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start cloudflared: {}", e))?;

    // Store PID
    let pid = child.id();
    if let Ok(mut app_state) = state.lock() {
        app_state.tunnel_pid = Some(pid);
    }

    // Read stderr in a thread to capture the tunnel URL
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;
    let app_clone = app.clone();
    let state_url = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let state_url_clone = state_url.clone();

    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let url_re = Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();

        for line in reader.lines() {
            if let Ok(line) = line {
                if let Some(mat) = url_re.find(&line) {
                    let url = mat.as_str().to_string();
                    let _ = app_clone.emit("tunnel-url", url.clone());
                    let _ = app_clone.emit("tunnel-status", "running");
                    if let Ok(mut u) = state_url_clone.lock() {
                        *u = Some(url);
                    }
                }
                let _ = app_clone.emit("tunnel-log", line);
            }
        }
        let _ = app_clone.emit("tunnel-status", "stopped");
    });

    // Wait for the URL to be captured
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let captured_url = state_url.lock().ok().and_then(|u| u.clone());
    let url = if let Some(url) = captured_url {
        url
    } else {
        // Give it more time
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        state_url
            .lock()
            .ok()
            .and_then(|u| u.clone())
            .ok_or("Tunnel started but URL not captured yet. Check tunnel-url events.".to_string())?
    };

    // Store URL in state
    if let Ok(mut app_state) = state.lock() {
        app_state.tunnel_url = Some(url.clone());
    }

    // Update base URLs in postiz.env
    let env_path = std::path::PathBuf::from(&install_path).join("postiz.env");
    let contents = std::fs::read_to_string(&env_path)
        .map_err(|e| format!("Failed to read env: {}", e))?;
    // backup
    std::fs::copy(&env_path, env_path.with_extension("env.bak")).ok();
    let mut new_lines = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some((key, _)) = trimmed.split_once('=') {
            match key.trim() {
                "MAIN_URL" => {
                    new_lines.push(format!("MAIN_URL={}", url));
                    continue;
                }
                "FRONTEND_URL" => {
                    new_lines.push(format!("FRONTEND_URL={}", url));
                    continue;
                }
                "NEXT_PUBLIC_BACKEND_URL" => {
                    new_lines.push(format!("NEXT_PUBLIC_BACKEND_URL={}/api", url));
                    continue;
                }
                _ => {}
            }
        }
        new_lines.push(line.to_string());
    }
    std::fs::write(&env_path, new_lines.join("\n"))
        .map_err(|e| format!("Failed to write env: {}", e))?;

    // Restart Postiz
    let path = std::path::PathBuf::from(&install_path);
    let _ = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "down"])
        .current_dir(&path)
        .output();

    let _ = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d"])
        .current_dir(&path)
        .output();

    // Emit events
    let _ = app.emit("tunnel-url", url.clone());
    let _ = app.emit("tunnel-status", "running");

    Ok(url)
}
