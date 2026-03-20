use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::process::Child;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResumeState {
    pub version: u32,
    pub current_step: usize,
    pub install_path: Option<String>,
    pub port: u16,
    pub tunnel_url: Option<String>,
    pub tunnel_mode: String, // "temporary", "permanent", "none"
    pub permanent_domain: Option<String>,
    pub providers_configured: Vec<String>,
    pub providers_stale: Vec<String>,
    pub reboot_pending_for: Option<String>,
    pub last_updated: String,
}

impl Default for ResumeState {
    fn default() -> Self {
        Self {
            version: 1,
            current_step: 0,
            install_path: None,
            port: 4007,
            tunnel_url: None,
            tunnel_mode: "none".to_string(),
            permanent_domain: None,
            providers_configured: Vec::new(),
            providers_stale: Vec::new(),
            reboot_pending_for: None,
            last_updated: String::new(),
        }
    }
}

/// Comprehensive snapshot of the current install state — the single source of truth.
#[derive(Debug, Serialize, Clone)]
pub struct InstallSnapshot {
    // Install
    pub install_path: Option<String>,
    pub install_exists: bool,
    pub has_staged_temp: bool,
    pub port: u16,

    // Docker
    pub docker_installed: bool,
    pub docker_running: bool,
    pub containers: Vec<crate::commands::docker::ContainerInfo>,
    pub all_healthy: bool,
    pub postiz_responding: bool,

    // Tunnel
    pub tunnel_alive: bool,
    pub tunnel_url: Option<String>,
    pub tunnel_mode: String,
    pub permanent_domain: Option<String>,

    // Providers
    pub providers_configured: Vec<String>,
    pub providers_stale: Vec<String>,

    // State
    pub current_step: usize,
    pub last_error: Option<String>,
    pub recovery_available: bool,
}

/// Result of preflight validation before install.
#[derive(Debug, Serialize, Clone)]
pub struct PreflightResult {
    pub ok: bool,
    pub checks: Vec<PreflightCheck>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PreflightCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

#[allow(dead_code)]
pub struct AppState {
    pub install_path: Option<PathBuf>,
    pub port: u16,
    pub local_url: Option<String>,
    pub tunnel_url: Option<String>,
    pub tunnel_process: Option<Child>,
    pub tunnel_mode: String, // "temporary", "permanent", "none"
    pub permanent_domain: Option<String>,
    pub current_step: usize,
    pub pending_env_changes: HashMap<String, String>,
    pub stale_providers: HashSet<String>,
    pub providers_configured: HashSet<String>,
    pub reboot_pending: Option<String>,
    pub docker_logs: Vec<String>,
    pub docker_child_pid: Option<u32>,
    pub tunnel_pid: Option<u32>,
    pub last_error: Option<String>,
    pub has_shown_tray_notification: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            install_path: None,
            port: 4007,
            local_url: None,
            tunnel_url: None,
            tunnel_process: None,
            tunnel_mode: "none".to_string(),
            permanent_domain: None,
            current_step: 0,
            pending_env_changes: HashMap::new(),
            stale_providers: HashSet::new(),
            providers_configured: HashSet::new(),
            reboot_pending: None,
            docker_logs: Vec::new(),
            docker_child_pid: None,
            tunnel_pid: None,
            last_error: None,
            has_shown_tray_notification: false,
        }
    }
}

pub type SharedState = Mutex<AppState>;
