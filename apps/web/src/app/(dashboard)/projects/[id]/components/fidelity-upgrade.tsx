"use client";

import { useEffect, useState } from "react";
import type { DistillOptionsDto, TeacherPrecision } from "@/lib/api-client";
import { useTeacherCostEstimate } from "@/hooks/use-teachers";

// The API clamps a request into this range, so the input refuses out-of-range
// numbers instead of accepting one and quietly training on a different value.
const MIN_TOP_K = 8;
const MAX_TOP_K = 256;

const PRECISION_CHOICES: { value: TeacherPrecision; label: string }[] = [
  { value: "bf16", label: "Full — recommended" },
  { value: "fp8", label: "Compressed — faster, slightly less exact" },
  { value: "int4", label: "Smallest — cheapest, least exact" },
];

const FIELD_CLASS =
  "w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white";

/**
 * The fidelity upgrade offer for a teacher-generated dataset: opt-in, priced
 * before it is chosen, and absent whenever the platform cannot run the teacher
 * that wrote the data.
 */
export function FidelityUpgrade({
  datasetId,
  studentModel,
  onChange,
}: {
  datasetId: string;
  studentModel: string;
  onChange: (options: DistillOptionsDto | null) => void;
}) {
  const [upgraded, setUpgraded] = useState(false);
  const [showDetails, setShowDetails] = useState(false);
  const [topK, setTopK] = useState<number | null>(null);
  const [precision, setPrecision] = useState<TeacherPrecision>("bf16");
  const { data } = useTeacherCostEstimate(
    datasetId,
    studentModel,
    topK ?? undefined,
  );

  const effectiveTopK = topK ?? data?.top_k_logprobs;
  const offered = data?.eligible === true && data.estimate != null;

  useEffect(() => {
    onChange(
      offered && upgraded
        ? { method: "logit", precision, top_k_logprobs: effectiveTopK }
        : null,
    );
  }, [offered, upgraded, precision, effectiveTopK, onChange]);

  if (!data?.eligible || !data.estimate) {
    if (!data?.reason) return null;
    return (
      <p className="text-xs text-zinc-400 dark:text-zinc-600">{data.reason}</p>
    );
  }

  const estimate = data.estimate;

  return (
    <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-zinc-50/50 dark:bg-zinc-900/50 p-3 space-y-3">
      <div>
        <p className="text-sm font-medium text-zinc-900 dark:text-white">
          Higher-fidelity training available
        </p>
        <p className="text-xs text-zinc-500 mt-1">
          {data.teacher_model} wrote this data, and it is a model we can run
          ourselves — so your model can learn how sure the teacher was of every
          word it chose, not just the words. That usually lands noticeably
          closer to the teacher.
        </p>
      </div>

      <div className="space-y-1.5">
        <label className="flex items-start gap-2 text-sm cursor-pointer">
          <input
            type="radio"
            name="distill-fidelity"
            checked={!upgraded}
            onChange={() => setUpgraded(false)}
            className="mt-1 accent-violet-600"
          />
          <span>
            <span className="text-zinc-900 dark:text-white">
              Standard — the teacher&apos;s answers
            </span>
            <span className="block text-xs text-zinc-500">
              Trains on the examples you already have. No extra GPU time.
            </span>
          </span>
        </label>

        <label className="flex items-start gap-2 text-sm cursor-pointer">
          <input
            type="radio"
            name="distill-fidelity"
            checked={upgraded}
            onChange={() => setUpgraded(true)}
            className="mt-1 accent-violet-600"
          />
          <span>
            <span className="text-zinc-900 dark:text-white">
              Higher fidelity — adds about ${estimate.est_cost_usd.toFixed(2)}{" "}
              of GPU time
            </span>
            <span className="block text-xs text-zinc-500">
              Runs {data.teacher_model} over your examples first, on a{" "}
              {estimate.gpu_class.toUpperCase()} for roughly{" "}
              {estimate.est_gpu_hours.toFixed(1)} hours.
            </span>
            {estimate.basis === "approximate" && (
              <span className="block text-xs text-zinc-400 dark:text-zinc-600 mt-0.5">
                That price is worked out from how many examples this dataset
                has, not from a measured count of their words, so treat it as a
                rough guide — the run is billed for the GPU time it actually
                uses.
              </span>
            )}
          </span>
        </label>
      </div>

      {upgraded && (
        <div>
          <button
            type="button"
            onClick={() => setShowDetails(!showDetails)}
            aria-expanded={showDetails}
            className="text-xs text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300"
          >
            {showDetails ? "▾" : "▸"} Fidelity details
          </button>
          {showDetails && (
            <div className="mt-2 grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div>
                <label className="block text-xs text-zinc-500 mb-1">
                  How much of the teacher&apos;s confidence to keep
                </label>
                <input
                  type="number"
                  min={MIN_TOP_K}
                  max={MAX_TOP_K}
                  value={effectiveTopK}
                  onChange={(e) =>
                    setTopK(
                      Math.max(
                        MIN_TOP_K,
                        Math.min(
                          MAX_TOP_K,
                          Number(e.target.value) || MIN_TOP_K,
                        ),
                      ),
                    )
                  }
                  className={FIELD_CLASS}
                />
                <p className="mt-1 text-xs text-zinc-400 dark:text-zinc-600">
                  Default is right for almost everyone.
                </p>
              </div>
              <div>
                <label className="block text-xs text-zinc-500 mb-1">
                  How the teacher runs while it scores
                </label>
                <select
                  value={precision}
                  onChange={(e) =>
                    setPrecision(e.target.value as TeacherPrecision)
                  }
                  className={FIELD_CLASS}
                >
                  {PRECISION_CHOICES.map((choice) => (
                    <option key={choice.value} value={choice.value}>
                      {choice.label}
                    </option>
                  ))}
                </select>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
