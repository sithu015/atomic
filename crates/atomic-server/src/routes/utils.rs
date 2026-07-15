//! Utility routes

use crate::db_extractor::Db;
use actix_web::{web, HttpResponse};
use serde::Deserialize;
use utoipa::ToSchema;

/// Optional body for `POST /api/utils/compact-tags`. With `merges`, the
/// listed merges are applied directly through the same guarded path the
/// LLM's suggestions use (hierarchy checks, atom retagging, canvas-cache
/// invalidation) — the operator's escape hatch for pairs the
/// deliberately-conservative LLM pass declines, like an auto-tagger's
/// broader/narrower double-stamps. Without a body (the pre-existing
/// call shape) the LLM suggestion flow runs unchanged.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CompactTagsRequest {
    #[serde(default)]
    pub merges: Vec<TagMergeRequest>,
}

/// One explicit merge. Mirrors `atomic_core::compaction::TagMerge` (the
/// core type carries no OpenAPI derives; the transport owns its schema).
#[derive(Debug, Deserialize, ToSchema)]
pub struct TagMergeRequest {
    pub winner_name: String,
    pub loser_name: String,
    #[serde(default)]
    pub reason: String,
}

impl From<TagMergeRequest> for atomic_core::compaction::TagMerge {
    fn from(r: TagMergeRequest) -> Self {
        Self {
            winner_name: r.winner_name,
            loser_name: r.loser_name,
            reason: r.reason,
        }
    }
}

#[utoipa::path(get, path = "/api/utils/sqlite-vec", responses((status = 200, description = "sqlite-vec version")), tag = "utils")]
pub async fn check_sqlite_vec(db: Db) -> HttpResponse {
    match db.0.check_sqlite_vec().await {
        Ok(version) => HttpResponse::Ok().json(serde_json::json!({"version": version})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": format!("sqlite-vec not loaded: {}", e)})),
    }
}

#[utoipa::path(post, path = "/api/utils/compact-tags", responses((status = 200, description = "Tag compaction results")), tag = "utils")]
pub async fn compact_tags(db: Db, body: Option<web::Json<CompactTagsRequest>>) -> HttpResponse {
    // All orchestration (provider/model resolution via the settings_for_ai
    // overlay, capabilities, merge application) lives in the core facade so the
    // explicit-provider-config path is honored — a raw get_settings() read here
    // would bypass it and let a cloud tenant's settings drive the provider.
    let explicit: Option<Vec<atomic_core::compaction::TagMerge>> = body
        .map(|b| b.into_inner().merges.into_iter().map(Into::into).collect())
        .filter(|merges: &Vec<_>| !merges.is_empty());
    let outcome = match explicit {
        Some(merges) => db.0.apply_tag_merges(&merges).await,
        None => db.0.compact_tags().await,
    };
    match outcome {
        Ok(result) => HttpResponse::Ok().json(serde_json::json!({
            "tags_merged": result.tags_merged,
            "atoms_retagged": result.atoms_retagged
        })),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": e.to_string()})),
    }
}
