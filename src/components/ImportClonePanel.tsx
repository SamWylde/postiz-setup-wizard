import { useState, useEffect } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  validateCloneFile,
  importClone,
  getDefaultInstallPath,
  onTransferProgress,
  saveResumeState,
  type CloneManifest,
  type TransferProgress,
} from "../lib/tauri";
import { useWizardStore } from "../store/wizardStore";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { Card } from "./ui/Card";
import { StatusIndicator } from "./ui/StatusIndicator";
import { showToast } from "./ui/Toast";
import { FolderOpen, ArrowLeft, Upload } from "lucide-react";

interface ImportClonePanelProps {
  onCancel: () => void;
}

const PHASE_LABELS: Record<string, string> = {
  decrypting: "Decrypting archive...",
  extracting: "Extracting configuration...",
  pulling: "Pulling Docker images (this may take several minutes)...",
  restoring: "Restoring data...",
  starting: "Starting services...",
  health_check: "Waiting for services to become healthy...",
  complete: "Import complete!",
};

export function ImportClonePanel({ onCancel }: ImportClonePanelProps) {
  const {
    setStep,
    setInstallPath,
    setPort,
    setTunnelMode,
    setTunnelProvider,
    setProviderStatus,
    setTransferReviewPending,
  } = useWizardStore();

  const [clonePath, setClonePath] = useState("");
  const [password, setPassword] = useState("");
  const [manifest, setManifest] = useState<CloneManifest | null>(null);
  const [installPath, setLocalInstallPath] = useState("");
  const [customPort, setCustomPort] = useState<number | undefined>(undefined);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const [validating, setValidating] = useState(false);
  const [importing, setImporting] = useState(false);
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getDefaultInstallPath().then(setLocalInstallPath).catch(() => {});
  }, []);

  useEffect(() => {
    const unlisten = onTransferProgress((e) => setProgress(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const handlePickFile = async () => {
    const selected = await openDialog({
      title: "Select backup file",
      filters: [{ name: "Postiz Clone", extensions: ["postizclone"] }],
    });
    if (selected) {
      setClonePath(selected as string);
      setManifest(null);
      setError(null);
    }
  };

  const handleValidate = async () => {
    if (!clonePath || !password) return;
    setValidating(true);
    setError(null);
    try {
      const m = await validateCloneFile(clonePath, password);
      setManifest(m);
    } catch (err) {
      setError(String(err));
    } finally {
      setValidating(false);
    }
  };

  const handleImport = async () => {
    setImporting(true);
    setError(null);
    try {
      const result = await importClone(clonePath, password, installPath, customPort);
      // Parse the actual port from backend response
      let actualPort: number;
      try {
        actualPort = JSON.parse(result).port as number;
        if (!actualPort || actualPort < 1) throw new Error("missing port");
      } catch {
        throw new Error("Import succeeded but returned an unexpected response. Please check Docker status.");
      }
      // Update store state
      setInstallPath(installPath);
      setPort(actualPort);
      setTransferReviewPending(true);
      // Hydrate tunnel and provider state from the imported manifest
      if (manifest) {
        if (manifest.tunnel_mode) {
          setTunnelMode(manifest.tunnel_mode as "temporary" | "permanent" | "none");
        }
        if (manifest.tunnel_provider) {
          setTunnelProvider(manifest.tunnel_provider as "cloudflared" | "ngrok" | "zrok" | "pinggy");
        }
        // Mark all imported providers as stale — user needs to update redirect URLs
        for (const id of manifest.providers_configured) {
          setProviderStatus(id, "stale");
        }
      }
      await saveResumeState().catch(() => {});
      showToast("Import complete! Set up your web link to continue.", "success");
      setStep(3); // CreateWebLink
    } catch (err) {
      setError(String(err));
      showToast("Import failed", "error");
      // Keep importing=true so the error shows in the progress view with retry button.
      // The retry button sets importing=false to return to the form.
    }
  };

  if (importing) {
    return (
      <div className="space-y-4">
        <h3 className="text-lg font-medium text-gray-900">Importing backup...</h3>
        <StatusIndicator
          status="loading"
          label={
            progress
              ? PHASE_LABELS[progress.phase] ?? progress.message
              : "Starting import..."
          }
          detail={
            progress?.current != null && progress?.total != null
              ? `${progress.current}/${progress.total}`
              : undefined
          }
        />
        {error && (
          <div className="rounded-lg bg-red-50 p-3">
            <p className="text-sm text-red-700 font-medium mb-1">Import failed</p>
            <p className="text-sm text-red-600">{error}</p>
            <Button
              variant="secondary"
              className="mt-2"
              onClick={() => {
                setImporting(false);
                setError(null);
                setProgress(null);
              }}
            >
              Try Again
            </Button>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <button onClick={onCancel} className="text-gray-400 hover:text-gray-600">
          <ArrowLeft className="h-4 w-4" />
        </button>
        <h3 className="text-lg font-medium text-gray-900">Import from Backup</h3>
      </div>

      {/* Step 1: Pick file */}
      <Card>
        <label className="block text-sm font-medium text-gray-700 mb-1">
          Backup file
        </label>
        <div className="flex gap-2">
          <div className="flex-1">
            <Input
              value={clonePath}
              onChange={(e) => setClonePath(e.target.value)}
              placeholder="Select a .postizclone file"
              readOnly
            />
          </div>
          <Button variant="secondary" onClick={handlePickFile}>
            <Upload className="h-4 w-4" />
            Browse
          </Button>
        </div>

        {clonePath && (
          <div className="mt-3 space-y-3">
            <Input
              label="Backup password"
              secret
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="Enter the encryption password"
            />
            {!manifest && (
              <Button
                onClick={handleValidate}
                loading={validating}
                disabled={password.length < 8}
              >
                Verify Backup
              </Button>
            )}
          </div>
        )}
      </Card>

      {error && !importing && (
        <div className="rounded-lg bg-red-50 p-3">
          <p className="text-sm text-red-600">{error}</p>
        </div>
      )}

      {/* Step 2: Manifest preview */}
      {manifest && (
        <Card>
          <h4 className="text-sm font-medium text-gray-700 mb-2">
            Backup Details
          </h4>
          <div className="space-y-1 text-sm text-gray-600">
            <p>
              Source: <span className="font-medium">{manifest.source_hostname}</span>
            </p>
            <p>
              Created:{" "}
              <span className="font-medium">
                {new Date(manifest.created_at).toLocaleDateString()}
              </span>
            </p>
            <p>
              Wizard version:{" "}
              <span className="font-mono">{manifest.wizard_version}</span>
            </p>
            {manifest.providers_configured.length > 0 && (
              <p>
                Providers:{" "}
                <span className="font-medium capitalize">
                  {manifest.providers_configured.join(", ")}
                </span>
              </p>
            )}
            <p>
              Volumes: <span className="font-medium">{manifest.volumes.length}</span>
            </p>
          </div>
        </Card>
      )}

      {/* Step 3: Install path + Import */}
      {manifest && (
        <Card>
          <label className="block text-sm font-medium text-gray-700 mb-1">
            Install location
          </label>
          <div className="flex gap-2">
            <div className="flex-1">
              <Input
                value={installPath}
                onChange={(e) => setLocalInstallPath(e.target.value)}
              />
            </div>
            <Button
              variant="secondary"
              onClick={async () => {
                const selected = await openDialog({
                  directory: true,
                  title: "Choose install folder",
                });
                if (selected) setLocalInstallPath(selected as string);
              }}
            >
              <FolderOpen className="h-4 w-4" />
              Browse
            </Button>
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
                label="Custom port (optional)"
                type="number"
                value={customPort ?? ""}
                onChange={(e) => {
                  const v = e.target.value ? Number(e.target.value) : undefined;
                  setCustomPort(v);
                }}
                placeholder="Default: auto-detect from 4007"
              />
            </div>
          )}

          <div className="mt-4">
            <Button onClick={handleImport} disabled={!installPath}>
              Import & Restore
            </Button>
          </div>
        </Card>
      )}
    </div>
  );
}
