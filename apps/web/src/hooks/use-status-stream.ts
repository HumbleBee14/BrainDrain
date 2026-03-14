"use client";

import { useAuth } from "@clerk/nextjs";
import { useCallback, useEffect, useRef, useState } from "react";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";

const INITIAL_RECONNECT_DELAY_MS = 1000;
const MAX_RECONNECT_DELAY_MS = 30000;

/**
 * Generic SSE hook for streaming resource status updates.
 *
 * Uses fetch() with ReadableStream (instead of EventSource) to support
 * Bearer token authentication. Automatically reconnects with exponential
 * backoff. Pushes the latest status into React Query cache via onUpdate.
 *
 * The server only sends events when status actually changes, eliminating
 * the O(n) polling load from N clients.
 *
 * @param path - API path relative to API_URL (e.g., "/api/v1/training-jobs/123/status/stream")
 * @param enabled - Whether the stream should be active
 * @param onUpdate - Callback when a status update event is received
 */
export function useStatusStream<T>(
  path: string | null,
  enabled: boolean,
  onUpdate: (data: T) => void,
): { connected: boolean; error: string | null } {
  const { getToken } = useAuth();
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectDelayRef = useRef(INITIAL_RECONNECT_DELAY_MS);
  const onUpdateRef = useRef(onUpdate);
  onUpdateRef.current = onUpdate;

  const cleanup = useCallback(() => {
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
    setConnected(false);
  }, []);

  const connect = useCallback(async () => {
    if (!path) return;

    if (abortRef.current) {
      abortRef.current.abort();
    }

    const controller = new AbortController();
    abortRef.current = controller;

    try {
      const token = await getToken();
      const response = await fetch(`${API_URL}${path}`, {
        headers: {
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
          Accept: "text/event-stream",
        },
        signal: controller.signal,
      });

      if (!response.ok) {
        throw new Error(`SSE connection failed: ${response.status}`);
      }

      if (!response.body) {
        throw new Error("ReadableStream not supported");
      }

      setConnected(true);
      setError(null);
      reconnectDelayRef.current = INITIAL_RECONNECT_DELAY_MS;

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done || controller.signal.aborted) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() || "";

        let eventType = "";
        for (const line of lines) {
          if (line.startsWith("event:")) {
            eventType = line.slice(6).trim();
          } else if (line.startsWith("data:") && eventType === "status") {
            const data = line.slice(5).trim();
            try {
              const parsed = JSON.parse(data) as T;
              onUpdateRef.current(parsed);
            } catch {
              // Ignore parse errors
            }
            eventType = "";
          } else if (line === "") {
            eventType = "";
          }
        }
      }
    } catch (err) {
      if (err instanceof DOMException && err.name === "AbortError") {
        return;
      }

      setConnected(false);
      const message = err instanceof Error ? err.message : "Connection lost";
      setError(`${message}. Reconnecting...`);

      const delay = reconnectDelayRef.current;
      reconnectDelayRef.current = Math.min(delay * 2, MAX_RECONNECT_DELAY_MS);

      reconnectTimerRef.current = setTimeout(() => {
        if (!abortRef.current?.signal.aborted) {
          connect();
        }
      }, delay);
    }
  }, [path, getToken]);

  useEffect(() => {
    if (!path || !enabled) {
      cleanup();
      return;
    }

    connect();
    return cleanup;
  }, [path, enabled, connect, cleanup]);

  return { connected, error };
}
