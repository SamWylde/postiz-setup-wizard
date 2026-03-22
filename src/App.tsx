import { useEffect, useState } from "react";
import { useWizardStore } from "./store/wizardStore";
import { WizardLayout } from "./components/wizard/WizardLayout";
import { PrepareComputer } from "./screens/PrepareComputer";
import { InstallPostiz } from "./screens/InstallPostiz";
import { CreateAccount } from "./screens/CreateAccount";
import { CreateWebLink } from "./screens/CreateWebLink";
import { ConnectProviders } from "./screens/ConnectProviders";
import { SetupComplete } from "./screens/SetupComplete";
import { StatusDashboard } from "./screens/StatusDashboard";
import { RecoveryCenter } from "./screens/RecoveryCenter";
import {
  getInstallSnapshot,
  loadResumeState,
  saveResumeState,
  getTunnelStatus,
  updateBaseUrls,
  repairStack,
  checkForUpdate,
  type InstallSnapshot,
} from "./lib/tauri";
import { listen, emit } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ToastContainer, showToast } from "./components/ui/Toast";

const screens = [
  PrepareComputer,
  InstallPostiz,
  CreateAccount,
  CreateWebLink,
  ConnectProviders,
  SetupComplete,
  StatusDashboard,
];

type AppView = "loading" | "wizard" | "recovery" | "dashboard";

function App() {
  const currentStep = useWizardStore((s) => s.currentStep);
  const [view, setView] = useState<AppView>("loading");
  const [snapshot, setSnapshot] = useState<InstallSnapshot | null>(null);
  const {
    setStep,
    setInstallPath,
    setPort,
    setTunnelUrl,
    setTunnelStatus,
    setTunnelMode,
    setTunnelProvider,
    setPermanentDomain,
    setProviderStatus,
    setTransferReviewPending,
  } = useWizardStore();

  // Snapshot-based routing on startup
  useEffect(() => {
    const init = async () => {
      // Load resume state separately so a corrupt install-state.json
      // does not prevent snapshot discovery from running
      let resume: Awaited<ReturnType<typeof loadResumeState>> = null;
      try {
        resume = await loadResumeState();

        if (resume) {
          // Restore frontend state from resume
          if (resume.install_path) setInstallPath(resume.install_path);
          if (resume.port) setPort(resume.port);
          if (resume.tunnel_mode) setTunnelMode(resume.tunnel_mode as "temporary" | "permanent" | "none");
          if (resume.permanent_domain) setPermanentDomain(resume.permanent_domain);
          if (resume.tunnel_provider) {
            setTunnelProvider(resume.tunnel_provider as "cloudflared" | "ngrok" | "zrok" | "pinggy");
          }

          for (const id of resume.providers_configured) {
            setProviderStatus(id, "configured");
          }
          for (const id of resume.providers_stale) {
            setProviderStatus(id, "stale");
          }
          if (resume.transfer_review_pending) {
            setTransferReviewPending(true);
          }
        }
      } catch {
        // Resume state is missing or corrupt — continue with snapshot discovery
      }

      // Now get a live snapshot of actual system state
      let snap: InstallSnapshot;
      try {
        snap = await getInstallSnapshot();
      } catch {
        // Can't even probe the system — start fresh
        setView("wizard");
        return;
      }
      setSnapshot(snap);

      if (!snap.install_exists) {
        // No install found — fresh setup or stale resume state.
        // Only resume the step if it's still in the pre-install range (step 0-1).
        // Beyond that, the install no longer exists so reset to start.
        if (resume && resume.current_step > 0 && resume.current_step <= 1) {
          setStep(resume.current_step);
        } else if (resume && resume.current_step > 1) {
          // Install was removed or lost — reset to beginning
          setStep(0);
        }
        setView("wizard");
        return;
      }

      // If snapshot discovered an install but resume state was missing/incomplete,
      // populate the store from the snapshot so the app can route correctly
      if (!resume && snap.install_path) {
        setInstallPath(snap.install_path);
        setPort(snap.port);
      }

      // Check tunnel status (live PID check)
      try {
        const tunnel = await getTunnelStatus();
        if (tunnel.status === "running" && tunnel.url) {
          setTunnelUrl(tunnel.url);
          setTunnelStatus("running");

          // Detect tunnel URL change — mark providers stale
          if (
            resume?.tunnel_url &&
            tunnel.url !== resume.tunnel_url &&
            resume.providers_configured.length > 0
          ) {
            if (resume.install_path) {
              await updateBaseUrls(resume.install_path, tunnel.url);
              await repairStack(resume.install_path);
            }
            for (const id of resume.providers_configured) {
              setProviderStatus(id, "stale");
            }
            // Persist the stale state so it survives a crash
            await saveResumeState().catch(() => {});
          }
        } else {
          setTunnelUrl(null);
          setTunnelStatus("idle");
        }
      } catch {
        setTunnelUrl(null);
        setTunnelStatus("idle");
      }

      // Route based on snapshot
      if (resume?.transfer_review_pending && snap.all_healthy && snap.postiz_responding) {
        // Import completed but user hasn't finished tunnel/provider setup
        setStep(3); // CreateWebLink
        setView("wizard");
      } else if (snap.all_healthy && snap.postiz_responding) {
        // If the user stopped mid-wizard (step < 5), resume there instead of
        // jumping to the dashboard — a healthy stack doesn't mean setup is complete.
        // Step 1 (install) with a healthy stack means install finished but user
        // closed before advancing — bump them to step 2 (create account).
        if (resume && resume.current_step >= 1 && resume.current_step < 5) {
          setStep(Math.max(resume.current_step, 2));
          setView("wizard");
        } else if (!resume && snap.current_step >= 1 && snap.current_step < 5) {
          // No resume state but snapshot inferred progress from disk
          setStep(Math.max(snap.current_step, 2));
          setView("wizard");
        } else {
          // Setup was complete (step >= 5) or no resume — go to dashboard
          setStep(6);
          setView("dashboard");
        }
      } else if (snap.recovery_available) {
        // Install exists but issues detected — show recovery
        if (resume) setStep(resume.current_step);
        setView("recovery");
      } else {
        // Install exists, nothing running, resume wizard
        if (resume && resume.current_step > 0) {
          setStep(resume.current_step);
        }
        setView("wizard");
      }
    };

    init();
  }, []);

  // Listen for copy-to-clipboard events from tray menu
  useEffect(() => {
    const unlisten = listen<string>("copy-to-clipboard", (event) => {
      writeText(event.payload).catch(() => {});
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // Check for updates after startup settles (only on dashboard/recovery)
  useEffect(() => {
    if (view !== "dashboard" && view !== "recovery") return;

    const timer = setTimeout(async () => {
      try {
        const info = await checkForUpdate();
        if (info.available) {
          // Share with dashboard so the install button appears immediately
          await emit("update-info-discovered", info);
          showToast(
            `Update available: v${info.latest_version}`,
            "success",
          );
        }
      } catch {
        // silently ignore update check failures
      }
    }, 3000);

    return () => clearTimeout(timer);
  }, [view]);

  // Listen for tray "Check for Updates" trigger
  useEffect(() => {
    const unlisten = listen("trigger-update-check", async () => {
      try {
        const info = await checkForUpdate();
        if (info.available) {
          await emit("update-info-discovered", info);
          showToast(
            `Update available: v${info.latest_version}`,
            "success",
          );
        } else {
          showToast("You're running the latest version.", "success");
        }
      } catch (err) {
        showToast(`Update check failed: ${String(err)}`, "error");
      }
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const handleResumeWizard = () => {
    setView("wizard");
  };

  if (view === "loading") {
    return (
      <WizardLayout>
        <div className="flex items-center justify-center h-64">
          <p className="text-gray-500">Loading...</p>
        </div>
      </WizardLayout>
    );
  }

  if (view === "recovery" && snapshot) {
    return (
      <>
        <WizardLayout>
          <RecoveryCenter
            snapshot={snapshot}
            onResumeWizard={handleResumeWizard}
          />
        </WizardLayout>
        <ToastContainer />
      </>
    );
  }

  const Screen = screens[currentStep] ?? PrepareComputer;

  return (
    <>
      <WizardLayout>
        <Screen />
      </WizardLayout>
      <ToastContainer />
    </>
  );
}

export default App;
