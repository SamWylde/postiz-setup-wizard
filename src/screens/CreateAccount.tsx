import { useEffect, useState } from "react";
import { useWizardStore } from "../store/wizardStore";
import { getStackStatus } from "../lib/tauri";
import { open } from "@tauri-apps/plugin-shell";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusIndicator } from "../components/ui/StatusIndicator";
import { NavigationButtons } from "../components/wizard/NavigationButtons";
import { showToast } from "../components/ui/Toast";

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

  const localUrl = `http://localhost:${port}`;

  useEffect(() => {
    let cancelled = false;

    const pollHealth = async () => {
      while (!cancelled) {
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
        await new Promise((r) => setTimeout(r, 3000));
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
  }, []);

  const handleOpenPostiz = () => {
    open(localUrl);
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
                : "Waiting for Postiz to start..."
            }
            detail={postizReady ? localUrl : undefined}
          />

          {postizReady && (
            <>
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

              <Button
                variant={accountCreated ? "ghost" : "secondary"}
                onClick={handleAccountCreated}
                loading={verifying}
                disabled={accountCreated}
              >
                {accountCreated
                  ? "Ready to continue"
                  : "I've created my account — continue"}
              </Button>
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
