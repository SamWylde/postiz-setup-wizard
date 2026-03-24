import { useCallback, useEffect, useRef, useState } from "react";
import {
  getInstallSnapshot,
  getDockerLogs,
  restartAndVerify,
  stopStack,
  reconnectTunnel,
  saveResumeState,
  exportDiagnostics,
  checkForUpdate,
  installUpdate,
  getCurrentAppVersion,
  checkPostizUpdate,
  upgradePostiz,
  onUpgradeProgress,
  type InstallSnapshot,
  type UpdateInfo,
  type PostizUpdateInfo,
} from "../lib/tauri";
import { useWizardStore } from "../store/wizardStore";
import { open } from "@tauri-apps/plugin-shell";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { CopyField } from "../components/ui/CopyField";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { CollapsiblePanel } from "../components/ui/CollapsiblePanel";
import { LogViewer } from "../components/ui/LogViewer";
import { showToast } from "../components/ui/Toast";
import { ExportCloneDialog } from "../components/ExportCloneDialog";
import {
  ExternalLink,
  RefreshCw,
  Square,
  Globe,
  Download,
  Archive,
  Activity,
  Server,
  Wifi,
  WifiOff,
  ArrowRight,
  CloudDownload,
  CheckCircle2,
  Package,
} from "lucide-react";

const POLL_INTERVAL = 15_000;

/** Map Docker container names to user-friendly service labels.
 *  Known containers from docker-compose.yml:
 *    postiz, postiz-postgres, postiz-redis, temporal, temporal-postgresql,
 *    temporal-elasticsearch, temporal-admin-tools, temporal-ui
 */
function friendlyServiceName(containerName: string): string {
  const lower = containerName.toLowerCase();
  // Main Postiz app — container_name is just "postiz"
  if (lower === "postiz" || (lower.includes("postiz") && (lower.includes("app") || lower.includes("frontend")))) return "Postiz App";
  if (lower === "postiz-postgres") return "Database (PostgreSQL)";
  if (lower === "postiz-redis") return "Cache (Redis)";
  if (lower === "temporal-postgresql") return "Task Scheduler DB";
  if (lower === "temporal-elasticsearch") return "Search Engine (Elasticsearch)";
  if (lower === "temporal-admin-tools") return "Task Scheduler Admin";
  if (lower === "temporal-ui") return "Task Scheduler UI";
  if (lower === "temporal") return "Task Scheduler (Temporal)";
  // Generalized fallback for unknown containers
  if (lower.includes("postgres")) return "Database (PostgreSQL)";
  if (lower.includes("redis")) return "Cache (Redis)";
  if (lower.includes("temporal")) return "Task Scheduler (Temporal)";
  if (lower.includes("elasticsearch") || lower.includes("elastic")) return "Search Engine (Elasticsearch)";
  // Fall back to the raw name with dashes cleaned up
  return containerName.replace(/[-_]+/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

/** Map Docker state/status to user-friendly text */
function friendlyContainerStatus(state: string, health: string): string {
  if (health === "healthy") return "Healthy";
  if (state === "running" && health === "starting") return "Starting up...";
  if (state === "running" && health === "unhealthy") return "Running (unhealthy)";
  if (state === "running") return "Running";
  if (state === "exited") return "Stopped";
  if (state === "restarting") return "Restarting...";
  if (state === "created") return "Created (not started)";
  return `${state}${health ? ` - ${health}` : ""}`;
}

export function StatusDashboard() {
  const { installPath, port, tunnelProvider, permanentDomain, tunnelMode, setStep, setProviderStatus, providers } = useWizardStore();

  const [snapshot, setSnapshot] = useState<InstallSnapshot | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [restarting, setRestarting] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [showStopConfirm, setShowStopConfirm] = useState(false);
  const [showExport, setShowExport] = useState(false);
  const [showReconnectDialog, setShowReconnectDialog] = useState(false);
  const [reconnectConfig, setReconnectConfig] = useState("");
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [updateStatus, setUpdateStatus] = useState<string>("");
  const [updatePercent, setUpdatePercent] = useState<number>(0);
  const [appVersion, setAppVersion] = useState<string>("");
  const [postizUpdate, setPostizUpdate] = useState<PostizUpdateInfo | null>(null);
  const [postizChecking, setPostizChecking] = useState(false);
  const [postizUpgrading, setPostizUpgrading] = useState(false);
  const [upgradeStatus, setUpgradeStatus] = useState("");
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
    getCurrentAppVersion().then(setAppVersion).catch(() => {});
    intervalRef.current = setInterval(fetchSnapshot, POLL_INTERVAL);
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [fetchSnapshot]);

  // Listen for update info discovered by App.tsx (startup check or tray menu)
  useEffect(() => {
    const unlisten = listen<UpdateInfo>("update-info-discovered", (event) => {
      setUpdateInfo(event.payload);
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  // Listen for upgrade progress events
  useEffect(() => {
    const unlisten = onUpgradeProgress((event) => {
      setUpgradeStatus(event.payload.message);
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  // Listen for wizard update status and progress
  useEffect(() => {
    const unlistenStatus = listen<string>("update-status", (event) => {
      const status = event.payload;
      if (status === "downloading") {
        setUpdateStatus("Downloading update...");
      } else if (status === "installing") {
        setUpdateStatus("Installing update — app will restart...");
      } else if (status === "error") {
        setUpdateStatus("");
      }
    });
    const unlistenProgress = listen<{ percent: number; downloaded: number; total: number }>(
      "update-progress",
      (event) => {
        setUpdatePercent(event.payload.percent);
        if (event.payload.percent > 0) {
          setUpdateStatus(`Downloading update... ${event.payload.percent}%`);
        }
      },
    );
    return () => {
      unlistenStatus.then((f) => f()).catch(() => {});
      unlistenProgress.then((f) => f()).catch(() => {});
    };
  }, []);

  const handleCheckPostizUpdate = async () => {
    setPostizChecking(true);
    try {
      const info = await checkPostizUpdate();
      setPostizUpdate(info);
      if (!info.update_available) {
        showToast("Postiz is already up to date.", "success");
      }
    } catch (err) {
      showToast(`Check failed: ${String(err)}`, "error");
    } finally {
      setPostizChecking(false);
    }
  };

  const handleUpgradePostiz = async () => {
    setPostizUpgrading(true);
    setUpgradeStatus("Starting upgrade...");
    try {
      await upgradePostiz();
      await fetchSnapshot();
      setPostizUpdate(null);
      setUpgradeStatus("");
      showToast("Postiz upgraded successfully!", "success");
    } catch (err) {
      showToast(`Upgrade failed: ${String(err)}`, "error");
      setUpgradeStatus("");
    } finally {
      setPostizUpgrading(false);
    }
  };

  const handleCheckForUpdate = async () => {
    setUpdateChecking(true);
    try {
      const info = await checkForUpdate();
      setUpdateInfo(info);
      if (info.available) {
        await emit("update-info-discovered", info);
      } else {
        showToast("You're running the latest version.", "success");
      }
    } catch (err) {
      showToast(`Update check failed: ${String(err)}`, "error");
    } finally {
      setUpdateChecking(false);
    }
  };

  const handleInstallUpdate = async () => {
    setUpdateInstalling(true);
    try {
      // On Windows, the app process exits during install, so this
      // promise may never resolve. The "installing" status is emitted
      // by Rust before exit. If it does resolve (non-Windows), restart happens server-side.
      await installUpdate();
    } catch (err) {
      showToast(`Update failed: ${String(err)}`, "error");
      setUpdateInstalling(false);
    }
  };

  const handleOpenPostiz = async () => {
    // Prefer live tunnel URL, then permanent domain, then localhost
    const url = snapshot?.tunnel_url ?? (permanentDomain && tunnelMode === "permanent" ? permanentDomain : localUrl);
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

  const needsReconnectConfig = tunnelProvider === "ngrok" || tunnelProvider === "pinggy";

  const handleReconnectTunnelClick = () => {
    if (needsReconnectConfig) {
      setShowReconnectDialog(true);
    } else {
      handleReconnectTunnel();
    }
  };

  const handleReconnectTunnel = async () => {
    setShowReconnectDialog(false);
    setReconnecting(true);
    try {
      const previousUrl = snapshot?.tunnel_url;
      const url = await reconnectTunnel(port, installPath, tunnelProvider, reconnectConfig || undefined);
      setReconnectConfig("");
      await fetchSnapshot();
      // If the URL changed, mark all configured providers as stale
      if (previousUrl && url !== previousUrl) {
        const configured = Object.entries(providers)
          .filter(([, s]) => s === "configured")
          .map(([id]) => id);
        for (const id of configured) {
          setProviderStatus(id, "stale");
        }
        if (configured.length > 0) {
          showToast(`Tunnel connected: ${url}. Provider redirect URLs need updating.`, "success");
        } else {
          showToast(`Tunnel connected: ${url}`, "success");
        }
      } else {
        showToast(`Tunnel connected: ${url}`, "success");
      }
      // Persist the updated state
      await saveResumeState().catch(() => {});
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
              label={friendlyServiceName(c.name)}
              detail={friendlyContainerStatus(c.state, c.health)}
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
                  : snapshot.tunnel_mode === "permanent" && snapshot.permanent_domain
                    ? "success"
                    : snapshot.tunnel_mode === "none"
                      ? "success"
                      : "error"
            }
            label="Web Link"
            detail={
              snapshot == null
                ? "Checking..."
                : snapshot.tunnel_alive
                  ? snapshot.tunnel_url ?? "Active"
                  : snapshot.tunnel_mode === "permanent" && snapshot.permanent_domain
                    ? `Custom domain: ${snapshot.permanent_domain}`
                    : snapshot.tunnel_mode === "none"
                      ? "Local-only mode"
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

      {/* Update section */}
      <Card className="mb-6">
        <div className="flex items-center gap-2 mb-3">
          <CloudDownload className="h-4 w-4 text-gray-500" />
          <h3 className="text-sm font-medium text-gray-700">App Updates</h3>
        </div>
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm text-gray-600">
              Current version: <span className="font-mono font-medium">v{appVersion}</span>
            </p>
            {updateInfo?.available && (
              <p className="text-sm text-green-600 mt-1">
                Update available: <span className="font-mono font-medium">v{updateInfo.latest_version}</span>
              </p>
            )}
            {updateInfo && !updateInfo.available && (
              <p className="text-sm text-gray-500 mt-1 flex items-center gap-1">
                <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
                Up to date
              </p>
            )}
          </div>
          <div className="flex gap-2">
            {updateInfo?.available ? (
              <Button
                onClick={handleInstallUpdate}
                loading={updateInstalling}
              >
                Install Update & Restart
              </Button>
            ) : (
              <Button
                variant="secondary"
                onClick={handleCheckForUpdate}
                loading={updateChecking}
              >
                Check for Updates
              </Button>
            )}
          </div>
        </div>
        {updateStatus && (
          <div className="mt-3 pt-3 border-t border-gray-100">
            <div className="flex items-center gap-2">
              <p className="text-sm text-blue-600">{updateStatus}</p>
              {updatePercent > 0 && updatePercent < 100 && (
                <div className="flex-1 max-w-xs h-2 bg-gray-200 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-blue-500 rounded-full transition-all"
                    style={{ width: `${updatePercent}%` }}
                  />
                </div>
              )}
            </div>
          </div>
        )}
        {updateInfo?.body && (
          <div className="mt-3 pt-3 border-t border-gray-100">
            <p className="text-xs font-medium text-gray-500 mb-1">Release notes</p>
            <p className="text-sm text-gray-600 whitespace-pre-wrap">{updateInfo.body}</p>
          </div>
        )}
      </Card>

      {/* Postiz Version / Upgrade section */}
      <Card className="mb-6">
        <div className="flex items-center gap-2 mb-3">
          <Package className="h-4 w-4 text-gray-500" />
          <h3 className="text-sm font-medium text-gray-700">Postiz Version</h3>
        </div>
        <div className="flex items-center justify-between">
          <div>
            {postizUpdate?.update_available ? (
              <p className="text-sm text-green-600">
                A newer Postiz app image is available. This upgrades only the Postiz app container — databases and other services are not affected.
              </p>
            ) : postizUpdate && !postizUpdate.update_available ? (
              <p className="text-sm text-gray-500 flex items-center gap-1">
                <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
                Postiz app image is up to date
              </p>
            ) : (
              <p className="text-sm text-gray-500">
                Check if a newer Postiz app container image is available. Upgrades only the Postiz app — other services stay untouched.
              </p>
            )}
            {postizUpgrading && upgradeStatus && (
              <p className="text-xs text-blue-600 mt-1">{upgradeStatus}</p>
            )}
          </div>
          <div className="flex gap-2">
            {postizUpdate?.update_available ? (
              <Button
                onClick={handleUpgradePostiz}
                loading={postizUpgrading}
              >
                Upgrade Postiz
              </Button>
            ) : (
              <Button
                variant="secondary"
                onClick={handleCheckPostizUpdate}
                loading={postizChecking}
              >
                Check for Updates
              </Button>
            )}
          </div>
        </div>
        {postizUpdate?.update_available && (
          <div className="mt-3 pt-3 border-t border-gray-100 space-y-2">
            <div className="rounded bg-amber-50 px-3 py-2">
              <p className="text-xs text-amber-700">
                <strong>Before upgrading:</strong> Consider exporting a full backup using the button below. The upgrade will pull the latest Postiz app image and recreate only the Postiz container — databases and other services are not restarted. If the new version fails health checks, it will automatically roll back to the previous image.
              </p>
            </div>
          </div>
        )}
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
          {!snapshot?.tunnel_alive && snapshot?.tunnel_mode === "permanent" && snapshot?.permanent_domain && (
            <CopyField value={snapshot.permanent_domain} label="Custom Domain" />
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
                Connected platforms
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
              Web link mode:{" "}
              <span className="font-medium">
                {snapshot?.tunnel_mode === "temporary"
                  ? "Temporary tunnel"
                  : snapshot?.tunnel_mode === "permanent"
                    ? "Custom domain"
                    : snapshot?.tunnel_mode === "none"
                      ? "Local only"
                      : "Unknown"}
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

        {snapshot?.tunnel_mode === "temporary" && !snapshot?.tunnel_alive && (
          <Button
            variant="secondary"
            onClick={handleReconnectTunnelClick}
            loading={reconnecting}
          >
            <Globe className="h-4 w-4" />
            Reconnect Web Link{tunnelProvider ? ` (${tunnelProvider})` : ""}
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

        <Button
          variant="secondary"
          onClick={() => setShowExport(!showExport)}
        >
          <Archive className="h-4 w-4" />
          Export Full Backup
        </Button>

        <Button variant="ghost" onClick={handleMinimizeToTray}>
          Minimize to Tray
        </Button>
      </div>

      {showReconnectDialog && (
        <Card className="mb-6">
          <h4 className="text-sm font-medium text-gray-700 mb-2">
            {tunnelProvider === "ngrok" ? "ngrok Authtoken" : "Pinggy Token"} (optional)
          </h4>
          <p className="text-xs text-gray-500 mb-3">
            Provide your {tunnelProvider === "ngrok" ? "ngrok authtoken" : "Pinggy token"} to reconnect, or leave blank to attempt without it.
            {" "}
            {tunnelProvider === "ngrok" ? (
              <button onClick={() => open("https://ngrok.com/signup")} className="text-blue-600 hover:text-blue-700 underline">Get one at ngrok.com</button>
            ) : (
              <button onClick={() => open("https://pinggy.io")} className="text-blue-600 hover:text-blue-700 underline">Get one at pinggy.io</button>
            )}
          </p>
          <input
            type="password"
            className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-400 focus:outline-none focus:ring-1 focus:ring-blue-400 mb-3"
            placeholder={tunnelProvider === "ngrok" ? "ngrok authtoken" : "Pinggy token"}
            value={reconnectConfig}
            onChange={(e) => setReconnectConfig(e.target.value)}
          />
          <div className="flex items-center gap-2">
            <Button variant="primary" onClick={handleReconnectTunnel}>
              Reconnect
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                setShowReconnectDialog(false);
                setReconnectConfig("");
              }}
            >
              Cancel
            </Button>
          </div>
        </Card>
      )}

      {showExport && (
        <div className="mb-6">
          <ExportCloneDialog onClose={() => setShowExport(false)} />
        </div>
      )}

      {/* Docker logs */}
      <CollapsiblePanel title="Service Logs">
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
