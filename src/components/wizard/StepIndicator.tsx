import { Check } from "lucide-react";

interface StepIndicatorProps {
  stepNumber: number;
  name: string;
  description: string;
  state: "pending" | "active" | "complete";
  onClick?: () => void;
}

export function StepIndicator({
  stepNumber,
  name,
  description,
  state,
  onClick,
}: StepIndicatorProps) {
  const isClickable = state === "complete" && onClick;

  return (
    <div
      onClick={isClickable ? onClick : undefined}
      className={`flex items-start gap-3 rounded-lg px-3 py-2.5 transition-colors ${
        state === "active"
          ? "bg-blue-50"
          : state === "complete"
            ? "opacity-60 hover:opacity-80 hover:bg-gray-50"
            : "opacity-40"
      } ${isClickable ? "cursor-pointer" : ""}`}
    >
      <div
        className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs font-medium ${
          state === "complete"
            ? "bg-green-100 text-green-700"
            : state === "active"
              ? "bg-blue-600 text-white"
              : "bg-gray-200 text-gray-500"
        }`}
      >
        {state === "complete" ? <Check className="h-3.5 w-3.5" /> : stepNumber}
      </div>
      <div className="min-w-0">
        <p
          className={`text-sm font-medium ${state === "active" ? "text-blue-900" : "text-gray-700"}`}
        >
          {name}
        </p>
        <p className="text-xs text-gray-500 truncate">{description}</p>
      </div>
    </div>
  );
}
