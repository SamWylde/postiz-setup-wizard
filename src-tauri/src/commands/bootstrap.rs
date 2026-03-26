use serde::Serialize;
use std::process::Command;
use sysinfo::{Disks, System};
use tauri::State;

use super::silent_cmd;
use crate::state::SharedState;

#[derive(Debug, Serialize, Clone)]
pub struct MachineState {
    pub windows_version_ok: bool,
    pub wsl2_installed: bool,
    pub docker_installed: bool,
    pub docker_running: bool,
    pub docker_linux_mode: bool,
    pub cloudflared_installed: bool,
    pub caddy_installed: bool,
    pub ssh_available: bool,
    pub disk_space_gb: f64,
    pub ram_available_gb: f64,
    pub existing_install: Option<String>,
}

fn check_command(cmd: &str, args: &[&str]) -> bool {
    silent_cmd(cmd)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn check_resolved_binary(name: &str, args: &[&str]) -> bool {
    let binary = resolve_binary(name);
    silent_cmd(&binary)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Resolve a binary by checking PostizWizard local install first, then system PATH.
pub fn resolve_binary(name: &str) -> String {
    let local = dirs::data_local_dir()
        .unwrap_or_default()
        .join("PostizWizard")
        .join(name)
        .join(format!("{}.exe", name));
    if local.exists() {
        return local.to_string_lossy().to_string();
    }

    let winget_link = dirs::data_local_dir()
        .unwrap_or_default()
        .join("Microsoft")
        .join("WinGet")
        .join("Links")
        .join(format!("{}.exe", name));
    if winget_link.exists() {
        return winget_link.to_string_lossy().to_string();
    }

    name.to_string()
}

fn get_command_output(cmd: &str, args: &[&str]) -> Option<String> {
    silent_cmd(cmd).args(args).output().ok().and_then(|o| {
        if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).to_string())
        } else {
            None
        }
    })
}

#[tauri::command]
pub async fn scan_machine(state: State<'_, SharedState>) -> Result<MachineState, String> {
    // Run all blocking shell commands off the main thread so the UI stays responsive
    let (machine, existing_install) = tokio::task::spawn_blocking(move || scan_machine_blocking())
        .await
        .map_err(|e| format!("System scan panicked: {}", e))?;

    // Update state with any existing install info (needs State which isn't Send)
    {
        let mut app_state = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref path) = existing_install {
            app_state.install_path = Some(std::path::PathBuf::from(path));
        }
    }

    Ok(machine)
}

fn scan_machine_blocking() -> (MachineState, Option<String>) {
    // Check Windows version (we're Windows-only, so this should always be true in practice)
    let windows_version_ok = true;

    // Check WSL2
    let wsl2_installed = get_command_output("wsl", &["--status"]).is_some();

    // Check Docker
    let docker_installed = check_command("docker", &["--version"]);

    // Single docker info call — used for both "running" and "linux mode" checks
    let docker_info = get_command_output("docker", &["info"]);
    let docker_running = docker_info.is_some();

    // Check if Docker is in Linux container mode
    let docker_linux_mode = docker_info
        .map(|info| info.contains("linux") || info.contains("Linux"))
        .unwrap_or(false);

    // Check tunnel providers
    let cloudflared_installed = check_resolved_binary("cloudflared", &["--version"]);
    let caddy_installed = check_resolved_binary("caddy", &["version"]);
    let ssh_available = check_command("ssh", &["-V"]);

    // Check disk space
    let disks = Disks::new_with_refreshed_list();
    let disk_space_gb = disks
        .iter()
        .find(|d| {
            d.mount_point()
                .to_str()
                .map(|s| s.starts_with("C:"))
                .unwrap_or(false)
        })
        .map(|d| d.available_space() as f64 / 1_073_741_824.0)
        .unwrap_or(0.0);

    // Check available RAM
    let mut sys = System::new();
    sys.refresh_memory();
    let ram_available_gb = sys.available_memory() as f64 / 1_073_741_824.0;

    // Check for existing Postiz install — check default location and pointer file
    let existing_install = {
        let default_state = dirs::data_local_dir()
            .map(|d| d.join("Postiz").join("install-state.json"))
            .filter(|p| p.exists())
            .and_then(|p| p.parent().map(|par| par.to_string_lossy().to_string()));

        if default_state.is_some() {
            default_state
        } else {
            // Check pointer file for custom install path
            let pointer_path =
                dirs::data_local_dir().map(|d| d.join("Postiz").join("install-pointer.json"));

            pointer_path.and_then(|p| {
                std::fs::read_to_string(&p).ok().and_then(|contents| {
                    serde_json::from_str::<serde_json::Value>(&contents)
                        .ok()
                        .and_then(|val| val["install_path"].as_str().map(|s| s.to_string()))
                        .filter(|path| {
                            std::path::Path::new(path)
                                .join("install-state.json")
                                .exists()
                        })
                })
            })
        }
    };

    let machine = MachineState {
        windows_version_ok,
        wsl2_installed,
        docker_installed,
        docker_running,
        docker_linux_mode,
        cloudflared_installed,
        caddy_installed,
        ssh_available,
        disk_space_gb,
        ram_available_gb,
        existing_install: existing_install.clone(),
    };

    (machine, existing_install)
}

#[derive(Debug, serde::Deserialize)]
pub enum BootstrapAction {
    InstallWsl2,
    InstallDocker,
    StartDocker,
    SwitchLinuxContainers,
    InstallCloudflared,
    InstallCaddy,
}

#[tauri::command]
pub async fn run_bootstrap(action: BootstrapAction) -> Result<String, String> {
    match action {
        BootstrapAction::InstallWsl2 => {
            let output = silent_cmd("wsl")
                .args(["--install", "--no-distribution"])
                .output()
                .map_err(|e| format!("Failed to run wsl --install: {}", e))?;

            if output.status.success() {
                Ok("WSL2 installed successfully.".to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("WSL2 installation failed: {}", stderr))
            }
        }
        BootstrapAction::InstallDocker => {
            // Download Docker Desktop installer
            let download_url =
                "https://desktop.docker.com/win/main/amd64/Docker%20Desktop%20Installer.exe";
            let temp_dir = std::env::temp_dir();
            let installer_path = temp_dir.join("DockerDesktopInstaller.exe");

            // Download using PowerShell
            let output = silent_cmd("powershell")
                .args([
                    "-Command",
                    &format!(
                        "$ProgressPreference = 'SilentlyContinue'; Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
                        download_url,
                        installer_path.display()
                    ),
                ])
                .output()
                .map_err(|e| format!("Failed to download Docker Desktop: {}", e))?;

            if !output.status.success() {
                return Err(format!(
                    "Download failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }

            // Launch installer (requires UAC)
            Command::new(&installer_path)
                .args(["install", "--accept-license"])
                .spawn()
                .map_err(|e| format!("Failed to launch Docker installer: {}", e))?;

            Ok(
                "Docker Desktop installer launched. Please follow the on-screen prompts."
                    .to_string(),
            )
        }
        BootstrapAction::StartDocker => {
            // Try to find Docker Desktop — check Program Files, then PATH
            let candidates = [
                r"C:\Program Files\Docker\Docker\Docker Desktop.exe",
                r"C:\Program Files (x86)\Docker\Docker\Docker Desktop.exe",
            ];
            let docker_path = candidates
                .iter()
                .find(|p| std::path::Path::new(p).exists())
                .ok_or_else(|| {
                    "Docker Desktop not found. Please install it from https://docker.com/products/docker-desktop".to_string()
                })?;

            Command::new(docker_path)
                .spawn()
                .map_err(|e| format!("Failed to start Docker Desktop: {}", e))?;

            Ok("Docker Desktop is starting...".to_string())
        }
        BootstrapAction::SwitchLinuxContainers => {
            let candidates = [
                r"C:\Program Files\Docker\Docker\DockerCli.exe",
                r"C:\Program Files (x86)\Docker\Docker\DockerCli.exe",
            ];
            let cli_path = candidates
                .iter()
                .find(|p| std::path::Path::new(p).exists())
                .ok_or_else(|| {
                    "DockerCli.exe not found. Please reinstall Docker Desktop.".to_string()
                })?;
            let output = silent_cmd(cli_path)
                .args(["-SwitchLinuxEngine"])
                .output()
                .map_err(|e| format!("Failed to switch to Linux containers: {}", e))?;

            if output.status.success() {
                Ok("Switched to Linux containers.".to_string())
            } else {
                Err(format!(
                    "Failed to switch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ))
            }
        }
        BootstrapAction::InstallCloudflared => {
            let output = silent_cmd("winget")
                .args([
                    "install",
                    "Cloudflare.cloudflared",
                    "--accept-source-agreements",
                    "--accept-package-agreements",
                ])
                .output()
                .map_err(|e| format!("Failed to install cloudflared: {}", e))?;

            if output.status.success() {
                Ok("cloudflared installed successfully.".to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                Err(format!("Installation failed: {} {}", stdout, stderr))
            }
        }
        BootstrapAction::InstallCaddy => {
            let output = silent_cmd("winget")
                .args([
                    "install",
                    "CaddyServer.Caddy",
                    "--accept-source-agreements",
                    "--accept-package-agreements",
                ])
                .output()
                .map_err(|e| format!("Failed to install Caddy: {}", e))?;

            if output.status.success() {
                Ok("Caddy installed successfully.".to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                Err(format!("Caddy installation failed: {} {}", stdout, stderr))
            }
        }
    }
}
