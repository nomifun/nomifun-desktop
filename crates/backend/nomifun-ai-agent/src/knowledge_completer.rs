//! Production [`KnowledgeCompleter`]: resolves a default provider/model and
//! runs a one-shot completion. Same layering as the companion learner's
//! `LiveCompanionCompleter` and the IDMM sidecar's `LiveCompleter` — the knowledge
//! crate holds only the trait, this crate provides the provider-backed
//! implementation, and the app layer wires it via
//! `KnowledgeService::set_completer`.
//!
//! Unlike companion/IDMM there is no per-feature model setting (yet): knowledge
//! autogen is a background curation task, so the default is the first enabled
//! provider/model pair with an explicit Chat capability.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use nomifun_common::AppError;
use nomifun_db::{
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    ProviderModelRow,
};
use nomifun_knowledge::KnowledgeCompleter;
use nomifun_model_invoke::ModelInvokeService;

use crate::factory::provider_config::{one_shot_completion, resolve_provider_config, user_message};

/// READMEs can be sizeable; keep enough room that the strict-JSON overview
/// reply (description + full readme_markdown) never gets cut mid-object —
/// a truncated reply is guaranteed-unparseable. The prompt side also bounds
/// the README length (see `autogen::OVERVIEW_SYSTEM`).
const KNOWLEDGE_MAX_TOKENS: u32 = 8192;

/// Provider-backed completer for knowledge autogen / snapshot compression.
pub struct LiveKnowledgeCompleter {
    pub provider_repo: Arc<dyn IProviderRepository>,
    pub provider_model_repo: Arc<dyn IProviderModelRepository>,
    pub model_invoke: Arc<ModelInvokeService>,
    pub workspace: PathBuf,
}

impl LiveKnowledgeCompleter {
    /// First enabled provider/model pair with an exact Chat capability.
    async fn resolve_default_model(&self) -> Result<(String, String), AppError> {
        resolve_default_model(
            &self.provider_repo,
            &self.provider_model_repo,
            self.model_invoke.provider_model_capability_repo(),
        )
        .await
        .ok_or_else(|| {
            AppError::Conflict(
                "knowledge autogen unavailable: no enabled Chat-capable provider/model is configured"
                    .into(),
            )
        })
    }

    /// Resolve the given `(provider_id, model)` into a provider config and run
    /// the one-shot completion. Shared by [`KnowledgeCompleter::complete`]
    /// (which feeds it the default pick) and
    /// [`KnowledgeCompleter::complete_with`] (which feeds it the caller's
    /// explicit pick), so the resolve→complete tail is identical regardless
    /// of how the model was chosen.
    async fn complete_for_model(
        &self,
        system: &str,
        user: &str,
        provider_id: &str,
        model: &str,
    ) -> Result<String, AppError> {
        let cfg = resolve_provider_config(
            self.model_invoke.as_ref(),
            provider_id,
            model,
            &self.workspace,
        )
        .await?;
        one_shot_completion(&cfg, system, vec![user_message(user)], KNOWLEDGE_MAX_TOKENS).await
    }
}

#[async_trait::async_trait]
impl KnowledgeCompleter for LiveKnowledgeCompleter {
    async fn complete(&self, system: &str, user: &str) -> Result<String, AppError> {
        let (provider_id, model) = self.resolve_default_model().await?;
        self.complete_for_model(system, user, &provider_id, &model).await
    }

    /// Honor the caller's explicit `(provider_id, model)`, skipping the
    /// default-model resolution entirely — the knowledge UI uses this to let
    /// the user pick which model generates/regenerates a base.
    async fn complete_with(
        &self,
        system: &str,
        user: &str,
        provider_id: &str,
        model: &str,
    ) -> Result<String, AppError> {
        self.complete_for_model(system, user, provider_id, model).await
    }
}

/// First enabled provider-model row in repository `sort_order` whose exact
/// `(provider_id, model)` key has a persisted Chat capability.
pub(crate) fn first_enabled_model<'a, 'c, I>(
    rows: I,
    chat_capabilities: &HashSet<(&'c str, &'c str)>,
) -> Option<String>
where
    I: IntoIterator<Item = &'a ProviderModelRow>,
{
    rows.into_iter()
        .filter(|row| row.enabled)
        .find_map(|row| {
            let model = row.model.trim();
            (!model.is_empty()
                && chat_capabilities.contains(&(row.provider_id.as_str(), model)))
            .then(|| model.to_owned())
        })
}

/// Resolve the app's DEFAULT `(provider_id, model)`: the first enabled provider
/// (creation order) and its first enabled Chat-capable model (row `sort_order`
/// order). `None` when no enabled Chat-capable pair is configured. The shared "what model
/// would the app use by default" resolution — reused wherever a caller has no
/// explicit model.
pub async fn resolve_default_model(
    provider_repo: &std::sync::Arc<dyn IProviderRepository>,
    provider_model_repo: &std::sync::Arc<dyn IProviderModelRepository>,
    provider_model_capability_repo: &std::sync::Arc<dyn IProviderModelCapabilityRepository>,
) -> Option<(String, String)> {
    let providers = provider_repo.list().await.ok()?;
    // Rows come back ordered by (provider_id, sort_order, model), so each
    // provider's group preserves its catalog order.
    let rows = provider_model_repo.list().await.ok()?;
    let mut grouped: HashMap<&str, Vec<&ProviderModelRow>> = HashMap::new();
    for row in &rows {
        grouped.entry(row.provider_id.as_str()).or_default().push(row);
    }
    let capabilities = provider_model_capability_repo.list().await.ok()?;
    let chat_capabilities: HashSet<(&str, &str)> = capabilities
        .iter()
        .filter(|capability| capability.task == "chat")
        .map(|capability| (capability.provider_id.as_str(), capability.model.as_str()))
        .collect();
    providers.iter().filter(|p| p.enabled).find_map(|p| {
        let provider_rows = grouped.get(p.provider_id.as_str())?;
        first_enabled_model(provider_rows.iter().copied(), &chat_capabilities)
            .map(|model| (p.provider_id.clone(), model))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_row(model: &str, enabled: bool, sort_order: i64) -> ProviderModelRow {
        ProviderModelRow {
            id: 0,
            provider_id: "provider".into(),
            model: model.into(),
            display_name: None,
            enabled,
            sort_order,
            description: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn first_enabled_model_requires_an_exact_chat_capability() {
        let rows = [
            model_row("disabled", false, 0),
            model_row("   ", true, 1),
            model_row("tts-only", true, 2),
            model_row("selected", true, 3),
        ];
        let chat_capabilities = HashSet::from([("provider", "selected")]);
        assert_eq!(
            first_enabled_model(rows.iter(), &chat_capabilities),
            Some("selected".into())
        );
    }
}
