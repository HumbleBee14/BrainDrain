/**
 * Centralized query key factories for TanStack Query.
 *
 * Provides type-safe, hierarchical query keys for cache management.
 * The structure supports granular invalidation: invalidating `projects.all`
 * also invalidates `projects.list(...)` and `projects.detail(...)` because
 * TanStack Query matches prefixes.
 *
 * Usage:
 *   queryKey: queryKeys.projects.list(0, 20)
 *   queryClient.invalidateQueries({ queryKey: queryKeys.projects.all })
 */
export const queryKeys = {
  projects: {
    all: ["projects"] as const,
    list: (offset: number, limit: number) =>
      ["projects", "list", offset, limit] as const,
    detail: (id: string) => ["projects", "detail", id] as const,
  },

  documents: {
    all: (projectId: string) => ["documents", projectId] as const,
    list: (projectId: string, offset: number, limit: number) =>
      ["documents", projectId, "list", offset, limit] as const,
  },

  datasets: {
    all: (projectId: string) => ["datasets", projectId] as const,
    list: (projectId: string, offset: number, limit: number) =>
      ["datasets", projectId, "list", offset, limit] as const,
    detail: (id: string) => ["datasets", "detail", id] as const,
    preview: (id: string, maxRows: number) =>
      ["datasets", "preview", id, maxRows] as const,
  },

  trainingJobs: {
    all: (projectId: string) => ["training-jobs", projectId] as const,
    list: (projectId: string, offset: number, limit: number) =>
      ["training-jobs", projectId, "list", offset, limit] as const,
    detail: (id: string) => ["training-jobs", "detail", id] as const,
  },

  models: {
    all: (projectId: string) => ["models", projectId] as const,
    list: (projectId: string, offset: number, limit: number) =>
      ["models", projectId, "list", offset, limit] as const,
    detail: (id: string) => ["models", "detail", id] as const,
  },

  evaluations: {
    all: (modelId: string) => ["evaluations", modelId] as const,
    list: (modelId: string, offset: number, limit: number) =>
      ["evaluations", modelId, "list", offset, limit] as const,
    detail: (id: string) => ["evaluations", "detail", id] as const,
  },

  deployments: {
    status: (modelId: string) => ["deployment-status", modelId] as const,
  },

  pipeline: {
    status: (projectId: string) => ["pipeline-status", projectId] as const,
  },

  apiKeys: {
    all: (modelId: string) => ["api-keys", modelId] as const,
  },
} as const;
