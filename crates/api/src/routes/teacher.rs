//! Teacher picker endpoints for distillation setup.
//!
//! `catalog` lists the curated open teacher models; `classify` returns the
//! provider policy for a chosen endpoint + model so the UI can render the
//! policy badge before anything is launched.

use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppResult;
use crate::services::teacher::policy::{
    ProviderPolicy, TeacherCatalogEntry, classify_provider, teacher_catalog,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/teachers/catalog", get(get_teacher_catalog))
        .route("/teachers/classify", post(classify_teacher))
}

/// Request to classify a teacher choice. Deliberately excludes the API key —
/// classification needs only the endpoint and model.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ClassifyTeacherRequest {
    pub api_base_url: String,
    pub model: String,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ClassifyTeacherResponse {
    pub policy: ProviderPolicy,
}

/// List the curated open teacher models (permissive licenses).
#[utoipa::path(
    get,
    path = "/api/v1/teachers/catalog",
    tag = "Training",
    responses((status = 200, description = "Curated teacher models", body = [TeacherCatalogEntry])),
    security(("jwt" = []))
)]
pub async fn get_teacher_catalog(
    _user: AuthenticatedUser,
) -> AppResult<Json<Vec<TeacherCatalogEntry>>> {
    Ok(Json(teacher_catalog().to_vec()))
}

/// Classify a teacher endpoint + model into a provider policy.
#[utoipa::path(
    post,
    path = "/api/v1/teachers/classify",
    tag = "Training",
    request_body = ClassifyTeacherRequest,
    responses((status = 200, description = "Provider policy", body = ClassifyTeacherResponse)),
    security(("jwt" = []))
)]
pub async fn classify_teacher(
    _user: AuthenticatedUser,
    Json(body): Json<ClassifyTeacherRequest>,
) -> AppResult<Json<ClassifyTeacherResponse>> {
    Ok(Json(ClassifyTeacherResponse {
        policy: classify_provider(&body.api_base_url, &body.model),
    }))
}
