import { useCallback, useEffect, useRef, useState } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  applyManualDomain,
  connectCloudflareZeroTrust,
  switchToLocalOnly,
  stopTunnel,
  cancelDockerOperation,
  saveResumeState,
  runBootstrap,
  scanMachine,
  getInstallSnapshot,
  type BootstrapAction,
  type InstallSnapshot,
} from "../lib/tauri";
import { friendlyError } from "../lib/errors";
import { showToast } from "../components/ui/Toast";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { CopyField } from "../components/ui/CopyField";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { NavigationButtons } from "../components/wizard/NavigationButtons";
import { open } from "@tauri-apps/plugin-shell";
import {
  AlertTriangle,
  Globe,
  Monitor,
  Shield,
  Download,
  RefreshCw,
} from "lucide-react";

type LinkMethod = "custom-domain" | "cloudflare-zt" | "local-only" | null;

const POSTIZ_CADDY_GUIDE_URL = "https://docs.postiz.com/reverse-proxies/caddy";
const CADDY_QUICK_START_URL = "https://caddyserver.com/docs/quick-starts/caddyfile";
const CLOUDFLARE_ADD_SITE_URL =
  "https://developers.cloudflare.com/learning-paths/clientless-access/initial-setup/add-site/";
const CLOUDFLARE_REGISTER_DOMAIN_URL =
  "https://developers.cloudflare.com/registrar/get-started/register-domain/";
const CLOUDFLARE_TUNNEL_OVERVIEW_URL = "https://developers.cloudflare.com/tunnel/";
const CLOUDFLARE_PUBLISHED_APPS_URL =
  "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/routing-to-tunnel/";

export function CreateWebLink() {
  const {
    port,
    installPath,
    setTunnelStatus,
    tunnelUrl,
    setTunnelUrl,
    tunnelMode,
    setTunnelMode,
    tunnelProvider,
    setTunnelProvider,
    tunnelConfig,
    setTunnelConfig,
    permanentDomain,
    setPermanentDomain,
    setStep,
    machineState,
    setMachineState,
    providers,
    setProviderStatus,
  } = useWizardStore();

  const [statusMessage, setStatusMessage] = useState("");
  const [selectedMethod, setSelectedMethod] = useState<LinkMethod>(null);
  const [applying, setApplying] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [switchingLocal, setSwitchingLocal] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [showCaddy, setShowCaddy] = useState(true);
  const [unreachableUrl, setUnreachableUrl] = useState<string | null>(null);
  const [editingActiveLink, setEditingActiveLink] = useState(false);

  const [linkSnapshot, setLinkSnapshot] = useState<InstallSnapshot | null>(null);
  const [webLinkKind, setWebLinkKind] = useState<string | null>(null);

  const mountedRef = useRef(true);
  const opRef = useRef(0);

  useEffect(() => {
    return () => { mountedRef.current = false; };
  }, []);

  // Rescan machine state on mount
  useEffect(() => {
    if (!machineState) {
      scanMachine()
        .then((state) => setMachineState(state))
        .catch((err) => setScanError(String(err)));
    }
  }, []);

  const refreshLinkSnapshot = useCallback(async () => {
    const snap = await getInstallSnapshot();
    if (!mountedRef.current) return snap;
    setLinkSnapshot(snap);
    setWebLinkKind(snap.web_link_kind);
    return snap;
  }, []);

  useEffect(() => {
    refreshLinkSnapshot().catch(() => {});
  }, [refreshLinkSnapshot, tunnelMode, tunnelProvider]);

  const cloudflaredInstalled = machineState?.cloudflared_installed ?? false;

  const configuredProviders = Object.entries(providers).filter(
    ([, status]) => status === "configured",
  );

  const fallbackLinkKind =
    tunnelMode === "none"
      ? "none"
      : tunnelMode === "permanent"
        ? tunnelProvider === "manual"
          ? "manual"
          : "cloudflare"
        : null;
  const effectiveLinkKind = linkSnapshot?.web_link_kind ?? webLinkKind ?? fallbackLinkKind;
  const activeLinkUrl =
    (linkSnapshot?.tunnel_alive && linkSnapshot.tunnel_url) ||
    linkSnapshot?.permanent_domain ||
    tunnelUrl ||
    permanentDomain ||
    null;
  const hasConfiguredLink =
    effectiveLinkKind === "manual" ||
    effectiveLinkKind === "cloudflare" ||
    effectiveLinkKind === "legacy_shared";
  const hasUsableLink =
    tunnelMode === "none" ||
    (effectiveLinkKind === "manual" && !!activeLinkUrl) ||
    (effectiveLinkKind === "cloudflare" && Boolean(linkSnapshot?.tunnel_alive));
  const cloudflareLinkDisconnected =
    effectiveLinkKind === "cloudflare" &&
    linkSnapshot !== null &&
    !linkSnapshot.tunnel_alive;
  const showingMethodSelection =
    (editingActiveLink || !hasConfiguredLink || effectiveLinkKind === "legacy_shared") &&
    !applying &&
    !switchingLocal;

  const activeLinkStatus =
    effectiveLinkKind === "cloudflare"
      ? linkSnapshot === null
        ? "loading"
        : linkSnapshot.tunnel_alive
          ? "success"
          : "warning"
      : "success";
  const activeLinkLabel =
    effectiveLinkKind === "cloudflare"
      ? linkSnapshot === null
        ? "Checking current web link..."
        : linkSnapshot.tunnel_alive
          ? "Web link is active"
          : "Web link is configured but disconnected"
      : "Web link is active";
  const activeLinkDetail =
    effectiveLinkKind === "cloudflare"
      ? linkSnapshot === null
        ? "Checking whether the Cloudflare tunnel is connected."
        : linkSnapshot.tunnel_alive
          ? "Cloudflare Zero Trust is connected."
          : "Reconnect or change it before connecting social accounts."
      : effectiveLinkKind === "manual"
        ? "Your custom domain is configured."
        : undefined;

  const handleInstallCloudflared = async () => {
    setInstalling(true);
    try {
      await runBootstrap("InstallCloudflared" as BootstrapAction);
    } catch (err) {
      showToast(friendlyError(String(err)), "error");
    }
    try {
      const state = await scanMachine();
      setMachineState(state);
      setScanError(null);
    } catch (err) {
      showToast(friendlyError(String(err)), "error");
    }
    setInstalling(false);
  };

  // ── Custom Domain ──────────────────────────────────────────────────

  const handleApplyCustomDomain = async (force = false) => {
    const raw = permanentDomain.trim();
    if (!raw) return;

    const domain = raw.replace(/\/+$/, "");
    if (!domain.startsWith("https://")) {
      setStatusMessage("Domain must start with https:// — social platforms require HTTPS.");
      return;
    }

    try {
      new URL(domain);
    } catch {
      setStatusMessage("Invalid URL format. Enter a full URL like https://postiz.example.com");
      return;
    }

    const opId = ++opRef.current;
    setApplying(true);
    setStatusMessage("Applying domain configuration...");
    setUnreachableUrl(null);

    try {
      await applyManualDomain(installPath, domain, force);
      if (!mountedRef.current || opRef.current !== opId) return;

      setPermanentDomain(domain);
      setTunnelUrl(null);
      setTunnelMode("permanent");
      setTunnelProvider("manual");
      setTunnelConfig("");
      setTunnelStatus("running");
      setStatusMessage("Domain applied successfully!");
      setEditingActiveLink(false);
      setSelectedMethod(null);
      setUnreachableUrl(null);
      await saveResumeState();
      await refreshLinkSnapshot().catch(() => {});
    } catch (err) {
      if (!mountedRef.current || opRef.current !== opId) return;
      const errStr = String(err);
      if (errStr.startsWith("UNREACHABLE:")) {
        setUnreachableUrl(domain);
        setStatusMessage(
          `Could not reach ${domain}. Make sure your reverse proxy is running and DNS is configured.`,
        );
      } else {
        setStatusMessage(friendlyError(errStr));
      }
    } finally {
      if (mountedRef.current && opRef.current === opId) setApplying(false);
    }
  };

  // ── Cloudflare Zero Trust ──────────────────────────────────────────

  const handleConnectZeroTrust = async () => {
    const url = permanentDomain.trim().replace(/\/+$/, "");
    const token = tunnelConfig.trim();

    if (!url || !token) {
      setStatusMessage("Both the public URL and tunnel token are required.");
      return;
    }
    if (!url.startsWith("https://")) {
      setStatusMessage("URL must start with https://");
      return;
    }
    try {
      new URL(url);
    } catch {
      setStatusMessage("Invalid URL format. Enter a full URL like https://postiz.example.com");
      return;
    }

    const opId = ++opRef.current;
    setApplying(true);
    setStatusMessage("Connecting Cloudflare tunnel...");
    setUnreachableUrl(null);

    try {
      await connectCloudflareZeroTrust(installPath, port, url, token);
      if (!mountedRef.current || opRef.current !== opId) return;

      setPermanentDomain(url);
      setTunnelUrl(url);
      setTunnelMode("permanent");
      setTunnelProvider("cloudflared");
      setTunnelConfig(token);
      setTunnelStatus("running");
      setStatusMessage("Cloudflare tunnel connected!");
      setEditingActiveLink(false);
      setSelectedMethod(null);
      await saveResumeState();
      await refreshLinkSnapshot().catch(() => {});
    } catch (err) {
      if (!mountedRef.current || opRef.current !== opId) return;
      setStatusMessage(friendlyError(String(err)));
    } finally {
      if (mountedRef.current && opRef.current === opId) setApplying(false);
    }
  };

  // ── Local Only ─────────────────────────────────────────────────────

  const handleChooseLocalOnly = async () => {
    const opId = ++opRef.current;
    setSwitchingLocal(true);
    setStatusMessage("Switching to local-only mode...");

    try {
      const previousMainUrl = await switchToLocalOnly(installPath, port);
      if (!mountedRef.current || opRef.current !== opId) return;

      // Only mark providers stale if hostname actually changed
      const hadPublicUrl =
        previousMainUrl && !previousMainUrl.includes("localhost");
      if (hadPublicUrl && configuredProviders.length > 0) {
        for (const [providerName] of configuredProviders) {
          setProviderStatus(providerName, "stale");
        }
      }

      setTunnelMode("none");
      setTunnelProvider("manual");
      setPermanentDomain("");
      setTunnelConfig("");
      setTunnelUrl(null);
      setTunnelStatus("idle");
      setStatusMessage("");
      setEditingActiveLink(false);
      setSelectedMethod(null);
      setUnreachableUrl(null);
      await saveResumeState();
      await refreshLinkSnapshot().catch(() => {});
    } catch (err) {
      if (!mountedRef.current || opRef.current !== opId) return;
      setStatusMessage(friendlyError(String(err)));
    } finally {
      if (mountedRef.current && opRef.current === opId) {
        setSwitchingLocal(false);
      }
    }
  };

  // ── Change Web Link ────────────────────────────────────────────────

  const handleChangeLink = () => {
    setEditingActiveLink(true);
    setStatusMessage("");
    setSelectedMethod(null);
    setUnreachableUrl(null);
  };

  const handleSelectMethod = (method: LinkMethod) => {
    setSelectedMethod((current) => (current === method ? null : method));
    setStatusMessage("");
    setUnreachableUrl(null);
  };

  const canProceed =
    !editingActiveLink &&
    hasUsableLink;

  // ── Render ─────────────────────────────────────────────────────────

  return (
    <div className="max-w-2xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        Create a web link
      </h2>
      <p className="text-gray-600 mb-6">
        Social media platforms need a public URL to communicate with Postiz.
        Choose how you'd like to make Postiz accessible.
      </p>

      <Card className="mb-6 bg-slate-50/70">
        <div className="space-y-3">
          <div>
            <h3 className="text-sm font-medium text-gray-900 mb-1">
              How this step works
            </h3>
            <p className="text-sm text-gray-600">
              Postiz needs one stable public HTTPS address, such as
              {" "}
              <code className="bg-white px-1 rounded">https://postiz.example.com</code>,
              for login callbacks and social platform integrations.
            </p>
          </div>
          <ol className="text-sm text-gray-600 list-decimal list-inside space-y-1">
            <li>Pick the public hostname you want people and integrations to use.</li>
            <li>Choose how traffic reaches this computer.</li>
            <li>This wizard updates Postiz to use that URL everywhere.</li>
          </ol>
          <div className="grid gap-3 md:grid-cols-3">
            <div className="rounded-lg border border-gray-200 bg-white p-3">
              <p className="text-xs font-medium text-gray-900 mb-1">
                Custom Domain
              </p>
              <p className="text-xs text-gray-600">
                You handle DNS and a reverse proxy such as Caddy, Nginx, or
                Traefik.
              </p>
            </div>
            <div className="rounded-lg border border-gray-200 bg-white p-3">
              <p className="text-xs font-medium text-gray-900 mb-1">
                Cloudflare Zero Trust
              </p>
              <p className="text-xs text-gray-600">
                Cloudflare hosts the public edge, and `cloudflared` forwards
                traffic to Postiz without opening router ports.
              </p>
            </div>
            <div className="rounded-lg border border-gray-200 bg-white p-3">
              <p className="text-xs font-medium text-gray-900 mb-1">
                Local Only
              </p>
              <p className="text-xs text-gray-600">
                Postiz stays on this computer only. Social integrations will not
                work.
              </p>
            </div>
          </div>
          <p className="text-xs text-gray-500">
            This app can update Postiz once your web link exists, but it cannot
            buy domains, change your router, or edit your DNS provider for you.
            Those steps happen in your registrar, DNS provider, router, or
            Cloudflare account.
          </p>
        </div>
      </Card>

      {/* Scan error banner */}
      {scanError && !machineState && (
        <div className="rounded-lg bg-red-50 border border-red-200 p-4 mb-4">
          <p className="text-sm text-red-700 font-medium mb-1">System scan failed</p>
          <p className="text-sm text-red-600 mb-3">{scanError}</p>
          <Button
            variant="secondary"
            onClick={() => {
              setScanError(null);
              scanMachine()
                .then((state) => setMachineState(state))
                .catch((err) => setScanError(String(err)));
            }}
          >
            Retry scan
          </Button>
        </div>
      )}

      {/* Legacy shared-domain warning */}
      {effectiveLinkKind === "legacy_shared" && (
        <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-4 mb-6">
          <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
          <div className="text-sm text-amber-800">
            <p className="font-medium mb-1">
              Shared tunnel domain no longer supported
            </p>
            <p className="mb-2">
              {activeLinkUrl
                ? `Your current web link (${activeLinkUrl}) uses a shared tunnel domain.`
                : "Your previous setup used a shared tunnel domain."}{" "}
              {linkSnapshot?.web_link_reason
                ?? "Login doesn't work on these domains due to a known Postiz limitation."}{" "}
              Choose a supported link method below.
            </p>
          </div>
        </div>
      )}

      {/* ── Active Link Display ─────────────────────────── */}
      {hasConfiguredLink && effectiveLinkKind !== "legacy_shared" && !editingActiveLink && (
        <Card className="mb-6">
          <div className="space-y-4">
            <StatusIndicator
              status={activeLinkStatus}
              label={activeLinkLabel}
              detail={activeLinkDetail}
            />
            {activeLinkUrl && (
              <CopyField
                value={activeLinkUrl}
                label={cloudflareLinkDisconnected ? "Configured public URL" : "Your web link"}
              />
            )}
            <div className="flex gap-2">
              <Button variant="secondary" onClick={handleChangeLink}>
                <RefreshCw className="h-4 w-4" />
                {cloudflareLinkDisconnected ? "Reconnect or Change Web Link" : "Change Web Link"}
              </Button>
            </div>
          </div>
        </Card>
      )}

      {/* ── Method Selection ─────────────────────────── */}
      {showingMethodSelection && (
        <>
          {editingActiveLink && activeLinkUrl && (
            <Card className="mb-4">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <p className="text-sm text-gray-600">
                  Your current web link will stay active until you apply a replacement.
                </p>
                <Button
                  variant="ghost"
                  onClick={() => {
                    setEditingActiveLink(false);
                    setSelectedMethod(null);
                    setStatusMessage("");
                    setUnreachableUrl(null);
                  }}
                >
                  Keep current link
                </Button>
              </div>
            </Card>
          )}

          <div className="grid gap-4 mb-6">
            {/* Card 1: Custom Domain (Recommended) */}
            <button
              onClick={() => handleSelectMethod("custom-domain")}
              className={`text-left rounded-lg border-2 p-4 transition-colors ${
                selectedMethod === "custom-domain"
                  ? "border-blue-500 bg-blue-50"
                  : "border-gray-200 hover:border-gray-300 bg-white"
              }`}
            >
              <div className="flex items-center gap-3">
                <Globe className="h-5 w-5 text-blue-600 shrink-0" />
                <div>
                  <p className="text-sm font-medium text-gray-900">
                    Custom Domain
                    <span className="ml-2 text-xs font-normal text-blue-600 bg-blue-100 px-1.5 py-0.5 rounded">
                      Recommended
                    </span>
                  </p>
                  <p className="text-xs text-gray-500 mt-0.5">
                    Best if you want full control and can manage DNS plus a
                    reverse proxy such as Caddy, Nginx, or Traefik.
                  </p>
                </div>
              </div>
            </button>

            {/* Card 2: Cloudflare Zero Trust */}
            <button
              onClick={() => handleSelectMethod("cloudflare-zt")}
              className={`text-left rounded-lg border-2 p-4 transition-colors ${
                selectedMethod === "cloudflare-zt"
                  ? "border-blue-500 bg-blue-50"
                  : "border-gray-200 hover:border-gray-300 bg-white"
              }`}
            >
              <div className="flex items-center gap-3">
                <Shield className="h-5 w-5 text-orange-500 shrink-0" />
                <div>
                  <p className="text-sm font-medium text-gray-900">
                    Cloudflare Zero Trust
                  </p>
                  <p className="text-xs text-gray-500 mt-0.5">
                    Best if you do not want router changes. Cloudflare handles
                    the public edge, but you still need a domain or subdomain in
                    Cloudflare.
                  </p>
                </div>
              </div>
            </button>

            {/* Card 3: Local Only */}
            <button
              onClick={() => handleSelectMethod("local-only")}
              className={`text-left rounded-lg border-2 p-4 transition-colors ${
                selectedMethod === "local-only"
                  ? "border-blue-500 bg-blue-50"
                  : "border-gray-200 hover:border-gray-300 bg-white"
              }`}
            >
              <div className="flex items-center gap-3">
                <Monitor className="h-5 w-5 text-gray-500 shrink-0" />
                <div>
                  <p className="text-sm font-medium text-gray-900">
                    Local Only
                  </p>
                  <p className="text-xs text-gray-500 mt-0.5">
                    Simplest option. Access Postiz on this computer only and
                    skip social media integrations.
                  </p>
                </div>
              </div>
            </button>
          </div>

          {/* ── Custom Domain Expanded ─────────────────── */}
          {selectedMethod === "custom-domain" && (
            <Card className="mb-6">
              <div className="space-y-4">
                <div className="flex items-center gap-2 mb-1">
                  <Globe className="h-4 w-4 text-blue-600" />
                  <h3 className="text-sm font-medium text-gray-900">
                    Custom Domain Setup
                  </h3>
                </div>

                <div className="rounded-lg border border-blue-200 bg-blue-50 p-4 space-y-3">
                  <div>
                    <p className="text-sm font-medium text-blue-950">
                      If you already own a domain, do this in order
                    </p>
                    <p className="text-xs text-blue-900/80 mt-1">
                      Most users should use a subdomain like
                      {" "}
                      <code className="bg-white px-1 rounded">postiz.yourdomain.com</code>
                      {" "}
                      so they do not disturb an existing website or email setup.
                    </p>
                  </div>
                  <ol className="text-sm text-blue-950 list-decimal list-inside space-y-1">
                    <li>Choose an unused subdomain such as `postiz.yourdomain.com`.</li>
                    <li>Create a DNS record for that subdomain pointing to your public IP.</li>
                    <li>Forward ports `80` and `443` to the computer running Postiz.</li>
                    <li>Run Caddy or another reverse proxy that sends that hostname to `http://localhost:{port}`.</li>
                    <li>Open `https://postiz.yourdomain.com` in a browser and make sure it loads.</li>
                    <li>Paste that exact URL below, then click `Verify & Apply`.</li>
                  </ol>
                </div>

                <div className="bg-blue-50 rounded-lg border border-blue-200 p-3 space-y-2">
                  <p className="text-xs font-medium text-blue-900">
                    What this option means
                  </p>
                  <p className="text-xs text-blue-900/90">
                    People visit your public hostname, DNS sends that hostname to
                    your network, and a reverse proxy on your machine or server
                    forwards the request to Postiz at
                    {" "}
                    <code className="bg-white px-1 rounded">http://localhost:{port}</code>.
                  </p>
                  <p className="text-xs text-blue-900/80">
                    HTTPS is required here because social platforms send login
                    callbacks back to this exact URL.
                  </p>
                </div>

                <div className="grid gap-3 md:grid-cols-2">
                  <div className="rounded-lg border border-gray-200 bg-gray-50 p-3">
                    <p className="text-xs font-medium text-gray-900 mb-1">
                      What is DNS?
                    </p>
                    <p className="text-xs text-gray-600">
                      DNS is the part that points a name like
                      {" "}
                      <code className="bg-white px-1 rounded">postiz.example.com</code>
                      {" "}
                      to your public IP address.
                    </p>
                  </div>
                  <div className="rounded-lg border border-gray-200 bg-gray-50 p-3">
                    <p className="text-xs font-medium text-gray-900 mb-1">
                      What is a reverse proxy?
                    </p>
                    <p className="text-xs text-gray-600">
                      It is the web server that accepts HTTPS from the internet
                      and forwards requests to Postiz running locally on port
                      {" "}
                      <code className="bg-white px-1 rounded">{port}</code>.
                    </p>
                  </div>
                </div>

                <div className="bg-gray-50 rounded-lg p-3">
                  <p className="text-xs text-gray-700 font-medium mb-2">
                    Before you click Verify and Apply
                  </p>
                  <ol className="text-xs text-gray-600 list-decimal list-inside space-y-1">
                    <li>Own or control the domain or subdomain you want to use.</li>
                    <li>Point that hostname at your public IP with DNS.</li>
                    <li>
                      Make sure ports 80 and 443 reach the machine running your
                      reverse proxy, either directly or through your router and
                      firewall.
                    </li>
                    <li>
                      Configure your reverse proxy to forward that hostname to
                      {" "}
                      <code className="bg-white px-1 rounded">http://localhost:{port}</code>.
                    </li>
                  </ol>
                </div>

                <div className="rounded-lg border border-gray-200 bg-white p-3 space-y-2">
                  <p className="text-xs font-medium text-gray-900">
                    If you already own a domain
                  </p>
                  <ol className="text-xs text-gray-600 list-decimal list-inside space-y-1">
                    <li>
                      Pick an unused subdomain such as
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">postiz.yourdomain.com</code>
                      {" "}
                      instead of replacing your main domain.
                    </li>
                    <li>
                      In your DNS provider, create an
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">A</code>
                      {" "}
                      record for that subdomain pointing to your public IPv4
                      address. Add an
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">AAAA</code>
                      {" "}
                      record too if you use IPv6.
                    </li>
                    <li>
                      If your home IP changes often, use your router&apos;s
                      dynamic DNS support or choose the Cloudflare option
                      instead.
                    </li>
                    <li>
                      Forward ports 80 and 443 from your router to the machine
                      running Caddy or your existing reverse proxy.
                    </li>
                    <li>
                      Configure that proxy to serve
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">postiz.yourdomain.com</code>
                      {" "}
                      over HTTPS and forward to
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">http://localhost:{port}</code>.
                    </li>
                    <li>
                      Open
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">https://postiz.yourdomain.com</code>
                      {" "}
                      from a device outside your home network if possible, then
                      paste that exact URL below.
                    </li>
                  </ol>
                  <p className="text-xs text-gray-500">
                    Using a subdomain is usually safest because it leaves your
                    existing website, email, and other DNS records alone.
                  </p>
                </div>

                {/* Caddy / other proxy toggle */}
                <div className="flex gap-2 text-xs">
                  <button
                    onClick={() => setShowCaddy(true)}
                    className={`px-2 py-1 rounded ${showCaddy ? "bg-blue-100 text-blue-700" : "text-gray-500 hover:text-gray-700"}`}
                  >
                    Caddy (easiest)
                  </button>
                  <button
                    onClick={() => setShowCaddy(false)}
                    className={`px-2 py-1 rounded ${!showCaddy ? "bg-blue-100 text-blue-700" : "text-gray-500 hover:text-gray-700"}`}
                  >
                    I already have a proxy
                  </button>
                </div>

                {showCaddy && (
                  <div className="bg-gray-50 rounded-lg p-3 space-y-3">
                    <div>
                      <p className="text-xs font-medium text-gray-700 mb-1">
                        What is Caddy?
                      </p>
                      <p className="text-xs text-gray-600">
                        Caddy is a web server and reverse proxy. It can
                        automatically get and renew HTTPS certificates, which
                        makes it one of the easiest ways to publish Postiz on
                        your own domain.
                      </p>
                    </div>

                    <div>
                      <p className="text-xs font-medium text-gray-700 mb-1">
                        Typical Caddy setup
                      </p>
                      <ol className="text-xs text-gray-600 list-decimal list-inside space-y-1">
                        <li>
                          Create a DNS record for
                          {" "}
                          <code className="bg-white px-1 rounded">postiz.example.com</code>
                          {" "}
                          that points to your public IP.
                        </li>
                        <li>
                          Forward ports 80 and 443 from your router or firewall
                          to the machine running Caddy.
                        </li>
                        <li>
                          Save the Caddyfile below and start or reload Caddy.
                        </li>
                      </ol>
                    </div>

                    <p className="text-xs text-gray-600">
                      Example Caddyfile for your Postiz install:
                    </p>
                    <pre className="text-xs bg-white rounded border border-gray-200 p-2 overflow-x-auto">
{`postiz.example.com {
    reverse_proxy localhost:${port}
}`}
                    </pre>

                    <p className="text-xs text-gray-500">
                      When someone visits your public hostname, Caddy accepts the
                      HTTPS request and forwards it to Postiz on
                      {" "}
                      <code className="bg-white px-1 rounded">localhost:{port}</code>.
                    </p>

                    <div className="flex flex-wrap gap-2">
                      <button
                        onClick={() => open(POSTIZ_CADDY_GUIDE_URL)}
                        className="text-xs text-blue-600 hover:text-blue-700 underline"
                      >
                        Postiz Caddy guide
                      </button>
                      <button
                        onClick={() => open(CADDY_QUICK_START_URL)}
                        className="text-xs text-blue-600 hover:text-blue-700 underline"
                      >
                        Caddy quick start
                      </button>
                    </div>
                  </div>
                )}

                {!showCaddy && (
                  <div className="bg-gray-50 rounded-lg p-3 space-y-2">
                    <p className="text-xs font-medium text-gray-700">
                      Using Nginx, Traefik, or another reverse proxy
                    </p>
                    <p className="text-xs text-gray-600">
                      That is fine. The important part is that your proxy serves
                      the same public hostname over HTTPS and forwards requests
                      to
                      {" "}
                      <code className="bg-white px-1 rounded">http://localhost:{port}</code>.
                    </p>
                    <ul className="text-xs text-gray-600 list-disc list-inside space-y-1">
                      <li>The public URL must be the hostname users will open.</li>
                      <li>Your proxy must terminate HTTPS for that hostname.</li>
                      <li>DNS must already point that hostname to your network.</li>
                    </ul>
                  </div>
                )}

                <Input
                  label="Your public URL"
                  placeholder="https://postiz.example.com"
                  value={permanentDomain}
                  onChange={(e) => setPermanentDomain(e.target.value)}
                  disabled={applying}
                />

                {statusMessage && (
                  <StatusIndicator
                    status={applying ? "loading" : unreachableUrl ? "warning" : "error"}
                    label={statusMessage}
                  />
                )}

                {unreachableUrl && !applying && (
                  <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-3">
                    <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0 mt-0.5" />
                    <div className="text-sm text-amber-800">
                      <p className="mb-2">
                        The URL is not reachable. Make sure your reverse proxy is
                        running, DNS points to this machine, and ports 80 and 443
                        are reachable from the internet.
                      </p>
                      <Button
                        variant="secondary"
                        onClick={() => handleApplyCustomDomain(true)}
                      >
                        Apply anyway
                      </Button>
                    </div>
                  </div>
                )}

                {applying && (
                  <Button
                    variant="secondary"
                    onClick={async () => {
                      opRef.current++;
                      try { await cancelDockerOperation(); } catch {}
                      setApplying(false);
                      setStatusMessage("Cancelled.");
                    }}
                  >
                    Cancel
                  </Button>
                )}

                <div className="flex items-center gap-2">
                  <Button
                    onClick={() => handleApplyCustomDomain(false)}
                    disabled={!permanentDomain.trim() || applying}
                  >
                    {applying ? "Applying..." : "Verify & Apply"}
                  </Button>
                </div>

                {/* Show resulting env values preview */}
                {permanentDomain.trim() && permanentDomain.trim().startsWith("https://") && (
                  <details className="text-xs text-gray-500">
                    <summary className="cursor-pointer hover:text-gray-700">
                      Environment variables that will be set
                    </summary>
                    <pre className="mt-1 bg-gray-50 rounded border border-gray-200 p-2 overflow-x-auto">
{`MAIN_URL=${permanentDomain.trim().replace(/\/+$/, "")}
FRONTEND_URL=${permanentDomain.trim().replace(/\/+$/, "")}
NEXT_PUBLIC_BACKEND_URL=${permanentDomain.trim().replace(/\/+$/, "")}/api`}
                    </pre>
                  </details>
                )}
              </div>
            </Card>
          )}

          {/* ── Cloudflare Zero Trust Expanded ─────────── */}
          {selectedMethod === "cloudflare-zt" && (
            <Card className="mb-6">
              <div className="space-y-4">
                <div className="flex items-center gap-2 mb-1">
                  <Shield className="h-4 w-4 text-orange-500" />
                  <h3 className="text-sm font-medium text-gray-900">
                    Cloudflare Zero Trust Setup
                  </h3>
                </div>

                <div className="bg-orange-50 rounded-lg border border-orange-200 p-3 space-y-2">
                  <p className="text-xs font-medium text-orange-900">
                    What this option means
                  </p>
                  <p className="text-xs text-orange-900/90">
                    Cloudflare provides the public HTTPS endpoint, and
                    {" "}
                    <code className="bg-white px-1 rounded">cloudflared</code>
                    {" "}
                    on this machine creates an outbound tunnel back to Postiz at
                    {" "}
                    <code className="bg-white px-1 rounded">http://localhost:{port}</code>.
                  </p>
                  <p className="text-xs text-orange-900/80">
                    That means no inbound router port forwarding, but you still
                    need a domain or subdomain managed by Cloudflare.
                  </p>
                </div>

                {!cloudflaredInstalled && (
                  <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-3">
                    <Download className="h-4 w-4 text-amber-600 shrink-0 mt-0.5" />
                    <div className="text-sm text-amber-800">
                      <p className="mb-2">
                        Cloudflared is required but not installed.
                      </p>
                      <Button
                        variant="secondary"
                        onClick={handleInstallCloudflared}
                        loading={installing}
                      >
                        <Download className="h-3 w-3" /> Install cloudflared
                      </Button>
                    </div>
                  </div>
                )}

                <div className="bg-gray-50 rounded-lg p-3">
                  <p className="text-xs text-gray-700 font-medium mb-2">
                    Before you start
                  </p>
                  <p className="text-xs text-gray-600 mb-2">
                    Cloudflare Tunnel itself can be used on Cloudflare&apos;s Free
                    plan, but Cloudflare does <strong>not</strong> include a free
                    registered domain with this setup. You need one of these first:
                  </p>
                  <ul className="text-xs text-gray-600 list-disc list-inside space-y-1 mb-3">
                    <li>A domain you already own, added or transferred to Cloudflare</li>
                    <li>A new domain you register first, then manage in Cloudflare</li>
                  </ul>
                  <p className="text-xs text-gray-500 mb-3">
                    Once your domain is in Cloudflare, you can create subdomains
                    like <code className="bg-white px-1 rounded">postiz.yourdomain.com</code> for free.
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <button
                      onClick={() => open(CLOUDFLARE_ADD_SITE_URL)}
                      className="text-xs text-blue-600 hover:text-blue-700 underline"
                    >
                      Add an existing domain to Cloudflare
                    </button>
                    <button
                      onClick={() => open(CLOUDFLARE_REGISTER_DOMAIN_URL)}
                      className="text-xs text-blue-600 hover:text-blue-700 underline"
                    >
                      Register a new domain in Cloudflare
                    </button>
                  </div>
                </div>

                <div className="rounded-lg border border-gray-200 bg-white p-3 space-y-2">
                  <p className="text-xs font-medium text-gray-900">
                    If you already own a domain
                  </p>
                  <ol className="text-xs text-gray-600 list-decimal list-inside space-y-1">
                    <li>
                      Choose an unused subdomain such as
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">postiz.yourdomain.com</code>.
                    </li>
                    <li>
                      If your domain is already managed in Cloudflare, you can
                      keep it there and use that subdomain immediately.
                    </li>
                    <li>
                      If your domain is at another registrar or DNS provider,
                      add the site to Cloudflare or transfer it there first.
                    </li>
                    <li>
                      In the Zero Trust tunnel, create a public hostname for
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">postiz.yourdomain.com</code>
                      {" "}
                      and point it at
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">http://localhost:{port}</code>.
                    </li>
                    <li>
                      Copy the tunnel token from Cloudflare and paste it into
                      this screen so the local
                      {" "}
                      <code className="bg-gray-100 px-1 rounded">cloudflared</code>
                      {" "}
                      service can connect.
                    </li>
                  </ol>
                  <p className="text-xs text-gray-500">
                    You usually do not need to touch your router for this
                    method, because the tunnel is outbound from your machine to
                    Cloudflare.
                  </p>
                </div>

                <div className="bg-gray-50 rounded-lg p-3">
                  <p className="text-xs text-gray-600 font-medium mb-2">
                    Steps in the Cloudflare dashboard:
                  </p>
                  <ol className="text-xs text-gray-600 list-decimal list-inside space-y-1">
                    <li>
                      Get a domain into Cloudflare first using one of the links above
                    </li>
                    <li>
                      Go to{" "}
                      <button
                        onClick={() => open("https://one.dash.cloudflare.com/")}
                        className="text-blue-600 hover:text-blue-700 underline"
                      >
                        Zero Trust dashboard
                      </button>{" "}
                      → Networks → Tunnels
                    </li>
                    <li>Create a <strong>Cloudflared</strong> tunnel</li>
                    <li>
                      Add a public hostname route (e.g.{" "}
                      <code className="bg-white px-1 rounded">postiz.example.com</code>)
                    </li>
                    <li>
                      Set the service URL to{" "}
                      <code className="bg-white px-1 rounded">
                        http://localhost:{port}
                      </code>
                    </li>
                    <li>Copy the tunnel token from the install command</li>
                    <li>Paste the URL and token below</li>
                  </ol>
                  <div className="flex flex-wrap gap-2 mt-3">
                    <button
                      onClick={() => open(CLOUDFLARE_TUNNEL_OVERVIEW_URL)}
                      className="text-xs text-blue-600 hover:text-blue-700 underline"
                    >
                      Cloudflare Tunnel overview
                    </button>
                    <button
                      onClick={() => open(CLOUDFLARE_PUBLISHED_APPS_URL)}
                      className="text-xs text-blue-600 hover:text-blue-700 underline"
                    >
                      Published application guide
                    </button>
                  </div>
                </div>

                <div className="grid gap-3 md:grid-cols-2">
                  <div className="rounded-lg border border-gray-200 bg-gray-50 p-3">
                    <p className="text-xs font-medium text-gray-900 mb-1">
                      What is cloudflared?
                    </p>
                    <p className="text-xs text-gray-600">
                      It is Cloudflare&apos;s small connector program. It runs on
                      this machine and keeps the tunnel connected to your
                      Cloudflare account.
                    </p>
                  </div>
                  <div className="rounded-lg border border-gray-200 bg-gray-50 p-3">
                    <p className="text-xs font-medium text-gray-900 mb-1">
                      What is the tunnel token?
                    </p>
                    <p className="text-xs text-gray-600">
                      It is the secret token Cloudflare shows in the tunnel
                      install command. This wizard uses that token to connect
                      `cloudflared` on your machine to the tunnel you created.
                    </p>
                  </div>
                </div>

                <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-3">
                  <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0 mt-0.5" />
                  <p className="text-xs text-amber-800">
                    Do not put Cloudflare Access in front of the Postiz
                    hostname — OAuth callbacks from social platforms must reach
                    the app directly.
                  </p>
                </div>

                <Input
                  label="Public URL"
                  placeholder="https://postiz.example.com"
                  value={permanentDomain}
                  onChange={(e) => setPermanentDomain(e.target.value)}
                  disabled={applying}
                />
                <Input
                  label="Tunnel token"
                  placeholder="Paste your tunnel token here"
                  value={tunnelConfig}
                  onChange={(e) => setTunnelConfig(e.target.value)}
                  disabled={applying}
                  secret
                />

                {statusMessage && (
                  <StatusIndicator
                    status={applying ? "loading" : "error"}
                    label={statusMessage}
                  />
                )}

                {applying && (
                  <Button
                    variant="secondary"
                    onClick={async () => {
                      opRef.current++;
                      try { await stopTunnel(); } catch {}
                      try { await cancelDockerOperation(); } catch {}
                      setApplying(false);
                      setStatusMessage("Cancelled.");
                    }}
                  >
                    Cancel
                  </Button>
                )}

                <Button
                  onClick={handleConnectZeroTrust}
                  disabled={
                    !permanentDomain.trim() ||
                    !tunnelConfig.trim() ||
                    !cloudflaredInstalled ||
                    applying
                  }
                >
                  {applying ? "Connecting..." : "Connect Tunnel"}
                </Button>
              </div>
            </Card>
          )}

          {/* ── Local Only Expanded ────────────────────── */}
          {selectedMethod === "local-only" && (
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
                  <button
                    onClick={() => open(`http://localhost:${port}`)}
                    className="font-mono text-blue-600 hover:text-blue-700 underline"
                  >
                    http://localhost:{port}
                  </button>{" "}
                  on this computer only. You can still use Postiz to draft and
                  organize content, but connecting social media accounts requires
                  a public URL.
                </p>

                {configuredProviders.length > 0 && (
                  <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-3">
                    <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
                    <div className="text-sm text-amber-800">
                      <p className="font-medium mb-1">
                        Social integrations may be affected
                      </p>
                      <p>
                        You previously configured{" "}
                        {configuredProviders.length}{" "}
                        {configuredProviders.length === 1
                          ? "provider"
                          : "providers"}{" "}
                        ({configuredProviders.map(([name]) => name).join(", ")}
                        ). OAuth callbacks require a public URL, so these
                        integrations won't work in local-only mode.
                      </p>
                    </div>
                  </div>
                )}

                {switchingLocal && (
                  <>
                    <StatusIndicator status="loading" label={statusMessage} />
                    <Button
                      variant="secondary"
                      onClick={async () => {
                        opRef.current++;
                        try { await cancelDockerOperation(); } catch {}
                        setSwitchingLocal(false);
                        setStatusMessage("Switch cancelled.");
                      }}
                    >
                      Cancel
                    </Button>
                  </>
                )}

                <Button
                  onClick={handleChooseLocalOnly}
                  disabled={switchingLocal}
                  loading={switchingLocal}
                >
                  <Monitor className="h-4 w-4" />
                  {switchingLocal ? "Switching..." : "Use locally only"}
                </Button>
              </div>
            </Card>
          )}
        </>
      )}

      {/* ── In-progress status (applying domain or connecting tunnel) ── */}
      {applying && (
        <Card className="mb-6">
          <StatusIndicator status="loading" label={statusMessage} />
        </Card>
      )}

      {/* ── Local only confirmation ── */}
      {tunnelMode === "none" && !selectedMethod && (
        <Card className="mb-6">
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Monitor className="h-4 w-4 text-gray-600" />
              <h3 className="text-sm font-medium text-gray-900">
                Local-only mode
              </h3>
            </div>
            <p className="text-sm text-gray-600">
              Postiz is available at{" "}
              <button
                onClick={() => open(`http://localhost:${port}`)}
                className="font-mono text-blue-600 hover:text-blue-700 underline"
              >
                http://localhost:{port}
              </button>{" "}
              on this computer only.
            </p>
            <p className="text-sm text-gray-500">
              You can set up a web link by choosing a method above.
            </p>
            <Button
              variant="secondary"
              onClick={() => {
                handleSelectMethod("custom-domain");
                setStatusMessage("");
              }}
            >
              Set up a web link
            </Button>
          </div>
        </Card>
      )}

      <NavigationButtons
        canProceed={canProceed}
        loading={applying || switchingLocal}
        onNext={() => setStep(4)}
      />
    </div>
  );
}
