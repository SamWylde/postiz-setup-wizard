use serde::Serialize;
use std::process::Command;
use sysinfo::{Disks, System};
use tauri::State;

use crate::state::SharedState;

#[derive(Debug, Serialize, Clone)]
pub struct MachineState {
    pub windows_version_ok: bool,
    pub wsl2_installed: bool,
    pub docker_installed: bool,
    pub docker_running: bool,
    pub docker_linux_mode: bool,
    pub cloudflared_installed: bool,
    pub disk_space_gb: f64,
    pub ram_available_gb: f64,
    pub existing_install: Option<String>,
    pub reboot_required: bool,
}

fn check_command(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).output().is_ok()
}

fn get_command_output(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).to_string())
            } else {
                None
            }
        })
}

#[tauri::command]
pub fn scan_machine(state: State<SharedState>) -> Result<MachineState, String> {
    // Check Windows version (we're Windows-only, so this should always be true in practice)
    let windows_version_ok = true;

    // Check WSL2
    let wsl2_installed = get_command_output("wsl", &["--status"]).is_some();

    // Check Docker
    let docker_installed = check_command("docker", &["--version"]);

    let docker_running = get_command_output("docker", &["info"]).is_some();

    // Check if Docker is in Linux container mode
    let docker_linux_mode = if docker_running {
        get_command_output("docker", &["info"])
            .map(|info| info.contains("linux") || info.contains("Linux"))
            .unwrap_or(false)
    } else {
        false
    };

    // Check cloudflared
    let cloudflared_installed = check_command("cloudflared", &["--version"]);

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
            .map(|p| p.parent().unwrap().to_string_lossy().to_string());

        if default_state.is_some() {
            default_state
        } else {
            // Check pointer file for custom install path
            let pointer_path = dirs::data_local_dir()
                .map(|d| d.join("Postiz").join("install-pointer.json"));

            pointer_path.and_then(|p| {
                std::fs::read_to_string(&p).ok().and_then(|contents| {
                    serde_json::from_str::<serde_json::Value>(&contents).ok().and_then(|val| {
                        val["install_path"].as_str().map(|s| s.to_string())
                    }).filter(|path| {
                        std::path::Path::new(path).join("install-state.json").exists()
                    })
                })
            })
        }
    };

    // Check if reboot is required (WSL install pending)
    let reboot_required = false; // TODO: detect pending reboot

    // Update state with any existing install info
    if let Ok(mut app_state) = state.lock() {
        if let Some(ref path) = existing_install {
            app_state.install_path = Some(std::path::PathBuf::from(path));
        }
    }

    Ok(MachineState {
        windows_version_ok,
        wsl2_installed,
        docker_installed,
        docker_running,
        docker_linux_mode,
        cloudflared_installed,
        disk_space_gb,
        ram_available_gb,
        existing_install,
        reboot_required,
    })
}

#[derive(Debug, serde::Deserialize)]
pub enum BootstrapAction {
    InstallWsl2,
    InstallDocker,
    StartDocker,
    SwitchLinuxContainers,
    InstallCloudflared,
}

#[tauri::command]
pub async fn run_bootstrap(action: BootstrapAction) -> Result<String, String> {
    match action {
        BootstrapAction::InstallWsl2 => {
            let output = Command::new("wsl")
                .args(["--install", "--no-distribution"])
                .output()
                .map_err(|e| format!("Failed to run wsl --install: {}", e))?;

            if output.status.success() {
                Ok("WSL2 installation started. A reboot may be required.".to_string())
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
            let output = Command::new("powershell")
                .args([
                    "-Command",
                    &format!(
                        "Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
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

            Ok("Docker Desktop installer launched. Please follow the on-screen prompts.".to_string())
        }
        BootstrapAction::StartDocker => {
            // Try to find and start Docker Desktop
            let docker_path =
                r"C:\Program Files\Docker\Docker\Docker Desktop.exe";

            Command::new(docker_path)
                .spawn()
                .map_err(|e| format!("Failed to start Docker Desktop: {}", e))?;

            Ok("Docker Desktop is starting...".to_string())
        }
        BootstrapAction::SwitchLinuxContainers => {
            let output = Command::new(
                r"C:\Program Files\Docker\Docker\DockerCli.exe",
            )
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
            let output = Command::new("winget")
                .args(["install", "Cloudflare.cloudflared", "--accept-source-agreements", "--accept-package-agreements"])
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
    }
}
