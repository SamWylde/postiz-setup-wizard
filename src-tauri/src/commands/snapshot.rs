use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use tauri::State;

use crate::state::{InstallSnapshot, PreflightCheck, PreflightResult, SharedState};

#[tauri::command]
pub async fn get_install_snapshot(
    state: State<'_, SharedState>,
) -> Result<InstallSnapshot, String> {
    // 1. Read state fields
    let (
        install_path,
        port,
        current_step,
        last_error,
        tunnel_mode,
        permanent_domain,
        tunnel_pid,
        providers_configured,
        providers_stale,
    ) = {
        let s = state
            .lock()
            .map_err(|e| format!("State lock failed: {}", e))?;
        (
            s.install_path.clone(),
            s.port,
            s.current_step,
            s.last_error.clone(),
            s.tunnel_mode.clone(),
            s.permanent_domain.clone(),
            s.tunnel_pid,
            s.providers_configured.iter().cloned().collect::<Vec<_>>(),
            s.stale_providers.iter().cloned().collect::<Vec<_>>(),
        )
    };

    // If state has no install_path, probe the default location on disk
    // so we can discover installs even if resume state was lost
    let install_path = install_path.or_else(|| {
        let default_path = dirs::data_local_dir()?.join("Postiz");
        if default_path.join("docker-compose.yml").exists()
            && default_path.join("postiz.env").exists()
        {
            Some(default_path)
        } else {
            None
        }
    });

    let install_path_str = install_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    // 2. Check if install dir exists with required files
    let install_exists = install_path
        .as_ref()
        .map(|p| p.join("docker-compose.yml").exists() && p.join("postiz.env").exists())
        .unwrap_or(false);

    // 3. Check if .tmp staging dir exists
    let has_staged_temp = install_path
        .as_ref()
        .map(|p| p.join(".tmp").exists())
        .unwrap_or(false);

    // 4. Check docker installed
    let docker_installed = Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // 5. Check docker running
    let docker_running = Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // 6. Get containers if install exists and docker running
    let mut containers = Vec::new();
    let mut all_healthy = false;

    if install_exists && docker_running {
        if let Some(ref path) = install_path {
            let output = Command::new("docker")
                .args(["compose", "ps", "--format", "json"])
                .current_dir(path)
                .output();

            if let Ok(output) = output {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                        let name = val["Name"].as_str().unwrap_or("unknown").to_string();
                        let container_state =
                            val["State"].as_str().unwrap_or("unknown").to_string();
                        let status = val["Status"].as_str().unwrap_or("unknown").to_string();
                        let health = val["Health"].as_str().unwrap_or("").to_string();

                        containers.push(crate::commands::docker::ContainerInfo {
                            name,
                            state: container_state,
                            status,
                            health,
                        });
                    }
                }
            }

            all_healthy = !containers.is_empty()
                && containers.iter().all(|c| {
                    c.state == "running" && (c.health.is_empty() || c.health == "healthy")
                });
        }
    }

    // 7. HTTP check for postiz responding
    let postiz_responding = reqwest::Client::new()
        .get(format!("http://localhost:{}", port))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map(|r| r.status().is_success() || r.status().is_redirection())
        .unwrap_or(false);

    // 8. Check tunnel alive (CSV format for reliable PID matching)
    let tunnel_alive = tunnel_pid
        .map(|pid| {
            Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/NH", "/FO", "CSV"])
                .output()
                .map(|o| {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    !stdout.contains("INFO:") && stdout.contains(&format!("\"{}\"", pid))
                })
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let tunnel_url = if tunnel_alive {
        state
            .lock()
            .ok()
            .and_then(|s| s.tunnel_url.clone())
    } else {
        None
    };

    // 10. Determine recovery_available
    let recovery_available =
        install_exists && (!all_healthy || !postiz_responding || has_staged_temp);

    Ok(InstallSnapshot {
        install_path: install_path_str,
        install_exists,
        has_staged_temp,
        port,
        docker_installed,
        docker_running,
        containers,
        all_healthy,
        postiz_responding,
        tunnel_alive,
        tunnel_url,
        tunnel_mode,
        permanent_domain,
        providers_configured,
        providers_stale,
        current_step,
        last_error,
        recovery_available,
    })
}

#[tauri::command]
pub async fn validate_preflight(
    path: String,
    port: u16,
    tunnel_mode: String,
    allow_existing: Option<bool>,
    _state: State<'_, SharedState>,
) -> Result<PreflightResult, String> {
    let allow_existing = allow_existing.unwrap_or(false);
    let mut checks = Vec::new();

    // 1. Install path writable
    let parent = PathBuf::from(&path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(&path));
    let temp_file = parent.join(".postiz-write-test");
    let writable = std::fs::write(&temp_file, "test").is_ok();
    if writable {
        let _ = std::fs::remove_file(&temp_file);
    }
    checks.push(PreflightCheck {
        name: "Install path writable".to_string(),
        passed: writable,
        message: if writable {
            "Parent directory is writable.".to_string()
        } else {
            format!("Cannot write to {}. Check permissions.", parent.display())
        },
    });

    // 2. Install path available (skip check if retrying a failed install)
    let compose_exists = PathBuf::from(&path).join("docker-compose.yml").exists();
    checks.push(PreflightCheck {
        name: "Install path available".to_string(),
        passed: !compose_exists || allow_existing,
        message: if !compose_exists {
            "Install path is available.".to_string()
        } else if allow_existing {
            "Existing install found — will retry in place.".to_string()
        } else {
            "An existing docker-compose.yml was found. This may be a re-install.".to_string()
        },
    });

    // 3. Main port available (soft check if using default — prepare_install will auto-find free port)
    let port_available = TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok();
    let is_default_port = port == 4007;
    checks.push(PreflightCheck {
        name: "Port available".to_string(),
        passed: port_available || is_default_port,
        message: if port_available {
            format!("Port {} is available.", port)
        } else if is_default_port {
            format!(
                "Port {} is in use — a free port will be selected automatically.",
                port
            )
        } else {
            format!(
                "Port {} is already in use. Choose a different port or stop the conflicting service.",
                port
            )
        },
    });

    // 3b. Temporal port (7233) available
    let temporal_port_available = TcpListener::bind("127.0.0.1:7233").is_ok();
    checks.push(PreflightCheck {
        name: "Temporal port available".to_string(),
        passed: temporal_port_available,
        message: if temporal_port_available {
            "Port 7233 (Temporal) is available.".to_string()
        } else {
            "Port 7233 is already in use. Temporal requires this port. Stop the conflicting service before installing.".to_string()
        },
    });

    // 3c. Temporal UI port (8080) available
    let temporal_ui_port_available = TcpListener::bind("127.0.0.1:8080").is_ok();
    checks.push(PreflightCheck {
        name: "Temporal UI port available".to_string(),
        passed: temporal_ui_port_available,
        message: if temporal_ui_port_available {
            "Port 8080 (Temporal UI) is available.".to_string()
        } else {
            "Port 8080 is already in use. Temporal UI requires this port. Stop the conflicting service before installing.".to_string()
        },
    });

    // 4. Docker installed
    let docker_installed = Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    checks.push(PreflightCheck {
        name: "Docker installed".to_string(),
        passed: docker_installed,
        message: if docker_installed {
            "Docker is installed.".to_string()
        } else {
            "Docker is not installed. Please install Docker Desktop first.".to_string()
        },
    });

    // 5. Docker running
    let docker_info_output = Command::new("docker").arg("info").output();
    let docker_running = docker_info_output
        .as_ref()
        .map(|o| o.status.success())
        .unwrap_or(false);
    checks.push(PreflightCheck {
        name: "Docker running".to_string(),
        passed: docker_running,
        message: if docker_running {
            "Docker is running.".to_string()
        } else {
            "Docker is not running. Please start Docker Desktop.".to_string()
        },
    });

    // 6. Docker Linux mode
    let docker_linux = docker_info_output
        .as_ref()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let combined = format!("{}{}", stdout, stderr);
            combined.contains("linux") || combined.contains("Linux")
        })
        .unwrap_or(false);
    checks.push(PreflightCheck {
        name: "Docker Linux mode".to_string(),
        passed: docker_linux,
        message: if docker_linux {
            "Docker is running in Linux container mode.".to_string()
        } else {
            "Docker must be switched to Linux container mode. Right-click the Docker tray icon and select 'Switch to Linux containers'.".to_string()
        },
    });

    // 7. Disk space — need 3GB on the drive containing the install path
    let disk_ok = check_disk_space(&path, 3 * 1024 * 1024 * 1024);
    checks.push(PreflightCheck {
        name: "Disk space".to_string(),
        passed: disk_ok,
        message: if disk_ok {
            "Sufficient disk space available (3 GB required).".to_string()
        } else {
            "Less than 3 GB of free disk space. Free up space before installing.".to_string()
        },
    });

    // 8. Available RAM — need 2GB
    let ram_ok = check_available_ram(2 * 1024 * 1024 * 1024);
    checks.push(PreflightCheck {
        name: "Available RAM".to_string(),
        passed: ram_ok,
        message: if ram_ok {
            "Sufficient RAM available (2 GB required).".to_string()
        } else {
            "Less than 2 GB of RAM available. Close other applications to free memory.".to_string()
        },
    });

    // 9. cloudflared check (only for temporary tunnel mode)
    if tunnel_mode == "temporary" {
        let cf_installed = Command::new("cloudflared")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        checks.push(PreflightCheck {
            name: "cloudflared installed".to_string(),
            passed: cf_installed,
            message: if cf_installed {
                "cloudflared is installed.".to_string()
            } else {
                "cloudflared is not installed. It is required for temporary tunnel mode.".to_string()
            },
        });
    }

    let ok = checks.iter().all(|c| c.passed);

    Ok(PreflightResult { ok, checks })
}

fn check_disk_space(path: &str, required_bytes: u64) -> bool {
    use sysinfo::Disks;

    let disks = Disks::new_with_refreshed_list();
    let path_buf = PathBuf::from(path);

    // Find the disk that contains the given path by matching mount point
    let mut best_match: Option<&sysinfo::Disk> = None;
    let mut best_len = 0;

    for disk in disks.list() {
        let mount = disk.mount_point();
        let mount_str = mount.to_string_lossy().to_lowercase();
        let path_str = path_buf.to_string_lossy().to_lowercase();

        if path_str.starts_with(&mount_str) && mount_str.len() > best_len {
            best_len = mount_str.len();
            best_match = Some(disk);
        }
    }

    best_match
        .map(|d| d.available_space() >= required_bytes)
        .unwrap_or(false)
}

fn check_available_ram(required_bytes: u64) -> bool {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();

    sys.available_memory() >= required_bytes
}
