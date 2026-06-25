use super::*;
use nomifun_api_types::BehaviorPolicy;
use nomifun_common::AgentType;
use nomifun_common::constants::{TEAM_CAPABLE_BACKENDS, has_mcp_capability};

/// Known ACP vendor labels. Kept in lockstep with the `agent_metadata`
/// seed in `005_agent_metadata.sql` — a caller hitting an unknown
/// vendor should trigger a schema drift discussion, not silently fall
/// through.
const ACP_VENDOR_LABELS: &[&str] = &[
    "claude",
    "codex",
    "gemini",
    "qwen",
    "codebuddy",
    "droid",
    "goose",
    "auggie",
    "kimi",
    "opencode",
    "copilot",
    "qoder",
    "vibe",
    "cursor",
    "kiro",
    "hermes",
    "snow",
];

pub(super) fn parse_agent_type(backend: &str) -> Result<AgentType, TeamError> {
    // Any registered ACP vendor label collapses to `AgentType::Acp`.
    if ACP_VENDOR_LABELS.contains(&backend) {
        return Ok(AgentType::Acp);
    }
    // Otherwise interpret as a top-level `AgentType` (e.g. "acp",
    // "nanobot", "nomi", "remote", "openclaw-gateway").
    let quoted = format!("\"{backend}\"");
    if let Ok(agent_type) = serde_json::from_str::<AgentType>(&quoted) {
        return Ok(agent_type);
    }
    Err(TeamError::InvalidRequest(format!("unsupported backend: {backend}")))
}

/// Resolve the most permissive session mode for a given backend string.
/// Reuses `AgentType::full_auto_mode_id` from nomifun-common.
pub(crate) fn resolve_full_auto_mode(backend: &str) -> &'static str {
    let agent_type = if ACP_VENDOR_LABELS.contains(&backend) {
        AgentType::Acp
    } else {
        let quoted = format!("\"{backend}\"");
        serde_json::from_str::<AgentType>(&quoted).unwrap_or(AgentType::Acp)
    };
    agent_type.full_auto_mode_id(Some(backend))
}

impl TeamSessionService {
    /// Check if a backend is allowed to participate in team mode.
    /// Hard whitelist passes immediately; then checks behavior_policy.supports_team;
    /// finally queries persisted `agent_capabilities` for MCP transport declarations.
    pub(crate) async fn is_backend_team_capable(&self, backend: &str) -> bool {
        if TEAM_CAPABLE_BACKENDS.contains(&backend) {
            return true;
        }
        let Ok(Some(row)) = self.agent_metadata_repo.find_builtin_by_backend(backend).await else {
            return false;
        };
        let bp_supports = row
            .behavior_policy
            .as_deref()
            .and_then(|s| serde_json::from_str::<BehaviorPolicy>(s).ok())
            .is_some_and(|bp| bp.supports_team);
        if bp_supports {
            return true;
        }
        let caps = row
            .agent_capabilities
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        has_mcp_capability(caps.as_ref())
    }

    /// Return all backends currently team-capable (hard whitelist + behavior_policy + dynamically detected).
    /// Used to build the Lead prompt's `available_agent_types` list.
    pub(crate) async fn list_team_capable_backends(&self) -> Vec<(String, String)> {
        let Ok(rows) = self.agent_metadata_repo.list_all().await else {
            return TEAM_CAPABLE_BACKENDS
                .iter()
                .map(|b| (b.to_string(), capitalize(b)))
                .collect();
        };
        let mut result: Vec<(String, String)> = Vec::new();
        for row in &rows {
            if !row.enabled {
                continue;
            }
            // Use backend if present, otherwise agent_type as identifier
            let key = match row.backend.as_deref() {
                Some(b) => b.to_string(),
                None => row.agent_type.clone(),
            };

            // Check behavior_policy.supports_team (covers agents with backend=NULL like nomi)
            let bp_supports = row
                .behavior_policy
                .as_deref()
                .and_then(|s| serde_json::from_str::<BehaviorPolicy>(s).ok())
                .is_some_and(|bp| bp.supports_team);
            if bp_supports {
                result.push((key, row.name.clone()));
                continue;
            }

            // Hard whitelist (only works when backend is present)
            if let Some(backend) = row.backend.as_deref()
                && TEAM_CAPABLE_BACKENDS.contains(&backend)
            {
                result.push((key, row.name.clone()));
                continue;
            }

            // Dynamic MCP detection
            let caps = row
                .agent_capabilities
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
            if has_mcp_capability(caps.as_ref()) {
                result.push((key, row.name.clone()));
            }
        }
        // Ensure hard whitelist entries are present even if not in DB
        for &b in TEAM_CAPABLE_BACKENDS {
            if !result.iter().any(|(bk, _)| bk == b) {
                result.push((b.to_string(), capitalize(b)));
            }
        }
        result
    }

    /// Return the `team_list_models` response built from DB rows.
    /// Falls back to the hardcoded response if the DB query fails.
    /// For internal agents (like nomi with backend=NULL), enriches
    /// with models from the providers table.
    pub(crate) async fn list_models_from_db(&self, agent_type_filter: Option<&str>) -> serde_json::Value {
        let Ok(rows) = self.agent_metadata_repo.list_all().await else {
            return crate::mcp::tools::handle_team_list_models(&serde_json::Value::Null);
        };
        let provider_models = self.collect_provider_models().await;
        crate::mcp::tools::build_list_models_from_rows(&rows, agent_type_filter, &provider_models)
    }

    /// Collect all enabled provider model IDs grouped by provider name.
    /// Returns a flat list of model IDs for use by internal agents (nomi).
    async fn collect_provider_models(&self) -> Vec<String> {
        let Ok(providers) = self.provider_repo.list().await else {
            return vec![];
        };
        providers
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| serde_json::from_str::<Vec<String>>(&p.models).unwrap_or_default())
            .collect()
    }

    /// Find the provider ID that contains a given model name.
    /// Iterates all enabled providers and checks their models JSON array.
    pub(crate) async fn resolve_provider_for_model(&self, model: &str) -> Option<String> {
        let providers = self.provider_repo.list().await.ok()?;
        for p in providers {
            if !p.enabled {
                continue;
            }
            let models: Vec<String> = serde_json::from_str(&p.models).unwrap_or_default();
            if models.iter().any(|m| m == model) {
                return Some(p.id);
            }
        }
        None
    }

    pub(crate) async fn default_model_for_backend(&self, backend: &str) -> Option<String> {
        let row = self.agent_metadata_repo.find_builtin_by_backend(backend).await.ok()??;
        let json: serde_json::Value = serde_json::from_str(row.available_models.as_deref()?).ok()?;
        if let Some(id) = json.get("current_model_id").and_then(|v| v.as_str())
            && !id.is_empty()
        {
            return Some(id.to_owned());
        }
        let arr = json
            .get("available_models")
            .and_then(|v| v.as_array())
            .or_else(|| json.as_array())?;
        arr.first()
            .and_then(|e| e.get("id").and_then(|v| v.as_str()))
            .map(|s| s.to_owned())
    }

    pub async fn spawn_agent_in_session(
        &self,
        team_id: &str,
        caller_slot_id: &str,
        req: crate::session::SpawnAgentRequest,
    ) -> Result<TeamAgent, TeamError> {
        let entry = self
            .sessions
            .get(team_id)
            .ok_or_else(|| TeamError::SessionNotFound(team_id.into()))?;
        entry.session.spawn_agent(caller_slot_id, req).await
    }

    pub fn dispose_all(&self) {
        let keys: Vec<String> = self.sessions.iter().map(|entry| entry.key().clone()).collect();
        for key in keys {
            self.stop_session(&key);
        }
        info!("All team sessions disposed");
    }

    pub(crate) fn conversation_service_ref(&self) -> &ConversationService {
        &self.conversation_service
    }

    /// Create the conversation + persist the new agent slot for a spawn.
    ///
    /// Holds the per-team `add_agent` lock for the entirety of the
    /// read-modify-write on `teams.agents`, matching [`TeamSessionService::add_agent`]
    /// (W4-D23) so concurrent spawns cannot race and drop slots.
    ///
    /// The lock is *not* held across the process warmup step — callers
    /// (`TeamSession::spawn_agent`) wire that up separately so a slow
    /// `warmup` never stalls other spawns against the same team.
    pub(crate) async fn persist_spawned_agent(
        &self,
        team_id: &str,
        user_id: &str,
        name: String,
        backend: String,
        model: String,
        custom_agent_id: Option<String>,
    ) -> Result<TeamAgent, TeamError> {
        let lock = self
            .add_agent_locks
            .entry(team_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Validate the team exists (FK target) + capture roster size for the
        // new slot's sort_order.
        self.repo
            .get_team(team_id)
            .await?
            .ok_or_else(|| TeamError::TeamNotFound(team_id.into()))?;
        let existing_agents = self.repo.list_team_agents(team_id).await?;

        let agent_type = parse_agent_type(&backend)?;
        let provider_id = if agent_type == AgentType::Nomi {
            self.resolve_provider_for_model(&model).await.unwrap_or(backend.clone())
        } else {
            backend.clone()
        };
        // Top-level `model` is nomi-only per spec 2026-05-12; for other
        // agent types the model/provider ride along in `extra`.
        let (top_level_model, extra) = if agent_type == AgentType::Nomi {
            (
                Some(ProviderWithModel {
                    provider_id,
                    model: model.clone(),
                    use_model: None,
                }),
                serde_json::json!({
                    "teamId": team_id,
                    "backend": backend,
                }),
            )
        } else {
            (
                None,
                serde_json::json!({
                    "teamId": team_id,
                    "backend": backend,
                    "provider_id": provider_id,
                    "current_model_id": model.clone(),
                }),
            )
        };
        let conv_req = CreateConversationRequest {
            r#type: agent_type,
            name: Some(name.clone()),
            model: top_level_model,
            source: None,
            channel_chat_id: None,
            extra,
        };
        let conv = self
            .conversation_service
            .create(user_id, conv_req)
            .await
            .map_err(TeamError::from_conversation_create)?;

        let agent = TeamAgent {
            slot_id: generate_prefixed_id("slot"),
            name,
            role: TeammateRole::Teammate,
            conversation_id: conv.id.to_string(),
            backend,
            model,
            custom_agent_id,
            status: None,
            conversation_type: None,
            cli_path: None,
        };

        // Conversation created above satisfies the slot's conversation_id FK
        // (spec §9.A). Append at the end of the roster.
        self.repo
            .create_team_agent(&agent.to_row(team_id, existing_agents.len() as i64))
            .await?;

        Ok(agent)
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_type_known_backends() {
        assert_eq!(parse_agent_type("acp").unwrap(), AgentType::Acp);
        assert_eq!(parse_agent_type("nanobot").unwrap(), AgentType::Nanobot);
        assert_eq!(parse_agent_type("remote").unwrap(), AgentType::Remote);
        assert_eq!(parse_agent_type("nomi").unwrap(), AgentType::Nomi);
    }

    #[test]
    fn parse_agent_type_unknown_backend_returns_error() {
        let err = parse_agent_type("unknown").unwrap_err();
        assert!(matches!(err, TeamError::InvalidRequest(_)));
    }

    #[test]
    fn parse_agent_type_openclaw_gateway() {
        assert_eq!(
            parse_agent_type("openclaw-gateway").unwrap(),
            AgentType::OpenclawGateway
        );
    }
}
