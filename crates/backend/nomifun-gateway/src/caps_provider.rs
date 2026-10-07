//! Provider-domain capability (registry form): the read-only model-provider
//! catalog. The shared nomi model-resolution chain stays in `provider_support`
//! (used by the cron + conversation capabilities), this only exposes listing.

use std::future::Future;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::CompatibilityCapabilityHost;
use crate::provider_support::ProviderSupportDeps;
use crate::registry::{Capability, CapabilityMeta, EffectClass};
use crate::server::ok;
use crate::provider_support::load_provider_summaries;

/// Cap on models listed per provider (keeps the tool result inside the calling
/// agent's context budget; the full list lives in desktop Settings).
const MAX_MODELS_PER_PROVIDER: usize = 20;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListProvidersParams {
    /// Include disabled providers too (default false: enabled only).
    #[serde(default)]
    include_disabled: Option<bool>,
}

#[derive(Clone)]
struct ProviderCapabilityDeps {
    support: ProviderSupportDeps,
}

fn adapt<P, F, Fut>(
    handler: F,
) -> impl Fn(Arc<CompatibilityCapabilityHost>, crate::deps::CallerCtx, P) -> Fut + Send + Sync + 'static
where
    P: Send + 'static,
    F: Fn(Arc<ProviderCapabilityDeps>, P) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    move |deps, _ctx, params| {
        handler(Arc::new(ProviderCapabilityDeps {
            support: ProviderSupportDeps {
                provider_repo: deps.provider_repo.clone(),
                provider_model_repo: deps.provider_model_repo.clone(),
                provider_model_capability_repo: deps.provider_model_capability_repo.clone(),
                companion_service: deps.companion_service.clone(),
            },
        }), params)
    }
}

async fn list(deps: Arc<ProviderCapabilityDeps>, p: ListProvidersParams) -> Value {
    let include_disabled = p.include_disabled.unwrap_or(false);
    let summaries = match load_provider_summaries(&deps.support).await {
        Ok(s) => s,
        Err(e) => return e,
    };
    let items: Vec<Value> = summaries
        .iter()
        .filter(|p| include_disabled || p.enabled)
        .map(|p| {
            json!({
                "provider_id": p.provider_id,
                "name": p.name,
                "platform": p.platform,
                "enabled": p.enabled,
                "models": p.models.iter().take(MAX_MODELS_PER_PROVIDER).map(|model| json!({
                    "model": model.model,
                    "capabilities": model.capabilities,
                })).collect::<Vec<_>>(),
                "model_count": p.models.len(),
                "models_truncated": p.models.len() > MAX_MODELS_PER_PROVIDER,
            })
        })
        .collect();
    if items.is_empty() {
        return ok(json!({
            "providers": [],
            "note": "no model provider is configured/enabled on this desktop yet — add one in Settings → Providers (or via nomi_create_provider) before creating nomi conversations or cron jobs"
        }));
    }
    ok(json!({ "providers": items }))
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<ListProvidersParams, _, _>(
        CapabilityMeta::new(
            "nomi_list_providers",
            "provider",
            "Read-only catalog of configured model providers and their enabled models (no API keys), for guiding a model choice before creating sessions / cron jobs.",
            EffectClass::Read,
        ),
        adapt(list),
    ));
}
