"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type Document,
  type PaginatedResponse,
  type UploadResponse,
} from "@/lib/api-client";

export function useDocuments(
  projectId: string,
  offset = 0,
  limit = 50,
  pollingInterval?: number | false,
) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<Document>>({
    queryKey: ["documents", projectId, offset, limit],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.documents.list(token, projectId, offset, limit);
    },
    enabled: !!projectId,
    refetchInterval: pollingInterval,
  });
}

export function useUploadDocuments(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<UploadResponse[], Error, File[]>({
    mutationFn: async (files) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.documents.upload(token, projectId, files);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["documents", projectId] });
      queryClient.invalidateQueries({ queryKey: ["pipeline-status", projectId] });
    },
  });
}
