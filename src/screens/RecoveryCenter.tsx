import { useEffect, useRef, useState } from "react";
import {
  getInstallSnapshot,
  restartAndVerify,
  cancelDockerOperation,
  stopStack,
  cleanStagedFiles,
  onDockerProgress,
  type InstallSnapshot,
} from "../lib/tauri";
import { useWizardStore } from "../store/wizardStore";
import { open } from "@tauri-apps/plugin-shell";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { showToast } from "../components/ui/Toast";
import {
  ArrowRight,
  Wrench,
  Globe,
  FolderOpen,
  Trash2,
  RotateCcw,
  AlertTriangle,
  Loader2,
} from "lucide-react";

interface RecoveryCenterProps {
  snapshot: InstallSnapshot;
  onResumeWizard: () => void;
}

export function RecoveryCenter({
  snapshot: initialSnapshot,
  onResumeWizard,
}: RecoveryCenterProps) {
  const { setStep } = useWizardStore();
  const [snapshot, setSnapshot] = useState<InstallSnapshot>(initialSnapshot);
  const [repairing, setRepairing] = useState(false);
  const [repairProgress, setRepairProgress] = useState("");
  const [repairElapsed, setRepairElapsed] = useState(0);
  const repairTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [cleaningStaged, setCleaningStaged] = useState(false);
  const [showCleanConfirm, setShowCleanConfirm] = useState(false);
  const [showRebuildConfirm, setShowRebuildConfirm] = useState(false);
  const [stopping, setStopping] = useState(false);
  const displayedStep = Math.min(snapshot.current_step, 5) + 1;

  const installPath = snapshot.install_path ?? "";

  const refreshSnapshot = async () => {
    try {
      const updated = await getInstallSnapshot();
      setSnapshot(updated);
    } catch {
      // silently ignore refresh errors
    }
  };

  const handleResume = () => {
    setStep(snapshot.current_step);
    onResumeWizard();
  };

  // Listen for docker-progress events during repair
  useEffect(() => {
    if (!repairing) return;
    const unlisten = onDockerProgress((e) => setRepairProgress(e.payload));
    return () => {
      unlisten.then((f) => f()).catch(() => {});
    };
  }, [repairing]);

  const handleRepair = async () => {
    setRepairing(true);
    setRepairProgress("");
    setRepairElapsed(0);
    repairTimerRef.current = setInterval(
      () => setRepairElapsed((e) => e + 1),
      1000,
    );
    try {
      await restartAndVerify(installPath);
      await refreshSnapshot();
      showToast("Services repaired successfully", "success");
    } catch (err) {
      showToast(`Repair failed: ${String(err)}`, "error");
    } finally {
      setRepairing(false);
      if (repairTimerRef.current) {
        clearInterval(repairTimerRef.current);
        repairTimerRef.current = null;
      }
    }
  };

  const handleCancelRepair = async () => {
    try {
      await cancelDockerOperation();
    } catch {
      // Best effort
    }
  };

  const fmtElapsed = (s: number) => {
    const m = Math.floor(s / 60);
    return m === 0 ? `${s % 60}s` : `${m}m ${s % 60}s`;
  };

  const handleOpenFolder = async () => {
    try {
      await open(installPath);
    } catch (err) {
      showToast(`Could not open folder: ${String(err)}`, "error");
    }
  };

  const handleCleanStaged = async () => {
    setCleaningStaged(true);
    try {
      await cleanStagedFiles(installPath);
      await refreshSnapshot();
      showToast("Staged files removed", "success");
    } catch (err) {
      showToast(`Cleanup failed: ${String(err)}`, "error");
    } finally {
      setCleaningStaged(false);
      setShowCleanConfirm(false);
    }
  };

  const handleCleanRebuild = async () => {
    setStopping(true);
    try {
      await stopStack(installPath);
      showToast(
        "Containers stopped. Please manually delete the install folder, then restart the wizard.",
        "info",
      );
    } catch (err) {
      showToast(`Stop failed: ${String(err)}`, "error");
    } finally {
      setStopping(false);
      setShowRebuildConfirm(false);
    }
  };

  return (
    <div className="max-w-3xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        Recovery Center
      </h2>
      <p className="text-gray-600 mb-6">
        We found an existing Postiz installation. The easiest option is to
        resume where you left off.
      </p>

      {/* Snapshot status card */}
      <Card className="mb-6">
        <h3 className="text-sm font-medium text-gray-700 mb-3">
          Current State
        </h3>
        <div className="space-y-1">
          <StatusIndicator
            status={
              snapshot.docker_running
                ? snapshot.all_healthy
                  ? "success"
                  : "warning"
                : "error"
            }
            label="Docker"
            detail={
              !snapshot.docker_installed
                ? "Not installed"
                : !snapshot.docker_running
                  ? "Not running"
                  : snapshot.all_healthy
                    ? "All containers healthy"
                    : "Some containers unhealthy"
            }
          />
          <StatusIndicator
            status={snapshot.postiz_responding ? "success" : "error"}
            label="Postiz Application"
            detail={snapshot.postiz_responding ? "Responding" : "Not responding"}
          />
          <StatusIndicator
            status={
              snapshot.tunnel_alive
                ? "success"
                : snapshot.web_link_kind === "manual" && snapshot.permanent_domain
                  ? "success"
                  : snapshot.web_link_kind === "cloudflare"
                    ? "warning"
                  : snapshot.tunnel_mode === "none"
                    ? "warning"
                  : "error"
            }
            label="Web Link"
            detail={
              snapshot.tunnel_alive
                ? snapshot.tunnel_url ?? "Active"
                : snapshot.web_link_kind === "manual" && snapshot.permanent_domain
                  ? `Custom domain: ${snapshot.permanent_domain}`
                  : snapshot.web_link_kind === "cloudflare"
                    ? snapshot.permanent_domain
                      ? `Cloudflare tunnel disconnected (${snapshot.permanent_domain})`
                      : "Cloudflare tunnel disconnected"
                  : snapshot.tunnel_mode === "none"
                    ? "Local-only mode (no social platform connections)"
                    : "Disconnected"
            }
          />
        </div>
        {snapshot.last_error && (
          <div className="mt-3 flex items-start gap-2 rounded-lg bg-red-50 border border-red-200 p-3">
            <AlertTriangle className="h-4 w-4 text-red-500 shrink-0 mt-0.5" />
            <p className="text-sm text-red-700">{snapshot.last_error}</p>
          </div>
        )}
      </Card>

      {/* Recommended action */}
      <Card className="mb-6 border-blue-200 bg-blue-50/50">
        <div className="flex items-start gap-3 mb-4">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-blue-100">
            <ArrowRight className="h-5 w-5 text-blue-600" />
          </div>
          <div className="min-w-0">
            <h4 className="text-sm font-semibold text-gray-900">
              Continue Setup
            </h4>
            <p className="text-xs text-gray-500 mt-0.5">
              Pick up where you left off (step {displayedStep} of 6)
            </p>
          </div>
        </div>
        <Button variant="primary" onClick={handleResume}>
          Continue
        </Button>
      </Card>

      {/* Other actions */}
      <h3 className="text-sm font-medium text-gray-700 mb-3">
        Troubleshooting
      </h3>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
        {/* Repair Services */}
        <Card className="flex flex-col">
          <div className="flex items-start gap-3 mb-4">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-amber-100">
              {repairing ? (
                <Loader2 className="h-5 w-5 text-amber-600 animate-spin" />
              ) : (
                <Wrench className="h-5 w-5 text-amber-600" />
              )}
            </div>
            <div className="min-w-0">
              <h4 className="text-sm font-semibold text-gray-900">
                {repairing ? "Restarting Services..." : "Restart Services"}
              </h4>
              {repairing ? (
                <p className="text-xs text-blue-600 mt-0.5">
                  {fmtElapsed(repairElapsed)}
                  {repairProgress ? ` — ${repairProgress}` : ""}
                </p>
              ) : (
                <p className="text-xs text-gray-500 mt-0.5">
                  Restart Postiz if it's not responding
                </p>
              )}
            </div>
          </div>
          <div className="mt-auto flex gap-2">
            {repairing ? (
              <Button variant="secondary" onClick={handleCancelRepair}>
                Cancel
              </Button>
            ) : (
              <Button
                variant="secondary"
                onClick={handleRepair}
                disabled={!installPath}
              >
                Restart
              </Button>
            )}
          </div>
        </Card>

        {/* Manage Web Link */}
        <Card className="flex flex-col">
          <div className="flex items-start gap-3 mb-4">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-purple-100">
              <Globe className="h-5 w-5 text-purple-600" />
            </div>
            <div className="min-w-0">
              <h4 className="text-sm font-semibold text-gray-900">
                Manage Web Link
              </h4>
              <p className="text-xs text-gray-500 mt-0.5">
                Set up or change the public URL for social platforms
              </p>
            </div>
          </div>
          <div className="mt-auto">
            <Button
              variant="secondary"
              onClick={() => { setStep(3); onResumeWizard(); }}
            >
              Manage
            </Button>
          </div>
        </Card>

        {/* Open Install Folder */}
        <Card className="flex flex-col">
          <div className="flex items-start gap-3 mb-4">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-gray-100">
              <FolderOpen className="h-5 w-5 text-gray-600" />
            </div>
            <div className="min-w-0">
              <h4 className="text-sm font-semibold text-gray-900">
                Open Install Folder
              </h4>
              <p className="text-xs text-gray-500 mt-0.5">
                View Postiz files on your computer
              </p>
            </div>
          </div>
          <div className="mt-auto">
            <Button
              variant="secondary"
              onClick={handleOpenFolder}
              disabled={!installPath}
            >
              Open
            </Button>
          </div>
        </Card>

        {/* Clean up incomplete changes */}
        <Card className="flex flex-col">
          <div className="flex items-start gap-3 mb-4">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-orange-100">
              <Trash2 className="h-5 w-5 text-orange-600" />
            </div>
            <div className="min-w-0">
              <h4 className="text-sm font-semibold text-gray-900">
                Clean Up Temp Files
              </h4>
              <p className="text-xs text-gray-500 mt-0.5">
                Remove leftover files from an interrupted setup
              </p>
            </div>
          </div>
          <div className="mt-auto">
            {!showCleanConfirm ? (
              <Button
                variant="secondary"
                onClick={() => setShowCleanConfirm(true)}
                disabled={!snapshot.has_staged_temp}
              >
                Clean Up
              </Button>
            ) : (
              <div className="flex items-center gap-2">
                <Button
                  variant="primary"
                  onClick={handleCleanStaged}
                  loading={cleaningStaged}
                >
                  Confirm
                </Button>
                <Button
                  variant="ghost"
                  onClick={() => setShowCleanConfirm(false)}
                >
                  Cancel
                </Button>
              </div>
            )}
          </div>
        </Card>

        {/* Start Over */}
        <Card className="flex flex-col">
          <div className="flex items-start gap-3 mb-4">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-red-100">
              <RotateCcw className="h-5 w-5 text-red-600" />
            </div>
            <div className="min-w-0">
              <h4 className="text-sm font-semibold text-gray-900">
                Start Over
              </h4>
              <p className="text-xs text-gray-500 mt-0.5">
                Stop Postiz and remove everything to start fresh
              </p>
            </div>
          </div>
          <div className="mt-auto">
            {!showRebuildConfirm ? (
              <Button
                variant="secondary"
                onClick={() => setShowRebuildConfirm(true)}
              >
                Start Over
              </Button>
            ) : (
              <div className="space-y-2">
                <p className="text-xs text-red-600 font-medium">
                  This will stop Postiz and all its services. You'll need to
                  delete the install folder yourself, then run setup again.
                </p>
                <div className="flex items-center gap-2">
                  <Button
                    variant="primary"
                    onClick={handleCleanRebuild}
                    loading={stopping}
                  >
                    Yes, Stop Everything
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => setShowRebuildConfirm(false)}
                  >
                    Cancel
                  </Button>
                </div>
              </div>
            )}
          </div>
        </Card>
      </div>
    </div>
  );
}
