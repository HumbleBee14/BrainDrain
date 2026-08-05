"use client";

import { useQuery } from "@tanstack/react-query";
import { useAuth } from "@clerk/nextjs";
import {
  api,
  type ClassifyTeacherResponse,
  type ImproveOfferResponse,
  type TeacherCatalogEntry,
  type TeacherCostEstimateResponse,
} from "@/lib/api-client";

/** Curated open teacher models (permissive licenses) for the teacher picker. */
export function useTeacherCatalog() {
  const { getToken } = useAuth();
  return useQuery<TeacherCatalogEntry[]>({
    queryKey: ["teacher-catalog"],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.teachers.catalog(token);
    },
    staleTime: 10 * 60 * 1000,
  });
}

/**
 * Provider policy for a chosen teacher endpoint + model — drives the policy
 * badge before anything is launched.
 */
export function useTeacherPolicy(apiBaseUrl: string, model: string) {
  const { getToken } = useAuth();
  const enabled = apiBaseUrl.startsWith("http") && model.trim().length > 0;
  return useQuery<ClassifyTeacherResponse>({
    queryKey: ["teacher-policy", apiBaseUrl, model],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.teachers.classify(token, apiBaseUrl, model.trim());
    },
    enabled,
    staleTime: 10 * 60 * 1000,
  });
}

/**
 * Whether a dataset can be trained on its teacher's answer confidence, and what
 * that would cost. An ineligible answer carries the reason to show the user.
 */
export function useTeacherCostEstimate(
  datasetId: string,
  studentModel: string,
  topKLogprobs?: number,
) {
  const { getToken } = useAuth();
  const enabled = datasetId.length > 0 && studentModel.trim().length > 0;
  return useQuery<TeacherCostEstimateResponse>({
    queryKey: ["teacher-cost-estimate", datasetId, studentModel, topKLogprobs],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.teachers.costEstimate(token, {
        dataset_id: datasetId,
        student_model: studentModel.trim(),
        top_k_logprobs: topKLogprobs,
      });
    },
    enabled,
    staleTime: 5 * 60 * 1000,
  });
}

/**
 * Whether a trained model can be sharpened against its own teacher, and what
 * that costs. Ineligibility is a normal answer carrying its own reason — most
 * models have no teacher behind them — so the caller renders nothing.
 */
export function useImproveOffer(modelId: string | null) {
  const { getToken } = useAuth();
  return useQuery<ImproveOfferResponse>({
    queryKey: ["improve-offer", modelId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.teachers.improveOffer(token, modelId as string);
    },
    enabled: Boolean(modelId),
    staleTime: 5 * 60 * 1000,
  });
}
