import { useEffect, useState } from "react";
import { CheckCircle, XCircle, Info, X } from "lucide-react";

export interface ToastData {
  id: string;
  message: string;
  type: "success" | "error" | "info";
}

let addToastFn: ((toast: Omit<ToastData, "id">) => void) | null = null;

export function showToast(message: string, type: ToastData["type"] = "info") {
  addToastFn?.({ message, type });
}

export function ToastContainer() {
  const [toasts, setToasts] = useState<ToastData[]>([]);

  useEffect(() => {
    addToastFn = (toast) => {
      const id = Math.random().toString(36).slice(2);
      setToasts((prev) => [...prev, { ...toast, id }]);
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 4000);
    };
    return () => {
      addToastFn = null;
    };
  }, []);

  if (toasts.length === 0) return null;

  const iconMap = {
    success: <CheckCircle className="h-4 w-4 text-green-500 shrink-0" />,
    error: <XCircle className="h-4 w-4 text-red-500 shrink-0" />,
    info: <Info className="h-4 w-4 text-blue-500 shrink-0" />,
  };

  return (
    <div className="fixed bottom-4 right-4 z-[100] space-y-2">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          className="toast-enter flex items-center gap-3 rounded-lg border border-gray-200 bg-white px-4 py-3 shadow-lg max-w-sm"
        >
          {iconMap[toast.type]}
          <p className="text-sm text-gray-700 flex-1">{toast.message}</p>
          <button
            onClick={() =>
              setToasts((prev) => prev.filter((t) => t.id !== toast.id))
            }
            className="text-gray-400 hover:text-gray-600 shrink-0"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
