//! Plan current default-model cleanup; immutable Session artifacts stay intact.
use super::*;
use nomifun_agent_contracts::ResolvedSnapshotEnvelope;

#[derive(Debug, Clone)]
pub struct PreparedAgentModelDetachment {
    pub expected_current: PresetRevisionRef,
    pub revision: AgentPresetRevision,
    pub snapshot: ResolvedSnapshotEnvelope,
}

impl AgentControlPlane {
    /// No writes, registry refresh or default resolver. The catalog owner
    /// commits these CAS-guarded artifacts with its deletion transaction.
    pub async fn plan_catalog_model_detachment(
        &self, owner: &UserId, provider_id: &str, model: Option<&str>,
    ) -> Result<Vec<PreparedAgentModelDetachment>, ControlPlaneError> {
        let mut result = Vec::new();
        for preset in self.store.list_presets(owner).await? {
            if preset.session_only { continue; }
            let Some(current) = self.current_revision(&preset).await? else { continue; };
            if nomifun_agent_contracts::catalog_model_detached_payload(&current.payload,provider_id,model).is_none() { continue; }
            let snapshot = self.store.get_snapshot(&current.reference).await?
                .ok_or_else(|| ControlPlaneError::Wire("current Agent configuration has no snapshot".into()))?;
            if let Some((revision,snapshot)) = self.compiler.prepare_catalog_model_detachment(
                &current,&snapshot,provider_id,model,
            )? {
                result.push(PreparedAgentModelDetachment {expected_current:current.reference,revision,snapshot});
            }
        }
        Ok(result)
    }
}
