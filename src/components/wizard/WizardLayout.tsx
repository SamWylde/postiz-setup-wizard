import { useEffect, useState } from "react";
import { useWizardStore } from "../../store/wizardStore";
import { getCurrentAppVersion, installUpdate } from "../../lib/tauri";
import { listen } from "@tauri-apps/api/event";
import { StepIndicator } from "./StepIndicator";
import { Activity, Wrench } from "lucide-react";

const steps = [
  { name: "Prepare Computer", description: "Check prerequisites" },
  { name: "Install Postiz", description: "Download and set up" },
  { name: "Create Account", description: "Set up your admin account" },
  { name: "Create Web Link", description: "Set up public access" },
  { name: "Connect Platforms", description: "Configure social media" },
  { name: "Setup Complete", description: "You're all set!" },
];

interface WizardLayoutProps {
  children: React.ReactNode;
  view?: "wizard" | "dashboard" | "recovery" | "loading";
}

export function WizardLayout({ children, view = "wizard" }: WizardLayoutProps) {
  const currentStep = useWizardStore((s) => s.currentStep);
  const setStep = useWizardStore((s) => s.setStep);
  const [appVersion, setAppVersion] = useState("v0.1.0");
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [latestVersion, setLatestVersion] = useState("");
  const [installing, setInstalling] = useState(false);
  const [installStatus, setInstallStatus] = useState("");

  useEffect(() => {
    getCurrentAppVersion()
      .then((v) => setAppVersion(`v${v}`))
      .catch(() => {});
  }, []);

  // Listen for update discovery
  useEffect(() => {
    const unlisten = listen<{ available: boolean; latest_version: string }>("update-info-discovered", (event) => {
      if (event.payload.available) {
        setUpdateAvailable(true);
        setLatestVersion(event.payload.latest_version);
      }
    });
    return () => { unlisten.then((f) => f()).catch(() => {}); };
  }, []);

  // Listen for update status and progress
  useEffect(() => {
    const unlistenStatus = listen<string>("update-status", (event) => {
      const status = event.payload;
      if (status === "downloading") setInstallStatus("Downloading...");
      else if (status === "installing") setInstallStatus("Installing — app will restart...");
      else if (status === "error") { setInstallStatus(""); setInstalling(false); }
    });
    const unlistenProgress = listen<{ percent: number; downloaded: number; total: number }>("update-progress", (event) => {
      if (event.payload.percent > 0) {
        setInstallStatus(`Downloading... ${event.payload.percent}%`);
      }
    });
    return () => {
      unlistenStatus.then((f) => f()).catch(() => {});
      unlistenProgress.then((f) => f()).catch(() => {});
    };
  }, []);

  const handleInstall = async () => {
    setInstalling(true);
    try {
      await installUpdate();
    } catch {
      setInstalling(false);
      setInstallStatus("");
    }
  };

  return (
    <div className="flex h-screen">
      {/* Sidebar */}
      <div className="w-64 shrink-0 border-r border-gray-200 bg-white p-6 flex flex-col">
        <div className="mb-8">
          <h1 className="text-lg font-semibold text-gray-900">Postiz</h1>
          <p className="text-xs text-gray-500">
            {view === "dashboard"
              ? "Dashboard"
              : view === "recovery"
                ? "Recovery Center"
                : "Setup Wizard"}
          </p>
        </div>

        {/* Show view-specific sidebar content */}
        {view === "dashboard" && (
          <div className="flex-1 space-y-2">
            <div className="flex items-center gap-2 rounded-lg bg-blue-50 px-3 py-2">
              <Activity className="h-4 w-4 text-blue-600" />
              <span className="text-sm font-medium text-blue-700">Live Dashboard</span>
            </div>
            <p className="text-xs text-gray-500 px-1">
              Monitor your Postiz services, manage your tunnel, and check for updates.
            </p>
          </div>
        )}

        {view === "recovery" && (
          <div className="flex-1 space-y-2">
            <div className="flex items-center gap-2 rounded-lg bg-amber-50 px-3 py-2">
              <Wrench className="h-4 w-4 text-amber-600" />
              <span className="text-sm font-medium text-amber-700">Recovery Center</span>
            </div>
            <p className="text-xs text-gray-500 px-1">
              We found an existing install with issues. Use the options here to repair, resume, or start over.
            </p>
          </div>
        )}

        {(view === "wizard" || view === "loading") && (
          <nav className="flex-1 space-y-1">
            {steps.map((step, index) => (
              <StepIndicator
                key={index}
                stepNumber={index + 1}
                name={step.name}
                description={step.description}
                state={
                  index < currentStep
                    ? "complete"
                    : index === currentStep
                      ? "active"
                      : "pending"
                }
                onClick={
                  index < currentStep ? () => setStep(index) : undefined
                }
              />
            ))}
          </nav>
        )}

        <div className="pt-4 border-t border-gray-200">
          <p className="text-xs text-gray-400">{appVersion}</p>
        </div>
      </div>

      {/* Main content */}
      <div className="flex-1 overflow-y-auto bg-gray-50">
        {updateAvailable && (
          <div className="bg-blue-600 px-4 py-2 flex items-center justify-between text-white text-sm">
            <span>Update available: v{latestVersion}</span>
            <button onClick={handleInstall} disabled={installing}>
              {installStatus || "Install & Restart"}
            </button>
          </div>
        )}
        <div className="p-8">{children}</div>
      </div>
    </div>
  );
}
