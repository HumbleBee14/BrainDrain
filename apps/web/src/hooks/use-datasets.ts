"use client";

import { useAuth } from "@clerk/nextjs";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type Dataset, type PaginatedResponse } from "@/lib/api-client";

export function useDatasets(projectId: string, offset = 0, limit = 20) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<Dataset>>({
    queryKey: ["datasets", projectId, "list", offset, limit],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.datasets.list(token, projectId, offset, limit);
    },
    enabled: !!projectId,
  });
}

export function useDataset(id: string) {
  const { getToken } = useAuth();

  return useQuery<Dataset>({
    queryKey: ["datasets", "detail", id],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.datasets.get(token, id);
    },
    enabled: !!id,
  });
}

export function useDatasetPreview(id: string, maxRows = 20) {
  const { getToken } = useAuth();

  return useQuery<Record<string, unknown>[]>({
    queryKey: ["datasets", "preview", id, maxRows],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.datasets.preview(token, id, maxRows);
    },
    enabled: !!id,
  });
}

export function useApproveDataset() {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.datasets.approve(token, id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["datasets"] });
    },
  });
}

export function useRejectDataset() {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.datasets.reject(token, id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["datasets"] });
    },
  });
}
