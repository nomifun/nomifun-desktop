//! SQLite-backed assistant repositories.

use nomifun_common::{TimestampMs, now_ms};
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{
    AssistantOverrideRow, AssistantRow, AssistantTagRow, CreateAssistantParams, CreateAssistantTagParams,
    UpdateAssistantParams, UpdateAssistantTagParams, UpsertOverrideParams,
};
use crate::repository::assistant::{IAssistantOverrideRepository, IAssistantRepository, IAssistantTagRepository};

/// SQLite-backed implementation of [`IAssistantRepository`].
#[derive(Clone, Debug)]
pub struct SqliteAssistantRepository {
    pool: SqlitePool,
}

impl SqliteAssistantRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn is_unique_violation(err: &dyn sqlx::error::DatabaseError) -> bool {
    err.code().is_some_and(|c| c == "2067" || c == "1555")
}

#[async_trait::async_trait]
impl IAssistantRepository for SqliteAssistantRepository {
    async fn list(&self) -> Result<Vec<AssistantRow>, DbError> {
        let rows = sqlx::query_as::<_, AssistantRow>("SELECT * FROM assistants ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    async fn get(&self, id: &str) -> Result<Option<AssistantRow>, DbError> {
        let row = sqlx::query_as::<_, AssistantRow>("SELECT * FROM assistants WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn create(&self, params: &CreateAssistantParams<'_>) -> Result<AssistantRow, DbError> {
        let now = now_ms();

        sqlx::query(
            "INSERT INTO assistants \
                (id, name, description, avatar, preset_agent_type, enabled_skills, \
                 custom_skill_names, disabled_builtin_skills, prompts, models, \
                 name_i18n, description_i18n, prompts_i18n, audience_tags, scenario_tags, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(params.id)
        .bind(params.name)
        .bind(params.description)
        .bind(params.avatar)
        .bind(params.preset_agent_type)
        .bind(params.enabled_skills)
        .bind(params.custom_skill_names)
        .bind(params.disabled_builtin_skills)
        .bind(params.prompts)
        .bind(params.models)
        .bind(params.name_i18n)
        .bind(params.description_i18n)
        .bind(params.prompts_i18n)
        .bind(params.audience_tags)
        .bind(params.scenario_tags)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db_err) if is_unique_violation(db_err.as_ref()) => {
                DbError::Conflict(format!("Assistant with id '{}' already exists", params.id))
            }
            _ => DbError::Query(e),
        })?;

        Ok(AssistantRow {
            id: params.id.to_string(),
            name: params.name.to_string(),
            description: params.description.map(String::from),
            avatar: params.avatar.map(String::from),
            preset_agent_type: params.preset_agent_type.to_string(),
            enabled_skills: params.enabled_skills.map(String::from),
            custom_skill_names: params.custom_skill_names.map(String::from),
            disabled_builtin_skills: params.disabled_builtin_skills.map(String::from),
            prompts: params.prompts.map(String::from),
            models: params.models.map(String::from),
            name_i18n: params.name_i18n.map(String::from),
            description_i18n: params.description_i18n.map(String::from),
            prompts_i18n: params.prompts_i18n.map(String::from),
            audience_tags: params.audience_tags.map(String::from),
            scenario_tags: params.scenario_tags.map(String::from),
            created_at: now,
            updated_at: now,
        })
    }

    async fn update(&self, id: &str, params: &UpdateAssistantParams<'_>) -> Result<Option<AssistantRow>, DbError> {
        let Some(existing) = self.get(id).await? else {
            return Ok(None);
        };

        let merged = merge_update(existing, params);

        sqlx::query(
            "UPDATE assistants SET \
                name = ?, description = ?, avatar = ?, preset_agent_type = ?, \
                enabled_skills = ?, custom_skill_names = ?, disabled_builtin_skills = ?, \
                prompts = ?, models = ?, name_i18n = ?, description_i18n = ?, \
                prompts_i18n = ?, audience_tags = ?, scenario_tags = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&merged.name)
        .bind(&merged.description)
        .bind(&merged.avatar)
        .bind(&merged.preset_agent_type)
        .bind(&merged.enabled_skills)
        .bind(&merged.custom_skill_names)
        .bind(&merged.disabled_builtin_skills)
        .bind(&merged.prompts)
        .bind(&merged.models)
        .bind(&merged.name_i18n)
        .bind(&merged.description_i18n)
        .bind(&merged.prompts_i18n)
        .bind(&merged.audience_tags)
        .bind(&merged.scenario_tags)
        .bind(merged.updated_at)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(Some(merged))
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM assistants WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn upsert(&self, params: &CreateAssistantParams<'_>) -> Result<AssistantRow, DbError> {
        let now = now_ms();

        sqlx::query(
            "INSERT INTO assistants \
                (id, name, description, avatar, preset_agent_type, enabled_skills, \
                 custom_skill_names, disabled_builtin_skills, prompts, models, \
                 name_i18n, description_i18n, prompts_i18n, audience_tags, scenario_tags, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                name = excluded.name, \
                description = excluded.description, \
                avatar = excluded.avatar, \
                preset_agent_type = excluded.preset_agent_type, \
                enabled_skills = excluded.enabled_skills, \
                custom_skill_names = excluded.custom_skill_names, \
                disabled_builtin_skills = excluded.disabled_builtin_skills, \
                prompts = excluded.prompts, \
                models = excluded.models, \
                name_i18n = excluded.name_i18n, \
                description_i18n = excluded.description_i18n, \
                prompts_i18n = excluded.prompts_i18n, \
                audience_tags = excluded.audience_tags, \
                scenario_tags = excluded.scenario_tags, \
                updated_at = excluded.updated_at",
        )
        .bind(params.id)
        .bind(params.name)
        .bind(params.description)
        .bind(params.avatar)
        .bind(params.preset_agent_type)
        .bind(params.enabled_skills)
        .bind(params.custom_skill_names)
        .bind(params.disabled_builtin_skills)
        .bind(params.prompts)
        .bind(params.models)
        .bind(params.name_i18n)
        .bind(params.description_i18n)
        .bind(params.prompts_i18n)
        .bind(params.audience_tags)
        .bind(params.scenario_tags)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let row = self
            .get(params.id)
            .await?
            .ok_or_else(|| DbError::Init(format!("upsert did not produce row for id '{}'", params.id)))?;
        Ok(row)
    }
}

fn merge_update(existing: AssistantRow, params: &UpdateAssistantParams<'_>) -> AssistantRow {
    let now = now_ms();
    AssistantRow {
        id: existing.id,
        name: params.name.map(String::from).unwrap_or(existing.name),
        description: params.description.map_or(existing.description, |v| v.map(String::from)),
        avatar: params.avatar.map_or(existing.avatar, |v| v.map(String::from)),
        preset_agent_type: params
            .preset_agent_type
            .map(String::from)
            .unwrap_or(existing.preset_agent_type),
        enabled_skills: params
            .enabled_skills
            .map_or(existing.enabled_skills, |v| v.map(String::from)),
        custom_skill_names: params
            .custom_skill_names
            .map_or(existing.custom_skill_names, |v| v.map(String::from)),
        disabled_builtin_skills: params
            .disabled_builtin_skills
            .map_or(existing.disabled_builtin_skills, |v| v.map(String::from)),
        prompts: params.prompts.map_or(existing.prompts, |v| v.map(String::from)),
        models: params.models.map_or(existing.models, |v| v.map(String::from)),
        name_i18n: params.name_i18n.map_or(existing.name_i18n, |v| v.map(String::from)),
        description_i18n: params
            .description_i18n
            .map_or(existing.description_i18n, |v| v.map(String::from)),
        prompts_i18n: params
            .prompts_i18n
            .map_or(existing.prompts_i18n, |v| v.map(String::from)),
        audience_tags: params.audience_tags.map_or(existing.audience_tags, |v| v.map(String::from)),
        scenario_tags: params.scenario_tags.map_or(existing.scenario_tags, |v| v.map(String::from)),
        created_at: existing.created_at,
        updated_at: now,
    }
}

/// SQLite-backed implementation of [`IAssistantOverrideRepository`].
#[derive(Clone, Debug)]
pub struct SqliteAssistantOverrideRepository {
    pool: SqlitePool,
}

impl SqliteAssistantOverrideRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IAssistantOverrideRepository for SqliteAssistantOverrideRepository {
    async fn get(&self, assistant_id: &str) -> Result<Option<AssistantOverrideRow>, DbError> {
        let row = sqlx::query_as::<_, AssistantOverrideRow>("SELECT * FROM assistant_overrides WHERE assistant_id = ?")
            .bind(assistant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn get_all(&self) -> Result<Vec<AssistantOverrideRow>, DbError> {
        let rows = sqlx::query_as::<_, AssistantOverrideRow>("SELECT * FROM assistant_overrides")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    async fn upsert(&self, params: &UpsertOverrideParams<'_>) -> Result<AssistantOverrideRow, DbError> {
        let now = now_ms();
        let last_used_at: Option<TimestampMs> = params.last_used_at;

        // `preset_agent_type` has three-way semantics in the params struct
        // (see `UpsertOverrideParams`). At the SQL layer we flatten it into a
        // `(write?, value)` pair: on CONFLICT, if the caller did not specify
        // a new value, `COALESCE(new_flag, 0)` keeps the existing column.
        let (pat_write, pat_value): (bool, Option<&str>) = match params.preset_agent_type {
            Some(v) => (true, v),
            None => (false, None),
        };

        sqlx::query(
            "INSERT INTO assistant_overrides \
                (assistant_id, enabled, sort_order, last_used_at, preset_agent_type, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(assistant_id) DO UPDATE SET \
                enabled = excluded.enabled, \
                sort_order = excluded.sort_order, \
                last_used_at = COALESCE(excluded.last_used_at, assistant_overrides.last_used_at), \
                preset_agent_type = CASE WHEN ? THEN ? ELSE assistant_overrides.preset_agent_type END, \
                updated_at = excluded.updated_at",
        )
        .bind(params.assistant_id)
        .bind(params.enabled)
        .bind(params.sort_order)
        .bind(last_used_at)
        .bind(pat_value)
        .bind(now)
        .bind(pat_write)
        .bind(pat_value)
        .execute(&self.pool)
        .await?;

        let row = self.get(params.assistant_id).await?.ok_or_else(|| {
            DbError::Init(format!(
                "upsert did not produce override row for id '{}'",
                params.assistant_id
            ))
        })?;
        Ok(row)
    }

    async fn delete(&self, assistant_id: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM assistant_overrides WHERE assistant_id = ?")
            .bind(assistant_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_orphans(&self, valid_ids: &[&str]) -> Result<u64, DbError> {
        if valid_ids.is_empty() {
            let result = sqlx::query("DELETE FROM assistant_overrides")
                .execute(&self.pool)
                .await?;
            return Ok(result.rows_affected());
        }

        let placeholders = std::iter::repeat_n("?", valid_ids.len()).collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM assistant_overrides WHERE assistant_id NOT IN ({placeholders})");
        let mut q = sqlx::query(&sql);
        for id in valid_ids {
            q = q.bind(*id);
        }
        let result = q.execute(&self.pool).await?;
        Ok(result.rows_affected())
    }
}

/// SQLite-backed implementation of [`IAssistantTagRepository`].
#[derive(Clone, Debug)]
pub struct SqliteAssistantTagRepository {
    pool: SqlitePool,
}

impl SqliteAssistantTagRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IAssistantTagRepository for SqliteAssistantTagRepository {
    async fn list(&self) -> Result<Vec<AssistantTagRow>, DbError> {
        let rows = sqlx::query_as::<_, AssistantTagRow>(
            "SELECT * FROM assistant_tags ORDER BY dimension ASC, sort_order ASC, created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get(&self, key: &str) -> Result<Option<AssistantTagRow>, DbError> {
        let row = sqlx::query_as::<_, AssistantTagRow>("SELECT * FROM assistant_tags WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn create(&self, params: &CreateAssistantTagParams<'_>) -> Result<AssistantTagRow, DbError> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO assistant_tags (key, dimension, label, sort_order, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(params.key)
        .bind(params.dimension)
        .bind(params.label)
        .bind(params.sort_order)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db_err) if is_unique_violation(db_err.as_ref()) => {
                DbError::Conflict(format!("Tag with key '{}' already exists", params.key))
            }
            _ => DbError::Query(e),
        })?;
        Ok(AssistantTagRow {
            key: params.key.to_string(),
            dimension: params.dimension.to_string(),
            label: params.label.to_string(),
            sort_order: params.sort_order,
            created_at: now,
        })
    }

    async fn update(&self, key: &str, params: &UpdateAssistantTagParams<'_>) -> Result<Option<AssistantTagRow>, DbError> {
        let Some(existing) = self.get(key).await? else {
            return Ok(None);
        };
        let label = params.label.unwrap_or(&existing.label);
        let sort_order = params.sort_order.unwrap_or(existing.sort_order);
        sqlx::query("UPDATE assistant_tags SET label = ?, sort_order = ? WHERE key = ?")
            .bind(label)
            .bind(sort_order)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(Some(AssistantTagRow {
            key: existing.key,
            dimension: existing.dimension,
            label: label.to_string(),
            sort_order,
            created_at: existing.created_at,
        }))
    }

    async fn delete(&self, key: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM assistant_tags WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    async fn setup() -> (
        SqliteAssistantRepository,
        SqliteAssistantOverrideRepository,
        crate::Database,
    ) {
        let db = init_database_memory().await.unwrap();
        let a = SqliteAssistantRepository::new(db.pool().clone());
        let o = SqliteAssistantOverrideRepository::new(db.pool().clone());
        (a, o, db)
    }

    fn params<'a>(id: &'a str, name: &'a str) -> CreateAssistantParams<'a> {
        CreateAssistantParams {
            id,
            name,
            description: Some("desc"),
            avatar: None,
            preset_agent_type: "gemini",
            enabled_skills: Some(r#"["skill-a"]"#),
            custom_skill_names: None,
            disabled_builtin_skills: None,
            prompts: Some(r#"["hello"]"#),
            models: None,
            name_i18n: Some(r#"{"zh-CN":"助手"}"#),
            description_i18n: None,
            prompts_i18n: None,
            audience_tags: Some(r#"["office"]"#),
            scenario_tags: None,
        }
    }

    #[tokio::test]
    async fn assistant_list_empty() {
        let (a, _o, _db) = setup().await;
        assert!(a.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn assistant_create_then_get() {
        let (a, _o, _db) = setup().await;
        let row = a.create(&params("u1", "User One")).await.unwrap();
        assert_eq!(row.id, "u1");
        assert_eq!(row.name, "User One");
        assert_eq!(row.preset_agent_type, "gemini");
        assert_eq!(row.enabled_skills.as_deref(), Some(r#"["skill-a"]"#));
        assert!(row.created_at > 0);
        assert_eq!(row.created_at, row.updated_at);

        let fetched = a.get("u1").await.unwrap().unwrap();
        assert_eq!(fetched.name, "User One");
    }

    #[tokio::test]
    async fn assistant_tags_round_trip_and_partial_update() {
        let (a, _o, _db) = setup().await;
        a.create(&params("u1", "Tagged")).await.unwrap();
        let got = a.get("u1").await.unwrap().unwrap();
        assert_eq!(got.audience_tags.as_deref(), Some(r#"["office"]"#));
        assert!(got.scenario_tags.is_none());

        // Setting Some(Some(..)) writes; omitting (None) keeps prior value.
        let upd = UpdateAssistantParams {
            scenario_tags: Some(Some(r#"["document"]"#)),
            ..Default::default()
        };
        let updated = a.update("u1", &upd).await.unwrap().unwrap();
        assert_eq!(updated.audience_tags.as_deref(), Some(r#"["office"]"#)); // preserved
        assert_eq!(updated.scenario_tags.as_deref(), Some(r#"["document"]"#)); // written
    }

    #[tokio::test]
    async fn assistant_create_duplicate_id_returns_conflict() {
        let (a, _o, _db) = setup().await;
        a.create(&params("u1", "A")).await.unwrap();
        let err = a.create(&params("u1", "B")).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn assistant_get_missing_returns_none() {
        let (a, _o, _db) = setup().await;
        assert!(a.get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn assistant_list_orders_by_updated_at_desc() {
        let (a, _o, _db) = setup().await;
        a.create(&params("u1", "first")).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        a.create(&params("u2", "second")).await.unwrap();

        let list = a.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "u2");
        assert_eq!(list[1].id, "u1");
    }

    #[tokio::test]
    async fn assistant_update_partial_keeps_other_fields() {
        let (a, _o, _db) = setup().await;
        a.create(&params("u1", "original")).await.unwrap();

        let upd = UpdateAssistantParams {
            name: Some("renamed"),
            ..Default::default()
        };
        let updated = a.update("u1", &upd).await.unwrap().unwrap();
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.preset_agent_type, "gemini");
        assert_eq!(updated.description.as_deref(), Some("desc"));
        assert_eq!(updated.enabled_skills.as_deref(), Some(r#"["skill-a"]"#));
        assert!(updated.updated_at >= updated.created_at);
    }

    #[tokio::test]
    async fn assistant_update_clears_nullable_with_some_none() {
        let (a, _o, _db) = setup().await;
        a.create(&params("u1", "has-desc")).await.unwrap();

        let upd = UpdateAssistantParams {
            description: Some(None),
            ..Default::default()
        };
        let updated = a.update("u1", &upd).await.unwrap().unwrap();
        assert!(updated.description.is_none());
    }

    #[tokio::test]
    async fn assistant_update_nonexistent_returns_none() {
        let (a, _o, _db) = setup().await;
        let res = a
            .update(
                "nope",
                &UpdateAssistantParams {
                    name: Some("x"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn assistant_delete_existing_returns_true() {
        let (a, _o, _db) = setup().await;
        a.create(&params("u1", "x")).await.unwrap();
        assert!(a.delete("u1").await.unwrap());
        assert!(a.get("u1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn assistant_delete_missing_returns_false() {
        let (a, _o, _db) = setup().await;
        assert!(!a.delete("nope").await.unwrap());
    }

    #[tokio::test]
    async fn assistant_upsert_inserts_then_updates() {
        let (a, _o, _db) = setup().await;
        let first = a.upsert(&params("u1", "first")).await.unwrap();
        assert_eq!(first.name, "first");

        let mut p = params("u1", "second");
        p.preset_agent_type = "claude";
        let second = a.upsert(&p).await.unwrap();
        assert_eq!(second.name, "second");
        assert_eq!(second.preset_agent_type, "claude");

        let list = a.list().await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn override_get_missing_returns_none() {
        let (_a, o, _db) = setup().await;
        assert!(o.get("u1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn override_upsert_inserts_row() {
        let (_a, o, _db) = setup().await;
        let row = o
            .upsert(&UpsertOverrideParams {
                assistant_id: "u1",
                enabled: false,
                sort_order: 5,
                last_used_at: Some(1000),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(row.assistant_id, "u1");
        assert!(!row.enabled);
        assert_eq!(row.sort_order, 5);
        assert_eq!(row.last_used_at, Some(1000));
    }

    #[tokio::test]
    async fn override_upsert_updates_existing() {
        let (_a, o, _db) = setup().await;
        o.upsert(&UpsertOverrideParams {
            assistant_id: "u1",
            enabled: true,
            sort_order: 0,
            last_used_at: Some(1000),
            ..Default::default()
        })
        .await
        .unwrap();

        let updated = o
            .upsert(&UpsertOverrideParams {
                assistant_id: "u1",
                enabled: false,
                sort_order: 3,
                last_used_at: None,
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!updated.enabled);
        assert_eq!(updated.sort_order, 3);
        // last_used_at None does not overwrite previous value (COALESCE)
        assert_eq!(updated.last_used_at, Some(1000));
    }

    #[tokio::test]
    async fn override_get_all_returns_rows() {
        let (_a, o, _db) = setup().await;
        o.upsert(&UpsertOverrideParams {
            assistant_id: "u1",
            enabled: true,
            sort_order: 0,
            last_used_at: None,
            ..Default::default()
        })
        .await
        .unwrap();
        o.upsert(&UpsertOverrideParams {
            assistant_id: "u2",
            enabled: false,
            sort_order: 1,
            last_used_at: None,
            ..Default::default()
        })
        .await
        .unwrap();

        let all = o.get_all().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn override_delete() {
        let (_a, o, _db) = setup().await;
        o.upsert(&UpsertOverrideParams {
            assistant_id: "u1",
            enabled: true,
            sort_order: 0,
            last_used_at: None,
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(o.delete("u1").await.unwrap());
        assert!(!o.delete("u1").await.unwrap());
    }

    #[tokio::test]
    async fn override_delete_orphans_removes_only_absent() {
        let (_a, o, _db) = setup().await;
        for id in ["a", "b", "c"] {
            o.upsert(&UpsertOverrideParams {
                assistant_id: id,
                enabled: true,
                sort_order: 0,
                last_used_at: None,
                ..Default::default()
            })
            .await
            .unwrap();
        }
        let removed = o.delete_orphans(&["a", "c"]).await.unwrap();
        assert_eq!(removed, 1);
        let remaining: Vec<String> = o.get_all().await.unwrap().into_iter().map(|r| r.assistant_id).collect();
        assert!(remaining.contains(&"a".to_string()));
        assert!(remaining.contains(&"c".to_string()));
        assert!(!remaining.contains(&"b".to_string()));
    }

    #[tokio::test]
    async fn override_delete_orphans_empty_valid_ids_clears_table() {
        let (_a, o, _db) = setup().await;
        o.upsert(&UpsertOverrideParams {
            assistant_id: "a",
            enabled: true,
            sort_order: 0,
            last_used_at: None,
            ..Default::default()
        })
        .await
        .unwrap();
        let removed = o.delete_orphans(&[]).await.unwrap();
        assert_eq!(removed, 1);
        assert!(o.get_all().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tag_repo_create_list_update_delete() {
        let db = init_database_memory().await.unwrap();
        let r = SqliteAssistantTagRepository::new(db.pool().clone());
        r.create(&CreateAssistantTagParams { key: "utag-1", dimension: "audience", label: "营销", sort_order: 3 })
            .await
            .unwrap();
        let all = r.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].label, "营销");

        let updated = r
            .update("utag-1", &UpdateAssistantTagParams { label: Some("市场营销"), sort_order: None })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.label, "市场营销");
        assert_eq!(updated.sort_order, 3); // preserved

        assert!(r.delete("utag-1").await.unwrap());
        assert!(r.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tag_repo_update_missing_returns_none() {
        let db = init_database_memory().await.unwrap();
        let r = SqliteAssistantTagRepository::new(db.pool().clone());
        let res = r
            .update("nope", &UpdateAssistantTagParams { label: Some("x"), sort_order: None })
            .await
            .unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn tag_repo_delete_missing_returns_false() {
        let db = init_database_memory().await.unwrap();
        let r = SqliteAssistantTagRepository::new(db.pool().clone());
        assert!(!r.delete("nope").await.unwrap());
    }

    #[tokio::test]
    async fn tag_repo_duplicate_key_conflicts() {
        let db = init_database_memory().await.unwrap();
        let r = SqliteAssistantTagRepository::new(db.pool().clone());
        let p = CreateAssistantTagParams { key: "k", dimension: "scenario", label: "A", sort_order: 0 };
        r.create(&p).await.unwrap();
        assert!(matches!(r.create(&p).await.unwrap_err(), DbError::Conflict(_)));
    }
}
