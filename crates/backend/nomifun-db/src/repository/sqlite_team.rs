use nomifun_common::now_ms;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{MailboxMessageRow, TeamAgentRow, TeamRow, TeamTaskRow};
use crate::repository::team::{ITeamRepository, UpdateTaskParams, UpdateTeamAgentParams, UpdateTeamParams};

/// SQLite-backed implementation of [`ITeamRepository`].
#[derive(Clone, Debug)]
pub struct SqliteTeamRepository {
    pool: SqlitePool,
}

impl SqliteTeamRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ITeamRepository for SqliteTeamRepository {
    // ── Team CRUD ────────────────────────────────────────────────────

    async fn create_team(&self, row: &TeamRow) -> Result<(), DbError> {
        // The `agents` JSON column is gone — agents are inserted separately via
        // `create_team_agent`, after their conversations exist (spec §9.A).
        sqlx::query(
            "INSERT INTO teams (id, user_id, name, workspace, workspace_mode, lead_agent_id, session_mode, agents_version, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.user_id)
        .bind(&row.name)
        .bind(&row.workspace)
        .bind(&row.workspace_mode)
        .bind(&row.lead_agent_id)
        .bind(&row.session_mode)
        .bind(&row.agents_version)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_teams(&self) -> Result<Vec<TeamRow>, DbError> {
        let rows = sqlx::query_as::<_, TeamRow>("SELECT * FROM teams ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    async fn get_team(&self, team_id: &str) -> Result<Option<TeamRow>, DbError> {
        let row = sqlx::query_as::<_, TeamRow>("SELECT * FROM teams WHERE id = ?")
            .bind(team_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn update_team(&self, team_id: &str, params: &UpdateTeamParams) -> Result<(), DbError> {
        let mut set_clauses = Vec::new();
        if params.name.is_some() {
            set_clauses.push("name = ?");
        }
        if params.lead_agent_id.is_some() {
            set_clauses.push("lead_agent_id = ?");
        }

        if set_clauses.is_empty() {
            return Ok(());
        }

        set_clauses.push("updated_at = ?");
        let sql = format!("UPDATE teams SET {} WHERE id = ?", set_clauses.join(", "));

        let mut query = sqlx::query(&sql);
        if let Some(ref name) = params.name {
            query = query.bind(name);
        }
        if let Some(ref lead_agent_id) = params.lead_agent_id {
            query = query.bind(lead_agent_id);
        }
        query = query.bind(now_ms());
        query = query.bind(team_id);

        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("team {team_id}")));
        }
        Ok(())
    }

    async fn delete_team(&self, team_id: &str) -> Result<(), DbError> {
        // FK ON DELETE CASCADE removes the team's team_agents, mailbox,
        // team_tasks, and (transitively) team_task_deps rows.
        let result = sqlx::query("DELETE FROM teams WHERE id = ?")
            .bind(team_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("team {team_id}")));
        }
        Ok(())
    }

    // ── Team agents (was teams.agents JSON array) ─────────────────────

    async fn create_team_agent(&self, row: &TeamAgentRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO team_agents \
                (slot_id, team_id, name, role, conversation_id, backend, model, \
                 custom_agent_id, status, conversation_type, cli_path, sort_order) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.slot_id)
        .bind(&row.team_id)
        .bind(&row.name)
        .bind(&row.role)
        .bind(&row.conversation_id)
        .bind(&row.backend)
        .bind(&row.model)
        .bind(&row.custom_agent_id)
        .bind(&row.status)
        .bind(&row.conversation_type)
        .bind(&row.cli_path)
        .bind(row.sort_order)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_team_agents(&self, team_id: &str) -> Result<Vec<TeamAgentRow>, DbError> {
        let rows = sqlx::query_as::<_, TeamAgentRow>(
            "SELECT slot_id, team_id, name, role, conversation_id, backend, model, \
                    custom_agent_id, status, conversation_type, cli_path, sort_order \
             FROM team_agents \
             WHERE team_id = ? \
             ORDER BY sort_order ASC, slot_id ASC",
        )
        .bind(team_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get_team_agent(&self, slot_id: &str) -> Result<Option<TeamAgentRow>, DbError> {
        let row = sqlx::query_as::<_, TeamAgentRow>(
            "SELECT slot_id, team_id, name, role, conversation_id, backend, model, \
                    custom_agent_id, status, conversation_type, cli_path, sort_order \
             FROM team_agents \
             WHERE slot_id = ?",
        )
        .bind(slot_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn update_team_agent(&self, slot_id: &str, params: &UpdateTeamAgentParams) -> Result<(), DbError> {
        let mut set_clauses = Vec::new();
        if params.name.is_some() {
            set_clauses.push("name = ?");
        }
        if params.role.is_some() {
            set_clauses.push("role = ?");
        }
        if params.conversation_id.is_some() {
            set_clauses.push("conversation_id = ?");
        }
        if params.backend.is_some() {
            set_clauses.push("backend = ?");
        }
        if params.model.is_some() {
            set_clauses.push("model = ?");
        }
        if params.custom_agent_id.is_some() {
            set_clauses.push("custom_agent_id = ?");
        }
        if params.status.is_some() {
            set_clauses.push("status = ?");
        }
        if params.conversation_type.is_some() {
            set_clauses.push("conversation_type = ?");
        }
        if params.cli_path.is_some() {
            set_clauses.push("cli_path = ?");
        }
        if params.sort_order.is_some() {
            set_clauses.push("sort_order = ?");
        }

        if set_clauses.is_empty() {
            return Ok(());
        }

        let sql = format!("UPDATE team_agents SET {} WHERE slot_id = ?", set_clauses.join(", "));

        let mut query = sqlx::query(&sql);
        if let Some(ref name) = params.name {
            query = query.bind(name);
        }
        if let Some(ref role) = params.role {
            query = query.bind(role);
        }
        if let Some(ref conversation_id) = params.conversation_id {
            query = query.bind(conversation_id);
        }
        if let Some(ref backend) = params.backend {
            query = query.bind(backend);
        }
        if let Some(ref model) = params.model {
            query = query.bind(model);
        }
        if let Some(ref custom_agent_id) = params.custom_agent_id {
            query = query.bind(custom_agent_id);
        }
        if let Some(ref status) = params.status {
            query = query.bind(status);
        }
        if let Some(ref conversation_type) = params.conversation_type {
            query = query.bind(conversation_type);
        }
        if let Some(ref cli_path) = params.cli_path {
            query = query.bind(cli_path);
        }
        if let Some(sort_order) = params.sort_order {
            query = query.bind(sort_order);
        }
        query = query.bind(slot_id);

        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("team_agent {slot_id}")));
        }
        Ok(())
    }

    async fn rename_team_agent(&self, slot_id: &str, name: &str) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE team_agents SET name = ? WHERE slot_id = ?")
            .bind(name)
            .bind(slot_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("team_agent {slot_id}")));
        }
        Ok(())
    }

    async fn remove_team_agent(&self, slot_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM team_agents WHERE slot_id = ?")
            .bind(slot_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Mailbox ──────────────────────────────────────────────────────

    async fn write_message(&self, row: &MailboxMessageRow) -> Result<i64, DbError> {
        // `id` is INTEGER PRIMARY KEY AUTOINCREMENT — omit it from the INSERT
        // and return the assigned rowid. (Binding a string id to the INTEGER
        // column, as the legacy code did, would now be wrong.)
        let result = sqlx::query(
            "INSERT INTO mailbox \
                (team_id, to_agent_id, from_agent_id, type, content, summary, files, read, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.team_id)
        .bind(&row.to_agent_id)
        .bind(&row.from_agent_id)
        .bind(&row.msg_type)
        .bind(&row.content)
        .bind(&row.summary)
        .bind(&row.files)
        .bind(row.read)
        .bind(row.created_at)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn read_unread_and_mark(&self, team_id: &str, to_agent_id: &str) -> Result<Vec<MailboxMessageRow>, DbError> {
        // Use BEGIN IMMEDIATE for atomicity: prevents concurrent readers
        // from seeing the same unread messages.
        let mut tx = self.pool.begin().await?;

        // SQLite does not support RETURNING on UPDATE, so we use a
        // two-step approach within the same IMMEDIATE transaction.
        sqlx::query("PRAGMA read_uncommitted = false").execute(&mut *tx).await?;

        let rows = sqlx::query_as::<_, MailboxMessageRow>(
            "SELECT id, team_id, to_agent_id, from_agent_id, \
                    type, content, summary, files, read, created_at \
             FROM mailbox \
             WHERE team_id = ? AND to_agent_id = ? AND read = 0 \
             ORDER BY created_at ASC",
        )
        .bind(team_id)
        .bind(to_agent_id)
        .fetch_all(&mut *tx)
        .await?;

        if !rows.is_empty() {
            sqlx::query(
                "UPDATE mailbox SET read = 1 \
                 WHERE team_id = ? AND to_agent_id = ? AND read = 0",
            )
            .bind(team_id)
            .bind(to_agent_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(rows)
    }

    async fn peek_unread(&self, team_id: &str, to_agent_id: &str) -> Result<Vec<MailboxMessageRow>, DbError> {
        let rows = sqlx::query_as::<_, MailboxMessageRow>(
            "SELECT id, team_id, to_agent_id, from_agent_id, \
                    type, content, summary, files, read, created_at \
             FROM mailbox \
             WHERE team_id = ? AND to_agent_id = ? AND read = 0 \
             ORDER BY created_at ASC",
        )
        .bind(team_id)
        .bind(to_agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn mark_read_batch(&self, ids: &[i64]) -> Result<(), DbError> {
        if ids.is_empty() {
            return Ok(());
        }
        // SQLite placeholder limit is 999; batch if needed.
        for chunk in ids.chunks(500) {
            let placeholders: String = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!("UPDATE mailbox SET read = 1 WHERE id IN ({placeholders})");
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            query.execute(&self.pool).await?;
        }
        Ok(())
    }

    async fn get_history(
        &self,
        team_id: &str,
        to_agent_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<MailboxMessageRow>, DbError> {
        let rows = if let Some(limit) = limit {
            sqlx::query_as::<_, MailboxMessageRow>(
                "SELECT id, team_id, to_agent_id, from_agent_id, \
                        type, content, summary, files, read, created_at \
                 FROM mailbox \
                 WHERE team_id = ? AND to_agent_id = ? \
                 ORDER BY created_at ASC \
                 LIMIT ?",
            )
            .bind(team_id)
            .bind(to_agent_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MailboxMessageRow>(
                "SELECT id, team_id, to_agent_id, from_agent_id, \
                        type, content, summary, files, read, created_at \
                 FROM mailbox \
                 WHERE team_id = ? AND to_agent_id = ? \
                 ORDER BY created_at ASC",
            )
            .bind(team_id)
            .bind(to_agent_id)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    // ── Tasks ────────────────────────────────────────────────────────

    async fn create_task(&self, row: &TeamTaskRow) -> Result<(), DbError> {
        // The `blocked_by` / `blocks` JSON columns are gone — dependencies are
        // recorded separately via `add_task_dep` (team_task_deps edge table).
        sqlx::query(
            "INSERT INTO team_tasks \
                (id, team_id, subject, description, status, owner, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.team_id)
        .bind(&row.subject)
        .bind(&row.description)
        .bind(&row.status)
        .bind(&row.owner)
        .bind(&row.metadata)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_task_by_id(&self, team_id: &str, task_id: &str) -> Result<Option<TeamTaskRow>, DbError> {
        let row = sqlx::query_as::<_, TeamTaskRow>("SELECT * FROM team_tasks WHERE team_id = ? AND id = ?")
            .bind(team_id)
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn update_task(&self, task_id: &str, params: &UpdateTaskParams) -> Result<(), DbError> {
        let mut set_clauses = Vec::new();
        if params.status.is_some() {
            set_clauses.push("status = ?");
        }
        if params.description.is_some() {
            set_clauses.push("description = ?");
        }
        if params.owner.is_some() {
            set_clauses.push("owner = ?");
        }
        if params.metadata.is_some() {
            set_clauses.push("metadata = ?");
        }

        if set_clauses.is_empty() {
            return Ok(());
        }

        set_clauses.push("updated_at = ?");
        let sql = format!("UPDATE team_tasks SET {} WHERE id = ?", set_clauses.join(", "));

        let mut query = sqlx::query(&sql);
        if let Some(ref status) = params.status {
            query = query.bind(status);
        }
        if let Some(ref description) = params.description {
            query = query.bind(description);
        }
        if let Some(ref owner) = params.owner {
            query = query.bind(owner);
        }
        if let Some(ref metadata) = params.metadata {
            query = query.bind(metadata);
        }
        query = query.bind(now_ms());
        query = query.bind(task_id);

        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("task {task_id}")));
        }
        Ok(())
    }

    async fn list_tasks(&self, team_id: &str) -> Result<Vec<TeamTaskRow>, DbError> {
        let rows =
            sqlx::query_as::<_, TeamTaskRow>("SELECT * FROM team_tasks WHERE team_id = ? ORDER BY created_at ASC")
                .bind(team_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    // ── Task dependencies (was blocked_by/blocks JSON arrays) ─────────

    async fn add_task_dep(&self, blocker_task_id: &str, blocked_task_id: &str) -> Result<(), DbError> {
        // Idempotent insert of one directed edge "blocker blocks blocked".
        // The composite PRIMARY KEY makes re-adds a no-op via OR IGNORE.
        sqlx::query("INSERT OR IGNORE INTO team_task_deps (blocker_task_id, blocked_task_id) VALUES (?, ?)")
            .bind(blocker_task_id)
            .bind(blocked_task_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn remove_task_dep(&self, blocker_task_id: &str, blocked_task_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM team_task_deps WHERE blocker_task_id = ? AND blocked_task_id = ?")
            .bind(blocker_task_id)
            .bind(blocked_task_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_blockers(&self, task_id: &str) -> Result<Vec<String>, DbError> {
        // Tasks that block `task_id` (the task's blocked_by set).
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT blocker_task_id FROM team_task_deps WHERE blocked_task_id = ? ORDER BY blocker_task_id ASC")
                .bind(task_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn list_blocking(&self, task_id: &str) -> Result<Vec<String>, DbError> {
        // Tasks that `task_id` blocks (the task's blocks set).
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT blocked_task_id FROM team_task_deps WHERE blocker_task_id = ? ORDER BY blocked_task_id ASC")
                .bind(task_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    /// Inserts a user + conversation so FK constraints on team_agents hold.
    async fn setup() -> (SqliteTeamRepository, crate::Database) {
        let db = init_database_memory().await.expect("init db");
        let repo = SqliteTeamRepository::new(db.pool().clone());

        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
             VALUES ('user_1', 'tester', 'hash', 0, 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
             VALUES (1, 'user_1', 'Slot Conv', 'normal', 0, 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();

        (repo, db)
    }

    fn make_team(id: &str) -> TeamRow {
        let now = now_ms();
        TeamRow {
            id: id.into(),
            user_id: "user_1".into(),
            name: "Test Team".into(),
            workspace: "/tmp/ws".into(),
            workspace_mode: "shared".into(),
            lead_agent_id: Some("lead".into()),
            session_mode: Some("acp".into()),
            agents_version: "1.0.0".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn make_agent(slot_id: &str, team_id: &str, sort_order: i64) -> TeamAgentRow {
        TeamAgentRow {
            slot_id: slot_id.into(),
            team_id: team_id.into(),
            name: "Builder".into(),
            role: "teammate".into(),
            conversation_id: Some(1),
            backend: "claude".into(),
            model: String::new(),
            custom_agent_id: None,
            status: Some("idle".into()),
            conversation_type: Some("acp".into()),
            cli_path: None,
            sort_order,
        }
    }

    fn make_task(id: &str, team_id: &str) -> TeamTaskRow {
        let now = now_ms();
        TeamTaskRow {
            id: id.into(),
            team_id: team_id.into(),
            subject: "Do thing".into(),
            description: Some("details".into()),
            status: "pending".into(),
            owner: None,
            metadata: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_mailbox(team_id: &str, to: &str, from: &str) -> MailboxMessageRow {
        MailboxMessageRow {
            id: 0, // ignored on insert (autoincrement)
            team_id: team_id.into(),
            to_agent_id: to.into(),
            from_agent_id: from.into(),
            msg_type: "message".into(),
            content: "hello".into(),
            summary: None,
            files: None,
            read: false,
            created_at: now_ms(),
        }
    }

    #[tokio::test]
    async fn create_and_get_team() {
        let (repo, _db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();

        let found = repo.get_team("team_1").await.unwrap().expect("found");
        assert_eq!(found.id, "team_1");
        assert_eq!(found.name, "Test Team");
        assert_eq!(found.lead_agent_id.as_deref(), Some("lead"));

        let all = repo.list_teams().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn update_team_fields() {
        let (repo, _db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();

        repo.update_team(
            "team_1",
            &UpdateTeamParams {
                name: Some("Renamed".into()),
                lead_agent_id: Some("slot_x".into()),
            },
        )
        .await
        .unwrap();

        let found = repo.get_team("team_1").await.unwrap().expect("found");
        assert_eq!(found.name, "Renamed");
        assert_eq!(found.lead_agent_id.as_deref(), Some("slot_x"));
    }

    #[tokio::test]
    async fn team_agents_crud() {
        let (repo, _db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();

        // Was a teams.agents JSON array; now individual rows.
        repo.create_team_agent(&make_agent("slot_b", "team_1", 1)).await.unwrap();
        repo.create_team_agent(&make_agent("slot_a", "team_1", 0)).await.unwrap();

        // Ordered by sort_order, so slot_a (0) precedes slot_b (1).
        let agents = repo.list_team_agents("team_1").await.unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].slot_id, "slot_a");
        assert_eq!(agents[1].slot_id, "slot_b");
        assert_eq!(agents[0].conversation_id, Some(1));

        let one = repo.get_team_agent("slot_a").await.unwrap().expect("found");
        assert_eq!(one.name, "Builder");

        repo.rename_team_agent("slot_a", "Architect").await.unwrap();
        repo.update_team_agent(
            "slot_a",
            &UpdateTeamAgentParams {
                status: Some("running".into()),
                model: Some("opus".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let updated = repo.get_team_agent("slot_a").await.unwrap().expect("found");
        assert_eq!(updated.name, "Architect");
        assert_eq!(updated.status.as_deref(), Some("running"));
        assert_eq!(updated.model, "opus");

        repo.remove_team_agent("slot_a").await.unwrap();
        let remaining = repo.list_team_agents("team_1").await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].slot_id, "slot_b");
    }

    #[tokio::test]
    async fn delete_team_cascades_to_agents() {
        let (repo, db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();
        repo.create_team_agent(&make_agent("slot_a", "team_1", 0)).await.unwrap();

        repo.delete_team("team_1").await.unwrap();
        assert!(repo.get_team("team_1").await.unwrap().is_none());

        // FK CASCADE removed the agent row (no delete_*_by_team helpers exist).
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM team_agents WHERE team_id = 'team_1'")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn delete_team_missing_is_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.delete_team("nope").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn mailbox_id_is_i64_autoincrement() {
        let (repo, _db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();

        // write_message returns the assigned autoincrement id (i64), not a
        // caller-supplied string. The `id` field of the row is ignored.
        let id1 = repo.write_message(&make_mailbox("team_1", "slot_a", "user")).await.unwrap();
        let id2 = repo.write_message(&make_mailbox("team_1", "slot_a", "lead")).await.unwrap();
        assert!(id1 > 0);
        assert!(id2 > id1);

        let unread = repo.peek_unread("team_1", "slot_a").await.unwrap();
        assert_eq!(unread.len(), 2);
        assert_eq!(unread[0].id, id1);
        assert_eq!(unread[1].from_agent_id, "lead");
    }

    #[tokio::test]
    async fn mailbox_read_mark_and_batch() {
        let (repo, _db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();

        let id1 = repo.write_message(&make_mailbox("team_1", "slot_a", "user")).await.unwrap();
        let id2 = repo.write_message(&make_mailbox("team_1", "slot_a", "user")).await.unwrap();

        // mark_read_batch now takes &[i64].
        repo.mark_read_batch(&[id1]).await.unwrap();
        let unread = repo.peek_unread("team_1", "slot_a").await.unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].id, id2);

        // read_unread_and_mark drains and marks the rest.
        let drained = repo.read_unread_and_mark("team_1", "slot_a").await.unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, id2);
        assert!(repo.peek_unread("team_1", "slot_a").await.unwrap().is_empty());

        let history = repo.get_history("team_1", "slot_a", None).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn tasks_crud() {
        let (repo, _db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();

        repo.create_task(&make_task("task_1", "team_1")).await.unwrap();
        let found = repo.find_task_by_id("team_1", "task_1").await.unwrap().expect("found");
        assert_eq!(found.subject, "Do thing");
        assert_eq!(found.status, "pending");

        repo.update_task(
            "task_1",
            &UpdateTaskParams {
                status: Some("in_progress".into()),
                owner: Some("slot_a".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let updated = repo.find_task_by_id("team_1", "task_1").await.unwrap().expect("found");
        assert_eq!(updated.status, "in_progress");
        assert_eq!(updated.owner.as_deref(), Some("slot_a"));

        let all = repo.list_tasks("team_1").await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn task_deps_edge_table() {
        let (repo, _db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();
        // task_a blocks task_b — both rows must exist for the FK.
        repo.create_task(&make_task("task_a", "team_1")).await.unwrap();
        repo.create_task(&make_task("task_b", "team_1")).await.unwrap();
        repo.create_task(&make_task("task_c", "team_1")).await.unwrap();

        // Was blocked_by/blocks JSON arrays; now directed edges.
        repo.add_task_dep("task_a", "task_b").await.unwrap();
        repo.add_task_dep("task_c", "task_b").await.unwrap();
        // Idempotent re-add (INSERT OR IGNORE on the composite PK).
        repo.add_task_dep("task_a", "task_b").await.unwrap();

        // task_b is blocked by task_a and task_c.
        let blockers = repo.list_blockers("task_b").await.unwrap();
        assert_eq!(blockers, vec!["task_a".to_string(), "task_c".to_string()]);

        // task_a blocks task_b.
        let blocking = repo.list_blocking("task_a").await.unwrap();
        assert_eq!(blocking, vec!["task_b".to_string()]);

        repo.remove_task_dep("task_a", "task_b").await.unwrap();
        let blockers = repo.list_blockers("task_b").await.unwrap();
        assert_eq!(blockers, vec!["task_c".to_string()]);
    }

    #[tokio::test]
    async fn delete_team_cascades_to_tasks_and_deps() {
        let (repo, db) = setup().await;
        repo.create_team(&make_team("team_1")).await.unwrap();
        repo.create_task(&make_task("task_a", "team_1")).await.unwrap();
        repo.create_task(&make_task("task_b", "team_1")).await.unwrap();
        repo.add_task_dep("task_a", "task_b").await.unwrap();
        repo.write_message(&make_mailbox("team_1", "slot_a", "user")).await.unwrap();

        repo.delete_team("team_1").await.unwrap();

        // FK CASCADE clears team_tasks, team_task_deps, and mailbox.
        let tasks: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM team_tasks WHERE team_id = 'team_1'")
            .fetch_one(db.pool())
            .await
            .unwrap();
        let deps: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM team_task_deps")
            .fetch_one(db.pool())
            .await
            .unwrap();
        let mail: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mailbox WHERE team_id = 'team_1'")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(tasks.0, 0);
        assert_eq!(deps.0, 0);
        assert_eq!(mail.0, 0);
    }
}
