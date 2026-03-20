use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use tauri::State;

use crate::state::SharedState;

#[tauri::command]
pub async fn import_existing_install(
    path: String,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = PathBuf::from(&path);

    // 1. Check docker-compose.yml exists
    if !install_path.join("docker-compose.yml").exists() {
        return Err(format!(
            "No docker-compose.yml found at {}. This does not appear to be a Postiz installation.",
            path
        ));
    }

    // 2. Check postiz.env exists
    let env_path = install_path.join("postiz.env");
    if !env_path.exists() {
        return Err(format!(
            "No postiz.env found at {}. This does not appear to be a Postiz installation.",
            path
        ));
    }

    // 3. Read postiz.env and extract port from MAIN_URL
    let env_contents =
        fs::read_to_string(&env_path).map_err(|e| format!("Failed to read postiz.env: {}", e))?;

    let mut port: u16 = 4007; // default
    let mut env_map: Vec<(String, String)> = Vec::new();

    for line in env_contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim().to_string();
            env_map.push((key, value));
        }
    }

    // Extract port from MAIN_URL
    for (key, value) in &env_map {
        if key == "MAIN_URL" {
            // Parse port from URL like http://localhost:4007 or https://example.com:8080
            if let Some(port_str) = extract_port_from_url(value) {
                if let Ok(p) = port_str.parse::<u16>() {
                    port = p;
                }
            }
            break;
        }
    }

    // 4. Detect configured providers
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
        ("INSTAGRAM_APP_ID", "instagram"),
    ];

    let mut providers_configured: HashSet<String> = HashSet::new();

    for (env_key, provider_name) in provider_keys {
        for (key, value) in &env_map {
            if key == *env_key && !value.is_empty() {
                providers_configured.insert(provider_name.to_string());
                break;
            }
        }
    }

    let provider_count = providers_configured.len();

    // 5. Update AppState
    {
        let mut s = state
            .lock()
            .map_err(|e| format!("State lock failed: {}", e))?;
        s.install_path = Some(install_path.clone());
        s.port = port;
        s.local_url = Some(format!("http://localhost:{}", port));
        s.providers_configured = providers_configured;
    }

    // 6. Write pointer file
    let pointer_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Postiz");
    let pointer_path = pointer_dir.join("install-pointer.json");

    let _ = fs::create_dir_all(&pointer_dir);
    let pointer = serde_json::json!({ "install_path": path });
    fs::write(
        &pointer_path,
        serde_json::to_string_pretty(&pointer).unwrap_or_default(),
    )
    .map_err(|e| format!("Failed to write install pointer: {}", e))?;

    // 7. Return success
    Ok(format!(
        "Successfully imported existing Postiz installation. Detected port: {}, providers configured: {}.",
        port, provider_count
    ))
}

/// Extract the port portion from a URL string.
/// Handles formats like "http://localhost:4007", "https://example.com:8080/path"
fn extract_port_from_url(url: &str) -> Option<String> {
    // Remove scheme
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    // Remove path
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);

    // Handle IPv6 addresses like [::1]:port
    if host_port.contains('[') {
        // IPv6: look for ]:port
        if let Some(bracket_end) = host_port.rfind("]:") {
            return Some(host_port[bracket_end + 2..].to_string());
        }
        return None;
    }

    // Split on ':' and take the last part if it looks like a port
    let parts: Vec<&str> = host_port.split(':').collect();
    if parts.len() == 2 {
        Some(parts[1].to_string())
    } else {
        None
    }
}
