import { useState, useEffect, useCallback, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/ipc";
import type { ServerStatus } from "../lib/types";

const POLL_INTERVAL = 5000; // 5s fallback polling
type PendingAction = "starting" | "stopping";

export function useServerStatus() {
  const [status, setStatus] = useState<ServerStatus>({
    running: false,
    state: "stopped",
    uptime_secs: 0,
    pid: null,
  });
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  const pendingActionRef = useRef<PendingAction | null>(null);
  const statusVersionRef = useRef(0);
  const mountedRef = useRef(true);

  const fetch = useCallback(async () => {
    const version = statusVersionRef.current;
    const next = await api.getServerStatus();
    // A poll started before a command or event must not restore an older state.
    if (mountedRef.current && version === statusVersionRef.current) {
      setStatus(next);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;

    // Initial fetch
    fetch().catch(console.error);

    // Event-driven updates (primary)
    const unlisten = listen<ServerStatus>("server-status", (event) => {
      if (mountedRef.current) {
        statusVersionRef.current += 1;
        setStatus(event.payload);
      }
    });
    unlisten.catch(console.error);

    // Polling fallback (secondary)
    const timer = setInterval(() => fetch().catch(console.error), POLL_INTERVAL);

    return () => {
      mountedRef.current = false;
      unlisten.then((fn) => fn()).catch(console.error);
      clearInterval(timer);
    };
  }, [fetch]);

  const runAction = useCallback(async (action: PendingAction) => {
    if (pendingActionRef.current || status.state === "starting" || status.state === "stopping") {
      return;
    }

    pendingActionRef.current = action;
    statusVersionRef.current += 1;
    setPendingAction(action);
    try {
      const next = await (action === "starting" ? api.startServer() : api.stopServer());
      statusVersionRef.current += 1;
      if (mountedRef.current) setStatus(next);
    } catch (error) {
      await fetch().catch(console.error);
      throw error;
    } finally {
      pendingActionRef.current = null;
      if (mountedRef.current) setPendingAction(null);
    }
  }, [fetch, status.state]);

  const startServer = useCallback(() => runAction("starting"), [runAction]);
  const stopServer = useCallback(() => runAction("stopping"), [runAction]);

  return {
    // Keep the clicked operation visible until its command has completed.
    status: pendingAction ? { ...status, state: pendingAction } : status,
    startServer,
    stopServer,
  };
}
