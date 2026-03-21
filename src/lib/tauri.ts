import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { MachineState, StackStatus, ContainerInfo } from "../store/wizardStore";

export interface InstallSnapshot {
  install_path: string | null;
  install_exists: boolean;
  has_staged_temp: boolean;
  port: number;
  docker_installed: boolean;
  docker_running: boolean;
  containers: ContainerInfo[];
  all_healthy: boolean;
  postiz_responding: boolean;
  tunnel_alive: boolean;
  tunnel_url: string | null;
  tunnel_mode: string;
  permanent_domain: string | null;
  providers_configured: string[];
  providers_stale: string[];
  current_step: number;
  last_error: string | null;
  recovery_available: boolean;
}

export interface PreflightResult {
  ok: boolean;
  checks: { name: string; passed: boolean; message: string }[];
}

// Bootstrap commands
export const scanMachine = () => invoke<MachineState>("scan_machine");

export type BootstrapAction =
  | "InstallWsl2"
  | "InstallDocker"
  | "StartDocker"
  | "SwitchLinuxContainers"
  | "InstallCloudflared";

export const runBootstrap = (action: BootstrapAction) =>
  invoke<string>("run_bootstrap", { action });

// Install commands
export const getDefaultInstallPath = () =>
  invoke<string>("get_default_install_path");

export const prepareInstall = (path: string, customPort?: number) =>
  invoke<number>("prepare_install", { path, customPort });

export const commitInstall = (path: string) =>
  invoke<string>("commit_install", { path });

export const cleanStagedFiles = (path: string) =>
  invoke<string>("clean_staged_files", { path });

// Docker commands
export const startStack = (path: string) =>
  invoke<string>("start_stack", { path });

export const getStackStatus = (path: string) =>
  invoke<StackStatus>("get_stack_status", { path });

export const stopStack = (path: string) =>
  invoke<string>("stop_stack", { path });

export const repairStack = (path: string) =>
  invoke<string>("repair_stack", { path });

export const getDockerLogs = (path: string) =>
  invoke<string[]>("get_docker_logs", { path });

export const restartAndVerify = (path: string) =>
  invoke<string>("restart_and_verify", { path });

// Tunnel commands
export const startTunnel = (port: number) =>
  invoke<string>("start_tunnel", { port });

export const stopTunnel = () => invoke<string>("stop_tunnel");

export const getTunnelStatus = () =>
  invoke<{ status: string; url: string | null }>("get_tunnel_status");

export const reconnectTunnel = (port: number, installPath: string) =>
  invoke<string>("reconnect_tunnel", { port, installPath });

// Env file commands
export const stageProviderConfig = (
  provider: string,
  entries: Record<string, string>,
) => invoke<string>("stage_provider_config", { provider, entries });

export const applyProviderChanges = (path: string) =>
  invoke<string>("apply_provider_changes", { path });

export const applyConfigTransaction = (path: string) =>
  invoke<string>("apply_config_transaction", { path });

export const updateBaseUrls = (path: string, baseUrl: string) =>
  invoke<string>("update_base_urls", { path, baseUrl });

export const readEnvValue = (path: string, key: string) =>
  invoke<string | null>("read_env_value", { path, key });

// Snapshot & diagnostics
export const getInstallSnapshot = () =>
  invoke<InstallSnapshot>("get_install_snapshot");

export const validatePreflight = (
  path: string,
  port: number,
  tunnelMode: string,
  allowExisting?: boolean,
) => invoke<PreflightResult>("validate_preflight", { path, port, tunnelMode, allowExisting });

export const exportDiagnostics = () =>
  invoke<string>("export_diagnostics");

export const importExistingInstall = (path: string) =>
  invoke<string>("import_existing_install", { path });

// Resume commands
export const loadResumeState = (installPath?: string) =>
  invoke<{
    version: number;
    current_step: number;
    install_path: string | null;
    port: number;
    tunnel_url: string | null;
    tunnel_mode: string;
    permanent_domain: string | null;
    providers_configured: string[];
    providers_stale: string[];
    reboot_pending_for: string | null;
  } | null>("load_resume_state", { installPath });

export const saveResumeState = () => invoke<string>("save_resume_state");

export const updateStep = (step: number) =>
  invoke<void>("update_step", { step });

export const syncTunnelConfig = (
  tunnelMode: string,
  permanentDomain: string | null,
) => invoke<void>("sync_tunnel_config", { tunnelMode, permanentDomain });

export const syncProviderStatus = (
  configured: string[],
  stale: string[],
) => invoke<void>("sync_provider_status", { configured, stale });

// Cancel
export const cancelInstall = () => invoke<string>("cancel_install");

// Secrets
export const generateSecrets = () =>
  invoke<[string, string]>("generate_secrets");

// Updater commands
export interface UpdateInfo {
  available: boolean;
  current_version: string;
  latest_version: string;
  body?: string;
}

export const checkForUpdate = () =>
  invoke<UpdateInfo>("check_for_update");

export const installUpdate = () =>
  invoke<void>("install_update");

export const getCurrentAppVersion = () =>
  invoke<string>("get_current_app_version");

// Event listeners
export const onDockerProgress = (
  callback: (event: { payload: string }) => void,
): Promise<UnlistenFn> => listen("docker-progress", callback);

export const onDockerLog = (
  callback: (event: { payload: string }) => void,
): Promise<UnlistenFn> => listen("docker-log", callback);

export const onTunnelUrl = (
  callback: (event: { payload: string }) => void,
): Promise<UnlistenFn> => listen("tunnel-url", callback);

export const onTunnelStatus = (
  callback: (event: { payload: string }) => void,
): Promise<UnlistenFn> => listen("tunnel-status", callback);

export const onUpdateStatus = (
  callback: (event: { payload: string }) => void,
): Promise<UnlistenFn> => listen("update-status", callback);

export const onUpdateProgress = (
  callback: (event: { payload: { percent: number; downloaded: number; total: number } }) => void,
): Promise<UnlistenFn> => listen("update-progress", callback);
