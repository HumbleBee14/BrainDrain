"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery } from "@tanstack/react-query";
import { api, type Model, type PaginatedResponse } from "@/lib/api-client";

export function useModels(projectId: string, offset = 0, limit = 20) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<Model>>({
    queryKey: ["models", projectId, offset, limit],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.models.list(token, projectId, offset, limit);
    },
    enabled: !!projectId,
  });
}

export function useModel(id: string, enabled = true) {
  const { getToken } = useAuth();

  return useQuery<Model>({
    queryKey: ["model", id],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.models.get(token, id);
    },
    enabled: !!id && enabled,
  });
}
