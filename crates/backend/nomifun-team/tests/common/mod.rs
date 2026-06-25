use nomifun_common::now_ms;
use nomifun_db::models::{MailboxMessageRow, TeamAgentRow, TeamRow, TeamTaskRow};
use nomifun_db::{DbError, ITeamRepository, UpdateTaskParams, UpdateTeamAgentParams, UpdateTeamParams};
use std::sync::Mutex;

/// In-memory backing store for [`MockTeamRepo`].
///
/// Mirrors the post-primary-key-redesign schema: `team_agents` and
/// `team_task_deps` are first-class tables (was `teams.agents` /
/// `team_tasks.blocked_by`/`blocks` JSON arrays), and `mailbox.id` is an
/// autoincrement `i64`.
#[derive(Default)]
pub struct MockState {
    pub teams: Vec<TeamRow>,
    pub team_agents: Vec<TeamAgentRow>,
    pub messages: Vec<MailboxMessageRow>,
    pub next_message_id: i64,
    pub tasks: Vec<TeamTaskRow>,
    /// Dependency edges: `(blocker_task_id, blocked_task_id)`.
    pub task_deps: Vec<(String, String)>,
}

pub struct MockTeamRepo {
    pub state: Mutex<MockState>,
}

impl Default for MockTeamRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTeamRepo {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MockState {
                next_message_id: 1,
                ..MockState::default()
            }),
        }
    }
}

#[async_trait::async_trait]
impl ITeamRepository for MockTeamRepo {
    // ── Team CRUD ───────────────────────────────────────────────────

    async fn create_team(&self, row: &TeamRow) -> Result<(), DbError> {
        self.state.lock().unwrap().teams.push(row.clone());
        Ok(())
    }
    async fn list_teams(&self) -> Result<Vec<TeamRow>, DbError> {
        Ok(self.state.lock().unwrap().teams.clone())
    }
    async fn get_team(&self, id: &str) -> Result<Option<TeamRow>, DbError> {
        Ok(self.state.lock().unwrap().teams.iter().find(|t| t.id == id).cloned())
    }
    async fn update_team(&self, id: &str, params: &UpdateTeamParams) -> Result<(), DbError> {
        let mut state = self.state.lock().unwrap();
        let team = state
            .teams
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| DbError::NotFound(id.to_owned()))?;
        if let Some(ref name) = params.name {
            team.name = name.clone();
        }
        if let Some(ref lead) = params.lead_agent_id {
            team.lead_agent_id = Some(lead.clone());
        }
        team.updated_at = now_ms();
        Ok(())
    }
    async fn delete_team(&self, id: &str) -> Result<(), DbError> {
        // Emulate the FK ON DELETE CASCADE chain.
        let mut state = self.state.lock().unwrap();
        state.teams.retain(|t| t.id != id);
        state.team_agents.retain(|a| a.team_id != id);
        state.messages.retain(|m| m.team_id != id);
        let task_ids: Vec<String> = state
            .tasks
            .iter()
            .filter(|t| t.team_id == id)
            .map(|t| t.id.clone())
            .collect();
        state.tasks.retain(|t| t.team_id != id);
        state
            .task_deps
            .retain(|(blocker, blocked)| !task_ids.contains(blocker) && !task_ids.contains(blocked));
        Ok(())
    }

    // ── Team agents (was teams.agents JSON array) ─────────────────────

    async fn create_team_agent(&self, row: &TeamAgentRow) -> Result<(), DbError> {
        self.state.lock().unwrap().team_agents.push(row.clone());
        Ok(())
    }
    async fn list_team_agents(&self, team_id: &str) -> Result<Vec<TeamAgentRow>, DbError> {
        let state = self.state.lock().unwrap();
        let mut agents: Vec<TeamAgentRow> = state
            .team_agents
            .iter()
            .filter(|a| a.team_id == team_id)
            .cloned()
            .collect();
        agents.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.slot_id.cmp(&b.slot_id)));
        Ok(agents)
    }
    async fn get_team_agent(&self, slot_id: &str) -> Result<Option<TeamAgentRow>, DbError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .team_agents
            .iter()
            .find(|a| a.slot_id == slot_id)
            .cloned())
    }
    async fn update_team_agent(&self, slot_id: &str, params: &UpdateTeamAgentParams) -> Result<(), DbError> {
        let mut state = self.state.lock().unwrap();
        let agent = state
            .team_agents
            .iter_mut()
            .find(|a| a.slot_id == slot_id)
            .ok_or_else(|| DbError::NotFound(slot_id.to_owned()))?;
        if let Some(ref v) = params.name {
            agent.name = v.clone();
        }
        if let Some(ref v) = params.role {
            agent.role = v.clone();
        }
        if let Some(ref v) = params.conversation_id {
            agent.conversation_id = Some(v.clone());
        }
        if let Some(ref v) = params.backend {
            agent.backend = v.clone();
        }
        if let Some(ref v) = params.model {
            agent.model = v.clone();
        }
        if let Some(ref v) = params.custom_agent_id {
            agent.custom_agent_id = Some(v.clone());
        }
        if let Some(ref v) = params.status {
            agent.status = Some(v.clone());
        }
        if let Some(ref v) = params.conversation_type {
            agent.conversation_type = Some(v.clone());
        }
        if let Some(ref v) = params.cli_path {
            agent.cli_path = Some(v.clone());
        }
        if let Some(v) = params.sort_order {
            agent.sort_order = v;
        }
        Ok(())
    }
    async fn rename_team_agent(&self, slot_id: &str, name: &str) -> Result<(), DbError> {
        let mut state = self.state.lock().unwrap();
        let agent = state
            .team_agents
            .iter_mut()
            .find(|a| a.slot_id == slot_id)
            .ok_or_else(|| DbError::NotFound(slot_id.to_owned()))?;
        agent.name = name.to_owned();
        Ok(())
    }
    async fn remove_team_agent(&self, slot_id: &str) -> Result<(), DbError> {
        self.state.lock().unwrap().team_agents.retain(|a| a.slot_id != slot_id);
        Ok(())
    }

    // ── Mailbox ─────────────────────────────────────────────────────

    async fn write_message(&self, row: &MailboxMessageRow) -> Result<i64, DbError> {
        let mut state = self.state.lock().unwrap();
        let id = state.next_message_id;
        state.next_message_id += 1;
        let mut stored = row.clone();
        stored.id = id;
        state.messages.push(stored);
        Ok(id)
    }

    async fn read_unread_and_mark(&self, team_id: &str, to_agent_id: &str) -> Result<Vec<MailboxMessageRow>, DbError> {
        let mut state = self.state.lock().unwrap();
        let mut result = vec![];
        for msg in &mut state.messages {
            if msg.team_id == team_id && msg.to_agent_id == to_agent_id && !msg.read {
                msg.read = true;
                result.push(msg.clone());
            }
        }
        Ok(result)
    }

    async fn peek_unread(&self, team_id: &str, to_agent_id: &str) -> Result<Vec<MailboxMessageRow>, DbError> {
        let state = self.state.lock().unwrap();
        let result = state
            .messages
            .iter()
            .filter(|m| m.team_id == team_id && m.to_agent_id == to_agent_id && !m.read)
            .cloned()
            .collect();
        Ok(result)
    }

    async fn mark_read_batch(&self, ids: &[i64]) -> Result<(), DbError> {
        let mut state = self.state.lock().unwrap();
        for msg in &mut state.messages {
            if ids.contains(&msg.id) {
                msg.read = true;
            }
        }
        Ok(())
    }

    async fn get_history(
        &self,
        team_id: &str,
        to_agent_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<MailboxMessageRow>, DbError> {
        let state = self.state.lock().unwrap();
        let iter = state
            .messages
            .iter()
            .filter(|m| m.team_id == team_id && m.to_agent_id == to_agent_id);
        let msgs: Vec<_> = match limit {
            Some(n) => iter.take(n as usize).cloned().collect(),
            None => iter.cloned().collect(),
        };
        Ok(msgs)
    }

    // ── Tasks ────────────────────────────────────────────────────────

    async fn create_task(&self, row: &TeamTaskRow) -> Result<(), DbError> {
        self.state.lock().unwrap().tasks.push(row.clone());
        Ok(())
    }

    async fn find_task_by_id(&self, team_id: &str, task_id: &str) -> Result<Option<TeamTaskRow>, DbError> {
        let state = self.state.lock().unwrap();
        let found = state
            .tasks
            .iter()
            .find(|t| t.team_id == team_id && t.id == task_id)
            .cloned();
        Ok(found)
    }

    async fn update_task(&self, task_id: &str, params: &UpdateTaskParams) -> Result<(), DbError> {
        let mut state = self.state.lock().unwrap();
        let task = state
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| DbError::NotFound(task_id.to_owned()))?;
        if let Some(ref s) = params.status {
            task.status = s.clone();
        }
        if let Some(ref d) = params.description {
            task.description = Some(d.clone());
        }
        if let Some(ref o) = params.owner {
            task.owner = Some(o.clone());
        }
        if let Some(ref m) = params.metadata {
            task.metadata = Some(m.clone());
        }
        task.updated_at = now_ms();
        Ok(())
    }

    async fn list_tasks(&self, team_id: &str) -> Result<Vec<TeamTaskRow>, DbError> {
        let state = self.state.lock().unwrap();
        let tasks = state.tasks.iter().filter(|t| t.team_id == team_id).cloned().collect();
        Ok(tasks)
    }

    // ── Task dependencies (was blocked_by/blocks JSON arrays) ─────────

    async fn add_task_dep(&self, blocker_task_id: &str, blocked_task_id: &str) -> Result<(), DbError> {
        let mut state = self.state.lock().unwrap();
        let edge = (blocker_task_id.to_owned(), blocked_task_id.to_owned());
        if !state.task_deps.contains(&edge) {
            state.task_deps.push(edge);
        }
        Ok(())
    }

    async fn remove_task_dep(&self, blocker_task_id: &str, blocked_task_id: &str) -> Result<(), DbError> {
        let mut state = self.state.lock().unwrap();
        state
            .task_deps
            .retain(|(blocker, blocked)| !(blocker == blocker_task_id && blocked == blocked_task_id));
        Ok(())
    }

    async fn list_blockers(&self, task_id: &str) -> Result<Vec<String>, DbError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .task_deps
            .iter()
            .filter(|(_, blocked)| blocked == task_id)
            .map(|(blocker, _)| blocker.clone())
            .collect())
    }

    async fn list_blocking(&self, task_id: &str) -> Result<Vec<String>, DbError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .task_deps
            .iter()
            .filter(|(blocker, _)| blocker == task_id)
            .map(|(_, blocked)| blocked.clone())
            .collect())
    }
}
