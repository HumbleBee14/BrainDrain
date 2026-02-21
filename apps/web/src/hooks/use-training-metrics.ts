"use client";

import { useAuth } from "@clerk/nextjs";
import { useCallback, useEffect, useRef, useState } from "react";
import type { TrainingMetricsEntry } from "@/lib/api-client";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";

/** Maximum metrics entries to keep in memory (sliding window). */
const MAX_METRICS = 1000;

/** Initial reconnection delay in ms. Doubles on each retry (exponential backoff). */
const INITIAL_RECONNECT_DELAY_MS = 1000;

/** Maximum reconnection delay in ms. */
const MAX_RECONNECT_DELAY_MS = 30000;

interface UseTrainingMetricsStreamResult {
  metrics: TrainingMetricsEntry[];
  connected: boolean;
  error: string | null;
}

/**
 * SSE hook for streaming real-time training metrics.
 *
 * Uses fetch() with ReadableStream instead of EventSource to support
 * Bearer token authentication. Automatically reconnects with exponential
 * backoff on connection loss.
 */
export function useTrainingMetricsStream(
  jobId: string,
  enabled = true
): UseTrainingMetricsStreamResult {
  const { getToken } = useAuth();
  const [metrics, setMetrics] = useState<TrainingMetricsEntry[]>([]);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectDelayRef = useRef(INITIAL_RECONNECT_DELAY_MS);

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
    // Clean up any existing connection
    if (abortRef.current) {
      abortRef.current.abort();
    }

    const controller = new AbortController();
    abortRef.current = controller;

    try {
      const token = await getToken();
      const url = `${API_URL}/api/v1/training-jobs/${jobId}/metrics/stream`;

      const response = await fetch(url, {
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

        // Parse SSE events from the buffer
        const lines = buffer.split("\n");
        // Keep the last incomplete line in the buffer
        buffer = lines.pop() || "";

        let eventType = "";
        for (const line of lines) {
          if (line.startsWith("event:")) {
            eventType = line.slice(6).trim();
          } else if (line.startsWith("data:") && eventType === "metrics") {
            const data = line.slice(5).trim();
            try {
              const parsed = JSON.parse(data);
              if (
                typeof parsed === "object" &&
                parsed !== null &&
                "step" in parsed
              ) {
                setMetrics((prev) => {
                  const updated = [...prev, parsed as TrainingMetricsEntry];
                  return updated.length > MAX_METRICS
                    ? updated.slice(-MAX_METRICS)
                    : updated;
                });
              }
            } catch {
              // Ignore parse errors for non-JSON events
            }
            eventType = "";
          } else if (line === "") {
            // Empty line resets the event
            eventType = "";
          }
        }
      }
    } catch (err) {
      // Don't report abort errors (intentional cleanup)
      if (err instanceof DOMException && err.name === "AbortError") {
        return;
      }

      setConnected(false);
      const message =
        err instanceof Error ? err.message : "Connection lost";
      setError(`${message}. Reconnecting...`);

      // Schedule reconnection with exponential backoff
      const delay = reconnectDelayRef.current;
      reconnectDelayRef.current = Math.min(
        delay * 2,
        MAX_RECONNECT_DELAY_MS
      );

      reconnectTimerRef.current = setTimeout(() => {
        if (!abortRef.current?.signal.aborted) {
          connect();
        }
      }, delay);
    }
  }, [jobId, getToken]);

  useEffect(() => {
    if (!jobId || !enabled) {
      cleanup();
      return;
    }

    connect();
    return cleanup;
  }, [jobId, enabled, connect, cleanup]);

  return { metrics, connected, error };
}
