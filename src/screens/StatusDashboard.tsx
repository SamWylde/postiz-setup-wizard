import { useCallback, useEffect, useRef, useState } from "react";
import {
  getInstallSnapshot,
  getDockerLogs,
  restartAndVerify,
  stopStack,
  reconnectTunnel,
  exportDiagnostics,
  type InstallSnapshot,
} from "../lib/tauri";
import { useWizardStore } from "../store/wizardStore";
import { open } from "@tauri-apps/plugin-shell";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { CopyField } from "../components/ui/CopyField";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { CollapsiblePanel } from "../components/ui/CollapsiblePanel";
import { LogViewer } from "../components/ui/LogViewer";
import { showToast } from "../components/ui/Toast";
import {
  ExternalLink,
  RefreshCw,
  Square,
  Globe,
  Download,
  Activity,
  Server,
  Wifi,
  WifiOff,
  ArrowRight,
} from "lucide-react";

const POLL_INTERVAL = 15_000;

export function StatusDashboard() {
  const { installPath, port, setStep } = useWizardStore();

  const [snapshot, setSnapshot] = useState<InstallSnapshot | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [restarting, setRestarting] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [showStopConfirm, setShowStopConfirm] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const localUrl = `http://localhost:${port}`;

  const fetchSnapshot = useCallback(async () => {
    try {
      const snap = await getInstallSnapshot();
      setSnapshot(snap);
    } catch {
      // silently ignore poll errors
    }
  }, []);

  useEffect(() => {
    fetchSnapshot();
    intervalRef.current = setInterval(fetchSnapshot, POLL_INTERVAL);
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [fetchSnapshot]);

  const handleOpenPostiz = async () => {
    const url = snapshot?.tunnel_url ?? localUrl;
    try {
      await open(url);
    } catch (err) {
      showToast(`Could not open URL: ${String(err)}`, "error");
    }
  };

  const handleRestart = async () => {
    setRestarting(true);
    try {
      await restartAndVerify(installPath);
      await fetchSnapshot();
      showToast("Services restarted successfully", "success");
    } catch (err) {
      showToast(`Restart failed: ${String(err)}`, "error");
    } finally {
      setRestarting(false);
    }
  };

  const handleStop = async () => {
    setStopping(true);
    try {
      await stopStack(installPath);
      await fetchSnapshot();
      showToast("Services stopped", "success");
    } catch (err) {
      showToast(`Stop failed: ${String(err)}`, "error");
    } finally {
      setStopping(false);
      setShowStopConfirm(false);
    }
  };

  const handleReconnectTunnel = async () => {
    setReconnecting(true);
    try {
      const url = await reconnectTunnel(port, installPath);
      await fetchSnapshot();
      showToast(`Tunnel connected: ${url}`, "success");
    } catch (err) {
      showToast(`Reconnect failed: ${String(err)}`, "error");
    } finally {
      setReconnecting(false);
    }
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

  const handleMinimizeToTray = async () => {
    const window = getCurrentWindow();
    await window.hide();
  };

  const handleFetchLogs = async () => {
    try {
      const fetched = await getDockerLogs(installPath);
      setLogs(fetched);
    } catch (err) {
      showToast(`Could not fetch logs: ${String(err)}`, "error");
    }
  };

  const overallHealth =
    snapshot == null
      ? "loading"
      : snapshot.all_healthy && snapshot.postiz_responding
        ? "success"
        : "error";

  return (
    <div className="max-w-3xl">
      <div className="flex items-center justify-between mb-6">
        <div>
          <h2 className="text-2xl font-semibold text-gray-900">
            Postiz Dashboard
          </h2>
          <p className="text-sm text-gray-500 mt-1">
            Live status &mdash; refreshes every 15 seconds
          </p>
        </div>
        <button
          onClick={() => setStep(5)}
          className="inline-flex items-center gap-1.5 text-sm text-blue-600 hover:text-blue-700"
        >
          Back to Summary
          <ArrowRight className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Status section */}
      <Card className="mb-6">
        <div className="flex items-center gap-2 mb-3">
          <Activity className="h-4 w-4 text-gray-500" />
          <h3 className="text-sm font-medium text-gray-700">System Status</h3>
        </div>
        <div className="space-y-1">
          {/* Per-container status */}
          {snapshot?.containers.map((c) => (
            <StatusIndicator
              key={c.name}
              status={
                snapshot == null
                  ? "loading"
                  : c.health === "healthy"
                    ? "success"
                    : c.state === "running"
                      ? "warning"
                      : "error"
              }
              label={c.name}
              detail={`${c.state} - ${c.status}`}
            />
          ))}
          {snapshot && snapshot.containers.length === 0 && (
            <StatusIndicator
              status="error"
              label="Docker containers"
              detail="No containers found"
            />
          )}

          {/* Postiz app */}
          <StatusIndicator
            status={
              snapshot == null
                ? "loading"
                : snapshot.postiz_responding
                  ? "success"
                  : "error"
            }
            label="Postiz Application"
            detail={
              snapshot == null
                ? "Checking..."
                : snapshot.postiz_responding
                  ? "Responding"
                  : "Not responding"
            }
          />

          {/* Tunnel */}
          <StatusIndicator
            status={
              snapshot == null
                ? "loading"
                : snapshot.tunnel_alive
                  ? "success"
                  : snapshot.tunnel_mode === "none"
                    ? "warning"
                    : "error"
            }
            label="Tunnel"
            detail={
              snapshot == null
                ? "Checking..."
                : snapshot.tunnel_alive
                  ? snapshot.tunnel_url ?? "Active"
                  : snapshot.tunnel_mode === "none"
                    ? "Not configured"
                    : "Disconnected"
            }
          />

          {/* Overall */}
          <div className="pt-2 mt-2 border-t border-gray-100">
            <StatusIndicator
              status={overallHealth as "success" | "error" | "loading"}
              label="Overall Health"
              detail={
                snapshot == null
                  ? "Checking..."
                  : overallHealth === "success"
                    ? "All systems operational"
                    : "Issues detected"
              }
            />
          </div>
        </div>
      </Card>

      {/* Info section */}
      <Card className="mb-6">
        <div className="flex items-center gap-2 mb-3">
          <Server className="h-4 w-4 text-gray-500" />
          <h3 className="text-sm font-medium text-gray-700">
            Installation Info
          </h3>
        </div>
        <div className="space-y-3">
          {snapshot?.tunnel_alive && snapshot.tunnel_url && (
            <CopyField value={snapshot.tunnel_url} label="Public URL" />
          )}
          <CopyField value={localUrl} label="Local URL" />

          <div>
            <p className="text-sm font-medium text-gray-700 mb-1">
              Install path
            </p>
            <p className="text-sm text-gray-600 font-mono">{installPath}</p>
          </div>

          {snapshot && snapshot.providers_configured.length > 0 && (
            <div>
              <p className="text-sm font-medium text-gray-700 mb-1">
                Configured platforms
              </p>
              <p className="text-sm text-gray-600">
                {snapshot.providers_configured.join(", ")}
              </p>
            </div>
          )}

          <div className="flex items-center gap-2">
            {snapshot?.tunnel_alive ? (
              <Wifi className="h-4 w-4 text-green-500" />
            ) : (
              <WifiOff className="h-4 w-4 text-gray-400" />
            )}
            <p className="text-sm text-gray-600">
              Tunnel mode:{" "}
              <span className="font-medium">
                {snapshot?.tunnel_mode ?? "unknown"}
              </span>
              {snapshot?.permanent_domain && (
                <span className="text-gray-400">
                  {" "}
                  ({snapshot.permanent_domain})
                </span>
              )}
            </p>
          </div>
        </div>
      </Card>

      {/* Actions row */}
      <div className="flex flex-wrap items-center gap-3 mb-6">
        <Button onClick={handleOpenPostiz}>
          <ExternalLink className="h-4 w-4" />
          Open Postiz
        </Button>

        <Button
          variant="secondary"
          onClick={handleRestart}
          loading={restarting}
        >
          <RefreshCw className="h-4 w-4" />
          Restart
        </Button>

        {!showStopConfirm ? (
          <Button
            variant="secondary"
            onClick={() => setShowStopConfirm(true)}
          >
            <Square className="h-4 w-4" />
            Stop
          </Button>
        ) : (
          <div className="flex items-center gap-2">
            <Button
              variant="primary"
              onClick={handleStop}
              loading={stopping}
            >
              Confirm Stop
            </Button>
            <Button
              variant="ghost"
              onClick={() => setShowStopConfirm(false)}
            >
              Cancel
            </Button>
          </div>
        )}

        {snapshot?.tunnel_mode === "temporary" && (
          <Button
            variant="secondary"
            onClick={handleReconnectTunnel}
            loading={reconnecting}
          >
            <Globe className="h-4 w-4" />
            Reconnect Tunnel
          </Button>
        )}

        <Button
          variant="secondary"
          onClick={handleExportDiagnostics}
          loading={exporting}
        >
          <Download className="h-4 w-4" />
          Export Diagnostics
        </Button>

        <Button variant="ghost" onClick={handleMinimizeToTray}>
          Minimize to Tray
        </Button>
      </div>

      {/* Docker logs */}
      <CollapsiblePanel title="Docker Logs">
        <div className="space-y-2">
          <Button variant="ghost" onClick={handleFetchLogs}>
            <RefreshCw className="h-3.5 w-3.5" />
            Refresh Logs
          </Button>
          <LogViewer logs={logs} />
        </div>
      </CollapsiblePanel>
    </div>
  );
}
