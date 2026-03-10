"use client";

import { useState, useCallback } from "react";
import type { ZodSchema, ZodError, ZodObject, ZodRawShape } from "zod";

interface UseFormValidationResult<T> {
  /** Per-field error messages */
  errors: Partial<Record<keyof T, string>>;
  /** Validate all fields, returns parsed data or null */
  validate: (data: unknown) => T | null;
  /** Validate a single field */
  validateField: (field: keyof T, value: unknown) => string | null;
  /** Clear all errors */
  clearErrors: () => void;
  /** Clear error for a specific field */
  clearFieldError: (field: keyof T) => void;
  /** Whether any errors exist */
  hasErrors: boolean;
}

export function useFormValidation<T>(
  schema: ZodSchema<T>,
): UseFormValidationResult<T> {
  const [errors, setErrors] = useState<Partial<Record<keyof T, string>>>({});

  const validate = useCallback(
    (data: unknown): T | null => {
      const result = schema.safeParse(data);
      if (result.success) {
        setErrors({});
        return result.data;
      }

      const fieldErrors: Partial<Record<keyof T, string>> = {};
      for (const issue of (result.error as ZodError).issues) {
        const field = issue.path[0] as keyof T;
        if (field && !fieldErrors[field]) {
          fieldErrors[field] = issue.message;
        }
      }
      setErrors(fieldErrors);
      return null;
    },
    [schema],
  );

  const validateField = useCallback(
    (field: keyof T, value: unknown): string | null => {
      // Extract the individual field schema if possible (ZodObject),
      // otherwise fall back to full-schema parse filtering for our field.
      const fieldKey = field as string;
      let fieldError: string | null = null;

      const shape = (schema as unknown as ZodObject<ZodRawShape>).shape;
      if (shape && fieldKey in shape) {
        const fieldSchema = shape[fieldKey];
        const result = fieldSchema.safeParse(value);
        if (result.success) {
          setErrors((prev) => {
            const next = { ...prev };
            delete next[field];
            return next;
          });
          return null;
        }
        fieldError = (result.error as ZodError).issues[0]?.message || "Invalid";
      } else {
        // Fallback: parse full object and filter for our field's error
        const result = schema.safeParse({ [field]: value });
        if (result.success) {
          setErrors((prev) => {
            const next = { ...prev };
            delete next[field];
            return next;
          });
          return null;
        }
        const issue = (result.error as ZodError).issues.find(
          (i) => i.path[0] === field,
        );
        fieldError = issue?.message || "Invalid";
      }

      setErrors((prev) => ({ ...prev, [field]: fieldError }));
      return fieldError;
    },
    [schema],
  );

  const clearErrors = useCallback(() => setErrors({}), []);

  const clearFieldError = useCallback(
    (field: keyof T) =>
      setErrors((prev) => {
        const next = { ...prev };
        delete next[field];
        return next;
      }),
    [],
  );

  return {
    errors,
    validate,
    validateField,
    clearErrors,
    clearFieldError,
    hasErrors: Object.keys(errors).length > 0,
  };
}
