//! Tool-stage Product consumers share the existing frozen Service invoker.
use super::*;
use crate::runtime_tool_middleware_contract::{BeforeToolInput, BeforeToolDecision, ToolCallMiddleware};
pub use nomifun_agent_contracts::tool_middleware::{BEFORE_ACTION_ID, before_action, schemas};

#[derive(Clone)]
pub(super) struct Binding {
    pub resolved: ResolvedCapability,
    consumer: Option<Arc<dyn ToolCallMiddleware>>,
}

impl Binding {
    pub fn consumer(&self) -> Result<Arc<dyn ToolCallMiddleware>, NomiPluginToolError> {
        self.consumer.clone().ok_or_else(|| NomiPluginToolError::Contract(
            "Selected Product before_tool check has no execution adapter".into()))
    }
}

pub fn validate_selection(content: &nomifun_agent_contracts::ResolvedSnapshotContent) -> Result<(), NomiPluginToolError> {
    selected(content).map(|_| ())
}

pub(super) fn selected(content: &nomifun_agent_contracts::ResolvedSnapshotContent) -> Result<Vec<Binding>, NomiPluginToolError> {
    let mut bindings = Vec::new();
    for resolved in content.contributions() {
        for action in &resolved.actions {
            let id = action.action_id.as_ref();
            if (id.starts_with("agent.before_") || id.starts_with("agent.after_"))
                && id != BEFORE_ACTION_ID && id != model_middleware::ACTION_ID {
                return Err(NomiPluginToolError::Contract("Selected Agent hook phase is unsupported".into()));
            }
        }
        if !resolved.actions.iter().any(|a| a.action_id.as_ref() == BEFORE_ACTION_ID) { continue; }
        resolved.validate().map_err(|e| NomiPluginToolError::Contract(e.message))?;
        if resolved.contribution_lock.source_kind != ContributionSourceKind::PluginProductActiveRelease
            || resolved.actions != [before_action()]
            || !resolved.required_resource_kinds.is_empty()
            || (!resolved.action_allowlist.is_empty() && !resolved.action_allowlist.contains(&BEFORE_ACTION_ID.into())) {
            return Err(NomiPluginToolError::Contract("before_tool requires the exact Product action and frozen authority without resources".into()));
        }
        bindings.push(Binding { resolved: resolved.clone(), consumer: None });
    }
    let positions = model_middleware::middleware_positions(content)?;
    bindings.sort_by_key(|b| (positions.get(&b.resolved.capability.id).copied().unwrap_or(usize::MAX), b.resolved.capability.id.clone()));
    Ok(bindings)
}

pub(super) fn bind(bindings: &mut [Binding], actions: &[&NomiPluginProductToolAction], snapshot: &ResolvedSnapshotRef,
    invoker: Arc<dyn NomiPluginProductToolInvoker>) -> Result<(), NomiPluginToolError> {
    if bindings.len() != actions.len() { return Err(NomiPluginToolError::Contract("Missing or unexpected Product before_tool adapter".into())); }
    for binding in bindings {
        let matching = actions.iter().filter(|a| a.identity.resolved_capability == binding.resolved
            && &a.identity.resolved_snapshot_ref == snapshot && a.identity.action == before_action()).collect::<Vec<_>>();
        if matching.len() != 1 { return Err(NomiPluginToolError::Contract("Missing or duplicate exact Product before_tool adapter".into())); }
        binding.consumer = Some(Arc::new(ProductToolCheck { action: (**matching[0]).clone(), invoker: invoker.clone() }));
    }
    Ok(())
}

struct ProductToolCheck {
    action: NomiPluginProductToolAction,
    invoker: Arc<dyn NomiPluginProductToolInvoker>,
}

#[async_trait]
impl ToolCallMiddleware for ProductToolCheck {
    async fn before_tool(&self, input: BeforeToolInput) -> Result<BeforeToolDecision, String> {
        let key = format!("nomi-before-tool:{}", uuid::Uuid::now_v7());
        let value = self.invoker.invoke(NomiPluginProductToolInvocation {
                cancellation: Default::default(),
            identity: self.action.identity.clone(), operation_id: key.clone().into(),
            idempotency_key: key.clone().into(), correlation_id: key.into(),
            input: StrictJsonValue(serde_json::to_value(input).map_err(|_| "Invalid tool check input")?),
        }).await.map_err(|_| "Product before_tool invocation failed".to_owned())?;
        let bytes = canonical_json_bytes(&value).map_err(|_| "Invalid tool check output".to_owned())?;
        crate::runtime_tool_middleware_contract::decode_decision(&bytes)
    }
    fn label(&self) -> &str { self.action.capability_id().as_ref() }
}
