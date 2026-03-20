interface StatusIndicatorProps {
  status: "success" | "warning" | "error" | "idle" | "loading";
  label: string;
  detail?: string;
}

const statusColors = {
  success: "bg-green-500",
  warning: "bg-yellow-500",
  error: "bg-red-500",
  idle: "bg-gray-300",
  loading: "bg-blue-500 animate-pulse",
};

export function StatusIndicator({
  status,
  label,
  detail,
}: StatusIndicatorProps) {
  return (
    <div className="flex items-center gap-3 py-1.5">
      <span
        className={`inline-block h-2.5 w-2.5 rounded-full shrink-0 ${statusColors[status]}`}
      />
      <div className="min-w-0">
        <span className="text-sm text-gray-900">{label}</span>
        {detail && (
          <span className="ml-2 text-xs text-gray-500">{detail}</span>
        )}
      </div>
    </div>
  );
}
