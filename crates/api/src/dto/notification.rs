use platform_db::models::{NotificationDelivery, NotificationPreference};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Notification preference returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct NotificationPreferenceResponse {
    pub id: String,
    pub channel: String,
    pub event_type: String,
    pub enabled: bool,
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
}

impl From<NotificationPreference> for NotificationPreferenceResponse {
    fn from(p: NotificationPreference) -> Self {
        Self {
            id: p.id.to_string(),
            channel: p.channel,
            event_type: p.event_type,
            enabled: p.enabled,
            config: p.config,
        }
    }
}

/// Request to batch-update notification preferences.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePreferencesRequest {
    pub preferences: Vec<PreferenceUpdate>,
}

/// Single preference update within a batch.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreferenceUpdate {
    pub channel: String,
    pub event_type: String,
    pub enabled: bool,
    #[schema(value_type = Option<Object>)]
    pub config: Option<serde_json::Value>,
}

/// Notification delivery record returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct NotificationDeliveryResponse {
    pub id: String,
    pub event_type: String,
    pub channel: String,
    pub status: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub created_at: String,
    pub sent_at: Option<String>,
}

impl From<NotificationDelivery> for NotificationDeliveryResponse {
    fn from(d: NotificationDelivery) -> Self {
        Self {
            id: d.id.to_string(),
            event_type: d.event_type,
            channel: d.channel,
            status: d.status,
            attempts: d.attempts,
            last_error: d.last_error,
            created_at: d.created_at.to_rfc3339(),
            sent_at: d.sent_at.map(|t| t.to_rfc3339()),
        }
    }
}
