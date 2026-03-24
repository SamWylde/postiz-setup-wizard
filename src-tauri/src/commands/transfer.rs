use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read as IoRead, Write as IoWrite};
use std::path::PathBuf;
use std::process::Command;
use tauri::{Emitter, State};

use crate::commands::env_file::parse_env_file;
use crate::state::SharedState;

// --- Types ---

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VolumeEntry {
    pub archive_name: String,
    pub mount_path: String,
    pub service: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CloneManifest {
    pub format_version: u32,
    pub created_at: String,
    pub source_hostname: String,
    pub source_port: u16,
    pub wizard_version: String,
    pub tunnel_mode: String,
    #[serde(default)]
    pub tunnel_provider: String,
    pub providers_configured: Vec<String>,
    pub volumes: Vec<VolumeEntry>,
}

#[derive(Clone, Serialize)]
pub struct TransferProgress {
    pub phase: String,
    pub message: String,
    pub current: Option<u32>,
    pub total: Option<u32>,
}

// Known volume mount paths from our docker-compose template
const VOLUME_MOUNTS: &[(&str, &str, &str)] = &[
    ("postiz", "/config/", "postiz-config"),
    ("postiz", "/uploads/", "postiz-uploads"),
    ("postiz-postgres", "/var/lib/postgresql/data", "postgres-volume"),
    ("postiz-redis", "/data", "postiz-redis-data"),
    (
        "temporal-elasticsearch",
        "/var/lib/elasticsearch/data",
        "elasticsearch-data",
    ),
    (
        "temporal-postgresql",
        "/var/lib/postgresql/data",
        "temporal-postgres-data",
    ),
];

fn emit_progress(app: &tauri::AppHandle, phase: &str, message: &str, current: Option<u32>, total: Option<u32>) {
    let _ = app.emit(
        "transfer-progress",
        TransferProgress {
            phase: phase.to_string(),
            message: message.to_string(),
            current,
            total,
        },
    );
}

// --- Volume Discovery ---

/// Inspect a container to find the actual Docker volume name for a given mount path.
fn discover_volume_name(container_name: &str, mount_path: &str) -> Result<String, String> {
    let output = Command::new("docker")
        .args(["inspect", container_name, "--format", "json"])
        .output()
        .map_err(|e| format!("Failed to inspect container {}: {}", container_name, e))?;

    if !output.status.success() {
        return Err(format!(
            "docker inspect failed for {}: {}",
            container_name,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let containers: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse inspect output: {}", e))?;

    let container = containers
        .first()
        .ok_or_else(|| format!("No inspect data for {}", container_name))?;

    let mounts = container["Mounts"]
        .as_array()
        .ok_or_else(|| format!("No Mounts found for {}", container_name))?;

    for mount in mounts {
        let dest = mount["Destination"].as_str().unwrap_or("");
        // Normalize: strip trailing slashes for comparison
        let dest_norm = dest.trim_end_matches('/');
        let target_norm = mount_path.trim_end_matches('/');
        if dest_norm == target_norm {
            if let Some(name) = mount["Name"].as_str() {
                return Ok(name.to_string());
            }
        }
    }

    Err(format!(
        "Volume for mount path {} not found in container {}",
        mount_path, container_name
    ))
}

/// Discover all volume names by inspecting running containers.
fn discover_all_volumes() -> Result<Vec<(VolumeEntry, String)>, String> {
    let mut results = Vec::new();

    for (service, mount_path, logical_name) in VOLUME_MOUNTS {
        match discover_volume_name(service, mount_path) {
            Ok(actual_name) => {
                results.push((
                    VolumeEntry {
                        archive_name: format!("{}.tar", logical_name),
                        mount_path: mount_path.to_string(),
                        service: service.to_string(),
                    },
                    actual_name,
                ));
            }
            Err(e) => {
                log::warn!("Could not discover volume for {}/{}: {}", service, mount_path, e);
                // Non-fatal: skip volumes that don't exist (e.g., fresh install)
            }
        }
    }

    if results.is_empty() {
        return Err("No Docker volumes found. Is the stack running?".to_string());
    }

    Ok(results)
}

fn rewrite_env_urls(contents: &str, new_port: u16) -> String {
    let base_url = format!("http://localhost:{}", new_port);
    let backend_url = format!("{}/api", base_url);

    let mut lines: Vec<String> = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some((key, _)) = trimmed.split_once('=') {
            match key.trim() {
                "MAIN_URL" => lines.push(format!("MAIN_URL={}", base_url)),
                "FRONTEND_URL" => lines.push(format!("FRONTEND_URL={}", base_url)),
                "NEXT_PUBLIC_BACKEND_URL" => {
                    lines.push(format!("NEXT_PUBLIC_BACKEND_URL={}", backend_url))
                }
                _ => lines.push(line.to_string()),
            }
        } else {
            lines.push(line.to_string());
        }
    }
    let mut result = lines.join("\n");
    if contents.ends_with('\n') {
        result.push('\n');
    }
    result
}

// --- Export ---

#[tauri::command]
pub async fn export_clone(
    path: String,
    password: String,
    output_path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_path = PathBuf::from(&path);

    if password.len() < 4 {
        return Err("Password must be at least 4 characters.".to_string());
    }

    // Verify install exists
    if !install_path.join("docker-compose.yml").exists() {
        return Err("No installation found at the specified path.".to_string());
    }

    // Step 1: Discover volumes while containers are still running
    emit_progress(&app, "discovering", "Discovering Docker volumes...", None, None);

    let volumes = discover_all_volumes()?;
    let volume_count = volumes.len() as u32;

    // Get state info for manifest
    let (port, tunnel_mode, tunnel_provider, providers) = {
        let app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        (
            app_state.port,
            app_state.tunnel_mode.clone(),
            app_state.tunnel_provider.as_str().to_string(),
            app_state.providers_configured.iter().cloned().collect::<Vec<_>>(),
        )
    };

    // Step 2: Stop stack for consistent backup
    emit_progress(&app, "stopping", "Stopping services for consistent backup...", None, None);

    let stop_result = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "down"])
        .current_dir(&install_path)
        .output();

    // Always restart on exit, even if export fails
    let install_path_for_restart = install_path.clone();
    let restart_on_exit = scopeguard::guard((), |_| {
        let _ = Command::new("docker")
            .args(["compose", "--env-file", "postiz.env", "up", "-d"])
            .current_dir(&install_path_for_restart)
            .output();
    });

    if let Err(e) = stop_result {
        return Err(format!("Failed to stop stack: {}", e));
    }

    // Step 3: Backup volumes using temporary alpine containers
    let temp_dir = std::env::temp_dir().join(format!("postiz-export-{}", std::process::id()));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    // Cleanup temp dir on exit
    let temp_dir_cleanup = temp_dir.clone();
    let _cleanup = scopeguard::guard((), |_| {
        let _ = fs::remove_dir_all(&temp_dir_cleanup);
    });

    // Step 4: Build zip archive in memory, streaming each volume directly
    emit_progress(&app, "encrypting", "Building archive...", None, None);

    let mut zip_buffer = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_buffer));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // Collect volume entries for the manifest
        let volume_entries: Vec<VolumeEntry> = volumes.iter().map(|(e, _)| e.clone()).collect();

        // Build manifest
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let manifest = CloneManifest {
            format_version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_hostname: hostname,
            source_port: port,
            wizard_version: env!("CARGO_PKG_VERSION").to_string(),
            tunnel_mode,
            tunnel_provider,
            providers_configured: providers,
            volumes: volume_entries,
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("Failed to serialize manifest: {}", e))?;

        zip.start_file("manifest.json", options)
            .map_err(|e| format!("Failed to write manifest to zip: {}", e))?;
        zip.write_all(manifest_json.as_bytes())
            .map_err(|e| format!("Failed to write manifest bytes: {}", e))?;

        // Add install files
        let files_to_include = [
            ("files/docker-compose.yml", install_path.join("docker-compose.yml")),
            ("files/postiz.env", install_path.join("postiz.env")),
            (
                "files/dynamicconfig/development-sql.yaml",
                install_path.join("dynamicconfig").join("development-sql.yaml"),
            ),
        ];

        for (zip_path, disk_path) in &files_to_include {
            if disk_path.exists() {
                let contents = fs::read(disk_path)
                    .map_err(|e| format!("Failed to read {}: {}", disk_path.display(), e))?;
                zip.start_file(*zip_path, options)
                    .map_err(|e| format!("Failed to add {} to zip: {}", zip_path, e))?;
                zip.write_all(&contents)
                    .map_err(|e| format!("Failed to write {} bytes: {}", zip_path, e))?;
            }
        }

        // Stream each volume tar directly into the zip (one at a time, not all in memory)
        for (idx, (entry, actual_name)) in volumes.iter().enumerate() {
            emit_progress(
                &app,
                "backing_up",
                &format!("Backing up volume: {} ({}/{})", entry.service, idx + 1, volume_count),
                Some(idx as u32 + 1),
                Some(volume_count),
            );

            let tar_name = &entry.archive_name;
            let tar_path = temp_dir.join(tar_name);

            // Use docker run to tar the volume contents to a temp file on disk
            let output = Command::new("docker")
                .args([
                    "run", "--rm",
                    "-v", &format!("{}:/source:ro", actual_name),
                    "-v", &format!("{}:/backup", temp_dir.to_string_lossy()),
                    "alpine",
                    "tar", "cf", &format!("/backup/{}", tar_name), "-C", "/source", ".",
                ])
                .output()
                .map_err(|e| format!("Failed to backup volume {}: {}", actual_name, e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("Volume backup failed for {}: {}", actual_name, stderr));
            }

            // Stream from disk into the zip, then delete the temp tar
            let zip_path = format!("volumes/{}", entry.archive_name);
            zip.start_file(&zip_path, options)
                .map_err(|e| format!("Failed to add {} to zip: {}", zip_path, e))?;

            let mut tar_file = fs::File::open(&tar_path)
                .map_err(|e| format!("Failed to open tar file {}: {}", tar_name, e))?;
            std::io::copy(&mut tar_file, &mut zip)
                .map_err(|e| format!("Failed to stream {} into zip: {}", tar_name, e))?;
            drop(tar_file);

            // Remove temp tar immediately to free disk space
            let _ = fs::remove_file(&tar_path);
        }

        zip.finish()
            .map_err(|e| format!("Failed to finalize zip: {}", e))?;
    }

    // Step 5: Encrypt with age
    emit_progress(&app, "encrypting", "Encrypting archive...", None, None);

    let mut encrypted = Vec::new();
    {
        let encryptor = age::Encryptor::with_user_passphrase(age::secrecy::SecretString::from(password));
        let mut writer = encryptor
            .wrap_output(&mut encrypted)
            .map_err(|e| format!("Failed to create age encryptor: {}", e))?;

        writer
            .write_all(&zip_buffer)
            .map_err(|e| format!("Failed to encrypt data: {}", e))?;
        writer
            .finish()
            .map_err(|e| format!("Failed to finalize encryption: {}", e))?;
    }

    fs::write(&output_path, &encrypted)
        .map_err(|e| format!("Failed to write archive: {}", e))?;

    emit_progress(&app, "restarting", "Restarting services...", None, None);

    // scopeguard will restart the stack
    drop(restart_on_exit);

    Ok(output_path)
}

// --- Validate ---

#[tauri::command]
pub async fn validate_clone_file(
    clone_path: String,
    password: String,
) -> Result<CloneManifest, String> {
    if password.len() < 4 {
        return Err("Password must be at least 4 characters.".to_string());
    }

    let encrypted = fs::read(&clone_path)
        .map_err(|e| format!("Failed to read archive: {}", e))?;

    let decryptor = age::Decryptor::new(&encrypted[..])
        .map_err(|e| format!("Invalid archive format: {}", e))?;

    if !decryptor.is_scrypt() {
        return Err("Archive is not password-protected.".to_string());
    }

    let mut decrypted = Vec::new();
    let identity = age::scrypt::Identity::new(age::secrecy::SecretString::from(password));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| "Invalid password.".to_string())?;
    reader
        .read_to_end(&mut decrypted)
        .map_err(|e| format!("Failed to decrypt archive: {}", e))?;

    let cursor = std::io::Cursor::new(&decrypted);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Invalid archive contents: {}", e))?;

    let manifest_str = {
        let mut manifest_file = zip
            .by_name("manifest.json")
            .map_err(|_| "Archive is missing manifest.json".to_string())?;
        let mut buf = String::new();
        manifest_file
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read manifest: {}", e))?;
        buf
    };

    let manifest: CloneManifest = serde_json::from_str(&manifest_str)
        .map_err(|e| format!("Invalid manifest: {}", e))?;

    if manifest.format_version != 1 {
        return Err(format!(
            "Unsupported archive version: {}. Please update the app.",
            manifest.format_version
        ));
    }

    Ok(manifest)
}

// --- Import ---

const DOCKER_COMPOSE_TEMPLATE: &str = include_str!("../templates/docker-compose.yml");
const DYNAMIC_CONFIG_TEMPLATE: &str =
    include_str!("../templates/dynamicconfig/development-sql.yaml");

#[tauri::command]
pub async fn import_clone(
    clone_path: String,
    password: String,
    install_path: String,
    custom_port: Option<u16>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let install_dir = PathBuf::from(&install_path);

    if password.len() < 4 {
        return Err("Password must be at least 4 characters.".to_string());
    }

    // Preflight: port
    let port = if let Some(cp) = custom_port {
        if cp < 1024 {
            return Err(format!("Port {} is reserved. Choose 1024-65535.", cp));
        }
        if std::net::TcpListener::bind(("127.0.0.1", cp)).is_err() {
            return Err(format!("Port {} is already in use.", cp));
        }
        cp
    } else {
        const PORT_RANGE_START: u16 = 4007;
        const PORT_RANGE_END: u16 = 5007;
        let mut found_port = None;
        for p in PORT_RANGE_START..=PORT_RANGE_END {
            if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
                found_port = Some(p);
                break;
            }
        }
        found_port.ok_or_else(|| {
            format!(
                "Could not find a free port in range {}-{}. Free up a port or specify one manually.",
                PORT_RANGE_START, PORT_RANGE_END
            )
        })?
    };

    // Preflight: install path
    if install_dir.join("docker-compose.yml").exists() {
        return Err("An installation already exists at this path. Choose a different location.".to_string());
    }

    // Step 1: Decrypt
    emit_progress(&app, "decrypting", "Decrypting archive...", None, None);

    let encrypted = fs::read(&clone_path)
        .map_err(|e| format!("Failed to read archive: {}", e))?;

    let decryptor = age::Decryptor::new(&encrypted[..])
        .map_err(|e| format!("Invalid archive format: {}", e))?;

    if !decryptor.is_scrypt() {
        return Err("Archive is not password-protected.".to_string());
    }

    let mut decrypted = Vec::new();
    let identity = age::scrypt::Identity::new(age::secrecy::SecretString::from(password));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| "Invalid password.".to_string())?;
    reader
        .read_to_end(&mut decrypted)
        .map_err(|e| format!("Failed to decrypt: {}", e))?;

    let cursor = std::io::Cursor::new(&decrypted);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Invalid archive: {}", e))?;

    // Step 2: Read manifest
    let manifest: CloneManifest = {
        let mut f = zip.by_name("manifest.json").map_err(|_| "Missing manifest.json".to_string())?;
        let mut buf = String::new();
        f.read_to_string(&mut buf).map_err(|e| format!("Failed to read manifest: {}", e))?;
        serde_json::from_str(&buf).map_err(|e| format!("Invalid manifest: {}", e))?
    };

    if manifest.format_version != 1 {
        return Err(format!("Unsupported archive version: {}", manifest.format_version));
    }

    // Step 3: Extract env file and get postgres password
    emit_progress(&app, "extracting", "Extracting configuration...", None, None);

    let env_contents = {
        let mut f = zip.by_name("files/postiz.env").map_err(|_| "Missing postiz.env in archive".to_string())?;
        let mut buf = String::new();
        f.read_to_string(&mut buf).map_err(|e| format!("Failed to read postiz.env: {}", e))?;
        buf
    };

    let env_map = parse_env_file(&env_contents);
    let postgres_password = env_map
        .get("POSTGRES_PASSWORD")
        .ok_or("postiz.env is missing POSTGRES_PASSWORD")?
        .clone();

    // Step 4: Write install files
    fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Failed to create install directory: {}", e))?;

    // From this point, cleanup on failure
    let cleanup_dir = install_dir.clone();
    let should_cleanup = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let should_cleanup_clone = should_cleanup.clone();
    let _cleanup = scopeguard::guard((), |_| {
        if should_cleanup_clone.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = Command::new("docker")
                .args(["compose", "down", "--volumes"])
                .current_dir(&cleanup_dir)
                .output();
            let _ = fs::remove_dir_all(&cleanup_dir);
        }
    });

    // Regenerate docker-compose.yml from template with new port
    let compose_contents = DOCKER_COMPOSE_TEMPLATE
        .replace("{{PORT}}", &port.to_string())
        .replace("{{POSTGRES_PASSWORD}}", &postgres_password);
    fs::write(install_dir.join("docker-compose.yml"), &compose_contents)
        .map_err(|e| format!("Failed to write docker-compose.yml: {}", e))?;

    // Rewrite env URLs for new port
    let new_env = rewrite_env_urls(&env_contents, port);
    fs::write(install_dir.join("postiz.env"), &new_env)
        .map_err(|e| format!("Failed to write postiz.env: {}", e))?;

    // Write dynamicconfig
    let dynconfig_dir = install_dir.join("dynamicconfig");
    fs::create_dir_all(&dynconfig_dir)
        .map_err(|e| format!("Failed to create dynamicconfig dir: {}", e))?;
    fs::write(dynconfig_dir.join("development-sql.yaml"), DYNAMIC_CONFIG_TEMPLATE)
        .map_err(|e| format!("Failed to write dynamic config: {}", e))?;

    // Step 5: Pull images
    emit_progress(&app, "pulling", "Pulling Docker images (this may take several minutes)...", None, None);

    let pull_output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "pull"])
        .current_dir(&install_dir)
        .output()
        .map_err(|e| format!("Failed to pull images: {}", e))?;

    if !pull_output.status.success() {
        let stderr = String::from_utf8_lossy(&pull_output.stderr);
        return Err(format!("Failed to pull Docker images: {}", stderr));
    }

    // Step 6: Create containers (without starting) to establish volumes
    emit_progress(&app, "restoring", "Creating containers...", None, None);

    let create_output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "create"])
        .current_dir(&install_dir)
        .output()
        .map_err(|e| format!("Failed to create containers: {}", e))?;

    if !create_output.status.success() {
        let stderr = String::from_utf8_lossy(&create_output.stderr);
        return Err(format!("Failed to create containers: {}", stderr));
    }

    // Step 7: Restore volumes
    let volume_count = manifest.volumes.len() as u32;
    let temp_dir = std::env::temp_dir().join(format!("postiz-import-{}", std::process::id()));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let temp_dir_cleanup = temp_dir.clone();
    let _temp_cleanup = scopeguard::guard((), |_| {
        let _ = fs::remove_dir_all(&temp_dir_cleanup);
    });

    for (idx, volume_entry) in manifest.volumes.iter().enumerate() {
        emit_progress(
            &app,
            "restoring",
            &format!("Restoring volume: {} ({}/{})", volume_entry.service, idx + 1, volume_count),
            Some(idx as u32 + 1),
            Some(volume_count),
        );

        // Find the new volume name by inspecting the new container
        let new_volume_name = discover_volume_name(&volume_entry.service, &volume_entry.mount_path)?;

        // Extract tar from zip to temp file
        let tar_zip_path = format!("volumes/{}", volume_entry.archive_name);
        let tar_bytes = {
            let mut f = zip.by_name(&tar_zip_path).map_err(|_| {
                format!("Missing {} in archive", tar_zip_path)
            })?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)
                .map_err(|e| format!("Failed to read {}: {}", tar_zip_path, e))?;
            buf
        };

        let tar_temp_path = temp_dir.join(&volume_entry.archive_name);
        fs::write(&tar_temp_path, &tar_bytes)
            .map_err(|e| format!("Failed to write temp tar: {}", e))?;

        // Restore into volume
        let output = Command::new("docker")
            .args([
                "run", "--rm",
                "-v", &format!("{}:/dest", new_volume_name),
                "-v", &format!("{}:/backup:ro", temp_dir.to_string_lossy()),
                "alpine",
                "sh", "-c",
                &format!("rm -rf /dest/* && tar xf /backup/{} -C /dest", volume_entry.archive_name),
            ])
            .output()
            .map_err(|e| format!("Failed to restore volume {}: {}", new_volume_name, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Volume restore failed for {}: {}", new_volume_name, stderr));
        }

        // Clean up tar temp file to save space
        let _ = fs::remove_file(&tar_temp_path);
    }

    // Step 8: Start the stack
    emit_progress(&app, "starting", "Starting services...", None, None);

    let start_output = Command::new("docker")
        .args(["compose", "--env-file", "postiz.env", "up", "-d"])
        .current_dir(&install_dir)
        .output()
        .map_err(|e| format!("Failed to start stack: {}", e))?;

    if !start_output.status.success() {
        let stderr = String::from_utf8_lossy(&start_output.stderr);
        return Err(format!("Failed to start services: {}", stderr));
    }

    // Step 9: Health check
    emit_progress(&app, "health_check", "Waiting for services to become healthy...", None, None);

    let client = reqwest::Client::new();
    let mut healthy = false;
    for attempt in 1..=40 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        emit_progress(
            &app,
            "health_check",
            &format!("Health check attempt {}/40...", attempt),
            Some(attempt),
            Some(40),
        );

        let responding = client
            .get(format!("http://localhost:{}", port))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().is_redirection())
            .unwrap_or(false);

        if responding {
            healthy = true;
            break;
        }
    }

    if !healthy {
        // Don't clean up — leave install in place so Recovery Center can handle it
        should_cleanup.store(false, std::sync::atomic::Ordering::Relaxed);
        return Err("Health check timed out. Services may still be starting. Check the Recovery Center.".to_string());
    }

    // Step 10: Update state
    {
        let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        app_state.install_path = Some(install_dir.clone());
        app_state.port = port;
        app_state.local_url = Some(format!("http://localhost:{}", port));
        app_state.current_step = 3; // CreateWebLink
        app_state.transfer_review_pending = true;
        app_state.tunnel_mode = manifest.tunnel_mode.clone();
        app_state.tunnel_provider = crate::state::TunnelProvider::from_str_loose(&manifest.tunnel_provider);

        // Mark all imported providers as stale (callback URLs need updating)
        for provider in &manifest.providers_configured {
            app_state.providers_configured.insert(provider.clone());
            app_state.stale_providers.insert(provider.clone());
        }
    }

    // Write install pointer
    let pointer_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Postiz");
    let _ = fs::create_dir_all(&pointer_dir);
    let pointer_path = pointer_dir.join("install-pointer.json");
    let pointer = serde_json::json!({ "install_path": install_path });
    let content = serde_json::to_string_pretty(&pointer).unwrap_or_default();
    let tmp = pointer_path.with_extension("tmp");
    fs::write(&tmp, &content)
        .map_err(|e| format!("Failed to write install pointer: {}", e))?;
    fs::rename(&tmp, &pointer_path)
        .map_err(|e| format!("Failed to rename install pointer: {}", e))?;

    // Disable cleanup — import succeeded
    should_cleanup.store(false, std::sync::atomic::Ordering::Relaxed);

    emit_progress(&app, "complete", "Import complete!", None, None);

    // Return port as JSON so frontend can read the actual assigned port
    Ok(serde_json::json!({ "port": port }).to_string())
}

