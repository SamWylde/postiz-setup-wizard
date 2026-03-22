import { useEffect, useState } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  startTunnel,
  updateBaseUrls,
  restartAndVerify,
  saveResumeState,
  runBootstrap,
  scanMachine,
  onTunnelUrl,
  onTunnelStatus,
  type TunnelProvider,
  type BootstrapAction,
} from "../lib/tauri";
import { friendlyError } from "../lib/errors";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { CopyField } from "../components/ui/CopyField";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { NavigationButtons } from "../components/wizard/NavigationButtons";
import { AlertTriangle, Globe, Link, Monitor, ChevronDown, Download } from "lucide-react";

type TunnelMode = "temporary" | "permanent" | "none";

interface ProviderOption {
  id: TunnelProvider;
  name: string;
  description: string;
  installed: boolean;
  installAction: BootstrapAction | null;
}

const PROVIDER_WARNINGS: Record<TunnelProvider, string> = {
  cloudflared: "This link changes every time you restart this app.",
  ngrok: "Free tier has request limits. Upgrade for a stable URL.",
  zrok: "Free share URLs are ephemeral. Use `zrok reserve` for stable URLs.",
  pinggy: "Free tier URL changes on restart. Use a token for stability.",
};

export function CreateWebLink() {
  const {
    port,
    installPath,
    tunnelStatus,
    setTunnelStatus,
    tunnelUrl,
    setTunnelUrl,
    tunnelMode,
    setTunnelMode,
    tunnelProvider,
    setTunnelProvider,
    permanentDomain,
    setPermanentDomain,
    setRemoteReachable,
    setStep,
    machineState,
    setMachineState,
  } = useWizardStore();
  const [statusMessage, setStatusMessage] = useState("");
  const [domainApplied, setDomainApplied] = useState(false);
  const [domainReachable, setDomainReachable] = useState(true);
  const [applyingDomain, setApplyingDomain] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [providerConfig, setProviderConfig] = useState("");
  const [installing, setInstalling] = useState<string | null>(null);

  // Rescan machine state on mount so provider availability is up to date
  // (e.g. after resume or recovery when machineState may be null)
  useEffect(() => {
    if (!machineState) {
      scanMachine()
        .then((state) => setMachineState(state))
        .catch(() => {});
    }
  }, []);

  useEffect(() => {
    const unlistenUrl = onTunnelUrl((e) => {
      setTunnelUrl(e.payload);
    });
    const unlistenStatus = onTunnelStatus((e) => {
      if (e.payload === "running") setTunnelStatus("running");
      if (e.payload === "stopped") setTunnelStatus("error");
    });

    return () => {
      unlistenUrl.then((f) => f());
      unlistenStatus.then((f) => f());
    };
  }, []);

  const providerOptions: ProviderOption[] = [
    {
      id: "cloudflared",
      name: "Cloudflare",
      description: "No account needed. URL changes on restart.",
      installed: machineState?.cloudflared_installed ?? false,
      installAction: "InstallCloudflared",
    },
    {
      id: "ngrok",
      name: "ngrok",
      description: "Free account for static URLs.",
      installed: machineState?.ngrok_installed ?? false,
      installAction: "InstallNgrok",
    },
    {
      id: "zrok",
      name: "zrok",
      description: "Open-source. Free stable URLs.",
      installed: machineState?.zrok_installed ?? false,
      installAction: "InstallZrok",
    },
    {
      id: "pinggy",
      name: "Pinggy",
      description: "No install needed (uses SSH).",
      installed: machineState?.ssh_available ?? true,
      installAction: null,
    },
  ];

  const selectedProvider = providerOptions.find((p) => p.id === tunnelProvider);

  const handleInstallProvider = async (opt: ProviderOption) => {
    if (!opt.installAction) return;
    setInstalling(opt.id);
    try {
      await runBootstrap(opt.installAction);
    } catch {
      // Error will be visible on re-scan
    }
    // Re-scan machine state so the freshly installed provider shows as available
    try {
      const state = await scanMachine();
      setMachineState(state);
    } catch {
      // Ignore scan errors
    }
    setInstalling(null);
  };

  const handleStartTunnel = async () => {
    setTunnelMode("temporary");
    setTunnelStatus("starting");
    setStatusMessage("Starting secure tunnel...");

    try {
      const config = providerConfig.trim() || undefined;
      const url = await startTunnel(port, tunnelProvider, config);
      setTunnelUrl(url);
      setTunnelStatus("restarting");
      setStatusMessage("Updating Postiz configuration...");

      await updateBaseUrls(installPath, url);
      await restartAndVerify(installPath);

      setStatusMessage("Verifying remote access...");

      await new Promise((r) => setTimeout(r, 10000));

      try {
        await fetch(url, {
          mode: "no-cors",
          signal: AbortSignal.timeout(10000),
        });
        setRemoteReachable(true);
        setTunnelStatus("running");
        setStatusMessage("Web link is active!");
      } catch {
        setRemoteReachable(false);
        setTunnelStatus("running");
        setStatusMessage(
          "Web link created but remote verification failed. The link may still work — social platforms will connect directly.",
        );
      }
    } catch (err) {
      setTunnelStatus("error");
      setStatusMessage(friendlyError(String(err)));
    }
  };

  const handleApplyDomain = async () => {
    const raw = permanentDomain.trim();
    if (!raw) return;

    // Validate URL format
    const domain = raw.replace(/\/+$/, "");
    if (!domain.startsWith("https://")) {
      setStatusMessage("Domain must start with https:// — social platforms require HTTPS.");
      return;
    }
    try {
      const parsed = new URL(domain);
      if (!parsed.hostname.includes(".")) {
        setStatusMessage("Please enter a valid domain (e.g. https://postiz.example.com).");
        return;
      }
    } catch {
      setStatusMessage("Invalid URL format. Enter a full URL like https://postiz.example.com");
      return;
    }

    setApplyingDomain(true);
    setStatusMessage("Applying domain configuration...");

    try {
      await updateBaseUrls(installPath, domain);
      await restartAndVerify(installPath);
      await saveResumeState();

      setTunnelUrl(domain);
      setTunnelStatus("running");

      // Verify the public URL is actually reachable via HTTPS
      setStatusMessage("Verifying public URL...");
      let reachable = false;
      try {
        await fetch(domain, {
          mode: "no-cors",
          signal: AbortSignal.timeout(10000),
        });
        reachable = true;
        setStatusMessage("Domain applied and verified!");
      } catch {
        setStatusMessage(
          "Domain applied, but could not reach it publicly. Verify DNS and HTTPS are configured correctly — social platform callbacks may not work until this is resolved.",
        );
      }
      setDomainReachable(reachable);
      setDomainApplied(true);
    } catch (err) {
      setStatusMessage(friendlyError(String(err)));
    } finally {
      setApplyingDomain(false);
    }
  };

  const handleChooseLocalOnly = () => {
    setTunnelMode("none");
  };

  const isActive = tunnelStatus === "running" && tunnelUrl;

  const canProceed =
    tunnelMode === "none" ||
    (tunnelMode === "permanent" && domainApplied) ||
    (tunnelMode === "temporary" && !!isActive);

  return (
    <div className="max-w-2xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        Create a web link
      </h2>
      <p className="text-gray-600 mb-6">
        Social media platforms need a public URL to communicate with Postiz.
        Choose a tunnel provider and we'll create one automatically.
      </p>

      {/* Provider selector + primary action */}
      {tunnelStatus === "idle" && tunnelMode !== "permanent" && tunnelMode !== "none" && (
        <>
          <Card className="mb-4">
            <div className="flex items-center gap-2 mb-3">
              <Globe className="h-4 w-4 text-blue-600" />
              <h3 className="text-sm font-medium text-gray-900">
                Choose a tunnel provider
              </h3>
            </div>
            <div className="grid grid-cols-2 gap-3">
              {providerOptions.map((opt) => (
                <button
                  key={opt.id}
                  onClick={() => {
                    if (opt.installed) {
                      setTunnelProvider(opt.id);
                      setProviderConfig("");
                    }
                  }}
                  disabled={!opt.installed && installing !== null}
                  className={`relative text-left rounded-lg border-2 p-3 transition-colors ${
                    tunnelProvider === opt.id
                      ? "border-blue-500 bg-blue-50"
                      : opt.installed
                        ? "border-gray-200 hover:border-gray-300 bg-white"
                        : "border-gray-100 bg-gray-50 opacity-75"
                  }`}
                >
                  <p className="text-sm font-medium text-gray-900">{opt.name}</p>
                  <p className="text-xs text-gray-500 mt-0.5">{opt.description}</p>
                  {!opt.installed && opt.installAction && (
                    <Button
                      variant="secondary"
                      className="mt-2 text-xs"
                      loading={installing === opt.id}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleInstallProvider(opt);
                      }}
                    >
                      <Download className="h-3 w-3" />
                      Install
                    </Button>
                  )}
                </button>
              ))}
            </div>
          </Card>

          {/* Per-provider config inputs */}
          {tunnelProvider === "ngrok" && (
            <Card className="mb-4">
              <Input
                label="ngrok authtoken (optional)"
                value={providerConfig}
                onChange={(e) => setProviderConfig(e.target.value)}
                placeholder="Paste your ngrok authtoken for a stable URL"
                secret
              />
              <p className="text-xs text-gray-500 mt-1">
                Get a free authtoken at ngrok.com/signup
              </p>
            </Card>
          )}
          {tunnelProvider === "zrok" && (
            <Card className="mb-4">
              <p className="text-sm text-gray-700 mb-2">
                <strong>First time using zrok?</strong> You need to create a free account
                and enable your environment before sharing:
              </p>
              <ol className="text-sm text-gray-600 list-decimal list-inside space-y-1 mb-3">
                <li>Sign up at <span className="font-mono text-blue-600">zrok.io</span> and copy your enable token</li>
                <li>Open a terminal and run: <code className="bg-gray-100 px-1 py-0.5 rounded text-xs">zrok enable YOUR_TOKEN</code></li>
              </ol>
              <p className="text-xs text-gray-500">
                You only need to do this once. After enabling, zrok will remember your account.
              </p>
            </Card>
          )}
          {tunnelProvider === "pinggy" && (
            <Card className="mb-4">
              <Input
                label="Pinggy token (optional)"
                value={providerConfig}
                onChange={(e) => setProviderConfig(e.target.value)}
                placeholder="Paste your Pinggy token for a stable URL"
                secret
              />
              <p className="text-xs text-gray-500 mt-1">
                Get a token at pinggy.io for a persistent URL
              </p>
            </Card>
          )}

          {/* Start button */}
          <Card className="mb-6">
            <Button
              onClick={handleStartTunnel}
              disabled={!selectedProvider?.installed}
            >
              <Globe className="h-4 w-4" />
              Create Web Link
              {selectedProvider && ` with ${selectedProvider.name}`}
            </Button>
          </Card>
        </>
      )}

      {/* Tunnel in progress / active */}
      {tunnelMode === "temporary" && tunnelStatus !== "idle" && (
        <Card className="mb-6">
          <div className="space-y-4">
            <StatusIndicator
              status={
                isActive
                  ? "success"
                  : tunnelStatus === "error"
                    ? "error"
                    : "loading"
              }
              label={statusMessage}
            />

            {tunnelUrl && (
              <CopyField value={tunnelUrl} label="Your web link" />
            )}

            {tunnelStatus === "error" && (
              <Button variant="secondary" onClick={handleStartTunnel}>
                Try again
              </Button>
            )}
          </div>
        </Card>
      )}

      {/* Provider-specific temporary mode warning */}
      {tunnelMode === "temporary" && isActive && (
        <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-4 mb-6">
          <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
          <div className="text-sm text-amber-800">
            <p className="font-medium mb-1">This link is temporary</p>
            <p>{PROVIDER_WARNINGS[tunnelProvider]}</p>
            <p className="mt-1">
              If it changes, you'll need to update the redirect URLs in your
              social media developer portals.
            </p>
          </div>
        </div>
      )}

      {/* Local only confirmation */}
      {tunnelMode === "none" && (
        <Card className="mb-6">
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Monitor className="h-4 w-4 text-gray-600" />
              <h3 className="text-sm font-medium text-gray-900">
                Local-only mode
              </h3>
            </div>
            <p className="text-sm text-gray-600">
              Postiz will be available at{" "}
              <span className="font-mono text-blue-600">
                http://localhost:{port}
              </span>{" "}
              on this computer only. You can still use Postiz to draft and
              organize content, but connecting social media accounts requires a
              public URL.
            </p>
            <p className="text-sm text-gray-500">
              You can set up a web link later by coming back to this step.
            </p>
            <Button
              variant="secondary"
              onClick={() => {
                setTunnelMode("temporary" as TunnelMode);
              }}
            >
              Create a web link instead
            </Button>
          </div>
        </Card>
      )}

      {/* Other options (collapsed) */}
      {tunnelStatus === "idle" && tunnelMode !== "permanent" && (
        <div className="mb-6">
          {/* Local only option */}
          {tunnelMode !== "none" && (
            <div className="rounded-lg border border-gray-200 bg-gray-50 p-4 mb-3">
              <p className="text-sm text-gray-700 mb-2">
                <strong>Don't need social media integrations?</strong> You can
                skip this step and use Postiz locally at{" "}
                <span className="font-mono text-blue-600">
                  http://localhost:{port}
                </span>
                .
              </p>
              <Button variant="ghost" onClick={handleChooseLocalOnly}>
                <Monitor className="h-4 w-4" />
                Use locally only
              </Button>
            </div>
          )}

          {/* Advanced: permanent domain */}
          <button
            onClick={() => setShowAdvanced(!showAdvanced)}
            className="inline-flex items-center gap-1 text-xs text-gray-500 hover:text-gray-700"
          >
            <ChevronDown
              className={`h-3 w-3 transition-transform ${showAdvanced ? "rotate-180" : ""}`}
            />
            Advanced: I have my own domain
          </button>

          {showAdvanced && (
            <div className="mt-3 rounded-lg border border-gray-200 bg-gray-50 p-4">
              <div className="flex items-start gap-2 mb-3">
                <Link className="h-4 w-4 text-gray-500 mt-0.5 shrink-0" />
                <div>
                  <p className="text-sm font-medium text-gray-900">
                    Use your own domain
                  </p>
                  <p className="text-xs text-gray-500 mt-1">
                    This requires you to already have a domain name, DNS
                    configured to point to this machine, and HTTPS set up (via a
                    reverse proxy like Nginx/Caddy or a Cloudflare tunnel with a
                    custom domain). This is an advanced option for experienced
                    users.
                  </p>
                </div>
              </div>
              <Button
                variant="secondary"
                onClick={() => {
                  setTunnelMode("permanent");
                  setDomainApplied(false);
                  setShowAdvanced(false);
                }}
              >
                Configure my domain
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Permanent mode: domain input */}
      {tunnelMode === "permanent" && (
        <Card className="mb-6">
          <div className="space-y-4">
            <div className="flex items-center gap-2 mb-1">
              <Link className="h-4 w-4 text-gray-600" />
              <h3 className="text-sm font-medium text-gray-900">
                Custom Domain
              </h3>
            </div>
            {!domainApplied ? (
              <>
                <Input
                  label="Your domain URL"
                  placeholder="https://postiz.example.com"
                  value={permanentDomain}
                  onChange={(e) => setPermanentDomain(e.target.value)}
                  disabled={applyingDomain}
                />
                <p className="text-xs text-gray-500">
                  Enter the full URL including <code>https://</code>. Your
                  domain must already resolve to this machine with HTTPS
                  configured.
                </p>
                {applyingDomain && (
                  <StatusIndicator status="loading" label={statusMessage} />
                )}
                <div className="flex items-center gap-2">
                  <Button
                    onClick={handleApplyDomain}
                    disabled={!permanentDomain.trim() || applyingDomain}
                  >
                    {applyingDomain ? "Applying..." : "Apply Domain"}
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setTunnelMode("temporary" as TunnelMode);
                    }}
                  >
                    Use temporary link instead
                  </Button>
                </div>
              </>
            ) : (
              <>
                <StatusIndicator status={domainReachable ? "success" : "warning"} label={statusMessage} />
                {tunnelUrl && (
                  <CopyField value={tunnelUrl} label="Your domain" />
                )}
                <Button
                  variant="secondary"
                  onClick={() => {
                    setDomainApplied(false);
                    setTunnelStatus("idle");
                    setTunnelUrl(null);
                  }}
                >
                  Change domain
                </Button>
              </>
            )}
          </div>
        </Card>
      )}

      <NavigationButtons
        canProceed={canProceed}
        loading={
          tunnelStatus === "starting" ||
          tunnelStatus === "restarting" ||
          applyingDomain
        }
        onNext={() => setStep(4)}
      />
    </div>
  );
}
