import { useState } from "react";
import { Copy, Check, ExternalLink, Eye, EyeOff } from "lucide-react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open } from "@tauri-apps/plugin-shell";

interface CopyFieldProps {
  value: string;
  label?: string;
  openable?: boolean;
  secret?: boolean;
}

export function CopyField({ value, label, openable, secret = false }: CopyFieldProps) {
  const isUrl = openable ?? value.startsWith("http");
  const [copied, setCopied] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const displayValue =
    secret && !revealed ? "•".repeat(Math.max(value.length, 8)) : value;

  const handleCopy = async () => {
    try {
      await writeText(value);
    } catch {
      try {
        // Fallback to navigator clipboard
        await navigator.clipboard.writeText(value);
      } catch {
        // Both clipboard APIs failed — don't show false success
        return;
      }
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="space-y-1.5">
      {label && (
        <label className="block text-sm font-medium text-gray-700">
          {label}
        </label>
      )}
      <div className="flex items-center gap-2">
        <div className="flex-1 rounded-lg border border-gray-300 bg-gray-50 px-3 py-2 text-sm font-mono text-gray-900 select-all overflow-x-auto">
          {displayValue}
        </div>
        {secret && (
          <button
            onClick={() => setRevealed((prev) => !prev)}
            className="shrink-0 rounded-lg border border-gray-300 p-2 text-gray-500 hover:bg-gray-50 hover:text-gray-700 transition-colors"
            title={revealed ? "Hide value" : "Show value"}
          >
            {revealed ? (
              <EyeOff className="h-4 w-4" />
            ) : (
              <Eye className="h-4 w-4" />
            )}
          </button>
        )}
        {isUrl && (
          <button
            onClick={() => open(value)}
            className="shrink-0 rounded-lg border border-gray-300 p-2 text-gray-500 hover:bg-gray-50 hover:text-blue-600 transition-colors"
            title="Open in browser"
          >
            <ExternalLink className="h-4 w-4" />
          </button>
        )}
        <button
          onClick={handleCopy}
          className="shrink-0 rounded-lg border border-gray-300 p-2 text-gray-500 hover:bg-gray-50 hover:text-gray-700 transition-colors"
          title="Copy to clipboard"
        >
          {copied ? (
            <Check className="h-4 w-4 text-green-500" />
          ) : (
            <Copy className="h-4 w-4" />
          )}
        </button>
      </div>
    </div>
  );
}
