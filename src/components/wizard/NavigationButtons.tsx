import { useCallback, useEffect, useRef } from "react";
import { useWizardStore } from "../../store/wizardStore";
import { Button } from "../ui/Button";

interface NavigationButtonsProps {
  canProceed?: boolean;
  nextLabel?: string;
  onNext?: () => void | Promise<void>;
  showBack?: boolean;
  loading?: boolean;
}

export function NavigationButtons({
  canProceed = true,
  nextLabel = "Next",
  onNext,
  showBack = true,
  loading = false,
}: NavigationButtonsProps) {
  const currentStep = useWizardStore((s) => s.currentStep);
  const setStep = useWizardStore((s) => s.setStep);

  // Use refs so the keyboard handler always sees the latest values
  // without re-registering the event listener on every render
  const onNextRef = useRef(onNext);
  onNextRef.current = onNext;
  const canProceedRef = useRef(canProceed);
  canProceedRef.current = canProceed;
  const loadingRef = useRef(loading);
  loadingRef.current = loading;

  const handleNext = useCallback(async () => {
    if (onNextRef.current) {
      await onNextRef.current();
    } else {
      setStep(currentStep + 1);
    }
  }, [currentStep, setStep]);

  const handleBack = useCallback(() => {
    if (currentStep > 0) {
      setStep(currentStep - 1);
    }
  }, [currentStep, setStep]);

  // Keyboard navigation: Enter = Next, Escape = Back
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      // Don't capture keys when a modal/dialog overlay is open
      if (document.querySelector("[role='dialog'], .fixed.inset-0.z-50")) return;

      if (e.key === "Enter" && canProceedRef.current && !loadingRef.current) {
        e.preventDefault();
        handleNext();
      } else if (e.key === "Escape" && currentStep > 0 && !loadingRef.current) {
        e.preventDefault();
        handleBack();
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [currentStep, handleNext, handleBack]);

  return (
    <div className="flex items-center justify-between pt-6 mt-6 border-t border-gray-200">
      {showBack && currentStep > 0 ? (
        <Button variant="ghost" onClick={handleBack} disabled={loading}>
          Back
        </Button>
      ) : (
        <div />
      )}
      <Button
        onClick={handleNext}
        disabled={!canProceed || loading}
        loading={loading}
      >
        {nextLabel}
      </Button>
    </div>
  );
}
