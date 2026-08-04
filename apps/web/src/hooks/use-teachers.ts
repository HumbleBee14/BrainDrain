"use client";

import { useQuery } from "@tanstack/react-query";
import { useAuth } from "@clerk/nextjs";
import {
  api,
  type ClassifyTeacherResponse,
  type TeacherCatalogEntry,
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
