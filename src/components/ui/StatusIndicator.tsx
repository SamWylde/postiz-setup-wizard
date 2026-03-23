interface StatusIndicatorProps {
  status: "success" | "warning" | "error" | "idle" | "loading";
  label: string;
  detail?: string;
}

const dotColors = {
  success: "bg-green-500",
  warning: "bg-yellow-500",
  error: "bg-red-500",
  idle: "bg-gray-300",
  loading: "bg-blue-500 animate-pulse",
};

const detailColors = {
  success: "text-green-600",
  warning: "text-amber-600",
  error: "text-red-600",
  idle: "text-gray-500",
  loading: "text-blue-600",
};

export function StatusIndicator({
  status,
  label,
  detail,
}: StatusIndicatorProps) {
  return (
    <div className="flex items-center gap-3 py-1.5">
      <span
        className={`inline-block h-2.5 w-2.5 rounded-full shrink-0 ${dotColors[status]}`}
      />
      <div className="min-w-0">
        <span className="text-sm text-gray-900">{label}</span>
        {detail && (
          <span className={`ml-2 text-xs font-medium ${detailColors[status]}`}>
            {detail}
          </span>
        )}
      </div>
    </div>
  );
}
