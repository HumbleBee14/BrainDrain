"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type Model,
  type DeploymentStatusResponse,
} from "@/lib/api-client";

export function useDeploymentStatus(modelId: string, enabled = true) {
  const { getToken } = useAuth();

  return useQuery<DeploymentStatusResponse>({
    queryKey: ["deployment-status", modelId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.deployments.status(token, modelId);
    },
    enabled: !!modelId && enabled,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return false;
      return data.deployment_status === "deploying" ? 3000 : false;
    },
  });
}

export function useDeployModel(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<Model, Error, void>({
    mutationFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.deployments.deploy(token, modelId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["deployment-status", modelId],
      });
      queryClient.invalidateQueries({
        queryKey: ["models", "detail", modelId],
      });
    },
  });
}

export function useUndeployModel(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<Model, Error, void>({
    mutationFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.deployments.undeploy(token, modelId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["deployment-status", modelId],
      });
      queryClient.invalidateQueries({
        queryKey: ["models", "detail", modelId],
      });
    },
  });
}
