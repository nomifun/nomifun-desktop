//! Product middleware uses the original Service invoker and frozen identity.
use super::*;
use nomi_agent::model_middleware::{BeforeModelInput, ModelRequestMiddleware, ModelRequestPatch};
pub use nomifun_agent_contracts::model_middleware::{ACTION_ID, action, schemas};

#[derive(Clone)]
pub(super) struct Binding {
    pub resolved: ResolvedCapability,
    pub consumer: Option<Arc<dyn ModelRequestMiddleware>>,
}

impl Binding {
    pub fn consumer(&self) -> Result<Arc<dyn ModelRequestMiddleware>, NomiPluginToolError> {
        self.consumer.clone().ok_or_else(|| {
            NomiPluginToolError::Contract(
                "Selected Product before_model middleware has no execution adapter".into(),
            )
        })
    }
}

/// Nomi preview/save admission shares runtime selection's exact contract checks.
/// The generic compiler validates order and identity, not support for Nomi hooks.
pub fn validate_selection(
    content: &nomifun_agent_contracts::ResolvedSnapshotContent,
) -> Result<(), NomiPluginToolError> {
    selected(content).map(|_| ())
}

pub(super) fn selected(
    content: &nomifun_agent_contracts::ResolvedSnapshotContent,
) -> Result<Vec<Binding>, NomiPluginToolError> {
    let mut bindings = Vec::new();
    for resolved in content.contributions() {
        if !resolved
            .actions
            .iter()
            .any(|a| a.action_id.as_ref() == ACTION_ID)
        {
            continue;
        }
        resolved
            .validate()
            .map_err(|e| NomiPluginToolError::Contract(e.message))?;
        if resolved.contribution_lock.source_kind
            != ContributionSourceKind::PluginProductActiveRelease
            || resolved.actions != [action()]
            || !resolved.required_resource_kinds.is_empty()
            || (!resolved.action_allowlist.is_empty()
                && !resolved.action_allowlist.contains(&ACTION_ID.into()))
        {
            return Err(NomiPluginToolError::Contract(
                "before_model requires the exact Product action and authority without resources"
                    .into(),
            ));
        }
        bindings.push(Binding {
            resolved: resolved.clone(),
            consumer: None,
        });
    }
    let mut positions = BTreeMap::new();
    for (index, id) in content.middleware_order.iter().enumerate() {
        if positions.insert(id, index).is_some()
            || !bindings
                .iter()
                .any(|binding| &binding.resolved.capability.id == id)
        {
            return Err(NomiPluginToolError::Contract(format!(
                "middleware_order has a duplicate or unsupported request middleware {}",
                id.as_ref()
            )));
        }
    }
    bindings.sort_by_key(|binding| {
        (
            positions
                .get(&binding.resolved.capability.id)
                .copied()
                .unwrap_or(usize::MAX),
            binding.resolved.capability.id.clone(),
        )
    });
    Ok(bindings)
}

pub(super) fn bind(
    bindings: &mut [Binding],
    actions: &[&NomiPluginProductToolAction],
    snapshot: &ResolvedSnapshotRef,
    invoker: Arc<dyn NomiPluginProductToolInvoker>,
) -> Result<(), NomiPluginToolError> {
    if bindings.len() != actions.len() {
        return Err(NomiPluginToolError::Contract(
            "Missing or unexpected Product before_model action adapter".into(),
        ));
    }
    for binding in bindings {
        let matching = actions
            .iter()
            .filter(|a| {
                a.identity.resolved_capability == binding.resolved
                    && &a.identity.resolved_snapshot_ref == snapshot
                    && a.identity.action == action()
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(NomiPluginToolError::Contract(
                "Missing or duplicate exact Product before_model adapter".into(),
            ));
        }
        binding.consumer = Some(Arc::new(ProductMiddleware {
            action: (**matching[0]).clone(),
            invoker: invoker.clone(),
        }));
    }
    Ok(())
}

struct ProductMiddleware {
    action: NomiPluginProductToolAction,
    invoker: Arc<dyn NomiPluginProductToolInvoker>,
}

#[async_trait]
impl ModelRequestMiddleware for ProductMiddleware {
    async fn before_model(&self, input: BeforeModelInput) -> Result<ModelRequestPatch, String> {
        let key = format!("nomi-before-model:{}", uuid::Uuid::now_v7());
        let value = self
            .invoker
            .invoke(NomiPluginProductToolInvocation {
                identity: self.action.identity.clone(),
                operation_id: key.clone().into(),
                idempotency_key: key.clone().into(),
                correlation_id: key.into(),
                input: StrictJsonValue(serde_json::to_value(input).map_err(|e| e.to_string())?),
            })
            .await
            .map_err(|_| "Product before_model invocation failed".to_owned())?;
        if canonical_json_bytes(&value)
            .map_err(|e| e.to_string())?
            .len()
            > nomi_agent::model_middleware::MAX_PATCH_BYTES
        {
            return Err("before_model patch exceeds 64 KiB".into());
        }
        serde_json::from_value(value.0).map_err(|_| "Invalid before_model patch".into())
    }

    fn label(&self) -> &str {
        self.action.capability_id().as_ref()
    }
}
