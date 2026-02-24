"use client";

import { useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useNotificationPreferences() {
  return useAuthedQuery({
    queryKey: ["notifications", "preferences"],
    queryFn: (token) => api.notifications.getPreferences(token),
  });
}

export function useUpdatePreferences() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (
      token: string,
      data: {
        preferences: Array<{
          channel: string;
          event_type: string;
          enabled: boolean;
          config?: Record<string, unknown>;
        }>;
      },
    ) => api.notifications.updatePreferences(token, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
    },
  });
}

export function useDeliveryHistory(offset = 0, limit = 20) {
  return useAuthedQuery({
    queryKey: ["notifications", "deliveries", offset, limit],
    queryFn: (token) => api.notifications.getDeliveries(token, offset, limit),
  });
}

export function useTestWebhook() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (token: string, preferenceId: string) =>
      api.notifications.testWebhook(token, preferenceId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["notifications", "deliveries"],
      });
    },
  });
}

export function useRetryDelivery() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (token: string, deliveryId: string) =>
      api.notifications.retryDelivery(token, deliveryId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["notifications", "deliveries"],
      });
    },
  });
}
