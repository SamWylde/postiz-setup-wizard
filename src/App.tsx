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
  parseTunnelProvider,
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
  const { setProviderStatus, hydrateFromResume } = useWizardStore();

  // Snapshot-based routing on startup
  useEffect(() => {
    const init = async () => {
      // Load resume state separately so a corrupt install-state.json
      // does not prevent snapshot discovery from running
      let resume: Awaited<ReturnType<typeof loadResumeState>> = null;
      try {
        resume = await loadResumeState();

        if (resume) {
          // Restore frontend state from resume — silent hydration, no side effects
          const providers: Record<string, import("./store/wizardStore").ProviderStatus> = {};
          for (const id of resume.providers_configured) {
            providers[id] = "configured";
          }
          for (const id of resume.providers_stale) {
            providers[id] = "stale";
          }

          hydrateFromResume({
            installPath: resume.install_path || undefined,
            port: resume.port || undefined,
            tunnelMode: (resume.tunnel_mode as "temporary" | "permanent" | "none" | "local_https") || undefined,
            permanentDomain: resume.permanent_domain || undefined,
            tunnelProvider: resume.tunnel_provider
              ? parseTunnelProvider(resume.tunnel_provider)
              : undefined,
            tunnelConfig: resume.tunnel_config || undefined,
            providers,
            transferReviewPending: resume.transfer_review_pending || undefined,
          });
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
        // No install found — use hydrateFromResume (not setStep) to avoid
        // persisting step changes during startup routing.
        if (resume && resume.current_step > 0 && resume.current_step <= 1) {
          hydrateFromResume({ currentStep: resume.current_step });
        }
        // If step > 1 but install gone, currentStep stays 0 (default)
        setView("wizard");
        return;
      }

      // If snapshot discovered an install but resume state was missing/incomplete,
      // populate the store from the snapshot so the app can route correctly
      if (!resume && snap.install_path) {
        const providers: Record<string, import("./store/wizardStore").ProviderStatus> = {};
        for (const id of snap.providers_configured) {
          providers[id] = "configured";
        }
        for (const id of snap.providers_stale) {
          providers[id] = "stale";
        }

        hydrateFromResume({
          installPath: snap.install_path,
          port: snap.port,
          tunnelMode: snap.tunnel_mode as "temporary" | "permanent" | "none" | "local_https",
          permanentDomain: snap.permanent_domain || undefined,
          tunnelProvider: parseTunnelProvider(snap.tunnel_provider),
          providers,
        });
      }

      // Check tunnel status (live PID check)
      try {
        const tunnel = await getTunnelStatus();
        if (tunnel.status === "running" && tunnel.url) {
          hydrateFromResume({ tunnelUrl: tunnel.url, tunnelStatus: "running" });

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
          hydrateFromResume({ tunnelUrl: null, tunnelStatus: "idle" });
        }
      } catch {
        hydrateFromResume({ tunnelUrl: null, tunnelStatus: "idle" });
      }

      // Route based on snapshot — use hydrateFromResume (not setStep) to
      // avoid persisting step changes during startup routing.
      if (resume?.transfer_review_pending && snap.all_healthy && snap.postiz_responding) {
        hydrateFromResume({ currentStep: 3 }); // CreateWebLink
        setView("wizard");
      } else if (snap.all_healthy && snap.postiz_responding) {
        // Install is fully working. Honor mid-setup steps (2–4) so users
        // who paused during account/web-link/provider setup resume where
        // they left off. Otherwise go to SetupComplete (step 5) at minimum.
        const targetStep = resume && resume.current_step >= 2 && resume.current_step < 5
          ? resume.current_step
          : 5;
        hydrateFromResume({ currentStep: targetStep });
        setView("wizard");
      } else if (snap.has_staged_temp) {
        if (resume) hydrateFromResume({ currentStep: resume.current_step });
        setView("recovery");
      } else if (snap.docker_running && snap.containers.length > 0 && !snap.all_healthy) {
        if (resume) hydrateFromResume({ currentStep: resume.current_step });
        setView("recovery");
      } else if (!snap.docker_running) {
        // Docker isn't running — show PrepareComputer (step 0) which handles
        // Docker prerequisites. Do NOT persist: resume state keeps real progress.
        hydrateFromResume({ currentStep: 0 });
        setView("wizard");
      } else if (!snap.postiz_responding && snap.install_exists) {
        if (resume) hydrateFromResume({ currentStep: resume.current_step });
        setView("recovery");
      } else {
        if (resume && resume.current_step > 0) {
          hydrateFromResume({ currentStep: resume.current_step });
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
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  // Check for updates after startup settles (all views except loading)
  useEffect(() => {
    if (view === "loading") return;

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
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  const handleResumeWizard = () => {
    setView("wizard");
  };

  if (view === "loading") {
    return (
      <WizardLayout view="loading">
        <div className="flex items-center justify-center h-64">
          <p className="text-gray-500">Loading...</p>
        </div>
      </WizardLayout>
    );
  }

  if (view === "recovery" && snapshot) {
    return (
      <>
        <WizardLayout view="recovery">
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
  const layoutView = currentStep === 6 ? "dashboard" : "wizard";

  return (
    <>
      <WizardLayout view={layoutView}>
        <Screen />
      </WizardLayout>
      <ToastContainer />
    </>
  );
}

export default App;
