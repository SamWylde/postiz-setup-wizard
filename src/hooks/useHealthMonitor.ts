import { useEffect, useRef, useCallback, useState } from "react";
import { getInstallSnapshot, type InstallSnapshot } from "../lib/tauri";
import { showToast } from "../components/ui/Toast";

interface HealthMonitorOptions {
  enabled: boolean;
  intervalMs?: number; // default 15000
}

export function useHealthMonitor(options: HealthMonitorOptions) {
  const { enabled, intervalMs = 15000 } = options;
  const [snapshot, setSnapshot] = useState<InstallSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const prevHealthy = useRef<boolean | null>(null);
  const prevTunnelAlive = useRef<boolean | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchSnapshot = useCallback(async () => {
    try {
      const snap = await getInstallSnapshot();
      setSnapshot(snap);
      setLoading(false);

      // Notify on state transitions
      const nowHealthy = snap.all_healthy && snap.postiz_responding;

      if (prevHealthy.current !== null && prevHealthy.current && !nowHealthy) {
        showToast("Postiz has stopped responding", "error");
      }
      if (prevHealthy.current !== null && !prevHealthy.current && nowHealthy) {
        showToast("Postiz is back online", "success");
      }

      if (prevTunnelAlive.current !== null && prevTunnelAlive.current && !snap.tunnel_alive && snap.tunnel_mode !== "none") {
        showToast("Tunnel connection lost", "error");
      }
      if (prevTunnelAlive.current !== null && !prevTunnelAlive.current && snap.tunnel_alive) {
        showToast("Tunnel reconnected", "success");
      }

      prevHealthy.current = nowHealthy;
      prevTunnelAlive.current = snap.tunnel_alive;
    } catch {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!enabled) {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
      return;
    }

    fetchSnapshot();
    intervalRef.current = setInterval(fetchSnapshot, intervalMs);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [enabled, intervalMs, fetchSnapshot]);

  return { snapshot, loading, refresh: fetchSnapshot };
}
