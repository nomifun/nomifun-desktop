use nomifun_common::{generate_prefixed_id, now_ms};
use sqlx::SqlitePool;

use crate::models::{OrchAssignmentRow, OrchRunRow, OrchRunTaskDepRow, OrchRunTaskRow};
use crate::repository::orch_run::{
    CreateAssignmentParams, CreateRunParams, CreateTaskParams, IRunRepository, UpdateRunParams,
    UpdateTaskParams,
};

#[derive(Clone, Debug)]
pub struct SqliteRunRepository {
    pool: SqlitePool,
}

impl SqliteRunRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IRunRepository for SqliteRunRepository {
    // --- runs ---

    async fn create_run(&self, p: CreateRunParams) -> Result<OrchRunRow, sqlx::Error> {
        let id = generate_prefixed_id("run");
        let now = now_ms();
        let status = "planning".to_string();
        sqlx::query(
            "INSERT INTO orch_runs (\
                id, workspace_id, user_id, goal, fleet_snapshot, autonomy, max_parallel, \
                lead_conv_id, status, summary, total_tokens, forked_from, work_dir, \
                created_at, updated_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.workspace_id)
        .bind(&p.user_id)
        .bind(&p.goal)
        .bind(&p.fleet_snapshot)
        .bind(&p.autonomy)
        .bind(p.max_parallel)
        .bind(p.lead_conv_id)
        .bind(&status)
        .bind(&p.work_dir)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(OrchRunRow {
            id,
            workspace_id: p.workspace_id,
            user_id: p.user_id,
            goal: p.goal,
            fleet_snapshot: p.fleet_snapshot,
            autonomy: p.autonomy,
            max_parallel: p.max_parallel,
            lead_conv_id: p.lead_conv_id,
            status,
            summary: None,
            total_tokens: None,
            forked_from: None,
            work_dir: p.work_dir,
            created_at: now,
            updated_at: now,
        })
    }

    async fn get_run(&self, id: &str) -> Result<Option<OrchRunRow>, sqlx::Error> {
        let row = sqlx::query_as::<_, OrchRunRow>("SELECT * FROM orch_runs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list_runs(&self, workspace_id: &str) -> Result<Vec<OrchRunRow>, sqlx::Error> {
        let rows = sqlx::query_as::<_, OrchRunRow>(
            "SELECT * FROM orch_runs WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_runs_by_status(&self, status: &str) -> Result<Vec<OrchRunRow>, sqlx::Error> {
        let rows = sqlx::query_as::<_, OrchRunRow>(
            "SELECT * FROM orch_runs WHERE status = ? ORDER BY created_at ASC",
        )
        .bind(status)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn update_run(&self, id: &str, p: UpdateRunParams) -> Result<(), sqlx::Error> {
        // Build the SET clause conservatively: only touch columns the caller
        // actually supplied. For nullable columns `None` = skip,
        // `Some(None)` = set NULL, `Some(Some(v))` = set v. `status` is a plain
        // non-null column (`None` = skip, `Some(v)` = set v).
        let mut sets: Vec<&str> = Vec::new();
        if p.status.is_some() {
            sets.push("status = ?");
        }
        if p.summary.is_some() {
            sets.push("summary = ?");
        }
        if p.lead_conv_id.is_some() {
            sets.push("lead_conv_id = ?");
        }
        if p.total_tokens.is_some() {
            sets.push("total_tokens = ?");
        }
        if sets.is_empty() {
            return Ok(());
        }
        sets.push("updated_at = ?");
        let sql = format!("UPDATE orch_runs SET {} WHERE id = ?", sets.join(", "));

        let mut q = sqlx::query(&sql);
        if let Some(status) = &p.status {
            q = q.bind(status);
        }
        if let Some(summary) = &p.summary {
            q = q.bind(summary);
        }
        if let Some(lead_conv_id) = &p.lead_conv_id {
            q = q.bind(lead_conv_id);
        }
        if let Some(total_tokens) = &p.total_tokens {
            q = q.bind(total_tokens);
        }
        q = q.bind(now_ms());
        q = q.bind(id);
        q.execute(&self.pool).await?;
        Ok(())
    }

    // --- tasks ---

    async fn create_task(&self, p: CreateTaskParams) -> Result<OrchRunTaskRow, sqlx::Error> {
        let id = generate_prefixed_id("rtask");
        let now = now_ms();
        sqlx::query(
            "INSERT INTO orch_run_tasks (\
                id, run_id, title, spec, task_profile, status, conversation_id, \
                output_summary, output_files, attempt, tokens, graph_x, graph_y, \
                created_at, updated_at\
            ) VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, NULL, 0, NULL, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.run_id)
        .bind(&p.title)
        .bind(&p.spec)
        .bind(&p.task_profile)
        .bind(&p.status)
        .bind(p.graph_x)
        .bind(p.graph_y)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(OrchRunTaskRow {
            id,
            run_id: p.run_id,
            title: p.title,
            spec: p.spec,
            task_profile: p.task_profile,
            status: p.status,
            conversation_id: None,
            output_summary: None,
            output_files: None,
            attempt: 0,
            tokens: None,
            graph_x: p.graph_x,
            graph_y: p.graph_y,
            created_at: now,
            updated_at: now,
        })
    }

    async fn list_tasks(&self, run_id: &str) -> Result<Vec<OrchRunTaskRow>, sqlx::Error> {
        let rows = sqlx::query_as::<_, OrchRunTaskRow>(
            "SELECT * FROM orch_run_tasks WHERE run_id = ? ORDER BY created_at ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get_task(&self, id: &str) -> Result<Option<OrchRunTaskRow>, sqlx::Error> {
        let row = sqlx::query_as::<_, OrchRunTaskRow>("SELECT * FROM orch_run_tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn update_task(&self, id: &str, p: UpdateTaskParams) -> Result<(), sqlx::Error> {
        // Nullable columns use double-`Option` (skip/NULL/set); `status`,
        // `attempt`, `graph_x`, `graph_y` are plain skip-on-`None`.
        let mut sets: Vec<&str> = Vec::new();
        if p.status.is_some() {
            sets.push("status = ?");
        }
        if p.conversation_id.is_some() {
            sets.push("conversation_id = ?");
        }
        if p.output_summary.is_some() {
            sets.push("output_summary = ?");
        }
        if p.output_files.is_some() {
            sets.push("output_files = ?");
        }
        if p.attempt.is_some() {
            sets.push("attempt = ?");
        }
        if p.tokens.is_some() {
            sets.push("tokens = ?");
        }
        if p.graph_x.is_some() {
            sets.push("graph_x = ?");
        }
        if p.graph_y.is_some() {
            sets.push("graph_y = ?");
        }
        if sets.is_empty() {
            return Ok(());
        }
        sets.push("updated_at = ?");
        let sql = format!("UPDATE orch_run_tasks SET {} WHERE id = ?", sets.join(", "));

        let mut q = sqlx::query(&sql);
        if let Some(status) = &p.status {
            q = q.bind(status);
        }
        if let Some(conversation_id) = &p.conversation_id {
            q = q.bind(conversation_id);
        }
        if let Some(output_summary) = &p.output_summary {
            q = q.bind(output_summary);
        }
        if let Some(output_files) = &p.output_files {
            q = q.bind(output_files);
        }
        if let Some(attempt) = &p.attempt {
            q = q.bind(attempt);
        }
        if let Some(tokens) = &p.tokens {
            q = q.bind(tokens);
        }
        if let Some(graph_x) = &p.graph_x {
            q = q.bind(graph_x);
        }
        if let Some(graph_y) = &p.graph_y {
            q = q.bind(graph_y);
        }
        q = q.bind(now_ms());
        q = q.bind(id);
        q.execute(&self.pool).await?;
        Ok(())
    }

    // --- deps ---

    async fn add_dep(&self, blocker: &str, blocked: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO orch_run_task_deps (blocker_task_id, blocked_task_id) VALUES (?, ?)",
        )
        .bind(blocker)
        .bind(blocked)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_deps(&self, run_id: &str) -> Result<Vec<OrchRunTaskDepRow>, sqlx::Error> {
        // Scope to a run by joining the blocked task back to its run.
        let rows = sqlx::query_as::<_, OrchRunTaskDepRow>(
            "SELECT d.blocker_task_id, d.blocked_task_id FROM orch_run_task_deps d \
             JOIN orch_run_tasks t ON t.id = d.blocked_task_id \
             WHERE t.run_id = ?",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_ready_tasks(&self, run_id: &str) -> Result<Vec<OrchRunTaskRow>, sqlx::Error> {
        // Ready = pending AND no incomplete blocker. A task with zero dep rows
        // trivially satisfies the NOT EXISTS (the subquery is empty); a task
        // with multiple blockers is ready only when EVERY blocker is 'done'
        // (any single non-'done' blocker makes the subquery non-empty).
        let rows = sqlx::query_as::<_, OrchRunTaskRow>(
            "SELECT t.* FROM orch_run_tasks t \
             WHERE t.run_id = ? AND t.status = 'pending' AND NOT EXISTS (\
                 SELECT 1 FROM orch_run_task_deps d \
                 JOIN orch_run_tasks bt ON bt.id = d.blocker_task_id \
                 WHERE d.blocked_task_id = t.id AND bt.status != 'done'\
             ) ORDER BY t.created_at ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // --- assignments ---

    async fn create_assignment(
        &self,
        p: CreateAssignmentParams,
    ) -> Result<OrchAssignmentRow, sqlx::Error> {
        let id = generate_prefixed_id("asg");
        let now = now_ms();
        let locked: i64 = if p.locked { 1 } else { 0 };
        sqlx::query(
            "INSERT INTO orch_assignments (\
                id, task_id, member_id, score, rationale, source, locked, created_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.task_id)
        .bind(&p.member_id)
        .bind(p.score)
        .bind(&p.rationale)
        .bind(&p.source)
        .bind(locked)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(OrchAssignmentRow {
            id,
            task_id: p.task_id,
            member_id: p.member_id,
            score: p.score,
            rationale: p.rationale,
            source: p.source,
            locked,
            created_at: now,
        })
    }

    async fn set_assignment(
        &self,
        p: CreateAssignmentParams,
    ) -> Result<OrchAssignmentRow, sqlx::Error> {
        // Upsert = delete-by-task + create. A task carries at most one effective
        // assignment; an override cleanly replaces it (rather than stacking rows,
        // which `get_assignment_for_task`'s "latest" semantics would otherwise
        // mask). Done in a transaction so a reader never observes the gap between
        // the delete and the insert.
        let id = generate_prefixed_id("asg");
        let now = now_ms();
        let locked: i64 = if p.locked { 1 } else { 0 };
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM orch_assignments WHERE task_id = ?")
            .bind(&p.task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO orch_assignments (\
                id, task_id, member_id, score, rationale, source, locked, created_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.task_id)
        .bind(&p.member_id)
        .bind(p.score)
        .bind(&p.rationale)
        .bind(&p.source)
        .bind(locked)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(OrchAssignmentRow {
            id,
            task_id: p.task_id,
            member_id: p.member_id,
            score: p.score,
            rationale: p.rationale,
            source: p.source,
            locked,
            created_at: now,
        })
    }

    async fn list_assignments(&self, run_id: &str) -> Result<Vec<OrchAssignmentRow>, sqlx::Error> {
        let rows = sqlx::query_as::<_, OrchAssignmentRow>(
            "SELECT a.* FROM orch_assignments a \
             JOIN orch_run_tasks t ON t.id = a.task_id \
             WHERE t.run_id = ? ORDER BY a.created_at ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get_assignment_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<OrchAssignmentRow>, sqlx::Error> {
        let row = sqlx::query_as::<_, OrchAssignmentRow>(
            "SELECT * FROM orch_assignments WHERE task_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::init_database_memory;
    use crate::repository::{
        CreateOrchWorkspaceParams, IOrchWorkspaceRepository, SqliteOrchWorkspaceRepository,
    };

    /// Helper: insert a workspace so run FK (`orch_runs.workspace_id`) holds.
    async fn make_workspace(pool: &SqlitePool) -> String {
        let ws_repo = SqliteOrchWorkspaceRepository::new(pool.clone());
        ws_repo
            .create(CreateOrchWorkspaceParams {
                user_id: "u1".into(),
                name: "工作区A".into(),
                default_fleet_id: None,
                workspace_dir: None,
                context: None,
            })
            .await
            .unwrap()
            .id
    }

    async fn done(repo: &SqliteRunRepository, task_id: &str) {
        repo.update_task(
            task_id,
            UpdateTaskParams {
                status: Some("done".into()),
                conversation_id: None,
                output_summary: None,
                output_files: None,
                attempt: None,
                tokens: None,
                graph_x: None,
                graph_y: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn run_task_dep_dag_roundtrip() {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let ws_id = make_workspace(&pool).await;
        let repo = SqliteRunRepository::new(pool);

        // --- run ---
        let run = repo
            .create_run(CreateRunParams {
                workspace_id: Some(ws_id.clone()),
                user_id: "u1".into(),
                goal: "构建编排引擎".into(),
                fleet_snapshot: "{}".into(),
                autonomy: "auto".into(),
                max_parallel: Some(2),
                lead_conv_id: None,
                work_dir: None,
            })
            .await
            .unwrap();
        assert!(run.id.starts_with("run_"), "run id prefix: {}", run.id);
        assert_eq!(run.status, "planning");
        assert!(run.lead_conv_id.is_none());

        // get_run / list_runs roundtrip
        assert_eq!(repo.get_run(&run.id).await.unwrap().unwrap().id, run.id);
        assert_eq!(repo.list_runs(&ws_id).await.unwrap().len(), 1);

        // --- 3 tasks A,B,C ---
        let mk = |title: &str| CreateTaskParams {
            run_id: run.id.clone(),
            title: title.into(),
            spec: format!("spec-{title}"),
            task_profile: None,
            status: "pending".into(),
            graph_x: None,
            graph_y: None,
        };
        let a = repo.create_task(mk("A")).await.unwrap();
        let b = repo.create_task(mk("B")).await.unwrap();
        let c = repo.create_task(mk("C")).await.unwrap();
        assert!(a.id.starts_with("rtask_"), "task id prefix: {}", a.id);
        assert_eq!(a.attempt, 0);
        assert_eq!(repo.list_tasks(&run.id).await.unwrap().len(), 3);
        assert_eq!(repo.get_task(&b.id).await.unwrap().unwrap().title, "B");

        // --- deps: A→B, B→C (A blocks B, B blocks C) ---
        repo.add_dep(&a.id, &b.id).await.unwrap();
        repo.add_dep(&b.id, &c.id).await.unwrap();
        let deps = repo.list_deps(&run.id).await.unwrap();
        assert_eq!(deps.len(), 2);

        // initial: only A is ready (B blocked by A, C blocked by B)
        let ready: Vec<String> = repo
            .list_ready_tasks(&run.id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ready, vec![a.id.clone()], "only A ready initially");

        // A done → B ready (A is no longer an incomplete blocker)
        done(&repo, &a.id).await;
        let ready: Vec<String> = repo
            .list_ready_tasks(&run.id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ready, vec![b.id.clone()], "B ready after A done");

        // B done → C ready
        done(&repo, &b.id).await;
        let ready: Vec<String> = repo
            .list_ready_tasks(&run.id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ready, vec![c.id.clone()], "C ready after B done");

        // --- assignment roundtrip ---
        let asg = repo
            .create_assignment(CreateAssignmentParams {
                task_id: c.id.clone(),
                member_id: "fmem_x".into(),
                score: Some(0.87),
                rationale: Some("最强后端".into()),
                source: "auto".into(),
                locked: true,
            })
            .await
            .unwrap();
        assert!(asg.id.starts_with("asg_"), "asg id prefix: {}", asg.id);
        assert_eq!(asg.locked, 1);
        let got = repo
            .get_assignment_for_task(&c.id)
            .await
            .unwrap()
            .expect("assignment exists");
        assert_eq!(got.id, asg.id);
        assert_eq!(got.member_id, "fmem_x");
        assert_eq!(repo.list_assignments(&run.id).await.unwrap().len(), 1);

        // --- update_run + list_runs_by_status('running') ---
        repo.update_run(
            &run.id,
            UpdateRunParams {
                status: Some("running".into()),
                summary: Some(Some("进行中".into())),
                lead_conv_id: Some(Some(42)),
                total_tokens: None,
            },
        )
        .await
        .unwrap();
        let running = repo.list_runs_by_status("running").await.unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, run.id);
        let refreshed = repo.get_run(&run.id).await.unwrap().unwrap();
        assert_eq!(refreshed.status, "running");
        assert_eq!(refreshed.summary.as_deref(), Some("进行中"));
        assert_eq!(refreshed.lead_conv_id, Some(42));
        assert!(refreshed.total_tokens.is_none(), "total_tokens skipped");
    }

    #[tokio::test]
    async fn no_dep_task_is_ready_and_multi_blocker_gating() {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let ws_id = make_workspace(&pool).await;
        let repo = SqliteRunRepository::new(pool);
        let run = repo
            .create_run(CreateRunParams {
                workspace_id: Some(ws_id),
                user_id: "u1".into(),
                goal: "g".into(),
                fleet_snapshot: "{}".into(),
                autonomy: "auto".into(),
                max_parallel: None,
                lead_conv_id: None,
                work_dir: None,
            })
            .await
            .unwrap();
        let mk = |title: &str| CreateTaskParams {
            run_id: run.id.clone(),
            title: title.into(),
            spec: "s".into(),
            task_profile: None,
            status: "pending".into(),
            graph_x: None,
            graph_y: None,
        };
        // p1, p2 are independent blockers of c; standalone has no deps.
        let p1 = repo.create_task(mk("p1")).await.unwrap();
        let p2 = repo.create_task(mk("p2")).await.unwrap();
        let c = repo.create_task(mk("c")).await.unwrap();
        let standalone = repo.create_task(mk("standalone")).await.unwrap();
        repo.add_dep(&p1.id, &c.id).await.unwrap();
        repo.add_dep(&p2.id, &c.id).await.unwrap();

        // standalone (zero deps) + both blockers are ready; c is gated.
        let ready: Vec<String> = repo
            .list_ready_tasks(&run.id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ready, vec![p1.id.clone(), p2.id.clone(), standalone.id.clone()]);

        // Only one blocker done → c still gated by the other.
        done(&repo, &p1.id).await;
        let ready: Vec<String> = repo
            .list_ready_tasks(&run.id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(!ready.contains(&c.id), "c gated while p2 incomplete");
        assert!(ready.contains(&p2.id));

        // Both blockers done → c ready.
        done(&repo, &p2.id).await;
        let ready: Vec<String> = repo
            .list_ready_tasks(&run.id)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(ready.contains(&c.id), "c ready after both blockers done");
    }
}
