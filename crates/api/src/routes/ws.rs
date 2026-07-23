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
use uuid::Uuid;

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

    let mut user = state.auth_chain().authenticate(token, state.db()).await?;
    crate::auth::resolve_role_and_bootstrap(&state, &mut user).await?;

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
                                match handle_subscribe(
                                    &channel,
                                    &mut subscriptions,
                                    tx.clone(),
                                    &state,
                                    &user,
                                )
                                .await
                                {
                                    Ok(()) => {
                                        let ack = serde_json::to_string(&WsMessage::Update {
                                            channel: channel.clone(),
                                            payload: serde_json::json!({"subscribed": true}),
                                        })
                                        .unwrap_or_default();
                                        let _ = sender.send(Message::Text(ack.into())).await;
                                    }
                                    Err(message) => {
                                        let err = serde_json::to_string(&WsMessage::Error {
                                            message,
                                        })
                                        .unwrap_or_default();
                                        let _ = sender.send(Message::Text(err.into())).await;
                                    }
                                }
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

/// Authorize a subscription and, on success, spawn a task that tails the
/// corresponding Redis Stream and forwards entries to `tx`.
///
/// Channel format: `training:{job_id}`
/// Redis Stream key: `training:metrics:{job_id}`
///
/// Returns `Err(message)` if the channel is unsupported or the resource does
/// not belong to the caller's tenant. Uses `XREAD BLOCK COUNT` in a loop —
/// never misses an entry, handles reconnects gracefully, and exits cleanly
/// when the task is aborted.
async fn handle_subscribe(
    channel: &str,
    subscriptions: &mut HashMap<String, JoinHandle<()>>,
    tx: mpsc::Sender<String>,
    state: &AppState,
    user: &AuthenticatedUser,
) -> Result<(), String> {
    // Already subscribed — no-op
    if subscriptions.contains_key(channel) {
        return Ok(());
    }

    // Authorize before tailing any stream: the client-supplied channel must map
    // to a resource owned by the caller's tenant. Unknown channel shapes are
    // denied so a client can never point a subscription at an arbitrary Redis key.
    let stream_key = authorize_channel(channel, state, user).await?;

    let channel_owned = channel.to_string();
    let mut redis = state.redis();

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
            #[allow(clippy::type_complexity)]
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
    Ok(())
}

/// Validate that `user` may subscribe to `channel` and return the Redis stream
/// key to tail.
///
/// Only `training:{job_id}` channels are supported, and the job must belong to
/// the caller's tenant — this is the authorization gate for the live metrics
/// stream. Any other channel shape is rejected.
async fn authorize_channel(
    channel: &str,
    state: &AppState,
    user: &AuthenticatedUser,
) -> Result<String, String> {
    let job_id = parse_training_channel(channel).ok_or_else(|| "Unsupported channel".to_string())?;

    match state
        .training_job_repo()
        .get_by_id(user.tenant_id, job_id)
        .await
    {
        Ok(Some(_)) => Ok(format!("training:metrics:{job_id}")),
        Ok(None) => Err("Resource not found".to_string()),
        Err(e) => {
            tracing::error!(error = %e, "WS subscribe authorization lookup failed");
            Err("Authorization check failed".to_string())
        }
    }
}

/// Parse a `training:{job_id}` channel into its job UUID. Returns `None` for any
/// other channel shape or a malformed UUID, so only well-formed training
/// channels are ever authorized.
fn parse_training_channel(channel: &str) -> Option<Uuid> {
    let job_id = channel.strip_prefix("training:")?;
    Uuid::parse_str(job_id).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_training_channel() {
        let id = Uuid::new_v4();
        assert_eq!(parse_training_channel(&format!("training:{id}")), Some(id));
    }

    #[test]
    fn rejects_unknown_prefix_and_malformed_uuid() {
        assert_eq!(parse_training_channel("billing:events"), None);
        assert_eq!(parse_training_channel("training:not-a-uuid"), None);
        assert_eq!(parse_training_channel("training:"), None);
        // No prefix at all must never map onto a raw Redis key.
        assert_eq!(parse_training_channel("training:metrics:secret"), None);
    }
}
