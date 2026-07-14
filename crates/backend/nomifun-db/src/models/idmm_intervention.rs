use serde::{Deserialize, Serialize};

/// Row in `idmm_interventions` — one persisted IDMM decision (the "思路"/audit
/// trail). Aggressively evicted: per-target cap + shared TTL; cascades away on
/// session delete. `target_id` is polymorphic (conversation TEXT / terminal
/// INTEGER stored as string) so there is no FK — app-level cascade handles it.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IdmmInterventionRow {
    pub id: String,
    /// Authenticated owner resolved from the supervised session before the
    /// intervention is persisted. Activity feeds are always partitioned by it.
    pub user_id: String,
    pub target_kind: String,
    pub target_id: String,
    pub watch: String,
    pub at: i64,
    pub signal: String,
    pub tier_used: String,
    pub category: Option<String>,
    pub action: String,
    pub detail: Option<String>,
    pub reason: Option<String>,
    pub confidence: Option<f64>,
    pub bypass_model: Option<String>,
    pub outcome: String,
}
