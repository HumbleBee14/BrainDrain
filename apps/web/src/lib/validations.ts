import { z } from "zod";

// ── Project schemas ──

export const createProjectSchema = z.object({
  name: z
    .string()
    .min(1, "Project name is required")
    .max(255, "Name must be under 255 characters")
    .transform((v) => v.trim()),
  description: z
    .string()
    .max(2000, "Description must be under 2000 characters")
    .optional()
    .transform((v) => v?.trim() || undefined),
  task_type: z.string().optional(),
});

export type CreateProjectInput = z.infer<typeof createProjectSchema>;

// ── Training job schemas ──

export const createTrainingJobSchema = z.object({
  dataset_id: z.string().uuid("Invalid dataset"),
  base_model: z.string().min(1, "Base model is required"),
  method: z.enum(["qlora", "lora"]).optional(),
  mode: z.enum(["quick", "aligned", "reasoning", "iterative"]).optional(),
  hyperparams: z.record(z.unknown()).optional(),
  gpu_class: z.string().optional(),
});

export type CreateTrainingJobInput = z.infer<typeof createTrainingJobSchema>;

// ── Evaluation schemas ──

export const createEvaluationSchema = z.object({
  judge_model: z.string().optional(),
  judge_api_base: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
});

export type CreateEvaluationInput = z.infer<typeof createEvaluationSchema>;

// ── API Key schemas ──

export const createApiKeySchema = z.object({
  name: z
    .string()
    .min(1, "Key name is required")
    .max(255, "Name must be under 255 characters")
    .transform((v) => v.trim()),
  rate_limit: z
    .number()
    .int()
    .min(1, "Rate limit must be at least 1")
    .max(10000, "Rate limit must be under 10,000")
    .optional(),
  expires_in_days: z
    .number()
    .int()
    .min(1, "Must be at least 1 day")
    .max(365, "Must be under 365 days")
    .optional(),
});

export type CreateApiKeyInput = z.infer<typeof createApiKeySchema>;
