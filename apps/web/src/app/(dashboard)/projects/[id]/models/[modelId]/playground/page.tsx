"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { useCallback, useRef, useState } from "react";
import { useModel } from "@/hooks/use-models";
import { useApiKeys, useCreateApiKey } from "@/hooks/use-api-keys";
import { useDeploymentStatus } from "@/hooks/use-deployments";
import { Breadcrumbs } from "@/components/breadcrumbs";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";

interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

export default function PlaygroundPage() {
  const params = useParams<{ id: string; modelId: string }>();
  const { data: model } = useModel(params.modelId);
  const { data: deployment } = useDeploymentStatus(params.modelId);
  const { data: apiKeys } = useApiKeys(params.modelId);
  const createApiKey = useCreateApiKey(params.modelId);

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [systemPrompt, setSystemPrompt] = useState(
    "You are a helpful assistant.",
  );
  const [temperature, setTemperature] = useState(0.7);
  const [maxTokens, setMaxTokens] = useState(512);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [playgroundKey, setPlaygroundKey] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const isActive = deployment?.deployment_status === "active";
  const keys = apiKeys ?? [];
  const activeKeys = keys.filter((k) => k.is_active);

  const ensureApiKey = useCallback(async (): Promise<string | null> => {
    if (playgroundKey) return playgroundKey;

    // Use first active key if available
    if (activeKeys.length > 0) {
      // We only have the prefix, not the full key. Need to create a new one.
    }

    // Auto-create a playground key
    try {
      const result = await createApiKey.mutateAsync({ name: "playground" });
      setPlaygroundKey(result.key);
      return result.key;
    } catch {
      setError("Failed to create API key for playground.");
      return null;
    }
  }, [playgroundKey, activeKeys, createApiKey]);

  const handleSend = async () => {
    if (!input.trim() || isLoading) return;

    const userMessage: ChatMessage = { role: "user", content: input.trim() };
    const newMessages = [...messages, userMessage];
    setMessages(newMessages);
    setInput("");
    setIsLoading(true);
    setError(null);

    const apiKey = await ensureApiKey();
    if (!apiKey) {
      setIsLoading(false);
      return;
    }

    const fullMessages: ChatMessage[] = [
      { role: "system", content: systemPrompt },
      ...newMessages,
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
      });

      if (!res.ok) {
        const body = await res
          .json()
          .catch(() => ({ error: { message: "Request failed" } }));
        throw new Error(body.error?.message || `HTTP ${res.status}`);
      }

      // SSE streaming: read tokens incrementally
      const reader = res.body?.getReader();
      if (!reader) throw new Error("No response body");

      const decoder = new TextDecoder();
      setMessages((prev) => [...prev, { role: "assistant", content: "" }]);

      let buffer = "";
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        // Keep last incomplete line in buffer
        buffer = lines.pop() || "";

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed.startsWith("data: ") || trimmed === "data: [DONE]")
            continue;

          try {
            const chunk = JSON.parse(trimmed.slice(6));
            const token = chunk.choices?.[0]?.delta?.content;
            if (token) {
              setMessages((prev) => {
                const updated = [...prev];
                const last = updated[updated.length - 1];
                updated[updated.length - 1] = {
                  ...last,
                  content: last.content + token,
                };
                return updated;
              });
            }
          } catch {
            // Skip malformed chunks
          }
        }

        messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Request failed");
    } finally {
      setIsLoading(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  if (!isActive) {
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
              { label: "Playground" },
            ]}
          />
          <h1 className="text-2xl font-bold text-zinc-900 dark:text-white">Playground</h1>
        </div>
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-8 text-center">
          <p className="text-zinc-500 mb-2">Model is not deployed.</p>
          <p className="text-xs text-zinc-400 dark:text-zinc-600 mb-4">
            Deploy the model first to use the playground.
          </p>
          <Link
            href={`/projects/${params.id}/models/${params.modelId}`}
            className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 transition"
          >
            Go to Deployment
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-[calc(100vh-120px)]">
      {/* Header */}
      <div className="mb-4 shrink-0">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: "Project", href: `/projects/${params.id}` },
            {
              label: model?.name || "Model",
              href: `/projects/${params.id}/models/${params.modelId}`,
            },
            { label: "Playground" },
          ]}
        />
        <div className="flex items-center justify-between">
          <h1 className="text-2xl font-bold text-zinc-900 dark:text-white">Playground</h1>
          <button
            onClick={() => setShowSettings(!showSettings)}
            className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-3 py-1.5 text-xs text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-600 transition"
          >
            {showSettings ? "Hide Settings" : "Settings"}
          </button>
        </div>
      </div>

      {/* Settings panel */}
      {showSettings && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-4 shrink-0 space-y-3">
          <div>
            <label className="block text-xs text-zinc-500 mb-1">
              System Prompt
            </label>
            <textarea
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
              rows={2}
              className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white resize-none"
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
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

      {/* Messages area */}
      <div className="flex-1 min-h-0 overflow-y-auto rounded-lg border border-zinc-200 dark:border-zinc-800 mb-4">
        {messages.length === 0 ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <p className="text-zinc-500">Start a conversation</p>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
                Using {model?.base_model.split("/").pop()} (fine-tuned)
              </p>
            </div>
          </div>
        ) : (
          <div className="p-4 space-y-4">
            {messages.map((msg, i) => (
              <div
                key={i}
                className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
              >
                <div
                  className={`max-w-[80%] rounded-lg px-4 py-3 text-sm ${
                    msg.role === "user"
                      ? "bg-blue-600 text-white"
                      : "bg-zinc-100 dark:bg-zinc-800 text-zinc-800 dark:text-zinc-200"
                  }`}
                >
                  <p className="whitespace-pre-wrap">{msg.content}</p>
                </div>
              </div>
            ))}
            {isLoading &&
              messages[messages.length - 1]?.role !== "assistant" && (
                <div className="flex justify-start">
                  <div className="bg-zinc-100 dark:bg-zinc-800 rounded-lg px-4 py-3 text-sm text-zinc-600 dark:text-zinc-400 animate-pulse">
                    Generating...
                  </div>
                </div>
              )}
            <div ref={messagesEndRef} />
          </div>
        )}
      </div>

      {/* Error display */}
      {error && <p className="text-sm text-red-400 mb-2 shrink-0">{error}</p>}

      {/* Input area */}
      <div className="flex gap-2 shrink-0">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Type a message... (Enter to send, Shift+Enter for newline)"
          rows={2}
          className="flex-1 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-4 py-3 text-sm text-zinc-900 dark:text-white resize-none focus:border-blue-500 focus:outline-none transition"
          disabled={isLoading}
        />
        <button
          onClick={handleSend}
          disabled={!input.trim() || isLoading}
          className="rounded-lg bg-blue-600 px-6 py-3 text-sm font-medium text-white hover:bg-blue-500 transition disabled:opacity-50 self-end"
        >
          Send
        </button>
      </div>

      {/* Clear button */}
      {messages.length > 0 && (
        <button
          onClick={() => {
            setMessages([]);
            setError(null);
          }}
          className="text-xs text-zinc-400 dark:text-zinc-600 mt-2 hover:text-zinc-600 dark:hover:text-zinc-400 transition self-center"
        >
          Clear conversation
        </button>
      )}
    </div>
  );
}
