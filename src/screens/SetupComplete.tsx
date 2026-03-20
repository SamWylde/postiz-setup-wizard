import { useState } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  getInstallSnapshot,
  exportDiagnostics,
  type InstallSnapshot,
} from "../lib/tauri";
import { open } from "@tauri-apps/plugin-shell";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { CopyField } from "../components/ui/CopyField";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { showToast } from "../components/ui/Toast";
import {
  ExternalLink,
  PartyPopper,
  ArrowRight,
  Download,
} from "lucide-react";

export function SetupComplete() {
  const { port, installPath, tunnelUrl, tunnelMode, providers, setStep } =
    useWizardStore();

  const [snapshot, setSnapshot] = useState<InstallSnapshot | null>(null);
  const [checking, setChecking] = useState(false);
  const [exporting, setExporting] = useState(false);

  const localUrl = `http://localhost:${port}`;
  const publicUrl = tunnelUrl ?? null;

  const configuredProviders = Object.entries(providers)
    .filter(([, s]) => s === "configured")
    .map(([id]) => id);

  const handleVerify = async () => {
    setChecking(true);
    try {
      const snap = await getInstallSnapshot();
      setSnapshot(snap);
    } catch {
      showToast("Could not verify status", "error");
    } finally {
      setChecking(false);
    }
  };

  const handleOpenPostiz = async () => {
    const url = publicUrl ?? localUrl;
    try {
      await open(url);
    } catch (err) {
      showToast(`Could not open URL: ${String(err)}`, "error");
    }
  };

  const handleMinimizeToTray = async () => {
    const window = getCurrentWindow();
    await window.hide();
  };

  const handleExportDiagnostics = async () => {
    setExporting(true);
    try {
      const filePath = await exportDiagnostics();
      showToast(`Diagnostics saved to ${filePath}`, "success");
    } catch (err) {
      showToast(`Export failed: ${String(err)}`, "error");
    } finally {
      setExporting(false);
    }
  };

  const allGood =
    snapshot !== null &&
    snapshot.all_healthy &&
    snapshot.postiz_responding;

  return (
    <div className="max-w-2xl">
      {/* Celebration header */}
      <div className="flex items-center gap-3 mb-2">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-green-100">
          <PartyPopper className="h-5 w-5 text-green-600" />
        </div>
        <h2 className="text-2xl font-semibold text-gray-900">
          You're all set!
        </h2>
      </div>
      <p className="text-gray-600 mb-6">
        Postiz is installed and running on your computer. Here's a summary of
        your setup.
      </p>

      {/* Verification */}
      <Card className="mb-6">
        <h3 className="text-sm font-medium text-gray-700 mb-3">
          Quick Health Check
        </h3>
        {snapshot === null ? (
          <Button onClick={handleVerify} loading={checking}>
            Verify Everything Works
          </Button>
        ) : (
          <div className="space-y-1">
            <StatusIndicator
              status={
                snapshot.all_healthy ? "success" : "error"
              }
              label="Docker containers"
              detail={
                snapshot.all_healthy
                  ? "All healthy"
                  : "Some issues detected"
              }
            />
            <StatusIndicator
              status={
                snapshot.postiz_responding ? "success" : "error"
              }
              label="Postiz application"
              detail={
                snapshot.postiz_responding
                  ? "Responding"
                  : "Not responding"
              }
            />
            {tunnelMode !== "none" && (
              <StatusIndicator
                status={
                  snapshot.tunnel_alive ? "success" : "warning"
                }
                label="Web link"
                detail={
                  snapshot.tunnel_alive
                    ? snapshot.tunnel_url ?? "Active"
                    : "Not connected"
                }
              />
            )}
            <div className="pt-2 mt-2 border-t border-gray-100">
              <StatusIndicator
                status={allGood ? "success" : "error"}
                label="Overall"
                detail={
                  allGood
                    ? "Everything is working!"
                    : "Some issues need attention"
                }
              />
            </div>
          </div>
        )}
      </Card>

      {/* Summary */}
      <Card className="mb-6">
        <h3 className="text-sm font-medium text-gray-700 mb-3">
          Your Setup
        </h3>
        <div className="space-y-3">
          {publicUrl && (
            <CopyField value={publicUrl} label="Public URL" />
          )}
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
              <p className="text-sm text-gray-600 capitalize">
                {configuredProviders.join(", ")}
              </p>
            </div>
          )}
        </div>
      </Card>

      {/* Important notes */}
      <Card className="mb-6 bg-blue-50 border-blue-200">
        <h3 className="text-sm font-medium text-blue-900 mb-2">
          Good to know
        </h3>
        <ul className="space-y-1.5 text-sm text-blue-800">
          <li>
            Keep this app running to maintain your setup. You can minimize it
            to the system tray.
          </li>
          {tunnelMode === "temporary" && (
            <li>
              Your web link is temporary and changes when the app restarts.
              You'll need to update redirect URLs in developer portals if it
              changes.
            </li>
          )}
          <li>
            Double-click the tray icon to reopen this window at any time.
          </li>
        </ul>
      </Card>

      {/* Actions */}
      <div className="flex flex-wrap items-center gap-3 mb-6">
        <Button onClick={handleOpenPostiz}>
          <ExternalLink className="h-4 w-4" />
          Open Postiz
        </Button>
        <Button variant="secondary" onClick={handleMinimizeToTray}>
          Minimize to Tray
        </Button>
        <Button
          variant="ghost"
          onClick={handleExportDiagnostics}
          loading={exporting}
        >
          <Download className="h-4 w-4" />
          Export Diagnostics
        </Button>
      </div>

      {/* Link to dashboard */}
      <div className="pt-4 border-t border-gray-200">
        <button
          onClick={() => setStep(6)}
          className="inline-flex items-center gap-1.5 text-sm text-blue-600 hover:text-blue-700"
        >
          Go to Status Dashboard
          <ArrowRight className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}
