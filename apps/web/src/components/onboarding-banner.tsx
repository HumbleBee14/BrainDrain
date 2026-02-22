"use client";

import { useOnboarding, type OnboardingStep } from "@/hooks/use-onboarding";
import Link from "next/link";

const STEP_LINKS: Record<OnboardingStep, string> = {
  create_project: "/projects/new",
  upload_document: "/projects",
  parse_documents: "/projects",
  generate_data: "/projects",
  start_training: "/projects",
  view_results: "/projects",
};

export function OnboardingBanner() {
  const {
    steps,
    stepLabels,
    completedSteps,
    currentStep,
    isComplete,
    isDismissed,
    progress,
    dismiss,
    loaded,
  } = useOnboarding();

  // Don't render until loaded from localStorage, or if dismissed/complete
  if (!loaded || isDismissed || isComplete) return null;

  return (
    <div className="border border-zinc-800 rounded-lg p-4 mb-6 bg-zinc-900/50">
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-3">
          <h3 className="text-sm font-medium text-white">Getting Started</h3>
          <span className="text-xs text-zinc-500">
            {completedSteps.length}/{steps.length} steps
          </span>
        </div>
        <button
          onClick={dismiss}
          className="text-zinc-500 hover:text-zinc-300 text-xs transition"
        >
          Dismiss
        </button>
      </div>

      {/* Progress bar */}
      <div className="w-full bg-zinc-800 rounded-full h-1.5 mb-3">
        <div
          className="bg-emerald-500 h-1.5 rounded-full transition-all duration-500"
          style={{ width: `${progress * 100}%` }}
        />
      </div>

      {/* Steps */}
      <div className="flex gap-2 flex-wrap">
        {steps.map((step) => {
          const completed = completedSteps.includes(step);
          const isCurrent = step === currentStep;

          return (
            <div
              key={step}
              className={`text-xs px-2.5 py-1 rounded-full transition ${
                completed
                  ? "bg-emerald-500/10 text-emerald-400"
                  : isCurrent
                    ? "bg-zinc-800 text-white ring-1 ring-emerald-500/50"
                    : "bg-zinc-800/50 text-zinc-500"
              }`}
            >
              {completed && "✓ "}{stepLabels[step]}
            </div>
          );
        })}
      </div>

      {/* CTA */}
      {currentStep && (
        <div className="mt-3">
          <Link
            href={STEP_LINKS[currentStep]}
            className="text-sm text-emerald-400 hover:text-emerald-300 transition"
          >
            Next: {stepLabels[currentStep]} →
          </Link>
        </div>
      )}
    </div>
  );
}
