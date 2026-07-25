"use client";

import { useAuth } from "@clerk/nextjs";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type DataGuide,
  type DataGuideStatus,
  type CreateDataGuideRequest,
  type GenerateFacetsRequest,
  type UpdateFacetsRequest,
  type GeneratePreviewRequest,
  type RateSamplesRequest,
  type UpdateGuidanceRequest,
} from "@/lib/api-client";

export const RUNNING_STATUSES: DataGuideStatus[] = [
  "generating_facets",
  "generating_preview",
  "generating",
];

function isRunning(status: DataGuideStatus | undefined): boolean {
  return !!status && RUNNING_STATUSES.includes(status);
}

export function useDataGuide(projectId: string) {
  const { getToken } = useAuth();

  return useQuery<DataGuide>({
    queryKey: ["data-guide", projectId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.get(token, projectId);
    },
    enabled: !!projectId,
    refetchInterval: (query) =>
      isRunning(query.state.data?.status) ? 3000 : false,
  });
}

export function useCreateDataGuide(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<DataGuide, Error, CreateDataGuideRequest>({
    mutationFn: async (data) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.create(token, projectId, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useResetDataGuide(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<DataGuide, Error, string>({
    mutationFn: async (id) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.reset(token, id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useGenerateFacets(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    DataGuide,
    Error,
    { id: string; data: GenerateFacetsRequest }
  >({
    mutationFn: async ({ id, data }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.generateFacets(token, id, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useUpdateFacets(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    DataGuide,
    Error,
    { id: string; data: UpdateFacetsRequest }
  >({
    mutationFn: async ({ id, data }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.updateFacets(token, id, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useGeneratePreview(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    DataGuide,
    Error,
    { id: string; data: GeneratePreviewRequest }
  >({
    mutationFn: async ({ id, data }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.generatePreview(token, id, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useRateSamples(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    DataGuide,
    Error,
    { id: string; data: RateSamplesRequest }
  >({
    mutationFn: async ({ id, data }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.rate(token, id, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useRefineGuidance(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<DataGuide, Error, string>({
    mutationFn: async (id) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.refine(token, id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useUpdateGuidance(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    DataGuide,
    Error,
    { id: string; data: UpdateGuidanceRequest }
  >({
    mutationFn: async ({ id, data }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.updateGuidance(token, id, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
    },
  });
}

export function useGenerateDataset(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<DataGuide, Error, string>({
    mutationFn: async (id) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dataGuides.generate(token, id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["data-guide", projectId] });
      queryClient.invalidateQueries({ queryKey: ["datasets"] });
    },
  });
}
