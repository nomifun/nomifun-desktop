use sqlx::SqlitePool;

use nomifun_common::{MessageId, PaginatedResult, ProviderWithModel};

use crate::error::DbError;
use crate::models::{
    ConversationArtifactRow, ConversationDeliveryReceiptRow, ConversationRow, MessageRow,
};
use crate::repository::bind::{BindValue, bind_value, bind_value_as};
use crate::repository::conversation::{
    ConversationFilters, ConversationMessageProjection, ConversationRowUpdate,
    IConversationRepository, MessageRowUpdate, MessageSearchRow, SortOrder,
};

/// SQLite-backed implementation of [`IConversationRepository`].
#[derive(Clone, Debug)]
pub struct SqliteConversationRepository {
    pool: SqlitePool,
}

impl SqliteConversationRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

async fn validate_execution_template_selection(
    pool: &SqlitePool,
    user_id: &str,
    template_id: &str,
    model: Option<&str>,
) -> Result<(), DbError> {
    let (provider_id, model) = model
        .and_then(effective_conversation_model_binding)
        .ok_or_else(|| {
            DbError::Conflict(
                "Conversation execution template requires a concrete lead model".to_owned(),
            )
        })?;
    let selectable: i64 = sqlx::query_scalar(
        "SELECT EXISTS( \
             SELECT 1 FROM agent_execution_templates template \
             JOIN agent_execution_template_participants participant \
               ON participant.template_id = template.id \
             WHERE template.id = ? AND template.user_id = ? \
               AND participant.provider_id = ? AND participant.model = ? \
         )",
    )
    .bind(template_id)
    .bind(user_id)
    .bind(provider_id)
    .bind(model)
    .fetch_one(pool)
    .await?;
    if selectable == 0 {
        return Err(DbError::Conflict(
            "Conversation execution template must be executable, owner-scoped, and contain the lead model"
                .to_owned(),
        ));
    }
    Ok(())
}

fn effective_conversation_model_binding(encoded: &str) -> Option<(String, String)> {
    let binding: ProviderWithModel = serde_json::from_str(encoded).ok()?;
    binding.validate().ok()?;
    let model = binding.use_model.unwrap_or_else(|| binding.model.clone());
    Some((binding.provider_id, model))
}

#[async_trait::async_trait]
impl IConversationRepository for SqliteConversationRepository {
    // ── Conversation CRUD ───────────────────────────────────────────

    async fn get(&self, id: &str) -> Result<Option<ConversationRow>, DbError> {
        let row = sqlx::query_as::<_, ConversationRow>("SELECT * FROM conversations WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row)
    }

    async fn create(&self, row: &ConversationRow) -> Result<String, DbError> {
        if let Some(template_id) = row.execution_template_id.as_deref() {
            validate_execution_template_selection(
                &self.pool,
                &row.user_id,
                template_id,
                row.model.as_deref(),
            )
            .await?;
        }
        sqlx::query(
            "INSERT INTO conversations \
                (id, user_id, name, type, extra, delegation_policy, execution_model_pool, \
                 decision_policy, execution_template_id, model, status, source, \
                 channel_chat_id, pinned, pinned_at, cron_job_id, preset_id, preset_revision, \
                 preset_snapshot, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.user_id)
        .bind(&row.name)
        .bind(&row.r#type)
        .bind(&row.extra)
        .bind(&row.delegation_policy)
        .bind(&row.execution_model_pool)
        .bind(&row.decision_policy)
        .bind(&row.execution_template_id)
        .bind(&row.model)
        .bind(&row.status)
        .bind(&row.source)
        .bind(&row.channel_chat_id)
        .bind(row.pinned)
        .bind(row.pinned_at)
        .bind(&row.cron_job_id)
        .bind(&row.preset_id)
        .bind(row.preset_revision)
        .bind(&row.preset_snapshot)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(row.id.clone())
    }

    async fn create_idempotent(
        &self,
        row: &ConversationRow,
        creation_key: &str,
    ) -> Result<(String, bool), DbError> {
        let creation_key = creation_key.trim();
        if creation_key.is_empty() {
            return Err(DbError::Conflict(
                "conversation creation key must not be empty".to_owned(),
            ));
        }
        if let Some(template_id) = row.execution_template_id.as_deref() {
            validate_execution_template_selection(
                &self.pool,
                &row.user_id,
                template_id,
                row.model.as_deref(),
            )
            .await?;
        }
        if let Some((conversation_id, existing_user_id)) = sqlx::query_as::<_, (String, String)>(
            "SELECT conversation_id, user_id FROM conversation_creation_keys \
             WHERE creation_key = ?",
        )
        .bind(creation_key)
        .fetch_optional(&self.pool)
        .await?
        {
            if existing_user_id != row.user_id {
                return Err(DbError::Conflict(
                    "conversation creation key belongs to another owner".to_owned(),
                ));
            }
            return Ok((conversation_id, false));
        }

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO conversations \
                (id, user_id, name, type, extra, delegation_policy, execution_model_pool, \
                 decision_policy, execution_template_id, model, status, source, \
                 channel_chat_id, pinned, pinned_at, cron_job_id, preset_id, preset_revision, \
                 preset_snapshot, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.user_id)
        .bind(&row.name)
        .bind(&row.r#type)
        .bind(&row.extra)
        .bind(&row.delegation_policy)
        .bind(&row.execution_model_pool)
        .bind(&row.decision_policy)
        .bind(&row.execution_template_id)
        .bind(&row.model)
        .bind(&row.status)
        .bind(&row.source)
        .bind(&row.channel_chat_id)
        .bind(row.pinned)
        .bind(row.pinned_at)
        .bind(&row.cron_job_id)
        .bind(&row.preset_id)
        .bind(row.preset_revision)
        .bind(&row.preset_snapshot)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await?;
        let candidate_id = row.id.clone();
        let key_result = sqlx::query(
            "INSERT INTO conversation_creation_keys \
                (creation_key, user_id, conversation_id, created_at) \
             VALUES (?, ?, ?, ?) ON CONFLICT(creation_key) DO NOTHING",
        )
        .bind(creation_key)
        .bind(&row.user_id)
        .bind(&candidate_id)
        .bind(row.created_at)
        .execute(&mut *tx)
        .await?;
        if key_result.rows_affected() == 1 {
            tx.commit().await?;
            return Ok((candidate_id, true));
        }

        // A concurrent creator committed the same operation while this writer
        // was waiting for SQLite's write lock.  Remove the unobservable
        // candidate inside this transaction and return the committed identity.
        sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(&candidate_id)
            .execute(&mut *tx)
            .await?;
        let (conversation_id, existing_user_id): (String, String) = sqlx::query_as(
            "SELECT conversation_id, user_id FROM conversation_creation_keys \
             WHERE creation_key = ?",
        )
        .bind(creation_key)
        .fetch_one(&mut *tx)
        .await?;
        if existing_user_id != row.user_id {
            return Err(DbError::Conflict(
                "conversation creation key belongs to another owner".to_owned(),
            ));
        }
        tx.commit().await?;
        Ok((conversation_id, false))
    }

    async fn find_by_creation_key(
        &self,
        user_id: &str,
        creation_key: &str,
    ) -> Result<Option<ConversationRow>, DbError> {
        Ok(sqlx::query_as::<_, ConversationRow>(
            "SELECT conversation.* FROM conversations conversation \
             JOIN conversation_creation_keys operation \
               ON operation.conversation_id = conversation.id \
             WHERE operation.creation_key = ? AND operation.user_id = ? \
               AND conversation.user_id = ?",
        )
        .bind(creation_key)
        .bind(user_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn claim_delivery_receipt(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        kind: &str,
        request_payload: &str,
        now: i64,
    ) -> Result<ConversationDeliveryReceiptRow, DbError> {
        let mut tx = self.pool.begin().await?;
        let message_id = MessageId::new().into_string();
        sqlx::query(
            "INSERT INTO conversation_delivery_receipts (\
                operation_id, message_id, conversation_id, user_id, kind, request_payload, status, \
                created_at, updated_at\
             ) SELECT ?, ?, conversation.id, ?, ?, ?, 'accepted', ?, ? \
               FROM conversations conversation \
              WHERE conversation.id = ? AND conversation.user_id = ? \
             ON CONFLICT(operation_id) DO NOTHING",
        )
        .bind(operation_id)
        .bind(message_id)
        .bind(user_id)
        .bind(kind)
        .bind(request_payload)
        .bind(now)
        .bind(now)
        .bind(conversation_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        let receipt = sqlx::query_as::<_, ConversationDeliveryReceiptRow>(
            "SELECT * FROM conversation_delivery_receipts WHERE operation_id = ?",
        )
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::NotFound("conversation delivery owner".to_owned()))?;
        if receipt.user_id != user_id
            || receipt.conversation_id != conversation_id
            || receipt.kind != kind
            || receipt.request_payload != request_payload
        {
            return Err(DbError::Conflict(
                "conversation delivery operation identity was reused".to_owned(),
            ));
        }
        tx.commit().await?;
        Ok(receipt)
    }

    async fn get_delivery_receipt(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<Option<ConversationDeliveryReceiptRow>, DbError> {
        Ok(sqlx::query_as::<_, ConversationDeliveryReceiptRow>(
            "SELECT * FROM conversation_delivery_receipts \
             WHERE operation_id = ? AND conversation_id = ? AND user_id = ?",
        )
        .bind(operation_id)
        .bind(conversation_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn complete_delivery_receipt(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        result_ok: bool,
        result_text: Option<&str>,
        result_error: Option<&str>,
        completed_at: i64,
    ) -> Result<bool, DbError> {
        let result = sqlx::query(
            "UPDATE conversation_delivery_receipts \
             SET status = 'completed', result_ok = ?, result_text = ?, result_error = ?, \
                 completed_at = MAX(created_at, updated_at, ?), \
                 updated_at = MAX(created_at, updated_at, ?) \
             WHERE operation_id = ? AND conversation_id = ? AND user_id = ? \
               AND status = 'accepted'",
        )
        .bind(result_ok)
        .bind(result_text)
        .bind(result_error)
        .bind(completed_at)
        .bind(completed_at)
        .bind(operation_id)
        .bind(conversation_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            return Ok(true);
        }
        let existing = self
            .get_delivery_receipt(user_id, conversation_id, operation_id)
            .await?;
        Ok(existing.is_some_and(|receipt| {
            receipt.status == "completed"
                && receipt.result_ok == Some(result_ok)
                && receipt.result_text.as_deref() == result_text
                && receipt.result_error.as_deref() == result_error
        }))
    }

    async fn project_assistant_message_with_receipt(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        kind: &str,
        request_payload: &str,
        message: &MessageRow,
        now: i64,
    ) -> Result<ConversationMessageProjection, DbError> {
        let valid_content = serde_json::from_str::<serde_json::Value>(&message.content)
            .is_ok_and(|value| value.is_object());
        if operation_id.trim().is_empty()
            || kind != "projection"
            || request_payload.trim().is_empty()
            || MessageId::parse(&message.id).is_err()
            || message.content.trim().is_empty()
            || !valid_content
            || message.r#type != "text"
            || message.position.as_deref() != Some("left")
            || message.status.as_deref() != Some("finish")
            || message.hidden
            || message.msg_id.as_deref() != Some(message.id.as_str())
        {
            return Err(DbError::Conflict(
                "assistant message projection requires one stable finished left-side text message"
                    .to_owned(),
            ));
        }
        if message.conversation_id != conversation_id {
            return Err(DbError::Conflict(
                "projected message does not belong to the target Conversation".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        let owned: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ? AND user_id = ?)",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if owned == 0 {
            return Err(DbError::NotFound("conversation".to_owned()));
        }

        // Claim the operation directly in its terminal state. The message is
        // inserted later in this same transaction, so either both facts commit
        // or neither one is externally visible.
        let claim = sqlx::query(
            "INSERT INTO conversation_delivery_receipts (\
                operation_id, message_id, conversation_id, user_id, kind, request_payload, status, \
                result_ok, result_text, result_error, created_at, updated_at, completed_at\
             ) VALUES (?, ?, ?, ?, ?, ?, 'completed', 1, ?, NULL, ?, ?, ?) \
             ON CONFLICT(operation_id) DO NOTHING",
        )
        .bind(operation_id)
        .bind(&message.id)
        .bind(conversation_id)
        .bind(user_id)
        .bind(kind)
        .bind(request_payload)
        .bind(&message.id)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        if claim.rows_affected() == 1 {
            sqlx::query(
                "INSERT INTO messages \
                    (id, conversation_id, msg_id, type, content, position, \
                     status, hidden, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&message.id)
            .bind(&message.conversation_id)
            .bind(&message.msg_id)
            .bind(&message.r#type)
            .bind(&message.content)
            .bind(&message.position)
            .bind(&message.status)
            .bind(message.hidden)
            .bind(message.created_at)
            .execute(&mut *tx)
            .await?;
            let persisted = sqlx::query_as::<_, MessageRow>(
                "SELECT * FROM messages WHERE id = ? AND conversation_id = ?",
            )
            .bind(&message.id)
            .bind(conversation_id)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(ConversationMessageProjection {
                inserted: true,
                message: persisted,
            });
        }

        let receipt = sqlx::query_as::<_, ConversationDeliveryReceiptRow>(
            "SELECT * FROM conversation_delivery_receipts WHERE operation_id = ?",
        )
        .bind(operation_id)
        .fetch_one(&mut *tx)
        .await?;
        if receipt.user_id != user_id
            || receipt.conversation_id != conversation_id
            || receipt.kind != kind
            || receipt.request_payload != request_payload
        {
            return Err(DbError::Conflict(
                "conversation projection operation identity was reused".to_owned(),
            ));
        }
        let message_id = Some(receipt.message_id.as_str()).filter(|_| {
            receipt.status == "completed"
                && receipt.result_ok == Some(true)
                && receipt.result_error.is_none()
        });
        let Some(message_id) = message_id else {
            return Err(DbError::Conflict(
                "conversation projection operation is not a completed message projection"
                    .to_owned(),
            ));
        };
        let persisted = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM messages WHERE id = ? AND conversation_id = ?",
        )
        .bind(message_id)
        .bind(conversation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::Conflict(
                "conversation projection receipt has no durable message".to_owned(),
            )
        })?;
        tx.commit().await?;
        Ok(ConversationMessageProjection {
            inserted: false,
            message: persisted,
        })
    }

    async fn update(&self, id: &str, updates: &ConversationRowUpdate) -> Result<(), DbError> {
        if updates.execution_template_id.is_some() || updates.model.is_some() {
            let current: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT user_id, execution_template_id, model FROM conversations WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
            if let Some((user_id, current_template_id, current_model)) = current {
                let template_id = updates
                    .execution_template_id
                    .as_ref()
                    .cloned()
                    .unwrap_or(current_template_id);
                let model = updates.model.as_ref().cloned().unwrap_or(current_model);
                if let Some(template_id) = template_id.as_deref() {
                    validate_execution_template_selection(
                        &self.pool,
                        &user_id,
                        template_id,
                        model.as_deref(),
                    )
                    .await?;
                }
            }
        }
        // Build dynamic SET clause
        let mut set_parts: Vec<String> = Vec::new();
        let mut binds: Vec<BindValue> = Vec::new();

        if let Some(ref name) = updates.name {
            set_parts.push("name = ?".to_string());
            binds.push(BindValue::Str(name.clone()));
        }
        if let Some(pinned) = updates.pinned {
            set_parts.push("pinned = ?".to_string());
            binds.push(BindValue::Bool(pinned));
        }
        if let Some(ref pinned_at) = updates.pinned_at {
            set_parts.push("pinned_at = ?".to_string());
            binds.push(BindValue::OptI64(*pinned_at));
        }
        if let Some(ref model) = updates.model {
            set_parts.push("model = ?".to_string());
            binds.push(BindValue::OptStr(model.clone()));
        }
        if let Some(ref extra) = updates.extra {
            set_parts.push("extra = ?".to_string());
            binds.push(BindValue::Str(extra.clone()));
        }
        if let Some(ref delegation_policy) = updates.delegation_policy {
            set_parts.push("delegation_policy = ?".to_string());
            binds.push(BindValue::Str(delegation_policy.clone()));
        }
        if let Some(ref execution_model_pool) = updates.execution_model_pool {
            set_parts.push("execution_model_pool = ?".to_string());
            binds.push(BindValue::OptStr(execution_model_pool.clone()));
        }
        if let Some(ref decision_policy) = updates.decision_policy {
            set_parts.push("decision_policy = ?".to_string());
            binds.push(BindValue::Str(decision_policy.clone()));
        }
        if let Some(ref execution_template_id) = updates.execution_template_id {
            set_parts.push("execution_template_id = ?".to_string());
            binds.push(BindValue::OptStr(execution_template_id.clone()));
        }
        if let Some(ref status) = updates.status {
            set_parts.push("status = ?".to_string());
            binds.push(BindValue::Str(status.clone()));
        }
        if let Some(ref cron_job_id) = updates.cron_job_id {
            set_parts.push("cron_job_id = ?".to_string());
            binds.push(BindValue::OptStr(cron_job_id.clone()));
        }
        if let Some(ref preset_id) = updates.preset_id {
            set_parts.push("preset_id = ?".to_string());
            binds.push(BindValue::OptStr(preset_id.clone()));
        }
        if let Some(ref preset_revision) = updates.preset_revision {
            set_parts.push("preset_revision = ?".to_string());
            binds.push(BindValue::OptI64(*preset_revision));
        }
        if let Some(ref preset_snapshot) = updates.preset_snapshot {
            set_parts.push("preset_snapshot = ?".to_string());
            binds.push(BindValue::OptStr(preset_snapshot.clone()));
        }
        if let Some(updated_at) = updates.updated_at {
            set_parts.push("updated_at = ?".to_string());
            binds.push(BindValue::I64(updated_at));
        }

        if set_parts.is_empty() {
            return Ok(());
        }

        let sql = format!("UPDATE conversations SET {} WHERE id = ?", set_parts.join(", "));

        let mut query = sqlx::query(&sql);
        for bind in &binds {
            query = bind_value(query, bind);
        }
        query = query.bind(id);

        let result = query.execute(&self.pool).await?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Conversation '{id}' not found")));
        }

        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Conversation '{id}' not found")));
        }

        Ok(())
    }

    async fn list_paginated(
        &self,
        user_id: &str,
        filters: &ConversationFilters,
    ) -> Result<PaginatedResult<ConversationRow>, DbError> {
        let limit = filters.effective_limit();
        // Fetch one extra row to determine hasMore
        let fetch_limit = limit + 1;

        let mut where_parts = vec!["c.user_id = ?".to_string()];
        let mut binds: Vec<BindValue> = vec![BindValue::Str(user_id.to_string())];

        // Cursor-based pagination: use updated_at of the cursor row
        if let Some(cursor_id) = &filters.cursor {
            where_parts.push(
                "(c.updated_at < (SELECT updated_at FROM conversations WHERE id = ?) \
                 OR (c.updated_at = (SELECT updated_at FROM conversations WHERE id = ?) \
                     AND c.id < ?))"
                    .to_string(),
            );
            binds.push(BindValue::Str(cursor_id.clone()));
            binds.push(BindValue::Str(cursor_id.clone()));
            binds.push(BindValue::Str(cursor_id.clone()));
        }

        append_filter_conditions(filters, &mut where_parts, &mut binds);

        let where_clause = where_parts.join(" AND ");

        // Count total matching rows (without cursor filter for total)
        let count_sql = build_count_sql(user_id, filters);
        let total = execute_count(&self.pool, &count_sql.0, &count_sql.1).await?;

        // Fetch page
        let sql = format!(
            "SELECT c.* FROM conversations c \
             WHERE {where_clause} \
             ORDER BY c.updated_at DESC, c.id DESC \
             LIMIT ?"
        );

        let mut query = sqlx::query_as::<_, ConversationRow>(&sql);
        for bind in &binds {
            query = bind_value_as(query, bind);
        }
        query = query.bind(fetch_limit);

        let mut rows = query.fetch_all(&self.pool).await?;

        let has_more = rows.len() as u32 > limit;
        if has_more {
            rows.pop();
        }

        Ok(PaginatedResult {
            items: rows,
            total,
            has_more,
        })
    }

    // ── Extended queries ────────────────────────────────────────────

    async fn find_by_source_and_chat(
        &self,
        user_id: &str,
        source: &str,
        chat_id: &str,
        agent_type: &str,
    ) -> Result<Option<ConversationRow>, DbError> {
        let row = sqlx::query_as::<_, ConversationRow>(
            "SELECT * FROM conversations \
             WHERE user_id = ? AND source = ? AND channel_chat_id = ? AND type = ? \
             ORDER BY updated_at DESC \
             LIMIT 1",
        )
        .bind(user_id)
        .bind(source)
        .bind(chat_id)
        .bind(agent_type)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    async fn list_by_cron_job(&self, user_id: &str, cron_job_id: &str) -> Result<Vec<ConversationRow>, DbError> {
        let rows = sqlx::query_as::<_, ConversationRow>(
            "SELECT * FROM conversations \
             WHERE user_id = ? \
             AND cron_job_id = ? \
             ORDER BY updated_at DESC",
        )
        .bind(user_id)
        .bind(cron_job_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn list_associated(&self, user_id: &str, conversation_id: &str) -> Result<Vec<ConversationRow>, DbError> {
        // First get the target conversation's workspace
        let target = sqlx::query_as::<_, ConversationRow>("SELECT * FROM conversations WHERE id = ? AND user_id = ?")
            .bind(conversation_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Conversation '{conversation_id}' not found")))?;

        // Extract workspace from extra JSON
        let workspace: Option<String> = serde_json::from_str::<serde_json::Value>(&target.extra)
            .ok()
            .and_then(|v: serde_json::Value| v.get("workspace")?.as_str().map(String::from));

        let Some(ref workspace) = workspace else {
            return Ok(Vec::new());
        };

        if workspace.is_empty() {
            return Ok(Vec::new());
        }

        // Find other conversations with the same workspace
        let rows = sqlx::query_as::<_, ConversationRow>(
            "SELECT * FROM conversations \
             WHERE user_id = ? \
               AND id != ? \
               AND json_extract(extra, '$.workspace') = ? \
             ORDER BY updated_at DESC",
        )
        .bind(user_id)
        .bind(conversation_id)
        .bind(workspace)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn list_conversations_using_model_provider(
        &self,
        provider_id: &str,
    ) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT id, name FROM conversations \
             WHERE model IS NOT NULL AND json_valid(model) \
               AND json_extract(model, '$.provider_id') = ? \
             ORDER BY updated_at DESC, id",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?)
    }

    // ── Message operations ──────────────────────────────────────────

    async fn get_messages(
        &self,
        conv_id: &str,
        page: u32,
        page_size: u32,
        order: SortOrder,
    ) -> Result<PaginatedResult<MessageRow>, DbError> {
        let effective_page = if page == 0 { 1 } else { page };
        let effective_size = if page_size == 0 { 50 } else { page_size };
        let offset = (effective_page - 1) * effective_size;
        let fetch_limit = effective_size + 1;

        let count_row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM messages \
                 WHERE conversation_id = ? \
                   AND type NOT IN ('cron_trigger', 'skill_suggest')",
        )
        .bind(conv_id)
        .fetch_one(&self.pool)
        .await?;
        let total = count_row.0 as u64;

        let sql = format!(
            "SELECT * FROM messages \
             WHERE conversation_id = ? \
               AND type NOT IN ('cron_trigger', 'skill_suggest') \
             ORDER BY created_at {}, id {} \
             LIMIT ? OFFSET ?",
            order.as_sql(),
            order.as_sql()
        );

        let mut rows = sqlx::query_as::<_, MessageRow>(&sql)
            .bind(conv_id)
            .bind(fetch_limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let has_more = rows.len() as u32 > effective_size;
        if has_more {
            rows.pop();
        }

        Ok(PaginatedResult {
            items: rows,
            total,
            has_more,
        })
    }

    async fn get_messages_keyset(
        &self,
        conv_id: &str,
        before: Option<(i64, String)>,
        limit: u32,
    ) -> Result<PaginatedResult<MessageRow>, DbError> {
        let effective_limit = if limit == 0 { 40 } else { limit };
        let fetch_limit = effective_limit + 1;

        // Newest-first window; the keyset predicate `(created_at, id) < cursor`
        // is covered by idx_messages_conv_created_id. `id` (msg_{uuidv7}, time-
        // ordered) is the stable tiebreaker for rows sharing a `created_at` ms.
        let mut rows = if let Some((before_created_at, before_id)) = before {
            sqlx::query_as::<_, MessageRow>(
                "SELECT * FROM messages \
                 WHERE conversation_id = ? \
                   AND type NOT IN ('cron_trigger', 'skill_suggest') \
                   AND (created_at < ? OR (created_at = ? AND id < ?)) \
                 ORDER BY created_at DESC, id DESC \
                 LIMIT ?",
            )
            .bind(conv_id)
            .bind(before_created_at)
            .bind(before_created_at)
            .bind(&before_id)
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MessageRow>(
                "SELECT * FROM messages \
                 WHERE conversation_id = ? \
                   AND type NOT IN ('cron_trigger', 'skill_suggest') \
                 ORDER BY created_at DESC, id DESC \
                 LIMIT ?",
            )
            .bind(conv_id)
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        };

        let has_more = rows.len() as u32 > effective_limit;
        if has_more {
            rows.pop();
        }

        Ok(PaginatedResult {
            items: rows,
            total: 0, // keyset windows don't compute a full count
            has_more,
        })
    }

    async fn get_message(&self, conv_id: &str, message_id: &str) -> Result<Option<MessageRow>, DbError> {
        let row = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM messages \
             WHERE conversation_id = ? \
               AND id = ? \
               AND type NOT IN ('cron_trigger', 'skill_suggest')",
        )
        .bind(conv_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    async fn insert_message(&self, message: &MessageRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO messages \
                (id, conversation_id, msg_id, type, content, position, \
                 status, hidden, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&message.id)
        .bind(&message.conversation_id)
        .bind(&message.msg_id)
        .bind(&message.r#type)
        .bind(&message.content)
        .bind(&message.position)
        .bind(&message.status)
        .bind(message.hidden)
        .bind(message.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn claim_message_correlation(
        &self,
        conversation_id: &str,
        turn_message_id: &str,
        message_type: &str,
        correlation_key: &str,
    ) -> Result<String, DbError> {
        nomifun_common::ConversationId::parse(conversation_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        MessageId::parse(turn_message_id).map_err(|error| DbError::Conflict(error.to_string()))?;
        let message_type = message_type.trim();
        let correlation_key = correlation_key.trim();
        if message_type.is_empty() || correlation_key.is_empty() {
            return Err(DbError::Conflict(
                "message correlation type and key must be non-empty canonical strings".to_owned(),
            ));
        }

        let candidate = MessageId::new().into_string();
        sqlx::query(
            "INSERT INTO message_correlations \
             (conversation_id, turn_message_id, message_type, correlation_key, message_id) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(conversation_id, turn_message_id, message_type, correlation_key) DO NOTHING",
        )
        .bind(conversation_id)
        .bind(turn_message_id)
        .bind(message_type)
        .bind(correlation_key)
        .bind(&candidate)
        .execute(&self.pool)
        .await?;

        sqlx::query_scalar(
            "SELECT message_id FROM message_correlations \
             WHERE conversation_id = ? AND turn_message_id = ? \
               AND message_type = ? AND correlation_key = ?",
        )
        .bind(conversation_id)
        .bind(turn_message_id)
        .bind(message_type)
        .bind(correlation_key)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    async fn update_message(&self, id: &str, updates: &MessageRowUpdate) -> Result<(), DbError> {
        let mut set_parts: Vec<String> = Vec::new();
        let mut binds: Vec<BindValue> = Vec::new();

        if let Some(ref content) = updates.content {
            set_parts.push("content = ?".to_string());
            binds.push(BindValue::Str(content.clone()));
        }
        if let Some(ref status) = updates.status {
            set_parts.push("status = ?".to_string());
            binds.push(BindValue::OptStr(status.clone()));
        }
        if let Some(hidden) = updates.hidden {
            set_parts.push("hidden = ?".to_string());
            binds.push(BindValue::Bool(hidden));
        }

        if set_parts.is_empty() {
            return Ok(());
        }

        let sql = format!("UPDATE messages SET {} WHERE id = ?", set_parts.join(", "));

        let mut query = sqlx::query(&sql);
        for bind in &binds {
            query = bind_value(query, bind);
        }
        query = query.bind(id);

        let result = query.execute(&self.pool).await?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Message '{id}' not found")));
        }

        Ok(())
    }

    async fn delete_messages_by_conversation(&self, conv_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM messages WHERE conversation_id = ?")
            .bind(conv_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn delete_messages_from(
        &self,
        conv_id: &str,
        from_created_at: i64,
        from_id: &str,
    ) -> Result<u64, DbError> {
        // Keyset 截断：删除 (created_at, id) >= 游标 的所有消息。
        // 命中复合索引 idx_messages_conv_created_id。
        let result = sqlx::query(
            "DELETE FROM messages \
             WHERE conversation_id = ? \
               AND (created_at > ? OR (created_at = ? AND id >= ?))",
        )
        .bind(conv_id)
        .bind(from_created_at)
        .bind(from_created_at)
        .bind(from_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    async fn get_message_by_msg_id(
        &self,
        conv_id: &str,
        msg_id: &str,
        msg_type: &str,
    ) -> Result<Option<MessageRow>, DbError> {
        let row = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM messages \
             WHERE conversation_id = ? AND msg_id = ? AND type = ?",
        )
        .bind(conv_id)
        .bind(msg_id)
        .bind(msg_type)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    async fn search_messages(
        &self,
        user_id: &str,
        keyword: &str,
        page: u32,
        page_size: u32,
    ) -> Result<PaginatedResult<MessageSearchRow>, DbError> {
        let effective_page = if page == 0 { 1 } else { page };
        let effective_size = if page_size == 0 { 20 } else { page_size };
        let offset = (effective_page - 1) * effective_size;
        let fetch_limit = effective_size + 1;

        let like_pattern = format!("%{keyword}%");

        let count_row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM messages m \
             INNER JOIN conversations c ON m.conversation_id = c.id \
             WHERE c.user_id = ? AND m.content LIKE ?",
        )
        .bind(user_id)
        .bind(&like_pattern)
        .fetch_one(&self.pool)
        .await?;
        let total = count_row.0 as u64;

        let rows = sqlx::query_as::<_, MessageSearchRow>(
            "SELECT \
                m.id AS message_id, \
                m.type, \
                m.content, \
                m.created_at, \
                c.id AS conversation_id, \
                c.name AS conversation_name, \
                c.type AS conversation_type, \
                c.extra AS conversation_extra, \
                c.delegation_policy AS conversation_delegation_policy, \
                c.execution_model_pool AS conversation_execution_model_pool, \
                c.decision_policy AS conversation_decision_policy, \
                c.execution_template_id AS conversation_execution_template_id, \
                c.model AS conversation_model, \
                c.status AS conversation_status, \
                c.source AS conversation_source, \
                c.channel_chat_id AS conversation_channel_chat_id, \
                c.pinned AS conversation_pinned, \
                c.pinned_at AS conversation_pinned_at, \
                c.created_at AS conversation_created_at, \
                c.updated_at AS conversation_updated_at \
             FROM messages m \
             INNER JOIN conversations c ON m.conversation_id = c.id \
             WHERE c.user_id = ? AND m.content LIKE ? \
             ORDER BY m.created_at DESC \
             LIMIT ? OFFSET ?",
        )
        .bind(user_id)
        .bind(&like_pattern)
        .bind(fetch_limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let has_more = rows.len() as u32 > effective_size;
        let items = if has_more {
            rows[..effective_size as usize].to_vec()
        } else {
            rows
        };

        Ok(PaginatedResult { items, total, has_more })
    }

    async fn list_artifacts(&self, conversation_id: &str) -> Result<Vec<ConversationArtifactRow>, DbError> {
        let rows = sqlx::query_as::<_, ConversationArtifactRow>(
            "SELECT * FROM conversation_artifacts \
             WHERE conversation_id = ? \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn get_artifact(
        &self,
        conversation_id: &str,
        artifact_id: &str,
    ) -> Result<Option<ConversationArtifactRow>, DbError> {
        let row = sqlx::query_as::<_, ConversationArtifactRow>(
            "SELECT * FROM conversation_artifacts WHERE conversation_id = ? AND id = ?",
        )
        .bind(conversation_id)
        .bind(artifact_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    async fn upsert_artifact(&self, artifact: &ConversationArtifactRow) -> Result<ConversationArtifactRow, DbError> {
        // Idempotency depends on `kind`:
        //   - skill_suggest: upsert against the partial UNIQUE index
        //     uq_conversation_artifacts_skill_suggest
        //     ON (conversation_id, cron_job_id) WHERE kind = 'skill_suggest'.
        //     The ON CONFLICT target must repeat the same WHERE predicate.
        //   - cron_trigger: plain INSERT, one row per trigger (no unique
        //     constraint, no ON CONFLICT clause).
        let id = if artifact.kind == "skill_suggest" {
            sqlx::query_scalar::<_, String>(
                "INSERT INTO conversation_artifacts \
                    (id, conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(conversation_id, cron_job_id) WHERE kind = 'skill_suggest' DO UPDATE SET \
                    status = excluded.status, \
                    payload = excluded.payload, \
                    updated_at = excluded.updated_at \
                 RETURNING id",
            )
            .bind(&artifact.id)
            .bind(&artifact.conversation_id)
            .bind(&artifact.cron_job_id)
            .bind(&artifact.kind)
            .bind(&artifact.status)
            .bind(&artifact.payload)
            .bind(artifact.created_at)
            .bind(artifact.updated_at)
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query(
                "INSERT INTO conversation_artifacts \
                    (id, conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&artifact.id)
            .bind(&artifact.conversation_id)
            .bind(&artifact.cron_job_id)
            .bind(&artifact.kind)
            .bind(&artifact.status)
            .bind(&artifact.payload)
            .bind(artifact.created_at)
            .bind(artifact.updated_at)
            .execute(&self.pool)
            .await?;
            artifact.id.clone()
        };

        self.get_artifact(&artifact.conversation_id, &id)
            .await?
            .ok_or_else(|| DbError::Init(format!("upsert artifact did not produce row for id '{id}'")))
    }

    async fn update_artifact_status(
        &self,
        conversation_id: &str,
        artifact_id: &str,
        status: &str,
        updated_at: i64,
    ) -> Result<Option<ConversationArtifactRow>, DbError> {
        let result = sqlx::query(
            "UPDATE conversation_artifacts \
             SET status = ?, updated_at = ? \
             WHERE conversation_id = ? AND id = ?",
        )
        .bind(status)
        .bind(updated_at)
        .bind(conversation_id)
        .bind(artifact_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_artifact(conversation_id, artifact_id).await
    }

    async fn mark_skill_suggest_artifacts_saved(
        &self,
        user_id: &str,
        cron_job_id: &str,
        updated_at: i64,
    ) -> Result<Vec<ConversationArtifactRow>, DbError> {
        sqlx::query(
            "UPDATE conversation_artifacts AS artifact \
             SET status = 'saved', updated_at = ? \
             WHERE artifact.kind = 'skill_suggest' \
               AND artifact.cron_job_id = ? \
               AND artifact.status != 'saved' \
               AND EXISTS (\
                   SELECT 1 \
                   FROM conversations AS conversation \
                   JOIN cron_jobs AS job ON job.id = artifact.cron_job_id \
                   WHERE conversation.id = artifact.conversation_id \
                     AND conversation.user_id = ? \
                     AND job.user_id = ?\
               )",
        )
        .bind(updated_at)
        .bind(cron_job_id)
        .bind(user_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        let rows = sqlx::query_as::<_, ConversationArtifactRow>(
            "SELECT artifact.* \
             FROM conversation_artifacts AS artifact \
             JOIN conversations AS conversation ON conversation.id = artifact.conversation_id \
             JOIN cron_jobs AS job ON job.id = artifact.cron_job_id \
             WHERE artifact.kind = 'skill_suggest' \
               AND artifact.cron_job_id = ? \
               AND conversation.user_id = ? \
               AND job.user_id = ? \
             ORDER BY artifact.created_at ASC, artifact.id ASC",
        )
        .bind(cron_job_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn delete_artifacts_by_conversation(&self, conversation_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM conversation_artifacts WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn list_legacy_cron_trigger_messages(&self, conversation_id: &str) -> Result<Vec<MessageRow>, DbError> {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM messages \
             WHERE conversation_id = ? AND type = 'cron_trigger' \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // ── conversation_mcp_servers junction ───────────────────────────

    async fn list_mcp_server_ids(&self, conversation_id: &str) -> Result<Vec<nomifun_common::McpServerId>, DbError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT mcp_server_id FROM conversation_mcp_servers \
             WHERE conversation_id = ? \
             ORDER BY sort_order ASC, mcp_server_id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        ids.into_iter()
            .map(|id| nomifun_common::McpServerId::parse(id).map_err(|e| DbError::Init(e.to_string())))
            .collect()
    }

    async fn set_mcp_server_ids(&self, conversation_id: &str, ids: &[nomifun_common::McpServerId]) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM conversation_mcp_servers WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;

        for (sort_order, mcp_server_id) in ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO conversation_mcp_servers (conversation_id, mcp_server_id, sort_order) \
                 VALUES (?, ?, ?)",
            )
            .bind(conversation_id)
            .bind(mcp_server_id.as_str())
            .bind(sort_order as i64)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
}

// ── Dynamic bind helpers ────────────────────────────────────────────

/// Appends shared filter conditions (source, cron_job_id, pinned) to WHERE
/// clause parts and bind values. Used by both `list_paginated` and the count
/// query to keep filter logic in one place.
fn append_filter_conditions(filters: &ConversationFilters, where_parts: &mut Vec<String>, binds: &mut Vec<BindValue>) {
    if let Some(ref source) = filters.source {
        where_parts.push("c.source = ?".to_string());
        binds.push(BindValue::Str(source.clone()));
    }
    if let Some(ref cron_job_id) = filters.cron_job_id {
        where_parts.push("c.cron_job_id = ?".to_string());
        binds.push(BindValue::Str(cron_job_id.clone()));
    }
    if let Some(pinned) = filters.pinned {
        where_parts.push("c.pinned = ?".to_string());
        binds.push(BindValue::Bool(pinned));
    }
    // Companion companion (work-partner) 单会话不计入普通会话列表/计数。
    // `extra.companion_session` 为 1 的行被排除;`IS NOT 1` 同时覆盖缺失/为 0
    // 的普通会话(json_extract 返回 NULL 时 `NULL IS NOT 1` 为真)。
    if filters.exclude_companion_companion {
        where_parts.push("json_extract(c.extra, '$.companion_session') IS NOT 1".to_string());
    }
    // Attempt conversations are aggregate-internal execution surfaces.  The
    // explicit execution link is the only source of truth; legacy JSON-extra
    // identity markers are deliberately ignored after migration 037.
    where_parts.push(
        "NOT EXISTS (SELECT 1 FROM conversation_execution_links execution_link \
         WHERE execution_link.conversation_id = c.id \
           AND execution_link.relation = 'attempt')"
            .to_string(),
    );
}

/// Builds a count query and bind values for the total (ignoring cursor).
fn build_count_sql(user_id: &str, filters: &ConversationFilters) -> (String, Vec<BindValue>) {
    let mut where_parts = vec!["c.user_id = ?".to_string()];
    let mut binds: Vec<BindValue> = vec![BindValue::Str(user_id.to_string())];

    append_filter_conditions(filters, &mut where_parts, &mut binds);

    let sql = format!(
        "SELECT COUNT(*) FROM conversations c WHERE {}",
        where_parts.join(" AND ")
    );

    (sql, binds)
}

/// Executes a dynamic count query.
async fn execute_count(pool: &SqlitePool, sql: &str, binds: &[BindValue]) -> Result<u64, DbError> {
    let mut query = sqlx::query_as::<_, (i64,)>(sql);
    for bind in binds {
        query = bind_value_as(query, bind);
    }
    let row = query.fetch_one(pool).await?;
    Ok(row.0 as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_INSTALLATION_OWNER: &str =
        "user_0190f5fe-7c00-7a00-8000-000000000001";

    async fn init_database_memory() -> Result<crate::Database, crate::DbError> {
        crate::init_database_memory_with_owner(
            nomifun_common::UserId::parse(TEST_INSTALLATION_OWNER.to_owned())
                .expect("canonical fixture owner"),
        )
        .await
    }

    async fn insert_fixture_provider(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO providers (\
                id, platform, name, base_url, api_key_encrypted, models, enabled, \
                capabilities, created_at, updated_at\
             ) VALUES (?, 'openai', ?, 'https://example.invalid', \
                       'encrypted', '[]', 1, '[]', 0, 0)",
        )
        .bind(id)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn setup() -> (SqliteConversationRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        insert_fixture_provider(
            db.pool(),
            "prov_0190f5fe-7c00-7a00-8abc-012345678901",
        )
        .await;
        let repo = SqliteConversationRepository::new(db.pool().clone());
        (repo, db)
    }

    fn sample_conversation(user_id: &str) -> ConversationRow {
        let now = nomifun_common::now_ms();
        ConversationRow {
            id: nomifun_common::ConversationId::new().into_string(),
            user_id: user_id.to_string(),
            name: "Test Conversation".to_string(),
            r#type: "gemini".to_string(),
            extra: r#"{"workspace":"/home/user/project"}"#.to_string(),
            delegation_policy: "automatic".to_string(),
            execution_model_pool: None,
            decision_policy: "automatic".to_string(),
            execution_template_id: None,
            model: Some(r#"{"provider_id":"prov_0190f5fe-7c00-7a00-8abc-012345678901","model":"claude-sonnet-4-20250514"}"#.to_string()),
            status: Some("pending".to_string()),
            source: Some("nomifun".to_string()),
            channel_chat_id: None,
            pinned: false,
            pinned_at: None,
            cron_job_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_message(conv_id: impl Into<String>) -> MessageRow {
        let now = nomifun_common::now_ms();
        MessageRow {
            id: nomifun_common::generate_prefixed_id("msg"),
            conversation_id: conv_id.into(),
            msg_id: Some("client_msg_1".to_string()),
            r#type: "text".to_string(),
            content: r#"{"content":"Hello world"}"#.to_string(),
            position: Some("right".to_string()),
            status: Some("finish".to_string()),
            hidden: false,
            created_at: now,
        }
    }

    /// Inserts a minimal valid `cron_jobs` row so conversations can reference it
    /// via the `cron_job_id` FK (foreign_keys is ON in the test pool).
    async fn insert_cron_job(pool: &SqlitePool, id: &str) {
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO cron_jobs \
                (id, user_id, name, schedule_kind, schedule_value, payload_message, agent_type, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, 'every', '60', '', 'gemini', 'user', ?, ?)",
        )
        .bind(id)
        .bind(TEST_INSTALLATION_OWNER)
        .bind(format!("job {id}"))
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Inserts a minimal valid `mcp_servers` row and returns its entity id so the
    /// junction can reference it via the `mcp_server_id` FK.
    async fn insert_mcp_server(pool: &SqlitePool, name: &str) -> nomifun_common::McpServerId {
        let id = nomifun_common::McpServerId::new();
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO mcp_servers \
                (id, name, transport_type, transport_config, created_at, updated_at) \
             VALUES (?, ?, 'stdio', '{}', ?, ?)",
        )
        .bind(id.as_str())
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    fn sample_artifact(conversation_id: impl Into<String>, kind: &str, cron_job_id: Option<&str>) -> ConversationArtifactRow {
        let now = nomifun_common::now_ms();
        ConversationArtifactRow {
            id: nomifun_common::ConversationArtifactId::new().into_string(),
            conversation_id: conversation_id.into(),
            cron_job_id: cron_job_id.map(str::to_string),
            kind: kind.to_string(),
            status: "active".to_string(),
            payload: r#"{}"#.to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    async fn link_lead_and_attempt_conversations(
        pool: &SqlitePool,
        lead_conversation_id: &str,
        attempt_conversation_id: &str,
    ) {
        let now = nomifun_common::now_ms();
        let execution_id = nomifun_common::AgentExecutionId::new().into_string();
        let participant_id = nomifun_common::AgentExecutionParticipantId::new().into_string();
        let step_id = nomifun_common::AgentExecutionStepId::new().into_string();
        let attempt_id = nomifun_common::AgentExecutionAttemptId::new().into_string();

        sqlx::query(
             "INSERT INTO agent_executions \
             (id, user_id, goal, status, plan_gate, adaptation_policy, decision_policy, \
              delegation_policy, max_parallel, initial_plan_input, created_at, updated_at) \
             VALUES (?, ?, 'test execution', 'running', 'automatic', 'fixed', \
                     'automatic', 'automatic', 4, '{\"mode\":\"automatic\"}', ?, ?)",
        )
        .bind(&execution_id)
        .bind(TEST_INSTALLATION_OWNER)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO agent_execution_participants \
             (id, execution_id, source_agent_id, provider_id, model, \
              introduced_in_revision, created_at) \
             VALUES (?, ?, 'test-agent', 'prov_0190f5fe-7c00-7a00-8abc-012345678901', 'fixture-model', 0, ?)",
        )
        .bind(&participant_id)
        .bind(&execution_id)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO agent_execution_steps \
             (id, execution_id, title, spec, kind, agent_mode, status, \
              assigned_participant_id, assignment_source, introduced_in_revision, \
              created_at, updated_at) \
             VALUES (?, ?, 'test step', 'test step', 'agent', 'normal', 'running', \
                     ?, 'manual', 0, ?, ?)",
        )
        .bind(&step_id)
        .bind(&execution_id)
        .bind(&participant_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO agent_execution_attempts \
             (id, execution_id, step_id, attempt_no, participant_id, status, \
              trigger_reason, started_at, created_at, updated_at) \
             VALUES (?, ?, ?, 0, ?, 'running', 'initial', ?, ?, ?)",
        )
        .bind(&attempt_id)
        .bind(&execution_id)
        .bind(&step_id)
        .bind(&participant_id)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO conversation_execution_links \
             (id, conversation_id, execution_id, relation, active, created_at, updated_at) \
             VALUES (?, ?, ?, 'lead', 1, ?, ?)",
        )
        .bind(nomifun_common::ConversationExecutionLinkId::new().into_string())
        .bind(lead_conversation_id)
        .bind(&execution_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO conversation_execution_links \
             (id, conversation_id, execution_id, relation, step_id, attempt_id, \
              active, created_at, updated_at) \
             VALUES (?, ?, ?, 'attempt', ?, ?, 1, ?, ?)",
        )
        .bind(nomifun_common::ConversationExecutionLinkId::new().into_string())
        .bind(attempt_conversation_id)
        .bind(&execution_id)
        .bind(&step_id)
        .bind(&attempt_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
    }

    // ── Conversation CRUD tests ─────────────────────────────────────

    #[tokio::test]
    async fn create_and_get_conversation() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);

        conv.id = repo.create(&conv).await.unwrap();
        assert!(nomifun_common::ConversationId::parse(conv.id.clone()).is_ok());
        let found = repo.get(&conv.id).await.unwrap().unwrap();

        assert_eq!(found.id, conv.id);
        assert_eq!(found.name, "Test Conversation");
        assert_eq!(found.r#type, "gemini");
        assert_eq!(found.status.as_deref(), Some("pending"));
        assert!(!found.pinned);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let (repo, _db) = setup().await;
        assert!(repo.get("conv_019abcdef012-7abc-8abc-0123-456789abcdee").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_conversation_name() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let now = nomifun_common::now_ms();
        repo.update(
            &conv.id,
            &ConversationRowUpdate {
                name: Some("Updated Name".to_string()),
                updated_at: Some(now),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let found = repo.get(&conv.id).await.unwrap().unwrap();
        assert_eq!(found.name, "Updated Name");
        assert!(found.updated_at >= conv.updated_at);
    }

    #[tokio::test]
    async fn update_conversation_pinned() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let pin_time = nomifun_common::now_ms();
        repo.update(
            &conv.id,
            &ConversationRowUpdate {
                pinned: Some(true),
                pinned_at: Some(Some(pin_time)),
                updated_at: Some(pin_time),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let found = repo.get(&conv.id).await.unwrap().unwrap();
        assert!(found.pinned);
        assert_eq!(found.pinned_at, Some(pin_time));
    }

    #[tokio::test]
    async fn update_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo
            .update(
                "conv_019abcdef012-7abc-8abc-0123-456789abcdee",
                &ConversationRowUpdate {
                    name: Some("x".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_empty_is_noop() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        // Empty update should succeed without error
        repo.update(&conv.id, &ConversationRowUpdate::default()).await.unwrap();
    }

    #[tokio::test]
    async fn delete_conversation() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        repo.delete(&conv.id).await.unwrap();
        assert!(repo.get(&conv.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_cascades_messages() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let msg = sample_message(conv.id.clone());
        repo.insert_message(&msg).await.unwrap();

        repo.delete(&conv.id).await.unwrap();

        // Messages should be gone due to CASCADE
        let result = repo.get_messages(&conv.id, 1, 50, SortOrder::Desc).await.unwrap();
        assert!(result.items.is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.delete("conv_019abcdef012-7abc-8abc-0123-456789abcdee").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    // ── Pagination tests ────────────────────────────────────────────

    #[tokio::test]
    async fn list_empty() {
        let (repo, _db) = setup().await;
        let result = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    limit: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(result.items.is_empty());
        assert_eq!(result.total, 0);
        assert!(!result.has_more);
    }

    #[tokio::test]
    async fn list_ordered_by_updated_at_desc() {
        let (repo, _db) = setup().await;

        let mut c1 = sample_conversation(TEST_INSTALLATION_OWNER);
        c1.name = "First".to_string();
        c1.updated_at = 1000;
        repo.create(&c1).await.unwrap();

        let mut c2 = sample_conversation(TEST_INSTALLATION_OWNER);
        c2.name = "Second".to_string();
        c2.updated_at = 2000;
        repo.create(&c2).await.unwrap();

        let mut c3 = sample_conversation(TEST_INSTALLATION_OWNER);
        c3.name = "Third".to_string();
        c3.updated_at = 3000;
        repo.create(&c3).await.unwrap();

        let result = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    limit: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.items.len(), 3);
        assert_eq!(result.total, 3);
        assert_eq!(result.items[0].name, "Third");
        assert_eq!(result.items[1].name, "Second");
        assert_eq!(result.items[2].name, "First");
    }

    #[tokio::test]
    async fn list_cursor_pagination() {
        let (repo, _db) = setup().await;

        let mut convs = Vec::new();
        for i in 0..5 {
            let mut c = sample_conversation(TEST_INSTALLATION_OWNER);
            c.name = format!("Conv {i}");
            c.updated_at = (i + 1) as i64 * 1000;
            repo.create(&c).await.unwrap();
            convs.push(c);
        }

        // Page 1: limit 2 → items[4,3], hasMore=true
        let page1 = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    limit: 2,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(page1.items.len(), 2);
        assert!(page1.has_more);
        assert_eq!(page1.items[0].name, "Conv 4");
        assert_eq!(page1.items[1].name, "Conv 3");

        // Page 2: cursor = last item of page 1
        let cursor = page1.items.last().unwrap().id.clone();
        let page2 = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    cursor: Some(cursor),
                    limit: 2,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(page2.items.len(), 2);
        assert!(page2.has_more);
        assert_eq!(page2.items[0].name, "Conv 2");
        assert_eq!(page2.items[1].name, "Conv 1");

        // Page 3: cursor = last item of page 2
        let cursor = page2.items.last().unwrap().id.clone();
        let page3 = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    cursor: Some(cursor),
                    limit: 2,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(page3.items.len(), 1);
        assert!(!page3.has_more);
        assert_eq!(page3.items[0].name, "Conv 0");
    }

    #[tokio::test]
    async fn list_filter_by_source() {
        let (repo, _db) = setup().await;

        let mut c1 = sample_conversation(TEST_INSTALLATION_OWNER);
        c1.source = Some("nomifun".to_string());
        repo.create(&c1).await.unwrap();

        let mut c2 = sample_conversation(TEST_INSTALLATION_OWNER);
        c2.source = Some("telegram".to_string());
        repo.create(&c2).await.unwrap();

        let result = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    source: Some("telegram".to_string()),
                    limit: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].source.as_deref(), Some("telegram"));
    }

    #[tokio::test]
    async fn list_filter_by_cron_job_id() {
        let (repo, _db) = setup().await;

        insert_cron_job(&repo.pool, "cron_abc").await;

        let mut c1 = sample_conversation(TEST_INSTALLATION_OWNER);
        c1.cron_job_id = Some("cron_abc".to_string());
        c1.id = repo.create(&c1).await.unwrap();

        let c2 = sample_conversation(TEST_INSTALLATION_OWNER);
        repo.create(&c2).await.unwrap();

        let result = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    cron_job_id: Some("cron_abc".to_string()),
                    limit: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].id, c1.id);
    }

    #[tokio::test]
    async fn list_filter_by_pinned() {
        let (repo, _db) = setup().await;

        let mut c1 = sample_conversation(TEST_INSTALLATION_OWNER);
        c1.pinned = true;
        c1.pinned_at = Some(nomifun_common::now_ms());
        repo.create(&c1).await.unwrap();

        let mut c2 = sample_conversation(TEST_INSTALLATION_OWNER);
        c2.pinned = false;
        repo.create(&c2).await.unwrap();

        let result = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    pinned: Some(true),
                    limit: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.total, 1);
        assert!(result.items[0].pinned);
    }

    #[tokio::test]
    async fn list_keeps_lead_and_excludes_attempt_conversation() {
        let (repo, db) = setup().await;

        let mut plain = sample_conversation(TEST_INSTALLATION_OWNER);
        plain.extra = r#"{"workspace":"/project"}"#.to_string();
        let plain_id = repo.create(&plain).await.unwrap();

        let lead = sample_conversation(TEST_INSTALLATION_OWNER);
        let lead_id = repo.create(&lead).await.unwrap();

        let attempt_id = repo
            .create(&sample_conversation(TEST_INSTALLATION_OWNER))
            .await
            .unwrap();
        link_lead_and_attempt_conversations(db.pool(), &lead_id, &attempt_id).await;

        let result = repo
            .list_paginated(
                TEST_INSTALLATION_OWNER,
                &ConversationFilters {
                    limit: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.total, 2);
        assert_eq!(result.items.len(), 2);
        assert!(
            result.items.iter().any(|c| c.id == plain_id),
            "plain conversation must remain visible"
        );
        assert!(
            result.items.iter().any(|c| c.id == lead_id),
            "lead conversation must remain visible"
        );
        assert!(
            result.items.iter().all(|c| c.id != attempt_id),
            "attempt conversation must be excluded"
        );
    }

    // ── Extended query tests ────────────────────────────────────────
    #[tokio::test]
    async fn find_by_source_and_chat() {
        let (repo, _db) = setup().await;

        let mut c = sample_conversation(TEST_INSTALLATION_OWNER);
        c.source = Some("telegram".to_string());
        c.channel_chat_id = Some("user:123".to_string());
        c.r#type = "gemini".to_string();
        c.id = repo.create(&c).await.unwrap();

        let found = repo
            .find_by_source_and_chat(TEST_INSTALLATION_OWNER, "telegram", "user:123", "gemini")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, c.id);

        // Different chat ID → not found
        let not_found = repo
            .find_by_source_and_chat(TEST_INSTALLATION_OWNER, "telegram", "user:999", "gemini")
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn list_by_cron_job() {
        let (repo, _db) = setup().await;

        insert_cron_job(&repo.pool, "cron_1").await;
        insert_cron_job(&repo.pool, "cron_2").await;

        let mut c1 = sample_conversation(TEST_INSTALLATION_OWNER);
        c1.cron_job_id = Some("cron_1".to_string());
        repo.create(&c1).await.unwrap();

        let mut c2 = sample_conversation(TEST_INSTALLATION_OWNER);
        c2.cron_job_id = Some("cron_1".to_string());
        repo.create(&c2).await.unwrap();

        let mut c3 = sample_conversation(TEST_INSTALLATION_OWNER);
        c3.cron_job_id = Some("cron_2".to_string());
        repo.create(&c3).await.unwrap();

        let result = repo.list_by_cron_job(TEST_INSTALLATION_OWNER, "cron_1").await.unwrap();
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn list_associated_by_workspace() {
        let (repo, _db) = setup().await;

        let mut c1 = sample_conversation(TEST_INSTALLATION_OWNER);
        c1.extra = r#"{"workspace":"/shared/project"}"#.to_string();
        c1.id = repo.create(&c1).await.unwrap();

        let mut c2 = sample_conversation(TEST_INSTALLATION_OWNER);
        c2.extra = r#"{"workspace":"/shared/project"}"#.to_string();
        c2.id = repo.create(&c2).await.unwrap();

        let mut c3 = sample_conversation(TEST_INSTALLATION_OWNER);
        c3.extra = r#"{"workspace":"/other/project"}"#.to_string();
        repo.create(&c3).await.unwrap();

        let associated = repo.list_associated(TEST_INSTALLATION_OWNER, &c1.id).await.unwrap();
        assert_eq!(associated.len(), 1);
        assert_eq!(associated[0].id, c2.id);
    }

    #[tokio::test]
    async fn list_associated_no_workspace() {
        let (repo, _db) = setup().await;

        let mut c = sample_conversation(TEST_INSTALLATION_OWNER);
        c.extra = r#"{}"#.to_string();
        c.id = repo.create(&c).await.unwrap();

        let associated = repo.list_associated(TEST_INSTALLATION_OWNER, &c.id).await.unwrap();
        assert!(associated.is_empty());
    }

    #[tokio::test]
    async fn list_associated_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.list_associated(TEST_INSTALLATION_OWNER, "conv_019abcdef012-7abc-8abc-0123-456789abcdee").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn cron_job_id_roundtrips_as_column() {
        let (repo, _db) = setup().await;

        insert_cron_job(&repo.pool, "cron_x").await;

        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.cron_job_id = Some("cron_x".to_string());
        conv.id = repo.create(&conv).await.unwrap();

        let found = repo.get(&conv.id).await.unwrap().unwrap();
        assert_eq!(found.cron_job_id.as_deref(), Some("cron_x"));

        // Clearing via update sets the column to NULL.
        repo.update(
            &conv.id,
            &ConversationRowUpdate {
                cron_job_id: Some(None),
                updated_at: Some(nomifun_common::now_ms()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cleared = repo.get(&conv.id).await.unwrap().unwrap();
        assert_eq!(cleared.cron_job_id, None);
    }

    // ── Artifact tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn cron_trigger_artifacts_insert_distinct_rows() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();
        insert_cron_job(&repo.pool, "cron_t").await;

        // cron_trigger has no unique constraint: each upsert is a fresh row with
        // a distinct auto-assigned i64 id.
        let a1 = repo
            .upsert_artifact(&sample_artifact(conv.id.clone(), "cron_trigger", Some("cron_t")))
            .await
            .unwrap();
        let a2 = repo
            .upsert_artifact(&sample_artifact(conv.id.clone(), "cron_trigger", Some("cron_t")))
            .await
            .unwrap();

        assert!(nomifun_common::ConversationArtifactId::parse(a1.id.clone()).is_ok());
        assert!(nomifun_common::ConversationArtifactId::parse(a2.id.clone()).is_ok());
        assert_ne!(a1.id, a2.id);

        let listed = repo.list_artifacts(&conv.id).await.unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[tokio::test]
    async fn skill_suggest_artifacts_upsert_is_idempotent() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();
        insert_cron_job(&repo.pool, "cron_s").await;

        let first = repo
            .upsert_artifact(&sample_artifact(conv.id.clone(), "skill_suggest", Some("cron_s")))
            .await
            .unwrap();

        // Second upsert for the same (conversation_id, cron_job_id) collides on the
        // partial UNIQUE index → updates in place, keeping the same id.
        let mut updated_input = sample_artifact(conv.id.clone(), "skill_suggest", Some("cron_s"));
        updated_input.payload = r#"{"v":2}"#.to_string();
        let second = repo.upsert_artifact(&updated_input).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.payload, r#"{"v":2}"#);

        let listed = repo.list_artifacts(&conv.id).await.unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn get_and_update_artifact_status_by_i64_id() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();
        insert_cron_job(&repo.pool, "cron_u").await;

        let inserted = repo
            .upsert_artifact(&sample_artifact(conv.id.clone(), "cron_trigger", Some("cron_u")))
            .await
            .unwrap();

        let fetched = repo
            .get_artifact(&conv.id, &inserted.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, inserted.id);

        let updated = repo
            .update_artifact_status(&conv.id, &inserted.id, "dismissed", nomifun_common::now_ms())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "dismissed");

        // Missing id → None.
        let missing = repo
            .update_artifact_status(&conv.id, "artifact_019abcdef012-7abc-8abc-0123-456789abcdee", "dismissed", nomifun_common::now_ms())
            .await
            .unwrap();
        assert!(missing.is_none());
    }

    // ── conversation_mcp_servers junction tests ─────────────────────

    #[tokio::test]
    async fn set_and_list_mcp_server_ids_preserves_order() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let a = insert_mcp_server(&repo.pool, "srv_a").await;
        let b = insert_mcp_server(&repo.pool, "srv_b").await;
        let c = insert_mcp_server(&repo.pool, "srv_c").await;

        // Empty by default.
        assert!(repo.list_mcp_server_ids(&conv.id).await.unwrap().is_empty());

        // Order is preserved via sort_order, not numeric id order.
        repo.set_mcp_server_ids(&conv.id, &[c.clone(), a.clone(), b.clone()])
            .await
            .unwrap();
        assert_eq!(
            repo.list_mcp_server_ids(&conv.id).await.unwrap(),
            vec![c.clone(), a.clone(), b.clone()]
        );

        // set replaces the whole set (DELETE + ordered INSERT).
        repo.set_mcp_server_ids(&conv.id, std::slice::from_ref(&b))
            .await
            .unwrap();
        assert_eq!(repo.list_mcp_server_ids(&conv.id).await.unwrap(), vec![b]);

        // Empty slice clears the selection.
        repo.set_mcp_server_ids(&conv.id, &[]).await.unwrap();
        assert!(repo.list_mcp_server_ids(&conv.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_conversation_cascades_mcp_junction() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();
        let a = insert_mcp_server(&repo.pool, "srv_cascade").await;

        repo.set_mcp_server_ids(&conv.id, std::slice::from_ref(&a))
            .await
            .unwrap();
        repo.delete(&conv.id).await.unwrap();

        // Junction rows are removed via ON DELETE CASCADE.
        let remaining = repo.list_mcp_server_ids(&conv.id).await.unwrap();
        assert!(remaining.is_empty());
    }

    // ── Message tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn insert_and_get_messages() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let msg = sample_message(conv.id.clone());
        repo.insert_message(&msg).await.unwrap();

        let result = repo.get_messages(&conv.id, 1, 50, SortOrder::Desc).await.unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].id, msg.id);
    }

    #[tokio::test]
    async fn get_messages_pagination() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        for i in 0..10 {
            let mut msg = sample_message(conv.id.clone());
            msg.id = nomifun_common::generate_prefixed_id("msg");
            msg.created_at = (i + 1) * 1000;
            repo.insert_message(&msg).await.unwrap();
        }

        let page1 = repo.get_messages(&conv.id, 1, 3, SortOrder::Desc).await.unwrap();
        assert_eq!(page1.items.len(), 3);
        assert_eq!(page1.total, 10);
        assert!(page1.has_more);
        // DESC: most recent first
        assert!(page1.items[0].created_at > page1.items[1].created_at);
    }

    #[tokio::test]
    async fn get_messages_asc_order() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        for i in 0..3 {
            let mut msg = sample_message(conv.id.clone());
            msg.id = nomifun_common::generate_prefixed_id("msg");
            msg.created_at = (i + 1) * 1000;
            repo.insert_message(&msg).await.unwrap();
        }

        let result = repo.get_messages(&conv.id, 1, 50, SortOrder::Asc).await.unwrap();
        assert!(result.items[0].created_at < result.items[1].created_at);
    }

    #[tokio::test]
    async fn update_message_content() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let msg = sample_message(conv.id.clone());
        repo.insert_message(&msg).await.unwrap();

        repo.update_message(
            &msg.id,
            &MessageRowUpdate {
                content: Some(r#"{"content":"Updated"}"#.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let result = repo.get_messages(&conv.id, 1, 50, SortOrder::Desc).await.unwrap();
        assert_eq!(result.items[0].content, r#"{"content":"Updated"}"#);
    }

    #[tokio::test]
    async fn update_message_not_found() {
        let (repo, _db) = setup().await;
        let err = repo
            .update_message(
                "no_id",
                &MessageRowUpdate {
                    hidden: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_messages_by_conversation() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        for _ in 0..3 {
            let mut msg = sample_message(conv.id.clone());
            msg.id = nomifun_common::generate_prefixed_id("msg");
            repo.insert_message(&msg).await.unwrap();
        }

        repo.delete_messages_by_conversation(&conv.id).await.unwrap();

        let result = repo.get_messages(&conv.id, 1, 50, SortOrder::Desc).await.unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.total, 0);
    }

    #[tokio::test]
    async fn get_message_by_msg_id() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let msg = sample_message(conv.id.clone());
        repo.insert_message(&msg).await.unwrap();

        let found = repo
            .get_message_by_msg_id(&conv.id, "client_msg_1", "text")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, msg.id);

        // Wrong type → not found
        let not_found = repo
            .get_message_by_msg_id(&conv.id, "client_msg_1", "tips")
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn search_messages_by_keyword() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let mut msg1 = sample_message(conv.id.clone());
        msg1.content = r#"{"content":"Rust 审查报告"}"#.to_string();
        repo.insert_message(&msg1).await.unwrap();

        let mut msg2 = sample_message(conv.id.clone());
        msg2.id = nomifun_common::generate_prefixed_id("msg");
        msg2.content = r#"{"content":"Python 测试"}"#.to_string();
        repo.insert_message(&msg2).await.unwrap();

        let result = repo.search_messages(TEST_INSTALLATION_OWNER, "审查", 1, 20).await.unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].conversation_name, "Test Conversation");
    }

    #[tokio::test]
    async fn search_messages_no_match() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let msg = sample_message(conv.id.clone());
        repo.insert_message(&msg).await.unwrap();

        let result = repo
            .search_messages(TEST_INSTALLATION_OWNER, "xxxxnotexist", 1, 20)
            .await
            .unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.total, 0);
    }

    #[tokio::test]
    async fn search_messages_pagination() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        for i in 0..5 {
            let mut msg = sample_message(conv.id.clone());
            msg.id = nomifun_common::generate_prefixed_id("msg");
            msg.content = format!(r#"{{"content":"match keyword item {i}"}}"#);
            msg.created_at = (i + 1) * 1000;
            repo.insert_message(&msg).await.unwrap();
        }

        let result = repo.search_messages(TEST_INSTALLATION_OWNER, "keyword", 1, 2).await.unwrap();
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.total, 5);
        assert!(result.has_more);
    }

    // ── Sort order tests ────────────────────────────────────────────

    #[test]
    fn sort_order_sql_representation() {
        assert_eq!(SortOrder::Asc.as_sql(), "ASC");
        assert_eq!(SortOrder::Desc.as_sql(), "DESC");
    }

    #[test]
    fn default_sort_order_is_asc() {
        assert_eq!(SortOrder::default(), SortOrder::Asc);
    }

    // ── Filters tests ───────────────────────────────────────────────

    #[test]
    fn effective_limit_default() {
        let f = ConversationFilters::default();
        assert_eq!(f.effective_limit(), 20);
    }

    #[test]
    fn effective_limit_custom() {
        let f = ConversationFilters {
            limit: 50,
            ..Default::default()
        };
        assert_eq!(f.effective_limit(), 50);
    }

    #[tokio::test]
    async fn delete_messages_from_removes_cursor_and_newer() {
        let (repo, _db) = setup().await;
        let mut conv = sample_conversation(TEST_INSTALLATION_OWNER);
        conv.id = repo.create(&conv).await.unwrap();

        let mk = |id: &str, created_at: i64| MessageRow {
            id: id.to_string(),
            conversation_id: conv.id.clone(),
            msg_id: Some(id.to_string()),
            r#type: "text".to_string(),
            content: r#"{"content":"x"}"#.to_string(),
            position: Some("right".to_string()),
            status: Some("finish".to_string()),
            hidden: false,
            created_at,
        };
        // 三条：t=100,200,300
        repo.insert_message(&mk("m1", 100)).await.unwrap();
        repo.insert_message(&mk("m2", 200)).await.unwrap();
        repo.insert_message(&mk("m3", 300)).await.unwrap();

        // 从 m2 (t=200) 起（含）删除 → 删 m2、m3，留 m1
        let deleted = repo.delete_messages_from(&conv.id, 200, "m2").await.unwrap();
        assert_eq!(deleted, 2);

        assert!(repo.get_message(&conv.id, "m1").await.unwrap().is_some());
        assert!(repo.get_message(&conv.id, "m2").await.unwrap().is_none());
        assert!(repo.get_message(&conv.id, "m3").await.unwrap().is_none());
    }
}
