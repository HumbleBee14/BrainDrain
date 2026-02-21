/**
 * WebSocket client for real-time updates.
 *
 * Provides a singleton WebSocket connection with automatic reconnection,
 * channel subscriptions, and heartbeat keepalive.
 *
 * Channels follow the pattern: "resource:id" (e.g., "training_job:uuid")
 */

type MessageHandler = (payload: unknown) => void;

interface WsMessage {
  type: "subscribe" | "unsubscribe" | "update" | "ping" | "pong" | "error";
  data?: {
    channel?: string;
    payload?: unknown;
    message?: string;
  };
}

const WS_URL = (process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000")
  .replace(/^http/, "ws");

const HEARTBEAT_INTERVAL_MS = 30_000;
const RECONNECT_BASE_MS = 1_000;
const RECONNECT_MAX_MS = 30_000;

class WebSocketClient {
  private ws: WebSocket | null = null;
  private subscriptions = new Map<string, Set<MessageHandler>>();
  private reconnectDelay = RECONNECT_BASE_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private token: string | null = null;
  private connecting = false;

  /** Set the auth token (call when token refreshes) */
  setToken(token: string) {
    this.token = token;
  }

  /** Connect to the WebSocket server */
  connect() {
    if (this.ws?.readyState === WebSocket.OPEN || this.connecting) return;
    this.connecting = true;

    const url = `${WS_URL}/api/v1/ws${this.token ? `?token=${this.token}` : ""}`;

    try {
      this.ws = new WebSocket(url);
    } catch {
      this.connecting = false;
      this.scheduleReconnect();
      return;
    }

    this.ws.onopen = () => {
      this.connecting = false;
      this.reconnectDelay = RECONNECT_BASE_MS;
      this.startHeartbeat();

      // Re-subscribe to all active channels
      for (const channel of this.subscriptions.keys()) {
        this.send({ type: "subscribe", data: { channel } });
      }
    };

    this.ws.onmessage = (event) => {
      try {
        const msg: WsMessage = JSON.parse(event.data);
        if (msg.type === "update" && msg.data?.channel) {
          const handlers = this.subscriptions.get(msg.data.channel);
          if (handlers) {
            for (const handler of handlers) {
              handler(msg.data.payload);
            }
          }
        }
      } catch {
        // Ignore invalid messages
      }
    };

    this.ws.onclose = () => {
      this.connecting = false;
      this.stopHeartbeat();
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      // onclose will fire after this
    };
  }

  /** Subscribe to a channel */
  subscribe(channel: string, handler: MessageHandler): () => void {
    if (!this.subscriptions.has(channel)) {
      this.subscriptions.set(channel, new Set());
      // Send subscribe message if connected
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.send({ type: "subscribe", data: { channel } });
      }
    }
    this.subscriptions.get(channel)!.add(handler);

    // Ensure we're connected
    this.connect();

    // Return unsubscribe function
    return () => {
      const handlers = this.subscriptions.get(channel);
      if (handlers) {
        handlers.delete(handler);
        if (handlers.size === 0) {
          this.subscriptions.delete(channel);
          if (this.ws?.readyState === WebSocket.OPEN) {
            this.send({ type: "unsubscribe", data: { channel } });
          }
        }
      }
    };
  }

  /** Disconnect and clean up */
  disconnect() {
    this.stopHeartbeat();
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    this.subscriptions.clear();
  }

  private send(msg: WsMessage) {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  private startHeartbeat() {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      this.send({ type: "ping" });
    }, HEARTBEAT_INTERVAL_MS);
  }

  private stopHeartbeat() {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  private scheduleReconnect() {
    if (this.reconnectTimer) return;
    if (this.subscriptions.size === 0) return; // No active subscriptions

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, this.reconnectDelay);

    this.reconnectDelay = Math.min(this.reconnectDelay * 2, RECONNECT_MAX_MS);
  }
}

/** Singleton WebSocket client instance */
export const wsClient = new WebSocketClient();
