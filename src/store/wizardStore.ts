import { create } from "zustand";
import { updateStep, saveResumeState, syncProviderStatus, syncTunnelConfig } from "../lib/tauri";

/** Fire-and-forget persistence — log errors instead of silently swallowing them. */
const persist = (p: Promise<unknown>) =>
  p.catch((err) => console.error("[wizardStore] persist failed:", err));

export interface MachineState {
  windows_version_ok: boolean;
  wsl2_installed: boolean;
  docker_installed: boolean;
  docker_running: boolean;
  docker_linux_mode: boolean;
  cloudflared_installed: boolean;
  ngrok_installed: boolean;
  zrok_installed: boolean;
  ssh_available: boolean;
  disk_space_gb: number;
  ram_available_gb: number;
  existing_install: string | null;
}

export interface ContainerInfo {
  name: string;
  state: string;
  status: string;
  health: string;
}

export interface StackStatus {
  containers: ContainerInfo[];
  all_healthy: boolean;
  postiz_responding: boolean;
}

export type ProviderStatus = "unconfigured" | "configured" | "stale";

export interface WizardState {
  currentStep: number;

  // Step 0: Prepare Computer
  machineState: MachineState | null;
  bootstrapStatus:
    | "idle"
    | "checking"
    | "ready"
    | "action-needed";

  // Step 1: Install Postiz
  installPath: string;
  port: number;
  installStatus:
    | "idle"
    | "preparing"
    | "pulling"
    | "starting"
    | "running"
    | "error";
  installError: string | null;

  // Step 2: Create Account
  postizReady: boolean;
  accountCreated: boolean;

  // Step 3: Create Web Link
  tunnelStatus: "idle" | "starting" | "running" | "restarting" | "error";
  tunnelUrl: string | null;
  tunnelMode: "temporary" | "permanent" | "none";
  tunnelProvider: "cloudflared" | "ngrok" | "zrok" | "pinggy";
  tunnelConfig: string;
  permanentDomain: string;
  remoteReachable: boolean;

  // Step 4: Connect Providers
  providers: Record<string, ProviderStatus>;
  activeProvider: string | null;

  // Step 5: Verify & Finish
  verified: boolean;

  // Transfer
  transferReviewPending: boolean;

  // Docker logs (for technical details panel)
  dockerLogs: string[];

  // Actions
  setStep: (step: number) => void;
  setMachineState: (state: MachineState | null) => void;
  setBootstrapStatus: (status: WizardState["bootstrapStatus"]) => void;
  setInstallPath: (path: string) => void;
  setPort: (port: number) => void;
  setInstallStatus: (status: WizardState["installStatus"]) => void;
  setInstallError: (error: string | null) => void;
  setPostizReady: (ready: boolean) => void;
  setAccountCreated: (created: boolean) => void;
  setTunnelStatus: (status: WizardState["tunnelStatus"]) => void;
  setTunnelUrl: (url: string | null) => void;
  setTunnelMode: (mode: WizardState["tunnelMode"]) => void;
  setTunnelProvider: (provider: WizardState["tunnelProvider"]) => void;
  setTunnelConfig: (config: string) => void;
  setPermanentDomain: (domain: string) => void;
  setRemoteReachable: (reachable: boolean) => void;
  setProviderStatus: (provider: string, status: ProviderStatus) => void;
  setActiveProvider: (provider: string | null) => void;
  setVerified: (verified: boolean) => void;
  setTransferReviewPending: (pending: boolean) => void;
  addDockerLog: (log: string) => void;
  clearDockerLogs: () => void;

  /** Silent hydration — sets fields in one call with NO side effects
   *  (no syncing to Rust, no saving resume state). */
  hydrateFromResume: (data: {
    currentStep?: number;
    installPath?: string;
    port?: number;
    tunnelMode?: WizardState["tunnelMode"];
    permanentDomain?: string;
    tunnelProvider?: WizardState["tunnelProvider"];
    tunnelConfig?: string;
    providers?: Record<string, ProviderStatus>;
    transferReviewPending?: boolean;
    tunnelUrl?: string | null;
    tunnelStatus?: WizardState["tunnelStatus"];
  }) => void;
}

export const useWizardStore = create<WizardState>((set) => ({
  currentStep: 0,

  machineState: null,
  bootstrapStatus: "idle",

  installPath: "",
  port: 4007,
  installStatus: "idle",
  installError: null,

  postizReady: false,
  accountCreated: false,

  tunnelStatus: "idle",
  tunnelUrl: null,
  tunnelMode: "temporary",
  tunnelProvider: "cloudflared",
  tunnelConfig: "",
  permanentDomain: "",
  remoteReachable: false,

  providers: {},
  activeProvider: null,

  verified: false,

  transferReviewPending: false,

  dockerLogs: [],

  setStep: (step) => {
    set({ currentStep: step });
    // Persist step to Rust backend and save resume state
    persist(updateStep(step));
    persist(saveResumeState());
  },
  setMachineState: (machineState) => set({ machineState }),
  setBootstrapStatus: (bootstrapStatus) => set({ bootstrapStatus }),
  setInstallPath: (installPath) => set({ installPath }),
  setPort: (port) => set({ port }),
  setInstallStatus: (installStatus) => set({ installStatus }),
  setInstallError: (installError) => set({ installError }),
  setPostizReady: (postizReady) => set({ postizReady }),
  setAccountCreated: (accountCreated) => set({ accountCreated }),
  setTunnelStatus: (tunnelStatus) => set({ tunnelStatus }),
  setTunnelUrl: (tunnelUrl) => set({ tunnelUrl }),
  setTunnelMode: (tunnelMode) => {
    set((state) => {
      persist(syncTunnelConfig(tunnelMode, state.permanentDomain || null, state.tunnelProvider, state.tunnelConfig || null)
        .then(() => saveResumeState()));
      return { tunnelMode };
    });
  },
  setTunnelProvider: (tunnelProvider) => {
    set((state) => {
      persist(syncTunnelConfig(state.tunnelMode, state.permanentDomain || null, tunnelProvider, state.tunnelConfig || null)
        .then(() => saveResumeState()));
      return { tunnelProvider };
    });
  },
  setTunnelConfig: (tunnelConfig) => {
    set((state) => {
      persist(syncTunnelConfig(state.tunnelMode, state.permanentDomain || null, state.tunnelProvider, tunnelConfig || null)
        .then(() => saveResumeState()));
      return { tunnelConfig };
    });
  },
  setPermanentDomain: (permanentDomain) => {
    set((state) => {
      persist(syncTunnelConfig(state.tunnelMode, permanentDomain || null, state.tunnelProvider, state.tunnelConfig || null)
        .then(() => saveResumeState()));
      return { permanentDomain };
    });
  },
  setRemoteReachable: (remoteReachable) => set({ remoteReachable }),
  setProviderStatus: (provider, status) => {
    set((state) => {
      const newProviders = { ...state.providers, [provider]: status };
      // Sync to Rust state then persist
      const configured = Object.entries(newProviders)
        .filter(([, s]) => s === "configured")
        .map(([id]) => id);
      const stale = Object.entries(newProviders)
        .filter(([, s]) => s === "stale")
        .map(([id]) => id);
      persist(syncProviderStatus(configured, stale)
        .then(() => saveResumeState()));
      return { providers: newProviders };
    });
  },
  setActiveProvider: (activeProvider) => set({ activeProvider }),
  setVerified: (verified) => set({ verified }),
  setTransferReviewPending: (transferReviewPending) => set({ transferReviewPending }),
  addDockerLog: (log) =>
    set((state) => ({
      dockerLogs: [...state.dockerLogs.slice(-499), log],
    })),
  clearDockerLogs: () => set({ dockerLogs: [] }),

  hydrateFromResume: (data) => {
    const patch: Partial<WizardState> = {};
    if (data.currentStep !== undefined) patch.currentStep = data.currentStep;
    if (data.installPath !== undefined) patch.installPath = data.installPath;
    if (data.port !== undefined) patch.port = data.port;
    if (data.tunnelMode !== undefined) patch.tunnelMode = data.tunnelMode;
    if (data.permanentDomain !== undefined) patch.permanentDomain = data.permanentDomain;
    if (data.tunnelProvider !== undefined) patch.tunnelProvider = data.tunnelProvider;
    if (data.tunnelConfig !== undefined) patch.tunnelConfig = data.tunnelConfig;
    if (data.providers !== undefined) patch.providers = data.providers;
    if (data.transferReviewPending !== undefined) patch.transferReviewPending = data.transferReviewPending;
    if (data.tunnelUrl !== undefined) patch.tunnelUrl = data.tunnelUrl;
    if (data.tunnelStatus !== undefined) patch.tunnelStatus = data.tunnelStatus;
    set(patch);
  },
}));
