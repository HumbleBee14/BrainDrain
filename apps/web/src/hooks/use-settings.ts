"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
  api,
  type UpdateLlmSettingsRequest,
  type UpdateAdminConfigRequest,
} from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useLlmSettings() {
  return useAuthedQuery({
    queryKey: ["settings", "llm"],
    queryFn: (token) => api.settings.getLlm(token),
  });
}

export function useUpdateLlmSettings() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (token: string, data: UpdateLlmSettingsRequest) =>
      api.settings.updateLlm(token, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["settings", "llm"] });
    },
  });
}

export function useDeleteLlmSettings() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    mutationFn: (token: string, _data: void) => api.settings.deleteLlm(token),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["settings", "llm"] });
    },
  });
}

export function useAdminConfig() {
  return useAuthedQuery({
    queryKey: ["settings", "admin"],
    queryFn: (token) => api.settings.getAdminConfig(token),
  });
}

export function useUpdateAdminConfig() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (token: string, data: UpdateAdminConfigRequest) =>
      api.settings.updateAdminConfig(token, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["settings", "admin"] });
    },
  });
}

export function useResetAdminConfig() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    mutationFn: (token: string, _data: void) =>
      api.settings.resetAdminConfig(token),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["settings", "admin"] });
    },
  });
}
