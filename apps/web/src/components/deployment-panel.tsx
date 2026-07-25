"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";

// A cold GPU boot measured ~2-4 min; past this the deploy is presumed lost.
const DEPLOY_TIMEOUT_MS = 8 * 60 * 1000;
const COLD_START_HINT_MS = 20 * 1000;

type Phase = "idle" | "deploying" | "active" | "timed_out" | "error";

function formatElapsed(ms: number): string {
  const total = Math.floor(ms / 1000);
  const mins = Math.floor(total / 60);
  const secs = total % 60;
  return mins > 0 ? `${mins}m ${secs}s` : `${secs}s`;
}

function PulseDot() {
  return (
    <span className="relative flex h-2 w-2" aria-hidden="true">
      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-violet-500 opacity-75" />
      <span className="relative inline-flex h-2 w-2 rounded-full bg-violet-500" />
    </span>
  );
}

function ProgressTrack() {
  return (
    <div
      className="mt-3 h-1 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800"
      role="progressbar"
      aria-label="Deploying model"
    >
      <div className="h-full w-1/3 animate-[deploy-slide_1.6s_ease-in-out_infinite] rounded-full bg-violet-500" />
      <style>{`@keyframes deploy-slide {
        0% { transform: translateX(-100%); }
        100% { transform: translateX(300%); }
      }`}</style>
    </div>
  );
}

export function DeploymentPanel({
  isActive,
  isDeploying,
  playgroundHref,
  onDeploy,
  onUndeploy,
  deployPending,
  undeployPending,
  deployError,
  undeployError,
}: {
  isActive: boolean;
  isDeploying: boolean;
  playgroundHref: string;
  onDeploy: () => void;
  onUndeploy: () => void;
  deployPending: boolean;
  undeployPending: boolean;
  deployError: string | null;
  undeployError: string | null;
}) {
  // Server state is the source of truth; `deployPending` only covers the
  // in-flight request, which a reload or logout would lose.
  const inProgress = isDeploying || deployPending;

  const [elapsedMs, setElapsedMs] = useState(0);
  const [timedOut, setTimedOut] = useState(false);
  const startedAtRef = useRef<number | null>(null);

  useEffect(() => {
    if (!inProgress) {
      startedAtRef.current = null;
      setElapsedMs(0);
      setTimedOut(false);
      return;
    }

    startedAtRef.current ??= Date.now();
    const tick = setInterval(() => {
      const started = startedAtRef.current;
      if (started === null) return;
      const next = Date.now() - started;
      setElapsedMs(next);
      if (next >= DEPLOY_TIMEOUT_MS) setTimedOut(true);
    }, 1000);

    return () => clearInterval(tick);
  }, [inProgress]);

  const phase: Phase = isActive
    ? "active"
    : inProgress && timedOut
      ? "timed_out"
      : inProgress
        ? "deploying"
        : deployError || undeployError
          ? "error"
          : "idle";

  const headline =
    phase === "active"
      ? "Model is actively serving requests"
      : phase === "timed_out"
        ? "Deployment is taking longer than expected"
        : phase === "deploying"
          ? "Deploying model..."
          : "Model is not deployed";

  const detail =
    phase === "active"
      ? "Chat with it in the playground, or create an API key for programmatic access."
      : phase === "timed_out"
        ? "The GPU may still be finishing in the background. Reload to check, or retry once the previous attempt is released."
        : phase === "deploying"
          ? elapsedMs > COLD_START_HINT_MS
            ? "Starting a GPU and loading the adapter. A cold start takes a few minutes."
            : "Registering the adapter with the serving engine."
          : "Deploy the model to make it available for inference.";

  return (
    <div className="rounded-lg border border-zinc-200 p-6 dark:border-zinc-800">
      <div className="flex flex-col justify-between gap-3 sm:flex-row sm:items-center">
        <div className="min-w-0">
          <p className="flex items-center gap-2 text-sm text-zinc-900 dark:text-white">
            {phase === "deploying" && <PulseDot />}
            {headline}
            {phase === "deploying" && (
              <span className="text-xs text-zinc-400 dark:text-zinc-500">
                {formatElapsed(elapsedMs)}
              </span>
            )}
          </p>
          <p className="mt-1 text-xs text-zinc-400 dark:text-zinc-500">
            {detail}
          </p>
        </div>

        <div className="flex shrink-0 gap-2">
          {isActive ? (
            <>
              <Link
                href={playgroundHref}
                className="inline-flex items-center justify-center gap-2 rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-violet-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-500 focus-visible:ring-offset-2 dark:focus-visible:ring-offset-zinc-950"
              >
                Open Playground
              </Link>
              <Button
                variant="danger"
                onClick={onUndeploy}
                loading={undeployPending}
              >
                {undeployPending ? "Undeploying..." : "Undeploy"}
              </Button>
            </>
          ) : phase === "timed_out" ? (
            <>
              <Button variant="secondary" onClick={() => location.reload()}>
                Reload status
              </Button>
              <Button onClick={onDeploy}>Retry</Button>
            </>
          ) : (
            <Button onClick={onDeploy} loading={inProgress} disabled={inProgress}>
              {inProgress ? "Deploying..." : "Deploy Model"}
            </Button>
          )}
        </div>
      </div>

      {phase === "deploying" && <ProgressTrack />}

      {deployError && (
        <p role="alert" className="mt-3 text-sm text-red-600 dark:text-red-400">
          {deployError}
        </p>
      )}
      {undeployError && (
        <p role="alert" className="mt-3 text-sm text-red-600 dark:text-red-400">
          {undeployError}
        </p>
      )}
    </div>
  );
}
