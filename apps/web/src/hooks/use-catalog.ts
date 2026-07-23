"use client";

import { useQuery } from "@tanstack/react-query";
import { api, type CatalogResponse } from "@/lib/api-client";

/**
 * Fetches the curated base-model catalog. Public endpoint, no auth token.
 * `taskType`/`pairCount` let the backend suggest a default model/mode.
 */
export function useModelCatalog(taskType?: string, pairCount?: number) {
  return useQuery<CatalogResponse>({
    queryKey: ["model-catalog", taskType, pairCount],
    queryFn: () => api.catalog.get(taskType, pairCount),
    staleTime: 5 * 60 * 1000,
  });
}
