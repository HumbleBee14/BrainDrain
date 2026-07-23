"use client";

import { useParams } from "next/navigation";
import { useState } from "react";
import { toast } from "sonner";
import { useModel } from "@/hooks/use-models";
import {
  useSamples,
  useRateSample,
  useSetCapture,
  usePromoteSamples,
} from "@/hooks/use-feedback";
import type { FeedbackRating, InferenceSample } from "@/lib/api-client";
import { Breadcrumbs } from "@/components/breadcrumbs";

const PAGE_SIZE = 20;

type Filter = FeedbackRating | "unrated" | "all";

const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "unrated", label: "Unrated" },
  { value: "positive", label: "Positive" },
  { value: "negative", label: "Negative" },
];

function RatingBadge({ rating }: { rating: FeedbackRating | null }) {
  if (!rating) {
    return (
      <span className="inline-flex items-center rounded-full border border-zinc-300 dark:border-zinc-700 bg-zinc-100 dark:bg-zinc-800 px-2.5 py-0.5 text-xs font-medium text-zinc-600 dark:text-zinc-400">
        unrated
      </span>
    );
  }
  const cls =
    rating === "positive"
      ? "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800"
      : "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/50 dark:text-red-400 dark:border-red-800";
  return (
    <span
      className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${cls}`}
    >
      {rating}
    </span>
  );
}

function lastUserMessage(sample: InferenceSample): string {
  for (let i = sample.messages.length - 1; i >= 0; i--) {
    if (sample.messages[i].role === "user") return sample.messages[i].content;
  }
  return sample.messages[0]?.content ?? "";
}

function SampleRow({
  sample,
  onRate,
  ratingPending,
  selected,
  onToggleSelect,
  correction,
  onCorrectionChange,
}: {
  sample: InferenceSample;
  onRate: (sampleId: string, rating: FeedbackRating) => void;
  ratingPending: boolean;
  selected: boolean;
  onToggleSelect: (sampleId: string) => void;
  correction: string;
  onCorrectionChange: (sampleId: string, value: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const promoted = !!sample.promoted_at;

  return (
    <div
      className={`rounded-lg border p-4 space-y-3 ${selected ? "border-violet-300 dark:border-violet-800" : "border-zinc-200 dark:border-zinc-800"}`}
    >
      <div className="flex items-start justify-between gap-3">
        <input
          type="checkbox"
          checked={selected}
          disabled={promoted}
          onChange={() => onToggleSelect(sample.id)}
          title={promoted ? "Already promoted" : "Select for promotion"}
          className="mt-1.5 accent-violet-600 disabled:opacity-40"
        />
        <button
          onClick={() => setExpanded((e) => !e)}
          className="text-left flex-1 min-w-0"
        >
          <p className="text-sm font-medium text-zinc-900 dark:text-white truncate">
            {lastUserMessage(sample)}
          </p>
          <p
            className={`text-sm text-zinc-600 dark:text-zinc-400 mt-1 ${expanded ? "whitespace-pre-wrap" : "truncate"}`}
          >
            {sample.response}
          </p>
        </button>
        <div className="flex items-center gap-2 shrink-0">
          {promoted && (
            <span className="inline-flex items-center rounded-full border border-violet-200 dark:border-violet-800 bg-violet-50 dark:bg-violet-900/30 px-2.5 py-0.5 text-xs font-medium text-violet-700 dark:text-violet-400">
              promoted
            </span>
          )}
          <RatingBadge rating={sample.rating} />
          <button
            onClick={() => onRate(sample.id, "positive")}
            disabled={ratingPending}
            title="Good response"
            className={`rounded-lg border px-2.5 py-1.5 text-sm transition disabled:opacity-50 ${
              sample.rating === "positive"
                ? "border-emerald-300 bg-emerald-50 dark:bg-emerald-900/30 dark:border-emerald-800"
                : "border-zinc-200 dark:border-zinc-800 hover:border-emerald-300"
            }`}
          >
            👍
          </button>
          <button
            onClick={() => onRate(sample.id, "negative")}
            disabled={ratingPending}
            title="Weak response"
            className={`rounded-lg border px-2.5 py-1.5 text-sm transition disabled:opacity-50 ${
              sample.rating === "negative"
                ? "border-red-300 bg-red-50 dark:bg-red-900/30 dark:border-red-800"
                : "border-zinc-200 dark:border-zinc-800 hover:border-red-300"
            }`}
          >
            👎
          </button>
        </div>
      </div>
      {expanded && (
        <div className="border-t border-zinc-100 dark:border-zinc-900 pt-3 space-y-2">
          {sample.messages.map((m, i) => (
            <div key={i} className="text-sm">
              <span className="text-xs uppercase tracking-wider text-zinc-400 dark:text-zinc-600 mr-2">
                {m.role}
              </span>
              <span className="text-zinc-700 dark:text-zinc-300 whitespace-pre-wrap">
                {m.content}
              </span>
            </div>
          ))}
          {sample.rating_comment && (
            <p className="text-xs text-zinc-500 italic">
              Feedback note: {sample.rating_comment}
            </p>
          )}
          <p className="text-xs text-zinc-400 dark:text-zinc-600">
            {new Date(sample.created_at).toLocaleString()}
          </p>
        </div>
      )}
      {selected && sample.rating === "negative" && (
        <div className="border-t border-zinc-100 dark:border-zinc-900 pt-3">
          <label className="block text-xs text-zinc-500 mb-1">
            Corrected response (required — this sample was rated negative)
          </label>
          <textarea
            value={correction}
            onChange={(e) => onCorrectionChange(sample.id, e.target.value)}
            rows={3}
            placeholder="Write the answer the model should have given..."
            className="w-full rounded-lg border border-zinc-200 dark:border-zinc-800 bg-transparent p-2 text-sm text-zinc-900 dark:text-white"
          />
        </div>
      )}
    </div>
  );
}

export default function FeedbackPage() {
  const params = useParams<{ id: string; modelId: string }>();
  const { data: model } = useModel(params.modelId);
  const [filter, setFilter] = useState<Filter>("all");
  const [offset, setOffset] = useState(0);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [corrections, setCorrections] = useState<Record<string, string>>({});

  const ratingParam = filter === "all" ? undefined : filter;
  const { data, isLoading } = useSamples(
    params.modelId,
    offset,
    PAGE_SIZE,
    ratingParam,
  );
  const rateSample = useRateSample(params.modelId);
  const setCapture = useSetCapture(params.modelId);
  const promoteSamples = usePromoteSamples(params.modelId);

  const samples = data?.data ?? [];
  const total = data?.total ?? 0;
  const captureOn = model?.capture_traffic ?? false;

  const handleRate = (sampleId: string, rating: FeedbackRating) => {
    rateSample.mutate(
      { sampleId, rating },
      { onError: (e) => toast.error(e.message) },
    );
  };

  const handleToggleCapture = () => {
    setCapture.mutate(!captureOn, {
      onSuccess: () =>
        toast.success(
          !captureOn ? "Traffic capture enabled" : "Traffic capture disabled",
        ),
      onError: (e) => toast.error(e.message),
    });
  };

  const toggleSelect = (sampleId: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(sampleId)) next.delete(sampleId);
      else next.add(sampleId);
      return next;
    });
  };

  const handlePromote = () => {
    const selected = samples.filter((s) => selectedIds.has(s.id));
    const missingCorrection = selected.find(
      (s) => s.rating === "negative" && !corrections[s.id]?.trim(),
    );
    if (missingCorrection) {
      toast.error(
        "Negative-rated samples need a corrected response before promotion.",
      );
      return;
    }
    promoteSamples.mutate(
      {
        samples: selected.map((s) => ({
          sample_id: s.id,
          corrected_response: corrections[s.id]?.trim() || undefined,
        })),
      },
      {
        onSuccess: (res) => {
          toast.success(
            `Created dataset "${res.dataset.name}" with ${res.promoted_count} examples. Review and approve it to train.`,
          );
          setSelectedIds(new Set());
          setCorrections({});
        },
        onError: (e) => toast.error(e.message),
      },
    );
  };

  return (
    <div>
      <div className="mb-8">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: "Project", href: `/projects/${params.id}` },
            {
              label: model?.name || "Model",
              href: `/projects/${params.id}/models/${params.modelId}`,
            },
            { label: "Feedback" },
          ]}
        />
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div>
            <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">
              Feedback
            </h1>
            <p className="text-sm text-zinc-500 mt-1">
              Captured production responses. Rate weak answers, correct them,
              and promote the results into a training dataset.
            </p>
          </div>
          <div className="flex gap-2">
            {selectedIds.size > 0 && (
              <button
                onClick={handlePromote}
                disabled={promoteSamples.isPending}
                className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50"
              >
                {promoteSamples.isPending
                  ? "Promoting..."
                  : `Promote ${selectedIds.size} to Training Data`}
              </button>
            )}
            <button
              onClick={handleToggleCapture}
              disabled={setCapture.isPending}
              className={`rounded-lg px-4 py-2 text-sm font-medium transition disabled:opacity-50 ${
                captureOn
                  ? "bg-emerald-600 text-white hover:bg-emerald-500"
                  : "border border-zinc-200 dark:border-zinc-800 text-zinc-700 dark:text-zinc-300 hover:border-zinc-400"
              }`}
            >
              {captureOn ? "Capture: On" : "Capture: Off"}
            </button>
          </div>
        </div>
      </div>

      {!captureOn && (
        <div className="rounded-lg border border-amber-200 dark:border-amber-800 bg-amber-50/50 dark:bg-amber-900/10 p-4 mb-6 text-sm text-amber-800 dark:text-amber-300">
          Traffic capture is off. Turn it on to start recording inference
          requests and responses for this model. Existing samples below remain
          reviewable.
        </div>
      )}

      <div className="flex items-center gap-2 mb-4">
        {FILTERS.map((f) => (
          <button
            key={f.value}
            onClick={() => {
              setFilter(f.value);
              setOffset(0);
            }}
            className={`rounded-full px-3 py-1 text-sm transition ${
              filter === f.value
                ? "bg-violet-600 text-white"
                : "border border-zinc-200 dark:border-zinc-800 text-zinc-600 dark:text-zinc-400 hover:border-zinc-400"
            }`}
          >
            {f.label}
          </button>
        ))}
        <span className="ml-auto text-sm text-zinc-500">{total} samples</span>
      </div>

      {isLoading ? (
        <div className="flex items-center justify-center py-20">
          <p className="text-zinc-500">Loading samples...</p>
        </div>
      ) : samples.length === 0 ? (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-12 text-center">
          <p className="text-zinc-500">
            {filter === "all"
              ? "No captured traffic yet. Enable capture and send requests to this model's API."
              : "No samples match this filter."}
          </p>
        </div>
      ) : (
        <div className="space-y-3">
          {samples.map((s) => (
            <SampleRow
              key={s.id}
              sample={s}
              onRate={handleRate}
              ratingPending={rateSample.isPending}
              selected={selectedIds.has(s.id)}
              onToggleSelect={toggleSelect}
              correction={corrections[s.id] ?? ""}
              onCorrectionChange={(id, value) =>
                setCorrections((prev) => ({ ...prev, [id]: value }))
              }
            />
          ))}
        </div>
      )}

      {total > PAGE_SIZE && (
        <div className="flex items-center justify-between mt-6">
          <button
            onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
            disabled={offset === 0}
            className="rounded-lg border border-zinc-200 dark:border-zinc-800 px-3 py-1.5 text-sm disabled:opacity-50"
          >
            Previous
          </button>
          <span className="text-sm text-zinc-500">
            {offset + 1}–{Math.min(offset + PAGE_SIZE, total)} of {total}
          </span>
          <button
            onClick={() => setOffset(offset + PAGE_SIZE)}
            disabled={offset + PAGE_SIZE >= total}
            className="rounded-lg border border-zinc-200 dark:border-zinc-800 px-3 py-1.5 text-sm disabled:opacity-50"
          >
            Next
          </button>
        </div>
      )}
    </div>
  );
}
