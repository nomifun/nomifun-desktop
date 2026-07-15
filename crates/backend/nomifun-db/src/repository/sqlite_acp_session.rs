//! SQLite-backed `acp_session` repository.

use nomifun_common::now_ms;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::AcpSessionRow;
use crate::repository::acp_session::{
    CreateAcpSessionParams, IAcpSessionRepository, PersistedSessionState, SaveRuntimeStateParams,
};

#[derive(Clone, Debug)]
pub struct SqliteAcpSessionRepository {
    pool: SqlitePool,
}

impl SqliteAcpSessionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn is_unique_violation(err: &dyn sqlx::error::DatabaseError) -> bool {
    err.code().is_some_and(|c| c == "2067" || c == "1555")
}

#[async_trait::async_trait]
impl IAcpSessionRepository for SqliteAcpSessionRepository {
    async fn get(&self, conversation_id: &str) -> Result<Option<AcpSessionRow>, DbError> {
        // `agent_id` is nullable in the schema (NULL = "no agent chosen yet").
        // COALESCE it back to the empty-string sentinel so `AcpSessionRow.agent_id`
        // stays a non-optional `String` for all downstream consumers.
        let row = sqlx::query_as::<_, AcpSessionRow>(
            "SELECT conversation_id, agent_backend, agent_source, \
                    COALESCE(agent_id, '') AS agent_id, session_id, session_status, \
                    session_config, last_active_at, suspended_at \
             FROM acp_session WHERE conversation_id = ?",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create(&self, params: &CreateAcpSessionParams<'_>) -> Result<AcpSessionRow, DbError> {
        let now = now_ms();
        // Write NULL (not the empty-string sentinel) when no agent is chosen so
        // the RESTRICT FK to agent_metadata is not evaluated against ''.
        let agent_id: Option<&str> = Some(params.agent_id).filter(|s| !s.is_empty());
        sqlx::query(
            "INSERT INTO acp_session \
                (conversation_id, agent_backend, agent_source, agent_id, \
                 session_id, session_status, session_config, last_active_at) \
             VALUES (?, ?, ?, ?, NULL, 'idle', '{}', ?)",
        )
        .bind(params.conversation_id)
        .bind(params.agent_backend)
        .bind(params.agent_source)
        .bind(agent_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db_err) if is_unique_violation(db_err.as_ref()) => DbError::Conflict(format!(
                "acp_session row for conversation '{}' already exists",
                params.conversation_id
            )),
            _ => DbError::Query(e),
        })?;

        self.get(params.conversation_id).await?.ok_or_else(|| {
            DbError::Init(format!(
                "create did not produce acp_session row for '{}'",
                params.conversation_id
            ))
        })
    }

    async fn update_session_id(&self, conversation_id: &str, session_id: &str) -> Result<bool, DbError> {
        let now = now_ms();
        let result = sqlx::query("UPDATE acp_session SET session_id = ?, last_active_at = ? WHERE conversation_id = ?")
            .bind(session_id)
            .bind(now)
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn clear_session_id(&self, conversation_id: &str) -> Result<bool, DbError> {
        // Read-modify-write the JSON blob to drop the cached usage while
        // leaving the user's mode/model/config selections intact (those are
        // preferences, not context). Same RMW rationale as
        // `save_runtime_state`: writes per conversation_id are serialised.
        let raw: Option<String> =
            sqlx::query_scalar("SELECT session_config FROM acp_session WHERE conversation_id = ?")
                .bind(conversation_id)
                .fetch_optional(&self.pool)
                .await?;

        let Some(raw) = raw else {
            return Ok(false);
        };

        let mut parsed: Value = serde_json::from_str(&raw).unwrap_or_else(|_| Value::Object(Default::default()));
        if let Some(runtime) = parsed
            .as_object_mut()
            .and_then(|obj| obj.get_mut("runtime"))
            .and_then(Value::as_object_mut)
        {
            runtime.remove("context_usage");
        }
        let new_config =
            serde_json::to_string(&parsed).map_err(|e| DbError::Init(format!("encode session_config: {e}")))?;

        let now = now_ms();
        let result = sqlx::query(
            "UPDATE acp_session SET session_id = NULL, session_status = 'idle', \
             session_config = ?, last_active_at = ? WHERE conversation_id = ?",
        )
        .bind(new_config)
        .bind(now)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete(&self, conversation_id: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM acp_session WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn load_runtime_state(&self, conversation_id: &str) -> Result<Option<PersistedSessionState>, DbError> {
        let raw: Option<String> =
            sqlx::query_scalar("SELECT session_config FROM acp_session WHERE conversation_id = ?")
                .bind(conversation_id)
                .fetch_optional(&self.pool)
                .await?;

        let Some(raw) = raw else {
            return Ok(None);
        };

        let parsed: Value =
            serde_json::from_str(&raw).map_err(|e| DbError::Init(format!("invalid session_config JSON: {e}")))?;
        let runtime = parsed.get("runtime");

        let mut state = PersistedSessionState::default();
        if let Some(rt) = runtime {
            state.current_mode_id = rt.get("current_mode_id").and_then(Value::as_str).map(ToOwned::to_owned);
            state.current_model_id = rt
                .get("current_model_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            state.config_selections_json = rt.get("config_selections").map(serde_json::Value::to_string);
            state.context_usage_json = rt.get("context_usage").map(serde_json::Value::to_string);
        }
        Ok(Some(state))
    }

    async fn save_runtime_state(
        &self,
        conversation_id: &str,
        params: &SaveRuntimeStateParams<'_>,
    ) -> Result<bool, DbError> {
        if params.is_empty() {
            return Ok(true);
        }

        // Read-modify-write. The service layer serialises writes per
        // conversation_id through a single consumer task, so a naive
        // RMW is race-free for our callers.
        let raw: Option<String> =
            sqlx::query_scalar("SELECT session_config FROM acp_session WHERE conversation_id = ?")
                .bind(conversation_id)
                .fetch_optional(&self.pool)
                .await?;

        let Some(raw) = raw else {
            return Ok(false);
        };

        let mut parsed: Value = serde_json::from_str(&raw).unwrap_or_else(|_| Value::Object(Default::default()));
        let runtime = parsed
            .as_object_mut()
            .ok_or_else(|| DbError::Init("session_config is not a JSON object".into()))?
            .entry("runtime")
            .or_insert_with(|| Value::Object(Default::default()));
        let runtime = runtime
            .as_object_mut()
            .ok_or_else(|| DbError::Init("session_config.runtime is not a JSON object".into()))?;

        if let Some(outer) = params.current_mode_id {
            match outer {
                Some(v) => {
                    runtime.insert("current_mode_id".into(), Value::String(v.to_owned()));
                }
                None => {
                    runtime.remove("current_mode_id");
                }
            }
        }
        if let Some(outer) = params.current_model_id {
            match outer {
                Some(v) => {
                    runtime.insert("current_model_id".into(), Value::String(v.to_owned()));
                }
                None => {
                    runtime.remove("current_model_id");
                }
            }
        }
        if let Some(outer) = params.config_selections_json {
            match outer {
                Some(json) => {
                    let v: Value = serde_json::from_str(json)
                        .map_err(|e| DbError::Init(format!("invalid config_selections JSON: {e}")))?;
                    runtime.insert("config_selections".into(), v);
                }
                None => {
                    runtime.remove("config_selections");
                }
            }
        }
        if let Some(outer) = params.context_usage_json {
            match outer {
                Some(json) => {
                    let v: Value = serde_json::from_str(json)
                        .map_err(|e| DbError::Init(format!("invalid context_usage JSON: {e}")))?;
                    runtime.insert("context_usage".into(), v);
                }
                None => {
                    runtime.remove("context_usage");
                }
            }
        }

        let new_config =
            serde_json::to_string(&parsed).map_err(|e| DbError::Init(format!("encode session_config: {e}")))?;
        let now = now_ms();
        let result = sqlx::query(
            "UPDATE acp_session SET session_config = ?, last_active_at = ? \
             WHERE conversation_id = ?",
        )
        .bind(new_config)
        .bind(now)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    const CONVERSATION_ID: &str =
        "conv_019abcdef012-7abc-8abc-0123-456789abcdef";
    const MISSING_CONVERSATION_ID: &str =
        "conv_019abcdef012-7abc-8abc-0123-456789abcdee";

    async fn setup() -> (SqliteAcpSessionRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteAcpSessionRepository::new(db.pool().clone());
        (repo, db)
    }

    fn create_params(conversation_id: &str) -> CreateAcpSessionParams<'_> {
        CreateAcpSessionParams {
            conversation_id,
            agent_backend: "claude",
            agent_source: "builtin",
            agent_id: "agent_builtin_claude",
        }
    }

    /// Insert a conversation so the `acp_session.conversation_id` FK
    /// (REFERENCES conversations(id) ON DELETE CASCADE) is satisfied before
    /// `create()` inserts the session row. The installation owner is seeded by
    /// init_database_memory, satisfying conversations.user_id FK; the
    /// `agent_builtin_claude` referenced by `create_params` is likewise seeded,
    /// satisfying the acp_session.agent_id FK.
    async fn seed_conversation(pool: &SqlitePool, id: &str) {
        let owner = crate::installation_owner_id(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
             VALUES (?, ?, 'c', 'normal', 'pending', 1, 1)",
        )
        .bind(id)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn create_then_get_roundtrips() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        let row = repo.create(&create_params(CONVERSATION_ID)).await.unwrap();
        assert_eq!(row.conversation_id, CONVERSATION_ID);
        assert_eq!(row.agent_backend, "claude");
        assert_eq!(row.session_id, None);
        assert_eq!(row.session_status, "idle");
        assert_eq!(row.session_config, "{}");

        let fetched = repo.get(CONVERSATION_ID).await.unwrap().unwrap();
        assert_eq!(fetched.conversation_id, CONVERSATION_ID);
    }

    #[tokio::test]
    async fn create_duplicate_returns_conflict() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();
        let err = repo.create(&create_params(CONVERSATION_ID)).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn update_session_id_flips_field() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();
        assert!(repo.update_session_id(CONVERSATION_ID, "sess-abc").await.unwrap());

        let fetched = repo.get(CONVERSATION_ID).await.unwrap().unwrap();
        assert_eq!(fetched.session_id.as_deref(), Some("sess-abc"));
        assert!(fetched.last_active_at.is_some());
    }

    #[tokio::test]
    async fn update_session_id_missing_row_returns_false() {
        let (repo, _db) = setup().await;
        assert!(!repo.update_session_id(MISSING_CONVERSATION_ID, "sid").await.unwrap());
    }

    #[tokio::test]
    async fn clear_session_id_nulls_sid_and_drops_usage_keeps_prefs() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();
        repo.update_session_id(CONVERSATION_ID, "sess-abc").await.unwrap();
        repo.save_runtime_state(
            CONVERSATION_ID,
            &SaveRuntimeStateParams {
                current_mode_id: Some(Some("code")),
                current_model_id: Some(Some("sonnet-4")),
                context_usage_json: Some(Some(r#"{"used":123,"total":200}"#)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(repo.clear_session_id(CONVERSATION_ID).await.unwrap());

        let row = repo.get(CONVERSATION_ID).await.unwrap().unwrap();
        assert_eq!(row.session_id, None, "session_id must be nulled");
        assert_eq!(row.session_status, "idle");

        let state = repo.load_runtime_state(CONVERSATION_ID).await.unwrap().unwrap();
        assert!(state.context_usage_json.is_none(), "cached usage must be dropped");
        assert_eq!(state.current_mode_id.as_deref(), Some("code"), "mode pref kept");
        assert_eq!(state.current_model_id.as_deref(), Some("sonnet-4"), "model pref kept");
    }

    #[tokio::test]
    async fn clear_session_id_missing_row_returns_false() {
        let (repo, _db) = setup().await;
        assert!(!repo.clear_session_id(MISSING_CONVERSATION_ID).await.unwrap());
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();
        assert!(repo.delete(CONVERSATION_ID).await.unwrap());
        assert!(repo.get(CONVERSATION_ID).await.unwrap().is_none());
        assert!(!repo.delete(CONVERSATION_ID).await.unwrap());
    }

    #[tokio::test]
    async fn load_runtime_state_missing_row() {
        let (repo, _db) = setup().await;
        assert!(repo.load_runtime_state(MISSING_CONVERSATION_ID).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn load_runtime_state_empty_config_returns_defaults() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();
        let state = repo.load_runtime_state(CONVERSATION_ID).await.unwrap().unwrap();
        assert_eq!(state, PersistedSessionState::default());
    }

    #[tokio::test]
    async fn save_runtime_state_writes_each_field() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();

        assert!(
            repo.save_runtime_state(
                CONVERSATION_ID,
                &SaveRuntimeStateParams {
                    current_mode_id: Some(Some("code")),
                    current_model_id: Some(Some("claude-sonnet-4")),
                    config_selections_json: Some(Some(r#"{"reasoning":"high"}"#)),
                    context_usage_json: Some(Some(r#"{"used":10,"total":100}"#)),
                },
            )
            .await
            .unwrap()
        );

        let state = repo.load_runtime_state(CONVERSATION_ID).await.unwrap().unwrap();
        assert_eq!(state.current_mode_id.as_deref(), Some("code"));
        assert_eq!(state.current_model_id.as_deref(), Some("claude-sonnet-4"));
        // The stored JSON should parse back to the same payload
        // regardless of key order (serde_json::Map preserves insertion
        // order but the caller shouldn't depend on it here).
        let selections: Value = serde_json::from_str(state.config_selections_json.as_deref().unwrap()).unwrap();
        assert_eq!(selections["reasoning"], "high");
        let usage: Value = serde_json::from_str(state.context_usage_json.as_deref().unwrap()).unwrap();
        assert_eq!(usage["used"], 10);
        assert_eq!(usage["total"], 100);
    }

    #[tokio::test]
    async fn save_runtime_state_partial_preserves_siblings() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();

        repo.save_runtime_state(
            CONVERSATION_ID,
            &SaveRuntimeStateParams {
                current_mode_id: Some(Some("code")),
                current_model_id: Some(Some("sonnet-4")),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Later write only touches current_model_id.
        repo.save_runtime_state(
            CONVERSATION_ID,
            &SaveRuntimeStateParams {
                current_model_id: Some(Some("opus-4")),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let state = repo.load_runtime_state(CONVERSATION_ID).await.unwrap().unwrap();
        assert_eq!(
            state.current_mode_id.as_deref(),
            Some("code"),
            "mode must survive the model-only write"
        );
        assert_eq!(state.current_model_id.as_deref(), Some("opus-4"));
    }

    #[tokio::test]
    async fn save_runtime_state_some_none_clears_field() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();

        repo.save_runtime_state(
            CONVERSATION_ID,
            &SaveRuntimeStateParams {
                current_mode_id: Some(Some("code")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        repo.save_runtime_state(
            CONVERSATION_ID,
            &SaveRuntimeStateParams {
                current_mode_id: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let state = repo.load_runtime_state(CONVERSATION_ID).await.unwrap().unwrap();
        assert!(state.current_mode_id.is_none());
    }

    #[tokio::test]
    async fn save_runtime_state_empty_params_is_noop() {
        let (repo, _db) = setup().await;
        seed_conversation(&repo.pool, CONVERSATION_ID).await;
        repo.create(&create_params(CONVERSATION_ID)).await.unwrap();
        assert!(
            repo.save_runtime_state(CONVERSATION_ID, &SaveRuntimeStateParams::default())
                .await
                .unwrap()
        );
        let state = repo.load_runtime_state(CONVERSATION_ID).await.unwrap().unwrap();
        assert_eq!(state, PersistedSessionState::default());
    }

    #[tokio::test]
    async fn save_runtime_state_missing_row_returns_false() {
        let (repo, _db) = setup().await;
        let ok = repo
            .save_runtime_state(
                MISSING_CONVERSATION_ID,
                &SaveRuntimeStateParams {
                    current_mode_id: Some(Some("x")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!ok);
    }
}
