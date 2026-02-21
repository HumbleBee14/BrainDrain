"use client";

import { useEffect } from "react";
import { useAuth } from "@clerk/nextjs";
import { analytics } from "@/lib/analytics";

/**
 * Hook to identify the current user for analytics.
 * Call once in the root layout or dashboard layout.
 */
export function useAnalyticsIdentify() {
  const { userId } = useAuth();

  useEffect(() => {
    if (userId) {
      analytics.identify(userId);
    }
  }, [userId]);
}

/**
 * Hook to track page views.
 * Call in layout components to track navigation.
 */
export function usePageView(pageName: string) {
  useEffect(() => {
    analytics.page(pageName);
  }, [pageName]);
}
