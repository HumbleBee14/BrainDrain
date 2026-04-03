use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app_state::AppState;
use crate::error::AppResult;

/// Model catalog routes — curated list of recommended base models.
pub fn router() -> Router<AppState> {
    Router::new().route("/models/catalog", get(get_catalog))
}

/// A recommended base model with metadata for the UI.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogModel {
    /// HuggingFace model ID (e.g. "unsloth/Llama-3.1-8B-Instruct")
    pub model_id: String,
    /// Short display name (e.g. "Llama 3.1 8B")
    pub display_name: String,
    /// Model parameter count for display
    pub size: String,
    /// Approximate VRAM needed in 4-bit quantization
    pub vram_4bit_gb: f32,
    /// Approximate VRAM needed in full precision
    pub vram_full_gb: f32,
    /// What this model is best for
    pub best_for: Vec<String>,
    /// Task types this model is recommended for
    pub recommended_for: Vec<String>,
    /// Whether this model requires a HuggingFace token (gated model)
    pub gated: bool,
    /// Suggested training mode based on model characteristics
    pub suggested_mode: String,
    /// Estimated training time in hours (for 1K pairs on A10G)
    pub est_hours_1k_pairs: f32,
    /// Estimated cost in USD (for 1K pairs on A10G at $1.10/hr)
    pub est_cost_1k_pairs: f32,
}

/// Response wrapping the catalog with auto-suggestion.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogResponse {
    /// Full catalog of recommended models
    pub models: Vec<CatalogModel>,
    /// Auto-suggested model ID based on task_type (if provided)
    pub suggested: Option<String>,
    /// Auto-suggested training mode based on dataset size (if provided)
    pub suggested_mode: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CatalogQuery {
    /// Optional task type to filter/suggest models
    pub task_type: Option<String>,
    /// Optional dataset pair count to suggest training mode
    pub pair_count: Option<i64>,
}

/// GET /api/v1/models/catalog
///
/// Returns a curated catalog of recommended base models with auto-suggestions.
/// Pass `?task_type=question_answering&pair_count=500` for smart defaults.
#[utoipa::path(
    get,
    path = "/api/v1/models/catalog",
    tag = "Training",
    params(
        ("task_type" = Option<String>, Query, description = "Task type for model suggestion"),
        ("pair_count" = Option<i64>, Query, description = "Dataset pair count for mode suggestion"),
    ),
    responses(
        (status = 200, description = "Model catalog with suggestions", body = CatalogResponse),
    ),
    security(("jwt" = []))
)]
pub async fn get_catalog(
    State(_state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> AppResult<Json<CatalogResponse>> {
    let catalog = build_catalog();

    let suggested = query
        .task_type
        .as_deref()
        .and_then(|tt| suggest_model(tt, &catalog));

    let suggested_mode = query.pair_count.map(suggest_mode);

    Ok(Json(CatalogResponse {
        models: catalog,
        suggested,
        suggested_mode,
    }))
}

/// Suggest a model ID based on task type.
fn suggest_model(task_type: &str, catalog: &[CatalogModel]) -> Option<String> {
    catalog
        .iter()
        .find(|m| m.recommended_for.iter().any(|r| r == task_type))
        .map(|m| m.model_id.clone())
}

/// Suggest a training mode based on dataset size.
fn suggest_mode(pair_count: i64) -> String {
    match pair_count {
        0..=500 => "quick".to_string(),
        501..=2000 => "quick".to_string(),
        2001..=5000 => "aligned".to_string(),
        _ => "aligned".to_string(),
    }
}

/// Build the curated model catalog.
///
/// This is a static list — no DB or HuggingFace API calls.
/// Add new models by appending to this function.
fn build_catalog() -> Vec<CatalogModel> {
    vec![
        CatalogModel {
            model_id: "unsloth/Llama-3.1-8B-Instruct".into(),
            display_name: "Llama 3.1 8B".into(),
            size: "8B".into(),
            vram_4bit_gb: 6.0,
            vram_full_gb: 16.0,
            best_for: vec![
                "General purpose".into(),
                "Q&A".into(),
                "Instruction following".into(),
            ],
            recommended_for: vec!["question_answering".into(), "instruction_following".into()],
            gated: true,
            suggested_mode: "quick".into(),
            est_hours_1k_pairs: 1.5,
            est_cost_1k_pairs: 1.65,
        },
        CatalogModel {
            model_id: "unsloth/Mistral-7B-Instruct-v0.3".into(),
            display_name: "Mistral 7B v0.3".into(),
            size: "7B".into(),
            vram_4bit_gb: 6.0,
            vram_full_gb: 14.0,
            best_for: vec!["Instruction following".into(), "Fast inference".into()],
            recommended_for: vec!["instruction_following".into()],
            gated: false,
            suggested_mode: "quick".into(),
            est_hours_1k_pairs: 1.2,
            est_cost_1k_pairs: 1.32,
        },
        CatalogModel {
            model_id: "unsloth/Phi-3.5-mini-instruct".into(),
            display_name: "Phi 3.5 Mini".into(),
            size: "3.8B".into(),
            vram_4bit_gb: 4.0,
            vram_full_gb: 8.0,
            best_for: vec!["Lightweight".into(), "Low latency".into(), "Edge deployment".into()],
            recommended_for: vec!["custom".into()],
            gated: false,
            suggested_mode: "quick".into(),
            est_hours_1k_pairs: 0.8,
            est_cost_1k_pairs: 0.88,
        },
        CatalogModel {
            model_id: "unsloth/gemma-2-9b-it".into(),
            display_name: "Gemma 2 9B".into(),
            size: "9B".into(),
            vram_4bit_gb: 8.0,
            vram_full_gb: 20.0,
            best_for: vec!["Multilingual".into(), "Strong reasoning".into()],
            recommended_for: vec!["reasoning".into()],
            gated: true,
            suggested_mode: "aligned".into(),
            est_hours_1k_pairs: 2.0,
            est_cost_1k_pairs: 2.20,
        },
        CatalogModel {
            model_id: "unsloth/Qwen2.5-7B-Instruct".into(),
            display_name: "Qwen 2.5 7B".into(),
            size: "7B".into(),
            vram_4bit_gb: 6.0,
            vram_full_gb: 16.0,
            best_for: vec!["Code".into(), "Reasoning".into(), "Math".into()],
            recommended_for: vec!["reasoning".into()],
            gated: false,
            suggested_mode: "reasoning".into(),
            est_hours_1k_pairs: 1.5,
            est_cost_1k_pairs: 1.65,
        },
        CatalogModel {
            model_id: "unsloth/Llama-3.2-1B-Instruct".into(),
            display_name: "Llama 3.2 1B".into(),
            size: "1B".into(),
            vram_4bit_gb: 2.0,
            vram_full_gb: 4.0,
            best_for: vec![
                "Ultra-lightweight".into(),
                "Mobile".into(),
                "Testing".into(),
            ],
            recommended_for: vec!["custom".into()],
            gated: true,
            suggested_mode: "quick".into(),
            est_hours_1k_pairs: 0.3,
            est_cost_1k_pairs: 0.33,
        },
        CatalogModel {
            model_id: "unsloth/Qwen2.5-Coder-7B-Instruct".into(),
            display_name: "Qwen 2.5 Coder 7B".into(),
            size: "7B".into(),
            vram_4bit_gb: 6.0,
            vram_full_gb: 16.0,
            best_for: vec![
                "Code generation".into(),
                "Code completion".into(),
                "Debugging".into(),
            ],
            recommended_for: vec!["instruction_following".into()],
            gated: false,
            suggested_mode: "quick".into(),
            est_hours_1k_pairs: 1.5,
            est_cost_1k_pairs: 1.65,
        },
    ]
}
