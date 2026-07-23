-- In-app notification read state. In-app deliveries are read directly by the
-- client (bell menu); read_at NULL means unread.
ALTER TABLE notification_deliveries ADD COLUMN IF NOT EXISTS read_at TIMESTAMPTZ;

-- Unread-count / in-app list query: channel + tenant, newest first.
CREATE INDEX IF NOT EXISTS idx_notification_deliveries_in_app
    ON notification_deliveries(tenant_id, created_at DESC)
    WHERE channel = 'in_app';
