use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use tauri::State;

use crate::commands::secrets::generate_random_string;
use crate::commands::silent_cmd;
use crate::state::SharedState;

const DOCKER_COMPOSE_TEMPLATE: &str = include_str!("../templates/docker-compose.yml");
const DYNAMIC_CONFIG_TEMPLATE: &str = include_str!("../templates/dynamicconfig/development-sql.yaml");

fn find_free_port(start: u16) -> u16 {
    let end = start.saturating_add(1000);
    for port in start..=end {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    start // fallback if no free port found in range
}

fn generate_env_contents(port: u16, jwt_secret: &str, postgres_password: &str) -> String {
    format!(
        r#"# === Postiz Setup Wizard Generated Configuration ===
# Do not edit manually — managed by Postiz Setup Wizard

# === Core Settings
MAIN_URL=http://localhost:{port}
FRONTEND_URL=http://localhost:{port}
NEXT_PUBLIC_BACKEND_URL=http://localhost:{port}/api
JWT_SECRET={jwt_secret}
DATABASE_URL=postgresql://postiz-user:{postgres_password}@postiz-postgres:5432/postiz-db-local
REDIS_URL=redis://postiz-redis:6379
BACKEND_INTERNAL_URL=http://localhost:3000
IS_GENERAL=true
DISABLE_REGISTRATION=false
RUN_CRON=true

# === Database
POSTGRES_PASSWORD={postgres_password}
POSTGRES_USER=postiz-user
POSTGRES_DB=postiz-db-local

# === Temporal
TEMPORAL_ADDRESS=temporal:7233

# === Storage
STORAGE_PROVIDER=local
UPLOAD_DIRECTORY=/uploads
NEXT_PUBLIC_UPLOAD_DIRECTORY=/uploads

# === Social Media Providers
X_API_KEY=
X_API_SECRET=
LINKEDIN_CLIENT_ID=
LINKEDIN_CLIENT_SECRET=
REDDIT_CLIENT_ID=
REDDIT_CLIENT_SECRET=
GITHUB_CLIENT_ID=
GITHUB_CLIENT_SECRET=
THREADS_APP_ID=
THREADS_APP_SECRET=
FACEBOOK_APP_ID=
FACEBOOK_APP_SECRET=
# Instagram uses the same Facebook App (no separate keys needed)
YOUTUBE_CLIENT_ID=
YOUTUBE_CLIENT_SECRET=
TIKTOK_CLIENT_ID=
TIKTOK_CLIENT_SECRET=
PINTEREST_CLIENT_ID=
PINTEREST_CLIENT_SECRET=
DRIBBBLE_CLIENT_ID=
DRIBBBLE_CLIENT_SECRET=
DISCORD_CLIENT_ID=
DISCORD_CLIENT_SECRET=
DISCORD_BOT_TOKEN_ID=
SLACK_ID=
SLACK_SECRET=
SLACK_SIGNING_SECRET=
MASTODON_URL=https://mastodon.social
MASTODON_CLIENT_ID=
MASTODON_CLIENT_SECRET=
BEEHIIVE_API_KEY=
BEEHIIVE_PUBLICATION_ID=

# === Chrome Extension
EXTENSION_ID=icpokdlcikdmemjkeoojhocmhmehpaia

# === Misc
OPENAI_API_KEY=
NEXT_PUBLIC_DISCORD_SUPPORT=
NEXT_PUBLIC_POLOTNO=
API_LIMIT=30
NX_ADD_PLUGINS=false

# === Payments (optional)
FEE_AMOUNT=0.05
STRIPE_PUBLISHABLE_KEY=
STRIPE_SECRET_KEY=
STRIPE_SIGNING_KEY=
STRIPE_SIGNING_KEY_CONNECT=
"#,
        port = port,
        jwt_secret = jwt_secret,
        postgres_password = postgres_password,
    )
}

fn generate_compose_contents(port: u16, postgres_password: &str) -> String {
    DOCKER_COMPOSE_TEMPLATE
        .replace("{{PORT}}", &port.to_string())
        .replace("{{POSTGRES_PASSWORD}}", postgres_password)
}

#[tauri::command]
pub fn prepare_install(
    path: String,
    custom_port: Option<u16>,
    state: State<SharedState>,
) -> Result<u16, String> {
    let install_path = PathBuf::from(&path);
    let tmp_path = install_path.join(".tmp");

    // Create temp staging directory
    fs::create_dir_all(&tmp_path).map_err(|e| format!("Failed to create directory: {}", e))?;
    fs::create_dir_all(tmp_path.join("dynamicconfig"))
        .map_err(|e| format!("Failed to create dynamicconfig directory: {}", e))?;

    // Determine port — validate custom port is free, auto-select if default
    let port = if let Some(cp) = custom_port {
        if cp < 1024 {
            return Err(format!(
                "Port {} is reserved. Choose a port between 1024 and 65535.",
                cp
            ));
        }
        if TcpListener::bind(("127.0.0.1", cp)).is_err() {
            return Err(format!(
                "Port {} is already in use. Choose a different port or stop the conflicting service.",
                cp
            ));
        }
        cp
    } else {
        find_free_port(4007)
    };

    // Generate secrets
    let jwt_secret = generate_random_string(64);
    let postgres_password = generate_random_string(32);

    // Write docker-compose.yml
    let compose_contents = generate_compose_contents(port, &postgres_password);
    fs::write(tmp_path.join("docker-compose.yml"), &compose_contents)
        .map_err(|e| format!("Failed to write docker-compose.yml: {}", e))?;

    // Write dynamicconfig
    fs::write(
        tmp_path.join("dynamicconfig").join("development-sql.yaml"),
        DYNAMIC_CONFIG_TEMPLATE,
    )
    .map_err(|e| format!("Failed to write dynamic config: {}", e))?;

    // Write postiz.env
    let env_contents = generate_env_contents(port, &jwt_secret, &postgres_password);
    fs::write(tmp_path.join("postiz.env"), &env_contents)
        .map_err(|e| format!("Failed to write postiz.env: {}", e))?;

    // Update app state
    {
        let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        app_state.install_path = Some(install_path.clone());
        app_state.port = port;
        app_state.local_url = Some(format!("http://localhost:{}", port));
    }

    Ok(port)
}

#[tauri::command]
pub fn commit_install(path: String) -> Result<String, String> {
    let install_path = PathBuf::from(&path);
    let tmp_path = install_path.join(".tmp");

    // Guard: the .tmp staging folder MUST exist — it proves this wizard created
    // the files being committed. Without it we might overwrite a foreign install.
    let compose_exists = install_path.join("docker-compose.yml").exists();
    let env_exists = install_path.join("postiz.env").exists();
    if !tmp_path.exists() {
        if compose_exists || env_exists {
            return Err(
                "Refusing to overwrite an existing Postiz install. \
                 A docker-compose.yml or postiz.env already exists at the target path \
                 without a staged .tmp folder. Use recovery/import instead."
                    .to_string(),
            );
        }
        return Err("No staged install found. Run prepare_install first.".to_string());
    }

    // Backup existing env file BEFORE overwriting with new files
    let env_path = install_path.join("postiz.env");
    if env_path.exists() {
        fs::copy(&env_path, install_path.join("postiz.env.bak"))
            .map_err(|e| format!("Failed to create env backup: {}", e))?;
    }

    // Move files from .tmp to install root
    let entries = fs::read_dir(&tmp_path)
        .map_err(|e| format!("Failed to read tmp directory: {}", e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let dest = install_path.join(entry.file_name());

        if entry.path().is_dir() {
            // Copy directory recursively
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest)
                .map_err(|e| format!("Failed to copy file: {}", e))?;
        }
    }

    // Clean up tmp directory
    fs::remove_dir_all(&tmp_path)
        .map_err(|e| format!("Failed to clean up tmp directory: {}", e))?;

    Ok("Install committed successfully.".to_string())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("Failed to create dir {}: {}", dst.display(), e))?;

    for entry in
        fs::read_dir(src).map_err(|e| format!("Failed to read dir {}: {}", src.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let dst_path = dst.join(entry.file_name());

        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), &dst_path)
                .map_err(|e| format!("Failed to copy: {}", e))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_default_install_path() -> Result<String, String> {
    dirs::data_local_dir()
        .map(|d| d.join("Postiz").to_string_lossy().to_string())
        .ok_or_else(|| "Could not determine local app data directory".to_string())
}

#[tauri::command]
pub fn clean_staged_files(path: String) -> Result<String, String> {
    let tmp_path = PathBuf::from(&path).join(".tmp");
    if tmp_path.exists() {
        fs::remove_dir_all(&tmp_path)
            .map_err(|e| format!("Failed to remove staged files: {}", e))?;
        Ok("Staged files removed.".to_string())
    } else {
        Ok("No staged files found.".to_string())
    }
}

/// Stop any running containers and remove all Postiz files from an install directory,
/// allowing a fresh reinstall at the same path.
#[tauri::command]
pub fn wipe_existing_install(path: String, state: State<SharedState>) -> Result<String, String> {
    let install_path = PathBuf::from(&path);

    if !install_path.exists() {
        return Ok("Path does not exist — nothing to wipe.".to_string());
    }

    // Only wipe if it looks like a Postiz install (safety check)
    let compose = install_path.join("docker-compose.yml");
    let env = install_path.join("postiz.env");
    if !compose.exists() && !env.exists() {
        return Err(
            "This directory does not appear to be a Postiz install (no docker-compose.yml or postiz.env found)."
                .to_string(),
        );
    }

    // 1. Try to stop containers (best-effort — they might not be running)
    let _ = silent_cmd("docker")
        .args(["compose", "--env-file", "postiz.env", "down", "--remove-orphans", "-v"])
        .current_dir(&install_path)
        .output();

    // 2. Remove known Postiz files and directories
    let items_to_remove = [
        "docker-compose.yml",
        "postiz.env",
        "postiz.env.bak",
        "dynamicconfig",
        ".tmp",
    ];

    let mut errors = Vec::new();
    for item in &items_to_remove {
        let p = install_path.join(item);
        if p.is_dir() {
            if let Err(e) = fs::remove_dir_all(&p) {
                errors.push(format!("Failed to remove {}: {}", item, e));
            }
        } else if p.exists() {
            if let Err(e) = fs::remove_file(&p) {
                errors.push(format!("Failed to remove {}: {}", item, e));
            }
        }
    }

    // 3. Clear app state so we don't reference the old install
    {
        let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        app_state.install_path = None;
        app_state.port = 4007;
        app_state.local_url = None;
    }

    // 4. Remove pointer file so next launch doesn't try to resume
    if let Some(data_dir) = dirs::data_local_dir() {
        let pointer = data_dir.join("Postiz").join("install-pointer.json");
        let _ = fs::remove_file(&pointer);
        let state_file = data_dir.join("Postiz").join("install-state.json");
        let _ = fs::remove_file(&state_file);
    }

    if errors.is_empty() {
        Ok("Existing install wiped successfully. You can now reinstall.".to_string())
    } else {
        Err(format!(
            "Partially wiped but some files could not be removed:\n{}",
            errors.join("\n")
        ))
    }
}
