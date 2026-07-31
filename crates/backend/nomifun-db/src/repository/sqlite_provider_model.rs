use nomifun_common::now_ms;
use nomifun_common::ProviderId;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{NewProviderModel, ProviderModelRow, ProviderModelUpdate};
use crate::repository::bind::{bind_value, BindValue};
use crate::repository::provider_model::IProviderModelRepository;

/// SQLite-backed implementation of [`IProviderModelRepository`].
#[derive(Clone, Debug)]
pub struct SqliteProviderModelRepository {
    pool: SqlitePool,
}

impl SqliteProviderModelRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Parse the provider id and take the parent-existence write lock inside the
/// caller's transaction (`UPDATE providers SET updated_at = updated_at`), so a
/// concurrent provider delete cannot race the child write.
async fn lock_parent_provider(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
) -> Result<ProviderId, DbError> {
    let provider_id = ProviderId::parse(provider_id).map_err(|error| {
        DbError::Conflict(format!(
            "Provider model provider_id '{provider_id}' is not a canonical UUIDv7: {error}"
        ))
    })?;
    let parent = sqlx::query("UPDATE providers SET updated_at = updated_at WHERE provider_id = ?")
        .bind(provider_id.as_str())
        .execute(&mut **tx)
        .await?;
    if parent.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "Provider model provider '{provider_id}' does not exist"
        )));
    }
    Ok(provider_id)
}

fn insert_query<'q>(
    provider_id: &'q str,
    row: &'q NewProviderModel<'q>,
    now: i64,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    sqlx::query(
        "INSERT INTO provider_models \
            (provider_id, model, enabled, sort_order, tasks, traits, protocol, \
             params, context_limit, description, source, health, health_checked_at, \
             created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?) \
         ON CONFLICT(provider_id, model) DO NOTHING",
    )
    .bind(provider_id)
    .bind(row.model)
    .bind(row.enabled)
    .bind(row.sort_order)
    .bind(row.tasks)
    .bind(row.traits)
    .bind(row.protocol)
    .bind(row.params)
    .bind(row.context_limit)
    .bind(row.description)
    .bind(row.source)
    .bind(row.health)
    .bind(now)
    .bind(now)
}

async fn fetch_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    model: &str,
) -> Result<ProviderModelRow, DbError> {
    let row = sqlx::query_as::<_, ProviderModelRow>(
        "SELECT * FROM provider_models WHERE provider_id = ? AND model = ?",
    )
    .bind(provider_id)
    .bind(model)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

#[async_trait::async_trait]
impl IProviderModelRepository for SqliteProviderModelRepository {
    async fn list(&self) -> Result<Vec<ProviderModelRow>, DbError> {
        let rows = sqlx::query_as::<_, ProviderModelRow>(
            "SELECT * FROM provider_models ORDER BY provider_id ASC, sort_order ASC, model ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderModelRow>, DbError> {
        // `(sort_order, id)` matches the response-projection tie-break
        // (`row_to_response` sorts by the same key), so equal sort_order
        // resolves by insertion order everywhere.
        let rows = sqlx::query_as::<_, ProviderModelRow>(
            "SELECT * FROM provider_models WHERE provider_id = ? \
             ORDER BY sort_order ASC, id ASC",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get(&self, provider_id: &str, model: &str) -> Result<Option<ProviderModelRow>, DbError> {
        let row = sqlx::query_as::<_, ProviderModelRow>(
            "SELECT * FROM provider_models WHERE provider_id = ? AND model = ?",
        )
        .bind(provider_id)
        .bind(model)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create(&self, provider_id: &str, row: &NewProviderModel<'_>) -> Result<ProviderModelRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let provider_id = lock_parent_provider(&mut tx, provider_id).await?;

        let result = insert_query(provider_id.as_str(), row, now)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "Provider model '{}' already exists for provider '{provider_id}'",
                row.model
            )));
        }
        let stored = fetch_row(&mut tx, provider_id.as_str(), row.model).await?;
        tx.commit().await?;
        Ok(stored)
    }

    async fn insert_if_absent(&self, provider_id: &str, row: &NewProviderModel<'_>) -> Result<bool, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let provider_id = lock_parent_provider(&mut tx, provider_id).await?;

        let result = insert_query(provider_id.as_str(), row, now)
            .execute(&mut *tx)
            .await?;
        let inserted = result.rows_affected() > 0;
        tx.commit().await?;
        Ok(inserted)
    }

    async fn update(&self, provider_id: &str, model: &str, update: &ProviderModelUpdate<'_>) -> Result<ProviderModelRow, DbError> {
        let mut tx = self.pool.begin().await?;
        let provider_id = lock_parent_provider(&mut tx, provider_id).await?;

        // Dynamic SET assembly; `None` = keep, `Some(None)` = clear (nullable
        // columns). `updated_at` is always written, so the statement is never
        // empty and existence is checked via rows_affected.
        let mut set_parts: Vec<&'static str> = Vec::new();
        let mut binds: Vec<BindValue> = Vec::new();
        if let Some(v) = update.enabled {
            set_parts.push("enabled = ?");
            binds.push(BindValue::Bool(v));
        }
        if let Some(v) = update.sort_order {
            set_parts.push("sort_order = ?");
            binds.push(BindValue::I64(v));
        }
        if let Some(v) = update.tasks {
            set_parts.push("tasks = ?");
            binds.push(BindValue::Str(v.to_string()));
        }
        if let Some(v) = update.traits {
            set_parts.push("traits = ?");
            binds.push(BindValue::Str(v.to_string()));
        }
        if let Some(v) = update.protocol {
            set_parts.push("protocol = ?");
            binds.push(BindValue::OptStr(v.map(String::from)));
        }
        if let Some(v) = update.connection_role {
            set_parts.push("connection_role = ?");
            binds.push(BindValue::OptStr(v.map(String::from)));
        }
        if let Some(v) = update.params {
            set_parts.push("params = ?");
            binds.push(BindValue::Str(v.to_string()));
        }
        if let Some(v) = update.context_limit {
            set_parts.push("context_limit = ?");
            binds.push(BindValue::OptI64(v));
        }
        if let Some(v) = update.description {
            set_parts.push("description = ?");
            binds.push(BindValue::OptStr(v.map(String::from)));
        }
        if let Some(v) = update.source {
            set_parts.push("source = ?");
            binds.push(BindValue::Str(v.to_string()));
        }
        set_parts.push("updated_at = ?");
        binds.push(BindValue::I64(now_ms()));

        let sql = format!(
            "UPDATE provider_models SET {} WHERE provider_id = ? AND model = ?",
            set_parts.join(", ")
        );
        let mut query = sqlx::query(&sql);
        for bind in &binds {
            query = bind_value(query, bind);
        }
        let result = query
            .bind(provider_id.as_str())
            .bind(model)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "Provider model '{model}' not found for provider '{provider_id}'"
            )));
        }
        let stored = fetch_row(&mut tx, provider_id.as_str(), model).await?;
        tx.commit().await?;
        Ok(stored)
    }

    async fn set_health(&self, provider_id: &str, model: &str, health_json: Option<&str>) -> Result<bool, DbError> {
        let now = now_ms();
        // `health_checked_at` records when the stored probe outcome was
        // observed; clearing the health clears the observation time too.
        let checked_at = health_json.map(|_| now);
        let result = sqlx::query(
            "UPDATE provider_models \
             SET health = ?, health_checked_at = ?, updated_at = ? \
             WHERE provider_id = ? AND model = ?",
        )
        .bind(health_json)
        .bind(checked_at)
        .bind(now)
        .bind(provider_id)
        .bind(model)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM provider_models WHERE provider_id = ? AND model = ?")
            .bind(provider_id)
            .bind(model)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;
    use crate::repository::provider::{CreateProviderParams, IProviderRepository};
    use crate::repository::sqlite_provider::SqliteProviderRepository;

    const PROVIDER_1: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const PROVIDER_2: &str = "0190f5fe-7c00-7a00-8abc-012345678902";

    async fn seed_provider(pool: &SqlitePool, provider_id: &str) {
        SqliteProviderRepository::new(pool.clone())
            .create(CreateProviderParams {
                provider_id: Some(provider_id),
                platform: "openai",
                name: provider_id,
                base_url: "https://x.test/v1",
                api_key_encrypted: "enc",
                models: "[]",
                enabled: true,
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_get_update_delete_roundtrip() {
        let db = init_database_memory().await.unwrap();
        seed_provider(db.pool(), PROVIDER_1).await;
        let r = SqliteProviderModelRepository::new(db.pool().clone());
        let created = r
            .create(
                PROVIDER_1,
                &NewProviderModel {
                    model: "gpt-image-1",
                    enabled: true,
                    sort_order: 0,
                    tasks: r#"["image_generation"]"#,
                    traits: "[]",
                    protocol: None,
                    params: "{}",
                    context_limit: None,
                    description: None,
                    source: "user",
                    health: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(created.provider_id, PROVIDER_1);
        assert_eq!(created.model, "gpt-image-1");
        assert!(created.enabled);
        assert_eq!(created.source, "user");
        assert!(created.health_checked_at.is_none());

        assert!(
            !r.insert_if_absent(
                PROVIDER_1,
                &NewProviderModel {
                    model: "gpt-image-1",
                    tasks: "[]",
                    traits: "[]",
                    params: "{}",
                    source: "inferred",
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
        );
        // Losing insert must not overwrite the existing row.
        let stored = r.get(PROVIDER_1, "gpt-image-1").await.unwrap().unwrap();
        assert_eq!(stored.tasks, r#"["image_generation"]"#);
        assert_eq!(stored.source, "user");

        let row = r
            .update(
                PROVIDER_1,
                "gpt-image-1",
                &ProviderModelUpdate {
                    context_limit: Some(Some(4096)),
                    description: Some(Some("img")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(row.context_limit, Some(4096));
        assert_eq!(row.description.as_deref(), Some("img"));
        assert_eq!(row.tasks, r#"["image_generation"]"#, "partial update keeps profile");

        // `Some(None)` clears a nullable column.
        let row = r
            .update(
                PROVIDER_1,
                "gpt-image-1",
                &ProviderModelUpdate {
                    description: Some(None),
                    connection_role: Some(Some("voice")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(row.description, None);
        assert_eq!(row.connection_role.as_deref(), Some("voice"));
        assert_eq!(row.context_limit, Some(4096), "unrelated fields kept");

        assert!(
            r.set_health(PROVIDER_1, "gpt-image-1", Some(r#"{"status":"healthy"}"#))
                .await
                .unwrap()
        );
        let stored = r.get(PROVIDER_1, "gpt-image-1").await.unwrap().unwrap();
        assert_eq!(stored.health.as_deref(), Some(r#"{"status":"healthy"}"#));
        assert!(stored.health_checked_at.is_some());
        assert!(!r.set_health(PROVIDER_1, "missing", None).await.unwrap());

        assert!(r.delete(PROVIDER_1, "gpt-image-1").await.unwrap());
        assert!(!r.delete(PROVIDER_1, "gpt-image-1").await.unwrap());
        assert!(r.get(PROVIDER_1, "gpt-image-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_duplicate_is_conflict() {
        let db = init_database_memory().await.unwrap();
        seed_provider(db.pool(), PROVIDER_1).await;
        let r = SqliteProviderModelRepository::new(db.pool().clone());
        let row = NewProviderModel {
            model: "gpt-4o",
            tasks: r#"["chat"]"#,
            traits: "[]",
            params: "{}",
            source: "inferred",
            enabled: true,
            ..Default::default()
        };
        r.create(PROVIDER_1, &row).await.unwrap();
        let err = r.create(PROVIDER_1, &row).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn unknown_provider_is_conflict() {
        let db = init_database_memory().await.unwrap();
        let r = SqliteProviderModelRepository::new(db.pool().clone());
        let row = NewProviderModel {
            model: "m",
            tasks: "[]",
            traits: "[]",
            params: "{}",
            source: "inferred",
            enabled: true,
            ..Default::default()
        };
        let err = r.create(PROVIDER_2, &row).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
        let err = r.insert_if_absent(PROVIDER_2, &row).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_models")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn list_scopes_and_orders_by_sort_order() {
        let db = init_database_memory().await.unwrap();
        seed_provider(db.pool(), PROVIDER_1).await;
        seed_provider(db.pool(), PROVIDER_2).await;
        let r = SqliteProviderModelRepository::new(db.pool().clone());
        for (provider, model, sort_order) in [
            (PROVIDER_1, "b-model", 1_i64),
            (PROVIDER_1, "a-model", 0),
            // Two rows tied on sort_order: insertion order (id) breaks the
            // tie, NOT the model name — matching the response projection.
            (PROVIDER_1, "z-tied", 2),
            (PROVIDER_1, "a-tied", 2),
            (PROVIDER_2, "other", 0),
        ] {
            assert!(
                r.insert_if_absent(
                    provider,
                    &NewProviderModel {
                        model,
                        sort_order,
                        tasks: "[]",
                        traits: "[]",
                        params: "{}",
                        source: "inferred",
                        enabled: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
            );
        }
        assert_eq!(r.list().await.unwrap().len(), 5);
        let scoped = r.list_for_provider(PROVIDER_1).await.unwrap();
        assert_eq!(
            scoped.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(),
            ["a-model", "b-model", "z-tied", "a-tied"],
            "equal sort_order resolves by id (insertion order), not model name"
        );
    }
}
