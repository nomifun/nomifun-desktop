//! 创意工坊 (Creative Workshop) capabilities (registry form). M0 exposes one
//! read-only tool — `nomi_workshop_list_canvases` — so the companion / canvas
//! assistant can see what canvases exist. M9 extends this domain (read canvas
//! state / apply node ops / trigger generation).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::GatewayDeps;
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::ok;

#[derive(Deserialize, JsonSchema)]
struct ListCanvasesParams {
    /// Optional case-insensitive substring filter over canvas titles.
    #[serde(default)]
    query: Option<String>,
}

async fn list_canvases(deps: Arc<GatewayDeps>, p: ListCanvasesParams) -> Value {
    let query = p.query.map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty());
    match deps.workshop_repo.list_canvases().await {
        Ok(rows) => {
            let canvases: Vec<Value> = rows
                .into_iter()
                .filter(|c| query.as_deref().is_none_or(|q| c.title.to_lowercase().contains(q)))
                .map(|c| {
                    json!({
                        "id": c.id,
                        "title": c.title,
                        "node_count": c.node_count,
                    })
                })
                .collect();
            ok(json!({ "total": canvases.len(), "canvases": canvases }))
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<ListCanvasesParams, _, _>(
        CapabilityMeta::new(
            "nomi_workshop_list_canvases",
            "workshop",
            "List 创意工坊 canvases (id / title / node_count), optionally filtered by a title substring.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| list_canvases(deps, p),
    ));
}
