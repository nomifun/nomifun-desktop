use crate::error::DbError;
use crate::models::{MailboxMessageRow, TeamAgentRow, TeamRow, TeamTaskRow};

/// Parameters for updating a team record.
///
/// The former `agents` JSON column is gone — agents are managed via the
/// dedicated `team_agents` table (see [`ITeamRepository::create_team_agent`]).
#[derive(Debug, Clone, Default)]
pub struct UpdateTeamParams {
    pub name: Option<String>,
    pub lead_agent_id: Option<String>,
}

/// Parameters for updating a `team_agents` slot record.
///
/// Each `Some` field is written; `None` leaves the existing value untouched.
#[derive(Debug, Clone, Default)]
pub struct UpdateTeamAgentParams {
    pub name: Option<String>,
    pub role: Option<String>,
    pub conversation_id: Option<i64>,
    pub backend: Option<String>,
    pub model: Option<String>,
    pub custom_agent_id: Option<String>,
    pub status: Option<String>,
    pub conversation_type: Option<String>,
    pub cli_path: Option<String>,
    pub sort_order: Option<i64>,
}

/// Parameters for updating a task record.
///
/// The former `blocked_by` JSON column is gone — dependencies are managed via
/// the `team_task_deps` edge table (see [`ITeamRepository::add_task_dep`]).
#[derive(Debug, Clone, Default)]
pub struct UpdateTaskParams {
    pub status: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub metadata: Option<String>,
}

/// Data access abstraction for team collaboration tables.
///
/// Covers five tables: `teams`, `team_agents`, `mailbox`, `team_tasks`, and
/// `team_task_deps`.
///
/// A `teams` row no longer embeds its agent roster (formerly the `agents` JSON
/// array) nor its task dependency graph (formerly `team_tasks.blocked_by` /
/// `blocks` JSON arrays). Both are now separate tables joined by FK. Callers
/// (the `nomifun-team` service / in-memory layer) assemble the full `Team` /
/// `TeamTask` aggregates from [`TeamRow`] + [`list_team_agents`] and
/// [`TeamTaskRow`] + [`list_blockers`] / [`list_blocking`].
///
/// Deleting a `teams` row cascades (via FK `ON DELETE CASCADE`) to
/// `team_agents`, `mailbox`, `team_tasks`, and transitively `team_task_deps`,
/// so [`delete_team`] is a single `DELETE` — there are no
/// `delete_mailbox_by_team` / `delete_tasks_by_team` helpers.
///
/// Object-safe via `async_trait` to support `Arc<dyn ITeamRepository>`.
///
/// [`list_team_agents`]: ITeamRepository::list_team_agents
/// [`delete_team`]: ITeamRepository::delete_team
/// [`list_blockers`]: ITeamRepository::list_blockers
/// [`list_blocking`]: ITeamRepository::list_blocking
#[async_trait::async_trait]
pub trait ITeamRepository: Send + Sync {
    // ── Team CRUD ────────────────────────────────────────────────────

    /// Inserts a new team record (the `teams` row only — agents are inserted
    /// separately via [`create_team_agent`](Self::create_team_agent)).
    async fn create_team(&self, row: &TeamRow) -> Result<(), DbError>;

    /// Returns all teams ordered by creation time ascending. The returned
    /// [`TeamRow`]s carry no agent roster; use
    /// [`list_team_agents`](Self::list_team_agents) per team to assemble it.
    async fn list_teams(&self) -> Result<Vec<TeamRow>, DbError>;

    /// Returns a single team by id, or `None` if not found. The returned
    /// [`TeamRow`] carries no agent roster; use
    /// [`list_team_agents`](Self::list_team_agents) to assemble it.
    async fn get_team(&self, team_id: &str) -> Result<Option<TeamRow>, DbError>;

    /// Updates a team by id with the provided fields.
    /// Returns `DbError::NotFound` if absent.
    async fn update_team(&self, team_id: &str, params: &UpdateTeamParams) -> Result<(), DbError>;

    /// Deletes a team by id. The FK `ON DELETE CASCADE` chain removes the
    /// team's `team_agents`, `mailbox`, `team_tasks`, and `team_task_deps`
    /// rows automatically. Returns `DbError::NotFound` if absent.
    async fn delete_team(&self, team_id: &str) -> Result<(), DbError>;

    // ── Team agents (was teams.agents JSON array) ─────────────────────

    /// Inserts a single agent slot into `team_agents`.
    ///
    /// FK ordering (spec §9.A): the slot's conversation (if any) must already
    /// exist, and the slot's `team_id` must reference an existing team.
    async fn create_team_agent(&self, row: &TeamAgentRow) -> Result<(), DbError>;

    /// Returns all agent slots for a team, ordered by `sort_order` ascending
    /// (the original array order), then `slot_id` for stable tie-breaking.
    async fn list_team_agents(&self, team_id: &str) -> Result<Vec<TeamAgentRow>, DbError>;

    /// Returns a single agent slot by `slot_id`, or `None` if not found.
    async fn get_team_agent(&self, slot_id: &str) -> Result<Option<TeamAgentRow>, DbError>;

    /// Updates an agent slot by `slot_id` with the provided fields.
    /// Returns `DbError::NotFound` if absent.
    async fn update_team_agent(&self, slot_id: &str, params: &UpdateTeamAgentParams) -> Result<(), DbError>;

    /// Renames an agent slot's display `name`.
    /// Returns `DbError::NotFound` if absent.
    async fn rename_team_agent(&self, slot_id: &str, name: &str) -> Result<(), DbError>;

    /// Removes a single agent slot by `slot_id`. Slots that don't exist are
    /// silently ignored.
    async fn remove_team_agent(&self, slot_id: &str) -> Result<(), DbError>;

    // ── Mailbox ──────────────────────────────────────────────────────

    /// Writes a message to the mailbox and returns its autoincrement `id`.
    ///
    /// The `id` field of `row` is ignored on insert (the column is
    /// `INTEGER PRIMARY KEY AUTOINCREMENT`); the assigned id is returned via
    /// `last_insert_rowid()`.
    async fn write_message(&self, row: &MailboxMessageRow) -> Result<i64, DbError>;

    /// Atomically reads all unread messages for `to_agent_id` in a team
    /// and marks them as read. Uses `BEGIN IMMEDIATE` for atomicity.
    async fn read_unread_and_mark(&self, team_id: &str, to_agent_id: &str) -> Result<Vec<MailboxMessageRow>, DbError>;

    /// Reads all unread messages for `to_agent_id` without marking them as read.
    async fn peek_unread(&self, team_id: &str, to_agent_id: &str) -> Result<Vec<MailboxMessageRow>, DbError>;

    /// Marks the given message IDs as read. IDs that don't exist are silently ignored.
    async fn mark_read_batch(&self, ids: &[i64]) -> Result<(), DbError>;

    /// Returns message history for an agent, optionally limited.
    /// Messages are ordered by `created_at` ascending.
    async fn get_history(
        &self,
        team_id: &str,
        to_agent_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<MailboxMessageRow>, DbError>;

    // ── Tasks ────────────────────────────────────────────────────────

    /// Creates a new task (the `team_tasks` row only — dependencies are
    /// recorded separately via [`add_task_dep`](Self::add_task_dep)).
    async fn create_task(&self, row: &TeamTaskRow) -> Result<(), DbError>;

    /// Finds a task by exact id within a team.
    async fn find_task_by_id(&self, team_id: &str, task_id: &str) -> Result<Option<TeamTaskRow>, DbError>;

    /// Updates a task by id with the provided fields.
    /// Returns `DbError::NotFound` if absent.
    async fn update_task(&self, task_id: &str, params: &UpdateTaskParams) -> Result<(), DbError>;

    /// Returns all tasks for a team, ordered by `created_at` ascending. The
    /// returned [`TeamTaskRow`]s carry no dependency lists; use
    /// [`list_blockers`](Self::list_blockers) /
    /// [`list_blocking`](Self::list_blocking) to assemble them.
    async fn list_tasks(&self, team_id: &str) -> Result<Vec<TeamTaskRow>, DbError>;

    // ── Task dependencies (was blocked_by/blocks JSON arrays) ─────────

    /// Records that `blocker_task_id` blocks `blocked_task_id`. Idempotent:
    /// re-adding an existing edge is a no-op (`INSERT OR IGNORE`).
    async fn add_task_dep(&self, blocker_task_id: &str, blocked_task_id: &str) -> Result<(), DbError>;

    /// Removes the edge stating that `blocker_task_id` blocks
    /// `blocked_task_id`. Missing edges are silently ignored.
    async fn remove_task_dep(&self, blocker_task_id: &str, blocked_task_id: &str) -> Result<(), DbError>;

    /// Returns the ids of tasks that block `task_id` (i.e. the task's
    /// `blocked_by` set: rows `WHERE blocked_task_id = task_id`).
    async fn list_blockers(&self, task_id: &str) -> Result<Vec<String>, DbError>;

    /// Returns the ids of tasks that `task_id` blocks (i.e. the task's
    /// `blocks` set: rows `WHERE blocker_task_id = task_id`).
    async fn list_blocking(&self, task_id: &str) -> Result<Vec<String>, DbError>;
}
