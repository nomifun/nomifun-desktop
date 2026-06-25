use std::sync::Arc;

use nomifun_common::{generate_prefixed_id, now_ms};
use nomifun_db::ITeamRepository;
use nomifun_db::UpdateTaskParams;
use nomifun_db::models::TeamTaskRow;
use tracing::debug;

use crate::error::TeamError;
use crate::types::{TaskStatus, TeamTask};

pub struct TaskBoard {
    repo: Arc<dyn ITeamRepository>,
}

/// Optional fields for task update.
#[derive(Debug, Clone, Default)]
pub struct TaskUpdate {
    pub status: Option<TaskStatus>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub blocked_by: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

impl TaskBoard {
    pub fn new(repo: Arc<dyn ITeamRepository>) -> Self {
        Self { repo }
    }

    pub async fn create_task(
        &self,
        team_id: &str,
        subject: &str,
        description: Option<&str>,
        owner: Option<&str>,
        blocked_by: &[String],
    ) -> Result<TeamTask, TeamError> {
        for dep_id in blocked_by {
            let dep = self.repo.find_task_by_id(team_id, dep_id).await?;
            if dep.is_none() {
                return Err(TeamError::BlockedTaskNotFound(dep_id.clone()));
            }
        }

        let task_id = generate_prefixed_id("task");
        let now = now_ms();

        let row = TeamTaskRow {
            id: task_id.clone(),
            team_id: team_id.to_owned(),
            subject: subject.to_owned(),
            description: description.map(str::to_owned),
            status: TaskStatus::Pending.to_string(),
            owner: owner.map(str::to_owned),
            metadata: None,
            created_at: now,
            updated_at: now,
        };

        self.repo.create_task(&row).await?;

        // Record each "dep blocks this task" edge in `team_task_deps`
        // (was the bidirectional blocked_by/blocks JSON arrays — spec §5.5).
        for dep_id in blocked_by {
            self.repo.add_task_dep(dep_id, &task_id).await?;
        }

        debug!(team_id, task_id = %task_id, subject, "task created");

        // The new task is blocked_by the deps and blocks nobody yet.
        TeamTask::from_parts(&row, blocked_by.to_vec(), Vec::new()).map_err(TeamError::Json)
    }

    pub async fn update_task(&self, team_id: &str, task_id: &str, update: &TaskUpdate) -> Result<TeamTask, TeamError> {
        self.repo
            .find_task_by_id(team_id, task_id)
            .await?
            .ok_or_else(|| TeamError::TaskNotFound(task_id.to_owned()))?;

        let params = UpdateTaskParams {
            status: update.status.map(|s| s.to_string()),
            description: update.description.clone(),
            owner: update.owner.clone(),
            metadata: update.metadata.as_ref().map(serde_json::to_string).transpose()?,
        };

        self.repo.update_task(task_id, &params).await?;

        // Reconcile the dependency edges when the caller passes a new
        // `blocked_by` set. Diff against the current blockers and add/remove
        // edges so we never duplicate or drop unrelated edges.
        if let Some(ref desired) = update.blocked_by {
            let current = self.repo.list_blockers(task_id).await?;
            for dep_id in desired {
                if !current.contains(dep_id) {
                    self.repo.add_task_dep(dep_id, task_id).await?;
                }
            }
            for dep_id in &current {
                if !desired.contains(dep_id) {
                    self.repo.remove_task_dep(dep_id, task_id).await?;
                }
            }
        }

        if update.status == Some(TaskStatus::Completed) {
            self.check_unblocks(task_id).await?;
        }

        let updated = self
            .repo
            .find_task_by_id(team_id, task_id)
            .await?
            .ok_or_else(|| TeamError::TaskNotFound(task_id.to_owned()))?;

        debug!(team_id, task_id, "task updated");

        self.assemble_task(&updated).await
    }

    pub async fn list_tasks(&self, team_id: &str) -> Result<Vec<TeamTask>, TeamError> {
        let rows = self.repo.list_tasks(team_id).await?;
        let mut tasks = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Ok(task) = self.assemble_task(row).await {
                tasks.push(task);
            }
        }
        Ok(tasks)
    }

    /// Assemble a [`TeamTask`] aggregate from its row + dependency edges
    /// (`blocked_by` = `list_blockers`, `blocks` = `list_blocking`).
    async fn assemble_task(&self, row: &TeamTaskRow) -> Result<TeamTask, TeamError> {
        let blocked_by = self.repo.list_blockers(&row.id).await?;
        let blocks = self.repo.list_blocking(&row.id).await?;
        TeamTask::from_parts(row, blocked_by, blocks).map_err(TeamError::Json)
    }

    /// When `completed_task_id` finishes, drop every "completed blocks X" edge
    /// so the downstream tasks it was blocking become unblocked (spec §5.5:
    /// `check_unblocks` is a per-edge DELETE).
    async fn check_unblocks(&self, completed_task_id: &str) -> Result<(), TeamError> {
        let downstream = self.repo.list_blocking(completed_task_id).await?;
        for downstream_id in &downstream {
            self.repo.remove_task_dep(completed_task_id, downstream_id).await?;
            debug!(
                completed = completed_task_id,
                unblocked = %downstream_id,
                "dependency unblocked"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::MockTeamRepo;

    // -- Helper ---------------------------------------------------------------

    async fn create_simple_task(board: &TaskBoard, team_id: &str, subject: &str) -> TeamTask {
        board.create_task(team_id, subject, None, None, &[]).await.unwrap()
    }

    // -- Tests ----------------------------------------------------------------

    #[tokio::test]
    async fn create_task_no_dependencies() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task = create_simple_task(&board, "t1", "Implement feature").await;
        assert_eq!(task.subject, "Implement feature");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.blocked_by.is_empty());
        assert!(task.blocks.is_empty());
    }

    #[tokio::test]
    async fn create_task_with_owner_and_description() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task = board
            .create_task("t1", "Design API", Some("REST endpoints"), Some("a1"), &[])
            .await
            .unwrap();
        assert_eq!(task.description.as_deref(), Some("REST endpoints"));
        assert_eq!(task.owner.as_deref(), Some("a1"));
    }

    #[tokio::test]
    async fn create_task_with_dependencies() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo.clone());

        let task_a = create_simple_task(&board, "t1", "Task A").await;
        let task_b = board
            .create_task("t1", "Task B", None, None, std::slice::from_ref(&task_a.id))
            .await
            .unwrap();

        assert_eq!(task_b.blocked_by, vec![task_a.id.clone()]);

        let blocks_a = repo.list_blocking(&task_a.id).await.unwrap();
        assert_eq!(blocks_a, vec![task_b.id]);
    }

    #[tokio::test]
    async fn create_task_nonexistent_dependency_fails() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let result = board.create_task("t1", "X", None, None, &["nonexistent".into()]).await;
        assert!(matches!(result, Err(TeamError::BlockedTaskNotFound(_))));
    }

    #[tokio::test]
    async fn update_task_status() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task = create_simple_task(&board, "t1", "Work").await;
        let updated = board
            .update_task(
                "t1",
                &task.id,
                &TaskUpdate {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn update_task_description_and_owner() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task = create_simple_task(&board, "t1", "Work").await;
        let updated = board
            .update_task(
                "t1",
                &task.id,
                &TaskUpdate {
                    description: Some("New desc".into()),
                    owner: Some("a2".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.description.as_deref(), Some("New desc"));
        assert_eq!(updated.owner.as_deref(), Some("a2"));
    }

    #[tokio::test]
    async fn update_nonexistent_task_fails() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let result = board.update_task("t1", "nonexistent", &TaskUpdate::default()).await;
        assert!(matches!(result, Err(TeamError::TaskNotFound(_))));
    }

    #[tokio::test]
    async fn complete_task_unblocks_downstream() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task_a = create_simple_task(&board, "t1", "A").await;
        let task_b = board
            .create_task("t1", "B", None, None, std::slice::from_ref(&task_a.id))
            .await
            .unwrap();

        assert_eq!(task_b.blocked_by, vec![task_a.id.clone()]);

        board
            .update_task(
                "t1",
                &task_a.id,
                &TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let tasks = board.list_tasks("t1").await.unwrap();
        let b = tasks.iter().find(|t| t.id == task_b.id).unwrap();
        assert!(b.blocked_by.is_empty());
    }

    #[tokio::test]
    async fn complete_task_unblocks_multiple_downstream() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task_a = create_simple_task(&board, "t1", "A").await;
        let task_b = board
            .create_task("t1", "B", None, None, std::slice::from_ref(&task_a.id))
            .await
            .unwrap();
        let task_c = board
            .create_task("t1", "C", None, None, std::slice::from_ref(&task_a.id))
            .await
            .unwrap();

        board
            .update_task(
                "t1",
                &task_a.id,
                &TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let tasks = board.list_tasks("t1").await.unwrap();
        let b = tasks.iter().find(|t| t.id == task_b.id).unwrap();
        let c = tasks.iter().find(|t| t.id == task_c.id).unwrap();
        assert!(b.blocked_by.is_empty());
        assert!(c.blocked_by.is_empty());
    }

    #[tokio::test]
    async fn partial_unblock_preserves_other_dependencies() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task_a = create_simple_task(&board, "t1", "A").await;
        let task_x = create_simple_task(&board, "t1", "X").await;
        let task_b = board
            .create_task("t1", "B", None, None, &[task_a.id.clone(), task_x.id.clone()])
            .await
            .unwrap();

        assert_eq!(task_b.blocked_by.len(), 2);

        board
            .update_task(
                "t1",
                &task_a.id,
                &TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let tasks = board.list_tasks("t1").await.unwrap();
        let b = tasks.iter().find(|t| t.id == task_b.id).unwrap();
        assert_eq!(b.blocked_by, vec![task_x.id]);
    }

    #[tokio::test]
    async fn complete_task_no_downstream_is_noop() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let task = create_simple_task(&board, "t1", "Standalone").await;
        let updated = board
            .update_task(
                "t1",
                &task.id,
                &TaskUpdate {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn list_tasks_empty() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        let tasks = board.list_tasks("t1").await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn list_tasks_returns_all() {
        let repo = Arc::new(MockTeamRepo::new());
        let board = TaskBoard::new(repo);

        create_simple_task(&board, "t1", "A").await;
        create_simple_task(&board, "t1", "B").await;
        create_simple_task(&board, "t2", "C").await;

        let tasks = board.list_tasks("t1").await.unwrap();
        assert_eq!(tasks.len(), 2);
    }
}
