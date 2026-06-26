use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row in the `fleets` table — a named group of agents available for orchestration.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct FleetRow {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: Option<String>,
    pub max_parallel: Option<i64>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row in the `fleet_members` table — one agent enrolled in a fleet.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct FleetMemberRow {
    pub id: String,
    pub fleet_id: String,
    pub agent_id: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub role_hint: Option<String>,
    pub capability_profile: Option<String>, // JSON
    pub constraints: Option<String>,        // JSON
    pub sort_order: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row in the `orch_workspaces` table — a user workspace scoping orchestration runs.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OrchWorkspaceRow {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub default_fleet_id: Option<String>,
    pub workspace_dir: Option<String>,
    pub context: Option<String>, // JSON
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row in the `orch_runs` table — a single orchestration run (goal decomposition + execution).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OrchRunRow {
    pub id: String,
    pub workspace_id: String,
    pub user_id: String,
    pub goal: String,
    pub fleet_snapshot: String, // JSON
    pub autonomy: String,
    pub max_parallel: Option<i64>,
    /// Lead/coordinator worker conversation — local `conversations.id` INTEGER.
    pub lead_conv_id: Option<i64>,
    pub status: String,
    pub summary: Option<String>,
    pub total_tokens: Option<i64>,
    pub forked_from: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row in the `orch_run_tasks` table — one decomposed task within a run.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OrchRunTaskRow {
    pub id: String,
    pub run_id: String,
    pub title: String,
    pub spec: String,
    pub task_profile: Option<String>, // JSON
    pub status: String,
    /// Worker conversation — local `conversations.id` INTEGER.
    pub conversation_id: Option<i64>,
    pub output_summary: Option<String>, // JSON
    pub output_files: Option<String>,   // JSON
    pub attempt: i64,
    pub tokens: Option<i64>,
    pub graph_x: Option<f64>,
    pub graph_y: Option<f64>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row in the `orch_run_task_deps` table — a blocker→blocked edge in the task DAG.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OrchRunTaskDepRow {
    pub blocker_task_id: String,
    pub blocked_task_id: String,
}

/// Row in the `orch_assignments` table — a member assigned to a task (auto-scored or locked).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OrchAssignmentRow {
    pub id: String,
    pub task_id: String,
    pub member_id: String,
    pub score: Option<f64>,
    pub rationale: Option<String>,
    pub source: String,
    pub locked: i64,
    pub created_at: TimestampMs,
}

#[cfg(test)]
mod tests {
    use crate::database::init_database_memory;

    #[tokio::test]
    async fn migration_018_creates_orchestrator_tables() {
        let db = init_database_memory()
            .await
            .expect("db init runs all migrations");
        let pool = db.pool();
        // 断言 7 张表存在
        for t in [
            "fleets",
            "fleet_members",
            "orch_workspaces",
            "orch_runs",
            "orch_run_tasks",
            "orch_run_task_deps",
            "orch_assignments",
        ] {
            let row: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(t)
            .fetch_one(pool)
            .await
            .unwrap();
            assert_eq!(row.0, 1, "table {t} should exist");
        }
    }
}
