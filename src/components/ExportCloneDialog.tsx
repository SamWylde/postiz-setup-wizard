import { useState, useEffect } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { useWizardStore } from "../store/wizardStore";
import {
  exportClone,
  onTransferProgress,
  type TransferProgress,
} from "../lib/tauri";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { StatusIndicator } from "./ui/StatusIndicator";
import { showToast } from "./ui/Toast";
import { Archive, X } from "lucide-react";

interface ExportCloneDialogProps {
  onClose: () => void;
}

const PHASE_LABELS: Record<string, string> = {
  discovering: "Discovering volumes...",
  stopping: "Stopping services...",
  backing_up: "Backing up data...",
  encrypting: "Encrypting archive...",
  restarting: "Restarting services...",
  complete: "Export complete!",
};

export function ExportCloneDialog({ onClose }: ExportCloneDialogProps) {
  const { installPath } = useWizardStore();

  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [exporting, setExporting] = useState(false);
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [outputFile, setOutputFile] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const passwordsMatch = password === confirmPassword;
  const canExport = password.length >= 4 && passwordsMatch && !exporting;

  useEffect(() => {
    const unlisten = onTransferProgress((e) => setProgress(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const handleExport = async () => {
    setError(null);
    setOutputFile(null);

    const savePath = await save({
      title: "Save backup file",
      defaultPath: `postiz-backup-${new Date().toISOString().slice(0, 10)}.postizclone`,
      filters: [{ name: "Postiz Clone", extensions: ["postizclone"] }],
    });

    if (!savePath) return;

    setExporting(true);
    try {
      const result = await exportClone(installPath, password, savePath);
      setOutputFile(result);
      showToast("Backup exported successfully", "success");
    } catch (err) {
      setError(String(err));
      showToast("Export failed", "error");
    } finally {
      setExporting(false);
    }
  };

  if (outputFile) {
    return (
      <div className="rounded-lg border border-green-200 bg-green-50 p-4">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-medium text-green-900">
            Backup exported successfully
          </h3>
          <button onClick={onClose} className="text-green-600 hover:text-green-700">
            <X className="h-4 w-4" />
          </button>
        </div>
        <p className="text-sm text-green-700 font-mono break-all">{outputFile}</p>
        <p className="text-xs text-green-600 mt-2">
          Transfer this file to your new machine and use "Import from backup" to restore.
        </p>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-gray-200 bg-gray-50 p-4">
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <Archive className="h-4 w-4 text-gray-500" />
          <h3 className="text-sm font-medium text-gray-700">
            Export Full Backup
          </h3>
        </div>
        {!exporting && (
          <button onClick={onClose} className="text-gray-400 hover:text-gray-500">
            <X className="h-4 w-4" />
          </button>
        )}
      </div>

      <p className="text-xs text-gray-500 mb-3">
        Creates an encrypted backup of your entire Postiz installation including
        database, uploads, and configuration. Services will be briefly stopped
        during export.
      </p>

      {!exporting ? (
        <div className="space-y-3">
          <Input
            label="Encryption password"
            secret
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="At least 4 characters"
          />
          <Input
            label="Confirm password"
            secret
            value={confirmPassword}
            onChange={(e) => setConfirmPassword(e.target.value)}
            placeholder="Type password again"
          />
          {password.length > 0 && confirmPassword.length > 0 && !passwordsMatch && (
            <p className="text-xs text-red-600">Passwords do not match.</p>
          )}
          {error && (
            <div className="rounded-lg bg-red-50 p-3">
              <p className="text-sm text-red-600">{error}</p>
            </div>
          )}
          <Button onClick={handleExport} disabled={!canExport}>
            Choose Location & Export
          </Button>
        </div>
      ) : (
        <div className="space-y-2">
          <StatusIndicator
            status="loading"
            label={
              progress
                ? PHASE_LABELS[progress.phase] ?? progress.message
                : "Starting export..."
            }
            detail={
              progress?.current != null && progress?.total != null
                ? `${progress.current}/${progress.total}`
                : undefined
            }
          />
        </div>
      )}
    </div>
  );
}
