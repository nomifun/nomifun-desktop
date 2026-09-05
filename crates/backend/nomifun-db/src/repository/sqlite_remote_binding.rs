use sqlx::{pool::PoolConnection, Sqlite, SqliteConnection, SqlitePool};
use sha2::{Digest, Sha256};

use nomifun_common::{generate_id, now_ms, validate_uuidv7};

use crate::error::DbError;
use crate::models::{
    NomiRemoteEventPage, NomiRemoteEventRow, NomiRemoteSessionRow, RemoteBindingRow,
};
use crate::repository::remote_binding::{
    AppendNomiRemoteEventParams, AppendNomiRemoteEventResult, CreateRemoteBindingParams,
    GetOrCreateRemoteSessionParams, IRemoteBindingRepository, NomiRemoteStateTransitionResult,
    RemoteOpenResult, TransitionNomiRemoteSessionParams, UpdateRemoteBindingParams,
};

#[derive(Clone, Debug)]
pub struct SqliteRemoteBindingRepository {
    pool: SqlitePool,
}

impl SqliteRemoteBindingRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn validate_uuid(value: &str, label: &str) -> Result<(), DbError> {
    validate_uuidv7(value).map_err(|error| {
        DbError::Conflict(format!("{label} '{value}' is not canonical UUIDv7: {error}"))
    }).map(|_| ())
}

fn validate_digest(value: &str, label: &str) -> Result<(), DbError> {
    if value.len() != 64
        || value != value.to_ascii_lowercase()
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DbError::Conflict(format!(
            "{label} must be a lowercase 64-character SHA-256 hex digest"
        )));
    }
    Ok(())
}

fn validate_object_json(value: &str, label: &str) -> Result<(), DbError> {
    let parsed: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| DbError::Conflict(format!("{label} is invalid JSON: {error}")))?;
    if !parsed.is_object() {
        return Err(DbError::Conflict(format!("{label} must be a JSON object")));
    }
    Ok(())
}

fn validate_binding_json(value: &str, version: i64) -> Result<(), DbError> {
    validate_object_json(value, "agent_binding_json")?;
    if version <= 0 {
        return Err(DbError::Conflict(
            "binding_version must be greater than zero".to_owned(),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(value).map_err(|error| {
        DbError::Conflict(format!("agent_binding_json is invalid JSON: {error}"))
    })?;
    let actual = parsed
        .get("binding_version")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| {
            DbError::Conflict("agent_binding_json must contain integer binding_version".to_owned())
        })?;
    if actual != version {
        return Err(DbError::Conflict(format!(
            "agent_binding_json binding_version {actual} does not match row version {version}"
        )));
    }
    Ok(())
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize_json).collect())
        }
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        other => other,
    }
}

fn canonical_json_string(value: &serde_json::Value, label: &str) -> Result<String, DbError> {
    serde_json::to_string(&canonicalize_json(value.clone()))
        .map_err(|error| DbError::Conflict(format!("{label} cannot be canonicalized: {error}")))
}

fn validate_binding_digest(json: &str, digest: &str) -> Result<(), DbError> {
    validate_digest(digest, "agent_binding_digest")?;
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| DbError::Conflict(format!("agent_binding_json is invalid JSON: {error}")))?;
    let canonical = serde_json::to_vec(&canonicalize_json(value))
        .map_err(|error| DbError::Conflict(format!("agent_binding_json cannot be canonicalized: {error}")))?;
    let actual = hex::encode(Sha256::digest(canonical));
    if actual != digest {
        return Err(DbError::Conflict(
            "agent_binding_digest does not match canonical AgentBindingValue JSON".to_owned(),
        ));
    }
    Ok(())
}

fn validate_name(value: &str) -> Result<(), DbError> {
    if value.trim().is_empty() {
        return Err(DbError::Conflict(
            "Remote binding name must not be empty".to_owned(),
        ));
    }
    Ok(())
}

fn validate_state(value: &str) -> Result<(), DbError> {
    if matches!(value, "opening" | "ready" | "failed" | "cancelled") {
        Ok(())
    } else {
        Err(DbError::Conflict(format!(
            "unsupported Remote session state '{value}'"
        )))
    }
}

fn operation_event_transition_is_allowed(existing: &str, next: &str) -> bool {
    matches!(
        (existing, next),
        (
            "turn/accepted",
            "turn/completed" | "turn/failed" | "turn/unknown"
        ) | (
            "session/cancel-requested",
            "session/cancelled"
                | "session/cancel-rejected"
                | "session/cancel-unknown"
        )
    )
}

async fn fetch_binding(
    conn: &mut SqliteConnection,
    owner: &str,
    binding_id: &str,
) -> Result<Option<RemoteBindingRow>, DbError> {
    Ok(sqlx::query_as(
        "SELECT remote_binding_id, owner_user_id, name, agent_binding_json, \
         nomi_snapshot_json, provenance_json, agent_binding_digest, binding_version, \
         created_at, updated_at FROM remote_bindings \
         WHERE owner_user_id = ? AND remote_binding_id = ?",
    )
    .bind(owner)
    .bind(binding_id)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn fetch_session(
    conn: &mut SqliteConnection,
    owner: &str,
    session_id: &str,
) -> Result<Option<NomiRemoteSessionRow>, DbError> {
    Ok(sqlx::query_as(
        "SELECT agent_session_id, owner_user_id, remote_binding_id, open_idempotency_key, \
         binding_version, agent_binding_digest, initial_input_digest, agent_binding_json, nomi_snapshot_json, \
         provenance_json, state, created_at, updated_at FROM nomi_remote_sessions \
         WHERE owner_user_id = ? AND agent_session_id = ?",
    )
    .bind(owner)
    .bind(session_id)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn fetch_session_by_key(
    conn: &mut SqliteConnection,
    owner: &str,
    key: &str,
) -> Result<Option<NomiRemoteSessionRow>, DbError> {
    Ok(sqlx::query_as(
        "SELECT agent_session_id, owner_user_id, remote_binding_id, open_idempotency_key, \
         binding_version, agent_binding_digest, initial_input_digest, agent_binding_json, nomi_snapshot_json, \
         provenance_json, state, created_at, updated_at FROM nomi_remote_sessions \
         WHERE owner_user_id = ? AND open_idempotency_key = ?",
    )
    .bind(owner)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn fetch_event_by_operation_key(
    conn: &mut SqliteConnection,
    owner: &str,
    session_id: &str,
    event_type: &str,
    operation_key_digest: &str,
) -> Result<Option<NomiRemoteEventRow>, DbError> {
    // The owner/session existence check is part of the repository contract:
    // an absent or foreign Session must not look like an empty event stream.
    if fetch_session(conn, owner, session_id).await?.is_none() {
        return Err(DbError::NotFound("Remote session".to_owned()));
    }
    Ok(sqlx::query_as(
        "SELECT event_id, agent_session_id, seq, event_type, payload_json, created_at \
         FROM nomi_remote_events \
         WHERE agent_session_id = ? AND event_type = ? \
           AND json_extract(payload_json, '$.operation_key_digest') = ? \
         ORDER BY seq LIMIT 1",
    )
    .bind(session_id)
    .bind(event_type)
    .bind(operation_key_digest)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn fetch_event_cursor(
    conn: &mut SqliteConnection,
    owner: &str,
    session_id: &str,
) -> Result<i64, DbError> {
    if fetch_session(conn, owner, session_id).await?.is_none() {
        return Err(DbError::NotFound("Remote session".to_owned()));
    }
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) FROM nomi_remote_events \
         WHERE agent_session_id = ?",
    )
    .bind(session_id)
    .fetch_one(&mut *conn)
    .await?)
}

async fn begin_immediate(pool: &SqlitePool) -> Result<PoolConnection<Sqlite>, DbError> {
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    Ok(conn)
}

async fn rollback(conn: &mut PoolConnection<Sqlite>) {
    let _ = sqlx::query("ROLLBACK").execute(&mut **conn).await;
}

async fn commit(conn: &mut PoolConnection<Sqlite>) -> Result<(), DbError> {
    sqlx::query("COMMIT").execute(&mut **conn).await?;
    Ok(())
}

async fn append_event_in_transaction(
    conn: &mut PoolConnection<Sqlite>,
    input: &AppendNomiRemoteEventParams,
) -> Result<AppendNomiRemoteEventResult, DbError> {
    if fetch_session(conn, &input.owner_user_id, &input.agent_session_id)
        .await?
        .is_none()
    {
        return Err(DbError::NotFound("Remote session".to_owned()));
    }

    let payload: serde_json::Value = serde_json::from_str(&input.payload_json)
        .map_err(|error| DbError::Conflict(format!("event payload JSON is invalid: {error}")))?;
    if !payload.is_object() && !payload.is_array() {
        return Err(DbError::Conflict(
            "event payload must be a JSON object or array".to_owned(),
        ));
    }
    let canonical_payload_json = canonical_json_string(&payload, "event payload")?;
    if let Some(operation_key_digest) = payload
        .get("operation_key_digest")
        .and_then(serde_json::Value::as_str)
    {
        validate_digest(operation_key_digest, "operation_key_digest")?;
    }

    // Operation-keyed events are idempotent at the same transaction boundary as
    // the sequence allocation. This is important for concurrent finalizers:
    // a read-then-append pair can otherwise allocate two terminal rows.
    if let Some(operation_key_digest) = payload
        .get("operation_key_digest")
        .and_then(serde_json::Value::as_str)
    {
        // First check the exact event phase. An operation may legitimately
        // have two rows (`turn/accepted` followed by `turn/completed`), so
        // looking only at the first row would allow a replay of the terminal
        // phase to append a duplicate terminal event.
        if let Some(existing) = sqlx::query_as::<_, NomiRemoteEventRow>(
            "SELECT event_id, agent_session_id, seq, event_type, payload_json, created_at \
             FROM nomi_remote_events \
             WHERE agent_session_id = ? AND event_type = ? \
               AND json_extract(payload_json, '$.operation_key_digest') = ? \
             ORDER BY seq LIMIT 1",
        )
        .bind(&input.agent_session_id)
        .bind(&input.event_type)
        .bind(operation_key_digest)
        .fetch_optional(&mut **conn)
        .await?
        {
            let existing_payload: serde_json::Value =
                serde_json::from_str(&existing.payload_json).map_err(|error| {
                    DbError::Conflict(format!(
                        "persisted Remote event payload is invalid: {error}"
                    ))
                })?;
            if canonical_json_string(&existing_payload, "persisted event payload")?
                != canonical_payload_json
            {
                return Err(DbError::Conflict(format!(
                    "Remote operation event key was reused with a different payload"
                )));
            }
            return Ok(AppendNomiRemoteEventResult {
                event: existing,
                inserted: false,
            });
        }

        // Then enforce the small, explicit phase graph against the latest
        // committed row for this operation. Terminal outcomes are absorbing:
        // a late completion cannot follow `turn/unknown` or `turn/failed`.
        if let Some(existing) = sqlx::query_as::<_, NomiRemoteEventRow>(
            "SELECT event_id, agent_session_id, seq, event_type, payload_json, created_at \
             FROM nomi_remote_events \
             WHERE agent_session_id = ? \
               AND json_extract(payload_json, '$.operation_key_digest') = ? \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(&input.agent_session_id)
        .bind(operation_key_digest)
        .fetch_optional(&mut **conn)
        .await?
        {
            if !operation_event_transition_is_allowed(&existing.event_type, &input.event_type) {
                return Err(DbError::Conflict(format!(
                    "Remote operation event key cannot transition from '{}' to '{}'",
                    existing.event_type, input.event_type
                )));
            }
        }
    }

    let seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM nomi_remote_events \
         WHERE agent_session_id = ?",
    )
    .bind(&input.agent_session_id)
    .fetch_one(&mut **conn)
    .await?;
    let event_id = generate_id();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO nomi_remote_events \
         (event_id, agent_session_id, seq, event_type, payload_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&event_id)
    .bind(&input.agent_session_id)
    .bind(seq)
    .bind(&input.event_type)
    .bind(&canonical_payload_json)
    .bind(now)
    .execute(&mut **conn)
    .await?;
    Ok(AppendNomiRemoteEventResult {
        event: NomiRemoteEventRow {
            event_id,
            agent_session_id: input.agent_session_id.clone(),
            seq,
            event_type: input.event_type.clone(),
            payload_json: canonical_payload_json,
            created_at: now,
        },
        inserted: true,
    })
}

#[async_trait::async_trait]
impl IRemoteBindingRepository for SqliteRemoteBindingRepository {
    async fn create_binding(
        &self,
        input: CreateRemoteBindingParams,
    ) -> Result<RemoteBindingRow, DbError> {
        validate_uuid(&input.remote_binding_id, "remote_binding_id")?;
        validate_uuid(&input.owner_user_id, "owner_user_id")?;
        validate_name(&input.name)?;
        validate_binding_json(&input.agent_binding_json, input.binding_version)?;
        validate_object_json(&input.nomi_snapshot_json, "nomi_snapshot_json")?;
        validate_object_json(&input.provenance_json, "provenance_json")?;
        validate_binding_digest(&input.agent_binding_json, &input.agent_binding_digest)?;
        let owner_exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_id = ?")
                .bind(&input.owner_user_id)
                .fetch_one(&self.pool)
                .await?;
        if owner_exists != 1 {
            return Err(DbError::NotFound("owner user".to_owned()));
        }
        let now = now_ms();
        sqlx::query(
            "INSERT INTO remote_bindings \
             (remote_binding_id, owner_user_id, name, agent_binding_json, nomi_snapshot_json, \
              provenance_json, agent_binding_digest, binding_version, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&input.remote_binding_id)
        .bind(&input.owner_user_id)
        .bind(&input.name)
        .bind(&input.agent_binding_json)
        .bind(&input.nomi_snapshot_json)
        .bind(&input.provenance_json)
        .bind(&input.agent_binding_digest)
        .bind(input.binding_version)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|error| match error {
            sqlx::Error::Database(database_error)
                if database_error.message().contains("UNIQUE") =>
            {
                DbError::Conflict(format!(
                    "Remote binding '{}' already exists",
                    input.remote_binding_id
                ))
            }
            other => DbError::Query(other),
        })?;
        self.get_binding(&input.owner_user_id, &input.remote_binding_id)
            .await?
            .ok_or_else(|| DbError::Init("created Remote binding disappeared".to_owned()))
    }

    async fn get_binding(
        &self,
        owner_user_id: &str,
        remote_binding_id: &str,
    ) -> Result<Option<RemoteBindingRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(remote_binding_id, "remote_binding_id")?;
        let mut conn = self.pool.acquire().await?;
        fetch_binding(&mut conn, owner_user_id, remote_binding_id).await
    }

    async fn list_bindings(&self, owner_user_id: &str) -> Result<Vec<RemoteBindingRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        Ok(sqlx::query_as(
            "SELECT remote_binding_id, owner_user_id, name, agent_binding_json, \
             nomi_snapshot_json, provenance_json, agent_binding_digest, binding_version, \
             created_at, updated_at FROM remote_bindings WHERE owner_user_id = ? \
             ORDER BY created_at, remote_binding_id",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn update_binding_cas(
        &self,
        input: UpdateRemoteBindingParams,
    ) -> Result<RemoteBindingRow, DbError> {
        validate_uuid(&input.owner_user_id, "owner_user_id")?;
        validate_uuid(&input.remote_binding_id, "remote_binding_id")?;
        validate_name(&input.name)?;
        validate_binding_json(&input.agent_binding_json, input.binding_version)?;
        validate_object_json(&input.nomi_snapshot_json, "nomi_snapshot_json")?;
        validate_object_json(&input.provenance_json, "provenance_json")?;
        validate_digest(
            &input.expected_agent_binding_digest,
            "expected_agent_binding_digest",
        )?;
        validate_binding_digest(&input.agent_binding_json, &input.agent_binding_digest)?;
        if input.binding_version != input.expected_binding_version + 1 {
            return Err(DbError::Conflict(
                "updated binding_version must be expected_binding_version + 1".to_owned(),
            ));
        }

        let mut conn = begin_immediate(&self.pool).await?;
        let result = async {
            let existing =
                fetch_binding(&mut conn, &input.owner_user_id, &input.remote_binding_id)
                    .await?
                    .ok_or_else(|| DbError::NotFound("Remote binding".to_owned()))?;
            if existing.binding_version != input.expected_binding_version {
                return Err(DbError::Conflict("Remote binding version changed".to_owned()));
            }
            if existing.agent_binding_digest != input.expected_agent_binding_digest {
                return Err(DbError::Conflict("Remote binding digest changed".to_owned()));
            }
            let now = now_ms();
            sqlx::query(
                "UPDATE remote_bindings SET name = ?, agent_binding_json = ?, \
                 nomi_snapshot_json = ?, provenance_json = ?, agent_binding_digest = ?, binding_version = ?, \
                 updated_at = ? WHERE owner_user_id = ? AND remote_binding_id = ? \
                 AND binding_version = ? AND agent_binding_digest = ?",
            )
            .bind(&input.name)
            .bind(&input.agent_binding_json)
            .bind(&input.nomi_snapshot_json)
            .bind(&input.provenance_json)
            .bind(&input.agent_binding_digest)
            .bind(input.binding_version)
            .bind(now)
            .bind(&input.owner_user_id)
            .bind(&input.remote_binding_id)
            .bind(input.expected_binding_version)
            .bind(&input.expected_agent_binding_digest)
            .execute(&mut *conn)
            .await?;
            fetch_binding(&mut conn, &input.owner_user_id, &input.remote_binding_id)
                .await?
                .ok_or_else(|| DbError::Init("updated Remote binding disappeared".to_owned()))
        }
        .await;
        match result {
            Ok(row) => {
                commit(&mut conn).await?;
                Ok(row)
            }
            Err(error) => {
                rollback(&mut conn).await;
                Err(error)
            }
        }
    }

    async fn delete_binding_cas(
        &self,
        owner_user_id: &str,
        remote_binding_id: &str,
        expected_binding_version: i64,
        expected_agent_binding_digest: &str,
    ) -> Result<(), DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(remote_binding_id, "remote_binding_id")?;
        validate_digest(expected_agent_binding_digest, "expected_agent_binding_digest")?;
        let result = sqlx::query(
            "DELETE FROM remote_bindings WHERE owner_user_id = ? AND remote_binding_id = ? \
             AND binding_version = ? AND agent_binding_digest = ?",
        )
        .bind(owner_user_id)
        .bind(remote_binding_id)
        .bind(expected_binding_version)
        .bind(expected_agent_binding_digest)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(if self.get_binding(owner_user_id, remote_binding_id).await?.is_some() {
                DbError::Conflict("Remote binding version or digest changed".to_owned())
            } else {
                DbError::NotFound("Remote binding".to_owned())
            });
        }
        Ok(())
    }

    async fn get_or_create_session(
        &self,
        input: GetOrCreateRemoteSessionParams,
    ) -> Result<RemoteOpenResult, DbError> {
        validate_uuid(&input.owner_user_id, "owner_user_id")?;
        validate_uuid(&input.remote_binding_id, "remote_binding_id")?;
        validate_uuid(&input.agent_session_id, "agent_session_id")?;
        validate_digest(
            &input.expected_agent_binding_digest,
            "expected_agent_binding_digest",
        )?;
        if let Some(digest) = input.initial_input_digest.as_deref() {
            validate_digest(digest, "initial_input_digest")?;
        }
        if input.open_idempotency_key.trim().is_empty() {
            return Err(DbError::Conflict(
                "open_idempotency_key must not be empty".to_owned(),
            ));
        }

        let mut conn = begin_immediate(&self.pool).await?;
        let result = async {
            if let Some(existing) =
                fetch_session_by_key(&mut conn, &input.owner_user_id, &input.open_idempotency_key)
                    .await?
            {
                if existing.remote_binding_id == input.remote_binding_id
                    && existing.binding_version == input.expected_binding_version
                    && existing.agent_binding_digest == input.expected_agent_binding_digest
                    && existing.agent_session_id == input.agent_session_id
                    && existing.initial_input_digest == input.initial_input_digest
                {
                    return Ok(RemoteOpenResult::Existing(existing));
                }
                return Err(DbError::Conflict(
                    "open idempotency key is already bound to a different Remote session"
                        .to_owned(),
                ));
            }
            let binding = fetch_binding(&mut conn, &input.owner_user_id, &input.remote_binding_id)
                .await?
                .ok_or_else(|| DbError::NotFound("Remote binding".to_owned()))?;
            if binding.binding_version != input.expected_binding_version {
                return Err(DbError::Conflict("Remote binding version changed".to_owned()));
            }
            if binding.agent_binding_digest != input.expected_agent_binding_digest {
                return Err(DbError::Conflict("Remote binding digest changed".to_owned()));
            }
            let conversation_owner: Option<String> = sqlx::query_scalar(
                "SELECT user_id FROM conversations WHERE conversation_id = ?",
            )
            .bind(&input.agent_session_id)
            .fetch_optional(&mut *conn)
            .await?;
            match conversation_owner {
                Some(owner) if owner == input.owner_user_id => {}
                Some(_) => {
                    return Err(DbError::Conflict(
                        "Conversation belongs to another owner".to_owned(),
                    ));
                }
                None => return Err(DbError::NotFound("Conversation for Remote session".to_owned())),
            }
            if fetch_session(&mut conn, &input.owner_user_id, &input.agent_session_id)
                .await?
                .is_some()
            {
                return Err(DbError::Conflict(
                    "agent_session_id is already owned by another Remote session".to_owned(),
                ));
            }
            let now = now_ms();
            sqlx::query(
                "INSERT INTO nomi_remote_sessions \
                 (agent_session_id, owner_user_id, remote_binding_id, open_idempotency_key, \
                  binding_version, agent_binding_digest, initial_input_digest, agent_binding_json, nomi_snapshot_json, \
                  provenance_json, state, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'opening', ?, ?)",
            )
            .bind(&input.agent_session_id)
            .bind(&input.owner_user_id)
            .bind(&input.remote_binding_id)
            .bind(&input.open_idempotency_key)
            .bind(binding.binding_version)
            .bind(&binding.agent_binding_digest)
            .bind(&input.initial_input_digest)
            .bind(&binding.agent_binding_json)
            .bind(&binding.nomi_snapshot_json)
            .bind(&binding.provenance_json)
            .bind(now)
            .bind(now)
            .execute(&mut *conn)
            .await?;
            let session = fetch_session(&mut conn, &input.owner_user_id, &input.agent_session_id)
                .await?
                .ok_or_else(|| DbError::Init("created Remote session disappeared".to_owned()))?;
            Ok(RemoteOpenResult::Created(session))
        }
        .await;
        match result {
            Ok(value) => {
                commit(&mut conn).await?;
                Ok(value)
            }
            Err(error) => {
                rollback(&mut conn).await;
                Err(error)
            }
        }
    }

    async fn get_session(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
    ) -> Result<Option<NomiRemoteSessionRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(agent_session_id, "agent_session_id")?;
        let mut conn = self.pool.acquire().await?;
        fetch_session(&mut conn, owner_user_id, agent_session_id).await
    }

    async fn get_session_by_open_key(
        &self,
        owner_user_id: &str,
        open_idempotency_key: &str,
    ) -> Result<Option<NomiRemoteSessionRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        if open_idempotency_key.trim().is_empty() {
            return Err(DbError::Conflict(
                "open_idempotency_key must not be empty".to_owned(),
            ));
        }
        let mut conn = self.pool.acquire().await?;
        fetch_session_by_key(&mut conn, owner_user_id, open_idempotency_key).await
    }

    async fn set_session_state(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
        state: &str,
    ) -> Result<NomiRemoteSessionRow, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(agent_session_id, "agent_session_id")?;
        validate_state(state)?;
        let mut conn = begin_immediate(&self.pool).await?;
        let result = async {
            let existing = fetch_session(&mut conn, owner_user_id, agent_session_id)
                .await?
                .ok_or_else(|| DbError::NotFound("Remote session".to_owned()))?;
            if existing.state == state {
                return Ok(existing);
            }
            let allowed = matches!(
                (existing.state.as_str(), state),
                ("opening", "ready" | "failed" | "cancelled")
                    | ("ready", "failed" | "cancelled")
            );
            if !allowed {
                return Err(DbError::Conflict(format!(
                    "Remote session state cannot transition from '{}' to '{}'",
                    existing.state, state
                )));
            }
            let now = now_ms();
            sqlx::query(
                "UPDATE nomi_remote_sessions SET state = ?, updated_at = ? \
                 WHERE owner_user_id = ? AND agent_session_id = ? AND state = ?",
            )
            .bind(state)
            .bind(now)
            .bind(owner_user_id)
            .bind(agent_session_id)
            .bind(&existing.state)
            .execute(&mut *conn)
            .await?;
            fetch_session(&mut conn, owner_user_id, agent_session_id)
                .await?
                .ok_or_else(|| DbError::Init("updated Remote session disappeared".to_owned()))
        }
        .await;
        match result {
            Ok(row) => {
                commit(&mut conn).await?;
                Ok(row)
            }
            Err(error) => {
                rollback(&mut conn).await;
                Err(error)
            }
        }
    }

    async fn transition_session_state_and_append_event(
        &self,
        input: TransitionNomiRemoteSessionParams,
    ) -> Result<NomiRemoteStateTransitionResult, DbError> {
        validate_uuid(&input.owner_user_id, "owner_user_id")?;
        validate_uuid(&input.agent_session_id, "agent_session_id")?;
        validate_state(&input.expected_state)?;
        validate_state(&input.next_state)?;
        if input.event_type.trim().is_empty() {
            return Err(DbError::Conflict(
                "Remote event type must not be empty".to_owned(),
            ));
        }
        let payload: serde_json::Value = serde_json::from_str(&input.payload_json)
            .map_err(|error| DbError::Conflict(format!("event payload JSON is invalid: {error}")))?;
        if !payload.is_object() && !payload.is_array() {
            return Err(DbError::Conflict(
                "event payload must be a JSON object or array".to_owned(),
            ));
        }
        let canonical_payload_json = canonical_json_string(&payload, "event payload")?;
        if let Some(operation_key_digest) = payload
            .get("operation_key_digest")
            .and_then(serde_json::Value::as_str)
        {
            validate_digest(operation_key_digest, "operation_key_digest")?;
        }

        let mut conn = begin_immediate(&self.pool).await?;
        let result = async {
            let existing = fetch_session(&mut conn, &input.owner_user_id, &input.agent_session_id)
                .await?
                .ok_or_else(|| DbError::NotFound("Remote session".to_owned()))?;

            // A transition operation key may already have committed before a
            // request was retried. Return the exact stored event and session
            // rather than allocating a second cursor entry. A prior request
            // phase may also exist under the same operation identity, so the
            // exact event type and latest phase are queried separately.
            let operation_event = if let Some(digest) = payload
                .get("operation_key_digest")
                .and_then(serde_json::Value::as_str)
            {
                let digest = digest.to_owned();
                Some(
                    sqlx::query_as::<_, NomiRemoteEventRow>(
                        "SELECT event_id, agent_session_id, seq, event_type, payload_json, created_at \
                         FROM nomi_remote_events \
                           WHERE agent_session_id = ? \
                           AND event_type = ? \
                           AND json_extract(payload_json, '$.operation_key_digest') = ? \
                         ORDER BY seq LIMIT 1",
                    )
                    .bind(&input.agent_session_id)
                    .bind(&input.event_type)
                    .bind(digest)
                    .fetch_optional(&mut *conn)
                    .await?,
                )
            } else {
                None
            };
            if let Some(Some(event)) = operation_event.as_ref() {
                let existing_payload: serde_json::Value =
                    serde_json::from_str(&event.payload_json).map_err(|error| {
                        DbError::Conflict(format!(
                            "persisted Remote transition payload is invalid: {error}"
                        ))
                    })?;
                if canonical_json_string(&existing_payload, "persisted event payload")?
                    != canonical_payload_json
                {
                    return Err(DbError::Conflict(
                        "Remote transition event key was reused with a different payload"
                            .to_owned(),
                    ));
                }
                if existing.state != input.next_state {
                    return Err(DbError::Conflict(format!(
                        "Remote transition event exists but session state is '{}', expected '{}'",
                        existing.state, input.next_state
                    )));
                }
                return Ok(NomiRemoteStateTransitionResult {
                    session: existing,
                    event: event.clone(),
                    changed: false,
                });
            }

            let latest_operation_event = if let Some(digest) = payload
                .get("operation_key_digest")
                .and_then(serde_json::Value::as_str)
            {
                Some(
                    sqlx::query_as::<_, NomiRemoteEventRow>(
                        "SELECT event_id, agent_session_id, seq, event_type, payload_json, created_at \
                         FROM nomi_remote_events \
                         WHERE agent_session_id = ? \
                           AND json_extract(payload_json, '$.operation_key_digest') = ? \
                         ORDER BY seq DESC LIMIT 1",
                    )
                    .bind(&input.agent_session_id)
                    .bind(digest)
                    .fetch_optional(&mut *conn)
                    .await?,
                )
            } else {
                None
            };
            if let Some(Some(event)) = latest_operation_event.as_ref()
                && !operation_event_transition_is_allowed(
                    &event.event_type,
                    &input.event_type,
                )
            {
                return Err(DbError::Conflict(format!(
                    "Remote transition event key cannot transition from '{}' to '{}'",
                    event.event_type, input.event_type
                )));
            }

            let state_changed = existing.state != input.next_state;
            if state_changed {
                if existing.state != input.expected_state {
                    return Err(DbError::Conflict(format!(
                        "Remote session state changed from expected '{}' to '{}'",
                        input.expected_state, existing.state
                    )));
                }
                let allowed = matches!(
                    (existing.state.as_str(), input.next_state.as_str()),
                    ("opening", "ready" | "failed" | "cancelled")
                        | ("ready", "failed" | "cancelled")
                );
                if !allowed {
                    return Err(DbError::Conflict(format!(
                        "Remote session state cannot transition from '{}' to '{}'",
                        existing.state, input.next_state
                    )));
                }
                let now = now_ms();
                let updated = sqlx::query(
                    "UPDATE nomi_remote_sessions SET state = ?, updated_at = ? \
                     WHERE owner_user_id = ? AND agent_session_id = ? AND state = ?",
                )
                .bind(&input.next_state)
                .bind(now)
                .bind(&input.owner_user_id)
                .bind(&input.agent_session_id)
                .bind(&input.expected_state)
                .execute(&mut *conn)
                .await?;
                if updated.rows_affected() != 1 {
                    return Err(DbError::Conflict(
                        "Remote session state transition lost its compare-and-swap race"
                            .to_owned(),
                    ));
                }
            } else if existing.state != input.expected_state {
                return Err(DbError::Conflict(format!(
                    "Remote session is '{}', not the requested expected state '{}'",
                    existing.state, input.expected_state
                )));
            }

            let event = append_event_in_transaction(
                &mut conn,
                &AppendNomiRemoteEventParams {
                    owner_user_id: input.owner_user_id.clone(),
                    agent_session_id: input.agent_session_id.clone(),
                    event_type: input.event_type.clone(),
                    payload_json: canonical_payload_json,
                },
            )
            .await?;
            let session = fetch_session(&mut conn, &input.owner_user_id, &input.agent_session_id)
                .await?
                .ok_or_else(|| DbError::Init("updated Remote session disappeared".to_owned()))?;
            Ok(NomiRemoteStateTransitionResult {
                session,
                event: event.event,
                changed: state_changed,
            })
        }
        .await;
        match result {
            Ok(value) => {
                commit(&mut conn).await?;
                Ok(value)
            }
            Err(error) => {
                rollback(&mut conn).await;
                Err(error)
            }
        }
    }

    async fn append_event(
        &self,
        input: AppendNomiRemoteEventParams,
    ) -> Result<NomiRemoteEventRow, DbError> {
        Ok(self.append_event_once(input).await?.event)
    }

    async fn append_event_once(
        &self,
        input: AppendNomiRemoteEventParams,
    ) -> Result<AppendNomiRemoteEventResult, DbError> {
        validate_uuid(&input.owner_user_id, "owner_user_id")?;
        validate_uuid(&input.agent_session_id, "agent_session_id")?;
        if input.event_type.trim().is_empty() {
            return Err(DbError::Conflict(
                "Remote event type must not be empty".to_owned(),
            ));
        }
        let mut conn = begin_immediate(&self.pool).await?;
        let result = async {
            append_event_in_transaction(&mut conn, &input).await
        }
        .await;
        match result {
            Ok(row) => {
                commit(&mut conn).await?;
                Ok(row)
            }
            Err(error) => {
                rollback(&mut conn).await;
                Err(error)
            }
        }
    }

    async fn find_event_by_operation_key(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
        event_type: &str,
        operation_key_digest: &str,
    ) -> Result<Option<NomiRemoteEventRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(agent_session_id, "agent_session_id")?;
        if event_type.trim().is_empty() {
            return Err(DbError::Conflict(
                "Remote event type must not be empty".to_owned(),
            ));
        }
        validate_digest(operation_key_digest, "operation_key_digest")?;
        let mut conn = self.pool.acquire().await?;
        fetch_event_by_operation_key(
            &mut conn,
            owner_user_id,
            agent_session_id,
            event_type,
            operation_key_digest,
        )
        .await
    }

    async fn current_event_cursor(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
    ) -> Result<i64, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(agent_session_id, "agent_session_id")?;
        let mut conn = self.pool.acquire().await?;
        fetch_event_cursor(&mut conn, owner_user_id, agent_session_id).await
    }

    async fn read_events(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> Result<NomiRemoteEventPage, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(agent_session_id, "agent_session_id")?;
        if after_seq < 0 || !(1..=1000).contains(&limit) {
            return Err(DbError::Conflict(
                "after_cursor must be non-negative and limit must be 1..=1000".to_owned(),
            ));
        }
        let mut conn = self.pool.acquire().await?;
        if fetch_session(&mut conn, owner_user_id, agent_session_id)
            .await?
            .is_none()
        {
            return Err(DbError::NotFound("Remote session".to_owned()));
        }
        let events: Vec<NomiRemoteEventRow> = sqlx::query_as(
            "SELECT event_id, agent_session_id, seq, event_type, payload_json, created_at \
             FROM nomi_remote_events WHERE agent_session_id = ? AND seq > ? \
             ORDER BY seq LIMIT ?",
        )
        .bind(agent_session_id)
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?;
        let next_cursor = events.last().map_or(after_seq, |event| event.seq);
        Ok(NomiRemoteEventPage {
            events,
            next_cursor,
        })
    }
}
