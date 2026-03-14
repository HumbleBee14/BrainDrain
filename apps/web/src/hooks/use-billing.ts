"use client";

import { api } from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useSubscription() {
  return useAuthedQuery({
    queryKey: ["billing", "subscription"],
    queryFn: (token) => api.billing.getSubscription(token),
  });
}

export function usePlanLimits() {
  return useAuthedQuery({
    queryKey: ["billing", "limits"],
    queryFn: (token) => api.billing.getLimits(token),
  });
}

export function useCreateCheckout() {
  return useAuthedMutation({
    mutationFn: (
      token: string,
      data: { plan: string; success_url: string; cancel_url: string },
    ) => api.billing.createCheckout(token, data),
  });
}

export function useCreatePortal() {
  return useAuthedMutation({
    mutationFn: (token: string, data: { return_url: string }) =>
      api.billing.createPortal(token, data),
  });
}
