"use client";

import { useEffect, useRef, useState } from "react";
import { useAuth } from "@clerk/nextjs";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";

type InAppItem = {
  id: string;
  event_type: string;
  payload: Record<string, unknown>;
  created_at: string;
  read_at: string | null;
};

function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  const diff = Date.now() - then;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function titleOf(item: InAppItem): string {
  const subject = item.payload["subject"];
  if (typeof subject === "string" && subject) return subject;
  return item.event_type.replace(/_/g, " ");
}

function bodyOf(item: InAppItem): string | null {
  const message = item.payload["message"];
  return typeof message === "string" ? message : null;
}

const PANEL_WIDTH_PX = 320;
const VIEWPORT_MARGIN_PX = 8;

export function NotificationBell({
  direction = "down",
}: {
  direction?: "up" | "down";
}) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [align, setAlign] = useState<"left" | "right">("right");
  const containerRef = useRef<HTMLDivElement>(null);

  const { data } = useQuery({
    queryKey: ["in-app-notifications"],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.notifications.getInApp(token);
    },
    refetchInterval: 30000,
  });

  const markRead = useMutation({
    mutationFn: async (id: string) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.notifications.markInAppRead(token, id);
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["in-app-notifications"] }),
  });

  const markAll = useMutation({
    mutationFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.notifications.markAllInAppRead(token);
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["in-app-notifications"] }),
  });

  // A right-aligned panel grows leftward, which leaves the viewport when the
  // trigger sits in a narrow left rail. Measure the trigger instead of trusting
  // a fixed side, so the panel stays on screen wherever the bell is mounted.
  useEffect(() => {
    if (!open) return;
    function place() {
      const anchor = containerRef.current?.getBoundingClientRect();
      if (!anchor) return;
      const fitsLeftward = anchor.right - PANEL_WIDTH_PX >= VIEWPORT_MARGIN_PX;
      setAlign(fitsLeftward ? "right" : "left");
    }
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open]);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function onClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [open]);

  const unread = data?.unread_count ?? 0;
  const items = data?.items ?? [];

  return (
    <div className="relative" ref={containerRef}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="relative p-2 rounded-md text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-white hover:bg-zinc-100 dark:hover:bg-zinc-800 transition"
        aria-label="Notifications"
      >
        <svg
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
          <path d="M13.73 21a2 2 0 0 1-3.46 0" />
        </svg>
        {unread > 0 && (
          <span className="absolute top-0.5 right-0.5 min-w-[16px] h-4 px-1 flex items-center justify-center rounded-full bg-red-500 text-white text-[10px] font-medium">
            {unread > 9 ? "9+" : unread}
          </span>
        )}
      </button>

      {open && (
        <div
          className={`absolute ${align === "right" ? "right-0" : "left-0"} ${
            direction === "up" ? "bottom-full mb-2" : "top-full mt-2"
          } w-80 max-w-[calc(100vw-1rem)] max-h-96 overflow-y-auto rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 shadow-lg z-50`}
        >
          <div className="flex items-center justify-between px-4 py-2 border-b border-zinc-200 dark:border-zinc-800">
            <span className="text-sm font-medium text-zinc-900 dark:text-white">
              Notifications
            </span>
            {unread > 0 && (
              <button
                type="button"
                onClick={() => markAll.mutate()}
                disabled={markAll.isPending}
                className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-white transition disabled:opacity-50"
              >
                Mark all read
              </button>
            )}
          </div>

          {items.length === 0 ? (
            <p className="px-4 py-6 text-sm text-zinc-500 text-center">
              No notifications yet
            </p>
          ) : (
            <ul>
              {items.map((item) => {
                const body = bodyOf(item);
                const isUnread = item.read_at === null;
                return (
                  <li key={item.id}>
                    <button
                      type="button"
                      onClick={() => isUnread && markRead.mutate(item.id)}
                      className={`w-full text-left px-4 py-3 border-b border-zinc-100 dark:border-zinc-800 hover:bg-zinc-50 dark:hover:bg-zinc-800/50 transition ${
                        isUnread ? "bg-zinc-50 dark:bg-zinc-800/30" : ""
                      }`}
                    >
                      <div className="flex items-start gap-2">
                        {isUnread && (
                          <span className="mt-1.5 w-2 h-2 rounded-full bg-blue-500 shrink-0" />
                        )}
                        <div className={isUnread ? "" : "pl-4"}>
                          <p className="text-sm font-medium text-zinc-900 dark:text-white capitalize">
                            {titleOf(item)}
                          </p>
                          {body && (
                            <p className="text-xs text-zinc-500 mt-0.5">{body}</p>
                          )}
                          <p className="text-[10px] text-zinc-400 mt-1">
                            {relativeTime(item.created_at)}
                          </p>
                        </div>
                      </div>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
