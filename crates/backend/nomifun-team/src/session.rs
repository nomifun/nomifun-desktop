use std::path::PathBuf;
use std::sync::{Arc, Weak};

use nomifun_ai_agent::IWorkerTaskManager;
use nomifun_ai_agent::types::SendMessageData;
use nomifun_common::AgentKillReason;
use nomifun_conversation::ConversationService;
use nomifun_db::ITeamRepository;
use nomifun_realtime::EventBroadcaster;
use tracing::{info, warn};

use crate::error::TeamError;
use crate::event_loop::EventLoopRegistry;
use crate::mailbox::Mailbox;
use crate::mcp::{TeamMcpServer, TeamMcpStdioConfig, TeamMcpStdioServerSpec};
use crate::prompts::{build_lead_prompt, build_teammate_prompt, build_wake_payload};
use crate::scheduler::{TeammateManager, normalize_name};
use crate::service::TeamSessionService;
use crate::service::spawn_support::resolve_full_auto_mode;
use crate::task_board::TaskBoard;
use crate::types::{MailboxMessageType, Team, TeamAgent, TeammateRole, TeammateStatus};

/// Bridge a String conversation_id (Option A domain representation) to the i64
/// FK/PK now used by the DB/row layer. Returns a `TeamError` when the id is not
/// a valid integer.
fn parse_conv_id(id: &str) -> Result<i64, TeamError> {
    id.parse::<i64>()
        .map_err(|_| TeamError::InvalidRequest(format!("invalid conversation id: {id}")))
}

/// Input for the wake path. Produced by [`TeamSession::compute_wake_input`],
/// consumed by D7b's `send_message` / `send_message_to_agent` (not implemented
/// in D7a). `first_message` includes the role prompt on cold starts.
#[derive(Debug, Clone)]
pub struct WakeInput {
    pub conversation_id: String,
    pub first_message: String,
    /// `false` when the mailbox is empty — caller should skip wake and
    /// leave the agent idle.
    pub should_send: bool,
    /// Unread mailbox rows used to build `first_message`. Returned so the
    /// caller can mirror non-user senders into the target agent's conversation
    /// as left bubbles (matches Nomi `TeammateManager.wake()`). These are
    /// **not** yet marked as read — the caller must call
    /// `mailbox.mark_read_batch` after successful delivery.
    pub unread: Vec<crate::types::MailboxMessage>,
    /// Role of the wake target. Leader wakes do **not** mirror mailbox rows
    /// into the conversation — the content is already embedded in the role
    /// prompt sent to the leader directly. Only teammate wakes get the left
    /// bubble treatment.
    pub agent_role: TeammateRole,
}

/// Input for [`TeamSession::spawn_agent`]. Populated by the lead agent when
/// it calls the `spawn_agent` MCP tool.
#[derive(Debug, Clone)]
pub struct SpawnAgentRequest {
    pub name: String,
    pub agent_type: Option<String>,
    pub custom_agent_id: Option<String>,
    pub model: Option<String>,
}

pub struct TeamSession {
    team: Team,
    scheduler: Arc<TeammateManager>,
    mailbox: Arc<Mailbox>,
    task_board: Arc<TaskBoard>,
    mcp_server: TeamMcpServer,
    backend_binary_path: Arc<PathBuf>,
    task_manager: Arc<dyn IWorkerTaskManager>,
    /// Owner user_id for this team — needed when spawn_agent creates a
    /// new conversation (conversations are scoped per user).
    user_id: String,
    /// Weak upward ref so `spawn_agent` can reach the DB-facing orchestration
    /// in `TeamSessionService` (conversation creation, persisted agent list)
    /// without creating a strong cycle with the session map that owns `self`.
    /// `None` in unit tests that don't exercise the DB path.
    service: Weak<TeamSessionService>,
    /// Used by the wake path to mirror non-user mailbox rows into the target
    /// agent's conversation as left bubbles (Nomi parity: see
    /// `TeammateManager.wake()`'s `teammate_message` emission).
    broadcaster: Arc<dyn EventBroadcaster>,
    /// Per-agent event loop registry. Each agent has a dedicated tokio task
    /// that drains its mailbox whenever notified.
    event_loops: Arc<EventLoopRegistry>,
}

impl TeamSession {
    pub async fn start(
        team: Team,
        repo: Arc<dyn ITeamRepository>,
        broadcaster: Arc<dyn EventBroadcaster>,
        backend_binary_path: Arc<PathBuf>,
        task_manager: Arc<dyn IWorkerTaskManager>,
        user_id: String,
        service: Weak<TeamSessionService>,
    ) -> Result<Self, TeamError> {
        let mailbox = Arc::new(Mailbox::new(repo.clone()));
        let task_board = Arc::new(TaskBoard::new(repo));

        let scheduler = Arc::new(TeammateManager::new(
            team.id.clone(),
            &team.agents,
            mailbox.clone(),
            task_board.clone(),
            broadcaster.clone(),
        ));

        let auth_token = nomifun_common::generate_id();
        let mcp_server = TeamMcpServer::start(
            auth_token,
            scheduler.clone(),
            team.id.clone(),
            broadcaster.clone(),
            service.clone(),
        )
        .await?;

        let event_loops = Arc::new(EventLoopRegistry::new());

        info!(
            team_id = %team.id,
            port = mcp_server.port(),
            "TeamSession started"
        );

        Ok(Self {
            team,
            scheduler,
            mailbox,
            task_board,
            mcp_server,
            backend_binary_path,
            task_manager,
            user_id,
            service,
            broadcaster,
            event_loops,
        })
    }

    pub fn team_id(&self) -> &str {
        &self.team.id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn scheduler(&self) -> &Arc<TeammateManager> {
        &self.scheduler
    }

    pub fn event_loops(&self) -> &Arc<EventLoopRegistry> {
        &self.event_loops
    }

    /// Signal an agent's event loop to drain its mailbox.
    pub fn notify_agent(&self, slot_id: &str) {
        self.event_loops.notify(slot_id);
    }

    pub fn mcp_stdio_config(&self, slot_id: &str) -> TeamMcpStdioConfig {
        TeamMcpStdioConfig {
            team_id: self.team.id.clone(),
            port: self.mcp_server.port(),
            token: self.mcp_server.auth_token().to_owned(),
            slot_id: slot_id.to_owned(),
            binary_path: self.backend_binary_path.to_string_lossy().into_owned(),
        }
    }

    /// Returns the stdio server spec that `TeamSessionService::ensure_session`
    /// (D9) persists into each agent's `conversation.extra` and that ACP
    /// `session/new` consumes via `mcp_servers`.
    pub fn stdio_spec(&self, slot_id: &str) -> TeamMcpStdioServerSpec {
        let binary_path = self.backend_binary_path.to_string_lossy();
        TeamMcpStdioServerSpec::from_config(binary_path.as_ref(), &self.mcp_stdio_config(slot_id))
    }

    /// Assemble the payload that will drive the next wake of `slot_id`.
    ///
    /// - Reads status, unread messages and tasks.
    /// - Cold-start agents (no prior status, or last status was `Error`)
    ///   receive the full role prompt prepended to the wake payload.
    /// - When the mailbox is empty, returns `WakeInput { should_send: false, .. }`
    ///   so the caller can skip the wake and mark the agent idle.
    /// - Filters out messages where `from_agent_id == slot_id` (prevent self-trigger).
    ///
    /// Messages are **not** marked as read here. The caller is responsible for
    /// calling `mailbox.mark_read_batch` after successful delivery.
    pub async fn compute_wake_input(&self, slot_id: &str) -> Result<Option<WakeInput>, TeamError> {
        let agent = self.scheduler.get_agent(slot_id).await?;
        let all_unread = self.mailbox.peek_unread(&self.team.id, slot_id).await?;
        // Filter out self-messages to prevent an agent from triggering itself.
        let unread: Vec<_> = all_unread.into_iter().filter(|m| m.from_agent_id != slot_id).collect();
        let tasks = self.scheduler.list_tasks().await?;

        let wake_body = build_wake_payload(&agent, &tasks, &unread);

        let needs_role_prompt = self.scheduler.take_needs_role_prompt(slot_id).await;

        let first_message = if needs_role_prompt {
            let role_prompt = match agent.role {
                TeammateRole::Lead => {
                    let available_agent_types = match self.service.upgrade() {
                        Some(svc) => svc.list_team_capable_backends().await,
                        None => crate::guide::capability::TEAM_CAPABLE_BACKENDS
                            .iter()
                            .map(|b| {
                                let mut c = b.chars();
                                let display = match c.next() {
                                    None => String::new(),
                                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                                };
                                (b.to_string(), display)
                            })
                            .collect(),
                    };
                    build_lead_prompt(
                        &self.team.name,
                        &self.scheduler.list_agents().await,
                        &available_agent_types,
                    )
                }
                TeammateRole::Teammate => build_teammate_prompt(&agent, &self.team.name),
            };
            format!("{role_prompt}\n\n{wake_body}")
        } else {
            wake_body
        };

        let should_send = !unread.is_empty();

        Ok(Some(WakeInput {
            conversation_id: agent.conversation_id,
            first_message,
            should_send,
            unread,
            agent_role: agent.role,
        }))
    }

    /// Handle agent Finish/Error events. Delegates to the scheduler's
    /// `finalize_turn` with no parsed actions (phase1 does not parse the
    /// trailing message for scheduler directives). Returns the leader slot_id
    /// that the caller should re-wake, if any; D7b wires that return value
    /// into the wake path. `is_error` is reserved for future status handling.
    pub async fn on_agent_finish(&self, conversation_id: &str, is_error: bool) -> Result<Option<String>, TeamError> {
        // Dedup: skip if another finish event already claimed this conversation
        // within the 5-second window (W4-D19a).
        if !self.scheduler.begin_finalize(conversation_id) {
            return Ok(None);
        }

        let slot_id = {
            let agents = self.scheduler.list_agents().await;
            agents
                .into_iter()
                .find(|a| a.conversation_id == conversation_id)
                .map(|a| a.slot_id)
                .ok_or_else(|| TeamError::AgentNotFound(format!("no agent with conversation_id={conversation_id}")))?
        };

        // The event loop's `finalize_turn` handles most cases, but
        // `on_agent_finish` remains callable for nomi resume and test scenarios.
        // `begin_finalize` dedup prevents double finalization.

        if is_error {
            self.scheduler.set_status(&slot_id, TeammateStatus::Error).await?;
        }

        let wake_target = self.scheduler.finalize_turn(&slot_id, &[]).await?;

        // Clear the dedup window unconditionally once finalize has run.
        self.scheduler.clear_finalized_turn(conversation_id);

        // Re-wake self if there are still unread messages in mailbox.
        // This handles the case where messages arrived while the agent was
        // working (e.g. shutdown_request). Mirrors Claude's useMailboxBridge:
        // when isLoading becomes false, poll mailbox and submit if non-empty.
        if wake_target.as_deref() != Some(&slot_id) {
            let has_unread = self.mailbox.has_unread(&self.team.id, &slot_id).await.unwrap_or(false);
            if has_unread {
                return Ok(Some(slot_id));
            }
        }

        Ok(wake_target)
    }

    /// Write a user message to the lead's mailbox and trigger a wake.
    ///
    /// Wake failures are logged but **not** propagated (D7b log-not-throw
    /// semantics — see backend-audit §3.5 #46): the mailbox row is already
    /// persisted, so surfacing an error to the HTTP caller would invite a
    /// retry that double-writes the message.
    pub async fn send_message(&self, content: &str, files: Option<Vec<String>>) -> Result<(), TeamError> {
        let lead_slot_id = self
            .scheduler
            .find_lead_slot_id()
            .await
            .ok_or_else(|| TeamError::AgentNotFound("no lead agent in team".into()))?;

        let lead_conv_id = self.scheduler.get_agent(&lead_slot_id).await?.conversation_id;

        self.mailbox
            .write_with_files(
                &self.team.id,
                &lead_slot_id,
                "user",
                MailboxMessageType::Message,
                content,
                None,
                files.as_deref(),
            )
            .await?;

        // Persist the user message as a right bubble in the leader's conversation.
        // Strip [SYSTEM NOTE: ...] blocks so internal instructions are not visible
        // to the user in the chat UI.
        if let Some(svc) = self.service.upgrade() {
            let visible_content = strip_system_notes(content);
            let msg_id = ConversationService::mint_msg_id();
            let row = nomifun_db::models::MessageRow {
                id: msg_id.clone(),
                conversation_id: parse_conv_id(&lead_conv_id)?,
                msg_id: Some(msg_id),
                r#type: "text".into(),
                content: serde_json::json!({ "content": visible_content }).to_string(),
                position: Some("right".into()),
                status: Some("finish".into()),
                hidden: false,
                created_at: nomifun_common::now_ms(),
            };
            if let Err(e) = svc.conversation_service_ref().insert_raw_message(&row).await {
                warn!(
                    team_id = %self.team.id,
                    error = %e,
                    "failed to persist user right bubble for leader (non-fatal)"
                );
            }
        }

        self.try_wake(&lead_slot_id, files).await;
        Ok(())
    }

    /// Write a user message to the specified agent's mailbox and trigger a wake.
    ///
    /// Same log-not-throw behaviour as [`send_message`]; see that method for
    /// rationale.
    pub async fn send_message_to_agent(
        &self,
        slot_id: &str,
        content: &str,
        files: Option<Vec<String>>,
    ) -> Result<(), TeamError> {
        let agent = self.scheduler.get_agent(slot_id).await?;

        self.mailbox
            .write_with_files(
                &self.team.id,
                slot_id,
                "user",
                MailboxMessageType::Message,
                content,
                None,
                files.as_deref(),
            )
            .await?;

        // Persist the user message as a right bubble in the agent's conversation
        // so the teammate panel shows what the user said.
        if let Some(svc) = self.service.upgrade() {
            let msg_id = ConversationService::mint_msg_id();
            let row = nomifun_db::models::MessageRow {
                id: msg_id.clone(),
                conversation_id: parse_conv_id(&agent.conversation_id)?,
                msg_id: Some(msg_id),
                r#type: "text".into(),
                content: serde_json::json!({ "content": content }).to_string(),
                position: Some("right".into()),
                status: Some("finish".into()),
                hidden: false,
                created_at: nomifun_common::now_ms(),
            };
            if let Err(e) = svc.conversation_service_ref().insert_raw_message(&row).await {
                warn!(
                    team_id = %self.team.id,
                    slot_id,
                    error = %e,
                    "failed to persist user right bubble (non-fatal)"
                );
            }
        }

        self.try_wake(slot_id, files).await;
        Ok(())
    }

    /// Signal an agent's event loop to process its mailbox.
    ///
    /// In the event-loop model, all wake logic lives inside the per-agent
    /// event loop. This method is the single trigger point used by:
    /// - `send_message` / `send_message_to_agent` (user messages)
    /// - MCP `team_send_message` / `team_shutdown_agent` (agent-to-agent)
    /// - `spawn_agent` (welcome message)
    /// - `on_agent_finish` cascade (all-settled → leader)
    ///
    /// When an event loop is registered for the slot, it just notifies it.
    /// When no loop exists (unit tests, race during spawn), falls back to
    /// a direct inline send so messages are not silently dropped.
    pub(crate) async fn try_wake(&self, slot_id: &str, files: Option<Vec<String>>) {
        // Prefer the event loop path
        if self.event_loops.has(slot_id) {
            self.event_loops.notify(slot_id);
            return;
        }

        // Fallback: inline send (used in unit tests and during spawn race)
        self.try_wake_inline(slot_id, files).await;
    }

    /// Legacy inline wake implementation. Used as fallback when no event loop
    /// is registered for the slot.
    async fn try_wake_inline(&self, slot_id: &str, files: Option<Vec<String>>) {
        if !self.scheduler.acquire_wake_lock(slot_id) {
            return;
        }

        let input = match self.compute_wake_input(slot_id).await {
            Ok(Some(input)) => input,
            Ok(None) => {
                self.scheduler.release_wake_lock(slot_id);
                return;
            }
            Err(err) => {
                warn!(
                    team_id = %self.team.id,
                    slot_id,
                    error = %err,
                    "try_wake_inline: compute_wake_input failed"
                );
                self.scheduler.release_wake_lock(slot_id);
                return;
            }
        };

        if !input.should_send {
            self.scheduler.release_wake_lock(slot_id);
            return;
        }

        self.mirror_unread_to_conversation(&input).await;

        let handle = if let Some(h) = self.task_manager.get_task(&input.conversation_id) {
            h
        } else {
            self.scheduler.release_wake_lock(slot_id);
            return;
        };

        if handle.status() == Some(nomifun_common::ConversationStatus::Running) {
            self.scheduler.release_wake_lock(slot_id);
            return;
        }

        let _ = self.scheduler.set_status(slot_id, TeammateStatus::Working).await;

        let msg_id = ConversationService::mint_msg_id();
        let data = SendMessageData {
            content: input.first_message,
            msg_id,
            files: files.unwrap_or_default(),
            inject_skills: Vec::new(),
            origin: None,
        };

        if let Err(err) = handle.send_message(data).await {
            warn!(
                team_id = %self.team.id,
                slot_id,
                error = %err,
                "try_wake_inline: send_message failed"
            );
            let _ = self.scheduler.set_status(slot_id, TeammateStatus::Idle).await;
            self.scheduler.release_wake_lock(slot_id);
            return;
        }

        let msg_ids: Vec<i64> = input.unread.iter().map(|m| m.id).collect();
        if !msg_ids.is_empty() {
            let _ = self.mailbox.mark_read_batch(&msg_ids).await;
        }

        self.scheduler.release_wake_lock(slot_id);
    }

    /// Mirror each non-user mailbox row into the target agent's conversation
    /// as a left bubble so the UI shows "who said what" when the user opens
    /// a teammate's chat panel.
    ///
    /// Skipped for:
    /// - Leader wakes: the mailbox content is embedded inside the role prompt
    ///   sent to the leader directly; duplicating it as a bubble would clutter
    ///   the leader's own thread (Nomi parity: `agent.role !== 'leader'`).
    /// - `from_agent_id == "user"`: user-originated messages are already
    ///   written to the conversation by the standard user-send path, and we
    ///   must not double-write them.
    /// - Test/unit contexts where `TeamSession::service` is a dangling
    ///   `Weak` (no conversation service reachable).
    ///
    /// Failures per-message are logged and swallowed — the mailbox rows are
    /// already marked read, and we never let a conversation-write failure
    /// block the wake itself.
    pub(crate) async fn mirror_unread_to_conversation(&self, input: &WakeInput) {
        if input.unread.is_empty() {
            return;
        }
        if matches!(input.agent_role, TeammateRole::Lead) {
            return;
        }
        let Some(service) = self.service.upgrade() else {
            return;
        };
        // Option A: the WakeInput holds a String id; the MessageRow FK is i64.
        // Bridge once; skip mirroring (non-fatal) if it is not a valid integer.
        let Ok(conversation_id_i64) = parse_conv_id(&input.conversation_id) else {
            warn!(
                team_id = %self.team.id,
                conversation_id = %input.conversation_id,
                "mirror_unread_to_conversation: invalid conversation id (skipped)"
            );
            return;
        };
        let conversation_service = service.conversation_service_ref();
        let agents = self.scheduler.list_agents().await;
        let total = input.unread.len();

        for msg in &input.unread {
            if msg.from_agent_id == "user" {
                continue;
            }
            let sender = agents.iter().find(|a| a.slot_id == msg.from_agent_id);
            let sender_name = sender
                .map(|a| a.name.clone())
                .unwrap_or_else(|| msg.from_agent_id.clone());
            let sender_backend = sender.map(|a| a.backend.clone());
            let sender_conv_id = sender.map(|a| a.conversation_id.clone());
            let display_content = if total > 1 {
                format!("[{sender_name}] {}", msg.content)
            } else {
                msg.content.clone()
            };
            let msg_id = ConversationService::mint_msg_id();
            let content_json = serde_json::json!({
                "content": display_content,
                "teammate_message": true,
                "sender_name": sender_name,
                "sender_backend": sender_backend,
                "sender_conversation_id": sender_conv_id,
            })
            .to_string();
            let row = nomifun_db::models::MessageRow {
                id: msg_id.clone(),
                conversation_id: conversation_id_i64,
                msg_id: Some(msg_id.clone()),
                r#type: "text".into(),
                content: content_json,
                position: Some("left".into()),
                status: Some("finish".into()),
                hidden: false,
                created_at: nomifun_common::now_ms(),
            };
            if let Err(err) = conversation_service.insert_raw_message(&row).await {
                warn!(
                    team_id = %self.team.id,
                    conversation_id = %input.conversation_id,
                    from = %msg.from_agent_id,
                    error = %err,
                    "mirror_unread_to_conversation: insert_raw_message failed (non-fatal)"
                );
                continue;
            }

            // Broadcast so the frontend can render the bubble in real-time
            // without a full message reload. The msg_id is included for
            // deduplication against the DB-persisted row.
            let ws_payload = serde_json::json!({
                "conversation_id": input.conversation_id,
                "msg_id": msg_id,
                "content": display_content,
                "from_slot_id": msg.from_agent_id,
                "from_name": sender_name,
                "teammate_message": true,
                "sender_backend": sender_backend,
            });
            let event = nomifun_api_types::WebSocketMessage::new("team.teammate.message", ws_payload);
            self.broadcaster.broadcast(event);
        }
    }

    pub async fn add_agent(&self, agent: &TeamAgent) {
        self.scheduler.add_agent(agent).await;
    }

    pub async fn remove_agent(&self, slot_id: &str) -> Result<(), TeamError> {
        self.event_loops.remove(slot_id);
        let conversation_id = self.scheduler.remove_agent(slot_id).await?;
        if let Some(conv_id) = conversation_id
            && let Err(e) = self.task_manager.kill(&conv_id, Some(AgentKillReason::TeamDeleted))
        {
            warn!(
                team_id = %self.team.id,
                slot_id,
                conversation_id = %conv_id,
                error = %e,
                "remove_agent: task_manager.kill failed (non-fatal)"
            );
        }
        Ok(())
    }

    pub async fn rename_agent(&self, slot_id: &str, new_name: &str) -> Result<(), TeamError> {
        self.scheduler.rename_agent(slot_id, new_name).await
    }

    /// Spawn a new teammate at the Lead's request (backing of `team_spawn_agent`).
    ///
    /// Validation chain mirrors the phase1 interface contract:
    /// 1. Caller must exist and carry `TeammateRole::Lead`.
    /// 2. `name` is normalized and must not collide with any live agent.
    /// 3. `agent_type` (falling back to the caller's backend when unset) must
    ///    be in the spawn whitelist.
    ///
    /// On success, a new conversation is created, the agent slot is persisted
    /// into the team row, the MCP stdio config is written into the conversation
    /// extras, the agent task is launched, and a welcome message is dropped
    /// into the new mailbox so the first wake reaches the spawned teammate
    /// with its role prompt.
    pub async fn spawn_agent(&self, caller_slot_id: &str, req: SpawnAgentRequest) -> Result<TeamAgent, TeamError> {
        // Step 1: caller must be a Lead. MCP dispatch already gates by role,
        // but this method is exposed on TeamSession so every entry point
        // (including future direct service callers) re-checks.
        let caller = self.scheduler.get_agent(caller_slot_id).await?;
        if caller.role != TeammateRole::Lead {
            return Err(TeamError::LeaderOnly("spawn_agent".into()));
        }

        // Step 2: normalize + uniqueness check against live scheduler state.
        let requested_name = req.name.trim().to_owned();
        if requested_name.is_empty() {
            return Err(TeamError::InvalidRequest("spawn_agent.name must not be empty".into()));
        }
        let normalized = normalize_name(&requested_name);
        if normalized.is_empty() {
            return Err(TeamError::InvalidRequest(
                "spawn_agent.name is empty after normalization".into(),
            ));
        }
        let existing = self.scheduler.list_agents().await;
        if existing.iter().any(|a| normalize_name(&a.name) == normalized) {
            return Err(TeamError::DuplicateAgentName(requested_name));
        }

        // Step 3: backend capability check. Hard whitelist passes immediately;
        // otherwise query persisted agent_capabilities for MCP support.
        let backend = req
            .agent_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(caller.backend.as_str())
            .to_owned();
        if !crate::guide::capability::TEAM_CAPABLE_BACKENDS.contains(&backend.as_str()) {
            let capable = match self.service.upgrade() {
                Some(svc) => svc.is_backend_team_capable(&backend).await,
                None => false,
            };
            if !capable {
                return Err(TeamError::BackendNotAllowed(backend));
            }
        }

        // Step 4: DB side-effects (new conversation + persisted agent slot).
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| TeamError::InvalidRequest("spawn_agent requires a live TeamSessionService".into()))?;
        let model = match req.model.as_deref().filter(|m| !m.is_empty()) {
            Some(m) => m.to_owned(),
            None => service
                .default_model_for_backend(&backend)
                .await
                .unwrap_or_else(|| caller.model.clone()),
        };
        let new_agent = service
            .persist_spawned_agent(
                &self.team.id,
                &self.user_id,
                requested_name,
                backend,
                model,
                req.custom_agent_id.clone(),
            )
            .await?;

        // Step 5: attach to the in-memory scheduler so wake-from-lead finds
        // the new slot immediately.
        self.scheduler.add_agent(&new_agent).await;

        // Step 6: welcome message. The mailbox write is the source of truth —
        // if the wake never fires (e.g. warmup raced), the next caller-triggered
        // wake will still drain this entry.
        self.mailbox
            .write(
                &self.team.id,
                &new_agent.slot_id,
                caller_slot_id,
                MailboxMessageType::Message,
                "You have been spawned as a teammate. Read your mailbox and wait for instructions.",
                None,
            )
            .await?;

        // Step 7: attach the CLI process and register the finish subscriber
        // in a background task. This involves spawning the CLI process and
        // completing the ACP protocol handshake, which can take significant
        // time (10-30s). Running it asynchronously ensures `spawn_agent`
        // returns promptly so the MCP tool call completes without blocking
        // the leader's connection loop.
        {
            let team_id = self.team.id.clone();
            let user_id = self.user_id.clone();
            let agent_clone = new_agent.clone();
            let mcp_stdio_cfg = self.mcp_stdio_config(&new_agent.slot_id);
            let task_manager = self.task_manager.clone();
            tokio::spawn(async move {
                // Push the team MCP stdio config into the new conversation's
                // extras, then kill + rebuild the agent task so the freshly
                // spawned process boots with the MCP handshake pointing at
                // our session.
                if let Err(err) = Self::attach_spawned_agent_process_bg(
                    &service,
                    &agent_clone,
                    mcp_stdio_cfg,
                    &user_id,
                    &task_manager,
                )
                .await
                {
                    warn!(
                        team_id = %team_id,
                        slot_id = %agent_clone.slot_id,
                        error = %err,
                        "failed to attach spawned agent process; agent is persisted but not yet running"
                    );
                }

                // Register the event loop for the newly spawned agent.
                service.register_event_loop(&team_id, &agent_clone.slot_id);

                // Notify the event loop to drain the welcome message.
                if let Err(e) = service.wake_agent_in_session(&team_id, &agent_clone.slot_id).await {
                    warn!(
                        team_id = %team_id,
                        slot_id = %agent_clone.slot_id,
                        error = %e,
                        "wake after spawn process ready failed (non-fatal; mailbox retained)"
                    );
                }
            });
        }

        Ok(new_agent)
    }

    /// Persist the team MCP stdio config into the spawned agent's conversation
    /// row, then kill any pre-existing task and warm up the new one.
    ///
    /// This is a static helper suitable for use inside `tokio::spawn` (no
    /// `&self` borrow). The caller passes all necessary context by value.
    async fn attach_spawned_agent_process_bg(
        service: &TeamSessionService,
        agent: &TeamAgent,
        mcp_stdio_cfg: crate::mcp::TeamMcpStdioConfig,
        user_id: &str,
        task_manager: &Arc<dyn IWorkerTaskManager>,
    ) -> Result<(), TeamError> {
        let patch = serde_json::json!({
            "team_mcp_stdio_config": mcp_stdio_cfg,
            "session_mode": resolve_full_auto_mode(&agent.backend),
        });

        service
            .conversation_service_ref()
            .update_extra(&agent.conversation_id, patch)
            .await
            .map_err(|e| {
                TeamError::InvalidRequest(format!(
                    "failed to persist team_mcp_stdio_config for {}: {e}",
                    agent.slot_id
                ))
            })?;

        let _ = task_manager.kill(&agent.conversation_id, Some(AgentKillReason::TeamMcpRebuild));

        service
            .conversation_service_ref()
            .warmup(user_id, &agent.conversation_id, task_manager)
            .await
            .map_err(|e| {
                TeamError::InvalidRequest(format!(
                    "failed to warm up spawned agent {}: {e}",
                    agent.conversation_id
                ))
            })?;

        Ok(())
    }

    pub fn stop(&self) {
        info!(team_id = %self.team.id, "TeamSession stopping");
        self.mcp_server.stop();
    }

    pub fn mailbox(&self) -> &Arc<Mailbox> {
        &self.mailbox
    }

    pub fn task_board(&self) -> &Arc<TaskBoard> {
        &self.task_board
    }
}

/// Remove `[SYSTEM NOTE: ...]` blocks from a message so they don't appear in
/// user-visible chat bubbles. The full content (with notes) is still delivered
/// to the agent via the mailbox/wake payload.
fn strip_system_notes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[SYSTEM NOTE:") {
        result.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find(']') {
            rest = &rest[start + end + 1..];
        } else {
            rest = &rest[start..];
            break;
        }
    }
    result.push_str(rest);
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::MockTeamRepo;
    use crate::types::{Team, TeamAgent, TeammateRole};
    use nomifun_ai_agent::agent_task::{AgentInstance, IAgentTask, IMockAgent};
    use nomifun_ai_agent::protocol::events::AgentStreamEvent;
    use nomifun_ai_agent::shared_kernel::approval_key;
    use nomifun_ai_agent::types::BuildTaskOptions;
    use nomifun_ai_agent::types::SendMessageData;
    use nomifun_api_types::{AgentModeResponse, WebSocketMessage};
    use nomifun_common::{AgentKillReason, AgentType, AppError, Confirmation, ConversationStatus, TimestampMs, now_ms};
    use std::sync::{Arc, Mutex};
    use tokio::sync::broadcast;

    struct NullBroadcaster;
    impl EventBroadcaster for NullBroadcaster {
        fn broadcast(&self, _msg: WebSocketMessage<serde_json::Value>) {}
    }

    /// RecordingBroadcaster used by the D29d-1 ratification test below to
    /// assert that `team.agent.spawned` is *not* emitted on failed spawns.
    #[derive(Default)]
    struct RecordingBroadcaster {
        events: Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
    }

    impl RecordingBroadcaster {
        fn new() -> Self {
            Self::default()
        }

        fn names(&self) -> Vec<String> {
            self.events.lock().unwrap().iter().map(|e| e.name.clone()).collect()
        }
    }

    impl EventBroadcaster for RecordingBroadcaster {
        fn broadcast(&self, msg: WebSocketMessage<serde_json::Value>) {
            self.events.lock().unwrap().push(msg);
        }
    }

    fn backend_path() -> Arc<PathBuf> {
        Arc::new(PathBuf::from("/tmp/nomicore-test"))
    }

    /// Mock agent whose `send_message` pushes the received payload into a
    /// shared log, optionally failing with a configurable error.
    struct RecordingAgent {
        conversation_id: String,
        sent: Arc<Mutex<Vec<SendMessageData>>>,
        fail_with: Option<String>,
        event_tx: broadcast::Sender<AgentStreamEvent>,
    }

    impl RecordingAgent {
        fn new(conversation_id: &str, sent: Arc<Mutex<Vec<SendMessageData>>>, fail_with: Option<String>) -> Self {
            let (event_tx, _) = broadcast::channel(4);
            Self {
                conversation_id: conversation_id.into(),
                sent,
                fail_with,
                event_tx,
            }
        }
    }

    #[async_trait::async_trait]
    impl IAgentTask for RecordingAgent {
        fn agent_type(&self) -> AgentType {
            AgentType::Acp
        }
        fn conversation_id(&self) -> &str {
            &self.conversation_id
        }
        fn workspace(&self) -> &str {
            "/tmp/ws"
        }
        fn status(&self) -> Option<ConversationStatus> {
            None
        }
        fn last_activity_at(&self) -> TimestampMs {
            now_ms()
        }
        fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
            self.event_tx.subscribe()
        }
        async fn send_message(&self, data: SendMessageData) -> Result<(), nomifun_ai_agent::AgentSendError> {
            self.sent.lock().unwrap().push(data);
            match &self.fail_with {
                Some(msg) => Err(nomifun_ai_agent::AgentSendError::from_app_error(AppError::Internal(
                    msg.clone(),
                ))),
                None => Ok(()),
            }
        }
        async fn cancel(&self) -> Result<(), AppError> {
            Ok(())
        }
        fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IMockAgent for RecordingAgent {
        fn get_confirmations(&self) -> Vec<Confirmation> {
            Vec::new()
        }
        fn check_approval(&self, action: &str, command_type: Option<&str>) -> bool {
            let _ = approval_key(Some(action), command_type);
            false
        }
        async fn mode(&self) -> Result<AgentModeResponse, AppError> {
            Ok(AgentModeResponse {
                mode: "default".into(),
                initialized: false,
            })
        }
    }

    /// In-memory stub for [`IWorkerTaskManager`]. Only `get_task` is
    /// exercised by D7b; the other methods are unreachable in these tests
    /// and panic to surface drift early.
    struct StubTaskManager {
        tasks: Mutex<std::collections::HashMap<String, AgentInstance>>,
        kill_calls: Mutex<Vec<(String, Option<AgentKillReason>)>>,
        kill_error: Option<String>,
    }

    impl StubTaskManager {
        fn new() -> Self {
            Self {
                tasks: Mutex::new(std::collections::HashMap::new()),
                kill_calls: Mutex::new(Vec::new()),
                kill_error: None,
            }
        }

        /// Build a stub whose `kill` always fails with `AppError::NotFound` so
        /// tests can exercise the non-fatal kill branch in `remove_agent`.
        fn with_kill_error(msg: &str) -> Self {
            Self {
                tasks: Mutex::new(std::collections::HashMap::new()),
                kill_calls: Mutex::new(Vec::new()),
                kill_error: Some(msg.to_owned()),
            }
        }

        fn insert(&self, conv_id: &str, handle: AgentInstance) {
            self.tasks.lock().unwrap().insert(conv_id.into(), handle);
        }

        fn kill_calls(&self) -> Vec<(String, Option<AgentKillReason>)> {
            self.kill_calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl IWorkerTaskManager for StubTaskManager {
        fn get_task(&self, conversation_id: &str) -> Option<AgentInstance> {
            self.tasks.lock().unwrap().get(conversation_id).cloned()
        }
        async fn get_or_build_task(
            &self,
            _conversation_id: &str,
            _options: BuildTaskOptions,
        ) -> Result<AgentInstance, AppError> {
            panic!("get_or_build_task should not be called in D7b tests")
        }
        fn kill(&self, conversation_id: &str, reason: Option<AgentKillReason>) -> Result<(), AppError> {
            self.kill_calls
                .lock()
                .unwrap()
                .push((conversation_id.to_owned(), reason));
            if let Some(msg) = &self.kill_error {
                return Err(AppError::NotFound(msg.clone()));
            }
            Ok(())
        }
        fn kill_and_wait(
            &self,
            conversation_id: &str,
            reason: Option<AgentKillReason>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
            let _ = self.kill(conversation_id, reason);
            Box::pin(std::future::ready(()))
        }
        fn clear(&self) {}
        fn active_count(&self) -> usize {
            self.tasks.lock().unwrap().len()
        }
        fn collect_idle(&self, _idle_threshold_ms: TimestampMs) -> Vec<String> {
            Vec::new()
        }
    }

    /// Build a task_manager pre-populated with a [`RecordingAgent`] per
    /// conversation in `conv_ids`. `fail_with` — when set — makes
    /// `send_message` fail for every agent so tests can exercise the
    /// log-not-throw path.
    fn task_manager_with_agents(
        conv_ids: &[&str],
        fail_with: Option<String>,
    ) -> (Arc<dyn IWorkerTaskManager>, Arc<Mutex<Vec<SendMessageData>>>) {
        let sent: Arc<Mutex<Vec<SendMessageData>>> = Arc::new(Mutex::new(Vec::new()));
        let stub = StubTaskManager::new();
        for conv_id in conv_ids {
            let agent = AgentInstance::Mock(Arc::new(RecordingAgent::new(conv_id, sent.clone(), fail_with.clone())));
            stub.insert(conv_id, agent);
        }
        (Arc::new(stub), sent)
    }

    /// Empty task_manager — `get_task` returns `None` for every conversation.
    fn empty_task_manager() -> Arc<dyn IWorkerTaskManager> {
        Arc::new(StubTaskManager::new())
    }

    fn make_team() -> Team {
        Team {
            id: "t1".into(),
            name: "Test Team".into(),
            agents: vec![
                TeamAgent {
                    slot_id: "lead-1".into(),
                    name: "Lead".into(),
                    role: TeammateRole::Lead,
                    conversation_id: "c1".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                    status: None,
                    conversation_type: None,
                    cli_path: None,
                },
                TeamAgent {
                    slot_id: "worker-1".into(),
                    name: "Worker".into(),
                    role: TeammateRole::Teammate,
                    conversation_id: "c2".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                    status: None,
                    conversation_type: None,
                    cli_path: None,
                },
            ],
            lead_agent_id: Some("lead-1".into()),
            created_at: 1000,
            updated_at: 1000,
        }
    }

    async fn start_session() -> TeamSession {
        let repo: Arc<dyn ITeamRepository> = Arc::new(MockTeamRepo::new());
        let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
        TeamSession::start(
            make_team(),
            repo,
            broadcaster,
            backend_path(),
            empty_task_manager(),
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn start_and_stop() {
        let session = start_session().await;
        assert_eq!(session.team_id(), "t1");
        assert!(session.mcp_server.port() > 0);
        session.stop();
    }

    #[tokio::test]
    async fn mcp_stdio_config_for_agent() {
        let session = start_session().await;
        let config = session.mcp_stdio_config("lead-1");
        assert_eq!(config.team_id, "t1");
        assert_eq!(config.slot_id, "lead-1");
        assert_eq!(config.port, session.mcp_server.port());
        session.stop();
    }

    #[tokio::test]
    async fn stdio_spec_uses_fixed_name_and_binary_path() {
        let session = start_session().await;
        let spec = session.stdio_spec("lead-1");
        assert_eq!(spec.name, crate::mcp::TEAM_MCP_SERVER_NAME);
        assert_eq!(spec.command, "/tmp/nomicore-test");
        assert_eq!(spec.args, vec!["mcp-bridge".to_string()]);
        assert!(spec.env.iter().any(|(k, v)| k == "TEAM_AGENT_SLOT_ID" && v == "lead-1"));
        session.stop();
    }

    #[tokio::test]
    async fn send_message_writes_to_lead_mailbox() {
        let repo = Arc::new(MockTeamRepo::new());
        let repo_dyn: Arc<dyn ITeamRepository> = repo.clone();
        let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
        let session = TeamSession::start(
            make_team(),
            repo_dyn,
            broadcaster,
            backend_path(),
            empty_task_manager(),
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap();
        session.send_message("Hello team", None).await.unwrap();

        let state = repo.state.lock().unwrap();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].to_agent_id, "lead-1");
        assert_eq!(state.messages[0].from_agent_id, "user");
        assert_eq!(state.messages[0].content, "Hello team");
        session.stop();
    }

    #[tokio::test]
    async fn send_message_to_agent_writes_to_mailbox() {
        let repo = Arc::new(MockTeamRepo::new());
        let repo_dyn: Arc<dyn ITeamRepository> = repo.clone();
        let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
        let session = TeamSession::start(
            make_team(),
            repo_dyn,
            broadcaster,
            backend_path(),
            empty_task_manager(),
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap();
        session
            .send_message_to_agent("worker-1", "Do this task", None)
            .await
            .unwrap();

        let state = repo.state.lock().unwrap();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].to_agent_id, "worker-1");
        assert_eq!(state.messages[0].content, "Do this task");
        session.stop();
    }

    #[tokio::test]
    async fn send_message_to_unknown_agent_returns_error() {
        let session = start_session().await;
        let result = session.send_message_to_agent("nonexistent", "Hello", None).await;
        assert!(result.is_err());
        session.stop();
    }

    #[tokio::test]
    async fn add_and_remove_agent() {
        let session = start_session().await;

        let new_agent = TeamAgent {
            slot_id: "new-1".into(),
            name: "NewAgent".into(),
            role: TeammateRole::Teammate,
            conversation_id: "c3".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        };
        session.add_agent(&new_agent).await;

        let agents = session.scheduler.list_agents().await;
        assert_eq!(agents.len(), 3);

        session.remove_agent("new-1").await.unwrap();
        let agents = session.scheduler.list_agents().await;
        assert_eq!(agents.len(), 2);

        session.stop();
    }

    // -- W5-D30d-1: remove_agent kills the agent process ---------------------

    #[tokio::test]
    async fn remove_agent_calls_task_manager_kill() {
        let repo: Arc<dyn ITeamRepository> = Arc::new(MockTeamRepo::new());
        let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
        let stub = Arc::new(StubTaskManager::new());
        let stub_dyn: Arc<dyn IWorkerTaskManager> = stub.clone();
        let session = TeamSession::start(
            make_team(),
            repo,
            broadcaster,
            backend_path(),
            stub_dyn,
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap();

        session.remove_agent("worker-1").await.unwrap();

        let calls = stub.kill_calls();
        assert_eq!(calls.len(), 1, "kill invoked exactly once");
        assert_eq!(calls[0].0, "c2", "kill targets removed slot's conversation_id");
        assert!(
            matches!(calls[0].1, Some(AgentKillReason::TeamDeleted)),
            "kill reason carries AgentKillReason"
        );
        session.stop();
    }

    #[tokio::test]
    async fn remove_agent_is_non_fatal_when_kill_fails() {
        let repo: Arc<dyn ITeamRepository> = Arc::new(MockTeamRepo::new());
        let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
        let stub = Arc::new(StubTaskManager::with_kill_error("task not found"));
        let stub_dyn: Arc<dyn IWorkerTaskManager> = stub.clone();
        let session = TeamSession::start(
            make_team(),
            repo,
            broadcaster,
            backend_path(),
            stub_dyn,
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap();

        // kill returns Err(AppError::NotFound) but remove_agent must still
        // succeed — NotFound means the worker already died, which is OK.
        session.remove_agent("worker-1").await.unwrap();

        let agents = session.scheduler.list_agents().await;
        assert_eq!(agents.len(), 1, "slot still removed even after kill failure");
        assert_eq!(stub.kill_calls().len(), 1);
        session.stop();
    }

    #[tokio::test]
    async fn rename_agent_in_session() {
        let session = start_session().await;
        session.rename_agent("worker-1", "Senior Worker").await.unwrap();

        let agent = session.scheduler.get_agent("worker-1").await.unwrap();
        assert_eq!(agent.name, "Senior Worker");

        session.stop();
    }

    #[tokio::test]
    async fn rename_unknown_agent_returns_error() {
        let session = start_session().await;
        let result = session.rename_agent("nonexistent", "X").await;
        assert!(result.is_err());
        session.stop();
    }

    #[tokio::test]
    async fn rename_agent_rejects_duplicate_in_session() {
        let session = start_session().await;
        let agents = session.scheduler.list_agents().await;
        let lead_name = agents.iter().find(|a| a.slot_id == "lead-1").unwrap().name.clone();

        // Rename worker-1 to the lead's name — should collide.
        let result = session.rename_agent("worker-1", &lead_name).await;
        assert!(result.is_err());

        session.stop();
    }

    // -- spawn_agent helpers + guard tests -----------------------------------

    fn sample_spawn_req() -> SpawnAgentRequest {
        SpawnAgentRequest {
            name: "Helper".into(),
            agent_type: None,
            custom_agent_id: None,
            model: None,
        }
    }

    #[tokio::test]
    async fn spawn_agent_rejects_unknown_caller() {
        let session = start_session().await;
        let result = session.spawn_agent("nonexistent", sample_spawn_req()).await;
        assert!(
            matches!(&result, Err(TeamError::AgentNotFound(_))),
            "unknown caller must surface AgentNotFound, got {result:?}"
        );
        session.stop();
    }

    // -- D7a new method tests ------------------------------------------------

    #[tokio::test]
    async fn compute_wake_input_cold_start_injects_lead_role_prompt() {
        let session = start_session().await;
        // Seed one unread message. `send_message` flips status to Working —
        // that is the post-send path; here we want to exercise cold-start
        // detection, so write directly to the mailbox instead.
        session
            .mailbox
            .write("t1", "lead-1", "user", MailboxMessageType::Message, "kick off", None)
            .await
            .unwrap();

        let input = session.compute_wake_input("lead-1").await.unwrap().expect("WakeInput");

        assert_eq!(input.conversation_id, "c1");
        assert!(input.should_send);
        assert!(
            input.first_message.contains("You are the Team Leader"),
            "expected lead role prompt, got: {}",
            input.first_message
        );
        assert!(input.first_message.contains("kick off"));
        session.stop();
    }

    #[tokio::test]
    async fn compute_wake_input_cold_start_injects_teammate_role_prompt() {
        let session = start_session().await;
        session
            .mailbox
            .write("t1", "worker-1", "user", MailboxMessageType::Message, "do X", None)
            .await
            .unwrap();

        let input = session
            .compute_wake_input("worker-1")
            .await
            .unwrap()
            .expect("WakeInput");

        assert!(
            input.first_message.contains("Teammate Agent"),
            "expected teammate role prompt, got: {}",
            input.first_message
        );
        assert!(input.first_message.contains("do X"));
        session.stop();
    }

    #[tokio::test]
    async fn compute_wake_input_warm_agent_skips_role_prompt() {
        let session = start_session().await;
        // Exit cold-start by setting a status once; any non-Error status
        // means the scheduler has seen this agent before.
        session
            .scheduler
            .set_status("lead-1", TeammateStatus::Idle)
            .await
            .unwrap();
        session
            .mailbox
            .write("t1", "lead-1", "user", MailboxMessageType::Message, "follow-up", None)
            .await
            .unwrap();

        let input = session.compute_wake_input("lead-1").await.unwrap().expect("WakeInput");

        assert!(input.should_send);
        assert!(
            !input.first_message.contains("Lead Agent of team"),
            "should not re-inject role prompt, got: {}",
            input.first_message
        );
        assert!(input.first_message.contains("follow-up"));
        session.stop();
    }

    #[tokio::test]
    async fn compute_wake_input_empty_mailbox_should_not_send() {
        let session = start_session().await;

        let input = session.compute_wake_input("lead-1").await.unwrap().expect("WakeInput");

        assert!(!input.should_send);
        session.stop();
    }

    #[tokio::test]
    async fn compute_wake_input_returns_unread_rows_and_role_for_teammate() {
        let session = start_session().await;
        session
            .mailbox
            .write(
                "t1",
                "worker-1",
                "lead-1",
                MailboxMessageType::Message,
                "from lead",
                None,
            )
            .await
            .unwrap();
        session
            .mailbox
            .write("t1", "worker-1", "user", MailboxMessageType::Message, "from user", None)
            .await
            .unwrap();

        let input = session
            .compute_wake_input("worker-1")
            .await
            .unwrap()
            .expect("WakeInput");

        assert_eq!(input.unread.len(), 2);
        assert!(matches!(input.agent_role, TeammateRole::Teammate));
        assert!(input.unread.iter().any(|m| m.from_agent_id == "lead-1"));
        assert!(input.unread.iter().any(|m| m.from_agent_id == "user"));
        session.stop();
    }

    #[tokio::test]
    async fn compute_wake_input_returns_lead_role() {
        let session = start_session().await;
        session
            .mailbox
            .write("t1", "lead-1", "user", MailboxMessageType::Message, "hi lead", None)
            .await
            .unwrap();

        let input = session.compute_wake_input("lead-1").await.unwrap().expect("WakeInput");

        assert!(matches!(input.agent_role, TeammateRole::Lead));
        assert_eq!(input.unread.len(), 1);
        session.stop();
    }

    #[tokio::test]
    async fn mirror_unread_to_conversation_is_noop_for_leader() {
        let session = start_session().await;
        session
            .mailbox
            .write(
                "t1",
                "lead-1",
                "worker-1",
                MailboxMessageType::Message,
                "lead-gets-this",
                None,
            )
            .await
            .unwrap();

        let input = session.compute_wake_input("lead-1").await.unwrap().expect("WakeInput");

        // The service `Weak` is dangling in unit tests, but the leader short-circuit
        // must hit before any upgrade — this call must not panic.
        session.mirror_unread_to_conversation(&input).await;
        session.stop();
    }

    #[tokio::test]
    async fn mirror_unread_to_conversation_skips_when_service_weak_is_dangling() {
        let session = start_session().await;
        session
            .mailbox
            .write("t1", "worker-1", "lead-1", MailboxMessageType::Message, "do it", None)
            .await
            .unwrap();

        let input = session
            .compute_wake_input("worker-1")
            .await
            .unwrap()
            .expect("WakeInput");

        // In unit tests, `service` is a dangling Weak — the mirror helper must
        // skip gracefully (no panic, no broadcast), leaving the wake path to
        // still forward `first_message` to the agent.
        session.mirror_unread_to_conversation(&input).await;
        session.stop();
    }

    #[tokio::test]
    async fn on_agent_finish_marks_idle_and_returns_lead_when_all_settled() {
        let session = start_session().await;

        // Worker is Working; on finish → mark idle → since the lead is the
        // only remaining non-idle member (actually also idle), all-idle
        // check returns the lead slot_id.
        session
            .scheduler
            .set_status("worker-1", TeammateStatus::Working)
            .await
            .unwrap();

        let result = session.on_agent_finish("c2", false).await.unwrap();
        assert_eq!(result.as_deref(), Some("lead-1"));

        let status = session.scheduler.get_status("worker-1").await.unwrap();
        assert_eq!(status, TeammateStatus::Idle);
        session.stop();
    }

    #[tokio::test]
    async fn on_agent_finish_lead_returns_none() {
        let session = start_session().await;
        session
            .scheduler
            .set_status("lead-1", TeammateStatus::Working)
            .await
            .unwrap();

        let result = session.on_agent_finish("c1", false).await.unwrap();
        assert!(result.is_none());
        session.stop();
    }

    #[tokio::test]
    async fn on_agent_finish_unknown_conversation_returns_error() {
        let session = start_session().await;
        let result = session.on_agent_finish("nope", false).await;
        assert!(result.is_err());
        session.stop();
    }

    // -- D7b wake-path tests -------------------------------------------------

    async fn start_session_with(task_manager: Arc<dyn IWorkerTaskManager>) -> (TeamSession, Arc<MockTeamRepo>) {
        let repo = Arc::new(MockTeamRepo::new());
        let repo_dyn: Arc<dyn ITeamRepository> = repo.clone();
        let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
        let session = TeamSession::start(
            make_team(),
            repo_dyn,
            broadcaster,
            backend_path(),
            task_manager,
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap();
        (session, repo)
    }

    #[tokio::test]
    async fn send_message_forwards_files_to_task_manager() {
        let (task_manager, sent) = task_manager_with_agents(&["c1"], None);
        let (session, _repo) = start_session_with(task_manager).await;

        session
            .send_message("Hello", Some(vec!["/tmp/a.txt".into(), "/tmp/b.txt".into()]))
            .await
            .unwrap();

        let log = sent.lock().unwrap();
        assert_eq!(log.len(), 1, "expected exactly one send_message call");
        assert_eq!(log[0].files, vec!["/tmp/a.txt", "/tmp/b.txt"]);
        assert!(log[0].content.contains("Hello"));
        assert!(!log[0].msg_id.is_empty());
        session.stop();
    }

    #[tokio::test]
    async fn send_message_without_active_task_does_not_error() {
        // Empty task_manager → get_task returns None → log-not-throw: the
        // mailbox write must still succeed and the call must return Ok.
        let (session, repo) = start_session_with(empty_task_manager()).await;

        session
            .send_message("queued", None)
            .await
            .expect("send_message must return Ok even when no task is active");

        let state = repo.state.lock().unwrap();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "queued");
        session.stop();
    }

    #[tokio::test]
    async fn send_message_swallows_task_manager_send_failure() {
        // Agent present but send_message fails — D7b must log and return Ok
        // (P0#46). A propagated error would invite retries that double-write
        // the mailbox.
        let (task_manager, sent) = task_manager_with_agents(&["c1"], Some("boom".into()));
        let (session, _repo) = start_session_with(task_manager).await;

        session
            .send_message("payload", None)
            .await
            .expect("wake failure must be swallowed");

        // The attempt still reached the agent, so the sent log has one entry.
        assert_eq!(sent.lock().unwrap().len(), 1);
        session.stop();
    }

    #[tokio::test]
    async fn send_message_to_agent_targets_specific_conversation() {
        let (task_manager, sent) = task_manager_with_agents(&["c1", "c2"], None);
        let (session, _repo) = start_session_with(task_manager).await;

        session
            .send_message_to_agent("worker-1", "do X", Some(vec!["/tmp/x.md".into()]))
            .await
            .unwrap();

        let log = sent.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].files, vec!["/tmp/x.md"]);
        assert!(log[0].content.contains("do X"));
        session.stop();
    }

    #[tokio::test]
    async fn send_message_with_empty_content_still_wakes() {
        // compute_wake_input returns should_send=true whenever the mailbox has
        // unread entries, regardless of content. Ensure the wake still fires
        // when a caller passes an empty string.
        let (task_manager, sent) = task_manager_with_agents(&["c1"], None);
        let (session, _repo) = start_session_with(task_manager).await;

        session.send_message("", None).await.unwrap();

        assert_eq!(sent.lock().unwrap().len(), 1);
        session.stop();
    }

    async fn start_session_with_lead_backend(backend: &str) -> TeamSession {
        let mut team = make_team();
        team.agents[0].backend = backend.to_string();
        let repo: Arc<dyn ITeamRepository> = Arc::new(MockTeamRepo::new());
        let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
        TeamSession::start(
            team,
            repo,
            broadcaster,
            backend_path(),
            empty_task_manager(),
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap()
    }

    async fn start_session_with_recorder(backend: &str) -> (TeamSession, Arc<RecordingBroadcaster>) {
        let mut team = make_team();
        team.agents[0].backend = backend.to_string();
        let repo: Arc<dyn ITeamRepository> = Arc::new(MockTeamRepo::new());
        let recorder = Arc::new(RecordingBroadcaster::new());
        let broadcaster: Arc<dyn EventBroadcaster> = recorder.clone();
        let session = TeamSession::start(
            team,
            repo,
            broadcaster,
            backend_path(),
            empty_task_manager(),
            "user-test".into(),
            Weak::<TeamSessionService>::new(),
        )
        .await
        .unwrap();
        (session, recorder)
    }

    fn spawn_req(agent_type: Option<&str>) -> SpawnAgentRequest {
        SpawnAgentRequest {
            name: "Helper".into(),
            agent_type: agent_type.map(str::to_owned),
            custom_agent_id: None,
            model: None,
        }
    }

    /// After all guards pass, the unit-test sessions have a null `service`
    /// Weak — so the spawn path must bail with InvalidRequest instead of
    /// panicking. This is the "validation passed, DB step not reachable"
    /// shape exercised below.
    fn assert_reached_db_step(err: TeamError) {
        match err {
            TeamError::InvalidRequest(msg) if msg.contains("live TeamSessionService") => {}
            other => panic!("expected service-unavailable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn spawn_agent_accepts_claude_backend() {
        let session = start_session_with_lead_backend("claude").await;
        let err = session
            .spawn_agent("lead-1", spawn_req(Some("claude")))
            .await
            .expect_err("unit test has no service wire; spawn stops at DB step");
        assert_reached_db_step(err);
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_accepts_codex_backend() {
        let session = start_session_with_lead_backend("claude").await;
        let err = session
            .spawn_agent("lead-1", spawn_req(Some("codex")))
            .await
            .expect_err("unit test has no service wire; spawn stops at DB step");
        assert_reached_db_step(err);
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_rejects_unknown_backend() {
        let session = start_session_with_lead_backend("claude").await;
        let err = session
            .spawn_agent("lead-1", spawn_req(Some("unknown_backend")))
            .await
            .expect_err("unknown backend must be rejected");
        assert!(
            matches!(&err, TeamError::BackendNotAllowed(b) if b == "unknown_backend"),
            "expected BackendNotAllowed(\"unknown_backend\"), got {err:?}"
        );
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_inherits_caller_backend_when_unspecified() {
        // No agent_type on the request -> must fall back to the caller's
        // backend ("claude"), which passes the whitelist.
        let session = start_session_with_lead_backend("claude").await;
        let err = session
            .spawn_agent("lead-1", spawn_req(None))
            .await
            .expect_err("unit test has no service wire; spawn stops at DB step");
        assert_reached_db_step(err);
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_rejects_when_inherited_backend_not_whitelisted() {
        // Caller's backend is "acp" (not whitelisted). With no explicit
        // agent_type, the inherited backend must be rejected.
        let session = start_session_with_lead_backend("acp").await;
        let err = session
            .spawn_agent("lead-1", spawn_req(None))
            .await
            .expect_err("non-whitelisted inherited backend must be rejected");
        assert!(
            matches!(&err, TeamError::BackendNotAllowed(b) if b == "acp"),
            "expected BackendNotAllowed(\"acp\"), got {err:?}"
        );
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_rejects_non_lead_caller() {
        let session = start_session_with_lead_backend("claude").await;
        let err = session
            .spawn_agent("worker-1", spawn_req(Some("claude")))
            .await
            .expect_err("non-lead caller must be rejected");
        assert!(
            matches!(&err, TeamError::LeaderOnly(what) if what == "spawn_agent"),
            "expected LeaderOnly(\"spawn_agent\"), got {err:?}"
        );
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_rejects_duplicate_name() {
        let session = start_session_with_lead_backend("claude").await;
        // The seeded team already has an agent named "Worker". Case + trim
        // normalization means "  worker " collides.
        let mut req = spawn_req(Some("claude"));
        req.name = "  worker ".into();
        let err = session
            .spawn_agent("lead-1", req)
            .await
            .expect_err("duplicate name must be rejected");
        assert!(
            matches!(&err, TeamError::DuplicateAgentName(_)),
            "expected DuplicateAgentName, got {err:?}"
        );
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_rejects_empty_name() {
        let session = start_session_with_lead_backend("claude").await;
        let mut req = spawn_req(Some("claude"));
        req.name = "   ".into();
        let err = session
            .spawn_agent("lead-1", req)
            .await
            .expect_err("empty name must be rejected");
        assert!(
            matches!(&err, TeamError::InvalidRequest(msg) if msg.contains("empty")),
            "expected InvalidRequest about empty name, got {err:?}"
        );
        session.stop();
    }

    // -- W5-D29d-1 ratification: spawn emit-order contract ------------------
    //
    // The success-path emission of `team.agent.spawned` is exercised by
    // `scheduler::tests::add_agent_broadcasts_spawned_event` — `spawn_agent`
    // reaches that emission via `scheduler.add_agent(&new_agent)` after
    // `persist_spawned_agent` returns. This ratification test locks the
    // *ordering* half of the contract: the event must NOT be published
    // before the DB step succeeds. If a future refactor hoists broadcast
    // above the persist/add_agent boundary (so the frontend sees a spawned
    // agent that never persisted), this test regresses.

    #[tokio::test]
    async fn spawn_agent_does_not_emit_before_db_step() {
        let (session, recorder) = start_session_with_recorder("claude").await;
        let err = session
            .spawn_agent("lead-1", spawn_req(Some("claude")))
            .await
            .expect_err("unit test has no service wire; spawn stops at DB step");
        assert_reached_db_step(err);
        assert!(
            !recorder.names().iter().any(|n| n == "team.agent.spawned"),
            "team.agent.spawned must not be emitted when spawn fails before add_agent; saw {:?}",
            recorder.names()
        );
        session.stop();
    }

    #[tokio::test]
    async fn spawn_agent_does_not_emit_on_guard_rejection() {
        let (session, recorder) = start_session_with_recorder("claude").await;
        let err = session
            .spawn_agent("worker-1", spawn_req(Some("claude")))
            .await
            .expect_err("non-lead caller must be rejected");
        assert!(matches!(&err, TeamError::LeaderOnly(what) if what == "spawn_agent"));
        assert!(
            !recorder.names().iter().any(|n| n == "team.agent.spawned"),
            "team.agent.spawned must not be emitted when guard rejects the caller; saw {:?}",
            recorder.names()
        );
        session.stop();
    }
}
