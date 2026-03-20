import { useEffect, useState } from "react";
import { useWizardStore } from "../store/wizardStore";
import { getStackStatus, getTunnelStatus, saveResumeState } from "../lib/tauri";
import { open } from "@tauri-apps/plugin-shell";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { CopyField } from "../components/ui/CopyField";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { providers } from "../components/providers/registry";
import { AlertTriangle, ExternalLink } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function VerifyFinish() {
  const {
    installPath,
    port,
    tunnelUrl,
    providers: providerStatuses,
  } = useWizardStore();

  const [checks, setChecks] = useState({
    containersHealthy: false,
    postizResponding: false,
    tunnelActive: false,
    checking: true,
  });

  const localUrl = `http://localhost:${port}`;

  useEffect(() => {
    const verify = async () => {
      try {
        const stackStatus = await getStackStatus(installPath);
        const tunnel = await getTunnelStatus();

        setChecks({
          containersHealthy: stackStatus.all_healthy,
          postizResponding: stackStatus.postiz_responding,
          tunnelActive: tunnel.status === "running",
          checking: false,
        });

        // Save final resume state
        await saveResumeState();
      } catch {
        setChecks((prev) => ({ ...prev, checking: false }));
      }
    };

    verify();
  }, []);

  const configuredProviders = Object.entries(providerStatuses)
    .filter(([_, status]) => status === "configured")
    .map(([id]) => providers.find((p) => p.id === id)?.name ?? id);

  const handleMinimizeToTray = async () => {
    const window = getCurrentWindow();
    await window.hide();
  };

  return (
    <div className="max-w-2xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        You're all set!
      </h2>
      <p className="text-gray-600 mb-6">
        Postiz is installed and configured. Here's a summary of your setup.
      </p>

      {/* Verification */}
      <Card className="mb-6">
        <h3 className="text-sm font-medium text-gray-700 mb-3">
          System Status
        </h3>
        <div className="space-y-1">
          <StatusIndicator
            status={
              checks.checking
                ? "loading"
                : checks.containersHealthy
                  ? "success"
                  : "error"
            }
            label="Docker containers"
            detail={checks.containersHealthy ? "All healthy" : "Issues detected"}
          />
          <StatusIndicator
            status={
              checks.checking
                ? "loading"
                : checks.postizResponding
                  ? "success"
                  : "error"
            }
            label="Postiz application"
            detail={checks.postizResponding ? "Responding" : "Not responding"}
          />
          <StatusIndicator
            status={
              checks.checking
                ? "loading"
                : checks.tunnelActive
                  ? "success"
                  : "warning"
            }
            label="Web link"
            detail={checks.tunnelActive ? "Active" : "Not active"}
          />
        </div>
      </Card>

      {/* Summary */}
      <Card className="mb-6">
        <h3 className="text-sm font-medium text-gray-700 mb-3">
          Your Setup
        </h3>
        <div className="space-y-3">
          {tunnelUrl && <CopyField value={tunnelUrl} label="Public URL" />}
          <CopyField value={localUrl} label="Local URL" />

          <div>
            <p className="text-sm font-medium text-gray-700 mb-1">
              Install location
            </p>
            <p className="text-sm text-gray-600 font-mono">{installPath}</p>
          </div>

          {configuredProviders.length > 0 && (
            <div>
              <p className="text-sm font-medium text-gray-700 mb-1">
                Connected platforms
              </p>
              <p className="text-sm text-gray-600">
                {configuredProviders.join(", ")}
              </p>
            </div>
          )}
        </div>
      </Card>

      {/* Important notes */}
      <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-4 mb-6">
        <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
        <div className="text-sm text-amber-800 space-y-2">
          <p>
            <strong>Keep this app running</strong> to maintain your web link.
            Minimize it to the system tray.
          </p>
          <p>
            <strong>The web link is temporary</strong> and changes when this app
            restarts. You'll need to update redirect URLs in developer portals if
            it changes.
          </p>
        </div>
      </div>

      {/* Actions */}
      <div className="flex items-center gap-3">
        <Button onClick={() => open(tunnelUrl ?? localUrl)}>
          <ExternalLink className="h-4 w-4" />
          Open Postiz
        </Button>
        <Button variant="secondary" onClick={handleMinimizeToTray}>
          Minimize to Tray
        </Button>
      </div>
    </div>
  );
}
