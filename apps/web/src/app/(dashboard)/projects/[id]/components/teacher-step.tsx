"use client";

import type { ProviderPolicy, TeacherConfigDto } from "@/lib/api-client";
import { useLlmSettings } from "@/hooks/use-settings";
import { useTeacherCatalog, useTeacherPolicy } from "@/hooks/use-teachers";

export type TeacherSource = "tenant" | "catalog" | "custom";

export interface TeacherDraft {
  source: TeacherSource | null;
  api_base_url: string;
  model: string;
  api_key: string;
  tos_acknowledged: boolean;
  include_cot: boolean;
}

export const EMPTY_TEACHER_DRAFT: TeacherDraft = {
  source: null,
  api_base_url: "",
  model: "",
  api_key: "",
  tos_acknowledged: false,
  include_cot: false,
};

export function teacherFromDraft(draft: TeacherDraft): TeacherConfigDto | null {
  if (!draft.source) return null;
  if (!draft.api_base_url.startsWith("http") || !draft.model.trim()) return null;
  return {
    api_base_url: draft.api_base_url.trim(),
    model: draft.model.trim(),
    api_key: draft.api_key.trim() || undefined,
    tos_acknowledged: draft.tos_acknowledged || undefined,
    include_cot: draft.include_cot || undefined,
  };
}

export function teacherDraftBlocked(
  draft: TeacherDraft,
  policy: ProviderPolicy | undefined,
): boolean {
  if (teacherFromDraft(draft) === null) return true;
  return policy === "restricted" && !draft.tos_acknowledged;
}

function PolicyBadge({ policy }: { policy: ProviderPolicy | undefined }) {
  if (!policy) return null;
  if (policy === "allowed") {
    return (
      <span className="inline-flex items-center rounded-full border border-emerald-300 dark:border-emerald-800 bg-emerald-50 dark:bg-emerald-900/20 px-2.5 py-0.5 text-xs font-medium text-emerald-700 dark:text-emerald-400">
        Allowed
      </span>
    );
  }
  if (policy === "restricted") {
    return (
      <span className="inline-flex items-center rounded-full border border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-2.5 py-0.5 text-xs font-medium text-amber-700 dark:text-amber-400">
        Requires acknowledgment
      </span>
    );
  }
  return (
    <span className="inline-flex items-center rounded-full border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-2.5 py-0.5 text-xs font-medium text-zinc-500">
      Unknown provider
    </span>
  );
}

function SourceCard({
  active,
  disabled,
  title,
  description,
  onSelect,
  children,
}: {
  active: boolean;
  disabled?: boolean;
  title: string;
  description: string;
  onSelect: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div
      className={`rounded-lg border p-3 transition ${
        active
          ? "border-violet-300 dark:border-violet-800 bg-violet-50/40 dark:bg-violet-900/10"
          : "border-zinc-200 dark:border-zinc-800"
      } ${disabled ? "opacity-50" : ""}`}
    >
      <button
        type="button"
        onClick={onSelect}
        disabled={disabled}
        className="flex w-full items-start gap-3 text-left disabled:cursor-not-allowed"
      >
        <span
          className={`mt-1 h-3.5 w-3.5 shrink-0 rounded-full border ${
            active
              ? "border-violet-500 bg-violet-500"
              : "border-zinc-300 dark:border-zinc-600"
          }`}
        />
        <span>
          <span className="block text-sm font-medium text-zinc-900 dark:text-white">
            {title}
          </span>
          <span className="block text-xs text-zinc-500 mt-0.5">
            {description}
          </span>
        </span>
      </button>
      {active && children && <div className="mt-3 pl-6">{children}</div>}
    </div>
  );
}

const FIELD_CLASS =
  "w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white";

function EndpointFields({
  draft,
  onChange,
  showModelField,
}: {
  draft: TeacherDraft;
  onChange: (draft: TeacherDraft) => void;
  showModelField: boolean;
}) {
  return (
    <div className="space-y-2">
      <div>
        <label className="block text-xs text-zinc-500 mb-1">
          Endpoint base URL
        </label>
        <input
          type="url"
          value={draft.api_base_url}
          onChange={(e) => onChange({ ...draft, api_base_url: e.target.value })}
          placeholder="https://api.example.com/v1"
          className={FIELD_CLASS}
        />
      </div>
      {showModelField && (
        <div>
          <label className="block text-xs text-zinc-500 mb-1">Model</label>
          <input
            type="text"
            value={draft.model}
            onChange={(e) => onChange({ ...draft, model: e.target.value })}
            placeholder="model id as the endpoint expects it"
            className={FIELD_CLASS}
          />
        </div>
      )}
      <div>
        <label className="block text-xs text-zinc-500 mb-1">API key</label>
        <input
          type="password"
          value={draft.api_key}
          onChange={(e) => onChange({ ...draft, api_key: e.target.value })}
          placeholder="sk-..."
          autoComplete="off"
          className={FIELD_CLASS}
        />
        <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
          We encrypt this and never display it again.
        </p>
      </div>
    </div>
  );
}

/**
 * Teacher picker for distillation: three explicit choices, nothing
 * preselected. The teacher is the model that writes the training examples.
 */
export function TeacherStep({
  draft,
  onChange,
  showCotToggle,
}: {
  draft: TeacherDraft;
  onChange: (draft: TeacherDraft) => void;
  showCotToggle: boolean;
}) {
  const { data: llmSettings } = useLlmSettings();
  const { data: catalog } = useTeacherCatalog();
  const { data: policyData } = useTeacherPolicy(draft.api_base_url, draft.model);
  const policy = teacherFromDraft(draft) ? policyData?.policy : undefined;

  const tenantReady = Boolean(
    llmSettings?.is_configured && llmSettings.api_base_url && llmSettings.model,
  );
  const judgeIsTeacher =
    tenantReady &&
    draft.model.trim() === llmSettings?.model &&
    draft.api_base_url.trim() === llmSettings?.api_base_url;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-sm font-medium text-zinc-900 dark:text-white">
          Who teaches your model?
        </p>
        <PolicyBadge policy={policy} />
      </div>

      <SourceCard
        active={draft.source === "tenant"}
        disabled={!tenantReady}
        title="Use my configured LLM"
        description={
          tenantReady
            ? `${llmSettings?.model} @ ${llmSettings?.api_base_url}`
            : "No custom LLM configured — set one in Settings → LLM"
        }
        onSelect={() =>
          onChange({
            ...draft,
            source: "tenant",
            api_base_url: llmSettings?.api_base_url ?? "",
            model: llmSettings?.model ?? "",
            api_key: "",
            tos_acknowledged: false,
          })
        }
      />

      <SourceCard
        active={draft.source === "catalog"}
        title="Recommended open models"
        description="Permissively licensed teachers — their outputs are yours to train on"
        onSelect={() =>
          onChange({
            ...draft,
            source: "catalog",
            api_base_url: "",
            model: catalog?.[0]?.model_id ?? "",
            api_key: "",
            tos_acknowledged: false,
          })
        }
      >
        <div className="space-y-2">
          <div className="space-y-1">
            {(catalog ?? []).map((entry) => (
              <label
                key={entry.model_id}
                className="flex items-start gap-2 text-sm cursor-pointer"
              >
                <input
                  type="radio"
                  name="teacher-catalog-model"
                  checked={draft.model === entry.model_id}
                  onChange={() => onChange({ ...draft, model: entry.model_id })}
                  className="mt-1 accent-violet-600"
                />
                <span>
                  <span className="text-zinc-900 dark:text-white">
                    {entry.model_id}
                  </span>
                  <span className="ml-2 text-xs text-zinc-400">
                    {entry.license}
                  </span>
                  <span className="block text-xs text-zinc-500">
                    {entry.why}
                  </span>
                </span>
              </label>
            ))}
          </div>
          <p className="text-xs text-zinc-500">
            Point at any endpoint that serves this model (OpenAI-compatible).
          </p>
          <EndpointFields draft={draft} onChange={onChange} showModelField={false} />
        </div>
      </SourceCard>

      <SourceCard
        active={draft.source === "custom"}
        title="Custom endpoint"
        description="Any OpenAI-compatible API you have access to"
        onSelect={() =>
          onChange({
            ...draft,
            source: "custom",
            api_base_url: "",
            model: "",
            api_key: "",
            tos_acknowledged: false,
          })
        }
      >
        <EndpointFields draft={draft} onChange={onChange} showModelField />
      </SourceCard>

      {policy === "restricted" && (
        <label className="flex items-start gap-2 rounded-lg border border-amber-300 dark:border-amber-800 bg-amber-50/40 dark:bg-amber-900/10 p-3 text-sm cursor-pointer">
          <input
            type="checkbox"
            checked={draft.tos_acknowledged}
            onChange={(e) =>
              onChange({ ...draft, tos_acknowledged: e.target.checked })
            }
            className="mt-0.5 accent-amber-600"
          />
          <span className="text-zinc-700 dark:text-zinc-300">
            I confirm my use of this provider&apos;s outputs for training
            complies with their terms.
          </span>
        </label>
      )}

      {judgeIsTeacher && (
        <p className="rounded-lg border border-blue-200 dark:border-blue-800 bg-blue-50/40 dark:bg-blue-900/10 p-3 text-xs text-zinc-600 dark:text-zinc-400">
          Your evaluation judge is the same model as your teacher — comparison
          scores may be inflated. Consider a different judge in Settings → LLM.
        </p>
      )}

      {showCotToggle && draft.source && (
        <label className="flex items-start gap-2 text-sm cursor-pointer">
          <input
            type="checkbox"
            checked={draft.include_cot}
            onChange={(e) =>
              onChange({ ...draft, include_cot: e.target.checked })
            }
            className="mt-0.5 accent-violet-600"
          />
          <span>
            <span className="text-zinc-900 dark:text-white">
              Include teacher reasoning traces
            </span>
            <span className="block text-xs text-zinc-500">
              Best with open models you host; some providers don&apos;t expose
              real reasoning and may restrict training on it.
            </span>
          </span>
        </label>
      )}

      <p className="text-xs text-zinc-400 dark:text-zinc-600">
        Generation calls use your teacher API key and are billed by your
        provider.
      </p>
    </div>
  );
}
