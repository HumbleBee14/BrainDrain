"use client";

import { useState, useEffect, useCallback } from "react";

export type OnboardingStep =
  | "create_project"
  | "upload_document"
  | "parse_documents"
  | "generate_data"
  | "start_training"
  | "view_results";

const STEPS: OnboardingStep[] = [
  "create_project",
  "upload_document",
  "parse_documents",
  "generate_data",
  "start_training",
  "view_results",
];

const STEP_LABELS: Record<OnboardingStep, string> = {
  create_project: "Create your first project",
  upload_document: "Upload a document",
  parse_documents: "Parse your documents",
  generate_data: "Generate training data",
  start_training: "Start a training job",
  view_results: "View your trained model",
};

const STORAGE_KEY = "platform_onboarding";

interface OnboardingState {
  completedSteps: OnboardingStep[];
  dismissed: boolean;
}

export function useOnboarding() {
  const [state, setState] = useState<OnboardingState>({
    completedSteps: [],
    dismissed: false,
  });
  const [loaded, setLoaded] = useState(false);

  // Load from localStorage on mount
  useEffect(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored);
        // Validate shape before setting state
        if (
          parsed &&
          typeof parsed === "object" &&
          Array.isArray(parsed.completedSteps) &&
          typeof parsed.dismissed === "boolean"
        ) {
          setState(parsed);
        }
      }
    } catch {
      // Ignore parse errors
    }
    setLoaded(true);
  }, []);

  // Save to localStorage on change
  useEffect(() => {
    if (loaded) {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    }
  }, [state, loaded]);

  const markStepComplete = useCallback((step: OnboardingStep) => {
    setState((prev) => {
      if (prev.completedSteps.includes(step)) return prev;
      return { ...prev, completedSteps: [...prev.completedSteps, step] };
    });
  }, []);

  const dismiss = useCallback(() => {
    setState((prev) => ({ ...prev, dismissed: true }));
  }, []);

  const reset = useCallback(() => {
    setState({ completedSteps: [], dismissed: false });
  }, []);

  const currentStep = STEPS.find((s) => !state.completedSteps.includes(s));
  const currentStepIndex = currentStep ? STEPS.indexOf(currentStep) : STEPS.length;
  const isComplete = state.completedSteps.length >= STEPS.length;
  const progress = state.completedSteps.length / STEPS.length;

  return {
    steps: STEPS,
    stepLabels: STEP_LABELS,
    completedSteps: state.completedSteps,
    currentStep,
    currentStepIndex,
    isComplete,
    isDismissed: state.dismissed,
    progress,
    markStepComplete,
    dismiss,
    reset,
    loaded,
  };
}
