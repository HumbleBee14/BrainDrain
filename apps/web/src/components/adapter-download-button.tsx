"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "@clerk/nextjs";
import { toast } from "sonner";

import { api, ApiClientError } from "@/lib/api-client";

/** Abort a stalled download rather than leaving the button spinning forever. */
const DOWNLOAD_TIMEOUT_MS = 5 * 60 * 1000;
/** How long the success state stays visible before returning to idle. */
const SUCCESS_RESET_MS = 4000;

type Phase = "idle" | "preparing" | "downloading" | "done" | "error";

interface Props {
  modelId: string;
  /** Adapter byte size, when known, to render a total alongside progress. */
  sizeBytes?: number | null;
  /** False while training is incomplete — there is nothing to download yet. */
  available: boolean;
  className?: string;
}

export function AdapterDownloadButton({
  modelId,
  sizeBytes,
  available,
  className = "",
}: Props) {
  const { getToken } = useAuth();
  const [phase, setPhase] = useState<Phase>("idle");
  const [loaded, setLoaded] = useState(0);
  const [total, setTotal] = useState<number | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const abortRef = useRef<AbortController | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const resetRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const objectUrlRef = useRef<string | null>(null);
  const timedOutRef = useRef(false);

  const clearTimers = useCallback(() => {
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    if (resetRef.current) clearTimeout(resetRef.current);
    timeoutRef.current = null;
    resetRef.current = null;
  }, []);

  useEffect(
    () => () => {
      clearTimers();
      abortRef.current?.abort();
      if (objectUrlRef.current) URL.revokeObjectURL(objectUrlRef.current);
    },
    [clearTimers],
  );

  const cancel = useCallback(() => {
    abortRef.current?.abort();
    clearTimers();
    setPhase("idle");
    setLoaded(0);
    setTotal(null);
  }, [clearTimers]);

  const start = useCallback(async () => {
    clearTimers();
    setErrorMessage(null);
    setLoaded(0);
    setTotal(sizeBytes ?? null);
    setPhase("preparing");

    const controller = new AbortController();
    abortRef.current = controller;
    timedOutRef.current = false;
    timeoutRef.current = setTimeout(() => {
      timedOutRef.current = true;
      controller.abort();
    }, DOWNLOAD_TIMEOUT_MS);

    try {
      const token = await getToken();
      if (!token) throw new Error("Your session expired — sign in again.");

      const { blob, filename } = await api.models.downloadAdapter(token, modelId, {
        signal: controller.signal,
        onProgress: (bytes, contentLength) => {
          setPhase("downloading");
          setLoaded(bytes);
          if (contentLength) setTotal(contentLength);
        },
      });

      const url = URL.createObjectURL(blob);
      objectUrlRef.current = url;
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = filename;
      anchor.click();
      // Revoke on the next tick so the browser has committed the download.
      setTimeout(() => {
        URL.revokeObjectURL(url);
        if (objectUrlRef.current === url) objectUrlRef.current = null;
      }, 0);

      clearTimers();
      setPhase("done");
      toast.success(`Adapter downloaded (${filename})`);
      resetRef.current = setTimeout(() => setPhase("idle"), SUCCESS_RESET_MS);
    } catch (err) {
      clearTimers();
      if (controller.signal.aborted) {
        // A user cancel already reset the button; only a timeout needs surfacing.
        if (timedOutRef.current) {
          setPhase("error");
          setErrorMessage("Download timed out. Check your connection and retry.");
          toast.error("Adapter download timed out");
        }
        return;
      }
      const message =
        err instanceof ApiClientError
          ? err.message
          : err instanceof Error
            ? err.message
            : "Download failed";
      setPhase("error");
      setErrorMessage(message);
      toast.error(message);
    }
  }, [clearTimers, getToken, modelId, sizeBytes]);

  if (!available) {
    return (
      <button
        type="button"
        disabled
        title="Available once training completes"
        className={`inline-flex items-center gap-2 rounded-lg border border-zinc-200 dark:border-zinc-800 px-3 py-1.5 text-xs text-zinc-400 dark:text-zinc-600 cursor-not-allowed ${className}`}
      >
        <DownloadIcon />
        Adapter unavailable
      </button>
    );
  }

  const busy = phase === "preparing" || phase === "downloading";
  const percent =
    total && total > 0 ? Math.min(100, Math.round((loaded / total) * 100)) : null;

  return (
    <div className={`inline-flex flex-col gap-1 ${className}`}>
      <div className="inline-flex items-center gap-2">
        <button
          type="button"
          onClick={busy ? undefined : start}
          disabled={busy}
          aria-busy={busy}
          className={buttonClass(phase)}
        >
          {phase === "preparing" && (
            <>
              <Spinner />
              Preparing…
            </>
          )}
          {phase === "downloading" && (
            <>
              <Spinner />
              {percent !== null
                ? `Downloading ${percent}%`
                : `Downloading ${formatBytes(loaded)}`}
            </>
          )}
          {phase === "done" && (
            <>
              <CheckIcon />
              Downloaded
            </>
          )}
          {phase === "error" && (
            <>
              <RetryIcon />
              Retry download
            </>
          )}
          {phase === "idle" && (
            <>
              <DownloadIcon />
              Download adapter
              {sizeBytes ? (
                <span className="text-zinc-400 dark:text-zinc-500">
                  ({formatBytes(sizeBytes)})
                </span>
              ) : null}
            </>
          )}
        </button>

        {busy && (
          <button
            type="button"
            onClick={cancel}
            className="rounded-lg border border-zinc-200 dark:border-zinc-800 px-2 py-1.5 text-xs text-zinc-500 hover:bg-zinc-50 dark:hover:bg-zinc-800/60 transition"
          >
            Cancel
          </button>
        )}
      </div>

      {phase === "downloading" && percent !== null && (
        <div
          className="h-1 w-full overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800"
          role="progressbar"
          aria-valuenow={percent}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div
            className="h-full bg-zinc-900 dark:bg-zinc-100 transition-[width] duration-200"
            style={{ width: `${percent}%` }}
          />
        </div>
      )}

      {phase === "error" && errorMessage && (
        <p className="text-xs text-red-600 dark:text-red-400 max-w-xs">
          {errorMessage}
        </p>
      )}
    </div>
  );
}

function buttonClass(phase: Phase): string {
  const base =
    "inline-flex items-center gap-2 rounded-lg border px-3 py-1.5 text-xs transition disabled:cursor-default";
  if (phase === "done") {
    return `${base} border-emerald-200 dark:border-emerald-800 text-emerald-700 dark:text-emerald-400 bg-emerald-50 dark:bg-emerald-900/30`;
  }
  if (phase === "error") {
    return `${base} border-red-200 dark:border-red-800 text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/30`;
  }
  return `${base} border-zinc-200 dark:border-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-50 dark:hover:bg-zinc-800/60`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function Spinner() {
  return (
    <svg
      className="h-3 w-3 animate-spin"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle
        className="opacity-25"
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeWidth="4"
      />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z"
      />
    </svg>
  );
}

function DownloadIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12" y1="15" x2="12" y2="3" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}

function RetryIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
    </svg>
  );
}
