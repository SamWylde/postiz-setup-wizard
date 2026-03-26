import { useEffect, useState } from "react";
import { useWizardStore } from "../store/wizardStore";
import { scanMachine, runBootstrap, type BootstrapAction } from "../lib/tauri";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { CollapsiblePanel } from "../components/ui/CollapsiblePanel";
import { NavigationButtons } from "../components/wizard/NavigationButtons";

export function PrepareComputer() {
  const { machineState, setMachineState, bootstrapStatus, setBootstrapStatus, setStep } =
    useWizardStore();
  const [actionLog, setActionLog] = useState<string[]>([]);
  const [currentAction, setCurrentAction] = useState<string | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [wslNeedsRestart, setWslNeedsRestart] = useState(false);

  const checkMachine = async () => {
    setBootstrapStatus("checking");
    setScanError(null);
    setWslNeedsRestart(false);
    try {
      const state = await scanMachine();
      setMachineState(state);

      // cloudflared is optional — only needed for temporary tunnel mode, checked later in preflight
      const allGood =
        state.docker_installed &&
        state.docker_running &&
        state.docker_linux_mode &&
        state.disk_space_gb >= 3 &&
        state.ram_available_gb >= 2;

      setBootstrapStatus(allGood ? "ready" : "action-needed");
    } catch (err) {
      setMachineState(null);
      setScanError(String(err));
      setActionLog((prev) => [...prev, `Scan failed: ${err}`]);
      setBootstrapStatus("action-needed");
    }
  };

  useEffect(() => {
    checkMachine();
  }, []);

  /** Run a bootstrap action. Returns true on success, false on failure. */
  const runAction = async (action: BootstrapAction, label: string): Promise<boolean> => {
    setCurrentAction(label);
    setActionLog((prev) => [...prev, `${label}...`]);
    try {
      const result = await runBootstrap(action);
      setActionLog((prev) => [...prev, result]);
      setCurrentAction(null);
      return true;
    } catch (err) {
      setActionLog((prev) => [...prev, `Error: ${err}`]);
      setCurrentAction(null);
      return false;
    }
  };

  const handleBootstrap = async () => {
    if (!machineState) return;

    if (!machineState.wsl2_installed) {
      const ok = await runAction("InstallWsl2", "Installing WSL2");
      if (ok) {
        // WSL2 install succeeded — rescan to verify
        await checkMachine();
        // If still not detected after rescan, a restart is likely needed
        const updated = useWizardStore.getState().machineState;
        if (updated && !updated.wsl2_installed) {
          setWslNeedsRestart(true);
          setActionLog((prev) => [
            ...prev,
            "WSL2 was installed but isn't active yet. A system restart may be required.",
          ]);
        }
      }
      return;
    }

    if (!machineState.docker_installed) {
      await runAction("InstallDocker", "Installing Docker Desktop");
      // Re-check after install
      await checkMachine();
      return;
    }

    if (!machineState.docker_running) {
      await runAction("StartDocker", "Starting Docker Desktop");
      // Wait and re-check
      await new Promise((r) => setTimeout(r, 10000));
      await checkMachine();
      return;
    }

    if (!machineState.docker_linux_mode) {
      await runAction("SwitchLinuxContainers", "Switching to Linux containers");
      await new Promise((r) => setTimeout(r, 5000));
      await checkMachine();
      return;
    }

    // cloudflared is optional at this stage — install it opportunistically
    // but don't block setup if it fails. It's checked again in preflight
    // if the user chooses temporary tunnel mode.
    if (!machineState.cloudflared_installed) {
      await runAction("InstallCloudflared", "Installing web link tool");
      await checkMachine();
      return;
    }
  };

  const getStatus = (ok: boolean) =>
    ok
      ? ("success" as const)
      : bootstrapStatus === "checking"
        ? ("loading" as const)
        : ("error" as const);

  const allReady = bootstrapStatus === "ready";

  return (
    <div className="max-w-2xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        Prepare your computer
      </h2>
      <p className="text-gray-600 mb-6">
        We'll check that everything needed to run Postiz is installed on your
        machine.
      </p>

      <Card className="mb-6">
        <div className="space-y-1">
          {machineState ? (
            <>
              <StatusIndicator
                status={
                  machineState.wsl2_installed
                    ? "success"
                    : wslNeedsRestart
                      ? "warning"
                      : getStatus(false)
                }
                label="Linux compatibility layer"
                detail={
                  machineState.wsl2_installed
                    ? "Ready"
                    : wslNeedsRestart
                      ? "Installed — restart your computer to activate WSL2"
                      : "Needs install (WSL2)"
                }
              />
              <StatusIndicator
                status={getStatus(machineState.docker_installed)}
                label="Docker Desktop"
                detail={
                  machineState.docker_installed ? "Installed" : "Needs install"
                }
              />
              <StatusIndicator
                status={getStatus(machineState.docker_running)}
                label="Docker Desktop running"
                detail={machineState.docker_running ? "Running" : "Needs to be started"}
              />
              <StatusIndicator
                status={getStatus(machineState.docker_linux_mode)}
                label="Docker container mode"
                detail={
                  machineState.docker_linux_mode
                    ? "Correctly set to Linux"
                    : "Needs to switch to Linux mode"
                }
              />
              <StatusIndicator
                status={
                  machineState.cloudflared_installed
                    ? "success"
                    : bootstrapStatus === "checking"
                      ? "loading"
                      : "warning"
                }
                label="Cloudflared (optional)"
                detail={
                  machineState.cloudflared_installed
                    ? "cloudflared installed"
                    : "Not installed — needed for Cloudflare Zero Trust tunnels"
                }
              />
              <StatusIndicator
                status={getStatus(machineState.disk_space_gb >= 3)}
                label="Disk space"
                detail={`${machineState.disk_space_gb.toFixed(1)} GB available (need 3 GB)`}
              />
              <StatusIndicator
                status={getStatus(machineState.ram_available_gb >= 2)}
                label="Memory (RAM)"
                detail={`${machineState.ram_available_gb.toFixed(1)} GB available (need 2 GB)`}
              />
            </>
          ) : scanError && bootstrapStatus !== "checking" ? (
            <>
              <StatusIndicator
                status="error"
                label="System scan failed"
                detail={scanError}
              />
              <div className="mt-3">
                <Button variant="secondary" onClick={checkMachine}>
                  Retry scan
                </Button>
              </div>
            </>
          ) : (
            <StatusIndicator status="loading" label="Checking system..." />
          )}
        </div>

        {bootstrapStatus === "action-needed" && machineState && (
          <div className="mt-4 pt-4 border-t border-gray-200 space-y-3">
            <Button
              onClick={handleBootstrap}
              loading={currentAction !== null}
            >
              {currentAction ?? "Set up this computer"}
            </Button>
            {wslNeedsRestart && (
              <Button variant="secondary" onClick={() => { setWslNeedsRestart(false); checkMachine(); }}>
                I've restarted — re-check now
              </Button>
            )}
          </div>
        )}

      </Card>

      {actionLog.length > 0 && (
        <CollapsiblePanel title="Technical details">
          <div className="space-y-1 font-mono text-xs text-gray-600">
            {actionLog.map((line, i) => (
              <div key={i}>{line}</div>
            ))}
          </div>
        </CollapsiblePanel>
      )}

      <NavigationButtons
        canProceed={allReady}
        onNext={() => setStep(1)}
      />
    </div>
  );
}
