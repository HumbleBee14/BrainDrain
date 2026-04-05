"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
  api,
  type CreateInferenceInstanceRequest,
  type UpdateInferenceInstanceLifecycleRequest,
} from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useInferenceInstances() {
  return useAuthedQuery({
    queryKey: ["admin", "inference-instances"],
    queryFn: (token) => api.inferenceInstances.list(token),
  });
}

export function useRegisterInstance() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (token: string, data: CreateInferenceInstanceRequest) =>
      api.inferenceInstances.register(token, data),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["admin", "inference-instances"],
      });
    },
  });
}

export function useUpdateInstanceLifecycle() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (
      token: string,
      data: { id: string } & UpdateInferenceInstanceLifecycleRequest,
    ) =>
      api.inferenceInstances.updateLifecycle(token, data.id, {
        lifecycle_state: data.lifecycle_state,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["admin", "inference-instances"],
      });
    },
  });
}

export function useDeleteInstance() {
  const queryClient = useQueryClient();
  return useAuthedMutation({
    mutationFn: (token: string, data: { id: string }) =>
      api.inferenceInstances.delete(token, data.id),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["admin", "inference-instances"],
      });
    },
  });
}
