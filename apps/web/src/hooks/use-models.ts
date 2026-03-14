"use client";

import { useAuth } from "@clerk/nextjs";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type Model, type PaginatedResponse } from "@/lib/api-client";

export function useModels(projectId: string, offset = 0, limit = 20) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<Model>>({
    queryKey: ["models", projectId, "list", offset, limit],
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
    queryKey: ["models", "detail", id],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.models.get(token, id);
    },
    enabled: !!id && enabled,
  });
}

export function useModelVersions(id: string) {
  const { getToken } = useAuth();

  return useQuery<Model[]>({
    queryKey: ["models", "versions", id],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.models.listVersions(token, id);
    },
    enabled: !!id,
  });
}

export function useRollbackModel(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<Model, Error, string>({
    mutationFn: async (targetVersionId: string) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.models.rollback(token, modelId, targetVersionId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["models"] });
    },
  });
}
