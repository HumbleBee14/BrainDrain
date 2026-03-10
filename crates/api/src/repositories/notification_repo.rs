use platform_db::models::{NotificationDelivery, NotificationPreference};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, NotificationRepository};

/// PostgreSQL implementation of the notification repository.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgNotificationRepo {
    db: PgPool,
}

impl PgNotificationRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl NotificationRepository for PgNotificationRepo {
    fn list_preferences(
        &self,
        tenant_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationPreference>>> {
        Box::pin(async move {
            let prefs = sqlx::query_as::<_, NotificationPreference>(
                r#"
                SELECT * FROM notification_preferences
                WHERE tenant_id = $1
                ORDER BY channel, event_type
                "#,
            )
            .bind(tenant_id)
            .fetch_all(&self.db)
            .await?;

            Ok(prefs)
        })
    }

    fn upsert_preference(
        &self,
        tenant_id: Uuid,
        channel: &str,
        event_type: &str,
        enabled: bool,
        config: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<NotificationPreference>> {
        let channel = channel.to_string();
        let event_type = event_type.to_string();
        Box::pin(async move {
            let pref = sqlx::query_as::<_, NotificationPreference>(
                r#"
                INSERT INTO notification_preferences (tenant_id, channel, event_type, enabled, config)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (tenant_id, channel, event_type)
                DO UPDATE SET enabled = EXCLUDED.enabled, config = EXCLUDED.config, updated_at = NOW()
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&channel)
            .bind(&event_type)
            .bind(enabled)
            .bind(config)
            .fetch_one(&self.db)
            .await?;

            Ok(pref)
        })
    }

    fn get_enabled_preferences(
        &self,
        tenant_id: Uuid,
        event_type: &str,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationPreference>>> {
        let event_type = event_type.to_string();
        Box::pin(async move {
            let prefs = sqlx::query_as::<_, NotificationPreference>(
                r#"
                SELECT * FROM notification_preferences
                WHERE tenant_id = $1 AND event_type = $2 AND enabled = true
                "#,
            )
            .bind(tenant_id)
            .bind(&event_type)
            .fetch_all(&self.db)
            .await?;

            Ok(prefs)
        })
    }

    fn create_delivery(
        &self,
        tenant_id: Uuid,
        preference_id: Uuid,
        event_type: &str,
        channel: &str,
        payload: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<NotificationDelivery>> {
        let event_type = event_type.to_string();
        let channel = channel.to_string();
        Box::pin(async move {
            let delivery = sqlx::query_as::<_, NotificationDelivery>(
                r#"
                INSERT INTO notification_deliveries
                    (tenant_id, preference_id, event_type, channel, payload)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(preference_id)
            .bind(&event_type)
            .bind(&channel)
            .bind(payload)
            .fetch_one(&self.db)
            .await?;

            Ok(delivery)
        })
    }

    fn list_deliveries(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationDelivery>>> {
        Box::pin(async move {
            let deliveries = sqlx::query_as::<_, NotificationDelivery>(
                r#"
                SELECT * FROM notification_deliveries
                WHERE tenant_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(tenant_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

            Ok(deliveries)
        })
    }

    fn count_deliveries(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM notification_deliveries WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn update_delivery_status(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> BoxFuture<'_, AppResult<()>> {
        let status = status.to_string();
        let error = error.map(|s| s.to_string());
        Box::pin(async move {
            sqlx::query(
                r#"
                UPDATE notification_deliveries
                SET status = $3, last_error = $4, attempts = attempts + 1,
                    sent_at = CASE WHEN $3 = 'sent' THEN NOW() ELSE sent_at END
                WHERE id = $1 AND tenant_id = $2
                "#,
            )
            .bind(id)
            .bind(tenant_id)
            .bind(&status)
            .bind(error.as_deref())
            .execute(&self.db)
            .await?;

            Ok(())
        })
    }

    fn get_delivery(
        &self,
        tenant_id: Uuid,
        delivery_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<NotificationDelivery>>> {
        Box::pin(async move {
            let delivery = sqlx::query_as::<_, NotificationDelivery>(
                "SELECT * FROM notification_deliveries WHERE id = $1 AND tenant_id = $2",
            )
            .bind(delivery_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(delivery)
        })
    }

    fn get_preference(
        &self,
        tenant_id: Uuid,
        preference_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<NotificationPreference>>> {
        Box::pin(async move {
            let pref = sqlx::query_as::<_, NotificationPreference>(
                "SELECT * FROM notification_preferences WHERE id = $1 AND tenant_id = $2",
            )
            .bind(preference_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(pref)
        })
    }

    fn list_pending_deliveries(
        &self,
        max_attempts: i32,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationDelivery>>> {
        Box::pin(async move {
            let deliveries = sqlx::query_as::<_, NotificationDelivery>(
                r#"
                SELECT * FROM notification_deliveries
                WHERE (status = 'pending' OR (status = 'failed' AND attempts < $1))
                ORDER BY created_at ASC
                LIMIT $2
                "#,
            )
            .bind(max_attempts)
            .bind(limit)
            .fetch_all(&self.db)
            .await?;

            Ok(deliveries)
        })
    }
}
