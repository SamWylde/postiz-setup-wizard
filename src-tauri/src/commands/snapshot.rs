use std::net::TcpListener;
use std::path::PathBuf;
use tauri::State;

use super::{parse_docker_ps_json, silent_cmd};
use crate::state::{InstallSnapshot, PreflightCheck, PreflightResult, SharedState};

fn infer_tunnel_provider(
    tunnel_mode: &str,
    existing_provider: &str,
    tunnel_pid: Option<u32>,
    tunnel_config: Option<&str>,
) -> String {
    match tunnel_mode {
        "temporary" => "cloudflared".to_string(),
        "permanent" => {
            let has_cloudflare_clue = tunnel_pid.is_some()
                || tunnel_config
                    .map(|config| !config.trim().is_empty())
                    .unwrap_or(false);

            if existing_provider == "manual" {
                "manual".to_string()
            } else if existing_provider == "cloudflared" && has_cloudflare_clue {
                "cloudflared".to_string()
            } else if has_cloudflare_clue {
                "cloudflared".to_string()
            } else {
                // A disk-discovered permanent MAIN_URL does not encode whether it
                // came from a manual reverse proxy or Cloudflare Zero Trust.
                // When there is no tracked tunnel process/token, default to a
                // manual domain so tray actions and saved state keep a usable URL.
                "manual".to_string()
            }
        }
        "none" => "manual".to_string(),
        "local_https" => "manual".to_string(),
        _ => existing_provider.to_string(),
    }
}

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
        tunnel_config,
        permanent_domain,
        tunnel_pid,
        providers_configured,
        providers_stale,
    ) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        (
            s.install_path.clone(),
            s.port,
            s.current_step,
            s.last_error.clone(),
            s.tunnel_mode.clone(),
            s.tunnel_provider.as_str().to_string(),
            s.tunnel_config.clone(),
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
        if !custom.exists() {
            eprintln!(
                "install-pointer.json points to '{}' which no longer exists on disk; ignoring stale pointer",
                custom.display()
            );
            return None;
        }
        if custom.join("docker-compose.yml").exists() && custom.join("postiz.env").exists() {
            Some(custom)
        } else {
            eprintln!(
                "install-pointer.json points to '{}' which exists but is missing docker-compose.yml or postiz.env; ignoring",
                custom.display()
            );
            None
        }
    });

    // Read metadata from postiz.env whenever we can so snapshots follow the
    // real install on disk, even if a previous UI flow persisted draft tunnel
    // state before the change was actually applied.
    let env_info = install_path.as_ref().map(|p| read_env_metadata(p));

    let port = env_info.as_ref().and_then(|info| info.port).unwrap_or(port);
    let tunnel_mode = env_info
        .as_ref()
        .and_then(|info| info.tunnel_mode.clone())
        .unwrap_or(tunnel_mode);
    let permanent_domain = env_info
        .as_ref()
        .and_then(|info| info.permanent_domain.clone())
        .or(permanent_domain);
    let tunnel_provider = infer_tunnel_provider(
        &tunnel_mode,
        &tunnel_provider,
        tunnel_pid,
        tunnel_config.as_deref(),
    );
    let providers_configured = if let Some(info) = env_info.as_ref() {
        if info.providers.is_empty() {
            providers_configured
        } else {
            info.providers.clone()
        }
    } else {
        providers_configured
    };
    let current_step = if discovered_from_disk {
        if let Some(info) = env_info.as_ref() {
            infer_discovered_step(
                &providers_configured,
                info.public_url_configured || permanent_domain.is_some(),
            )
        } else {
            current_step
        }
    } else {
        current_step
    };

    // Sync inferred disk-backed values into AppState so subsequent commands use
    // the actual install configuration rather than stale resume data.
    if let Some(ref p) = install_path {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.install_path = Some(p.clone());
        s.port = port;
        s.local_url = Some(format!("http://localhost:{}", port));
        s.tunnel_mode = tunnel_mode.clone();
        s.tunnel_provider = crate::state::TunnelProvider::from_str_loose(&tunnel_provider);
        s.permanent_domain = permanent_domain.clone();
        let configured_set = providers_configured.iter().cloned().collect();
        s.providers_configured = configured_set;
        s.stale_providers
            .retain(|provider| providers_configured.iter().any(|p| p == provider));
        if discovered_from_disk {
            s.current_step = current_step;
        }
    }

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
    let docker_installed = silent_cmd("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // 5. Check docker running
    let docker_running = silent_cmd("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // 6. Get containers if install exists and docker running
    let mut containers = Vec::new();
    let mut all_healthy = false;

    if install_exists && docker_running {
        if let Some(ref path) = install_path {
            let output = silent_cmd("docker")
                .args(["compose", "ps", "--format", "json"])
                .current_dir(path)
                .output();

            if let Ok(output) = output {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for val in parse_docker_ps_json(&stdout) {
                    containers.push(crate::commands::docker::ContainerInfo {
                        name: val["Name"].as_str().unwrap_or("unknown").to_string(),
                        state: val["State"].as_str().unwrap_or("unknown").to_string(),
                        status: val["Status"].as_str().unwrap_or("unknown").to_string(),
                        health: val["Health"].as_str().unwrap_or("").to_string(),
                    });
                }
            }

            // A container is "good" if it's running — Docker HEALTHCHECK is
            // unreliable (many containers don't have one, and even when they
            // do it can disagree with our own HTTP check).  We have a separate
            // postiz_responding check for the real health status.
            all_healthy = !containers.is_empty() && containers.iter().all(|c| c.state == "running");
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
            silent_cmd("tasklist")
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
            .unwrap_or_else(|e| e.into_inner())
            .tunnel_url
            .clone()
    } else {
        None
    };

    // 10. Determine recovery_available — only for genuine problems, not just
    //     "Docker isn't running".  The wizard's PrepareComputer step handles
    //     Docker prerequisites, so we don't need recovery for that.
    let has_unhealthy_containers = !containers.is_empty() && !all_healthy;
    let running_but_unresponsive = docker_running && !containers.is_empty() && !postiz_responding;
    let recovery_available =
        install_exists && (has_staged_temp || has_unhealthy_containers || running_but_unresponsive);

    // Derive web_link_* fields from tunnel_mode + tunnel_provider
    let (web_link_kind, web_link_supported, web_link_reason) = match tunnel_mode.as_str() {
        "none" => ("none".to_string(), true, None),
        "local_https" => (
            "local_https".to_string(),
            true,
            Some(
                "This hostname is mapped to this computer only. It is useful for local HTTPS OAuth testing, not for a public website."
                    .to_string(),
            ),
        ),
        "permanent" => {
            if tunnel_provider == "cloudflared" {
                ("cloudflare".to_string(), true, None)
            } else {
                // "manual" or any legacy permanent provider → treat as custom domain
                ("manual".to_string(), true, None)
            }
        }
        "temporary" => (
            "legacy_shared".to_string(),
            false,
            Some(
                "Shared tunnel domains are no longer supported. Login does not work on these domains due to a known Postiz limitation. Please switch to a custom domain or Cloudflare Zero Trust."
                    .to_string(),
            ),
        ),
        _ => ("none".to_string(), true, None),
    };

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
        web_link_kind,
        web_link_supported,
        web_link_reason,
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

    // 2. Install path available — only allow existing files when a .tmp staging
    //    folder proves this wizard created the partial state (retry scenario).
    let install_dir = PathBuf::from(&path);
    let compose_exists = install_dir.join("docker-compose.yml").exists();
    let env_exists = install_dir.join("postiz.env").exists();
    let has_existing_install = compose_exists || env_exists;
    let tmp_exists = install_dir.join(".tmp").exists();
    // allow_existing is only honoured when the .tmp marker is also present
    let retry_allowed = allow_existing && tmp_exists;
    checks.push(PreflightCheck {
        name: "Install path available".to_string(),
        passed: !has_existing_install || retry_allowed || tmp_exists,
        message: if !has_existing_install {
            "Install path is available.".to_string()
        } else if tmp_exists {
            "Staged .tmp folder found — resuming previous install attempt.".to_string()
        } else {
            "An existing Postiz install was found at this path (docker-compose.yml or postiz.env already exist). Use recovery/import to manage the existing install, or choose a different path.".to_string()
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
    let docker_installed = silent_cmd("docker")
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
    let docker_info_output = silent_cmd("docker").arg("info").output();
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

    // 9. Tunnel provider check (only for legacy temporary tunnels)
    if tunnel_mode == "temporary" {
        let cf_installed = silent_cmd("cloudflared")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        checks.push(PreflightCheck {
            name: "Tunnel provider available".to_string(),
            passed: cf_installed,
            message: if cf_installed {
                "cloudflared is available.".to_string()
            } else {
                "cloudflared not found. Install it to use a temporary Cloudflare tunnel."
                    .to_string()
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
    let mut wizard_web_link_mode: Option<String> = None;

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
            if key == "POSTIZ_WIZARD_WEB_LINK_MODE" {
                wizard_web_link_mode = Some(value.to_lowercase());
            }
            env_map.push((key, value));
        }
    }

    // Infer tunnel mode from MAIN_URL:
    // - localhost/127.0.0.1 means the install is in local-only mode.
    // - A known ephemeral tunnel domain means the user completed the
    //   temporary-link step, even if the process is no longer running.
    // - Any other non-localhost HTTPS URL is treated as a permanent domain.
    let (tunnel_mode, permanent_domain, public_url_configured) = if let Some(ref url) = main_url {
        let is_local = url.starts_with("http://localhost")
            || url.starts_with("https://localhost")
            || url.starts_with("http://127.0.0.1")
            || url.starts_with("https://127.0.0.1");
        let is_ephemeral = url.contains("trycloudflare.com")
            || url.contains("ngrok-free.app")
            || url.contains("ngrok.io")
            || url.contains(".zrok.io")
            || url.contains(".pinggy.link")
            || url.contains(".pinggy.io");
        let is_public_https = url.starts_with("https://") && !url.contains("localhost");
        if wizard_web_link_mode.as_deref() == Some("local_https") {
            (Some("local_https".to_string()), Some(url.clone()), true)
        } else if is_local {
            (Some("none".to_string()), None, false)
        } else if is_public_https && is_ephemeral {
            (Some("temporary".to_string()), None, true)
        } else if is_public_https {
            (Some("permanent".to_string()), Some(url.clone()), true)
        } else {
            (None, None, false)
        }
    } else {
        (None, None, false)
    };

    // Detect configured providers (uses shared key list from commands/mod.rs)
    let mut providers = Vec::new();
    for (env_key, provider_name) in super::PROVIDER_ENV_KEYS {
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
    fn read_env_metadata_treats_localhost_as_local_only() {
        let dir = make_temp_install_dir();
        fs::write(dir.join("postiz.env"), "MAIN_URL=http://localhost:4007\n")
            .expect("postiz.env should be written");

        let env = read_env_metadata(&dir);

        assert_eq!(env.tunnel_mode.as_deref(), Some("none"));
        assert_eq!(env.permanent_domain, None);
        assert!(!env.public_url_configured);

        fs::remove_dir_all(&dir).expect("temp install dir should be removed");
    }

    #[test]
    fn read_env_metadata_marks_local_https_domains_via_wizard_marker() {
        let dir = make_temp_install_dir();
        fs::write(
            dir.join("postiz.env"),
            "MAIN_URL=https://postiz.grantcue.test\nPOSTIZ_WIZARD_WEB_LINK_MODE=local_https\n",
        )
        .expect("postiz.env should be written");

        let env = read_env_metadata(&dir);

        assert_eq!(env.tunnel_mode.as_deref(), Some("local_https"));
        assert_eq!(
            env.permanent_domain.as_deref(),
            Some("https://postiz.grantcue.test")
        );
        assert!(env.public_url_configured);

        fs::remove_dir_all(&dir).expect("temp install dir should be removed");
    }

    #[test]
    fn infer_tunnel_provider_defaults_disk_discovered_permanent_urls_to_manual() {
        assert_eq!(
            super::infer_tunnel_provider("permanent", "cloudflared", None, None),
            "manual"
        );
        assert_eq!(
            super::infer_tunnel_provider("permanent", "cloudflared", Some(123), None),
            "cloudflared"
        );
        assert_eq!(
            super::infer_tunnel_provider("temporary", "manual", None, None),
            "cloudflared"
        );
    }
}
