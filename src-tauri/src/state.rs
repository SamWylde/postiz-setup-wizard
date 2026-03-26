use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TunnelProvider {
    #[default]
    Cloudflared,
    Manual,
}

impl TunnelProvider {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "manual" => Self::Manual,
            // Legacy values ("ngrok", "zrok", "pinggy") fall through to default
            _ => Self::Cloudflared,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cloudflared => "cloudflared",
            Self::Manual => "manual",
        }
    }
}

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
    #[serde(default)]
    pub transfer_review_pending: bool,
    #[serde(default)]
    pub tunnel_provider: String,
    #[serde(default)]
    pub tunnel_config: Option<String>,
    #[serde(default)]
    pub provider_credentials: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub postiz_email: Option<String>,
    #[serde(default)]
    pub postiz_password: Option<String>,
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
            transfer_review_pending: false,
            tunnel_provider: String::new(),
            tunnel_config: None,
            provider_credentials: HashMap::new(),
            postiz_email: None,
            postiz_password: None,
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
    pub tunnel_provider: String,
    pub permanent_domain: Option<String>,

    // Web link (derived)
    pub web_link_kind: String, // "none" | "manual" | "cloudflare" | "legacy_shared"
    pub web_link_supported: bool,
    pub web_link_reason: Option<String>,

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

#[derive(Debug, Serialize, Clone)]
pub struct PublicUrlCheck {
    pub reachable: bool,
    pub status_code: Option<u16>,
    pub error: Option<String>,
}

pub struct AppState {
    pub install_path: Option<PathBuf>,
    pub port: u16,
    pub local_url: Option<String>,
    pub tunnel_url: Option<String>,
    pub tunnel_mode: String, // "temporary", "permanent", "none"
    pub tunnel_provider: TunnelProvider,
    pub permanent_domain: Option<String>,
    pub current_step: usize,
    pub pending_env_changes: HashMap<String, String>,
    pub stale_providers: HashSet<String>,
    pub providers_configured: HashSet<String>,
    #[allow(dead_code)] // Retained for future use (e.g. exposing logs via command)
    pub docker_logs: Vec<String>,
    pub docker_child_pid: Option<u32>,
    pub docker_op_cancelled: bool,
    pub tunnel_config: Option<String>,
    pub provider_credentials: HashMap<String, HashMap<String, String>>,
    pub postiz_email: Option<String>,
    pub postiz_password: Option<String>,
    pub tunnel_pid: Option<u32>,
    pub last_error: Option<String>,
    pub has_shown_tray_notification: bool,
    pub transfer_review_pending: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            install_path: None,
            port: 4007,
            local_url: None,
            tunnel_url: None,
            tunnel_mode: "none".to_string(),
            tunnel_provider: TunnelProvider::default(),
            permanent_domain: None,
            current_step: 0,
            pending_env_changes: HashMap::new(),
            stale_providers: HashSet::new(),
            providers_configured: HashSet::new(),
            docker_logs: Vec::new(),
            tunnel_config: None,
            provider_credentials: HashMap::new(),
            postiz_email: None,
            postiz_password: None,
            docker_child_pid: None,
            docker_op_cancelled: false,
            tunnel_pid: None,
            last_error: None,
            has_shown_tray_notification: false,
            transfer_review_pending: false,
        }
    }
}

pub type SharedState = Mutex<AppState>;
