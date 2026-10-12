//! Host capture of global extensions into the existing immutable Session plan.
use std::{collections::BTreeSet, sync::Arc};
use nomifun_agent_control_plane::{ControlPlaneError, ResolvedSessionCapabilities, SessionCapabilitiesResolver};
use nomifun_agent_contracts::UserId;
use nomifun_api_types::SessionCapabilitySelectionDto;

pub(crate) struct GlobalSessionCapabilities {
    pub owner: UserId,
    pub skills: Arc<nomifun_skill_library::SkillPaths>,
    pub mcp: Arc<dyn nomifun_db::IMcpServerRepository>,
}

impl GlobalSessionCapabilities {
    /// Agent changes carry forward only extensions that the installation can
    /// still provide. This never reuses frozen MCP authority or mutates the
    /// source Snapshot; the retained identities are compiled into the target.
    pub(super) async fn retain_available_selection(
        &self,
        owner: &UserId,
        selection: &SessionCapabilitySelectionDto,
    ) -> Result<SessionCapabilitySelectionDto, ControlPlaneError> {
        let mut skill_names = canonical_names(&selection.skill_names).map_err(selection_failure)?;
        let mut mcp_server_ids = canonical_names(&selection.mcp_server_ids).map_err(selection_failure)?;
        let (inventory, ready) = self.inventory(owner).await?;
        let available = inventory.skills.iter().map(|skill| skill.name.as_str()).collect::<BTreeSet<_>>();
        skill_names.retain(|name| available.contains(name.as_str()));
        mcp_server_ids.retain(|id| ready.contains(id));
        Ok(SessionCapabilitySelectionDto {
            skill_names: skill_names.into_iter().collect(),
            mcp_server_ids: mcp_server_ids.into_iter().collect(),
        })
    }

    async fn inventory(
        &self,
        owner: &UserId,
    ) -> Result<(nomifun_skill_library::frozen::SkillInventory, BTreeSet<String>), ControlPlaneError> {
        if owner != &self.owner {
            return Err(ControlPlaneError::canonical("RESOURCE_OWNER_MISMATCH", axum::http::StatusCode::FORBIDDEN, "global extensions require the installation owner"));
        }
        let inventory = nomifun_skill_library::frozen::capture_inventory(&self.skills).await.map_err(|error| selection_failure(error.to_string()))?;
        if !inventory.unavailable.is_empty() {
            tracing::warn!(unavailable_skill_count = inventory.unavailable.len(), "unavailable Skill packages isolated from the Session inventory");
        }
        let rows = self.mcp.list().await.map_err(|error| selection_failure(error.to_string()))?;
        let ready = rows.iter().filter(|row| row.enabled && row.deleted_at.is_none() && row.last_test_status == "connected"
            && row.tools.as_ref().is_some_and(|tools| serde_json::from_str::<Vec<serde_json::Value>>(tools).is_ok_and(|tools| !tools.is_empty())))
            .map(|row| row.mcp_server_id.clone()).collect();
        Ok((inventory, ready))
    }
}

#[async_trait::async_trait]
impl SessionCapabilitiesResolver for GlobalSessionCapabilities {
    async fn resolve(&self, owner: &UserId, selection: Option<&SessionCapabilitySelectionDto>) -> Result<ResolvedSessionCapabilities, ControlPlaneError> {
        let failure = selection_failure;
        let (inventory, ready) = self.inventory(owner).await?;
        let skills = inventory.skills;
        let available = skills.iter().map(|skill| skill.name.as_str()).collect::<BTreeSet<_>>();
        let selected_skill_names = match selection {
            Some(selection) => canonical_names(&selection.skill_names).map_err(failure)?,
            None => nomifun_skill_library::frozen::default_skill_names(&self.skills).await.map_err(|error| failure(error.to_string()))?.into_iter().filter(|name| available.contains(name.as_str())).collect(),
        };
        if let Some(unavailable) = inventory.unavailable.iter().find(|skill| selected_skill_names.contains(&skill.name)) {
            return Err(failure(format!("Skill {} is unavailable for this Session: {}", unavailable.name, unavailable.reason)));
        }
        if selected_skill_names.iter().any(|name| !available.contains(name.as_str())) {
            return Err(failure("a selected Skill is no longer installed; refresh the global library".into()));
        }
        let mcp_server_ids = match selection {
            Some(selection) => canonical_names(&selection.mcp_server_ids).map_err(failure)?,
            None => ready.clone(),
        };
        if !mcp_server_ids.is_subset(&ready) {
            return Err(failure("a selected MCP server is disabled or has no tested tools; test its connection or deselect it".into()));
        }
        Ok(ResolvedSessionCapabilities { skills, selected_skill_names, mcp_server_ids })
    }
}

fn selection_failure(message: String) -> ControlPlaneError {
    ControlPlaneError::canonical("SESSION_CAPABILITIES_INVALID", axum::http::StatusCode::UNPROCESSABLE_ENTITY, message)
}

fn canonical_names(names: &[String]) -> Result<BTreeSet<String>, String> {
    let set = names.iter().cloned().collect::<BTreeSet<_>>();
    if names.len() > 128 || set.len() != names.len() || names.iter().any(|name| name.is_empty() || name != name.trim() || name.len() > 256) {
        return Err("extension selections must contain distinct nonempty names within the Session limit".into());
    }
    Ok(set)
}
