use nomifun_common::now_ms;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{CronJobRow, CronJobRunRow};
use crate::repository::bind::{BindValue, bind_value};
use crate::repository::cron::{CRON_RUN_HISTORY_LIMIT, ICronRepository, UpdateCronJobParams};

#[derive(Clone, Debug)]
pub struct SqliteCronRepository {
    pool: SqlitePool,
}

impl SqliteCronRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ICronRepository for SqliteCronRepository {
    async fn insert(&self, row: &CronJobRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO cron_jobs (\
                id, user_id, name, enabled, schedule_kind, schedule_value, schedule_tz, \
                schedule_description, payload_message, execution_mode, agent_config, \
                preset_id, preset_revision, preset_snapshot, \
                conversation_id, conversation_title, agent_type, created_by, \
                skill_content, description, created_at, updated_at, next_run_at, last_run_at, \
                last_status, last_error, run_count, retry_count, max_retries\
            ) VALUES (\
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?\
            )",
        )
        .bind(&row.id)
        .bind(&row.user_id)
        .bind(&row.name)
        .bind(row.enabled)
        .bind(&row.schedule_kind)
        .bind(&row.schedule_value)
        .bind(&row.schedule_tz)
        .bind(&row.schedule_description)
        .bind(&row.payload_message)
        .bind(&row.execution_mode)
        .bind(&row.agent_config)
        .bind(&row.preset_id)
        .bind(row.preset_revision)
        .bind(&row.preset_snapshot)
        .bind(&row.conversation_id)
        .bind(&row.conversation_title)
        .bind(&row.agent_type)
        .bind(&row.created_by)
        .bind(&row.skill_content)
        .bind(&row.description)
        .bind(row.created_at)
        .bind(row.updated_at)
        .bind(row.next_run_at)
        .bind(row.last_run_at)
        .bind(&row.last_status)
        .bind(&row.last_error)
        .bind(row.run_count)
        .bind(row.retry_count)
        .bind(row.max_retries)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update(
        &self,
        user_id: &str,
        id: &str,
        params: &UpdateCronJobParams,
    ) -> Result<(), DbError> {
        let mut set_parts: Vec<String> = Vec::new();
        let mut binds: Vec<BindValue> = Vec::new();

        macro_rules! push_str {
            ($field:ident) => {
                if let Some(ref v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::Str(v.clone()));
                }
            };
        }

        macro_rules! push_opt_str {
            ($field:ident) => {
                if let Some(ref v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::OptStr(v.clone()));
                }
            };
        }

        macro_rules! push_opt_i64 {
            ($field:ident) => {
                if let Some(ref v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::OptI64(*v));
                }
            };
        }

        macro_rules! push_i64 {
            ($field:ident) => {
                if let Some(v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::I64(v));
                }
            };
        }

        if let Some(v) = params.enabled {
            set_parts.push("enabled = ?".to_string());
            binds.push(BindValue::Bool(v));
        }

        push_str!(name);
        push_str!(schedule_kind);
        push_str!(schedule_value);
        push_opt_str!(schedule_tz);
        push_opt_str!(schedule_description);
        push_str!(payload_message);
        push_str!(execution_mode);
        push_opt_str!(agent_config);
        push_opt_str!(preset_id);
        push_opt_i64!(preset_revision);
        push_opt_str!(preset_snapshot);
        push_opt_i64!(conversation_id);
        push_opt_str!(conversation_title);
        push_str!(agent_type);
        push_opt_str!(skill_content);
        push_opt_str!(description);
        push_opt_i64!(next_run_at);
        push_opt_i64!(last_run_at);
        push_opt_str!(last_status);
        push_opt_str!(last_error);
        push_i64!(run_count);
        push_i64!(retry_count);

        if set_parts.is_empty() {
            let owned: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE id = ? AND user_id = ?)",
            )
            .bind(id)
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?;
            return if owned {
                Ok(())
            } else {
                Err(DbError::NotFound(format!("cron job '{id}'")))
            };
        }

        set_parts.push("updated_at = ?".to_string());
        binds.push(BindValue::I64(now_ms()));

        let sql = format!(
            "UPDATE cron_jobs SET {} WHERE id = ? AND user_id = ?",
            set_parts.join(", ")
        );

        let mut query = sqlx::query(&sql);
        for bind in &binds {
            query = bind_value(query, bind);
        }
        query = query.bind(id);
        query = query.bind(user_id);

        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("cron job '{id}'")));
        }
        Ok(())
    }

    async fn delete(&self, user_id: &str, id: &str) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("cron job '{id}'")));
        }
        Ok(())
    }

    async fn get_by_id(&self, user_id: &str, id: &str) -> Result<Option<CronJobRow>, DbError> {
        let row = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE id = ? AND user_id = ?",
        )
            .bind(id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list_all(&self, user_id: &str) -> Result<Vec<CronJobRow>, DbError> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE user_id = ? ORDER BY created_at ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get_by_id_for_scheduler(&self, id: &str) -> Result<Option<CronJobRow>, DbError> {
        let row = sqlx::query_as::<_, CronJobRow>("SELECT * FROM cron_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list_enabled_for_scheduler(&self) -> Result<Vec<CronJobRow>, DbError> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE enabled = 1 ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_by_conversation(
        &self,
        user_id: &str,
        conversation_id: i64,
    ) -> Result<Vec<CronJobRow>, DbError> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE user_id = ? AND conversation_id = ? ORDER BY created_at ASC",
        )
        .bind(user_id)
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete_by_conversation(
        &self,
        user_id: &str,
        conversation_id: i64,
    ) -> Result<u64, DbError> {
        let result = sqlx::query(
            "DELETE FROM cron_jobs WHERE user_id = ? AND conversation_id = ?",
        )
            .bind(user_id)
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    async fn insert_run_pruned(
        &self,
        user_id: &str,
        row: &CronJobRunRow,
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;

        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE id = ? AND user_id = ?)",
        )
        .bind(&row.job_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if !owned {
            return Err(DbError::NotFound(format!("cron job '{}'", row.job_id)));
        }

        sqlx::query(
            "INSERT INTO cron_job_runs (id, job_id, executed_at_ms, status, created_at_ms) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.job_id)
        .bind(row.executed_at_ms)
        .bind(&row.status)
        .bind(row.created_at_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM cron_job_runs \
             WHERE job_id = ? \
             AND id NOT IN (\
                 SELECT id FROM cron_job_runs \
                 WHERE job_id = ? \
                 ORDER BY executed_at_ms DESC, created_at_ms DESC, id DESC \
                 LIMIT ?\
             )",
        )
        .bind(&row.job_id)
        .bind(&row.job_id)
        .bind(CRON_RUN_HISTORY_LIMIT)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn list_runs_by_job(
        &self,
        user_id: &str,
        job_id: &str,
        limit: i64,
    ) -> Result<Vec<CronJobRunRow>, DbError> {
        let limit = limit.clamp(0, CRON_RUN_HISTORY_LIMIT);
        let rows = sqlx::query_as::<_, CronJobRunRow>(
            "SELECT run.* FROM cron_job_runs run \
             JOIN cron_jobs job ON job.id = run.job_id \
             WHERE run.job_id = ? AND job.user_id = ? \
             ORDER BY run.executed_at_ms DESC, run.created_at_ms DESC, run.id DESC \
             LIMIT ?",
        )
        .bind(job_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;
    use crate::models::CronJobRunRow;

    const INSTALLATION_OWNER: &str = "system_default_user";

    async fn setup() -> (SqliteCronRepository, crate::Database) {
        let db = init_database_memory().await.expect("init db");
        let repo = SqliteCronRepository::new(db.pool().clone());

        // General Cron repository tests exercise the complete host-capable
        // shape, so their aggregate and target Conversation explicitly belong
        // to the installation owner seeded by the baseline migration.
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
             VALUES (1, ?1, 'Test Conv', 'normal', 0, 0)",
        )
        .bind(INSTALLATION_OWNER)
        .execute(db.pool())
        .await
        .unwrap();

        (repo, db)
    }

    fn make_row(id: &str) -> CronJobRow {
        let now = now_ms();
        CronJobRow {
            id: id.into(),
            user_id: INSTALLATION_OWNER.into(),
            name: "Test Job".into(),
            enabled: true,
            schedule_kind: "every".into(),
            schedule_value: "60000".into(),
            schedule_tz: None,
            schedule_description: Some("Every minute".into()),
            payload_message: "ping".into(),
            execution_mode: "existing".into(),
            agent_config: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            conversation_id: Some(1),
            conversation_title: Some("Test Conv".into()),
            agent_type: "acp".into(),
            created_by: "user".into(),
            skill_content: None,
            description: None,
            created_at: now,
            updated_at: now,
            next_run_at: Some(now + 60_000),
            last_run_at: None,
            last_status: None,
            last_error: None,
            run_count: 0,
            retry_count: 0,
            max_retries: 3,
        }
    }

    fn make_run(job_id: &str, index: i64) -> CronJobRunRow {
        CronJobRunRow {
            id: format!("cron_run_{job_id}_{index}"),
            job_id: job_id.to_owned(),
            executed_at_ms: 1_000 + index,
            status: if index % 2 == 0 { "ok" } else { "error" }.to_owned(),
            created_at_ms: 2_000 + index,
        }
    }

    #[tokio::test]
    async fn insert_run_pruned_keeps_latest_seven_per_job() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_runs_a")).await.unwrap();
        repo.insert(&make_row("cron_runs_b")).await.unwrap();

        for index in 0..10 {
            repo.insert_run_pruned(INSTALLATION_OWNER, &make_run("cron_runs_a", index))
                .await
                .unwrap();
        }
        for index in 0..3 {
            repo.insert_run_pruned(INSTALLATION_OWNER, &make_run("cron_runs_b", index))
                .await
                .unwrap();
        }

        let runs_a = repo
            .list_runs_by_job(INSTALLATION_OWNER, "cron_runs_a", 20)
            .await
            .unwrap();
        let runs_b = repo
            .list_runs_by_job(INSTALLATION_OWNER, "cron_runs_b", 20)
            .await
            .unwrap();

        assert_eq!(runs_a.len(), 7);
        assert_eq!(runs_a[0].executed_at_ms, 1_009);
        assert_eq!(runs_a[6].executed_at_ms, 1_003);
        assert!(runs_a.iter().all(|run| run.job_id == "cron_runs_a"));

        assert_eq!(runs_b.len(), 3);
        assert_eq!(runs_b[0].executed_at_ms, 1_002);
        assert_eq!(runs_b[2].executed_at_ms, 1_000);
    }

    #[tokio::test]
    async fn insert_and_get_by_id() {
        let (repo, _db) = setup().await;
        let row = make_row("cron_1");
        repo.insert(&row).await.unwrap();

        let found = repo
            .get_by_id(INSTALLATION_OWNER, "cron_1")
            .await
            .unwrap()
            .expect("found");
        assert_eq!(found.id, "cron_1");
        assert_eq!(found.name, "Test Job");
        assert!(found.enabled);
        assert_eq!(found.schedule_kind, "every");
        assert_eq!(found.run_count, 0);
    }

    #[tokio::test]
    async fn get_by_id_returns_none_for_missing() {
        let (repo, _db) = setup().await;
        let result = repo.get_by_id(INSTALLATION_OWNER, "cron_missing").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_all_returns_all_rows() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_a")).await.unwrap();
        repo.insert(&make_row("cron_b")).await.unwrap();

        let all = repo.list_all(INSTALLATION_OWNER).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_enabled_filters_disabled() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_e1")).await.unwrap();

        let mut disabled = make_row("cron_e2");
        disabled.enabled = false;
        repo.insert(&disabled).await.unwrap();

        let enabled = repo.list_enabled_for_scheduler().await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, "cron_e1");
    }

    #[tokio::test]
    async fn list_by_conversation_filters_correctly() {
        let (repo, db) = setup().await;
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
             VALUES (2, ?1, 'Other', 'normal', 0, 0)",
        )
        .bind(INSTALLATION_OWNER)
        .execute(db.pool())
        .await
        .unwrap();

        repo.insert(&make_row("cron_c1")).await.unwrap();
        let mut other = make_row("cron_c2");
        other.conversation_id = Some(2);
        repo.insert(&other).await.unwrap();

        let conv1_jobs = repo.list_by_conversation(INSTALLATION_OWNER, 1).await.unwrap();
        assert_eq!(conv1_jobs.len(), 1);
        assert_eq!(conv1_jobs[0].id, "cron_c1");

        let conv2_jobs = repo.list_by_conversation(INSTALLATION_OWNER, 2).await.unwrap();
        assert_eq!(conv2_jobs.len(), 1);
        assert_eq!(conv2_jobs[0].id, "cron_c2");
    }

    #[tokio::test]
    async fn update_partial_fields() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_u1")).await.unwrap();

        let params = UpdateCronJobParams {
            name: Some("Renamed".into()),
            enabled: Some(false),
            run_count: Some(42),
            ..Default::default()
        };
        repo.update(INSTALLATION_OWNER, "cron_u1", &params).await.unwrap();

        let updated = repo
            .get_by_id(INSTALLATION_OWNER, "cron_u1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert!(!updated.enabled);
        assert_eq!(updated.run_count, 42);
        assert!(updated.updated_at >= updated.created_at);
    }

    #[tokio::test]
    async fn update_optional_nullable_fields() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_u2")).await.unwrap();

        let params = UpdateCronJobParams {
            last_status: Some(Some("ok".into())),
            last_error: Some(Some("timeout".into())),
            skill_content: Some(Some("---\nname: skill\n---\nDo it".into())),
            ..Default::default()
        };
        repo.update(INSTALLATION_OWNER, "cron_u2", &params).await.unwrap();

        let updated = repo
            .get_by_id(INSTALLATION_OWNER, "cron_u2")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.last_status.as_deref(), Some("ok"));
        assert_eq!(updated.last_error.as_deref(), Some("timeout"));
        assert!(updated.skill_content.is_some());

        let clear_params = UpdateCronJobParams {
            last_status: Some(None),
            last_error: Some(None),
            skill_content: Some(None),
            ..Default::default()
        };
        repo.update(INSTALLATION_OWNER, "cron_u2", &clear_params)
            .await
            .unwrap();

        let cleared = repo
            .get_by_id(INSTALLATION_OWNER, "cron_u2")
            .await
            .unwrap()
            .unwrap();
        assert!(cleared.last_status.is_none());
        assert!(cleared.last_error.is_none());
        assert!(cleared.skill_content.is_none());
    }

    #[tokio::test]
    async fn update_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let params = UpdateCronJobParams {
            name: Some("x".into()),
            ..Default::default()
        };
        let err = repo
            .update(INSTALLATION_OWNER, "cron_nope", &params)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_empty_params_is_noop() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_noop")).await.unwrap();

        let before = repo
            .get_by_id(INSTALLATION_OWNER, "cron_noop")
            .await
            .unwrap()
            .unwrap();
        repo.update(INSTALLATION_OWNER, "cron_noop", &UpdateCronJobParams::default())
            .await
            .unwrap();
        let after = repo
            .get_by_id(INSTALLATION_OWNER, "cron_noop")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(before.updated_at, after.updated_at);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_d1")).await.unwrap();

        repo.delete(INSTALLATION_OWNER, "cron_d1").await.unwrap();
        let result = repo.get_by_id(INSTALLATION_OWNER, "cron_d1").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.delete(INSTALLATION_OWNER, "cron_nope").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_by_conversation_removes_all_related() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_dc1")).await.unwrap();
        repo.insert(&make_row("cron_dc2")).await.unwrap();

        let deleted = repo.delete_by_conversation(INSTALLATION_OWNER, 1).await.unwrap();
        assert_eq!(deleted, 2);

        let remaining = repo.list_all(INSTALLATION_OWNER).await.unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn delete_by_conversation_returns_zero_for_no_match() {
        let (repo, _db) = setup().await;
        let deleted = repo
            .delete_by_conversation(INSTALLATION_OWNER, 999)
            .await
            .unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn update_schedule_fields() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("cron_s1")).await.unwrap();

        let params = UpdateCronJobParams {
            schedule_kind: Some("cron".into()),
            schedule_value: Some("0 0 9 * * *".into()),
            schedule_tz: Some(Some("Asia/Shanghai".into())),
            schedule_description: Some(Some("Daily at 9am".into())),
            next_run_at: Some(Some(9999999)),
            ..Default::default()
        };
        repo.update(INSTALLATION_OWNER, "cron_s1", &params).await.unwrap();

        let updated = repo
            .get_by_id(INSTALLATION_OWNER, "cron_s1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.schedule_kind, "cron");
        assert_eq!(updated.schedule_value, "0 0 9 * * *");
        assert_eq!(updated.schedule_tz.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(updated.next_run_at, Some(9999999));
    }

    #[tokio::test]
    async fn insert_all_schedule_kinds() {
        let (repo, _db) = setup().await;

        let mut at_job = make_row("cron_at");
        at_job.schedule_kind = "at".into();
        at_job.schedule_value = "1700000000000".into();
        repo.insert(&at_job).await.unwrap();

        let mut cron_job = make_row("cron_cron");
        cron_job.schedule_kind = "cron".into();
        cron_job.schedule_value = "0 */5 * * * *".into();
        cron_job.schedule_tz = Some("UTC".into());
        repo.insert(&cron_job).await.unwrap();

        let all = repo.list_all(INSTALLATION_OWNER).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn insert_with_skill_content() {
        let (repo, _db) = setup().await;
        let mut row = make_row("cron_sk");
        row.skill_content = Some("---\nname: My Skill\ndescription: A test\n---\nDo X".into());
        repo.insert(&row).await.unwrap();

        let found = repo
            .get_by_id(INSTALLATION_OWNER, "cron_sk")
            .await
            .unwrap()
            .unwrap();
        assert!(found.skill_content.unwrap().contains("My Skill"));
    }

    #[tokio::test]
    async fn insert_with_agent_config_json() {
        let (repo, _db) = setup().await;
        let mut row = make_row("cron_ac");
        row.agent_config = Some(r#"{"backend":"openai","name":"GPT","modelId":"gpt-4"}"#.into());
        repo.insert(&row).await.unwrap();

        let found = repo
            .get_by_id(INSTALLATION_OWNER, "cron_ac")
            .await
            .unwrap()
            .unwrap();
        let config = found.agent_config.unwrap();
        assert!(config.contains("openai"));
        assert!(config.contains("gpt-4"));
    }

    #[tokio::test]
    async fn secondary_user_cron_accepts_model_only_shape_and_rejects_host_agent() {
        let (repo, db) = setup().await;
        const SECONDARY_USER: &str = "cron_secondary_user";

        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
             VALUES (?1, 'cron_secondary', 'hash', 0, 0)",
        )
        .bind(SECONDARY_USER)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO providers (\
                id, platform, name, base_url, api_key_encrypted, models, enabled, \
                capabilities, created_at, updated_at\
             ) VALUES (\
                'provider-test', 'openai', 'Provider Test', 'https://example.invalid', \
                'encrypted', '[]', 1, '[]', 0, 0\
             )",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations \
                (id, user_id, name, type, model, delegation_policy, created_at, updated_at) \
             VALUES \
                (2, ?1, 'Model-only target', 'nomi', \
                 '{\"provider_id\":\"provider-test\",\"model\":\"model-test\"}', \
                 'disabled', 0, 0)",
        )
        .bind(SECONDARY_USER)
        .execute(db.pool())
        .await
        .unwrap();

        let mut allowed = make_row("cron_secondary_model_only");
        allowed.user_id = SECONDARY_USER.into();
        allowed.conversation_id = Some(2);
        allowed.conversation_title = Some("Model-only target".into());
        allowed.agent_type = "nomi".into();
        repo.insert(&allowed).await.unwrap();
        assert_eq!(repo.list_all(SECONDARY_USER).await.unwrap().len(), 1);

        let mut rejected = allowed;
        rejected.id = "cron_secondary_host_agent".into();
        rejected.agent_type = "acp".into();
        let err = repo.insert(&rejected).await.unwrap_err();
        assert!(
            err.to_string().contains("non-owner cron job must be model-only"),
            "unexpected authority error: {err}"
        );
    }
}
