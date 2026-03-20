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
  type InstallSnapshot,
} from "./lib/tauri";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ToastContainer } from "./components/ui/Toast";

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
    setPermanentDomain,
    setProviderStatus,
  } = useWizardStore();

  // Snapshot-based routing on startup
  useEffect(() => {
    const init = async () => {
      try {
        // First try to load resume state (restores Rust AppState)
        const resume = await loadResumeState();

        if (resume) {
          // Restore frontend state from resume
          if (resume.install_path) setInstallPath(resume.install_path);
          if (resume.port) setPort(resume.port);
          if (resume.tunnel_mode) setTunnelMode(resume.tunnel_mode as "temporary" | "permanent" | "none");
          if (resume.permanent_domain) setPermanentDomain(resume.permanent_domain);

          for (const id of resume.providers_configured) {
            setProviderStatus(id, "configured");
          }
          for (const id of resume.providers_stale) {
            setProviderStatus(id, "stale");
          }
        }

        // Now get a live snapshot of actual system state
        const snap = await getInstallSnapshot();
        setSnapshot(snap);

        if (!snap.install_exists) {
          // No install found — fresh setup
          if (resume && resume.current_step > 0) {
            setStep(resume.current_step);
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
        if (snap.all_healthy && snap.postiz_responding) {
          // Everything is running — go to dashboard
          setStep(6);
          setView("dashboard");
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
      } catch {
        // No resume state, no snapshot — start fresh
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
