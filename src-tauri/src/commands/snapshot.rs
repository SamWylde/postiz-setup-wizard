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
        tunnel_provider,
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
            s.tunnel_provider.as_str().to_string(),
            s.permanent_domain.clone(),
            s.tunnel_pid,
            s.providers_configured.iter().cloned().collect::<Vec<_>>(),
            s.stale_providers.iter().cloned().collect::<Vec<_>>(),
        )
    };

    // If state has no install_path, probe the default location and pointer file
    // so we can discover installs even if resume state was lost
    let discovered_from_disk = install_path.is_none();
    let install_path = install_path.or_else(|| {
        let default_path = dirs::data_local_dir()?.join("Postiz");
        if default_path.join("docker-compose.yml").exists()
            && default_path.join("postiz.env").exists()
        {
            return Some(default_path);
        }
        // Check pointer file for custom install path
        let pointer_path = dirs::data_local_dir()?.join("Postiz").join("install-pointer.json");
        let contents = std::fs::read_to_string(&pointer_path).ok()?;
        let val: serde_json::Value = serde_json::from_str(&contents).ok()?;
        let custom_path = val["install_path"].as_str()?;
        let custom = std::path::PathBuf::from(custom_path);
        if custom.join("docker-compose.yml").exists() && custom.join("postiz.env").exists() {
            Some(custom)
        } else {
            None
        }
    });

    // When we discovered an install from disk (no resume state loaded), read
    // metadata from postiz.env so the snapshot reflects actual configuration
    // instead of in-memory defaults.
    let (port, tunnel_mode, permanent_domain, providers_configured, current_step) =
        if discovered_from_disk {
            if let Some(ref p) = install_path {
                let env_info = read_env_metadata(p);
                let inferred_port = env_info.port.unwrap_or(port);
                let inferred_tunnel_mode = env_info.tunnel_mode.unwrap_or(tunnel_mode);
                let inferred_permanent_domain = env_info.permanent_domain.or(permanent_domain);
                let inferred_providers = if env_info.providers.is_empty() {
                    providers_configured
                } else {
                    env_info.providers
                };
                // Infer how far the user got using only configuration found on
                // disk. A public URL proves the web-link step was completed,
                // but a default in-memory "none" tunnel mode should not be
                // treated as proof that the user explicitly chose local-only.
                let inferred_step = infer_discovered_step(
                    &inferred_providers,
                    env_info.public_url_configured || inferred_permanent_domain.is_some(),
                );
                (
                    inferred_port,
                    inferred_tunnel_mode,
                    inferred_permanent_domain,
                    inferred_providers,
                    inferred_step,
                )
            } else {
                (port, tunnel_mode, permanent_domain, providers_configured, current_step)
            }
        } else {
            (port, tunnel_mode, permanent_domain, providers_configured, current_step)
        };

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
        tunnel_provider,
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

    // 9. Tunnel provider check (only for temporary tunnel mode)
    if tunnel_mode == "temporary" {
        let cf_installed = Command::new("cloudflared")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let ngrok_installed = crate::commands::bootstrap::resolve_binary("ngrok") != "ngrok"
            || Command::new("ngrok").arg("version").output().map(|o| o.status.success()).unwrap_or(false);
        let zrok_installed = crate::commands::bootstrap::resolve_binary("zrok") != "zrok"
            || Command::new("zrok").arg("version").output().map(|o| o.status.success()).unwrap_or(false);
        let ssh_available = Command::new("ssh").arg("-V").output().is_ok();
        let any_provider = cf_installed || ngrok_installed || zrok_installed || ssh_available;
        checks.push(PreflightCheck {
            name: "Tunnel provider available".to_string(),
            passed: any_provider,
            message: if any_provider {
                "At least one tunnel provider is available.".to_string()
            } else {
                "No tunnel provider found. Install cloudflared, ngrok, or zrok, or ensure SSH is available for Pinggy.".to_string()
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

/// Metadata extracted from a postiz.env file on disk.
struct EnvMetadata {
    port: Option<u16>,
    tunnel_mode: Option<String>,
    permanent_domain: Option<String>,
    providers: Vec<String>,
    public_url_configured: bool,
}

fn infer_discovered_step(providers: &[String], public_url_configured: bool) -> usize {
    if !providers.is_empty() {
        5
    } else if public_url_configured {
        4
    } else {
        2
    }
}

/// Read metadata from a postiz.env file so disk-discovered installs
/// have accurate port, tunnel mode, domain, and provider information.
fn read_env_metadata(install_path: &std::path::Path) -> EnvMetadata {
    let env_path = install_path.join("postiz.env");
    let contents = match std::fs::read_to_string(&env_path) {
        Ok(c) => c,
        Err(_) => {
            return EnvMetadata {
                port: None,
                tunnel_mode: None,
                permanent_domain: None,
                providers: Vec::new(),
                public_url_configured: false,
            }
        }
    };

    let mut port: Option<u16> = None;
    let mut main_url: Option<String> = None;
    let mut env_map: Vec<(String, String)> = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim().to_string();
            if key == "MAIN_URL" {
                main_url = Some(value.clone());
                // Extract port from URL
                let without_scheme = value
                    .strip_prefix("https://")
                    .or_else(|| value.strip_prefix("http://"))
                    .unwrap_or(&value);
                let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
                let parts: Vec<&str> = host_port.split(':').collect();
                if parts.len() == 2 {
                    port = parts[1].parse::<u16>().ok();
                }
            }
            env_map.push((key, value));
        }
    }

    // Infer tunnel mode from MAIN_URL:
    // - A known ephemeral tunnel domain means the user completed the
    //   temporary-link step, even if the process is no longer running.
    // - Any other non-localhost HTTPS URL is treated as a permanent domain.
    let (tunnel_mode, permanent_domain, public_url_configured) = if let Some(ref url) = main_url {
        let is_ephemeral = url.contains("trycloudflare.com")
            || url.contains("ngrok-free.app")
            || url.contains("ngrok.io")
            || url.contains(".zrok.io")
            || url.contains(".pinggy.link")
            || url.contains(".pinggy.io");
        let is_public_https = url.starts_with("https://") && !url.contains("localhost");
        if is_public_https && is_ephemeral {
            (Some("temporary".to_string()), None, true)
        } else if is_public_https {
            (Some("permanent".to_string()), Some(url.clone()), true)
        } else {
            (None, None, false)
        }
    } else {
        (None, None, false)
    };

    // Detect configured providers (same keys as import.rs)
    let provider_keys: &[(&str, &str)] = &[
        ("X_API_KEY", "x"),
        ("FACEBOOK_APP_ID", "facebook"),
        ("LINKEDIN_CLIENT_ID", "linkedin"),
        ("REDDIT_CLIENT_ID", "reddit"),
        ("THREADS_APP_ID", "threads"),
        ("YOUTUBE_CLIENT_ID", "youtube"),
        ("TIKTOK_CLIENT_ID", "tiktok"),
        ("PINTEREST_CLIENT_ID", "pinterest"),
        ("DISCORD_CLIENT_ID", "discord"),
        ("SLACK_ID", "slack"),
        ("MASTODON_CLIENT_ID", "mastodon"),
        ("DRIBBBLE_CLIENT_ID", "dribbble"),
    ];

    let mut providers = Vec::new();
    for (env_key, provider_name) in provider_keys {
        for (key, value) in &env_map {
            if key == *env_key && !value.is_empty() {
                providers.push(provider_name.to_string());
                break;
            }
        }
    }

    EnvMetadata {
        port,
        tunnel_mode,
        permanent_domain,
        providers,
        public_url_configured,
    }
}

#[cfg(test)]
mod tests {
    use super::{infer_discovered_step, read_env_metadata};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_install_dir() -> PathBuf {
        let unique = format!(
            "postiz-snapshot-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("temp install dir should be created");
        dir
    }

    #[test]
    fn infer_discovered_step_only_advances_for_real_public_url_or_providers() {
        assert_eq!(infer_discovered_step(&[], false), 2);
        assert_eq!(infer_discovered_step(&[], true), 4);
        assert_eq!(infer_discovered_step(&["x".to_string()], false), 5);
    }

    #[test]
    fn read_env_metadata_marks_ephemeral_urls_as_temporary() {
        let dir = make_temp_install_dir();
        fs::write(
            dir.join("postiz.env"),
            "MAIN_URL=https://example.trycloudflare.com\nX_API_KEY=test\n",
        )
        .expect("postiz.env should be written");

        let env = read_env_metadata(&dir);

        assert_eq!(env.tunnel_mode.as_deref(), Some("temporary"));
        assert_eq!(env.permanent_domain, None);
        assert!(env.public_url_configured);
        assert_eq!(env.providers, vec!["x".to_string()]);

        fs::remove_dir_all(&dir).expect("temp install dir should be removed");
    }

    #[test]
    fn read_env_metadata_keeps_localhost_out_of_public_url_inference() {
        let dir = make_temp_install_dir();
        fs::write(dir.join("postiz.env"), "MAIN_URL=http://localhost:4007\n")
            .expect("postiz.env should be written");

        let env = read_env_metadata(&dir);

        assert_eq!(env.tunnel_mode, None);
        assert_eq!(env.permanent_domain, None);
        assert!(!env.public_url_configured);

        fs::remove_dir_all(&dir).expect("temp install dir should be removed");
    }
}
