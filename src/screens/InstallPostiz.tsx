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
} from "../lib/tauri";
import { friendlyError } from "../lib/errors";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { FolderOpen } from "lucide-react";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { CollapsiblePanel } from "../components/ui/CollapsiblePanel";
import { LogViewer } from "../components/ui/LogViewer";
import { NavigationButtons } from "../components/wizard/NavigationButtons";

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m === 0) return `${s}s`;
  return `${m}m ${s}s`;
}

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

  const [statusMessage, setStatusMessage] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const isInstalling = ["preparing", "pulling", "starting"].includes(
    installStatus,
  );
  const isComplete = installStatus === "running";

  useEffect(() => {
    if (!installPath) {
      getDefaultInstallPath().then(setInstallPath).catch(console.error);
    }
  }, []);

  useEffect(() => {
    const unlistenProgress = onDockerProgress((e) =>
      setStatusMessage(e.payload),
    );
    const unlistenLog = onDockerLog((e) => addDockerLog(e.payload));

    return () => {
      unlistenProgress.then((f) => f());
      unlistenLog.then((f) => f());
    };
  }, []);

  // Elapsed time timer during install
  useEffect(() => {
    if (isInstalling) {
      setElapsed(0);
      timerRef.current = setInterval(() => setElapsed((e) => e + 1), 1000);
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
    setStatusMessage("Running pre-flight checks...");

    try {
      // Run preflight validation — allow existing files at the install path since this
      // screen is the only place installs are initiated, and retries after failure will
      // have committed files from a prior attempt already in place.
      const preflight = await validatePreflight(installPath, port, tunnelMode, true);
      if (!preflight.ok) {
        const failures = preflight.checks
          .filter((c) => !c.passed)
          .map((c) => c.message)
          .join("\n");
        setInstallStatus("error");
        setInstallError(failures);
        setStatusMessage("Pre-flight checks failed.");
        return;
      }

      setStatusMessage("Preparing files...");

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
      setStatusMessage("Downloading Postiz (this may take several minutes)...");

      await startStack(installPath);

      setInstallStatus("starting");
      setStatusMessage("Waiting for services to start...");

      let attempts = 0;
      const maxAttempts = 60;
      while (attempts < maxAttempts) {
        await new Promise((r) => setTimeout(r, 3000));
        try {
          const status = await getStackStatus(installPath);
          if (status.all_healthy && status.postiz_responding) {
            setInstallStatus("running");
            setPostizReady(true);
            setStatusMessage("Postiz is installed and running!");
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
      setStatusMessage("Installation timed out.");
    } catch (err) {
      setInstallStatus("error");
      setInstallError(friendlyError(String(err)));
      setStatusMessage("Installation failed.");
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
    setStatusMessage("");
  };

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

            <div className="mt-4">
              <Button
                onClick={handleInstall}
                disabled={showAdvanced && (port < 1024 || port > 65535)}
              >
                Install Postiz
              </Button>
            </div>
          </>
        ) : (
          <div className="space-y-4">
            <StatusIndicator
              status={
                installStatus === "error"
                  ? "error"
                  : isComplete
                    ? "success"
                    : "loading"
              }
              label={statusMessage}
              detail={isInstalling ? formatElapsed(elapsed) : undefined}
            />

            {installStatus === "pulling" && (
              <div className="rounded-lg bg-blue-50 p-3">
                <p className="text-xs text-blue-700">
                  Downloading Docker images for Postiz, PostgreSQL, Redis,
                  Temporal, and Elasticsearch. This typically takes 3-10 minutes
                  depending on your internet speed.
                </p>
              </div>
            )}

            {installError && (
              <div className="rounded-lg bg-red-50 p-3">
                <p className="text-sm text-red-700 font-medium mb-1">
                  Something went wrong
                </p>
                <p className="text-sm text-red-600">{installError}</p>
              </div>
            )}

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
                    // Clean up any staged temp files, but committed files stay (allow_existing handles them)
                    await cleanStagedFiles(installPath).catch(() => {});
                    setInstallStatus("idle");
                    setInstallError(null);
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
