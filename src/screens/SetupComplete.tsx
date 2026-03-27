import { useState, useEffect, useRef } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  getInstallSnapshot,
  exportDiagnostics,
  restartAndVerify,
  clearTransferReviewAndSave,
  savePostizCredentials,
  getPostizCredentials,
  saveResumeState,
  type InstallSnapshot,
} from "../lib/tauri";
import { open } from "@tauri-apps/plugin-shell";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { CopyField } from "../components/ui/CopyField";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { showToast } from "../components/ui/Toast";
import { ExportCloneDialog } from "../components/ExportCloneDialog";
import {
  ExternalLink,
  PartyPopper,
  ArrowRight,
  Download,
  Archive,
  RefreshCw,
} from "lucide-react";

export function SetupComplete() {
  const { port, installPath, tunnelUrl, tunnelMode, permanentDomain, providers, setStep, transferReviewPending, setTransferReviewPending } =
    useWizardStore();

  const [snapshot, setSnapshot] = useState<InstallSnapshot | null>(null);
  const [checking, setChecking] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [showExport, setShowExport] = useState(false);
  const [credEmail, setCredEmail] = useState("");
  const [credPassword, setCredPassword] = useState("");
  const [credsSaved, setCredsSaved] = useState(false);
  const credsLoaded = useRef(false);

  // Load saved credentials on mount
  useEffect(() => {
    if (credsLoaded.current) return;
    credsLoaded.current = true;
    getPostizCredentials().then(([email, password]) => {
      setCredEmail(email ?? "");
      setCredPassword(password ?? "");
      setCredsSaved(Boolean(email && password));
    }).catch(() => {});
  }, []);

  // Clear transfer review flag and persist atomically in one backend call
  useEffect(() => {
    if (transferReviewPending) {
      clearTransferReviewAndSave()
        .then(() => setTransferReviewPending(false))
        .catch(() => {});
    }
  }, [transferReviewPending, setTransferReviewPending]);

  const localUrl = `http://localhost:${port}`;
  const publicUrl = tunnelUrl ?? (
    (tunnelMode === "permanent" || tunnelMode === "local_https") && permanentDomain
      ? permanentDomain
      : null
  );

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
    const latestSnapshot = snapshot ?? await getInstallSnapshot().catch(() => null);
    if (latestSnapshot && snapshot == null) {
      setSnapshot(latestSnapshot);
    }

    // Open the tunnel URL when active — Postiz sets session cookies based on
    // FRONTEND_URL's domain, so the browser must access via the same URL.
    // Falls back to localhost when a Cloudflare tunnel is disconnected.
    const url = latestSnapshot?.tunnel_alive && latestSnapshot.tunnel_url
      ? latestSnapshot.tunnel_url
      : (latestSnapshot?.web_link_kind === "manual" || latestSnapshot?.web_link_kind === "local_https") && latestSnapshot?.permanent_domain
        ? latestSnapshot.permanent_domain
        : localUrl;
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

  const webLinkNeedsAttention =
    snapshot?.web_link_kind === "cloudflare" && !snapshot.tunnel_alive;
  const overallStatus =
    snapshot == null
      ? "loading"
      : !snapshot.all_healthy || !snapshot.postiz_responding
        ? "error"
        : webLinkNeedsAttention
          ? "warning"
          : "success";
  const publicUrlLabel =
    snapshot?.tunnel_alive
      ? "Public URL"
      : snapshot?.web_link_kind === "local_https"
        ? "Local HTTPS URL"
      : snapshot?.web_link_kind === "manual"
        ? "Custom Domain"
        : tunnelMode === "permanent"
          ? "Configured public URL"
          : "Public URL";

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
                    snapshot.tunnel_alive
                      ? "success"
                    : snapshot.web_link_kind === "local_https" && snapshot.permanent_domain
                      ? "success"
                    : snapshot.web_link_kind === "manual" && snapshot.permanent_domain
                      ? "success"
                      : snapshot.web_link_kind === "cloudflare"
                        ? "warning"
                      : configuredProviders.length > 0
                        ? "success"
                        : "warning"
                }
                label="Web link"
                detail={
                  snapshot.tunnel_alive
                    ? snapshot.tunnel_url ?? "Active"
                    : snapshot.web_link_kind === "local_https" && snapshot.permanent_domain
                      ? `Local HTTPS domain: ${snapshot.permanent_domain}`
                    : snapshot.web_link_kind === "manual" && snapshot.permanent_domain
                      ? `Custom domain: ${snapshot.permanent_domain}`
                      : snapshot.web_link_kind === "cloudflare"
                        ? snapshot.permanent_domain
                          ? `Cloudflare tunnel disconnected (${snapshot.permanent_domain})`
                          : "Cloudflare tunnel disconnected"
                      : configuredProviders.length > 0
                        ? "Not needed — accounts use saved tokens"
                        : "Not connected"
                }
              />
            )}
            <Button
              variant="ghost"
              className="mt-2"
              onClick={() => setStep(3)}
            >
              Manage Web Link
            </Button>
            <div className="pt-2 mt-2 border-t border-gray-100">
              <StatusIndicator
                status={overallStatus}
                label="Overall"
                detail={
                  overallStatus === "success"
                    ? "Everything is working!"
                    : overallStatus === "warning"
                      ? "Working locally, but your public web link needs attention"
                      : "Some issues need attention"
                }
              />
            </div>
            {overallStatus === "warning" && (
              <p className="pt-3 text-xs text-amber-700">
                Reconnect or change your public web link before adding or reconnecting social accounts.
              </p>
            )}
            {overallStatus === "error" && (
              <div className="flex items-center gap-2 pt-3">
                <Button
                  variant="secondary"
                  onClick={async () => {
                    setRestarting(true);
                    try {
                      await restartAndVerify(installPath);
                      const snap = await getInstallSnapshot();
                      setSnapshot(snap);
                      showToast("Services restarted", "success");
                    } catch (err) {
                      showToast(`Restart failed: ${String(err)}`, "error");
                    } finally {
                      setRestarting(false);
                    }
                  }}
                  loading={restarting}
                >
                  <RefreshCw className="h-4 w-4" />
                  Restart Services
                </Button>
                <Button variant="ghost" onClick={handleVerify} loading={checking}>
                  Re-check
                </Button>
                <button
                  onClick={() => setStep(6)}
                  className="text-sm text-blue-600 hover:text-blue-700 ml-2"
                >
                  View Details
                </button>
              </div>
            )}
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
            <CopyField value={publicUrl} label={publicUrlLabel} />
          )}
          <CopyField value={localUrl} label="Local URL" />
          <div>
            <p className="text-sm font-medium text-gray-700 mb-1">
              Install location
            </p>
            <p className="text-sm text-gray-600 font-mono">{installPath}</p>
          </div>
          <div>
            <p className="text-sm font-medium text-gray-700 mb-1">
              Connected platforms
            </p>
            {configuredProviders.length > 0 ? (
              <p className="text-sm text-gray-600 capitalize">
                {configuredProviders.join(", ")}
              </p>
            ) : (
              <p className="text-sm text-gray-500">None configured yet</p>
            )}
            <button
              onClick={() => setStep(4)}
              className="mt-2 text-sm text-blue-600 hover:text-blue-700 font-medium"
            >
              {configuredProviders.length > 0 ? "Connect more platforms" : "Connect social platforms"}
            </button>
          </div>
        </div>
      </Card>

      {/* Next steps — tell users how to actually connect accounts in Postiz */}
      {configuredProviders.length > 0 && (
        <Card className="mb-6 bg-amber-50 border-amber-200">
          <h3 className="text-sm font-medium text-amber-900 mb-3">
            Next: Connect your accounts in Postiz
          </h3>
          <p className="text-sm text-amber-800 mb-3">
            The wizard saved your API credentials. Now you need to link your
            actual social media accounts inside Postiz:
          </p>
          <ol className="space-y-2 text-sm text-amber-800 list-decimal list-inside">
            <li>
              Click <strong>"Open Postiz"</strong> below and sign in with the
              account you already created
            </li>
            <li>
              Go to <strong>Settings</strong> (gear icon) then look for <strong>Channels</strong> or <strong>Integrations</strong>
            </li>
            <li>
              Click <strong>"Connect"</strong> on each platform you configured
              ({configuredProviders.map(p => p.charAt(0).toUpperCase() + p.slice(1)).join(", ")})
            </li>
            <li>
              Authorize the app when redirected to the social media site
            </li>
          </ol>
          {tunnelMode === "temporary" && (
            <p className="text-xs text-amber-700 mt-3">
              Keep the tunnel running while you do this — the social media sites
              redirect back through it. Once all accounts are linked, the tunnel
              is no longer needed.
            </p>
          )}
        </Card>
      )}

      {/* Postiz login credentials — save for easy copy-paste */}
      <Card className="mb-6">
        <h3 className="text-sm font-medium text-gray-700 mb-3">
          Postiz Login Credentials
        </h3>
        {credsSaved ? (
          <div className="space-y-3">
            <CopyField value={credEmail} label="Email" />
            <CopyField value={credPassword} label="Password" secret />
            <button
              onClick={() => setCredsSaved(false)}
              className="text-xs text-blue-600 hover:text-blue-700"
            >
              Update credentials
            </button>
          </div>
        ) : (
          <div className="space-y-3">
            <p className="text-sm text-gray-600">
              Save your Postiz login here so you can easily copy it later,
              for example if your web link changes. It's stored encrypted on
              this computer.
            </p>
            <input
              type="email"
              className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-400 focus:outline-none focus:ring-1 focus:ring-blue-400"
              placeholder="Email"
              value={credEmail}
              onChange={(e) => setCredEmail(e.target.value)}
            />
            <input
              type="password"
              className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-400 focus:outline-none focus:ring-1 focus:ring-blue-400"
              placeholder="Password"
              value={credPassword}
              onChange={(e) => setCredPassword(e.target.value)}
            />
            <Button
              variant="secondary"
              disabled={!credEmail.trim() || !credPassword.trim()}
              onClick={async () => {
                await savePostizCredentials(credEmail.trim(), credPassword.trim());
                await saveResumeState();
                setCredsSaved(true);
                showToast("Credentials saved securely on this computer", "success");
              }}
            >
              Save Credentials
            </Button>
          </div>
        )}
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
              Once you've linked your accounts in Postiz (see steps above),
              the tunnel is no longer needed — Postiz uses saved tokens after
              that. You'll only need the tunnel again to add or reconnect an
              account.
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
        <Button
          variant="ghost"
          onClick={() => setShowExport(!showExport)}
        >
          <Archive className="h-4 w-4" />
          Export Full Backup
        </Button>
      </div>

      {showExport && (
        <div className="mb-6">
          <ExportCloneDialog onClose={() => setShowExport(false)} />
        </div>
      )}

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
