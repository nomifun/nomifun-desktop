mod response_builder;
pub(crate) mod spawn_support;

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use dashmap::DashMap;
use nomifun_ai_agent::IWorkerTaskManager;
use nomifun_api_types::{
    AddAgentRequest, CreateConversationRequest, CreateTeamRequest, GuideMcpConfig, TeamAgentResponse, TeamMcpPhase,
    TeamMcpStatusPayload, TeamResponse, WebSocketMessage,
};
use nomifun_common::{AgentKillReason, AgentType, ProviderWithModel, generate_prefixed_id, now_ms};
use nomifun_conversation::ConversationService;
use nomifun_db::models::TeamRow;
use nomifun_db::{IAgentMetadataRepository, IProviderRepository, ITeamRepository, UpdateTeamParams};
use nomifun_realtime::EventBroadcaster;
use tracing::{info, warn};

use self::spawn_support::{parse_agent_type, resolve_full_auto_mode};
use crate::error::TeamError;
use crate::event_loop::AgentLoopContext;
use crate::session::TeamSession;
use crate::types::{Team, TeamAgent, TeammateRole};

struct SessionEntry {
    session: Arc<TeamSession>,
}

pub struct TeamSessionService {
    repo: Arc<dyn ITeamRepository>,
    agent_metadata_repo: Arc<dyn IAgentMetadataRepository>,
    provider_repo: Arc<dyn IProviderRepository>,
    conversation_service: ConversationService,
    broadcaster: Arc<dyn EventBroadcaster>,
    task_manager: Arc<dyn IWorkerTaskManager>,
    backend_binary_path: Arc<PathBuf>,
    sessions: Arc<DashMap<String, SessionEntry>>,
    /// Per-team mutex serializing `add_agent` so concurrent callers cannot
    /// read-modify-write the `agents` JSON with stale state (last-writer-wins
    /// would otherwise drop entries).
    add_agent_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Per-team mutex serializing `ensure_session` so concurrent callers
    /// (e.g. create_team + frontend POST /session) cannot race and start
    /// two sessions for the same team.
    ensure_session_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Back-pointer used by [`TeamSession::spawn_agent`] to reach DB-facing
    /// orchestration without threading the service through every session method.
    /// Stored as `Weak` so the session map does not create a strong cycle with
    /// the service that owns it. Set once during [`TeamSessionService::new`]
    /// via [`Arc::new_cyclic`].
    self_ref: Weak<TeamSessionService>,
    /// Guide MCP server config used to refresh the leader's persisted
    /// `guide_mcp_config` on backend restart (port/token change each restart).
    /// `None` when the Guide server failed to start.
    guide_mcp_config: Option<GuideMcpConfig>,
}

impl TeamSessionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: Arc<dyn ITeamRepository>,
        agent_metadata_repo: Arc<dyn IAgentMetadataRepository>,
        provider_repo: Arc<dyn IProviderRepository>,
        conversation_service: ConversationService,
        broadcaster: Arc<dyn EventBroadcaster>,
        task_manager: Arc<dyn IWorkerTaskManager>,
        backend_binary_path: Arc<PathBuf>,
        guide_mcp_config: Option<GuideMcpConfig>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            repo,
            agent_metadata_repo,
            provider_repo,
            conversation_service,
            broadcaster,
            task_manager,
            backend_binary_path,
            sessions: Arc::new(DashMap::new()),
            add_agent_locks: Arc::new(DashMap::new()),
            ensure_session_locks: Arc::new(DashMap::new()),
            self_ref: weak.clone(),
            guide_mcp_config,
        })
    }

    /// Assemble the `Team` aggregate from a `teams` row + its `team_agents`
    /// rows. The roster used to live in a `TeamRow.agents` JSON column; it now
    /// lives in the dedicated `team_agents` table (spec §5.4), so we join it in
    /// here. Returns `None` agents on a load failure only by surfacing the
    /// `DbError` to the caller.
    async fn assemble_team(&self, row: &TeamRow) -> Result<Team, TeamError> {
        let agent_rows = self.repo.list_team_agents(&row.id).await?;
        Ok(Team::from_parts(row, &agent_rows))
    }

    /// Fetch + assemble a single team by id, or `TeamNotFound`.
    async fn load_team(&self, team_id: &str) -> Result<Team, TeamError> {
        let row = self
            .repo
            .get_team(team_id)
            .await?
            .ok_or_else(|| TeamError::TeamNotFound(team_id.into()))?;
        self.assemble_team(&row).await
    }

    /// Restore sessions for all existing teams. Called once at app startup
    /// so that MCP servers are available before any user sends a message.
    pub async fn restore_all_sessions(&self) {
        let teams = match self.repo.list_teams().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "failed to list teams for session restore");
                return;
            }
        };
        for team in &teams {
            if let Err(e) = self.ensure_session(&team.id).await {
                tracing::warn!(team_id = %team.id, error = %e, "failed to restore session on startup");
                continue;
            }
            // Patch the leader's persisted guide_mcp_config so it points at the
            // current restart's port/token (the Guide server picks a new random
            // port on every start).
            if let Some(ref cfg) = self.guide_mcp_config {
                let team_data = match self.load_team(&team.id).await {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if let Some(leader) = team_data.agents.iter().find(|a| a.role == TeammateRole::Lead) {
                    let patch = serde_json::json!({ "guide_mcp_config": cfg });
                    if let Err(e) = self
                        .conversation_service
                        .update_extra(&leader.conversation_id, patch)
                        .await
                    {
                        warn!(
                            team_id = %team.id,
                            conversation_id = %leader.conversation_id,
                            error = %e,
                            "failed to patch leader guide_mcp_config on restore"
                        );
                    }
                }
            }
        }
        if !teams.is_empty() {
            tracing::info!(count = teams.len(), "team sessions restored on startup");
        }
    }

    pub async fn create_team(&self, user_id: &str, req: CreateTeamRequest) -> Result<TeamResponse, TeamError> {
        if req.agents.is_empty() {
            return Err(TeamError::InvalidRequest("at least one agent is required".into()));
        }

        let team_id = generate_prefixed_id("team");
        let now = now_ms();
        let mut agents = Vec::with_capacity(req.agents.len());

        for (i, input) in req.agents.iter().enumerate() {
            let slot_id = generate_prefixed_id("slot");
            let role = if i == 0 {
                TeammateRole::Lead
            } else {
                TeammateRole::parse(&input.role).unwrap_or(TeammateRole::Teammate)
            };

            // Resolve the conversation_id: adopt an existing conversation when
            // the caller supplies one (single-chat → team-chat handoff), or
            // create a new one otherwise.
            let conv_id = if let Some(existing_id) = input.conversation_id {
                // Adopt the existing conversation by updating its extra with
                // teamId and backend so the agent is wired into this team.
                // The conversation service is string-keyed (Option A), so bridge
                // the now-i64 FK to a String at the call boundary.
                let existing_id_str = existing_id.to_string();
                self.conversation_service
                    .update_extra(
                        &existing_id_str,
                        serde_json::json!({"teamId": team_id, "backend": input.backend, "session_mode": resolve_full_auto_mode(&input.backend)}),
                    )
                    .await
                    .map_err(|e| TeamError::InvalidRequest(format!("failed to adopt conversation: {e}")))?;
                // Notify frontend that this conversation moved into a team so
                // the sidebar can remove it from the standalone list.
                self.broadcaster.broadcast(WebSocketMessage::new(
                    "conversation.listChanged",
                    serde_json::json!({
                        "conversation_id": existing_id,
                        "action": "updated",
                    }),
                ));
                existing_id_str
            } else {
                let agent_type = parse_agent_type(&input.backend)?;
                let provider_id = if agent_type == AgentType::Nomi {
                    self.resolve_provider_for_model(&input.model)
                        .await
                        .unwrap_or_else(|| input.backend.clone())
                } else {
                    input.backend.clone()
                };
                // Top-level `model` is nomi-only per spec 2026-05-12; for
                // other agent types the model/provider ride along in `extra`.
                let (top_level_model, extra) = if agent_type == AgentType::Nomi {
                    let mut extra = serde_json::json!({
                        "teamId": team_id,
                        "backend": input.backend,
                        "session_mode": resolve_full_auto_mode(&input.backend),
                    });
                    if let Some(ref ws) = req.workspace
                        && !ws.is_empty()
                    {
                        extra["workspace"] = serde_json::Value::String(ws.clone());
                    }
                    (
                        Some(ProviderWithModel {
                            provider_id,
                            model: input.model.clone(),
                            use_model: None,
                        }),
                        extra,
                    )
                } else {
                    let mut extra = serde_json::json!({
                        "teamId": team_id,
                        "backend": input.backend,
                        "session_mode": resolve_full_auto_mode(&input.backend),
                        "provider_id": provider_id,
                        "current_model_id": input.model.clone(),
                    });
                    if let Some(ref ws) = req.workspace
                        && !ws.is_empty()
                    {
                        extra["workspace"] = serde_json::Value::String(ws.clone());
                    }
                    (None, extra)
                };
                let conv_req = CreateConversationRequest {
                    r#type: agent_type,
                    name: Some(input.name.clone()),
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
                conv.id.to_string()
            };

            agents.push(TeamAgent {
                slot_id,
                name: input.name.clone(),
                role,
                conversation_id: conv_id,
                backend: input.backend.clone(),
                model: input.model.clone(),
                custom_agent_id: input.custom_agent_id.clone(),
                status: None,
                conversation_type: None,
                cli_path: None,
            });
        }

        let lead_agent_id = agents.first().map(|a| a.slot_id.clone());

        let row = TeamRow {
            id: team_id.clone(),
            user_id: user_id.to_owned(),
            name: req.name.clone(),
            workspace: req.workspace.clone().unwrap_or_default(),
            workspace_mode: "shared".into(),
            lead_agent_id: lead_agent_id.clone(),
            session_mode: None,
            agents_version: "1.0.1".into(),
            created_at: now,
            updated_at: now,
        };
        // Insert the `teams` row first so the `team_agents.team_id` FK is
        // satisfied. Each agent's conversation was already created above, so
        // the `team_agents.conversation_id` FK holds too (spec §9.A: session
        // before slot).
        self.repo.create_team(&row).await?;
        for (i, agent) in agents.iter().enumerate() {
            self.repo.create_team_agent(&agent.to_row(&team_id, i as i64)).await?;
        }

        let team = Team {
            id: team_id,
            name: req.name,
            agents,
            lead_agent_id,
            created_at: now,
            updated_at: now,
        };

        info!(team_id = %team.id, "Team created");

        self.broadcaster.broadcast(WebSocketMessage::new(
            "team.created",
            serde_json::json!({ "team_id": team.id, "team_name": team.name }),
        ));

        // Auto-start session so MCP is injected immediately after team creation.
        // Failure only logs — the team is persisted and frontend can retry
        // via POST /api/teams/{id}/session if needed.
        if let Err(e) = self.ensure_session_inner(&team.id, true).await {
            warn!(team_id = %team.id, error = %e, "auto ensure_session after create_team failed");
        }

        self.build_team_response(&team).await
    }

    pub async fn list_teams(&self) -> Result<Vec<TeamResponse>, TeamError> {
        let rows = self.repo.list_teams().await?;
        let mut teams = Vec::with_capacity(rows.len());
        for row in &rows {
            match self.assemble_team(row).await {
                Ok(team) => match self.build_team_response(&team).await {
                    Ok(resp) => teams.push(resp),
                    Err(e) => {
                        tracing::warn!(team_id = %row.id, error = %e, "skipping team with build error");
                    }
                },
                Err(e) => {
                    tracing::warn!(team_id = %row.id, error = %e, "skipping team with agent-roster load error");
                }
            }
        }
        Ok(teams)
    }

    pub async fn get_team(&self, team_id: &str) -> Result<TeamResponse, TeamError> {
        let team = self.load_team(team_id).await?;
        self.build_team_response(&team).await
    }

    pub async fn remove_team(&self, user_id: &str, team_id: &str) -> Result<(), TeamError> {
        let team = self.load_team(team_id).await?;

        self.stop_session(team_id);

        let kill_futures: Vec<_> = team
            .agents
            .iter()
            .map(|agent| {
                self.task_manager
                    .kill_and_wait(&agent.conversation_id, Some(AgentKillReason::TeamDeleted))
            })
            .collect();

        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            futures_util::future::join_all(kill_futures),
        )
        .await;

        for agent in &team.agents {
            let _ = self.conversation_service.delete(user_id, &agent.conversation_id).await;
        }

        // A single delete: the FK `ON DELETE CASCADE` chain removes the team's
        // `team_agents`, `mailbox`, `team_tasks`, and `team_task_deps` rows
        // (spec §5.4) — no manual `delete_mailbox_by_team`/`delete_tasks_by_team`.
        self.repo.delete_team(team_id).await?;

        self.add_agent_locks.remove(team_id);

        info!(team_id = %team_id, "Team removed");
        Ok(())
    }

    pub async fn rename_team(&self, team_id: &str, name: &str) -> Result<(), TeamError> {
        self.repo
            .get_team(team_id)
            .await?
            .ok_or_else(|| TeamError::TeamNotFound(team_id.into()))?;

        self.repo
            .update_team(
                team_id,
                &UpdateTeamParams {
                    name: Some(name.to_owned()),
                    lead_agent_id: None,
                },
            )
            .await?;
        Ok(())
    }

    pub async fn add_agent(
        &self,
        user_id: &str,
        team_id: &str,
        req: AddAgentRequest,
    ) -> Result<TeamAgentResponse, TeamError> {
        let lock = self
            .add_agent_locks
            .entry(team_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Validate the team exists (FK target for the new slot) and capture the
        // current roster size for the new slot's sort_order.
        self.repo
            .get_team(team_id)
            .await?
            .ok_or_else(|| TeamError::TeamNotFound(team_id.into()))?;
        let existing_agents = self.repo.list_team_agents(team_id).await?;

        let slot_id = generate_prefixed_id("slot");
        let role = TeammateRole::parse(&req.role).unwrap_or(TeammateRole::Teammate);
        let agent_type = parse_agent_type(&req.backend)?;

        let provider_id = if agent_type == AgentType::Nomi {
            self.resolve_provider_for_model(&req.model)
                .await
                .unwrap_or_else(|| req.backend.clone())
        } else {
            req.backend.clone()
        };
        // Top-level `model` is nomi-only per spec 2026-05-12; for other
        // agent types the model/provider ride along in `extra`.
        let (top_level_model, extra) = if agent_type == AgentType::Nomi {
            (
                Some(ProviderWithModel {
                    provider_id,
                    model: req.model.clone(),
                    use_model: None,
                }),
                serde_json::json!({
                    "teamId": team_id,
                    "backend": req.backend,
                }),
            )
        } else {
            (
                None,
                serde_json::json!({
                    "teamId": team_id,
                    "backend": req.backend,
                    "provider_id": provider_id,
                    "current_model_id": req.model.clone(),
                }),
            )
        };
        let conv_req = CreateConversationRequest {
            r#type: agent_type,
            name: Some(req.name.clone()),
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
            slot_id,
            name: req.name,
            role,
            conversation_id: conv.id.to_string(),
            backend: req.backend,
            model: req.model,
            custom_agent_id: req.custom_agent_id,
            status: None,
            conversation_type: None,
            cli_path: None,
        };

        // The slot's conversation now exists, so the `team_agents.conversation_id`
        // FK holds (spec §9.A). Append at the end of the roster.
        self.repo
            .create_team_agent(&agent.to_row(team_id, existing_agents.len() as i64))
            .await?;

        if let Some(session) = self.sessions.get(team_id).map(|e| Arc::clone(&e.session)) {
            session.add_agent(&agent).await;
            self.register_event_loop(team_id, &agent.slot_id);
        }

        self.build_agent_response(&agent).await
    }

    pub async fn remove_agent(&self, user_id: &str, team_id: &str, slot_id: &str) -> Result<(), TeamError> {
        self.repo
            .get_team(team_id)
            .await?
            .ok_or_else(|| TeamError::TeamNotFound(team_id.into()))?;

        let slot = self
            .repo
            .get_team_agent(slot_id)
            .await?
            .filter(|a| a.team_id == team_id)
            .ok_or_else(|| TeamError::AgentNotFound(slot_id.into()))?;

        if let Some(conv_id) = slot.conversation_id {
            // conversation_service is string-keyed (Option A); bridge the i64 FK.
            let _ = self.conversation_service.delete(user_id, &conv_id.to_string()).await;
        }

        self.repo.remove_team_agent(slot_id).await?;

        if let Some(session) = self.sessions.get(team_id).map(|e| Arc::clone(&e.session)) {
            let _ = session.remove_agent(slot_id).await;
        }

        Ok(())
    }

    pub async fn rename_agent(&self, team_id: &str, slot_id: &str, name: &str) -> Result<(), TeamError> {
        self.repo
            .get_team(team_id)
            .await?
            .ok_or_else(|| TeamError::TeamNotFound(team_id.into()))?;

        let normalized = crate::scheduler::normalize_name(name);
        if normalized.is_empty() {
            return Err(TeamError::InvalidRequest(
                "rename_agent.name is empty after normalization".into(),
            ));
        }

        let agents = self.repo.list_team_agents(team_id).await?;
        agents
            .iter()
            .find(|a| a.slot_id == slot_id)
            .ok_or_else(|| TeamError::AgentNotFound(slot_id.into()))?;

        // Uniqueness check against all other agents in the team.
        let has_conflict = agents
            .iter()
            .any(|a| a.slot_id != slot_id && crate::scheduler::normalize_name(&a.name) == normalized);
        if has_conflict {
            return Err(TeamError::DuplicateAgentName(name.to_owned()));
        }

        self.repo.rename_team_agent(slot_id, name).await?;

        if let Some(session) = self.sessions.get(team_id).map(|e| Arc::clone(&e.session)) {
            let _ = session.rename_agent(slot_id, name).await;
        }

        Ok(())
    }

    /// Start the team's MCP server and rebuild every agent process so it
    /// carries a fresh `team_mcp_stdio_config` pointing at the new server.
    ///
    /// Flow (mcp.md §4.3):
    /// 1. Start `TeamSession` (opens the MCP TCP server).
    /// 2. For each agent: persist `team_mcp_stdio_config` into
    ///    `conversation.extra` → `task_manager.kill(conv_id, TeamMcpRebuild)`
    ///    → `conversation_service.warmup(...)` rebuilds the ACP process with
    ///    the new extra.
    /// 3. Spawn per-agent event loops that drain the mailbox whenever notified.
    /// 4. Only insert into `sessions` after every step above succeeds — on
    ///    any failure, stop the session and leave the map untouched so a
    ///    retry can start cleanly.
    pub async fn ensure_session(&self, team_id: &str) -> Result<(), TeamError> {
        self.ensure_session_inner(team_id, false).await
    }

    async fn ensure_session_inner(&self, team_id: &str, skip_leader: bool) -> Result<(), TeamError> {
        if self.sessions.contains_key(team_id) {
            return Ok(());
        }

        let lock = self
            .ensure_session_locks
            .entry(team_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Re-check after acquiring lock (another caller may have completed).
        if self.sessions.contains_key(team_id) {
            return Ok(());
        }

        let row = match self.repo.get_team(team_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                self.broadcast_mcp_phase(team_id, "", TeamMcpPhase::LoadFailed, None, |p| {
                    p.error = Some(format!("team not found: {team_id}"));
                });
                return Err(TeamError::TeamNotFound(team_id.into()));
            }
            Err(e) => {
                self.broadcast_mcp_phase(team_id, "", TeamMcpPhase::LoadFailed, None, |p| {
                    p.error = Some(e.to_string());
                });
                return Err(e.into());
            }
        };
        let user_id = row.user_id.clone();
        let team = self.assemble_team(&row).await?;
        let agents_snapshot: Vec<TeamAgent> = team.agents.clone();

        let session = match TeamSession::start(
            team,
            self.repo.clone(),
            self.broadcaster.clone(),
            self.backend_binary_path.clone(),
            self.task_manager.clone(),
            user_id.clone(),
            self.self_ref.clone(),
        )
        .await
        {
            Ok(session) => session,
            Err(e) => {
                self.broadcast_mcp_phase(team_id, "", TeamMcpPhase::SessionError, None, |p| {
                    p.error = Some(e.to_string());
                });
                return Err(e);
            }
        };

        self.broadcast_mcp_phase(team_id, "", TeamMcpPhase::SessionInjecting, None, |_| {});

        if let Err(e) = self
            .rebuild_agent_processes(team_id, &session, &user_id, &agents_snapshot, skip_leader)
            .await
        {
            session.stop();
            return Err(e);
        }

        let session = Arc::new(session);

        // Spawn per-agent event loops
        self.spawn_event_loops(&session, &user_id, &agents_snapshot);

        let entry = SessionEntry {
            session: session.clone(),
        };
        self.sessions.insert(team_id.to_owned(), entry);

        // Notify all agents so they drain any pre-existing mailbox messages
        // (e.g. from a prior session or backend restart).
        for agent in &agents_snapshot {
            session.notify_agent(&agent.slot_id);
        }

        let active_count = if skip_leader {
            agents_snapshot.iter().filter(|a| a.role != TeammateRole::Lead).count()
        } else {
            agents_snapshot.len()
        };
        self.broadcast_mcp_phase(team_id, "", TeamMcpPhase::SessionReady, None, |p| {
            p.server_count = Some(active_count);
        });

        Ok(())
    }

    fn broadcast_mcp_phase<F>(&self, team_id: &str, slot_id: &str, phase: TeamMcpPhase, port: Option<u16>, customize: F)
    where
        F: FnOnce(&mut TeamMcpStatusPayload),
    {
        let mut payload = TeamMcpStatusPayload {
            team_id: team_id.to_owned(),
            slot_id: slot_id.to_owned(),
            phase,
            port,
            server_count: None,
            error: None,
        };
        customize(&mut payload);
        let event = WebSocketMessage::new(
            "team.mcpStatus",
            serde_json::to_value(payload).expect("serialize mcp status payload"),
        );
        self.broadcaster.broadcast(event);
    }

    async fn rebuild_agent_processes(
        &self,
        team_id: &str,
        session: &TeamSession,
        user_id: &str,
        agents: &[TeamAgent],
        skip_leader: bool,
    ) -> Result<(), TeamError> {
        for agent in agents {
            let cfg = session.mcp_stdio_config(&agent.slot_id);
            let patch = serde_json::json!({
                "team_mcp_stdio_config": cfg,
                "session_mode": resolve_full_auto_mode(&agent.backend),
            });

            // Always persist team_mcp_stdio_config into the leader's extra
            // so subsequent warmups pick it up. Only skip the kill+warmup
            // when the leader is already running (guide flow).
            if skip_leader && agent.role == TeammateRole::Lead {
                if let Err(e) = self
                    .conversation_service
                    .update_extra(&agent.conversation_id, patch)
                    .await
                {
                    warn!(
                        team_id,
                        slot_id = %agent.slot_id,
                        error = %e,
                        "failed to persist team_mcp_stdio_config for skipped leader"
                    );
                }
                continue;
            }

            if let Err(e) = self
                .conversation_service
                .update_extra(&agent.conversation_id, patch)
                .await
            {
                let msg = format!("failed to persist team_mcp_stdio_config for {}: {e}", agent.slot_id);
                self.broadcast_mcp_phase(team_id, &agent.slot_id, TeamMcpPhase::ConfigWriteFailed, None, |p| {
                    p.error = Some(msg.clone());
                });
                return Err(TeamError::InvalidRequest(msg));
            }

            let _ = self
                .task_manager
                .kill(&agent.conversation_id, Some(AgentKillReason::TeamMcpRebuild));

            if let Err(e) = self
                .conversation_service
                .warmup(user_id, &agent.conversation_id, &self.task_manager)
                .await
            {
                let msg = format!("failed to warm up rebuilt agent {}: {e}", agent.slot_id);
                self.broadcast_mcp_phase(team_id, &agent.slot_id, TeamMcpPhase::SessionError, None, |p| {
                    p.error = Some(msg.clone());
                });
                warn!(
                    team_id,
                    slot_id = %agent.slot_id,
                    conversation_id = %agent.conversation_id,
                    error = %e,
                    "warmup failed during rebuild"
                );
                return Err(TeamError::InvalidRequest(msg));
            }
        }
        Ok(())
    }

    /// Spawn per-agent event loops that drain the mailbox whenever notified.
    /// Each agent gets its own tokio task that runs until the session shuts down.
    fn spawn_event_loops(&self, session: &Arc<TeamSession>, user_id: &str, agents: &[TeamAgent]) {
        let registry = session.event_loops();

        for agent in agents {
            let ctx = AgentLoopContext {
                team_id: session.team_id().to_owned(),
                slot_id: agent.slot_id.clone(),
                user_id: user_id.to_owned(),
                session: session.clone(),
                scheduler: session.scheduler().clone(),
                mailbox: session.mailbox().clone(),
                task_manager: self.task_manager.clone(),
                conversation_service: self.conversation_service.clone(),
                broadcaster: self.broadcaster.clone(),
                registry: registry.clone(),
            };
            registry.spawn(&agent.slot_id, ctx);
        }
    }

    /// Register an event loop for a dynamically spawned agent.
    ///
    /// Called by [`TeamSession::spawn_agent`] after `attach_spawned_agent_process`
    /// succeeds so the newly booted agent gets its own drain loop — exactly as
    /// `spawn_event_loops` does for the initial members during `ensure_session`.
    pub(crate) fn register_event_loop(&self, team_id: &str, slot_id: &str) {
        let Some(entry) = self.sessions.get(team_id) else {
            return;
        };
        let session = Arc::clone(&entry.session);
        let registry = session.event_loops();

        let ctx = AgentLoopContext {
            team_id: team_id.to_owned(),
            slot_id: slot_id.to_owned(),
            user_id: session.user_id().to_owned(),
            session: session.clone(),
            scheduler: session.scheduler().clone(),
            mailbox: session.mailbox().clone(),
            task_manager: self.task_manager.clone(),
            conversation_service: self.conversation_service.clone(),
            broadcaster: self.broadcaster.clone(),
            registry: registry.clone(),
        };
        registry.spawn(slot_id, ctx);
    }

    pub async fn get_session_user_id(&self, team_id: &str) -> Option<String> {
        self.sessions.get(team_id).map(|e| e.session.user_id().to_owned())
    }

    pub fn get_session_scheduler(&self, team_id: &str) -> Option<Arc<crate::scheduler::TeammateManager>> {
        self.sessions.get(team_id).map(|e| e.session.scheduler().clone())
    }

    pub fn stop_session(&self, team_id: &str) {
        if let Some((_, entry)) = self.sessions.remove(team_id) {
            entry.session.event_loops().shutdown();
            entry.session.stop();
        }
    }

    pub async fn send_message(
        &self,
        team_id: &str,
        content: &str,
        files: Option<Vec<String>>,
    ) -> Result<(), TeamError> {
        self.ensure_session(team_id).await?;
        let session = {
            let entry = self
                .sessions
                .get(team_id)
                .ok_or_else(|| TeamError::SessionNotFound(team_id.into()))?;
            Arc::clone(&entry.session)
        };
        session.send_message(content, files).await
    }

    pub async fn send_message_to_agent(
        &self,
        team_id: &str,
        slot_id: &str,
        content: &str,
        files: Option<Vec<String>>,
    ) -> Result<(), TeamError> {
        self.ensure_session(team_id).await?;
        let session = {
            let entry = self
                .sessions
                .get(team_id)
                .ok_or_else(|| TeamError::SessionNotFound(team_id.into()))?;
            Arc::clone(&entry.session)
        };
        session.send_message_to_agent(slot_id, content, files).await
    }

    pub async fn set_session_mode(&self, team_id: &str, mode: &str) -> Result<(), TeamError> {
        let team = self.load_team(team_id).await?;

        for agent in &team.agents {
            if let Some(instance) = self.task_manager.get_task(&agent.conversation_id)
                && let Err(e) = instance.set_mode(mode).await
            {
                warn!(
                    team_id,
                    slot_id = %agent.slot_id,
                    conversation_id = %agent.conversation_id,
                    error = %e,
                    "failed to set session mode on agent"
                );
            }
            let patch = serde_json::json!({ "session_mode": mode });
            let _ = self
                .conversation_service
                .update_extra(&agent.conversation_id, patch)
                .await;
            let _ = self
                .conversation_service
                .save_acp_runtime_mode(&agent.conversation_id, mode)
                .await;
        }

        Ok(())
    }

    /// Wake a specific agent in a team session (trigger it to read mailbox).
    /// Called by MCP dispatch after `team_send_message` writes to mailbox.
    ///
    /// In the event-loop model this simply notifies the agent's event loop.
    pub async fn wake_agent_in_session(&self, team_id: &str, slot_id: &str) -> Result<(), TeamError> {
        let entry = self
            .sessions
            .get(team_id)
            .ok_or_else(|| TeamError::SessionNotFound(team_id.into()))?;
        entry.session.notify_agent(slot_id);
        Ok(())
    }
}
