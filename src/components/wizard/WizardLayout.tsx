import { useWizardStore } from "../../store/wizardStore";
import { StepIndicator } from "./StepIndicator";

const steps = [
  { name: "Prepare Computer", description: "Check prerequisites" },
  { name: "Install Postiz", description: "Download and set up" },
  { name: "Create Account", description: "Set up your admin account" },
  { name: "Create Web Link", description: "Set up public access" },
  { name: "Connect Platforms", description: "Configure social media" },
  { name: "Setup Complete", description: "You're all set!" },
];

interface WizardLayoutProps {
  children: React.ReactNode;
}

export function WizardLayout({ children }: WizardLayoutProps) {
  const currentStep = useWizardStore((s) => s.currentStep);
  const setStep = useWizardStore((s) => s.setStep);

  return (
    <div className="flex h-screen">
      {/* Sidebar */}
      <div className="w-64 shrink-0 border-r border-gray-200 bg-white p-6 flex flex-col">
        <div className="mb-8">
          <h1 className="text-lg font-semibold text-gray-900">Postiz</h1>
          <p className="text-xs text-gray-500">Setup Wizard</p>
        </div>

        <nav className="flex-1 space-y-1">
          {steps.map((step, index) => (
            <StepIndicator
              key={index}
              stepNumber={index + 1}
              name={step.name}
              description={step.description}
              state={
                index < currentStep
                  ? "complete"
                  : index === currentStep
                    ? "active"
                    : "pending"
              }
              onClick={
                index < currentStep ? () => setStep(index) : undefined
              }
            />
          ))}
        </nav>

        <div className="pt-4 border-t border-gray-200">
          <p className="text-xs text-gray-400">v0.1.0</p>
        </div>
      </div>

      {/* Main content */}
      <div className="flex-1 overflow-y-auto bg-gray-50 p-8">{children}</div>
    </div>
  );
}
