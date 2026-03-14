"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
  api,
  type Document,
  type PaginatedResponse,
  type UploadResponse,
} from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useDocuments(
  projectId: string,
  offset = 0,
  limit = 50,
  pollingInterval?: number | false,
) {
  return useAuthedQuery<PaginatedResponse<Document>>({
    queryKey: ["documents", projectId, offset, limit],
    queryFn: (token) => api.documents.list(token, projectId, offset, limit),
    enabled: !!projectId,
    refetchInterval: pollingInterval,
  });
}

export function useUploadDocuments(projectId: string) {
  const queryClient = useQueryClient();

  return useAuthedMutation<UploadResponse[], Error, File[]>({
    mutationFn: (token, files) => api.documents.upload(token, projectId, files),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["documents", projectId] });
      queryClient.invalidateQueries({
        queryKey: ["pipeline-status", projectId],
      });
    },
  });
}
