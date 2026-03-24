import { Check, Loader2, Circle, AlertCircle } from "lucide-react";
import type { PullProgress } from "../../lib/tauri";

export type InstallPhase =
  | "idle"
  | "preflight"
  | "preparing-files"
  | "pulling"
  | "starting-services"
  | "health-checks"
  | "ready"
  | "error";

interface PhaseDefinition {
  id: InstallPhase;
  label: string;
  guidance: string;
  timing?: string;
}

const PHASES: PhaseDefinition[] = [
  {
    id: "preflight",
    label: "Pre-flight checks",
    guidance: "Verifying Docker is running and disk space is available.",
    timing: "A few seconds",
  },
  {
    id: "preparing-files",
    label: "Preparing files",
    guidance: "Generating secrets and writing configuration files.",
    timing: "A few seconds",
  },
  {
    id: "pulling",
    label: "Downloading images",
    guidance:
      "Pulling Docker images for Postiz, PostgreSQL, Redis, Temporal, and Elasticsearch. This is the longest step — 3 to 10 minutes depending on your internet speed.",
    timing: "3–10 minutes",
  },
  {
    id: "starting-services",
    label: "Starting services",
    guidance:
      "Launching all containers. Some services depend on others, so startup order matters.",
    timing: "30–60 seconds",
  },
  {
    id: "health-checks",
    label: "Running health checks",
    guidance:
      "Waiting for all services to start and for Postiz to respond. This is normal — databases and search engines can take a few minutes to initialize on first install.",
    timing: "1–4 minutes",
  },
  {
    id: "ready",
    label: "Ready",
    guidance: "Postiz is installed and running!",
  },
];

function phaseIndex(phase: InstallPhase): number {
  return PHASES.findIndex((p) => p.id === phase);
}

type PhaseState = "complete" | "active" | "pending" | "error";

function getPhaseState(
  phaseDef: PhaseDefinition,
  currentPhase: InstallPhase,
  errorPhase: InstallPhase | null,
): PhaseState {
  const currentIdx = phaseIndex(currentPhase);
  const thisIdx = phaseIndex(phaseDef.id);

  if (errorPhase && phaseDef.id === errorPhase) return "error";
  if (currentPhase === "ready" || thisIdx < currentIdx) return "complete";
  if (thisIdx === currentIdx) return "active";
  return "pending";
}

interface InstallTimelineProps {
  currentPhase: InstallPhase;
  errorPhase?: InstallPhase | null;
  elapsed: number;
  progressDetail?: string;
  errorMessage?: string | null;
  pullProgress?: PullProgress | null;
}

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m === 0) return `${s}s`;
  return `${m}m ${s}s`;
}

export function InstallTimeline({
  currentPhase,
  errorPhase = null,
  elapsed,
  progressDetail,
  errorMessage,
  pullProgress,
}: InstallTimelineProps) {
  if (currentPhase === "idle") return null;

  return (
    <div className="space-y-0">
      {PHASES.map((phase, idx) => {
        const state = getPhaseState(phase, currentPhase, errorPhase);
        const isLast = idx === PHASES.length - 1;

        return (
          <div key={phase.id} className="flex gap-3">
            {/* Vertical line + icon */}
            <div className="flex flex-col items-center">
              <PhaseIcon state={state} />
              {!isLast && (
                <div
                  className={`w-0.5 flex-1 min-h-6 ${
                    state === "complete"
                      ? "bg-green-300"
                      : state === "active"
                        ? "bg-blue-200"
                        : state === "error"
                          ? "bg-red-200"
                          : "bg-gray-200"
                  }`}
                />
              )}
            </div>

            {/* Content */}
            <div className={`pb-4 ${isLast ? "pb-0" : ""}`}>
              <p
                className={`text-sm font-medium ${
                  state === "complete"
                    ? "text-green-700"
                    : state === "active"
                      ? "text-blue-700"
                      : state === "error"
                        ? "text-red-700"
                        : "text-gray-400"
                }`}
              >
                {phase.label}
                {state === "active" && phase.timing && (
                  <span className="font-normal text-gray-400 ml-2">
                    ({phase.timing})
                  </span>
                )}
              </p>

              {state === "active" && (
                <div className="mt-1 space-y-1">
                  <p className="text-xs text-gray-500">{phase.guidance}</p>

                  {/* Pull progress bar and service list */}
                  {phase.id === "pulling" && pullProgress && pullProgress.total_layers > 0 ? (
                    <div className="space-y-2">
                      <p className="text-xs text-blue-600 font-medium">
                        {pullProgress.message}
                      </p>
                      <div className="flex items-center gap-2.5">
                        <div className="flex-1 h-3 bg-gray-200 rounded-full overflow-hidden">
                          <div
                            className="h-full bg-gradient-to-r from-blue-500 to-blue-400 rounded-full transition-all duration-500 ease-out"
                            style={{ width: `${Math.round((pullProgress.completed_layers / pullProgress.total_layers) * 100)}%` }}
                          />
                        </div>
                        <span className="text-xs text-blue-700 font-semibold tabular-nums whitespace-nowrap min-w-[3ch] text-right">
                          {Math.round((pullProgress.completed_layers / pullProgress.total_layers) * 100)}%
                        </span>
                      </div>
                      {pullProgress.completed_services.length > 0 && (
                        <div className="flex flex-wrap gap-x-3 gap-y-0.5">
                          {pullProgress.completed_services.map((svc) => (
                            <span key={svc} className="text-xs text-green-600 font-medium">
                              &#10003; {svc}
                            </span>
                          ))}
                        </div>
                      )}
                      <p className="text-xs text-gray-400">
                        {formatElapsed(elapsed)} elapsed
                      </p>
                    </div>
                  ) : (
                    <p className="text-xs text-blue-600 font-medium">
                      {formatElapsed(elapsed)}
                      {progressDetail ? ` — ${progressDetail}` : ""}
                    </p>
                  )}
                </div>
              )}

              {state === "error" && errorMessage && (
                <div className="mt-1">
                  <p className="text-xs text-red-600">{errorMessage}</p>
                </div>
              )}

              {state === "active" && currentPhase === "health-checks" && elapsed > 120 && (
                <div className="mt-2 rounded bg-amber-50 px-2.5 py-1.5">
                  <p className="text-xs text-amber-700">
                    Taking longer than usual. Check Docker Desktop for container errors. If a container is restarting in a loop, try clicking "Try again" below.
                  </p>
                </div>
              )}

              {state === "active" && currentPhase === "pulling" && elapsed > 600 && (
                <div className="mt-2 rounded bg-amber-50 px-2.5 py-1.5">
                  <p className="text-xs text-amber-700">
                    Still downloading — this can be slow on limited connections. If it seems stuck, check your internet connection or try canceling and restarting.
                  </p>
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function PhaseIcon({ state }: { state: PhaseState }) {
  switch (state) {
    case "complete":
      return (
        <div className="flex h-5 w-5 items-center justify-center rounded-full bg-green-500 shrink-0">
          <Check className="h-3 w-3 text-white" strokeWidth={3} />
        </div>
      );
    case "active":
      return (
        <div className="flex h-5 w-5 items-center justify-center rounded-full bg-blue-500 shrink-0">
          <Loader2 className="h-3 w-3 text-white animate-spin" />
        </div>
      );
    case "error":
      return (
        <div className="flex h-5 w-5 items-center justify-center rounded-full bg-red-500 shrink-0">
          <AlertCircle className="h-3 w-3 text-white" />
        </div>
      );
    case "pending":
      return (
        <div className="flex h-5 w-5 items-center justify-center shrink-0">
          <Circle className="h-3 w-3 text-gray-300" />
        </div>
      );
  }
}
