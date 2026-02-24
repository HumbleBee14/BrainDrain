use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Pagination query parameters with enforced bounds.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PaginationParams {
    #[serde(default = "default_offset")]
    offset: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

impl PaginationParams {
    pub fn offset(&self) -> i64 {
        self.offset.max(0)
    }

    pub fn limit(&self) -> i64 {
        self.limit.clamp(1, 100)
    }
}

fn default_offset() -> i64 {
    0
}
fn default_limit() -> i64 {
    20
}

/// Paginated list response wrapper.
/// Note: Generic type — manually defined in generated/index.ts rather than auto-exported.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedResponse<T: Serialize + ToSchema> {
    pub data: Vec<T>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}
