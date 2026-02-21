"use client";

import { api } from "@/lib/api-client";
import { useAuthedQuery } from "@/hooks/use-authed-query";

export function useDashboardStats() {
  return useAuthedQuery({
    queryKey: ["dashboard", "stats"],
    queryFn: (token) => api.dashboard.getStats(token),
  });
}

export function useUsageSummary() {
  return useAuthedQuery({
    queryKey: ["dashboard", "usage"],
    queryFn: (token) => api.dashboard.getUsage(token),
  });
}

export function useRecentActivity() {
  return useAuthedQuery({
    queryKey: ["dashboard", "activity"],
    queryFn: (token) => api.dashboard.getActivity(token),
  });
}
