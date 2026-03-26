import { useState, useEffect, useRef } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  stageProviderConfig,
  applyConfigTransaction,
  updateBaseUrls,
  cancelDockerOperation,
  saveResumeState,
  readEnvValue,
  getSavedCredentials,
} from "../lib/tauri";
import {
  providers,
  getCallbackUrl,
  getHomepageUrl,
  type ProviderDefinition,
} from "../components/providers/registry";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { CopyField } from "../components/ui/CopyField";
import { NavigationButtons } from "../components/wizard/NavigationButtons";
import { friendlyError } from "../lib/errors";
import { showToast } from "../components/ui/Toast";
import { open } from "@tauri-apps/plugin-shell";
import {
  ExternalLink,
  X,
  Check,
  Clock,
  AlertTriangle,
  Lock,
  ChevronDown,
} from "lucide-react";
import * as icons from "lucide-react";

const iconMap: Record<string, React.ComponentType<{ className?: string }>> =
  icons as unknown as Record<string, React.ComponentType<{ className?: string }>>;

function ProviderIcon({ name }: { name: string }) {
  const Icon = iconMap[name];
  if (!Icon) return <div className="h-5 w-5 bg-gray-300 rounded" />;
  return <Icon className="h-5 w-5" />;
}

interface ProviderModalProps {
  provider: ProviderDefinition;
  baseUrl: string;
  installPath: string;
  onClose: () => void;
  onSave: (entries: Record<string, string>) => Promise<void> | void;
}

function ProviderModal({
  provider,
  baseUrl,
  installPath,
  onClose,
  onSave,
}: ProviderModalProps) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const modalRef = useRef<HTMLDivElement>(null);
  const callbackUrl = getCallbackUrl(provider, baseUrl);
  const homepageUrl = getHomepageUrl(provider, baseUrl);

  // Pre-populate credentials: try local install-state.json first, fall back to postiz.env
  useEffect(() => {
    if (provider.envKeys.length === 0) return;
    const load = async () => {
      // 1. Try saved credentials from install-state.json (wizard's own local store)
      try {
        const saved = await getSavedCredentials(provider.id);
        if (saved && Object.keys(saved).length > 0) {
          setValues(saved);
          return;
        }
      } catch {
        // install-state.json may not have credentials yet
      }

      // 2. Fall back to reading from postiz.env
      if (!installPath) return;
      const loaded: Record<string, string> = {};
      for (const envKey of provider.envKeys) {
        try {
          const val = await readEnvValue(installPath, envKey.key);
          if (val) loaded[envKey.key] = val;
        } catch {
          // env file may not exist yet
        }
      }
      if (Object.keys(loaded).length > 0) {
        setValues(loaded);
      }
    };
    load();
  }, [installPath, provider.id]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !saving) {
        onClose();
        return;
      }

      if (e.key === "Tab" && modalRef.current) {
        const focusable = modalRef.current.querySelectorAll<HTMLElement>(
          'button, input, [tabindex]:not([tabindex="-1"])',
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];

        if (e.shiftKey) {
          if (document.activeElement === first) {
            e.preventDefault();
            last.focus();
          }
        } else {
          if (document.activeElement === last) {
            e.preventDefault();
            first.focus();
          }
        }
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose, saving]);

  const modalTitleId = `provider-modal-title-${provider.id}`;

  const handleSave = async () => {
    setSaving(true);
    try {
      await onSave(values);
      onClose();
    } catch (err) {
      showToast(friendlyError(String(err)), "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div ref={modalRef} className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" role="dialog" aria-modal="true" aria-labelledby={modalTitleId}>
      <div className="bg-white rounded-xl shadow-2xl w-full max-w-2xl max-h-[80vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-gray-200">
          <div className="flex items-center gap-3">
            <ProviderIcon name={provider.icon} />
            <h3 id={modalTitleId} className="text-lg font-semibold text-gray-900">
              {provider.name} Setup
            </h3>
          </div>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-gray-600"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          {/* Portal link — prominent banner */}
          {provider.portalUrl && (
            <div className="rounded-lg bg-blue-50 border border-blue-200 p-4 flex items-center justify-between gap-4">
              <div>
                <p className="text-sm font-medium text-blue-900">Developer Portal</p>
                <p className="text-xs text-blue-700 mt-0.5">Create your app and get your credentials here</p>
              </div>
              <button
                onClick={() => open(provider.portalUrl)}
                className="shrink-0 inline-flex items-center gap-2 rounded-lg bg-blue-600 text-white px-4 py-2 text-sm font-medium hover:bg-blue-700 transition-colors"
              >
                Open Portal <ExternalLink className="h-4 w-4" />
              </button>
            </div>
          )}

          {/* Instructions */}
          <div>
            <h4 className="text-sm font-medium text-gray-700 mb-3">
              Setup Instructions
            </h4>
            <ol className="space-y-3">
              {provider.instructions.map((step, i) => (
                <li key={i} className="flex gap-3">
                  <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-gray-100 text-xs font-medium text-gray-600">
                    {i + 1}
                  </span>
                  <div className="space-y-2 flex-1">
                    <p className="text-sm text-gray-700">{step.text}</p>
                    {step.copyLabel && step.copyLabel !== "Website URL" && callbackUrl && (
                      <CopyField
                        value={callbackUrl}
                        label={step.copyLabel}
                      />
                    )}
                    {step.copyLabel === "Website URL" && homepageUrl && (
                      <CopyField value={homepageUrl} label="Website URL" />
                    )}
                  </div>
                </li>
              ))}
            </ol>
          </div>

          {/* Credentials */}
          {provider.envKeys.length > 0 && (
            <div>
              <h4 className="text-sm font-medium text-gray-700 mb-3">
                Credentials
              </h4>
              <div className="space-y-3">
                {provider.envKeys.map((envKey) => (
                  <Input
                    key={envKey.key}
                    label={envKey.label}
                    secret={envKey.secret}
                    placeholder={`Enter ${envKey.label}`}
                    value={values[envKey.key] ?? ""}
                    onChange={(e) =>
                      setValues({ ...values, [envKey.key]: e.target.value })
                    }
                  />
                ))}
              </div>
            </div>
          )}

          {/* Docs link */}
          <button
            onClick={() => open(provider.docsUrl)}
            className="inline-flex items-center gap-1 text-sm text-blue-600 hover:text-blue-700"
          >
            View full documentation <ExternalLink className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 px-6 py-4 border-t border-gray-200 bg-gray-50">
          <Button variant="ghost" onClick={onClose} disabled={saving}>
            Cancel
          </Button>
          {provider.envKeys.length > 0 ? (
            <Button
              onClick={handleSave}
              disabled={saving || provider.envKeys.some(
                (k) => !values[k.key]?.trim(),
              )}
              loading={saving}
            >
              Save
            </Button>
          ) : (
            <Button
              onClick={handleSave}
              loading={saving}
              disabled={saving}
            >
              Done
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

export function ConnectProviders() {
  const {
    tunnelUrl,
    tunnelMode,
    permanentDomain,
    port,
    installPath,
    providers: providerStatuses,
    setProviderStatus,
    activeProvider,
    setActiveProvider,
    setStep,
  } = useWizardStore();
  const [applying, setApplying] = useState(false);
  const [showMore, setShowMore] = useState(false);
  // Track which providers have been staged but not yet applied
  const [stagedProviders, setStagedProviders] = useState<Set<string>>(new Set());
  const applyOpRef = useRef(0);

  const handleSaveProvider = async (
    provider: ProviderDefinition,
    entries: Record<string, string>,
  ) => {
    if (provider.noEnvNeeded || Object.values(entries).some((v) => v.trim())) {
      await stageProviderConfig(provider.id, entries);
      // Track as staged locally — NOT persisted as configured until apply succeeds
      setStagedProviders((prev) => new Set(prev).add(provider.id));
    }
  };

  const [applyError, setApplyError] = useState<string | null>(null);

  const handleNext = async () => {
    const opId = ++applyOpRef.current;
    // Apply all staged changes transactionally (backup → write → restart → health check → rollback on failure)
    setApplying(true);
    setApplyError(null);
    try {
      // Re-apply tunnel URLs before restarting so Docker picks them up
      if (!usingLocalCallbacks && publicBaseUrl) {
        await updateBaseUrls(installPath, publicBaseUrl);
      }
      await applyConfigTransaction(installPath);
      if (applyOpRef.current !== opId) return; // cancelled
      // Only mark providers as configured after successful apply
      for (const id of stagedProviders) {
        setProviderStatus(id, "configured");
      }
      try {
        await saveResumeState();
      } catch {
        showToast("Provider config applied but failed to save resume state", "info");
      }
      setStagedProviders(new Set());
      setApplying(false);
      setStep(5);
    } catch (err) {
      if (applyOpRef.current !== opId) return; // cancelled
      setApplying(false);
      setApplyError(
        friendlyError(String(err)),
      );
    }
  };

  const activeProviderDef = providers.find((p) => p.id === activeProvider);

  const localBaseUrl = `http://localhost:${port}`;
  const usingLocalCallbacks = tunnelMode === "none";
  const publicBaseUrl =
    tunnelUrl ??
    (tunnelMode === "permanent" && permanentDomain ? permanentDomain : "");
  const missingPublicBaseUrl = !usingLocalCallbacks && !publicBaseUrl;
  const baseUrl = usingLocalCallbacks ? localBaseUrl : publicBaseUrl;
  const popularProviders = providers.filter((p) => p.popular);
  const otherProviders = providers.filter((p) => !p.popular);

  const providerNeedsCallbackBase = (provider: ProviderDefinition) =>
    Boolean(provider.callbackUrlTemplate.trim() || provider.homepageUrlTemplate?.trim());

  const getProviderGate = (provider: ProviderDefinition) => {
    if (provider.gated) {
      return provider.gated;
    }

    if (provider.requiresPermanentDomain && tunnelMode !== "permanent") {
      return "Needs permanent domain";
    }

    if (usingLocalCallbacks) {
      if (
        provider.noEnvNeeded ||
        provider.supportsLocalCallback ||
        !providerNeedsCallbackBase(provider)
      ) {
        return null;
      }
      return "Needs web link";
    }

    if (missingPublicBaseUrl && providerNeedsCallbackBase(provider)) {
      return "Web link unavailable";
    }

    return null;
  };

  const renderProviderCard = (provider: ProviderDefinition) => {
    const status = stagedProviders.has(provider.id)
      ? "staged" as const
      : providerStatuses[provider.id] ?? "unconfigured";
    const gateReason = getProviderGate(provider);
    const isGated = gateReason !== null;
    const isNoEnv = !!provider.noEnvNeeded;

    return (
      <button
        key={provider.id}
        onClick={() => !isGated && setActiveProvider(provider.id)}
        disabled={isGated}
        className={`relative flex items-center gap-3 rounded-xl border p-4 text-left transition-colors ${
          isGated
            ? "border-gray-200 bg-gray-50 opacity-60 cursor-not-allowed"
            : status === "configured"
              ? "border-green-200 bg-green-50 hover:bg-green-100"
              : status === "staged"
                ? "border-blue-200 bg-blue-50 hover:bg-blue-100"
                : status === "stale"
                  ? "border-amber-200 bg-amber-50 hover:bg-amber-100"
                  : "border-gray-200 bg-white hover:bg-gray-50"
        }`}
      >
        <ProviderIcon name={provider.icon} />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-gray-900 truncate">
            {provider.name}
          </p>
          <p className="text-xs text-gray-500">
            {isGated
              ? gateReason
              : isNoEnv && status !== "configured" && status !== "staged"
                ? "No setup needed"
                : status === "configured"
                  ? "Configured"
                  : status === "staged"
                    ? "Saved — will apply on continue"
                    : status === "stale"
                      ? "Needs update"
                      : "Not configured"}
          </p>
        </div>
        {status === "configured" && (
          <Check className="h-4 w-4 text-green-600 shrink-0" />
        )}
        {status === "staged" && (
          <Clock className="h-4 w-4 text-blue-600 shrink-0" />
        )}
        {status === "stale" && (
          <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0" />
        )}
        {isGated && (
          <Lock className="h-4 w-4 text-gray-400 shrink-0" />
        )}
      </button>
    );
  };

  return (
    <div className="max-w-3xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        Connect social platforms
      </h2>

      {usingLocalCallbacks ? (
        <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-4 mb-6">
          <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
          <div className="text-sm text-amber-800">
            <p className="font-medium mb-1">Using local callback URLs</p>
            <p>
              Postiz can still use{" "}
              <span className="font-mono">http://localhost:{port}</span> for
              many provider callback URLs. Providers that require a permanent
              public domain, such as TikTok, will stay locked until you
              configure one in the Web Link step.
            </p>
          </div>
        </div>
      ) : missingPublicBaseUrl ? (
        <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-4 mb-6">
          <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
          <div className="text-sm text-amber-800">
            <p className="font-medium mb-1">Your web link is unavailable right now</p>
            <p>
              Reconnect or repair the Web Link step before setting up providers
              that need callback URLs. Providers that do not use callback URLs
              can still be configured.
            </p>
          </div>
        </div>
      ) : (
        <p className="text-gray-600 mb-6">
          Most people only need 1 or 2 platforms to get started. You can always
          add more later from this screen.
        </p>
      )}

      {/* Popular / recommended providers */}
      <div className="mb-4">
        <h3 className="text-sm font-medium text-gray-700 mb-2">
          Popular platforms
        </h3>
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
          {popularProviders.map(renderProviderCard)}
        </div>
      </div>

      {/* More platforms (collapsed) */}
      <div className="mb-6">
        <button
          onClick={() => setShowMore(!showMore)}
          className="inline-flex items-center gap-1.5 text-sm text-gray-600 hover:text-gray-800 mb-2"
        >
          <ChevronDown
            className={`h-3.5 w-3.5 transition-transform ${showMore ? "rotate-180" : ""}`}
          />
          {showMore ? "Hide" : "Show"} more platforms ({otherProviders.length})
        </button>
        {showMore && (
          <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
            {otherProviders.map(renderProviderCard)}
          </div>
        )}
      </div>

      {/* Gated provider note */}
      {(tunnelMode !== "permanent" || missingPublicBaseUrl) && (
        <Card className="mb-6">
          <p className="text-xs text-gray-500">
            {missingPublicBaseUrl
              ? "Your current web link is not available, so callback-based providers stay locked until that is fixed."
              : "Some platforms (like TikTok) require a permanent domain. Go back to the Web Link step and configure your own domain to enable them."}
          </p>
        </Card>
      )}

      {activeProviderDef && (
        <ProviderModal
          provider={activeProviderDef}
          baseUrl={baseUrl}
          installPath={installPath}
          onClose={() => setActiveProvider(null)}
          onSave={(entries) =>
            handleSaveProvider(activeProviderDef, entries)
          }
        />
      )}

      {applying && (
        <div className="mb-4">
          <Button
            variant="secondary"
            onClick={async () => {
              applyOpRef.current++;
              try { await cancelDockerOperation(); } catch {}
              setApplying(false);
            }}
          >
            Cancel
          </Button>
        </div>
      )}

      {applyError && (
        <div className="rounded-lg bg-red-50 border border-red-200 p-4 mb-6">
          <p className="text-sm text-red-700 font-medium mb-1">
            Failed to apply provider changes
          </p>
          <p className="text-sm text-red-600 mb-3">{applyError}</p>
          <Button variant="secondary" onClick={handleNext}>
            Retry
          </Button>
        </div>
      )}

      <NavigationButtons
        canProceed={true}
        nextLabel="Apply & Continue"
        loading={applying}
        onNext={handleNext}
      />
    </div>
  );
}
