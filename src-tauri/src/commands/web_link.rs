use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tauri::{Emitter, State};

use super::docker::run_docker_compose;
use super::env_file::{
    parse_env_file, write_cloudflare_r2_storage, write_local_base_urls, write_public_base_urls,
    write_wizard_web_link_mode,
};
use super::silent_cmd;
use super::tunnel::{is_pid_alive, kill_existing_tunnel, start_cloudflared};
use crate::commands::bootstrap::resolve_binary;
use crate::state::{PublicUrlCheck, SharedState, TunnelProvider};

const MANAGED_CADDY_ADMIN_ADDR: &str = "127.0.0.1:2027";
const MANAGED_CADDY_SERVICE_NAME: &str = "PostizWizardCaddy";
const MANAGED_CADDY_SERVICE_DESCRIPTION: &str =
    "Managed Caddy reverse proxy for Postiz Setup Wizard.";
const MANAGED_CADDY_HTTP_RULE_NAME: &str = "Postiz Wizard Caddy HTTP";
const MANAGED_CADDY_HTTPS_RULE_NAME: &str = "Postiz Wizard Caddy HTTPS";
const LOCAL_HTTPS_CADDY_ADMIN_ADDR: &str = "127.0.0.1:2028";
const LOCAL_HTTPS_CADDY_SERVICE_NAME: &str = "PostizWizardLocalHttpsCaddy";
const LOCAL_HTTPS_CADDY_SERVICE_DESCRIPTION: &str =
    "Managed local HTTPS Caddy reverse proxy for Postiz Setup Wizard.";
const LOCAL_HTTPS_HOSTS_COMMENT: &str = "# Postiz Setup Wizard Local HTTPS";

#[derive(Clone)]
struct PreviousLinkState {
    tunnel_mode: String,
    tunnel_provider: TunnelProvider,
    tunnel_config: Option<String>,
    permanent_domain: Option<String>,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudflareR2Config {
    account_id: String,
    access_key: String,
    secret_access_key: String,
    bucket_name: String,
    bucket_url: String,
    #[serde(default)]
    region: String,
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

fn validate_public_base_url(public_url: &str) -> Result<(String, String), String> {
    let normalized = public_url.trim().trim_end_matches('/').to_string();
    let parsed = reqwest::Url::parse(&normalized).map_err(|_| {
        "Invalid URL format. Enter a full URL like https://postiz.example.com".to_string()
    })?;

    if parsed.scheme() != "https" {
        return Err("Domain must start with https://".to_string());
    }

    if parsed.host_str().is_none() {
        return Err("The public URL must include a hostname.".to_string());
    }

    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(
            "Use just the site URL, like https://postiz.example.com, with no path, query, or fragment."
                .to_string(),
        );
    }

    Ok((
        normalized,
        parsed.host_str().unwrap_or_default().to_string(),
    ))
}

fn validate_local_https_base_url(public_url: &str) -> Result<(String, String), String> {
    let (normalized, host) = validate_public_base_url(public_url)?;

    if host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" {
        return Err(
            "Use a real hostname like https://postiz.yourdomain.com, not localhost.".to_string(),
        );
    }

    if !host.contains('.') {
        return Err(
            "Use a full hostname with a dot, like https://postiz.yourdomain.com.".to_string(),
        );
    }

    Ok((normalized, host))
}

fn managed_caddy_dir(install_path: &Path) -> PathBuf {
    install_path.join("proxy").join("caddy")
}

fn managed_caddy_config_path(install_path: &Path) -> PathBuf {
    managed_caddy_dir(install_path).join("Caddyfile")
}

fn local_https_caddy_dir(install_path: &Path) -> PathBuf {
    install_path.join("proxy").join("caddy-local-https")
}

fn local_https_caddy_config_path(install_path: &Path) -> PathBuf {
    local_https_caddy_dir(install_path).join("Caddyfile")
}

fn build_managed_caddyfile(host: &str, port: u16) -> String {
    format!(
        "{{\n    admin {}\n}}\n\n{} {{\n    reverse_proxy 127.0.0.1:{}\n}}\n",
        MANAGED_CADDY_ADMIN_ADDR, host, port
    )
}

fn build_local_https_caddyfile(host: &str, port: u16) -> String {
    format!(
        "{{\n    admin {}\n}}\n\nhttps://{} {{\n    tls internal\n    reverse_proxy 127.0.0.1:{}\n}}\n",
        LOCAL_HTTPS_CADDY_ADMIN_ADDR, host, port
    )
}

fn build_managed_caddy_service_bin_path(caddy_binary: &Path, config_path: &Path) -> String {
    format!(
        "\"{}\" run --config \"{}\" --adapter caddyfile",
        caddy_binary.display(),
        config_path.display()
    )
}

fn write_managed_caddyfile(install_path: &Path, host: &str, port: u16) -> Result<PathBuf, String> {
    let dir = managed_caddy_dir(install_path);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create managed Caddy directory: {}", e))?;

    let config_path = managed_caddy_config_path(install_path);
    fs::write(&config_path, build_managed_caddyfile(host, port))
        .map_err(|e| format!("Failed to write managed Caddyfile: {}", e))?;

    Ok(config_path)
}

fn write_local_https_caddyfile(
    install_path: &Path,
    host: &str,
    port: u16,
) -> Result<PathBuf, String> {
    let dir = local_https_caddy_dir(install_path);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create local HTTPS Caddy directory: {}", e))?;

    let config_path = local_https_caddy_config_path(install_path);
    fs::write(&config_path, build_local_https_caddyfile(host, port))
        .map_err(|e| format!("Failed to write local HTTPS Caddyfile: {}", e))?;

    Ok(config_path)
}

fn run_caddy_output(args: &[&str]) -> Result<std::process::Output, String> {
    let binary = resolve_binary("caddy");
    silent_cmd(&binary)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run Caddy: {}", e))
}

fn command_error(context: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "No additional details were returned.".to_string()
    };
    format!("{} {}", context, detail)
}

fn output_text_lower(output: &std::process::Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase()
}

fn caddy_admin_unreachable(output: &std::process::Output) -> bool {
    let text = output_text_lower(output);
    text.contains("connection refused")
        || text.contains("actively refused")
        || text.contains("timeout")
        || text.contains("no such host")
}

fn sc_service_not_found(output: &std::process::Output) -> bool {
    let text = output_text_lower(output);
    text.contains("1060") || text.contains("does not exist")
}

fn sc_service_already_running(output: &std::process::Output) -> bool {
    let text = output_text_lower(output);
    text.contains("1056") || text.contains("already running")
}

fn sc_service_not_started(output: &std::process::Output) -> bool {
    let text = output_text_lower(output);
    text.contains("1062") || text.contains("has not been started")
}

fn locate_caddy_binary() -> Result<PathBuf, String> {
    let resolved = PathBuf::from(resolve_binary("caddy"));
    if resolved.exists() {
        return Ok(fs::canonicalize(&resolved).unwrap_or(resolved));
    }

    let output = silent_cmd("where")
        .args(["caddy"])
        .output()
        .map_err(|e| format!("Failed to locate Caddy: {}", e))?;
    if !output.status.success() {
        return Err("Caddy executable not found. Install Caddy first.".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let path = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| "Caddy executable not found. Install Caddy first.".to_string())?;

    let path = PathBuf::from(path);
    Ok(fs::canonicalize(&path).unwrap_or(path))
}

fn run_sc_output(args: &[&str]) -> Result<std::process::Output, String> {
    silent_cmd("sc.exe")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to manage Windows service: {}", e))
}

fn caddy_service_exists(service_name: &str) -> Result<bool, String> {
    let output = run_sc_output(&["query", service_name])?;
    if output.status.success() {
        Ok(true)
    } else if sc_service_not_found(&output) {
        Ok(false)
    } else {
        Err(command_error(
            "Managed Caddy service status could not be checked.",
            &output,
        ))
    }
}

fn caddy_service_running(service_name: &str) -> Result<bool, String> {
    let output = run_sc_output(&["query", service_name])?;
    if output.status.success() {
        Ok(output_text_lower(&output).contains("running"))
    } else if sc_service_not_found(&output) {
        Ok(false)
    } else {
        Err(command_error(
            "Managed Caddy service status could not be checked.",
            &output,
        ))
    }
}

fn wait_for_caddy_service_state(
    service_name: &str,
    should_be_running: bool,
) -> Result<bool, String> {
    for _ in 0..15 {
        if caddy_service_running(service_name)? == should_be_running {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(false)
}

fn ensure_caddy_service(
    service_name: &str,
    service_description: &str,
    display_name: &str,
    config_path: &Path,
) -> Result<(), String> {
    let caddy_binary = locate_caddy_binary()?;
    let bin_path = build_managed_caddy_service_bin_path(&caddy_binary, config_path);

    if caddy_service_exists(service_name)? {
        let output = run_sc_output(&[
            "config",
            service_name,
            "start=",
            "auto",
            "binPath=",
            &bin_path,
        ])?;
        if !output.status.success() {
            return Err(command_error(
                "Managed Caddy service could not be updated.",
                &output,
            ));
        }
    } else {
        let output = run_sc_output(&[
            "create",
            service_name,
            "start=",
            "auto",
            "binPath=",
            &bin_path,
            "displayname=",
            display_name,
        ])?;
        if !output.status.success() {
            return Err(command_error(
                "Managed Caddy service could not be created.",
                &output,
            ));
        }
    }

    let description = run_sc_output(&["description", service_name, service_description])?;
    if !description.status.success() {
        return Err(command_error(
            "Managed Caddy service description could not be updated.",
            &description,
        ));
    }

    let failure_flag = run_sc_output(&["failureflag", service_name, "1"])?;
    if !failure_flag.status.success() {
        return Err(command_error(
            "Managed Caddy service recovery settings could not be enabled.",
            &failure_flag,
        ));
    }

    let failure = run_sc_output(&[
        "failure",
        service_name,
        "reset=",
        "86400",
        "actions=",
        "restart/60000/restart/60000/restart/60000",
    ])?;
    if !failure.status.success() {
        return Err(command_error(
            "Managed Caddy service restart policy could not be updated.",
            &failure,
        ));
    }

    Ok(())
}

fn managed_caddy_service_exists() -> Result<bool, String> {
    caddy_service_exists(MANAGED_CADDY_SERVICE_NAME)
}

fn managed_caddy_service_running() -> Result<bool, String> {
    caddy_service_running(MANAGED_CADDY_SERVICE_NAME)
}

fn ensure_managed_caddy_service(config_path: &Path) -> Result<(), String> {
    ensure_caddy_service(
        MANAGED_CADDY_SERVICE_NAME,
        MANAGED_CADDY_SERVICE_DESCRIPTION,
        "Postiz Wizard Managed Caddy",
        config_path,
    )
}

fn stop_caddy_admin_process() -> Result<(), String> {
    let output = run_caddy_output(&["stop", "--address", MANAGED_CADDY_ADMIN_ADDR])?;
    if output.status.success() || caddy_admin_unreachable(&output) {
        Ok(())
    } else {
        Err(command_error(
            "Managed Caddy could not be stopped automatically.",
            &output,
        ))
    }
}

fn start_managed_caddy_service() -> Result<(), String> {
    start_caddy_service(MANAGED_CADDY_SERVICE_NAME)
}

fn start_caddy_service(service_name: &str) -> Result<(), String> {
    let output = run_sc_output(&["start", service_name])?;
    if !output.status.success() && !sc_service_already_running(&output) {
        return Err(command_error(
            "Managed Caddy service could not be started.",
            &output,
        ));
    }

    if wait_for_caddy_service_state(service_name, true)? {
        Ok(())
    } else {
        Err("Managed Caddy service did not reach the running state.".to_string())
    }
}

fn stop_managed_caddy_service() -> Result<(), String> {
    stop_caddy_service(MANAGED_CADDY_SERVICE_NAME)
}

fn stop_caddy_service(service_name: &str) -> Result<(), String> {
    if !caddy_service_exists(service_name)? {
        return Ok(());
    }

    let output = run_sc_output(&["stop", service_name])?;
    if !output.status.success() && !sc_service_not_started(&output) {
        return Err(command_error(
            "Managed Caddy service could not be stopped.",
            &output,
        ));
    }

    if wait_for_caddy_service_state(service_name, false)? {
        Ok(())
    } else {
        Err("Managed Caddy service did not stop cleanly.".to_string())
    }
}

fn delete_managed_caddy_service() -> Result<(), String> {
    delete_caddy_service(MANAGED_CADDY_SERVICE_NAME)
}

fn delete_caddy_service(service_name: &str) -> Result<(), String> {
    if !caddy_service_exists(service_name)? {
        return Ok(());
    }

    let output = run_sc_output(&["delete", service_name])?;
    if output.status.success() || sc_service_not_found(&output) {
        Ok(())
    } else {
        Err(command_error(
            "Managed Caddy service could not be removed.",
            &output,
        ))
    }
}

fn ensure_firewall_rule(rule_name: &str, port: u16) -> Result<(), String> {
    let localport = format!("localport={}", port);
    let name_arg = format!("name={}", rule_name);
    let output = silent_cmd("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &name_arg,
            "dir=in",
            "action=allow",
            "protocol=TCP",
            &localport,
        ])
        .output()
        .map_err(|e| format!("Failed to update Windows Firewall: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(
            "Windows Firewall could not be updated automatically.",
            &output,
        ))
    }
}

fn best_effort_open_http_https_firewall() -> Option<String> {
    let mut failures = Vec::new();
    if let Err(err) = ensure_firewall_rule(MANAGED_CADDY_HTTP_RULE_NAME, 80) {
        failures.push(err);
    }
    if let Err(err) = ensure_firewall_rule(MANAGED_CADDY_HTTPS_RULE_NAME, 443) {
        failures.push(err);
    }

    if failures.is_empty() {
        None
    } else {
        Some(
            "Windows Firewall could not be fully updated automatically. You may need to allow inbound TCP ports 80 and 443 yourself."
                .to_string(),
        )
    }
}

fn remove_firewall_rule(rule_name: &str) -> Result<(), String> {
    let name_arg = format!("name={}", rule_name);
    let output = silent_cmd("netsh")
        .args(["advfirewall", "firewall", "delete", "rule", &name_arg])
        .output()
        .map_err(|e| format!("Failed to update Windows Firewall: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let text = output_text_lower(&output);
        if text.contains("no rules match") {
            Ok(())
        } else {
            Err(command_error(
                "Windows Firewall rules could not be removed automatically.",
                &output,
            ))
        }
    }
}

fn stop_managed_caddy(install_path: &Path) -> Result<(), String> {
    let mut failures = Vec::new();
    let had_managed_config = managed_caddy_config_path(install_path).exists();
    let had_managed_service = managed_caddy_service_exists()?;

    if had_managed_service {
        if let Err(err) = stop_managed_caddy_service() {
            failures.push(err);
        }
        if let Err(err) = delete_managed_caddy_service() {
            failures.push(err);
        }
    }

    if install_path.exists() {
        let _ = fs::remove_file(managed_caddy_config_path(install_path));
        let _ = fs::remove_dir(managed_caddy_dir(install_path));
    }

    if had_managed_service || had_managed_config {
        if let Err(err) = stop_caddy_admin_process() {
            failures.push(err);
        }
    }

    let _ = remove_firewall_rule(MANAGED_CADDY_HTTP_RULE_NAME);
    let _ = remove_firewall_rule(MANAGED_CADDY_HTTPS_RULE_NAME);

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(" "))
    }
}

fn local_https_service_exists() -> Result<bool, String> {
    caddy_service_exists(LOCAL_HTTPS_CADDY_SERVICE_NAME)
}

fn local_https_service_running() -> Result<bool, String> {
    caddy_service_running(LOCAL_HTTPS_CADDY_SERVICE_NAME)
}

fn ensure_local_https_service(config_path: &Path) -> Result<(), String> {
    ensure_caddy_service(
        LOCAL_HTTPS_CADDY_SERVICE_NAME,
        LOCAL_HTTPS_CADDY_SERVICE_DESCRIPTION,
        "Postiz Wizard Local HTTPS Caddy",
        config_path,
    )
}

fn stop_local_https_service() -> Result<(), String> {
    stop_caddy_service(LOCAL_HTTPS_CADDY_SERVICE_NAME)
}

fn delete_local_https_service() -> Result<(), String> {
    delete_caddy_service(LOCAL_HTTPS_CADDY_SERVICE_NAME)
}

fn stop_local_https_admin_process() -> Result<(), String> {
    let output = run_caddy_output(&["stop", "--address", LOCAL_HTTPS_CADDY_ADMIN_ADDR])?;
    if output.status.success() || caddy_admin_unreachable(&output) {
        Ok(())
    } else {
        Err(command_error(
            "Managed local HTTPS Caddy could not be stopped automatically.",
            &output,
        ))
    }
}

fn trust_local_https_caddy() -> Option<String> {
    match run_caddy_output(&["trust", "--address", LOCAL_HTTPS_CADDY_ADMIN_ADDR]) {
        Ok(output) if output.status.success() => None,
        Ok(output) => Some(command_error(
            "The local HTTPS certificate could not be trusted automatically.",
            &output,
        )),
        Err(err) => Some(err),
    }
}

fn hosts_file_path() -> PathBuf {
    PathBuf::from(r"C:\Windows\System32\drivers\etc\hosts")
}

fn normalize_hosts_tokens(line: &str) -> Vec<String> {
    let trimmed = line.split('#').next().unwrap_or("").trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split_whitespace().map(|s| s.to_string()).collect()
}

fn add_hosts_mapping(host: &str) -> Result<(), String> {
    let hosts_path = hosts_file_path();
    let mut contents =
        fs::read_to_string(&hosts_path).map_err(|e| format!("Failed to read hosts file: {}", e))?;

    let has_mapping = contents.lines().any(|line| {
        let tokens = normalize_hosts_tokens(line);
        tokens.len() >= 2
            && tokens[0] == "127.0.0.1"
            && tokens[1..].iter().any(|token| token == host)
    });

    if has_mapping {
        return Ok(());
    }

    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(&format!(
        "127.0.0.1 {}\t{}\n",
        host, LOCAL_HTTPS_HOSTS_COMMENT
    ));

    let tmp = hosts_path.with_extension("tmp");
    fs::write(&tmp, contents).map_err(|e| format!("Failed to update hosts file: {}", e))?;
    fs::rename(&tmp, &hosts_path).map_err(|e| format!("Failed to replace hosts file: {}", e))?;
    Ok(())
}

fn remove_hosts_mapping(host: &str) -> Result<(), String> {
    let hosts_path = hosts_file_path();
    let contents =
        fs::read_to_string(&hosts_path).map_err(|e| format!("Failed to read hosts file: {}", e))?;

    let filtered = contents
        .lines()
        .filter(|line| {
            let tokens = normalize_hosts_tokens(line);
            !(tokens.len() >= 2
                && tokens[0] == "127.0.0.1"
                && tokens[1..].iter().any(|token| token == host))
        })
        .collect::<Vec<_>>()
        .join("\n");

    let final_contents = if filtered.is_empty() {
        String::new()
    } else {
        format!("{}\n", filtered)
    };

    let tmp = hosts_path.with_extension("tmp");
    fs::write(&tmp, final_contents).map_err(|e| format!("Failed to update hosts file: {}", e))?;
    fs::rename(&tmp, &hosts_path).map_err(|e| format!("Failed to replace hosts file: {}", e))?;
    Ok(())
}

fn configure_local_https_inner(
    install_path: &Path,
    public_url: &str,
    port: u16,
) -> Result<String, String> {
    let (_, host) = validate_local_https_base_url(public_url)?;
    let config_path = write_local_https_caddyfile(install_path, &host, port)?;
    let config_arg = config_path.to_string_lossy().to_string();

    let validate = run_caddy_output(&["adapt", "--config", &config_arg, "--adapter", "caddyfile"])?;
    if !validate.status.success() {
        return Err(command_error(
            "Managed local HTTPS Caddy configuration is invalid.",
            &validate,
        ));
    }

    ensure_local_https_service(&config_path)?;

    if local_https_service_running()? {
        let reload = run_caddy_output(&[
            "reload",
            "--config",
            &config_arg,
            "--adapter",
            "caddyfile",
            "--address",
            LOCAL_HTTPS_CADDY_ADMIN_ADDR,
        ])?;
        if !reload.status.success() {
            let _ = stop_local_https_service();
            start_caddy_service(LOCAL_HTTPS_CADDY_SERVICE_NAME)?;
        }
    } else {
        let _ = stop_local_https_admin_process();
        start_caddy_service(LOCAL_HTTPS_CADDY_SERVICE_NAME)?;
    }

    add_hosts_mapping(&host)?;

    let trust_note = trust_local_https_caddy();
    let mut message = format!(
        "Local HTTPS is configured for {} and forwarding traffic to Postiz on localhost:{}.",
        host, port
    );
    if let Some(note) = trust_note {
        message.push(' ');
        message.push_str("Windows did not trust the local certificate automatically. ");
        message.push_str(&note);
    }
    Ok(message)
}

fn stop_local_https(install_path: &Path, public_url: Option<&str>) -> Result<(), String> {
    let mut failures = Vec::new();
    let had_config = local_https_caddy_config_path(install_path).exists();
    let had_service = local_https_service_exists()?;

    if let Some(url) = public_url {
        if let Ok((_, host)) = validate_local_https_base_url(url) {
            if let Err(err) = remove_hosts_mapping(&host) {
                failures.push(err);
            }
        }
    }

    if had_service {
        if let Err(err) = stop_local_https_service() {
            failures.push(err);
        }
        if let Err(err) = delete_local_https_service() {
            failures.push(err);
        }
    }

    if install_path.exists() {
        let _ = fs::remove_file(local_https_caddy_config_path(install_path));
        let _ = fs::remove_dir_all(local_https_caddy_dir(install_path));
    }

    if had_service || had_config {
        if let Err(err) = stop_local_https_admin_process() {
            failures.push(err);
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(" "))
    }
}

fn configure_managed_caddy_inner(
    install_path: &Path,
    public_url: &str,
    port: u16,
) -> Result<String, String> {
    let (_, host) = validate_public_base_url(public_url)?;
    let config_path = write_managed_caddyfile(install_path, &host, port)?;
    let config_arg = config_path.to_string_lossy().to_string();

    let validate = run_caddy_output(&["adapt", "--config", &config_arg, "--adapter", "caddyfile"])?;
    if !validate.status.success() {
        return Err(command_error(
            "Managed Caddy configuration is invalid.",
            &validate,
        ));
    }

    ensure_managed_caddy_service(&config_path)?;

    let firewall_note = best_effort_open_http_https_firewall();

    if managed_caddy_service_running()? {
        let reload = run_caddy_output(&[
            "reload",
            "--config",
            &config_arg,
            "--adapter",
            "caddyfile",
            "--address",
            MANAGED_CADDY_ADMIN_ADDR,
        ])?;
        if !reload.status.success() {
            if caddy_admin_unreachable(&reload) {
                stop_managed_caddy_service()?;
                start_managed_caddy_service()?;
            } else {
                stop_managed_caddy_service()?;
                start_managed_caddy_service()?;
            }
        }
    } else {
        let _ = stop_caddy_admin_process();
        start_managed_caddy_service()?;
    }

    let mut message = format!(
        "Caddy is configured as a Windows service for {} and forwarding traffic to Postiz on localhost:{}.",
        host, port
    );
    if let Some(note) = firewall_note {
        message.push(' ');
        message.push_str(&note);
    }
    Ok(message)
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

fn validate_cloudflare_r2_config(config: &CloudflareR2Config) -> Result<(), String> {
    if config.account_id.trim().is_empty()
        || config.access_key.trim().is_empty()
        || config.secret_access_key.trim().is_empty()
        || config.bucket_name.trim().is_empty()
        || config.bucket_url.trim().is_empty()
    {
        return Err("Fill in every Cloudflare R2 field or leave R2 disabled for now.".to_string());
    }

    let bucket_url = config.bucket_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(bucket_url)
        .map_err(|_| "Cloudflare R2 public bucket URL is invalid.".to_string())?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err("Cloudflare R2 public bucket URL must be a full HTTPS URL.".to_string());
    }

    Ok(())
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
    install_path: &Path,
    app: &tauri::AppHandle,
    state: &SharedState,
    port: u16,
) -> Result<(), String> {
    match previous.tunnel_mode.as_str() {
        "temporary" => {
            let url = start_cloudflared(port, app, state).await?;
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.tunnel_mode = "temporary".to_string();
            s.tunnel_provider = TunnelProvider::Cloudflared;
            s.tunnel_url = Some(url);
            s.permanent_domain = None;
            Ok(())
        }
        "permanent" if previous.tunnel_provider == TunnelProvider::Cloudflared => {
            let token = previous
                .tunnel_config
                .clone()
                .ok_or_else(|| "Previous Cloudflare tunnel token was unavailable.".to_string())?;
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
        "local_https" => {
            let url = previous
                .permanent_domain
                .clone()
                .ok_or_else(|| "Previous local HTTPS domain was unavailable.".to_string())?;
            configure_local_https_inner(install_path, &url, port)?;
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.tunnel_mode = "local_https".to_string();
            s.tunnel_provider = TunnelProvider::Manual;
            s.permanent_domain = Some(url);
            s.tunnel_url = None;
            s.tunnel_config = None;
            Ok(())
        }
        _ => Ok(()),
    }
}

// ── apply_manual_domain ──────────────────────────────────────────────

#[tauri::command]
pub async fn configure_managed_caddy(
    path: String,
    public_url: String,
    port: u16,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let install_path = PathBuf::from(&path);
    let (normalized_url, _) = validate_public_base_url(&public_url)?;

    let _ = app.emit("docker-progress", "Writing managed Caddy configuration...");
    let message = configure_managed_caddy_inner(&install_path, &normalized_url, port)?;
    let _ = app.emit("docker-progress", &message);

    Ok(message)
}

#[tauri::command]
pub async fn disable_managed_caddy(path: String) -> Result<String, String> {
    let install_path = PathBuf::from(&path);
    stop_managed_caddy(&install_path)?;
    Ok("App-managed Caddy has been disabled.".to_string())
}

#[tauri::command]
pub async fn apply_manual_domain(
    path: String,
    public_url: String,
    force: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let (public_url, _) = validate_public_base_url(&public_url)?;

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

    if previous.tunnel_mode == "local_https" {
        if let Some(previous_url) = previous.permanent_domain.as_deref() {
            let _ = remove_hosts_mapping(&validate_local_https_base_url(previous_url)?.1);
        }
    }

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
        |path| {
            write_public_base_urls(path, &public_url)?;
            write_wizard_web_link_mode(path, "manual")
        },
    )
    .await;

    if let Err(err) = result {
        return match restore_previous_link(&previous, &install_path, &app, &state, port).await {
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

    if previous.tunnel_mode == "local_https" {
        let _ = stop_local_https(&install_path, previous.permanent_domain.as_deref());
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
        |path| {
            write_local_base_urls(path, port)?;
            write_wizard_web_link_mode(path, "none")
        },
    )
    .await;

    if let Err(err) = result {
        return match restore_previous_link(&previous, &install_path, &app, &state, port).await {
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

    if let Err(err) = stop_managed_caddy(&install_path) {
        let _ = app.emit(
            "docker-progress",
            format!("Warning: managed Caddy may still be running: {}", err),
        );
    }
    if previous.tunnel_mode == "local_https" {
        if let Err(err) = stop_local_https(&install_path, previous.permanent_domain.as_deref()) {
            let _ = app.emit(
                "docker-progress",
                format!("Warning: local HTTPS helper may still be running: {}", err),
            );
        }
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
    let (public_url, _) = validate_public_base_url(&public_url)?;
    let token = token.trim().to_string();

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

    if previous.tunnel_mode == "local_https" {
        if let Some(previous_url) = previous.permanent_domain.as_deref() {
            let _ = remove_hosts_mapping(&validate_local_https_base_url(previous_url)?.1);
        }
    }

    let _ = app.emit("docker-progress", "Stopping any active tunnel...");
    kill_existing_tunnel(&state);

    let _ = app.emit("docker-progress", "Starting Cloudflare tunnel...");
    let pid = match start_cloudflare_zero_trust_process(&token, &state).await {
        Ok(pid) => pid,
        Err(err) => {
            return match restore_previous_link(&previous, &install_path, &app, &state, local_port)
                .await
            {
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
        |path| {
            write_public_base_urls(path, &public_url)?;
            write_wizard_web_link_mode(path, "cloudflare")
        },
    )
    .await;

    if let Err(err) = result {
        kill_existing_tunnel(&state);
        return match restore_previous_link(&previous, &install_path, &app, &state, local_port).await
        {
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
        let restore_result =
            restore_previous_link(&previous, &install_path, &app, &state, local_port).await;

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
        let restore_result =
            restore_previous_link(&previous, &install_path, &app, &state, local_port).await;

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

    if let Err(err) = stop_managed_caddy(&install_path) {
        let _ = app.emit(
            "docker-progress",
            format!("Warning: managed Caddy may still be running: {}", err),
        );
    }
    if previous.tunnel_mode == "local_https" {
        let _ = stop_local_https(&install_path, previous.permanent_domain.as_deref());
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

#[tauri::command]
pub async fn apply_local_https_domain(
    path: String,
    port: u16,
    public_url: String,
    r2: Option<CloudflareR2Config>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let (public_url, host) = validate_local_https_base_url(&public_url)?;
    if let Some(ref config) = r2 {
        validate_cloudflare_r2_config(config)?;
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

    let _ = app.emit(
        "docker-progress",
        "Configuring local HTTPS hostname and hosts file...",
    );
    if let Err(err) = configure_local_https_inner(&install_path, &public_url, local_port) {
        let _ = stop_local_https(&install_path, Some(&public_url));
        return match restore_previous_link(&previous, &install_path, &app, &state, local_port).await
        {
            Ok(()) => Err(err),
            Err(restore_err) => Err(format!(
                "{} Previous web link could not be restored automatically: {}",
                err, restore_err
            )),
        };
    }

    let result = apply_link_change(
        &install_path,
        &env_path,
        &original_env,
        local_port,
        &app,
        &state,
        |path| {
            write_public_base_urls(path, &public_url)?;
            write_wizard_web_link_mode(path, "local_https")?;
            if let Some(ref config) = r2 {
                write_cloudflare_r2_storage(
                    path,
                    &config.account_id,
                    &config.access_key,
                    &config.secret_access_key,
                    &config.bucket_name,
                    &config.bucket_url,
                    &config.region,
                )?;
            }
            Ok(())
        },
    )
    .await;

    if let Err(err) = result {
        let _ = stop_local_https(&install_path, Some(&public_url));
        return match restore_previous_link(&previous, &install_path, &app, &state, local_port).await
        {
            Ok(()) => Err(err),
            Err(restore_err) => Err(format!(
                "{} Previous web link could not be restored automatically: {}",
                err, restore_err
            )),
        };
    }

    if previous.tunnel_mode == "local_https"
        && previous.permanent_domain.as_deref() != Some(public_url.as_str())
    {
        let _ = stop_local_https(&install_path, previous.permanent_domain.as_deref());
        configure_local_https_inner(&install_path, &public_url, local_port)?;
    }

    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.tunnel_mode = "local_https".to_string();
        s.tunnel_provider = TunnelProvider::Manual;
        s.permanent_domain = Some(public_url.clone());
        s.tunnel_url = None;
        s.tunnel_config = None;
    }

    let mut message = format!(
        "Local HTTPS domain applied: {}. This URL works on this computer and points to Postiz through managed Caddy.",
        public_url
    );
    if r2.is_some() {
        message.push_str(" Cloudflare R2 storage credentials were also saved.");
    }
    let _ = app.emit(
        "docker-progress",
        format!("Local HTTPS hostname {} is ready.", host),
    );
    Ok(message)
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        build_managed_caddy_service_bin_path, build_managed_caddyfile, validate_public_base_url,
        MANAGED_CADDY_ADMIN_ADDR,
    };

    #[test]
    fn validate_public_base_url_accepts_clean_https_url() {
        let (normalized, host) =
            validate_public_base_url("https://postiz.example.com/").expect("valid URL");
        assert_eq!(normalized, "https://postiz.example.com");
        assert_eq!(host, "postiz.example.com");
    }

    #[test]
    fn validate_public_base_url_rejects_paths() {
        let err = validate_public_base_url("https://postiz.example.com/login")
            .expect_err("path should be rejected");
        assert!(err.contains("no path"));
    }

    #[test]
    fn managed_caddyfile_uses_dedicated_admin_port_and_localhost_proxy() {
        let config = build_managed_caddyfile("postiz.example.com", 4007);
        assert!(config.contains(MANAGED_CADDY_ADMIN_ADDR));
        assert!(config.contains("postiz.example.com"));
        assert!(config.contains("reverse_proxy 127.0.0.1:4007"));
    }

    #[test]
    fn managed_caddy_service_bin_path_quotes_binary_and_config() {
        let command = build_managed_caddy_service_bin_path(
            Path::new(r"C:\Program Files\Caddy\caddy.exe"),
            Path::new(r"C:\Postiz\proxy\caddy\Caddyfile"),
        );
        assert_eq!(
            command,
            "\"C:\\Program Files\\Caddy\\caddy.exe\" run --config \"C:\\Postiz\\proxy\\caddy\\Caddyfile\" --adapter caddyfile"
        );
    }
}
