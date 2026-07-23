"use client";

import { useParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState, useMemo } from "react";
import { useModels } from "@/hooks/use-models";
import { useCreateApiKey } from "@/hooks/use-api-keys";
import { Breadcrumbs } from "@/components/breadcrumbs";
import { useProject } from "@/hooks/use-projects";
import {
  getStoredPlaygroundKey,
  storePlaygroundKey,
  clearStoredPlaygroundKey,
} from "@/lib/playground-key";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";

interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

interface PanelState {
  modelId: string;
  messages: ChatMessage[];
  isLoading: boolean;
  error: string | null;
  apiKey: string | null;
}

const INITIAL_PANEL: PanelState = {
  modelId: "",
  messages: [],
  isLoading: false,
  error: null,
  apiKey: null,
};

function ChatPanel({
  label,
  color,
  state,
  onModelChange,
  deployedModels,
  otherModelId,
  messagesEndRef,
}: {
  label: string;
  color: "violet" | "emerald";
  state: PanelState;
  onModelChange: (modelId: string) => void;
  deployedModels: { id: string; name: string; base_model: string }[];
  otherModelId: string;
  messagesEndRef: React.RefObject<HTMLDivElement | null>;
}) {
  const borderColor =
    color === "violet" ? "border-violet-200 dark:border-violet-800" : "border-emerald-200 dark:border-emerald-800";
  const headerBg =
    color === "violet" ? "bg-violet-100/20 dark:bg-violet-900/20" : "bg-emerald-100/20 dark:bg-emerald-900/20";
  const dotColor = color === "violet" ? "bg-violet-500" : "bg-emerald-500";

  return (
    <div className={`flex flex-col min-h-0 rounded-lg border ${borderColor}`}>
      {/* Panel header with model selector */}
      <div className={`${headerBg} px-4 py-3 shrink-0`}>
        <div className="flex items-center gap-2 mb-2">
          <span className={`w-2 h-2 rounded-full ${dotColor}`} />
          <span className="text-xs font-medium text-zinc-600 dark:text-zinc-400 uppercase tracking-wider">
            {label}
          </span>
        </div>
        <select
          value={state.modelId}
          onChange={(e) => onModelChange(e.target.value)}
          className="w-full rounded-md border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-1.5 text-sm text-zinc-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-zinc-500"
        >
          <option value="">Select a model...</option>
          {deployedModels.map((m) => (
            <option
              key={m.id}
              value={m.id}
              disabled={m.id === otherModelId}
            >
              {m.name} ({m.base_model.split("/").pop()})
            </option>
          ))}
        </select>
      </div>

      {/* Messages */}
      <div className="flex-1 min-h-0 overflow-y-auto">
        {!state.modelId ? (
          <div className="flex items-center justify-center h-full p-8">
            <p className="text-zinc-400 dark:text-zinc-600 text-sm">Select a deployed model</p>
          </div>
        ) : state.messages.length === 0 ? (
          <div className="flex items-center justify-center h-full p-8">
            <p className="text-zinc-400 dark:text-zinc-600 text-sm">
              Waiting for messages...
            </p>
          </div>
        ) : (
          <div className="p-3 space-y-3">
            {state.messages.map((msg, i) => (
              <div
                key={i}
                className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
              >
                <div
                  className={`max-w-[90%] rounded-lg px-3 py-2 text-sm ${
                    msg.role === "user"
                      ? "bg-zinc-200 dark:bg-zinc-700 text-zinc-900 dark:text-white"
                      : "bg-zinc-100/50 dark:bg-zinc-800/50 text-zinc-800 dark:text-zinc-200"
                  }`}
                >
                  <p className="whitespace-pre-wrap break-words">
                    {msg.content}
                  </p>
                </div>
              </div>
            ))}
            {state.isLoading &&
              state.messages[state.messages.length - 1]?.role !== "assistant" && (
                <div className="flex justify-start">
                  <div className="bg-zinc-100/50 dark:bg-zinc-800/50 rounded-lg px-3 py-2 text-sm text-zinc-500 animate-pulse">
                    Generating...
                  </div>
                </div>
              )}
            <div ref={messagesEndRef} />
          </div>
        )}
      </div>

      {/* Error */}
      {state.error && (
        <div className="px-3 py-2 shrink-0">
          <p className="text-xs text-red-400">{state.error}</p>
        </div>
      )}
    </div>
  );
}

export default function ABPlaygroundPage() {
  const params = useParams<{ id: string }>();
  const { data: project } = useProject(params.id);
  const { data: modelsData } = useModels(params.id, 0, 50);

  const [panelA, setPanelA] = useState<PanelState>({ ...INITIAL_PANEL });
  const [panelB, setPanelB] = useState<PanelState>({ ...INITIAL_PANEL });
  const [input, setInput] = useState("");
  const [systemPrompt, setSystemPrompt] = useState(
    "You are a helpful assistant.",
  );
  const [temperature, setTemperature] = useState(0.7);
  const [maxTokens, setMaxTokens] = useState(512);
  const [showSettings, setShowSettings] = useState(false);

  const refA = useRef<HTMLDivElement>(null);
  const refB = useRef<HTMLDivElement>(null);
  const abortControllerRef = useRef<AbortController | null>(null);

  // Abort any in-flight streaming requests on unmount
  useEffect(() => {
    return () => {
      abortControllerRef.current?.abort();
    };
  }, []);

  // API key hooks — conditionally used based on selected models
  const createKeyA = useCreateApiKey(panelA.modelId);
  const createKeyB = useCreateApiKey(panelB.modelId);

  // Filter to deployed models only
  const deployedModels = useMemo(() => {
    if (!modelsData?.data) return [];
    return modelsData.data.filter((m) => m.deployment_status === "active");
  }, [modelsData?.data]);

  const ensureApiKey = useCallback(
    async (
      panel: PanelState,
      setPanel: React.Dispatch<React.SetStateAction<PanelState>>,
      createKey: typeof createKeyA,
    ): Promise<string | null> => {
      if (panel.apiKey) return panel.apiKey;

      const stored = getStoredPlaygroundKey(panel.modelId);
      if (stored) {
        setPanel((prev) => ({ ...prev, apiKey: stored }));
        return stored;
      }

      try {
        const result = await createKey.mutateAsync({ name: "playground-ab" });
        setPanel((prev) => ({ ...prev, apiKey: result.key }));
        storePlaygroundKey(panel.modelId, result.key);
        return result.key;
      } catch {
        setPanel((prev) => ({
          ...prev,
          error: "Failed to create API key",
        }));
        return null;
      }
    },
    [],
  );

  const streamToPanel = useCallback(
    async (
      apiKey: string,
      modelId: string,
      messages: ChatMessage[],
      setPanel: React.Dispatch<React.SetStateAction<PanelState>>,
      scrollRef: React.RefObject<HTMLDivElement | null>,
      signal: AbortSignal,
    ) => {
      const fullMessages: ChatMessage[] = [
        { role: "system", content: systemPrompt },
        ...messages,
      ];

      try {
        const res = await fetch(`${API_URL}/v1/chat/completions`, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Authorization: `Bearer ${apiKey}`,
          },
          body: JSON.stringify({
            messages: fullMessages,
            temperature,
            max_tokens: maxTokens,
            stream: true,
          }),
          signal,
        });

        if (!res.ok) {
          if (res.status === 401 || res.status === 403) {
            clearStoredPlaygroundKey(modelId);
            setPanel((prev) => ({ ...prev, apiKey: null }));
          }
          const body = await res
            .json()
            .catch(() => ({ error: { message: "Request failed" } }));
          throw new Error(body.error?.message || `HTTP ${res.status}`);
        }

        const reader = res.body?.getReader();
        if (!reader) throw new Error("No response body");

        const decoder = new TextDecoder();
        setPanel((prev) => ({
          ...prev,
          messages: [...prev.messages, { role: "assistant", content: "" }],
        }));

        let buffer = "";
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop() || "";

          for (const line of lines) {
            const trimmed = line.trim();
            if (!trimmed.startsWith("data: ") || trimmed === "data: [DONE]")
              continue;

            try {
              const chunk = JSON.parse(trimmed.slice(6));
              const token = chunk.choices?.[0]?.delta?.content;
              if (token) {
                setPanel((prev) => {
                  const updated = [...prev.messages];
                  const last = updated[updated.length - 1];
                  updated[updated.length - 1] = {
                    ...last,
                    content: last.content + token,
                  };
                  return { ...prev, messages: updated };
                });
              }
            } catch {
              // Skip malformed chunks
            }
          }

          scrollRef.current?.scrollIntoView({ behavior: "smooth" });
        }
      } catch (err) {
        if (err instanceof DOMException && err.name === "AbortError") return;
        setPanel((prev) => ({
          ...prev,
          error: err instanceof Error ? err.message : "Request failed",
        }));
      } finally {
        setPanel((prev) => ({ ...prev, isLoading: false }));
      }
    },
    [systemPrompt, temperature, maxTokens],
  );

  const handleSend = async () => {
    if (!input.trim()) return;
    if (panelA.isLoading || panelB.isLoading) return;

    const userMessage: ChatMessage = { role: "user", content: input.trim() };
    setInput("");

    // Add user message to both panels
    const panelsToSend: {
      panel: PanelState;
      setPanel: React.Dispatch<React.SetStateAction<PanelState>>;
      createKey: typeof createKeyA;
      scrollRef: React.RefObject<HTMLDivElement | null>;
    }[] = [];

    if (panelA.modelId) {
      setPanelA((prev) => ({
        ...prev,
        messages: [...prev.messages, userMessage],
        isLoading: true,
        error: null,
      }));
      panelsToSend.push({
        panel: panelA,
        setPanel: setPanelA,
        createKey: createKeyA,
        scrollRef: refA,
      });
    }

    if (panelB.modelId) {
      setPanelB((prev) => ({
        ...prev,
        messages: [...prev.messages, userMessage],
        isLoading: true,
        error: null,
      }));
      panelsToSend.push({
        panel: panelB,
        setPanel: setPanelB,
        createKey: createKeyB,
        scrollRef: refB,
      });
    }

    if (panelsToSend.length === 0) return;

    // Abort any in-flight requests from the previous send
    abortControllerRef.current?.abort();
    const controller = new AbortController();
    abortControllerRef.current = controller;

    // Fire both requests in parallel
    await Promise.allSettled(
      panelsToSend.map(async ({ panel, setPanel, createKey, scrollRef }) => {
        const apiKey = await ensureApiKey(panel, setPanel, createKey);
        if (!apiKey) {
          setPanel((prev) => ({ ...prev, isLoading: false }));
          return;
        }
        const msgs = [...panel.messages, userMessage];
        await streamToPanel(
          apiKey,
          panel.modelId,
          msgs,
          setPanel,
          scrollRef,
          controller.signal,
        );
      }),
    );
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleClear = () => {
    setPanelA((prev) => ({ ...prev, messages: [], error: null }));
    setPanelB((prev) => ({ ...prev, messages: [], error: null }));
  };

  const handleModelChangeA = (modelId: string) => {
    setPanelA({ ...INITIAL_PANEL, modelId });
  };

  const handleModelChangeB = (modelId: string) => {
    setPanelB({ ...INITIAL_PANEL, modelId });
  };

  const canSend =
    (panelA.modelId || panelB.modelId) &&
    !panelA.isLoading &&
    !panelB.isLoading;

  return (
    <div className="flex flex-col h-[calc(100vh-160px)] md:h-[calc(100vh-120px)]">
      {/* Header */}
      <div className="mb-4 shrink-0">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: project?.name || "Project", href: `/projects/${params.id}` },
            { label: "A/B Playground" },
          ]}
        />
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">A/B Playground</h1>
            <p className="text-sm text-zinc-500 mt-0.5">
              Compare two models side-by-side with the same prompt
            </p>
          </div>
          <div className="flex items-center gap-2">
            {(panelA.messages.length > 0 || panelB.messages.length > 0) && (
              <button
                onClick={handleClear}
                className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-3 py-1.5 text-xs text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-600 transition"
              >
                Clear All
              </button>
            )}
            <button
              onClick={() => setShowSettings(!showSettings)}
              className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-3 py-1.5 text-xs text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-600 transition"
            >
              {showSettings ? "Hide Settings" : "Settings"}
            </button>
          </div>
        </div>
      </div>

      {/* No deployed models message */}
      {deployedModels.length === 0 && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-8 text-center mb-4 shrink-0">
          <p className="text-zinc-500 mb-1">No deployed models found.</p>
          <p className="text-xs text-zinc-400 dark:text-zinc-600">
            Deploy at least one model from the project page to use the A/B
            playground.
          </p>
        </div>
      )}

      {/* Settings panel */}
      {showSettings && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-4 shrink-0 space-y-3">
          <div>
            <label className="block text-xs text-zinc-500 mb-1">
              System Prompt (shared)
            </label>
            <textarea
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
              rows={2}
              className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white resize-none"
            />
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="block text-xs text-zinc-500 mb-1">
                Temperature: {temperature}
              </label>
              <input
                type="range"
                min={0}
                max={2}
                step={0.1}
                value={temperature}
                onChange={(e) => setTemperature(parseFloat(e.target.value))}
                className="w-full accent-blue-500"
              />
            </div>
            <div>
              <label className="block text-xs text-zinc-500 mb-1">
                Max Tokens: {maxTokens}
              </label>
              <input
                type="range"
                min={64}
                max={4096}
                step={64}
                value={maxTokens}
                onChange={(e) => setMaxTokens(parseInt(e.target.value))}
                className="w-full accent-blue-500"
              />
            </div>
          </div>
        </div>
      )}

      {/* Split-screen chat panels */}
      <div className="flex-1 min-h-0 grid grid-cols-1 lg:grid-cols-2 gap-3 md:gap-4 mb-4">
        <ChatPanel
          label="Model A"
          color="violet"
          state={panelA}
          onModelChange={handleModelChangeA}
          deployedModels={deployedModels}
          otherModelId={panelB.modelId}
          messagesEndRef={refA}
        />
        <ChatPanel
          label="Model B"
          color="emerald"
          state={panelB}
          onModelChange={handleModelChangeB}
          deployedModels={deployedModels}
          otherModelId={panelA.modelId}
          messagesEndRef={refB}
        />
      </div>

      {/* Shared input area */}
      <div className="flex gap-2 shrink-0">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            canSend
              ? "Type a message... (Enter to send, Shift+Enter for newline)"
              : "Select at least one model to start"
          }
          rows={2}
          className="flex-1 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-4 py-3 text-sm text-zinc-900 dark:text-white resize-none focus:border-blue-500 focus:outline-none transition disabled:opacity-50"
          disabled={!canSend}
        />
        <button
          onClick={handleSend}
          disabled={!input.trim() || !canSend}
          className="rounded-lg bg-blue-600 px-6 py-3 text-sm font-medium text-white hover:bg-blue-500 transition disabled:opacity-50 self-end"
        >
          Send
        </button>
      </div>
    </div>
  );
}
