-- Add Stripe billing fields to tenants
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS stripe_customer_id TEXT;
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS stripe_subscription_id TEXT;
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS plan_limits JSONB NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS idx_tenants_stripe_customer ON tenants (stripe_customer_id) WHERE stripe_customer_id IS NOT NULL;

ALTER TABLE tenants ADD CONSTRAINT uq_tenants_stripe_customer UNIQUE (stripe_customer_id);
