"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type ApiKeyResponse,
  type CreateApiKeyResponse,
  type CreateApiKeyInput,
} from "@/lib/api-client";

export function useApiKeys(modelId: string) {
  const { getToken } = useAuth();

  return useQuery<ApiKeyResponse[]>({
    queryKey: ["api-keys", modelId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.apiKeys.list(token, modelId);
    },
    enabled: !!modelId,
  });
}

export function useCreateApiKey(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<CreateApiKeyResponse, Error, CreateApiKeyInput>({
    mutationFn: async (data) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.apiKeys.create(token, modelId, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["api-keys", modelId] });
    },
  });
}

export function useRevokeApiKey(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<ApiKeyResponse, Error, string>({
    mutationFn: async (keyId) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.apiKeys.revoke(token, keyId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["api-keys", modelId] });
    },
  });
}
