import { useState } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  stageProviderConfig,
  applyConfigTransaction,
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
import { open } from "@tauri-apps/plugin-shell";
import {
  ExternalLink,
  X,
  Check,
  AlertTriangle,
  Lock,
  ChevronDown,
} from "lucide-react";
import * as icons from "lucide-react";

function ProviderIcon({ name }: { name: string }) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const Icon = (icons as any)[name];
  if (!Icon) return <div className="h-5 w-5 bg-gray-300 rounded" />;
  return <Icon className="h-5 w-5" />;
}

interface ProviderModalProps {
  provider: ProviderDefinition;
  baseUrl: string;
  onClose: () => void;
  onSave: (entries: Record<string, string>) => void;
}

function ProviderModal({
  provider,
  baseUrl,
  onClose,
  onSave,
}: ProviderModalProps) {
  const [values, setValues] = useState<Record<string, string>>({});
  const callbackUrl = getCallbackUrl(provider, baseUrl);
  const homepageUrl = getHomepageUrl(provider, baseUrl);

  const handleSave = () => {
    onSave(values);
    onClose();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" role="dialog" aria-modal="true">
      <div className="bg-white rounded-xl shadow-2xl w-full max-w-2xl max-h-[80vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-gray-200">
          <div className="flex items-center gap-3">
            <ProviderIcon name={provider.icon} />
            <h3 className="text-lg font-semibold text-gray-900">
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
          {/* Instructions */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <h4 className="text-sm font-medium text-gray-700">
                Setup Instructions
              </h4>
              {provider.portalUrl && (
                <button
                  onClick={() => open(provider.portalUrl)}
                  className="inline-flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"
                >
                  Open portal <ExternalLink className="h-3 w-3" />
                </button>
              )}
            </div>
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
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          {provider.envKeys.length > 0 ? (
            <Button
              onClick={handleSave}
              disabled={provider.envKeys.some(
                (k) => !values[k.key]?.trim(),
              )}
            >
              Save
            </Button>
          ) : (
            <Button
              onClick={() => {
                onSave({});
                onClose();
              }}
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

  const baseUrl = tunnelUrl ?? "";

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
    // Apply all staged changes transactionally (backup → write → restart → health check → rollback on failure)
    setApplying(true);
    setApplyError(null);
    try {
      await applyConfigTransaction(installPath);
      // Only mark providers as configured after successful apply
      for (const id of stagedProviders) {
        setProviderStatus(id, "configured");
      }
      setStagedProviders(new Set());
      setApplying(false);
      setStep(5);
    } catch (err) {
      setApplying(false);
      setApplyError(
        friendlyError(String(err)),
      );
    }
  };

  const activeProviderDef = providers.find((p) => p.id === activeProvider);

  const noPublicUrl = tunnelMode === "none" || !baseUrl;
  const popularProviders = providers.filter((p) => p.popular);
  const otherProviders = providers.filter((p) => !p.popular);

  const renderProviderCard = (provider: ProviderDefinition) => {
    const status = stagedProviders.has(provider.id)
      ? "configured" as const
      : providerStatuses[provider.id] ?? "unconfigured";
    const isGated = noPublicUrl || (provider.requiresPermanentDomain
      ? tunnelMode !== "permanent"
      : !!provider.gated);
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
              ? provider.requiresPermanentDomain
                ? "Needs permanent domain"
                : noPublicUrl
                  ? "Needs web link"
                  : "Not available"
              : isNoEnv && status !== "configured"
                ? "No setup needed"
                : status === "configured"
                  ? "Configured"
                  : status === "stale"
                    ? "Needs update"
                    : "Not configured"}
          </p>
        </div>
        {status === "configured" && (
          <Check className="h-4 w-4 text-green-600 shrink-0" />
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

      {noPublicUrl ? (
        <>
          <div className="flex items-start gap-3 rounded-lg bg-amber-50 border border-amber-200 p-4 mb-6">
            <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
            <div className="text-sm text-amber-800">
              <p className="font-medium mb-1">No public URL configured</p>
              <p>
                Social media platforms need a public URL to connect with Postiz.
                You're using local-only mode, so you can skip this step for now.
                Go back to the previous step to create a web link if you'd like
                to connect platforms.
              </p>
            </div>
          </div>
        </>
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
      {tunnelMode !== "permanent" && !noPublicUrl && (
        <Card className="mb-6">
          <p className="text-xs text-gray-500">
            Some platforms (like TikTok) require a permanent domain. Go back to
            the Web Link step and configure your own domain to enable them.
          </p>
        </Card>
      )}

      {activeProviderDef && (
        <ProviderModal
          provider={activeProviderDef}
          baseUrl={baseUrl}
          onClose={() => setActiveProvider(null)}
          onSave={(entries) =>
            handleSaveProvider(activeProviderDef, entries)
          }
        />
      )}

      {applyError && (
        <div className="rounded-lg bg-red-50 border border-red-200 p-4 mb-6">
          <p className="text-sm text-red-700 font-medium mb-1">
            Failed to apply provider changes
          </p>
          <p className="text-sm text-red-600">{applyError}</p>
        </div>
      )}

      <NavigationButtons
        canProceed={true}
        nextLabel={noPublicUrl ? "Skip & Continue" : "Apply & Continue"}
        loading={applying}
        onNext={handleNext}
      />
    </div>
  );
}
