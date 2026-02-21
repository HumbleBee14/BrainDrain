"use client";

import { useEffect, useRef } from "react";
import { useAuth } from "@clerk/nextjs";
import { wsClient } from "@/lib/ws-client";

/**
 * Hook to subscribe to a WebSocket channel for real-time updates.
 * Automatically handles connection lifecycle, auth token, and cleanup.
 *
 * @param channel - Channel name (e.g., "training_job:uuid")
 * @param onMessage - Callback when a message is received
 * @param enabled - Whether the subscription is active
 */
export function useWebSocket(
  channel: string | null,
  onMessage: (payload: unknown) => void,
  enabled = true
) {
  const { getToken } = useAuth();
  const callbackRef = useRef(onMessage);
  callbackRef.current = onMessage;

  // Keep token fresh
  useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    const refreshToken = async () => {
      const token = await getToken();
      if (token && !cancelled) {
        wsClient.setToken(token);
      }
    };

    refreshToken();
    const interval = setInterval(refreshToken, 50_000); // Refresh before expiry

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [enabled, getToken]);

  // Subscribe to channel
  useEffect(() => {
    if (!channel || !enabled) return;

    const unsubscribe = wsClient.subscribe(channel, (payload) => {
      callbackRef.current(payload);
    });

    return unsubscribe;
  }, [channel, enabled]);
}
