use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use tauri::State;

use super::silent_cmd;
use crate::state::{ResumeState, SharedState, TunnelProvider};

const DPAPI_PREFIX: &str = "dpapi:";
const SECRET_DECRYPT_WARNING: &str =
    "Some saved credentials could not be decrypted on this Windows account, so they were cleared.";

fn default_resume_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Postiz")
}

fn get_resume_path(install_path: Option<&str>) -> PathBuf {
    if let Some(path) = install_path {
        PathBuf::from(path).join("install-state.json")
    } else {
        default_resume_dir().join("install-state.json")
    }
}

/// Write a pointer file at the default location so we can find custom install paths on restart.
/// Contains just the path to the real install directory.
/// Uses atomic temp-file-then-rename to avoid corruption on crash/power loss.
fn write_install_pointer(install_path: &str) -> Result<(), String> {
    let default_dir = default_resume_dir();
    let pointer_path = default_dir.join("install-pointer.json");

    fs::create_dir_all(&default_dir)
        .map_err(|e| format!("Failed to create pointer directory: {}", e))?;
    let pointer = serde_json::json!({ "install_path": install_path });
    let content = serde_json::to_string_pretty(&pointer)
        .map_err(|e| format!("Failed to serialize pointer: {}", e))?;
    let tmp = pointer_path.with_extension("tmp");
    fs::write(&tmp, &content).map_err(|e| format!("Failed to write pointer temp file: {}", e))?;
    fs::rename(&tmp, &pointer_path)
        .map_err(|e| format!("Failed to rename pointer temp file: {}", e))?;
    Ok(())
}

/// Read the pointer file to discover a custom install path.
fn read_install_pointer() -> Option<String> {
    let pointer_path = default_resume_dir().join("install-pointer.json");
    let contents = fs::read_to_string(&pointer_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&contents).ok()?;
    val["install_path"].as_str().map(|s| s.to_string())
}

fn build_resume_state_locked(app_state: &crate::state::AppState) -> ResumeState {
    let install_path = app_state
        .install_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    ResumeState {
        version: 1,
        current_step: app_state.current_step,
        install_path,
        port: app_state.port,
        tunnel_url: app_state.tunnel_url.clone(),
        tunnel_mode: app_state.tunnel_mode.clone(),
        permanent_domain: app_state.permanent_domain.clone(),
        providers_configured: app_state.providers_configured.iter().cloned().collect(),
        providers_stale: app_state.stale_providers.iter().cloned().collect(),
        transfer_review_pending: app_state.transfer_review_pending,
        tunnel_provider: app_state.tunnel_provider.as_str().to_string(),
        tunnel_config: app_state.tunnel_config.clone(),
        provider_credentials: app_state.provider_credentials.clone(),
        postiz_email: app_state.postiz_email.clone(),
        postiz_password: app_state.postiz_password.clone(),
        last_updated: chrono::Utc::now().to_rfc3339(),
    }
}

#[cfg(target_os = "windows")]
fn run_dpapi_script(script: &str, input: &str) -> Result<String, String> {
    let mut child = silent_cmd("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to launch PowerShell for secret protection: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("Failed to send secret to PowerShell: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to read PowerShell output: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "PowerShell secret protection failed.".to_string()
        } else {
            format!("PowerShell secret protection failed: {}", stderr)
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn protect_secret(value: &str) -> Result<String, String> {
    let protected = run_dpapi_script(
        r#"
$ErrorActionPreference = 'Stop'
[void][System.Reflection.Assembly]::LoadWithPartialName('System.Security')
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$plain = [Console]::In.ReadToEnd()
$bytes = [System.Text.Encoding]::UTF8.GetBytes($plain)
$protected = [System.Security.Cryptography.ProtectedData]::Protect(
  $bytes,
  $null,
  [System.Security.Cryptography.DataProtectionScope]::CurrentUser
)
[Console]::Out.Write([Convert]::ToBase64String($protected))
"#,
        value,
    )?;
    Ok(format!("{}{}", DPAPI_PREFIX, protected))
}

#[cfg(not(target_os = "windows"))]
fn protect_secret(value: &str) -> Result<String, String> {
    Ok(value.to_string())
}

#[cfg(target_os = "windows")]
fn unprotect_secret(value: &str) -> Result<String, String> {
    run_dpapi_script(
        r#"
$ErrorActionPreference = 'Stop'
[void][System.Reflection.Assembly]::LoadWithPartialName('System.Security')
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$protected = [Console]::In.ReadToEnd().Trim()
$bytes = [Convert]::FromBase64String($protected)
$plain = [System.Security.Cryptography.ProtectedData]::Unprotect(
  $bytes,
  $null,
  [System.Security.Cryptography.DataProtectionScope]::CurrentUser
)
[Console]::Out.Write([System.Text.Encoding]::UTF8.GetString($plain))
"#,
        value,
    )
}

#[cfg(not(target_os = "windows"))]
fn unprotect_secret(value: &str) -> Result<String, String> {
    Ok(value.to_string())
}

fn protect_optional_secret(value: Option<String>) -> Result<Option<String>, String> {
    value.map(|v| protect_secret(&v)).transpose()
}

fn maybe_unprotect_secret(value: Option<String>) -> (Option<String>, bool) {
    match value {
        Some(secret) => {
            if let Some(protected) = secret.strip_prefix(DPAPI_PREFIX) {
                match unprotect_secret(protected) {
                    Ok(unprotected) => (Some(unprotected), false),
                    Err(_) => (None, true),
                }
            } else {
                (Some(secret), false)
            }
        }
        None => (None, false),
    }
}

fn protect_provider_credentials(
    provider_credentials: HashMap<String, HashMap<String, String>>,
) -> Result<HashMap<String, HashMap<String, String>>, String> {
    provider_credentials
        .into_iter()
        .map(|(provider, entries)| {
            let protected_entries = entries
                .into_iter()
                .map(|(key, value)| protect_secret(&value).map(|protected| (key, protected)))
                .collect::<Result<HashMap<_, _>, _>>()?;
            Ok((provider, protected_entries))
        })
        .collect()
}

fn maybe_unprotect_provider_credentials(
    provider_credentials: HashMap<String, HashMap<String, String>>,
) -> (HashMap<String, HashMap<String, String>>, bool) {
    let mut had_failure = false;

    let restored = provider_credentials
        .into_iter()
        .map(|(provider, entries)| {
            let mut restored_entries = HashMap::new();
            for (key, value) in entries {
                if let Some(protected) = value.strip_prefix(DPAPI_PREFIX) {
                    match unprotect_secret(protected) {
                        Ok(unprotected) => {
                            restored_entries.insert(key, unprotected);
                        }
                        Err(_) => {
                            had_failure = true;
                        }
                    }
                } else {
                    restored_entries.insert(key, value);
                }
            }
            (provider, restored_entries)
        })
        .collect();

    (restored, had_failure)
}

fn protect_resume_secrets(mut resume: ResumeState) -> Result<ResumeState, String> {
    resume.tunnel_config = protect_optional_secret(resume.tunnel_config)?;
    resume.provider_credentials = protect_provider_credentials(resume.provider_credentials)?;
    resume.postiz_email = protect_optional_secret(resume.postiz_email)?;
    resume.postiz_password = protect_optional_secret(resume.postiz_password)?;
    Ok(resume)
}

fn maybe_unprotect_resume_secrets(mut resume: ResumeState) -> (ResumeState, bool) {
    let (tunnel_config, tunnel_failed) = maybe_unprotect_secret(resume.tunnel_config);
    let (provider_credentials, provider_failed) =
        maybe_unprotect_provider_credentials(resume.provider_credentials);
    let (postiz_email, email_failed) = maybe_unprotect_secret(resume.postiz_email);
    let (postiz_password, password_failed) = maybe_unprotect_secret(resume.postiz_password);

    resume.tunnel_config = tunnel_config;
    resume.provider_credentials = provider_credentials;
    resume.postiz_email = postiz_email;
    resume.postiz_password = postiz_password;

    (
        resume,
        tunnel_failed || provider_failed || email_failed || password_failed,
    )
}

fn update_secret_warning(app_state: &mut crate::state::AppState, had_secret_errors: bool) {
    if had_secret_errors {
        if app_state.last_error.is_none()
            || app_state.last_error.as_deref() == Some(SECRET_DECRYPT_WARNING)
        {
            app_state.last_error = Some(SECRET_DECRYPT_WARNING.to_string());
        }
    } else if app_state.last_error.as_deref() == Some(SECRET_DECRYPT_WARNING) {
        app_state.last_error = None;
    }
}

fn write_resume_file(resume: ResumeState, install_path: Option<String>) -> Result<(), String> {
    let path = get_resume_path(install_path.as_deref());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let resume = protect_resume_secrets(resume)?;
    let json = serde_json::to_string_pretty(&resume)
        .map_err(|e| format!("Failed to serialize state: {}", e))?;

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &json).map_err(|e| format!("Failed to write resume state temp file: {}", e))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("Failed to rename resume state temp file: {}", e))?;

    if let Some(ref ip) = install_path {
        let default_path_str = default_resume_dir().to_string_lossy().to_string();
        if !ip.starts_with(&default_path_str) {
            write_install_pointer(ip)?;
        }
    }

    Ok(())
}

#[tauri::command]
pub fn load_resume_state(
    install_path: Option<String>,
    state: State<SharedState>,
) -> Result<Option<ResumeState>, String> {
    // Try the provided path first
    let path = get_resume_path(install_path.as_deref());

    let resume_path = if path.exists() {
        path
    } else if install_path.is_none() {
        // No explicit path given and default doesn't exist — check pointer file
        if let Some(pointer_path) = read_install_pointer() {
            let p = PathBuf::from(&pointer_path).join("install-state.json");
            if p.exists() {
                p
            } else {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }
    } else {
        return Ok(None);
    };

    let contents = fs::read_to_string(&resume_path)
        .map_err(|e| format!("Failed to read resume state: {}", e))?;

    let resume: ResumeState = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse resume state: {}", e))?;
    let (resume, had_secret_errors) = maybe_unprotect_resume_secrets(resume);

    // Restore Rust AppState from resume data so tray menu and save work correctly
    {
        let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref ip) = resume.install_path {
            app_state.install_path = Some(PathBuf::from(ip));
        }
        app_state.port = resume.port;
        app_state.local_url = Some(format!("http://localhost:{}", resume.port));
        // Do NOT restore tunnel_url — it must be verified as alive first.
        // The frontend will check via get_tunnel_status after load.
        app_state.current_step = resume.current_step;
        for p in &resume.providers_configured {
            app_state.providers_configured.insert(p.clone());
        }
        for p in &resume.providers_stale {
            app_state.stale_providers.insert(p.clone());
        }
        app_state.tunnel_mode = resume.tunnel_mode.clone();
        app_state.tunnel_provider = TunnelProvider::from_str_loose(&resume.tunnel_provider);
        app_state.tunnel_config = resume.tunnel_config.clone();
        app_state.permanent_domain = resume.permanent_domain.clone();
        app_state.transfer_review_pending = resume.transfer_review_pending;
        app_state.provider_credentials = resume.provider_credentials.clone();
        app_state.postiz_email = resume.postiz_email.clone();
        app_state.postiz_password = resume.postiz_password.clone();
        update_secret_warning(&mut app_state, had_secret_errors);
    }

    Ok(Some(resume))
}

#[tauri::command]
pub fn save_resume_state(state: State<SharedState>) -> Result<String, String> {
    let (resume, install_path) = {
        let app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        let resume = build_resume_state_locked(&app_state);
        let install_path = resume.install_path.clone();
        (resume, install_path)
        // Lock is dropped here before file I/O
    };

    write_resume_file(resume, install_path)?;

    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    update_secret_warning(&mut app_state, false);

    Ok("Resume state saved.".to_string())
}

#[tauri::command]
pub fn update_step(step: usize, state: State<SharedState>) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.current_step = step;
    Ok(())
}

/// Sync frontend tunnel config to Rust state so snapshot/resume captures it correctly.
#[tauri::command]
pub fn sync_tunnel_config(
    tunnel_mode: String,
    permanent_domain: Option<String>,
    tunnel_provider: Option<String>,
    tunnel_config: Option<String>,
    state: State<SharedState>,
) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.tunnel_mode = tunnel_mode;
    app_state.permanent_domain = permanent_domain;
    app_state.tunnel_config = tunnel_config;
    if let Some(ref provider) = tunnel_provider {
        app_state.tunnel_provider = TunnelProvider::from_str_loose(provider);
    }
    Ok(())
}

/// Sync frontend provider status to Rust state so save_resume_state captures it correctly.
#[tauri::command]
pub fn sync_provider_status(
    configured: Vec<String>,
    stale: Vec<String>,
    state: State<SharedState>,
) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.providers_configured = configured.into_iter().collect();
    app_state.stale_providers = stale.into_iter().collect();
    Ok(())
}

/// Atomically clear `transfer_review_pending` and persist resume state to disk.
/// This avoids the race where a separate `save_resume_state` call could persist
/// the old `true` value before the flag clear takes effect.
#[tauri::command]
pub fn clear_transfer_review_and_save(state: State<SharedState>) -> Result<(), String> {
    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    app_state.transfer_review_pending = false;

    let resume = build_resume_state_locked(&app_state);
    let install_path = resume.install_path.clone();

    // Drop the lock before doing I/O
    drop(app_state);

    write_resume_file(resume, install_path)?;

    let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
    update_secret_warning(&mut app_state, false);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{maybe_unprotect_resume_secrets, protect_resume_secrets};
    use crate::state::ResumeState;
    use std::collections::HashMap;

    #[test]
    fn legacy_plaintext_resume_state_still_loads() {
        let mut credentials = HashMap::new();
        credentials.insert("api_key".to_string(), "plain-secret".to_string());

        let resume = ResumeState {
            tunnel_config: Some("plain-token".to_string()),
            provider_credentials: HashMap::from([("x".to_string(), credentials)]),
            postiz_email: Some("owner@example.com".to_string()),
            postiz_password: Some("plain-password".to_string()),
            ..ResumeState::default()
        };

        let (restored, had_failures) = maybe_unprotect_resume_secrets(resume);
        assert!(!had_failures);
        assert_eq!(restored.tunnel_config.as_deref(), Some("plain-token"));
        assert_eq!(restored.postiz_password.as_deref(), Some("plain-password"));
        assert_eq!(
            restored
                .provider_credentials
                .get("x")
                .and_then(|entries| entries.get("api_key"))
                .map(String::as_str),
            Some("plain-secret")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn protected_resume_secrets_round_trip() {
        let mut credentials = HashMap::new();
        credentials.insert("api_key".to_string(), "super-secret".to_string());

        let resume = ResumeState {
            tunnel_config: Some("tunnel-token".to_string()),
            provider_credentials: HashMap::from([("x".to_string(), credentials)]),
            postiz_email: Some("owner@example.com".to_string()),
            postiz_password: Some("correct horse battery staple".to_string()),
            ..ResumeState::default()
        };

        let protected = protect_resume_secrets(resume).expect("resume secrets should protect");
        assert!(protected
            .tunnel_config
            .as_deref()
            .is_some_and(|value| value.starts_with(super::DPAPI_PREFIX)));

        let (restored, had_failures) = maybe_unprotect_resume_secrets(protected);
        assert!(!had_failures);
        assert_eq!(restored.tunnel_config.as_deref(), Some("tunnel-token"));
        assert_eq!(
            restored.postiz_password.as_deref(),
            Some("correct horse battery staple")
        );
        assert_eq!(
            restored
                .provider_credentials
                .get("x")
                .and_then(|entries| entries.get("api_key"))
                .map(String::as_str),
            Some("super-secret")
        );
    }
}
