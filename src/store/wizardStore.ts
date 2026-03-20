import { create } from "zustand";
import { updateStep, saveResumeState, syncProviderStatus, syncTunnelConfig } from "../lib/tauri";

export interface MachineState {
  windows_version_ok: boolean;
  wsl2_installed: boolean;
  docker_installed: boolean;
  docker_running: boolean;
  docker_linux_mode: boolean;
  cloudflared_installed: boolean;
  disk_space_gb: number;
  ram_available_gb: number;
  existing_install: string | null;
  reboot_required: boolean;
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
    | "action-needed"
    | "rebooting";

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
  permanentDomain: string;
  remoteReachable: boolean;

  // Step 4: Connect Providers
  providers: Record<string, ProviderStatus>;
  activeProvider: string | null;

  // Step 5: Verify & Finish
  verified: boolean;

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
  setPermanentDomain: (domain: string) => void;
  setRemoteReachable: (reachable: boolean) => void;
  setProviderStatus: (provider: string, status: ProviderStatus) => void;
  setActiveProvider: (provider: string | null) => void;
  setVerified: (verified: boolean) => void;
  addDockerLog: (log: string) => void;
  clearDockerLogs: () => void;
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
  tunnelMode: "none",
  permanentDomain: "",
  remoteReachable: false,

  providers: {},
  activeProvider: null,

  verified: false,

  dockerLogs: [],

  setStep: (step) => {
    set({ currentStep: step });
    // Persist step to Rust backend and save resume state
    updateStep(step).catch(() => {});
    saveResumeState().catch(() => {});
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
      syncTunnelConfig(tunnelMode, state.permanentDomain || null)
        .then(() => saveResumeState())
        .catch(() => {});
      return { tunnelMode };
    });
  },
  setPermanentDomain: (permanentDomain) => {
    set((state) => {
      syncTunnelConfig(state.tunnelMode, permanentDomain || null)
        .then(() => saveResumeState())
        .catch(() => {});
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
      syncProviderStatus(configured, stale)
        .then(() => saveResumeState())
        .catch(() => {});
      return { providers: newProviders };
    });
  },
  setActiveProvider: (activeProvider) => set({ activeProvider }),
  setVerified: (verified) => set({ verified }),
  addDockerLog: (log) =>
    set((state) => ({
      dockerLogs: [...state.dockerLogs.slice(-499), log],
    })),
  clearDockerLogs: () => set({ dockerLogs: [] }),
}));
