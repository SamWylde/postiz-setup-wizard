use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tauri::{Emitter, State};

use super::docker::run_docker_compose;
use super::env_file::{parse_env_file, write_local_base_urls, write_public_base_urls};
use super::silent_cmd;
use super::tunnel::{is_pid_alive, kill_existing_tunnel, start_cloudflared};
use crate::commands::bootstrap::resolve_binary;
use crate::state::{PublicUrlCheck, SharedState, TunnelProvider};

#[derive(Clone)]
struct PreviousLinkState {
    tunnel_mode: String,
    tunnel_provider: TunnelProvider,
    tunnel_config: Option<String>,
    permanent_domain: Option<String>,
}

fn capture_previous_link_state(state: &SharedState) -> PreviousLinkState {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    PreviousLinkState {
        tunnel_mode: s.tunnel_mode.clone(),
        tunnel_provider: s.tunnel_provider.clone(),
        tunnel_config: s.tunnel_config.clone(),
        permanent_domain: s.permanent_domain.clone(),
    }
}

fn current_port(state: &SharedState) -> u16 {
    state.lock().unwrap_or_else(|e| e.into_inner()).port
}

fn reset_cancel_flag(state: &SharedState) {
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .docker_op_cancelled = false;
}

fn write_env_contents(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("env.tmp");
    fs::write(&tmp, contents).map_err(|e| format!("Failed to write temp env file: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("Failed to replace env file: {}", e))?;
    Ok(())
}

async fn wait_for_local_postiz(port: u16, state: &SharedState) -> Result<bool, String> {
    let client = reqwest::Client::new();

    for _ in 0..20 {
        if state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .docker_op_cancelled
        {
            return Err("Operation cancelled.".to_string());
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let healthy = client
            .get(format!("http://localhost:{}", port))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().is_redirection())
            .unwrap_or(false);

        if healthy {
            return Ok(true);
        }
    }

    Ok(false)
}

async fn rollback_env_change(
    install_path: &Path,
    env_path: &Path,
    original_env: &str,
    port: u16,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<(), String> {
    let _ = app.emit("docker-progress", "Rolling back web link configuration...");

    reset_cancel_flag(state);
    write_env_contents(env_path, original_env)?;

    let output = run_docker_compose(
        &["compose", "--env-file", "postiz.env", "down"],
        install_path,
        state,
    )
    .map_err(|e| format!("Rollback failed while stopping services: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit(
            "docker-progress",
            format!(
                "Warning: rollback docker compose down had issues: {}",
                stderr
            ),
        );
    }

    let output = run_docker_compose(
        &["compose", "--env-file", "postiz.env", "up", "-d"],
        install_path,
        state,
    )
    .map_err(|e| format!("Rollback failed while starting services: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Rollback failed to start services: {}", stderr));
    }

    if wait_for_local_postiz(port, state).await? {
        Ok(())
    } else {
        Err(
            "Rollback restarted the stack, but Postiz did not become healthy again. Manual recovery may be required."
                .to_string(),
        )
    }
}

async fn apply_link_change<F>(
    install_path: &Path,
    env_path: &Path,
    original_env: &str,
    port: u16,
    app: &tauri::AppHandle,
    state: &SharedState,
    apply_change: F,
) -> Result<(), String>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    reset_cancel_flag(state);
    apply_change(env_path)?;

    let _ = app.emit(
        "docker-progress",
        "Restarting Postiz with new web link settings...",
    );

    let output = run_docker_compose(
        &["compose", "--env-file", "postiz.env", "down"],
        install_path,
        state,
    )
    .map_err(|e| format!("Failed to stop stack: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit(
            "docker-progress",
            format!("Warning: docker compose down had issues: {}", stderr),
        );
    }

    let output = run_docker_compose(
        &["compose", "--env-file", "postiz.env", "up", "-d"],
        install_path,
        state,
    )
    .map_err(|e| format!("Failed to start stack: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        rollback_env_change(install_path, env_path, original_env, port, app, state).await?;
        return Err(format!(
            "Failed to start Postiz with the new web link settings: {}. Changes have been rolled back.",
            stderr
        ));
    }

    let _ = app.emit("docker-progress", "Waiting for Postiz to respond...");

    match wait_for_local_postiz(port, state).await {
        Ok(true) => Ok(()),
        Ok(false) => {
            rollback_env_change(install_path, env_path, original_env, port, app, state).await?;
            Err(
                "Postiz did not become healthy after applying the new web link. Changes have been rolled back."
                    .to_string(),
            )
        }
        Err(err) => {
            rollback_env_change(install_path, env_path, original_env, port, app, state).await?;
            Err(err)
        }
    }
}

async fn start_cloudflare_zero_trust_process(
    token: &str,
    state: &SharedState,
) -> Result<u32, String> {
    let binary = resolve_binary("cloudflared");
    let child = silent_cmd(&binary)
        .args(["tunnel", "--no-autoupdate", "run", "--token", token])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start cloudflared: {}", e))?;

    let pid = child.id();
    state.lock().unwrap_or_else(|e| e.into_inner()).tunnel_pid = Some(pid);

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    if is_pid_alive(pid) {
        Ok(pid)
    } else {
        state.lock().unwrap_or_else(|e| e.into_inner()).tunnel_pid = None;
        Err("cloudflared exited immediately. Check your tunnel token and try again.".to_string())
    }
}

async fn restore_previous_link(
    previous: &PreviousLinkState,
    app: &tauri::AppHandle,
    state: &SharedState,
    port: u16,
) -> Result<(), String> {
    match previous.tunnel_provider.clone() {
        TunnelProvider::Cloudflared => match previous.tunnel_mode.as_str() {
            "temporary" => {
                let url = start_cloudflared(port, app, state).await?;
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.tunnel_mode = "temporary".to_string();
                s.tunnel_provider = TunnelProvider::Cloudflared;
                s.tunnel_url = Some(url);
                s.permanent_domain = None;
                Ok(())
            }
            "permanent" => {
                let token = previous.tunnel_config.clone().ok_or_else(|| {
                    "Previous Cloudflare tunnel token was unavailable.".to_string()
                })?;
                let pid = start_cloudflare_zero_trust_process(&token, state).await?;
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.tunnel_mode = "permanent".to_string();
                s.tunnel_provider = TunnelProvider::Cloudflared;
                s.tunnel_pid = Some(pid);
                s.tunnel_url = previous.permanent_domain.clone();
                s.permanent_domain = previous.permanent_domain.clone();
                s.tunnel_config = Some(token);
                Ok(())
            }
            _ => Ok(()),
        },
        TunnelProvider::Manual => Ok(()),
    }
}

// ── apply_manual_domain ──────────────────────────────────────────────

#[tauri::command]
pub async fn apply_manual_domain(
    path: String,
    public_url: String,
    force: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let public_url = public_url.trim().trim_end_matches('/').to_string();

    if !public_url.starts_with("https://") {
        return Err("Domain must start with https://".to_string());
    }

    let install_path = PathBuf::from(&path);
    let env_path = install_path.join("postiz.env");

    if !env_path.exists() {
        return Err(format!("Env file not found: {}", env_path.display()));
    }

    if !force {
        let check = verify_public_url_inner(&public_url).await;
        if !check.reachable {
            return Err(format!(
                "UNREACHABLE: Could not reach {}. {}",
                public_url,
                check.error.unwrap_or_default()
            ));
        }
    }

    let previous = capture_previous_link_state(&state);
    let port = current_port(&state);
    let original_env =
        fs::read_to_string(&env_path).map_err(|e| format!("Failed to read env file: {}", e))?;

    let _ = app.emit("docker-progress", "Stopping any active tunnel...");
    kill_existing_tunnel(&state);

    let _ = app.emit("docker-progress", "Writing domain configuration...");
    let result = apply_link_change(
        &install_path,
        &env_path,
        &original_env,
        port,
        &app,
        &state,
        |path| write_public_base_urls(path, &public_url),
    )
    .await;

    if let Err(err) = result {
        return match restore_previous_link(&previous, &app, &state, port).await {
            Ok(()) => Err(err),
            Err(restore_err) => Err(format!(
                "{} Previous web link could not be restored automatically: {}",
                err, restore_err
            )),
        };
    }

    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.tunnel_mode = "permanent".to_string();
        s.tunnel_provider = TunnelProvider::Manual;
        s.permanent_domain = Some(public_url.clone());
        s.tunnel_url = None;
        s.tunnel_config = None;
    }

    let _ = app.emit("docker-progress", "Domain applied successfully!");
    Ok(format!("Domain applied: {}", public_url))
}

// ── switch_to_local_only ─────────────────────────────────────────────

#[tauri::command]
pub async fn switch_to_local_only(
    path: String,
    port: u16,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = PathBuf::from(&path);
    let env_path = install_path.join("postiz.env");

    if !env_path.exists() {
        return Err(format!("Env file not found: {}", env_path.display()));
    }

    let original_env =
        fs::read_to_string(&env_path).map_err(|e| format!("Failed to read env file: {}", e))?;
    let previous = capture_previous_link_state(&state);
    let previous_main_url = {
        let map = parse_env_file(&original_env);
        map.get("MAIN_URL").cloned().unwrap_or_default()
    };

    let _ = app.emit("docker-progress", "Stopping any active tunnel...");
    kill_existing_tunnel(&state);

    let _ = app.emit("docker-progress", "Switching to local-only mode...");
    let result = apply_link_change(
        &install_path,
        &env_path,
        &original_env,
        port,
        &app,
        &state,
        |path| write_local_base_urls(path, port),
    )
    .await;

    if let Err(err) = result {
        return match restore_previous_link(&previous, &app, &state, port).await {
            Ok(()) => Err(err),
            Err(restore_err) => Err(format!(
                "{} Previous web link could not be restored automatically: {}",
                err, restore_err
            )),
        };
    }

    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.tunnel_mode = "none".to_string();
        s.tunnel_provider = TunnelProvider::Manual;
        s.tunnel_url = None;
        s.permanent_domain = None;
        s.tunnel_config = None;
    }

    let _ = app.emit("docker-progress", "Switched to local-only mode.");
    Ok(previous_main_url)
}

// ── connect_cloudflare_zero_trust ────────────────────────────────────

#[tauri::command]
pub async fn connect_cloudflare_zero_trust(
    path: String,
    port: u16,
    public_url: String,
    token: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let public_url = public_url.trim().trim_end_matches('/').to_string();
    let token = token.trim().to_string();

    if !public_url.starts_with("https://") {
        return Err("Domain must start with https://".to_string());
    }
    if token.is_empty() {
        return Err("Tunnel token is required.".to_string());
    }

    let install_path = PathBuf::from(&path);
    let env_path = install_path.join("postiz.env");

    if !env_path.exists() {
        return Err(format!("Env file not found: {}", env_path.display()));
    }

    let previous = capture_previous_link_state(&state);
    let local_port = {
        let state_port = current_port(&state);
        if state_port == 0 {
            port
        } else {
            state_port
        }
    };
    let original_env =
        fs::read_to_string(&env_path).map_err(|e| format!("Failed to read env file: {}", e))?;

    let _ = app.emit("docker-progress", "Stopping any active tunnel...");
    kill_existing_tunnel(&state);

    let _ = app.emit("docker-progress", "Starting Cloudflare tunnel...");
    let pid = match start_cloudflare_zero_trust_process(&token, &state).await {
        Ok(pid) => pid,
        Err(err) => {
            return match restore_previous_link(&previous, &app, &state, local_port).await {
                Ok(()) => Err(err),
                Err(restore_err) => Err(format!(
                    "{} Previous web link could not be restored automatically: {}",
                    err, restore_err
                )),
            };
        }
    };

    let _ = app.emit("docker-progress", "Writing domain configuration...");
    let result = apply_link_change(
        &install_path,
        &env_path,
        &original_env,
        local_port,
        &app,
        &state,
        |path| write_public_base_urls(path, &public_url),
    )
    .await;

    if let Err(err) = result {
        kill_existing_tunnel(&state);
        return match restore_previous_link(&previous, &app, &state, local_port).await {
            Ok(()) => Err(err),
            Err(restore_err) => Err(format!(
                "{} Previous web link could not be restored automatically: {}",
                err, restore_err
            )),
        };
    }

    if !is_pid_alive(pid) {
        kill_existing_tunnel(&state);

        let rollback_result = rollback_env_change(
            &install_path,
            &env_path,
            &original_env,
            local_port,
            &app,
            &state,
        )
        .await;
        let restore_result = restore_previous_link(&previous, &app, &state, local_port).await;

        let mut message =
            "Cloudflare tunnel exited before the new web link was ready. Changes have been rolled back."
                .to_string();
        if let Err(err) = rollback_result {
            message.push_str(&format!(" Rollback error: {}", err));
        }
        if let Err(err) = restore_result {
            message.push_str(&format!(
                " Previous web link could not be restored automatically: {}",
                err
            ));
        }
        return Err(message);
    }

    let _ = app.emit("docker-progress", "Verifying public URL...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let check = verify_public_url_inner(&public_url).await;

    if !check.reachable && check.status_code.is_some() {
        kill_existing_tunnel(&state);

        let rollback_result = rollback_env_change(
            &install_path,
            &env_path,
            &original_env,
            local_port,
            &app,
            &state,
        )
        .await;
        let restore_result = restore_previous_link(&previous, &app, &state, local_port).await;

        let mut message = format!(
            "The public URL did not return a healthy response (HTTP {}). Changes have been rolled back.",
            check.status_code.unwrap_or_default()
        );
        if let Some(error) = check.error.as_deref() {
            message.push_str(&format!(" {}", error));
        }
        if let Err(err) = rollback_result {
            message.push_str(&format!(" Rollback error: {}", err));
        }
        if let Err(err) = restore_result {
            message.push_str(&format!(
                " Previous web link could not be restored automatically: {}",
                err
            ));
        }
        return Err(message);
    }

    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.tunnel_mode = "permanent".to_string();
        s.tunnel_provider = TunnelProvider::Cloudflared;
        s.permanent_domain = Some(public_url.clone());
        s.tunnel_url = Some(public_url.clone());
        s.tunnel_config = Some(token);
    }

    if check.reachable {
        let _ = app.emit(
            "docker-progress",
            "Cloudflare tunnel connected and verified!",
        );
        Ok(format!("Connected: {}", public_url))
    } else {
        let _ = app.emit(
            "docker-progress",
            "Tunnel is running, but the public URL is not reachable yet. DNS or Cloudflare propagation may still be in progress.",
        );
        Ok(format!("Connected (verification pending): {}", public_url))
    }
}

// ── verify_public_url ────────────────────────────────────────────────

async fn verify_public_url_inner(public_url: &str) -> PublicUrlCheck {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_default();

    match client.get(public_url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let reachable = status.is_success() || status.is_redirection();
            PublicUrlCheck {
                reachable,
                status_code: Some(status.as_u16()),
                error: if reachable {
                    None
                } else {
                    Some(format!("The URL returned HTTP {}.", status.as_u16()))
                },
            }
        }
        Err(e) => PublicUrlCheck {
            reachable: false,
            status_code: None,
            error: Some(e.to_string()),
        },
    }
}

#[tauri::command]
pub async fn verify_public_url(public_url: String) -> Result<PublicUrlCheck, String> {
    let public_url = public_url.trim().trim_end_matches('/').to_string();
    if public_url.is_empty() {
        return Err("URL is required.".to_string());
    }
    Ok(verify_public_url_inner(&public_url).await)
}
