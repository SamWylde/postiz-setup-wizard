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
  tunnel_provider: string;
  permanent_domain: string | null;
  web_link_kind: "none" | "manual" | "cloudflare" | "legacy_shared" | "local_https";
  web_link_supported: boolean;
  web_link_reason: string | null;
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
  | "InstallCloudflared"
  | "InstallCaddy";

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

export const wipeExistingInstall = (path: string) =>
  invoke<string>("wipe_existing_install", { path });

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

export const cancelDockerOperation = () =>
  invoke<string>("cancel_docker_operation");

// Tunnel commands
export type TunnelProvider = "cloudflared" | "manual";

const VALID_TUNNEL_PROVIDERS: readonly TunnelProvider[] = ["cloudflared", "manual"];

/** Parse a string into a valid TunnelProvider, defaulting to "cloudflared" for unknown values. */
export function parseTunnelProvider(s: string | undefined | null): TunnelProvider {
  if (s && (VALID_TUNNEL_PROVIDERS as readonly string[]).includes(s)) {
    return s as TunnelProvider;
  }
  return "cloudflared";
}

export const startTunnel = (port: number, provider?: TunnelProvider, config?: string) =>
  invoke<string>("start_tunnel", { port, provider, config });

export const stopTunnel = () => invoke<string>("stop_tunnel");

export const getTunnelStatus = () =>
  invoke<{ status: string; url: string | null }>("get_tunnel_status");

export const reconnectTunnel = (port: number, installPath: string, provider?: TunnelProvider, config?: string) =>
  invoke<string>("reconnect_tunnel", { port, installPath, provider, config });

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

// Web link commands
export interface PublicUrlCheck {
  reachable: boolean;
  status_code: number | null;
  error: string | null;
}

export const applyManualDomain = (path: string, publicUrl: string, force?: boolean) =>
  invoke<string>("apply_manual_domain", { path, publicUrl, force: force ?? false });

export const configureManagedCaddy = (
  path: string, publicUrl: string, port: number,
) => invoke<string>("configure_managed_caddy", { path, publicUrl, port });

export const disableManagedCaddy = (path: string) =>
  invoke<string>("disable_managed_caddy", { path });

export const connectCloudflareZeroTrust = (
  path: string, port: number, publicUrl: string, token: string,
) => invoke<string>("connect_cloudflare_zero_trust", { path, port, publicUrl, token });

export interface CloudflareR2Config {
  accountId: string;
  accessKey: string;
  secretAccessKey: string;
  bucketName: string;
  bucketUrl: string;
  region?: string;
}

export const applyLocalHttpsDomain = (
  path: string,
  port: number,
  publicUrl: string,
  r2?: CloudflareR2Config | null,
) => invoke<string>("apply_local_https_domain", { path, port, publicUrl, r2: r2 ?? null });

export const switchToLocalOnly = (path: string, port: number) =>
  invoke<string>("switch_to_local_only", { path, port });

export const verifyPublicUrl = (publicUrl: string) =>
  invoke<PublicUrlCheck>("verify_public_url", { publicUrl });

export const readEnvValue = (path: string, key: string) =>
  invoke<string | null>("read_env_value", { path, key });

export const getSavedCredentials = (provider: string) =>
  invoke<Record<string, string>>("get_saved_credentials", { provider });

export const savePostizCredentials = (email: string, password: string) =>
  invoke<void>("save_postiz_credentials", { email, password });

export const getPostizCredentials = () =>
  invoke<[string | null, string | null]>("get_postiz_credentials");

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
    transfer_review_pending: boolean;
    tunnel_provider: string;
    tunnel_config: string | null;
    postiz_email: string | null;
    postiz_password: string | null;
  } | null>("load_resume_state", { installPath });

export const saveResumeState = () => invoke<string>("save_resume_state");

export const updateStep = (step: number) =>
  invoke<void>("update_step", { step });

export const syncTunnelConfig = (
  tunnelMode: string,
  permanentDomain: string | null,
  tunnelProvider?: string,
  tunnelConfig?: string | null,
) => invoke<void>("sync_tunnel_config", { tunnelMode, permanentDomain, tunnelProvider, tunnelConfig });

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

// Transfer commands
export interface CloneManifest {
  format_version: number;
  created_at: string;
  source_hostname: string;
  source_port: number;
  wizard_version: string;
  tunnel_mode: string;
  tunnel_provider: string;
  providers_configured: string[];
  volumes: { archive_name: string; mount_path: string; service: string }[];
}

export interface TransferProgress {
  phase: string;
  message: string;
  current: number | null;
  total: number | null;
}

export const exportClone = (path: string, password: string, outputPath: string) =>
  invoke<string>("export_clone", { path, password, outputPath });

export const validateCloneFile = (clonePath: string, password: string) =>
  invoke<CloneManifest>("validate_clone_file", { clonePath, password });

export const importClone = (
  clonePath: string,
  password: string,
  installPath: string,
  customPort?: number,
) => invoke<string>("import_clone", { clonePath, password, installPath, customPort });

export const clearTransferReviewAndSave = () =>
  invoke<void>("clear_transfer_review_and_save");

export const onTransferProgress = (
  callback: (event: { payload: TransferProgress }) => void,
): Promise<UnlistenFn> => listen("transfer-progress", callback);

// Upgrade commands
export interface PostizUpdateInfo {
  update_available: boolean;
  local_digest: string;
  remote_digest: string;
  image: string;
}

export interface UpgradeProgress {
  phase: string;
  message: string;
}

export const checkPostizUpdate = () =>
  invoke<PostizUpdateInfo>("check_postiz_update");

export const upgradePostiz = () =>
  invoke<string>("upgrade_postiz");

export const onUpgradeProgress = (
  callback: (event: { payload: UpgradeProgress }) => void,
): Promise<UnlistenFn> => listen("upgrade-progress", callback);

// Pull progress
export interface PullProgress {
  total_layers: number;
  completed_layers: number;
  message: string;
  completed_services: string[];
}

// Event listeners
export const onDockerProgress = (
  callback: (event: { payload: string }) => void,
): Promise<UnlistenFn> => listen("docker-progress", callback);

export const onDockerLog = (
  callback: (event: { payload: string }) => void,
): Promise<UnlistenFn> => listen("docker-log", callback);

export const onDockerPullProgress = (
  callback: (event: { payload: PullProgress }) => void,
): Promise<UnlistenFn> => listen("docker-pull-progress", callback);

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
