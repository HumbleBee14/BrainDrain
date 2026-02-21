"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type Evaluation,
  type PaginatedResponse,
  type CreateEvaluationInput,
} from "@/lib/api-client";

export function useEvaluations(modelId: string, offset = 0, limit = 20) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<Evaluation>>({
    queryKey: ["evaluations", modelId, "list", offset, limit],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.evaluations.list(token, modelId, offset, limit);
    },
    enabled: !!modelId,
  });
}

export function useEvaluation(id: string, enabled = true) {
  const { getToken } = useAuth();

  return useQuery<Evaluation>({
    queryKey: ["evaluations", "detail", id],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.evaluations.get(token, id);
    },
    enabled: !!id && enabled,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return false;
      return data.status === "running" ? 5000 : false;
    },
  });
}

export function useCreateEvaluation(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<Evaluation, Error, CreateEvaluationInput>({
    mutationFn: async (data) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.evaluations.create(token, modelId, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["evaluations", modelId],
      });
    },
  });
}
