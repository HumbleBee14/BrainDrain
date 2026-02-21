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
use serde::{Deserialize, Serialize};

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

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, user)))
}

async fn handle_socket(socket: WebSocket, user: AuthenticatedUser) {
    tracing::info!(user_id = %user.user_id, "WebSocket connected");

    let (mut sender, mut receiver) = socket.split();

    // Send connection acknowledgment
    let ack = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
    if sender.send(Message::Text(ack.into())).await.is_err() {
        return;
    }

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(data)) => {
                let _ = sender.send(Message::Pong(data)).await;
                continue;
            }
            Err(_) => break,
            _ => continue,
        };

        // Parse incoming message
        let parsed: Result<WsMessage, _> = serde_json::from_str(&msg);
        match parsed {
            Ok(WsMessage::Ping) => {
                let pong = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
                let _ = sender.send(Message::Text(pong.into())).await;
            }
            Ok(WsMessage::Subscribe { channel }) => {
                tracing::debug!(
                    user_id = %user.user_id,
                    channel = %channel,
                    "WebSocket subscribe"
                );
                // TODO: Register subscription in Redis pub/sub
                // For now, acknowledge the subscription
                let ack = serde_json::to_string(&WsMessage::Update {
                    channel: channel.clone(),
                    payload: serde_json::json!({"subscribed": true}),
                })
                .unwrap_or_default();
                let _ = sender.send(Message::Text(ack.into())).await;
            }
            Ok(WsMessage::Unsubscribe { channel }) => {
                tracing::debug!(
                    user_id = %user.user_id,
                    channel = %channel,
                    "WebSocket unsubscribe"
                );
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

    tracing::info!(user_id = %user.user_id, "WebSocket disconnected");
}
