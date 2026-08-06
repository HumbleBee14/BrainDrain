"use client";

import { Button } from "@/components/ui/button";

/**
 * The gated "move forward" control under each tab: enabled only when the
 * stage's prerequisite exists, with the unmet condition spelled out.
 */
export function NextStepBar({
  label,
  enabled,
  hint,
  onNext,
}: {
  label: string;
  enabled: boolean;
  hint: string;
  onNext: () => void;
}) {
  return (
    <div className="mt-8 flex items-center justify-end gap-3 border-t border-zinc-200 pt-4 dark:border-zinc-800">
      {!enabled && (
        <p className="text-sm text-zinc-400 dark:text-zinc-600">{hint}</p>
      )}
      <Button
        onClick={onNext}
        disabled={!enabled}
        title={enabled ? undefined : hint}
      >
        {label} →
      </Button>
    </div>
  );
}
