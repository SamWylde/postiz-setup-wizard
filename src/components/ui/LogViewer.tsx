import { useEffect, useRef } from "react";

interface LogViewerProps {
  logs: string[];
  maxHeight?: string;
}

export function LogViewer({ logs, maxHeight = "200px" }: LogViewerProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs.length]);

  return (
    <div
      className="rounded-lg bg-gray-900 p-3 font-mono text-xs text-gray-300 overflow-auto"
      style={{ maxHeight }}
    >
      {logs.length === 0 && (
        <span className="text-gray-500">No logs yet...</span>
      )}
      {logs.map((line, i) => (
        <div key={i} className="whitespace-pre-wrap break-all">
          {line}
        </div>
      ))}
      <div ref={bottomRef} />
    </div>
  );
}
