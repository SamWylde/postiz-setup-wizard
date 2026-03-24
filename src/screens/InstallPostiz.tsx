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
import { FolderOpen } from "lucide-react";
import { InstallTimeline, type InstallPhase } from "../components/ui/InstallTimeline";
import { CollapsiblePanel } from "../components/ui/CollapsiblePanel";
import { LogViewer } from "../components/ui/LogViewer";
import { NavigationButtons } from "../components/wizard/NavigationButtons";
import { ImportClonePanel } from "../components/ImportClonePanel";

const PRE_HEALTH_WAIT_MS = 5_000;
const HEALTH_POLL_INTERVAL_MS = 3_000;
const MAX_HEALTH_ATTEMPTS = 60;
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
  const [elapsed, setElapsed] = useState(0);
  const [installPhase, setInstallPhaseState] = useState<InstallPhase>("idle");
  const [errorPhase, setErrorPhase] = useState<InstallPhase | null>(null);
  const installPhaseRef = useRef<InstallPhase>("idle");
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const mountedRef = useRef(true);

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
      if (!preflight.ok) {
        const failures = preflight.checks
          .filter((c) => !c.passed)
          .map((c) => c.message)
          .join("\n");
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

      setInstallStatus("starting");
      setInstallPhase("starting-services");

      // Brief pause to let containers initialize before polling health
      await new Promise((r) => setTimeout(r, PRE_HEALTH_WAIT_MS));
      setInstallPhase("health-checks");

      let attempts = 0;
      while (mountedRef.current && attempts < MAX_HEALTH_ATTEMPTS) {
        await new Promise((r) => setTimeout(r, HEALTH_POLL_INTERVAL_MS));
        try {
          const status = await getStackStatus(installPath);
          if (status.all_healthy && status.postiz_responding) {
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

      setInstallStatus("error");
      setInstallError(
        "Services started but Postiz didn't become healthy within 3 minutes. Check Docker Desktop for container issues, then try again.",
      );
      setErrorPhase("health-checks");
    } catch (err) {
      setInstallStatus("error");
      setInstallError(friendlyError(String(err)));
      setErrorPhase(installPhaseRef.current);
    }
  };

  const handleCancel = async () => {
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
        {installStatus === "idle" ? (
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
                {port > 0 && port < 1024 && (
                  <p className="mt-1 text-xs text-amber-600">
                    Ports below 1024 are reserved. Choose a port between 1024 and 65535.
                  </p>
                )}
                {(port === 0) && (
                  <p className="mt-1 text-xs text-amber-600">
                    Port 0 is not valid. Choose a port between 1024 and 65535.
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
              {installStatus === "pulling" && (
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
