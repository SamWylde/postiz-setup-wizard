use regex::Regex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tauri::{Emitter, State};

use crate::commands::bootstrap::resolve_binary;
use crate::state::{SharedState, TunnelProvider};

#[derive(Debug, serde::Serialize, Clone)]
pub struct TunnelStatus {
    pub status: String, // "running", "starting", "stopped", "error"
    pub url: Option<String>,
}

/// Where to capture the tunnel URL from.
enum UrlSource {
    /// Read lines from stderr; apply regex to find URL.
    Stderr(String),
    /// Read lines from stdout; apply regex to find URL.
    Stdout(String),
    /// Poll an HTTP API endpoint for JSON with the URL.
    HttpApi(String),
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
            !stdout.contains("INFO:") && stdout.contains(&format!("\"{}\"", pid))
        })
        .unwrap_or(false)
}

/// Kill a tunnel process by PID and clear state.
fn kill_existing_tunnel(state: &SharedState) {
    let existing_pid = state.lock().ok().and_then(|s| s.tunnel_pid);
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

/// Spawn a tunnel process and capture its URL from output or HTTP API.
/// Returns (PID, shared URL holder). The URL may not be captured yet — caller must poll.
fn spawn_tunnel_process(
    cmd: &str,
    args: &[&str],
    env: &[(&str, String)],
    url_source: UrlSource,
    app: &tauri::AppHandle,
) -> Result<(u32, Arc<std::sync::Mutex<Option<String>>>), String> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, val) in env {
        command.env(key, val);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to start {}: {}", cmd, e))?;

    let pid = child.id();
    let state_url = Arc::new(std::sync::Mutex::new(None::<String>));

    match url_source {
        UrlSource::Stderr(pattern) => {
            let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;
            let app_clone = app.clone();
            let url_clone = state_url.clone();

            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                let url_re = Regex::new(&pattern).unwrap();
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if let Some(mat) = url_re.find(&line) {
                            let url = mat.as_str().to_string();
                            let _ = app_clone.emit("tunnel-url", url.clone());
                            let _ = app_clone.emit("tunnel-status", "running");
                            if let Ok(mut u) = url_clone.lock() {
                                *u = Some(url);
                            }
                        }
                        let _ = app_clone.emit("tunnel-log", line);
                    }
                }
                let _ = app_clone.emit("tunnel-status", "stopped");
            });
        }
        UrlSource::Stdout(pattern) => {
            let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
            let app_clone = app.clone();
            let url_clone = state_url.clone();

            // Also drain stderr to prevent blocking
            if let Some(stderr) = child.stderr.take() {
                let app_err = app.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            let _ = app_err.emit("tunnel-log", line);
                        }
                    }
                });
            }

            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                let url_re = Regex::new(&pattern).unwrap();
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if let Some(mat) = url_re.find(&line) {
                            let url = mat.as_str().to_string();
                            let _ = app_clone.emit("tunnel-url", url.clone());
                            let _ = app_clone.emit("tunnel-status", "running");
                            if let Ok(mut u) = url_clone.lock() {
                                *u = Some(url);
                            }
                        }
                        let _ = app_clone.emit("tunnel-log", line);
                    }
                }
                let _ = app_clone.emit("tunnel-status", "stopped");
            });
        }
        UrlSource::HttpApi(_api_url) => {
            // For HTTP API-based URL capture (ngrok), we drain both stdout/stderr
            // and poll the API separately in the caller.
            if let Some(stdout) = child.stdout.take() {
                let app_clone = app.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            let _ = app_clone.emit("tunnel-log", line);
                        }
                    }
                });
            }
            if let Some(stderr) = child.stderr.take() {
                let app_clone = app.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            let _ = app_clone.emit("tunnel-log", line);
                        }
                    }
                });
            }
        }
    }

    Ok((pid, state_url))
}

/// Wait for a URL to appear in the shared holder, with configurable timeouts.
async fn wait_for_url(
    state_url: &Arc<std::sync::Mutex<Option<String>>>,
    initial_wait_secs: u64,
    retry_wait_secs: u64,
) -> Option<String> {
    tokio::time::sleep(std::time::Duration::from_secs(initial_wait_secs)).await;
    let captured = state_url.lock().ok().and_then(|u| u.clone());
    if captured.is_some() {
        return captured;
    }
    tokio::time::sleep(std::time::Duration::from_secs(retry_wait_secs)).await;
    state_url.lock().ok().and_then(|u| u.clone())
}

/// Poll ngrok's local API to get the tunnel URL.
async fn poll_ngrok_api(api_url: &str, max_attempts: u32) -> Option<String> {
    let client = reqwest::Client::new();
    for _ in 0..max_attempts {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Ok(resp) = client.get(api_url).send().await {
            if let Ok(body) = resp.text().await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(url) = json["tunnels"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|t| t["public_url"].as_str())
                    {
                        return Some(url.to_string());
                    }
                }
            }
        }
    }
    None
}

// ── Provider-specific start functions ──

async fn start_cloudflared(
    port: u16,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<String, String> {
    let cloudflared_home = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("PostizWizard")
        .join("cloudflared");
    std::fs::create_dir_all(&cloudflared_home)
        .map_err(|e| format!("Failed to create cloudflared home: {}", e))?;

    let port_str = format!("http://localhost:{}", port);
    let (pid, state_url) = spawn_tunnel_process(
        "cloudflared",
        &["tunnel", "--url", &port_str],
        &[("CLOUDFLARED_HOME", cloudflared_home.to_string_lossy().to_string())],
        UrlSource::Stderr(r"https://[a-z0-9-]+\.trycloudflare\.com".to_string()),
        app,
    )?;

    if let Ok(mut s) = state.lock() {
        s.tunnel_pid = Some(pid);
    }

    match wait_for_url(&state_url, 10, 10).await {
        Some(url) => {
            if let Ok(mut s) = state.lock() {
                s.tunnel_url = Some(url.clone());
            }
            Ok(url)
        }
        None => {
            // Kill orphaned process
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
            if let Ok(mut s) = state.lock() {
                s.tunnel_pid = None;
            }
            Err("Tunnel started but URL not captured. The process has been stopped — please try again.".to_string())
        }
    }
}

async fn start_ngrok(
    port: u16,
    config: Option<String>,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<String, String> {
    let ngrok_bin = resolve_binary("ngrok");

    // If authtoken provided, configure it first
    if let Some(ref token) = config {
        if !token.trim().is_empty() {
            let output = Command::new(&ngrok_bin)
                .args(["config", "add-authtoken", token.trim()])
                .output()
                .map_err(|e| format!("Failed to configure ngrok authtoken: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("Failed to set ngrok authtoken: {}", stderr));
            }
        }
    }

    let port_str = port.to_string();
    let (pid, _state_url) = spawn_tunnel_process(
        &ngrok_bin,
        &["http", &port_str, "--log", "stdout", "--log-format", "json"],
        &[],
        UrlSource::HttpApi("http://localhost:4040/api/tunnels".to_string()),
        app,
    )?;

    if let Ok(mut s) = state.lock() {
        s.tunnel_pid = Some(pid);
    }

    // Poll ngrok API for the tunnel URL
    match poll_ngrok_api("http://localhost:4040/api/tunnels", 8).await {
        Some(url) => {
            if let Ok(mut s) = state.lock() {
                s.tunnel_url = Some(url.clone());
            }
            let _ = app.emit("tunnel-url", url.clone());
            let _ = app.emit("tunnel-status", "running");
            Ok(url)
        }
        None => {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
            if let Ok(mut s) = state.lock() {
                s.tunnel_pid = None;
            }
            Err("ngrok started but tunnel URL not available. Check your authtoken and try again.".to_string())
        }
    }
}

async fn start_zrok(
    port: u16,
    _config: Option<String>,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<String, String> {
    let zrok_bin = resolve_binary("zrok");
    let target = format!("http://localhost:{}", port);

    let (pid, state_url) = spawn_tunnel_process(
        &zrok_bin,
        &["share", "public", &target, "--headless"],
        &[],
        UrlSource::Stdout(r"https://[a-z0-9]+[.]share[.]zrok[.]io".to_string()),
        app,
    )?;

    if let Ok(mut s) = state.lock() {
        s.tunnel_pid = Some(pid);
    }

    match wait_for_url(&state_url, 10, 10).await {
        Some(url) => {
            if let Ok(mut s) = state.lock() {
                s.tunnel_url = Some(url.clone());
            }
            Ok(url)
        }
        None => {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
            if let Ok(mut s) = state.lock() {
                s.tunnel_pid = None;
            }
            Err("zrok started but URL not captured. Make sure you have run 'zrok enable' first.".to_string())
        }
    }
}

async fn start_pinggy(
    port: u16,
    config: Option<String>,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<String, String> {
    let port_str = format!("0:localhost:{}", port);

    let mut args: Vec<&str> = vec![
        "-p", "443",
        "-R", &port_str,
        "-o", "StrictHostKeyChecking=no",
        "-o", "ServerAliveInterval=30",
    ];

    // If a token is provided, pass it as the SSH user (token@a.pinggy.io)
    let host = if let Some(ref token) = config {
        if !token.trim().is_empty() {
            format!("{}@a.pinggy.io", token.trim())
        } else {
            "a.pinggy.io".to_string()
        }
    } else {
        "a.pinggy.io".to_string()
    };
    args.push(&host);

    let (pid, state_url) = spawn_tunnel_process(
        "ssh",
        &args,
        &[],
        UrlSource::Stdout(r"https://[a-z0-9-]+[.]a[.]pinggy[.]link".to_string()),
        app,
    )?;

    if let Ok(mut s) = state.lock() {
        s.tunnel_pid = Some(pid);
    }

    match wait_for_url(&state_url, 8, 7).await {
        Some(url) => {
            if let Ok(mut s) = state.lock() {
                s.tunnel_url = Some(url.clone());
            }
            Ok(url)
        }
        None => {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
            if let Ok(mut s) = state.lock() {
                s.tunnel_pid = None;
            }
            Err("Pinggy tunnel started but URL not captured. Please try again.".to_string())
        }
    }
}

// ── Tauri commands ──

#[tauri::command]
pub async fn start_tunnel(
    port: u16,
    provider: Option<String>,
    config: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let _ = app.emit("tunnel-status", "starting");

    // Stop any existing tunnel
    kill_existing_tunnel(&state);

    let provider_enum = TunnelProvider::from_str_loose(&provider.unwrap_or_default());

    // Store provider in state
    if let Ok(mut s) = state.lock() {
        s.tunnel_provider = provider_enum.clone();
    }

    match provider_enum {
        TunnelProvider::Cloudflared => start_cloudflared(port, &app, &state).await,
        TunnelProvider::Ngrok => start_ngrok(port, config, &app, &state).await,
        TunnelProvider::Zrok => start_zrok(port, config, &app, &state).await,
        TunnelProvider::Pinggy => start_pinggy(port, config, &app, &state).await,
    }
}

#[tauri::command]
pub async fn stop_tunnel(state: State<'_, SharedState>) -> Result<String, String> {
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

    let alive = pid.map(|p| is_pid_alive(p)).unwrap_or(false);

    let status = if alive && url.is_some() {
        "running".to_string()
    } else if alive {
        "starting".to_string()
    } else {
        "stopped".to_string()
    };

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
    provider: Option<String>,
    config: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let _ = app.emit("tunnel-status", "starting");

    // Stop existing tunnel
    kill_existing_tunnel(&state);

    let provider_enum = TunnelProvider::from_str_loose(&provider.unwrap_or_default());

    if let Ok(mut s) = state.lock() {
        s.tunnel_provider = provider_enum.clone();
    }

    // Start tunnel with the appropriate provider
    let url = match provider_enum {
        TunnelProvider::Cloudflared => start_cloudflared(port, &app, &state).await?,
        TunnelProvider::Ngrok => start_ngrok(port, config, &app, &state).await?,
        TunnelProvider::Zrok => start_zrok(port, config, &app, &state).await?,
        TunnelProvider::Pinggy => start_pinggy(port, config, &app, &state).await?,
    };

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
    let mut result = new_lines.join("\n");
    if contents.ends_with('\n') {
        result.push('\n');
    }
    std::fs::write(&env_path, result)
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

    // Update tunnel URL in state so save_resume_state captures it
    if let Ok(mut s) = state.lock() {
        s.tunnel_url = Some(url.clone());
    }

    // Emit events
    let _ = app.emit("tunnel-url", url.clone());
    let _ = app.emit("tunnel-status", "running");

    Ok(url)
}
