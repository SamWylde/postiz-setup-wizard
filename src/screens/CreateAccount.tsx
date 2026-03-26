import { useEffect, useState, useRef } from "react";
import { useWizardStore } from "../store/wizardStore";
import {
  getStackStatus,
  savePostizCredentials,
  getPostizCredentials,
  saveResumeState,
} from "../lib/tauri";
import { open } from "@tauri-apps/plugin-shell";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { NavigationButtons } from "../components/wizard/NavigationButtons";
import { showToast } from "../components/ui/Toast";

const HEALTH_POLL_INTERVAL_MS = 3_000;
const MAX_HEALTH_ATTEMPTS = 60;

export function CreateAccount() {
  const {
    port,
    installPath,
    postizReady,
    setPostizReady,
    accountCreated,
    setAccountCreated,
    setStep,
  } = useWizardStore();
  const [checking, setChecking] = useState(true);
  const [verifying, setVerifying] = useState(false);
  const [healthTimedOut, setHealthTimedOut] = useState(false);
  const [retryCount, setRetryCount] = useState(0);
  const [credEmail, setCredEmail] = useState("");
  const [credPassword, setCredPassword] = useState("");
  const [credsSaved, setCredsSaved] = useState(false);
  const credsLoaded = useRef(false);

  const localUrl = `http://localhost:${port}`;

  // Load any previously saved credentials
  useEffect(() => {
    if (credsLoaded.current) return;
    credsLoaded.current = true;
    getPostizCredentials().then(([email, password]) => {
      setCredEmail(email ?? "");
      setCredPassword(password ?? "");
      setCredsSaved(Boolean(email && password));
    }).catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;

    // ~3 minutes at 3s intervals

    const pollHealth = async () => {
      let attempts = 0;
      while (!cancelled && attempts < MAX_HEALTH_ATTEMPTS) {
        attempts++;
        try {
          const status = await getStackStatus(installPath);
          if (status.postiz_responding) {
            setPostizReady(true);
            setChecking(false);
            return;
          }
        } catch {
          // Keep polling
        }
        await new Promise((r) => setTimeout(r, HEALTH_POLL_INTERVAL_MS));
      }
      if (!cancelled) {
        setChecking(false);
        setHealthTimedOut(true);
        showToast(
          "Postiz did not respond after ~3 minutes. Check the Docker logs and try again.",
          "error",
        );
      }
    };

    if (!postizReady) {
      pollHealth();
    } else {
      setChecking(false);
    }

    return () => {
      cancelled = true;
    };
  }, [retryCount]);

  const handleOpenPostiz = async () => {
    try {
      await open(localUrl);
    } catch (err) {
      showToast(`Could not open URL: ${String(err)}`, "error");
    }
  };

  const handleAccountCreated = async () => {
    setVerifying(true);
    try {
      // We can't verify account creation itself — only that Postiz is still healthy
      const status = await getStackStatus(installPath);
      if (status.postiz_responding) {
        setAccountCreated(true);
        showToast("Postiz is running — proceeding to next step.", "success");
      } else {
        showToast(
          "Postiz isn't responding. Please wait a moment and try again.",
          "error",
        );
      }
    } catch {
      // Transient network issue shouldn't block setup
      setAccountCreated(true);
    }
    setVerifying(false);
  };

  return (
    <div className="max-w-2xl">
      <h2 className="text-2xl font-semibold text-gray-900 mb-2">
        Create your Postiz account
      </h2>
      <p className="text-gray-600 mb-6">
        Open Postiz in your browser and create your admin account.
      </p>

      <Card className="mb-6">
        <div className="space-y-4">
          <StatusIndicator
            status={postizReady ? "success" : checking ? "loading" : "error"}
            label={
              postizReady
                ? "Postiz is ready"
                : healthTimedOut
                  ? "Postiz did not respond after ~3 minutes"
                  : "Waiting for Postiz to start..."
            }
            detail={postizReady ? localUrl : undefined}
          />

          {healthTimedOut && (
            <Button
              variant="secondary"
              onClick={() => {
                setHealthTimedOut(false);
                setChecking(true);
                setRetryCount((c) => c + 1);
              }}
            >
              Retry
            </Button>
          )}

          {postizReady && (
            <>
              <p className="text-sm text-gray-600">
                Postiz is available at{" "}
                <button onClick={handleOpenPostiz} className="text-blue-600 hover:text-blue-700 underline font-mono">
                  {localUrl}
                </button>
              </p>
              <Button onClick={handleOpenPostiz}>Open Postiz</Button>

              <div className="bg-blue-50 rounded-lg p-4">
                <p className="text-sm text-blue-800 font-medium mb-2">
                  Steps to create your account:
                </p>
                <ol className="text-sm text-blue-800 space-y-1 list-decimal list-inside">
                  <li>Click "Open Postiz" above to open it in your browser</li>
                  <li>Create your admin account (email and password)</li>
                  <li>Come back here and click the button below</li>
                </ol>
              </div>

              {!accountCreated && (
                <Button
                  variant="secondary"
                  onClick={handleAccountCreated}
                  loading={verifying}
                >
                  I've created my account — continue
                </Button>
              )}

              {accountCreated && (
                <div className="bg-green-50 rounded-lg border border-green-200 p-4 space-y-3">
                  <p className="text-sm font-medium text-green-800">
                    Save your login credentials
                  </p>
                  <p className="text-sm text-green-700">
                    Enter the email and password you just used to create your
                    Postiz account. We'll save them encrypted on this computer so you can easily
                    copy them later (e.g. when the URL changes).
                  </p>
                  <input
                    type="email"
                    className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-400 focus:outline-none focus:ring-1 focus:ring-blue-400"
                    placeholder="Email"
                    value={credEmail}
                    onChange={(e) => {
                      setCredEmail(e.target.value);
                      if (credsSaved) setCredsSaved(false);
                    }}
                  />
                  <input
                    type="password"
                    className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-400 focus:outline-none focus:ring-1 focus:ring-blue-400"
                    placeholder="Password"
                    value={credPassword}
                    onChange={(e) => {
                      setCredPassword(e.target.value);
                      if (credsSaved) setCredsSaved(false);
                    }}
                  />
                  <div className="flex items-center gap-2">
                    <Button
                      variant="secondary"
                      disabled={!credEmail.trim() || !credPassword.trim() || credsSaved}
                      onClick={async () => {
                        await savePostizCredentials(credEmail.trim(), credPassword.trim());
                        await saveResumeState();
                        setCredsSaved(true);
                        showToast("Credentials saved securely on this computer", "success");
                      }}
                    >
                      {credsSaved ? "Saved" : "Save Credentials"}
                    </Button>
                    {!credsSaved && (
                      <span className="text-xs text-gray-500">Optional — you can skip this</span>
                    )}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </Card>

      <NavigationButtons
        canProceed={accountCreated}
        onNext={() => setStep(3)}
      />
    </div>
  );
}
