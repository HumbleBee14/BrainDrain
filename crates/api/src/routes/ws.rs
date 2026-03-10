use std::collections::HashMap;

use axum::{
    Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use futures::{SinkExt, StreamExt};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;

/// WebSocket message types for real-time updates.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    /// Subscribe to updates for a specific resource
    #[serde(rename = "subscribe")]
    Subscribe { channel: String },
    /// Unsubscribe from a channel
    #[serde(rename = "unsubscribe")]
    Unsubscribe { channel: String },
    /// Server-sent update
    #[serde(rename = "update")]
    Update {
        channel: String,
        payload: serde_json::Value,
    },
    /// Heartbeat ping
    #[serde(rename = "ping")]
    Ping,
    /// Heartbeat pong
    #[serde(rename = "pong")]
    Pong,
    /// Error message
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    token: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/ws", get(ws_handler))
}

/// WebSocket upgrade handler.
///
/// Browsers cannot set custom headers on WebSocket connections, so auth is
/// handled via a `?token=` query parameter. The handler extracts the token,
/// authenticates through the same auth chain used for REST endpoints, then
/// upgrades the connection.
async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, crate::error::AppError> {
    let token = query
        .token
        .as_deref()
        .ok_or(crate::error::AppError::Unauthorized)?;

    let user = state.auth_chain().authenticate(token, state.db()).await?;

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, user, state)))
}

async fn handle_socket(socket: WebSocket, user: AuthenticatedUser, state: AppState) {
    tracing::info!(user_id = %user.user_id, "WebSocket connected");

    let (mut sender, mut receiver) = socket.split();

    // mpsc channel: reader tasks → sender loop
    let (tx, mut rx) = mpsc::channel::<String>(256);

    // Track active subscription tasks so we can cancel on unsubscribe/disconnect
    let mut subscriptions: HashMap<String, JoinHandle<()>> = HashMap::new();

    // Send connection acknowledgment
    let ack = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
    if sender.send(Message::Text(ack.into())).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            // Forward any Redis stream messages to the WS client
            Some(msg) = rx.recv() => {
                if sender.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }

            // Handle incoming WS frames from the client
            frame = receiver.next() => {
                match frame {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Text(text))) => {
                        let parsed: Result<WsMessage, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(WsMessage::Ping) => {
                                let pong = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
                                let _ = sender.send(Message::Text(pong.into())).await;
                            }
                            Ok(WsMessage::Subscribe { channel }) => {
                                handle_subscribe(
                                    &channel,
                                    &mut subscriptions,
                                    tx.clone(),
                                    state.redis(),
                                    &user,
                                )
                                .await;

                                // Acknowledge
                                let ack = serde_json::to_string(&WsMessage::Update {
                                    channel: channel.clone(),
                                    payload: serde_json::json!({"subscribed": true}),
                                })
                                .unwrap_or_default();
                                let _ = sender.send(Message::Text(ack.into())).await;
                            }
                            Ok(WsMessage::Unsubscribe { channel }) => {
                                if let Some(handle) = subscriptions.remove(&channel) {
                                    handle.abort();
                                    tracing::debug!(
                                        user_id = %user.user_id,
                                        channel = %channel,
                                        "WebSocket unsubscribed"
                                    );
                                }
                            }
                            Ok(_) => {}
                            Err(_) => {
                                let err = serde_json::to_string(&WsMessage::Error {
                                    message: "Invalid message format".to_string(),
                                })
                                .unwrap_or_default();
                                let _ = sender.send(Message::Text(err.into())).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Cancel all reader tasks on disconnect
    for (_, handle) in subscriptions {
        handle.abort();
    }

    tracing::info!(user_id = %user.user_id, "WebSocket disconnected");
}

/// Spawn a task that tails a Redis Stream and forwards entries to `tx`.
///
/// Channel format: `training:{job_id}`
/// Redis Stream key: `training:metrics:{job_id}`
///
/// Uses `XREAD BLOCK COUNT` in a loop — never misses an entry, handles
/// reconnects gracefully, and exits cleanly when the task is aborted.
async fn handle_subscribe(
    channel: &str,
    subscriptions: &mut HashMap<String, JoinHandle<()>>,
    tx: mpsc::Sender<String>,
    mut redis: redis::aio::ConnectionManager,
    user: &AuthenticatedUser,
) {
    // Already subscribed — no-op
    if subscriptions.contains_key(channel) {
        return;
    }

    let stream_key = channel_to_stream_key(channel);
    let channel_owned = channel.to_string();

    tracing::debug!(
        user_id = %user.user_id,
        channel = %channel_owned,
        stream = %stream_key,
        "WebSocket subscribing to Redis stream"
    );

    let handle = tokio::spawn(async move {
        // Start from the latest entry ("$") so we only forward new metrics
        let mut last_id = "$".to_string();

        loop {
            // XREAD BLOCK 5000 COUNT 100 STREAMS {stream_key} {last_id}
            let result: redis::RedisResult<
                Option<Vec<(String, Vec<(String, HashMap<String, String>)>)>>,
            > = redis
                .xread_options(
                    &[&stream_key],
                    &[&last_id],
                    &redis::streams::StreamReadOptions::default()
                        .block(5000)
                        .count(100),
                )
                .await;

            match result {
                Ok(Some(streams)) => {
                    for (_key, entries) in streams {
                        for (entry_id, fields) in entries {
                            last_id = entry_id.clone();

                            // Convert stream entry fields → JSON payload
                            let payload: serde_json::Value = fields
                                .into_iter()
                                .map(|(k, v)| (k, serde_json::Value::String(v)))
                                .collect::<serde_json::Map<_, _>>()
                                .into();

                            let msg = serde_json::to_string(&WsMessage::Update {
                                channel: channel_owned.clone(),
                                payload,
                            })
                            .unwrap_or_default();

                            if tx.send(msg).await.is_err() {
                                // Client disconnected
                                return;
                            }

                            // If the worker signalled train_end, stop streaming
                            // (but stay subscribed — client may unsubscribe manually)
                        }
                    }
                }
                Ok(None) => {
                    // BLOCK timeout — no new entries. Loop and wait again.
                }
                Err(e) => {
                    tracing::warn!(stream = %stream_key, error = %e, "Redis XREAD error");
                    // Brief back-off before retry
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    });

    subscriptions.insert(channel.to_string(), handle);
}

/// Map a WS channel name to the corresponding Redis stream key.
///
/// `training:{job_id}` → `training:metrics:{job_id}`
/// Everything else passes through as-is (future extensibility).
fn channel_to_stream_key(channel: &str) -> String {
    if let Some(job_id) = channel.strip_prefix("training:") {
        return format!("training:metrics:{job_id}");
    }
    channel.to_string()
}
