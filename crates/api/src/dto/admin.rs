use serde::Serialize;
use ts_rs::TS;
use utoipa::ToSchema;

/// Result of erasing a tenant: the tenant id and how many S3 objects were
/// wiped. Billing and audit records are intentionally retained and not counted.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TenantErasureSummary {
    pub tenant_id: String,
    pub objects_deleted: u32,
}
