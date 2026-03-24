import { useEffect, useState, useRef } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  getDefaultInstallPath,
  prepareInstall,
  commitInstall,
  startStack,
  getStackStatus,
  cancelInstall,
  cleanStagedFiles,
  validatePreflight,
  saveResumeState,
  wipeExistingInstall,
  restartAndVerify,
  onDockerProgress,
  onDockerLog,
  onDockerPullProgress,
  type PullProgress,
} from "../lib/tauri";
import { friendlyError } from "../lib/errors";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { FolderOpen, AlertTriangle, Trash2, Wrench, FolderOpen as FolderIcon } from "lucide-react";
import { InstallTimeline, type InstallPhase } from "../components/ui/InstallTimeline";
import { CollapsiblePanel } from "../components/ui/CollapsiblePanel";
import { LogViewer } from "../components/ui/LogViewer";
import { NavigationButtons } from "../components/wizard/NavigationButtons";
import { ImportClonePanel } from "../components/ImportClonePanel";
import { showToast } from "../components/ui/Toast";

const PRE_HEALTH_WAIT_MS = 5_000;
const HEALTH_POLL_INTERVAL_MS = 3_000;
const MAX_HEALTH_ATTEMPTS = 80;
const ELAPSED_TICK_MS = 1_000;

export function InstallPostiz() {
  const {
    installPath,
    setInstallPath,
    installStatus,
    setInstallStatus,
    setInstallError,
    installError,
    port,
    setPort,
    tunnelMode,
    dockerLogs,
    addDockerLog,
    setStep,
    setPostizReady,
  } = useWizardStore();

  const [progressDetail, setProgressDetail] = useState("");
  const [pullProgress, setPullProgress] = useState<PullProgress | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [existingInstallDetected, setExistingInstallDetected] = useState(false);
  const [wipingInstall, setWipingInstall] = useState(false);
  const [repairingInstall, setRepairingInstall] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [installPhase, setInstallPhaseState] = useState<InstallPhase>("idle");
  const [errorPhase, setErrorPhase] = useState<InstallPhase | null>(null);
  const installPhaseRef = useRef<InstallPhase>("idle");
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const mountedRef = useRef(true);
  const cancelledRef = useRef(false);

  const setInstallPhase = (phase: InstallPhase) => {
    installPhaseRef.current = phase;
    setInstallPhaseState(phase);
  };
  const isInstalling = ["preparing", "pulling", "starting"].includes(
    installStatus,
  );
  const isComplete = installStatus === "running";

  useEffect(() => {
    mountedRef.current = true;
    if (!installPath) {
      getDefaultInstallPath().then(setInstallPath).catch(console.error);
    }
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    const unlistenProgress = onDockerProgress((e) =>
      setProgressDetail(e.payload),
    );
    const unlistenLog = onDockerLog((e) => addDockerLog(e.payload));
    const unlistenPull = onDockerPullProgress((e) =>
      setPullProgress(e.payload),
    );

    return () => {
      unlistenProgress.then((f) => f()).catch(() => {});
      unlistenLog.then((f) => f()).catch(() => {});
      unlistenPull.then((f) => f()).catch(() => {});
    };
  }, []);

  // Elapsed time timer during install
  useEffect(() => {
    if (isInstalling) {
      setElapsed(0);
      timerRef.current = setInterval(() => setElapsed((e) => e + 1), ELAPSED_TICK_MS);
    } else {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [isInstalling]);

  const handleInstall = async () => {
    cancelledRef.current = false;
    setInstallStatus("preparing");
    setInstallError(null);
    setErrorPhase(null);
    setPullProgress(null);
    setInstallPhase("preflight");

    try {
      // Run preflight validation in strict mode — the backend will only allow
      // existing files at the install path when a .tmp staging folder proves
      // this wizard created the partial state (i.e. a failed install retry).
      const preflight = await validatePreflight(installPath, port, tunnelMode, false);
      if (cancelledRef.current) return;
      if (!preflight.ok) {
        const failedChecks = preflight.checks.filter((c) => !c.passed);
        const hasExistingInstall = failedChecks.some((c) =>
          c.message.includes("existing Postiz install was found"),
        );
        if (hasExistingInstall) {
          setInstallStatus("idle");
          setInstallPhase("idle");
          setExistingInstallDetected(true);
          return;
        }
        const failures = failedChecks.map((c) => c.message).join("\n");
        setInstallStatus("error");
        setInstallError(failures);
        setErrorPhase("preflight");
        return;
      }

      setInstallPhase("preparing-files");

      // prepareInstall returns the actual port used (may differ from requested if default)
      const actualPort = await prepareInstall(
        installPath,
        port !== 4007 ? port : undefined,
      );
      if (cancelledRef.current) return;
      if (actualPort !== port) {
        setPort(actualPort);
      }

      await commitInstall(installPath);

      // Persist resume state immediately so the install is discoverable if the app
      // crashes during docker pull or health polling
      await saveResumeState().catch(() => {});

      setInstallStatus("pulling");
      setInstallPhase("pulling");

      await startStack(installPath);
      if (cancelledRef.current) return;

      setInstallStatus("starting");
      setInstallPhase("starting-services");

      // Brief pause to let containers initialize before polling health
      await new Promise((r) => setTimeout(r, PRE_HEALTH_WAIT_MS));
      if (cancelledRef.current) return;
      setInstallPhase("health-checks");

      // Core services that must be running for Postiz to function.
      // Auxiliary services (temporal-*) are for background jobs and can
      // recover via restart: always without blocking the install wizard.
      const CORE_CONTAINERS = ["postiz", "postiz-postgres", "postiz-redis"];

      let attempts = 0;
      while (mountedRef.current && !cancelledRef.current && attempts < MAX_HEALTH_ATTEMPTS) {
        await new Promise((r) => setTimeout(r, HEALTH_POLL_INTERVAL_MS));
        if (cancelledRef.current) return;
        try {
          const status = await getStackStatus(installPath);

          // Show per-container status during polling
          if (status.containers.length > 0) {
            const running = status.containers.filter((c) => c.state === "running").length;
            const total = status.containers.length;
            const deadCore = status.containers.filter(
              (c) =>
                CORE_CONTAINERS.includes(c.name) &&
                (c.state === "exited" || c.state === "dead"),
            );

            if (deadCore.length > 0) {
              // Core service crashed — fail immediately, no point waiting
              setInstallStatus("error");
              setInstallError(
                `${deadCore.map((c) => c.name).join(", ")} crashed. Open Docker Desktop to check logs, then try again.`,
              );
              setErrorPhase("health-checks");
              return;
            }

            setProgressDetail(
              status.postiz_responding
                ? "Postiz is responding — finalizing..."
                : `${running}/${total} containers running`,
            );
          }

          // Primary gate: Postiz responds to HTTP.
          // Auxiliary containers (temporal-*) may still be starting via
          // restart: always — they don't block the wizard.
          if (status.postiz_responding) {
            setInstallStatus("running");
            setPostizReady(true);
            setInstallPhase("ready");
            await saveResumeState().catch(() => {});
            return;
          }
        } catch {
          // Keep polling
        }
        attempts++;
      }

      if (cancelledRef.current) return;
      setInstallStatus("error");
      setInstallError(
        "Postiz didn't respond within 4 minutes. Check Docker Desktop for container issues, then try again.",
      );
      setErrorPhase("health-checks");
    } catch (err) {
      if (cancelledRef.current) return;
      setInstallStatus("error");
      setInstallError(friendlyError(String(err)));
      setErrorPhase(installPhaseRef.current);
    }
  };

  const handleCancel = async () => {
    cancelledRef.current = true;
    try {
      await cancelInstall();
    } catch {
      // Best effort
    }
    setInstallStatus("idle");
    setInstallError(null);
    setInstallPhase("idle");
    setErrorPhase(null);
    setPullProgress(null);
  };

  if (showImport) {
    return (
      <div className="max-w-2xl">
        <h2 className="text-2xl font-semibold text-gray-900 mb-2">
          Install Postiz
        </h2>
        <p className="text-gray-600 mb-6">
          Restore from a backup created on another machine.
        </p>
        <Card className="mb-6">
          <ImportClonePanel onCancel={() => setShowImport(false)} />
        </Card>
      </div>
    );
  }

  return (
    <div className="max-w-2xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        Install Postiz
      </h2>
      <p className="text-gray-600 mb-6">
        We'll download and set up Postiz on your computer using Docker.
      </p>

      <Card className="mb-6">
        {installStatus === "idle" && existingInstallDetected ? (
          <ExistingInstallPanel
            installPath={installPath}
            wipingInstall={wipingInstall}
            repairingInstall={repairingInstall}
            onWipe={async () => {
              setWipingInstall(true);
              try {
                await wipeExistingInstall(installPath);
                showToast("Old install removed. You can now reinstall.", "success");
                setExistingInstallDetected(false);
              } catch (err) {
                showToast(friendlyError(String(err)), "error");
              } finally {
                setWipingInstall(false);
              }
            }}
            onRepair={async () => {
              setRepairingInstall(true);
              try {
                await restartAndVerify(installPath);
                showToast("Repair successful! Services are running.", "success");
                setPostizReady(true);
                setInstallStatus("running");
                setInstallPhase("ready");
              } catch (err) {
                showToast(friendlyError(String(err)), "error");
              } finally {
                setRepairingInstall(false);
              }
            }}
            onChangePath={() => setExistingInstallDetected(false)}
            onImport={() => {
              setExistingInstallDetected(false);
              setShowImport(true);
            }}
          />
        ) : installStatus === "idle" ? (
          <>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Install location{" "}
                <span className="font-normal text-gray-400">(recommended)</span>
              </label>
              <div className="flex gap-2">
                <div className="flex-1">
                  <Input
                    value={installPath}
                    onChange={(e) => setInstallPath(e.target.value)}
                  />
                </div>
                <Button
                  variant="secondary"
                  onClick={async () => {
                    const selected = await openDialog({
                      directory: true,
                      title: "Choose install folder",
                    });
                    if (selected) setInstallPath(selected as string);
                  }}
                >
                  <FolderOpen className="h-4 w-4" />
                  Browse
                </Button>
              </div>
              <p className="mt-1 text-xs text-gray-500">
                Postiz needs about 3 GB of disk space. Configuration files and
                data will be stored in this folder.
              </p>
            </div>

            <button
              onClick={() => setShowAdvanced(!showAdvanced)}
              className="mt-3 text-sm text-blue-600 hover:text-blue-700"
            >
              {showAdvanced ? "Hide" : "Show"} advanced options
            </button>

            {showAdvanced && (
              <div className="mt-3">
                <Input
                  label="Port"
                  type="number"
                  value={port}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    if (v >= 0 && v <= 65535) setPort(v);
                  }}
                />
                {port < 1024 && (
                  <p className="mt-1 text-xs text-amber-600">
                    {port === 0 ? "Port 0 is not valid." : "Ports below 1024 are reserved."} Choose a port between 1024 and 65535.
                  </p>
                )}
                <p className="mt-1 text-xs text-gray-500">
                  Default: 4007. Will auto-select next free port if occupied.
                </p>
              </div>
            )}

            <div className="mt-4 space-y-2">
              <Button
                onClick={handleInstall}
                disabled={port === 0 || port < 1024 || port > 65535}
              >
                Install Postiz
              </Button>
              <div>
                <button
                  onClick={() => setShowImport(true)}
                  className="text-sm text-blue-600 hover:text-blue-700"
                >
                  Or import from a backup
                </button>
              </div>
            </div>
          </>
        ) : (
          <div className="space-y-4">
            <InstallTimeline
              currentPhase={installPhase}
              errorPhase={errorPhase}
              elapsed={elapsed}
              progressDetail={progressDetail}
              errorMessage={installError}
              pullProgress={pullProgress}
            />

            <div className="flex gap-2">
              {(installStatus === "pulling" || installStatus === "starting") && (
                <Button variant="secondary" onClick={handleCancel}>
                  Cancel
                </Button>
              )}
              {installStatus === "error" && (
                <Button
                  variant="secondary"
                  onClick={async () => {
                    await cleanStagedFiles(installPath).catch(() => {});
                    setInstallStatus("idle");
                    setInstallError(null);
                    setInstallPhase("idle");
                    setErrorPhase(null);
                    setPullProgress(null);
                  }}
                >
                  Try again
                </Button>
              )}
            </div>
          </div>
        )}
      </Card>

      {dockerLogs.length > 0 && (
        <CollapsiblePanel title="Technical details">
          <LogViewer logs={dockerLogs} />
        </CollapsiblePanel>
      )}

      <NavigationButtons
        canProceed={isComplete}
        loading={isInstalling}
        onNext={() => setStep(2)}
      />
    </div>
  );
}

function ExistingInstallPanel({
  installPath,
  wipingInstall,
  repairingInstall,
  onWipe,
  onRepair,
  onChangePath,
  onImport,
}: {
  installPath: string;
  wipingInstall: boolean;
  repairingInstall: boolean;
  onWipe: () => void;
  onRepair: () => void;
  onChangePath: () => void;
  onImport: () => void;
}) {
  const [confirmWipe, setConfirmWipe] = useState(false);

  return (
    <div className="space-y-4">
      <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-4">
        <AlertTriangle className="h-5 w-5 text-amber-500 mt-0.5 shrink-0" />
        <div>
          <p className="text-sm font-medium text-amber-800">
            Existing install found
          </p>
          <p className="text-sm text-amber-700 mt-1">
            A previous Postiz installation was found at{" "}
            <span className="font-mono text-xs bg-amber-100 px-1 py-0.5 rounded">
              {installPath}
            </span>
            . What would you like to do?
          </p>
        </div>
      </div>

      <div className="space-y-2">
        <button
          onClick={onRepair}
          disabled={wipingInstall || repairingInstall}
          className="w-full flex items-center gap-3 rounded-lg border border-gray-200 p-3 text-left hover:bg-gray-50 transition-colors disabled:opacity-50"
        >
          <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-blue-100 shrink-0">
            <Wrench className="h-4 w-4 text-blue-600" />
          </div>
          <div>
            <p className="text-sm font-medium text-gray-900">
              {repairingInstall ? "Repairing..." : "Try to repair"}
            </p>
            <p className="text-xs text-gray-500">
              Restart Docker containers and verify the existing install is working.
            </p>
          </div>
        </button>

        {!confirmWipe ? (
          <button
            onClick={() => setConfirmWipe(true)}
            disabled={wipingInstall || repairingInstall}
            className="w-full flex items-center gap-3 rounded-lg border border-gray-200 p-3 text-left hover:bg-gray-50 transition-colors disabled:opacity-50"
          >
            <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-red-100 shrink-0">
              <Trash2 className="h-4 w-4 text-red-600" />
            </div>
            <div>
              <p className="text-sm font-medium text-gray-900">
                Delete and reinstall
              </p>
              <p className="text-xs text-gray-500">
                Remove the old install files and Docker volumes, then start fresh.
              </p>
            </div>
          </button>
        ) : (
          <div className="rounded-lg border border-red-200 bg-red-50 p-3 space-y-2">
            <p className="text-sm text-red-800 font-medium">
              Are you sure? This will permanently delete all Postiz data including your database, uploaded files, and configuration.
            </p>
            <div className="flex gap-2">
              <Button
                variant="secondary"
                onClick={() => setConfirmWipe(false)}
                disabled={wipingInstall}
              >
                Cancel
              </Button>
              <button
                onClick={onWipe}
                disabled={wipingInstall}
                className="rounded-lg bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-50"
              >
                {wipingInstall ? "Deleting..." : "Yes, delete everything"}
              </button>
            </div>
          </div>
        )}

        <button
          onClick={onImport}
          disabled={wipingInstall || repairingInstall}
          className="w-full flex items-center gap-3 rounded-lg border border-gray-200 p-3 text-left hover:bg-gray-50 transition-colors disabled:opacity-50"
        >
          <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-green-100 shrink-0">
            <FolderIcon className="h-4 w-4 text-green-600" />
          </div>
          <div>
            <p className="text-sm font-medium text-gray-900">
              Import from backup
            </p>
            <p className="text-xs text-gray-500">
              Restore from a clone file created on this or another machine.
            </p>
          </div>
        </button>
      </div>

      <button
        onClick={onChangePath}
        className="text-sm text-blue-600 hover:text-blue-700"
      >
        Or choose a different install path
      </button>
    </div>
  );
}
