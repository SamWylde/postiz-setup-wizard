use tauri::State;

use super::silent_cmd;
use crate::state::SharedState;

#[tauri::command]
pub async fn export_diagnostics(state: State<'_, SharedState>) -> Result<String, String> {
    let (
        install_path,
        port,
        tunnel_mode,
        tunnel_url,
        providers_configured,
        providers_stale,
        current_step,
    ) = {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        (
            s.install_path.clone(),
            s.port,
            s.tunnel_mode.clone(),
            s.tunnel_url.clone(),
            s.providers_configured.iter().cloned().collect::<Vec<_>>(),
            s.stale_providers.iter().cloned().collect::<Vec<_>>(),
            s.current_step,
        )
    };

    let mut report = String::new();
    report.push_str("=== Postiz Setup Wizard Diagnostics ===\n");
    report.push_str(&format!(
        "Generated: {}\n\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));

    // Wizard state
    report.push_str("--- Wizard State ---\n");
    report.push_str(&format!(
        "Install path: {}\n",
        install_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "(not set)".to_string())
    ));
    report.push_str(&format!("Port: {}\n", port));
    report.push_str(&format!("Tunnel mode: {}\n", tunnel_mode));
    report.push_str(&format!(
        "Tunnel URL: {}\n",
        tunnel_url.as_deref().unwrap_or("(none)")
    ));
    report.push_str(&format!("Current step: {}\n", current_step));
    report.push_str(&format!(
        "Providers configured: {}\n",
        providers_configured.join(", ")
    ));
    report.push_str(&format!(
        "Providers stale: {}\n\n",
        providers_stale.join(", ")
    ));

    // Docker version
    report.push_str("--- Docker Version ---\n");
    match silent_cmd("docker").arg("--version").output() {
        Ok(o) => report.push_str(&String::from_utf8_lossy(&o.stdout)),
        Err(e) => report.push_str(&format!("Error: {}\n", e)),
    }
    report.push('\n');

    // Docker info
    report.push_str("--- Docker Info ---\n");
    match silent_cmd("docker").arg("info").output() {
        Ok(o) => {
            report.push_str(&String::from_utf8_lossy(&o.stdout));
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stderr.is_empty() {
                report.push_str(&format!("stderr: {}\n", stderr));
            }
        }
        Err(e) => report.push_str(&format!("Error: {}\n", e)),
    }
    report.push('\n');

    // Docker compose ps & logs (only if install path is set)
    if let Some(ref path) = install_path {
        report.push_str("--- Docker Compose PS ---\n");
        match silent_cmd("docker")
            .args(["compose", "ps"])
            .current_dir(path)
            .output()
        {
            Ok(o) => {
                report.push_str(&String::from_utf8_lossy(&o.stdout));
                let stderr = String::from_utf8_lossy(&o.stderr);
                if !stderr.is_empty() {
                    report.push_str(&format!("stderr: {}\n", stderr));
                }
            }
            Err(e) => report.push_str(&format!("Error: {}\n", e)),
        }
        report.push('\n');

        report.push_str("--- Docker Compose Logs (last 200 lines) ---\n");
        match silent_cmd("docker")
            .args(["compose", "logs", "--tail", "200"])
            .current_dir(path)
            .output()
        {
            Ok(o) => {
                report.push_str(&String::from_utf8_lossy(&o.stdout));
                let stderr = String::from_utf8_lossy(&o.stderr);
                if !stderr.is_empty() {
                    report.push_str(&stderr);
                }
            }
            Err(e) => report.push_str(&format!("Error: {}\n", e)),
        }
        report.push('\n');

        // Read safe env lines from postiz.env
        report.push_str("--- postiz.env (non-secret lines) ---\n");
        let env_path = path.join("postiz.env");
        if env_path.exists() {
            match std::fs::read_to_string(&env_path) {
                Ok(contents) => {
                    let secret_keywords = ["SECRET", "PASSWORD", "KEY", "TOKEN"];
                    let safe_prefixes = [
                        "MAIN_URL",
                        "FRONTEND_URL",
                        "NEXT_PUBLIC_BACKEND_URL",
                        "PORT",
                        "STORAGE_PROVIDER",
                        "IS_GENERAL",
                        "NEXT_PUBLIC",
                        "UPLOAD_DIRECTORY",
                        "BACKEND_INTERNAL_URL",
                    ];

                    for line in contents.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with('#') {
                            continue;
                        }

                        let upper = trimmed.to_uppercase();
                        let has_secret = secret_keywords.iter().any(|kw| upper.contains(kw));

                        // Check if the value contains embedded credentials (e.g. DATABASE_URL=postgresql://user:pass@host)
                        let has_embedded_creds = if let Some(eq_pos) = trimmed.find('=') {
                            let value = &trimmed[eq_pos + 1..];
                            // Matches patterns like ://user:password@ in connection strings
                            value.contains("://") && value.contains('@') && {
                                if let Some(proto_end) = value.find("://") {
                                    let after_proto = &value[proto_end + 3..];
                                    after_proto.contains(':')
                                        && after_proto.contains('@')
                                        && after_proto.find(':') < after_proto.find('@')
                                } else {
                                    false
                                }
                            }
                        } else {
                            false
                        };

                        if has_secret || has_embedded_creds {
                            // Only include if it's a known safe prefix
                            let is_safe = !has_embedded_creds
                                && safe_prefixes.iter().any(|prefix| upper.starts_with(prefix));
                            if is_safe {
                                report.push_str(trimmed);
                                report.push('\n');
                            } else {
                                // Redact: show key name only
                                if let Some(eq_pos) = trimmed.find('=') {
                                    report.push_str(&trimmed[..eq_pos]);
                                    report.push_str("=[REDACTED]\n");
                                }
                            }
                        } else {
                            report.push_str(trimmed);
                            report.push('\n');
                        }
                    }
                }
                Err(e) => report.push_str(&format!("Error reading env file: {}\n", e)),
            }
        } else {
            report.push_str("postiz.env not found.\n");
        }
        report.push('\n');
    }

    // Save to file
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let output_path = if let Some(ref path) = install_path {
        path.join(format!("postiz-diagnostics-{}.txt", timestamp))
    } else {
        let temp_dir = std::env::temp_dir();
        temp_dir.join(format!("postiz-diagnostics-{}.txt", timestamp))
    };

    std::fs::write(&output_path, &report)
        .map_err(|e| format!("Failed to write diagnostics file: {}", e))?;

    Ok(output_path.to_string_lossy().to_string())
}
