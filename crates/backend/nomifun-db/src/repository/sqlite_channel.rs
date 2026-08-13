use nomifun_common::{
    ChannelPluginId, ChannelSessionId, ChannelUserId, CompanionId, ConversationId,
    MessageId, UserId,
};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::{
    ChannelInboundReceiptRow, ChannelPairingCodeRow, ChannelPendingPromptRow, ChannelPluginRow,
    ChannelSessionRow, ChannelUserRow, NewChannelInboundReceiptRow, NewChannelPairingCodeRow,
    NewChannelPendingPromptRow, NewChannelPluginRow, NewChannelSessionRow, NewChannelUserRow,
};
use crate::repository::channel::{
    ChannelInboundClaim, IChannelRepository, PairingApprovalOutcome, PendingPromptEnqueue,
    SettleChannelInboundReceiptParams, UpdatePluginStatusParams,
};

/// SQLite-backed implementation of [`IChannelRepository`].
#[derive(Clone, Debug)]
pub struct SqliteChannelRepository {
    pool: SqlitePool,
}

impl SqliteChannelRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

async fn lock_channel_plugin(
    tx: &mut Transaction<'_, Sqlite>,
    channel_plugin_id: Option<&str>,
    context: &str,
) -> Result<(), DbError> {
    let Some(channel_plugin_id) = channel_plugin_id else {
        return Ok(());
    };
    let channel_plugin_id = ChannelPluginId::parse(channel_plugin_id).map_err(|error| {
        DbError::Conflict(format!(
            "{context} channel plugin '{channel_plugin_id}' is not a canonical UUIDv7: {error}"
        ))
    })?;
    let parent = sqlx::query(
        "UPDATE channel_plugins SET updated_at = updated_at WHERE channel_plugin_id = ?",
    )
    .bind(channel_plugin_id.as_str())
    .execute(&mut **tx)
    .await?;
    if parent.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "{context} channel plugin '{channel_plugin_id}' does not exist"
        )));
    }
    Ok(())
}

async fn lock_conversation(
    tx: &mut Transaction<'_, Sqlite>,
    conversation_id: Option<&str>,
    context: &str,
) -> Result<Option<String>, DbError> {
    let Some(conversation_id) = conversation_id else {
        return Ok(None);
    };
    let conversation_id = ConversationId::parse(conversation_id).map_err(|error| {
        DbError::Conflict(format!(
            "{context} conversation '{conversation_id}' is not a canonical UUIDv7: {error}"
        ))
    })?;
    let parent = sqlx::query(
        "UPDATE conversations SET updated_at = updated_at WHERE conversation_id = ?",
    )
    .bind(conversation_id.as_str())
    .execute(&mut **tx)
    .await?;
    if parent.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "{context} conversation '{}' does not exist",
            conversation_id
        )));
    }
    Ok(Some(conversation_id.into_string()))
}

fn canonical_plugin_companion_id(
    companion_id: Option<&str>,
) -> Result<Option<String>, DbError> {
    companion_id
        .map(|value| {
            CompanionId::parse(value)
                .map(CompanionId::into_string)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "channel plugin companion_id '{value}' is not a canonical UUIDv7: {error}"
                    ))
                })
        })
        .transpose()
}

fn validate_owner_domain(owner_domain: &str) -> Result<(), DbError> {
    match owner_domain {
        crate::models::CHANNEL_OWNER_DOMAIN_COMPANION
        | crate::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE => Ok(()),
        _ => Err(DbError::Conflict(format!(
            "channel plugin owner_domain '{owner_domain}' is not supported"
        ))),
    }
}

fn validate_group_access_mode(group_access_mode: &str) -> Result<(), DbError> {
    match group_access_mode {
        crate::models::CHANNEL_GROUP_ACCESS_MODE_ALL_MEMBERS
        | crate::models::CHANNEL_GROUP_ACCESS_MODE_ALLOWLIST
        | crate::models::CHANNEL_GROUP_ACCESS_MODE_DISABLED => Ok(()),
        _ => Err(DbError::Conflict(format!(
            "channel plugin group_access_mode '{group_access_mode}' is not supported"
        ))),
    }
}

fn validate_authorization_kind(authorization_kind: &str) -> Result<(), DbError> {
    match authorization_kind {
        crate::models::CHANNEL_USER_AUTHORIZATION_APPROVED
        | crate::models::CHANNEL_USER_AUTHORIZATION_AUTO_GROUP => Ok(()),
        _ => Err(DbError::Conflict(format!(
            "channel user authorization_kind '{authorization_kind}' is not supported"
        ))),
    }
}

fn validate_chat_kind(chat_kind: &str) -> Result<(), DbError> {
    match chat_kind {
        crate::models::CHANNEL_CHAT_KIND_UNKNOWN
        | crate::models::CHANNEL_CHAT_KIND_DIRECT
        | crate::models::CHANNEL_CHAT_KIND_GROUP => Ok(()),
        _ => Err(DbError::Conflict(format!(
            "channel session chat_kind '{chat_kind}' is not supported"
        ))),
    }
}

fn merge_chat_kind(existing: &str, incoming: &str) -> Result<String, DbError> {
    validate_chat_kind(existing)?;
    validate_chat_kind(incoming)?;
    match (existing, incoming) {
        (crate::models::CHANNEL_CHAT_KIND_UNKNOWN, value) => Ok(value.to_owned()),
        (value, crate::models::CHANNEL_CHAT_KIND_UNKNOWN) => Ok(value.to_owned()),
        (left, right) if left == right => Ok(left.to_owned()),
        (left, right) => Err(DbError::Conflict(format!(
            "channel session chat_kind cannot change from '{left}' to '{right}'"
        ))),
    }
}

fn validate_agent_type(agent_type: &str, context: &str) -> Result<(), DbError> {
    match agent_type {
        "acp" | "openclaw-gateway" | "nanobot" | "remote" | "nomi" => Ok(()),
        _ => Err(DbError::Conflict(format!(
            "{context} agent type '{agent_type}' is not supported"
        ))),
    }
}

#[async_trait::async_trait]
impl IChannelRepository for SqliteChannelRepository {
    // -- Plugin CRUD --------------------------------------------------

    async fn get_all_plugins(&self) -> Result<Vec<ChannelPluginRow>, DbError> {
        let rows = sqlx::query_as::<_, ChannelPluginRow>("SELECT * FROM channel_plugins ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    async fn list_plugins_by_owner_domain(
        &self,
        owner_domain: &str,
    ) -> Result<Vec<ChannelPluginRow>, DbError> {
        validate_owner_domain(owner_domain)?;
        let rows = sqlx::query_as::<_, ChannelPluginRow>(
            "SELECT * FROM channel_plugins WHERE owner_domain = ? ORDER BY created_at ASC",
        )
        .bind(owner_domain)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get_plugin(
        &self,
        channel_plugin_id: &str,
    ) -> Result<Option<ChannelPluginRow>, DbError> {
        let row = sqlx::query_as::<_, ChannelPluginRow>(
            "SELECT * FROM channel_plugins WHERE channel_plugin_id = ?",
        )
        .bind(channel_plugin_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create_plugin(&self, row: &NewChannelPluginRow) -> Result<ChannelPluginRow, DbError> {
        let channel_plugin_id = ChannelPluginId::new().into_string();
        let companion_id = canonical_plugin_companion_id(row.companion_id.as_deref())?;
        validate_owner_domain(&row.owner_domain)?;
        validate_group_access_mode(&row.group_access_mode)?;
        sqlx::query_as::<_, ChannelPluginRow>(
            "INSERT INTO channel_plugins \
                (channel_plugin_id, type, name, enabled, config, status, last_connected, \
                 companion_id, bot_key, owner_domain, group_access_mode, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING *",
        )
        .bind(channel_plugin_id)
        .bind(&row.r#type)
        .bind(&row.name)
        .bind(row.enabled)
        .bind(&row.config)
        .bind(&row.status)
        .bind(row.last_connected)
        .bind(&companion_id)
        .bind(&row.bot_key)
        .bind(&row.owner_domain)
        .bind(&row.group_access_mode)
        .bind(row.created_at)
        .bind(row.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                DbError::Conflict(format!(
                    "Bot '{}' on platform '{}' is already configured",
                    row.bot_key.as_deref().unwrap_or("?"),
                    row.r#type
                ))
            } else if is_owner_domain_violation(&e) {
                owner_domain_conflict()
            } else {
                DbError::Query(e)
            }
        })
    }

    async fn update_plugin(&self, row: &ChannelPluginRow) -> Result<ChannelPluginRow, DbError> {
        ChannelPluginId::parse(&row.channel_plugin_id).map_err(|error| {
            DbError::Conflict(format!(
                "channel plugin id '{}' is not a canonical UUIDv7: {error}",
                row.channel_plugin_id
            ))
        })?;
        let companion_id = canonical_plugin_companion_id(row.companion_id.as_deref())?;
        validate_owner_domain(&row.owner_domain)?;
        let updated = sqlx::query_as::<_, ChannelPluginRow>(
            "UPDATE channel_plugins SET \
                type = ?, name = ?, enabled = ?, config = ?, status = ?, \
                last_connected = ?, companion_id = ?, \
                bot_key = ?, owner_domain = ?, updated_at = ? \
             WHERE channel_plugin_id = ? \
             RETURNING *",
        )
        .bind(&row.r#type)
        .bind(&row.name)
        .bind(row.enabled)
        .bind(&row.config)
        .bind(&row.status)
        .bind(row.last_connected)
        .bind(&companion_id)
        .bind(&row.bot_key)
        .bind(&row.owner_domain)
        .bind(row.updated_at)
        .bind(&row.channel_plugin_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                DbError::Conflict(format!(
                    "Bot '{}' on platform '{}' is already configured",
                    row.bot_key.as_deref().unwrap_or("?"),
                    row.r#type
                ))
            } else if is_owner_domain_violation(&e) {
                owner_domain_conflict()
            } else {
                DbError::Query(e)
            }
        })?;
        updated.ok_or_else(|| {
            DbError::NotFound(format!(
                "Plugin '{}' not found",
                row.channel_plugin_id
            ))
        })
    }

    async fn update_plugin_group_access_mode_and_clear_non_direct_sessions(
        &self,
        channel_plugin_id: &str,
        group_access_mode: &str,
    ) -> Result<(), DbError> {
        validate_group_access_mode(group_access_mode)?;
        let mut tx = self.pool.begin().await?;
        let now = nomifun_common::now_ms();

        let updated = sqlx::query(
            "UPDATE channel_plugins \
             SET group_access_mode = ?, updated_at = ? \
             WHERE channel_plugin_id = ?",
        )
        .bind(group_access_mode)
        .bind(now)
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "Plugin '{channel_plugin_id}' not found"
            )));
        }

        sqlx::query(
            "UPDATE channel_pending_prompts AS prompt \
             SET state = 'cancelled', settled_at = ? \
             WHERE prompt.channel_plugin_id = ? AND prompt.state = 'queued' \
               AND EXISTS ( \
                   SELECT 1 FROM channel_sessions AS session \
                   WHERE session.channel_session_id = prompt.channel_session_id \
                     AND session.channel_plugin_id = ? \
                     AND session.chat_kind IN ('group', 'unknown') \
               )",
        )
        .bind(now)
        .bind(channel_plugin_id)
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM channel_session_bindings \
             WHERE channel_plugin_id = ? \
               AND channel_session_id IN ( \
                   SELECT channel_session_id FROM channel_sessions \
                   WHERE channel_plugin_id = ? \
                     AND chat_kind IN ('group', 'unknown') \
               )",
        )
        .bind(channel_plugin_id)
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM channel_sessions \
             WHERE channel_plugin_id = ? \
               AND chat_kind IN ('group', 'unknown')",
        )
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn update_plugin_status(
        &self,
        channel_plugin_id: &str,
        params: &UpdatePluginStatusParams,
    ) -> Result<(), DbError> {
        let mut set_clauses = Vec::new();
        if params.status.is_some() {
            set_clauses.push("status = ?");
        }
        if params.last_connected.is_some() {
            set_clauses.push("last_connected = ?");
        }
        if params.enabled.is_some() {
            set_clauses.push("enabled = ?");
        }

        if set_clauses.is_empty() {
            return Ok(());
        }

        set_clauses.push("updated_at = ?");
        let sql = format!(
            "UPDATE channel_plugins SET {} WHERE channel_plugin_id = ?",
            set_clauses.join(", ")
        );

        let now = nomifun_common::now_ms();
        let mut query = sqlx::query(&sql);

        if let Some(ref status) = params.status {
            query = query.bind(status);
        }
        if let Some(last_connected) = params.last_connected {
            query = query.bind(last_connected);
        }
        if let Some(enabled) = params.enabled {
            query = query.bind(enabled);
        }
        query = query.bind(now);
        query = query.bind(channel_plugin_id);

        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "Plugin '{channel_plugin_id}' not found"
            )));
        }
        Ok(())
    }

    async fn update_plugin_companion(
        &self,
        channel_plugin_id: &str,
        companion_id: Option<&str>,
    ) -> Result<(), DbError> {
        let companion_id = canonical_plugin_companion_id(companion_id)?;
        let result = sqlx::query(
            "UPDATE channel_plugins \
             SET companion_id = ?, \
                 updated_at = ? \
             WHERE channel_plugin_id = ?",
        )
        .bind(companion_id.as_deref())
        .bind(nomifun_common::now_ms())
        .bind(channel_plugin_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if is_owner_domain_violation(&e) {
                owner_domain_conflict()
            } else {
                DbError::Query(e)
            }
        })?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "Plugin '{channel_plugin_id}' not found"
            )));
        }
        Ok(())
    }

    async fn update_plugin_bot_key(
        &self,
        channel_plugin_id: &str,
        bot_key: &str,
    ) -> Result<(), DbError> {
        let bot_key = bot_key.trim();
        if bot_key.is_empty() {
            return Err(DbError::Conflict(
                "channel plugin bot_key must not be empty".to_owned(),
            ));
        }
        let result = sqlx::query(
            "UPDATE channel_plugins \
             SET bot_key = ?, updated_at = ? \
             WHERE channel_plugin_id = ?",
        )
        .bind(bot_key)
        .bind(nomifun_common::now_ms())
        .bind(channel_plugin_id)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            if is_unique_violation(&error) {
                DbError::Conflict(format!(
                    "Bot '{bot_key}' is already configured for this platform"
                ))
            } else {
                DbError::Query(error)
            }
        })?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "Plugin '{channel_plugin_id}' not found"
            )));
        }
        Ok(())
    }

    async fn delete_plugin(&self, channel_plugin_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE channel_plugins \
             SET updated_at = updated_at \
             WHERE channel_plugin_id = ?",
        )
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "Plugin '{channel_plugin_id}' not found"
            )));
        }

        // Receipts are retained as replay authority after a plugin is removed.
        // Clear only the nullable logical projection; the immutable scope id
        // remains part of the operation identity.
        sqlx::query(
            "UPDATE channel_inbound_receipts \
             SET channel_plugin_id = NULL WHERE channel_plugin_id = ?",
        )
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        // Deleting plugin-owned users cascades their authoritative sessions in
        // the same transaction.
        sqlx::query("DELETE FROM channel_session_bindings WHERE channel_plugin_id = ?")
            .bind(channel_plugin_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM channel_sessions \
             WHERE channel_user_id IN (\
                 SELECT channel_user_id FROM channel_users WHERE channel_plugin_id = ?\
             )",
        )
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM channel_users WHERE channel_plugin_id = ?")
            .bind(channel_plugin_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM channel_pairing_codes WHERE channel_plugin_id = ?")
            .bind(channel_plugin_id)
            .execute(&mut *tx)
            .await?;

        // Deleting a bot releases its customer-service binding (Cascade):
        // the binding is domain state of the customer-service crate, but the
        // channel aggregate owns the plugin row lifecycle.
        sqlx::query("DELETE FROM cs_channel_bindings WHERE channel_plugin_id = ?")
            .bind(channel_plugin_id)
            .execute(&mut *tx)
            .await?;

        // Sessions not owned by a cascaded user retain their history but no
        // longer point at the removed plugin.
        sqlx::query(
            "UPDATE channel_sessions \
             SET channel_plugin_id = NULL \
             WHERE channel_plugin_id = ?",
        )
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM channel_plugins WHERE channel_plugin_id = ?")
            .bind(channel_plugin_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // -- User CRUD ----------------------------------------------------

    async fn get_all_users(&self) -> Result<Vec<ChannelUserRow>, DbError> {
        let rows = sqlx::query_as::<_, ChannelUserRow>(
            "SELECT * FROM channel_users \
             WHERE authorization_kind = 'approved' ORDER BY authorized_at DESC",
        )
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    async fn get_user(
        &self,
        channel_user_id: &str,
    ) -> Result<Option<ChannelUserRow>, DbError> {
        let row = sqlx::query_as::<_, ChannelUserRow>(
            "SELECT * FROM channel_users WHERE channel_user_id = ?",
        )
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn get_user_by_platform(
        &self,
        platform_user_id: &str,
        platform_type: &str,
        channel_plugin_id: &str,
    ) -> Result<Option<ChannelUserRow>, DbError> {
        let row = sqlx::query_as::<_, ChannelUserRow>(
            "SELECT * FROM channel_users \
             WHERE platform_user_id = ? AND platform_type = ? AND channel_plugin_id = ?",
        )
        .bind(platform_user_id)
        .bind(platform_type)
        .bind(channel_plugin_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create_user(&self, row: &NewChannelUserRow) -> Result<ChannelUserRow, DbError> {
        validate_authorization_kind(&row.authorization_kind)?;
        if row.authorization_kind != crate::models::CHANNEL_USER_AUTHORIZATION_APPROVED {
            return Err(DbError::Conflict(
                "create_user requires authorization_kind 'approved'; use ensure_auto_group_user for group-learned identities"
                    .to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        lock_channel_plugin(
            &mut tx,
            row.channel_plugin_id.as_deref(),
            "channel user",
        )
        .await?;
        let channel_user_id = ChannelUserId::new().into_string();
        let inserted = sqlx::query_as::<_, ChannelUserRow>(
            "INSERT INTO channel_users \
                (channel_user_id, platform_user_id, platform_type, channel_plugin_id, \
                 display_name, authorization_kind, authorized_at, last_active) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(platform_user_id, platform_type, channel_plugin_id) DO UPDATE SET \
                 display_name = COALESCE(excluded.display_name, channel_users.display_name), \
                 authorization_kind = 'approved', \
                 authorized_at = excluded.authorized_at, \
                 last_active = COALESCE(excluded.last_active, channel_users.last_active) \
             WHERE channel_users.authorization_kind = 'auto_group' \
               AND excluded.authorization_kind = 'approved' \
             RETURNING *",
        )
        .bind(channel_user_id)
        .bind(&row.platform_user_id)
        .bind(&row.platform_type)
        .bind(&row.channel_plugin_id)
        .bind(&row.display_name)
        .bind(&row.authorization_kind)
        .bind(row.authorized_at)
        .bind(row.last_active)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                DbError::Conflict(format!(
                    "User '{}' on platform '{}' already exists",
                    row.platform_user_id, row.platform_type
                ))
            } else {
                DbError::Query(e)
            }
        })?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "User '{}' on platform '{}' already exists",
                row.platform_user_id, row.platform_type
            ))
        })?;
        tx.commit().await?;
        Ok(inserted)
    }

    async fn ensure_auto_group_user(
        &self,
        row: &NewChannelUserRow,
    ) -> Result<ChannelUserRow, DbError> {
        validate_authorization_kind(&row.authorization_kind)?;
        if row.authorization_kind != crate::models::CHANNEL_USER_AUTHORIZATION_AUTO_GROUP {
            return Err(DbError::Conflict(
                "ensure_auto_group_user requires authorization_kind 'auto_group'".to_owned(),
            ));
        }
        let channel_plugin_id = row.channel_plugin_id.as_deref().ok_or_else(|| {
            DbError::Conflict("auto-group channel users must be scoped to a plugin".to_owned())
        })?;
        let mut tx = self.pool.begin().await?;
        lock_channel_plugin(&mut tx, Some(channel_plugin_id), "auto-group channel user").await?;
        let channel_user_id = ChannelUserId::new().into_string();
        let user = sqlx::query_as::<_, ChannelUserRow>(
            "INSERT INTO channel_users \
                (channel_user_id, platform_user_id, platform_type, channel_plugin_id, \
                 display_name, authorization_kind, authorized_at, last_active) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(platform_user_id, platform_type, channel_plugin_id) DO UPDATE SET \
                 display_name = COALESCE(channel_users.display_name, excluded.display_name), \
                 last_active = CASE \
                     WHEN excluded.last_active IS NULL THEN channel_users.last_active \
                     WHEN channel_users.last_active IS NULL \
                          OR excluded.last_active > channel_users.last_active \
                     THEN excluded.last_active \
                     ELSE channel_users.last_active \
                 END \
             RETURNING *",
        )
        .bind(channel_user_id)
        .bind(&row.platform_user_id)
        .bind(&row.platform_type)
        .bind(&row.channel_plugin_id)
        .bind(&row.display_name)
        .bind(&row.authorization_kind)
        .bind(row.authorized_at)
        .bind(row.last_active)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(user)
    }

    async fn update_user_last_active(
        &self,
        channel_user_id: &str,
        last_active: nomifun_common::TimestampMs,
    ) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE channel_users SET last_active = ? WHERE channel_user_id = ?",
        )
            .bind(last_active)
            .bind(channel_user_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "User '{channel_user_id}' not found"
            )));
        }
        Ok(())
    }

    async fn delete_user(&self, channel_user_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE channel_users \
             SET display_name = display_name \
             WHERE channel_user_id = ?",
        )
        .bind(channel_user_id)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "User '{channel_user_id}' not found"
            )));
        }

        sqlx::query("DELETE FROM channel_session_bindings WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM channel_sessions WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM channel_users WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn revoke_user_and_cancel_pending(
        &self,
        channel_user_id: &str,
        now: nomifun_common::TimestampMs,
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE channel_users \
             SET display_name = display_name \
             WHERE channel_user_id = ?",
        )
        .bind(channel_user_id)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "User '{channel_user_id}' not found"
            )));
        }

        // A queued turn carries the authority of the user who originally
        // admitted it. Revocation therefore cancels all of that user's queued
        // turns, including Direct; otherwise a later drain could execute an
        // already-revoked host-capability prompt.
        sqlx::query(
            "UPDATE channel_pending_prompts AS prompt \
             SET state = 'cancelled', settled_at = ? \
             WHERE prompt.state = 'queued' \
               AND EXISTS ( \
                   SELECT 1 FROM channel_sessions AS session \
                   WHERE session.channel_session_id = prompt.channel_session_id \
                     AND session.channel_user_id = ? \
               )",
        )
        .bind(now)
        .bind(channel_user_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM channel_session_bindings WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM channel_sessions WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM channel_users WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // -- Session CRUD -----------------------------------------------------

    async fn get_all_sessions(&self) -> Result<Vec<ChannelSessionRow>, DbError> {
        let rows =
            sqlx::query_as::<_, ChannelSessionRow>("SELECT * FROM channel_sessions ORDER BY last_activity DESC")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    async fn get_session(&self, channel_session_id: &str) -> Result<Option<ChannelSessionRow>, DbError> {
        let row = sqlx::query_as::<_, ChannelSessionRow>(
            "SELECT * FROM channel_sessions WHERE channel_session_id = ?",
        )
            .bind(channel_session_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn get_or_create_session(
        &self,
        channel_user_id: &str,
        chat_id: &str,
        channel_plugin_id: &str,
        new_row: &NewChannelSessionRow,
    ) -> Result<ChannelSessionRow, DbError> {
        validate_agent_type(&new_row.agent_type, "channel session")?;
        validate_chat_kind(&new_row.chat_kind)?;
        if new_row.channel_user_id != channel_user_id
            || new_row.chat_id.as_deref() != Some(chat_id)
            || new_row.channel_plugin_id.as_deref() != Some(channel_plugin_id)
        {
            return Err(DbError::Conflict(
                "channel session lookup keys must match the inserted row".into(),
            ));
        }
        if chat_id.is_empty() || chat_id.len() > 512 {
            return Err(DbError::Conflict(
                "channel session chat_id must contain between 1 and 512 bytes".into(),
            ));
        }
        let session_id = ChannelSessionId::parse(&new_row.channel_session_id).map_err(|error| {
            DbError::Conflict(format!(
                "channel session id '{}' is not a canonical UUIDv7: {error}",
                new_row.channel_session_id
            ))
        })?;
        let mut tx = self.pool.begin().await?;
        lock_channel_plugin(&mut tx, Some(channel_plugin_id), "channel session").await?;
        let channel_user_id = ChannelUserId::parse(channel_user_id).map_err(|error| {
            DbError::Conflict(format!(
                "channel session user '{channel_user_id}' is not a canonical UUIDv7: {error}"
            ))
        })?;
        let user_plugin_id: Option<String> = sqlx::query_scalar(
            "UPDATE channel_users SET last_active = last_active WHERE channel_user_id = ? \
             RETURNING channel_plugin_id",
        )
        .bind(channel_user_id.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "channel session user '{channel_user_id}' does not exist"
            ))
        })?;
        if user_plugin_id.as_deref().is_some()
            && user_plugin_id.as_deref() != Some(channel_plugin_id)
        {
            return Err(DbError::Conflict(
                "channel session plugin does not match its channel user".into(),
            ));
        }
        let conversation_id = lock_conversation(
            &mut tx,
            new_row.conversation_id.as_deref(),
            "channel session",
        )
        .await?;

        // The binding is the durable authority for a chat scope. Migration 003
        // deterministically backfills the earliest legacy session when old
        // databases contain duplicate rows, without deleting any history.
        let bound_session_id: Option<String> = sqlx::query_scalar(
            "SELECT channel_session_id FROM channel_session_bindings \
             WHERE channel_plugin_id = ? AND channel_user_id = ? AND chat_id = ?",
        )
        .bind(channel_plugin_id)
        .bind(channel_user_id.as_str())
        .bind(chat_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(bound_session_id) = bound_session_id {
            let row = sqlx::query_as::<_, ChannelSessionRow>(
                "SELECT * FROM channel_sessions WHERE channel_session_id = ?",
            )
            .bind(&bound_session_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::Conflict(format!(
                    "channel session binding points to missing session '{bound_session_id}'"
                ))
            })?;
            // Touch last_activity.
            let now = nomifun_common::now_ms();
            let chat_kind = merge_chat_kind(&row.chat_kind, &new_row.chat_kind)?;
            // Legacy sessions could point at a private/shared conversation
            // before chat scope was persisted. Reclassification must sever
            // that link atomically so a group cannot inherit private context.
            let conversation_id = if row.chat_kind == crate::models::CHANNEL_CHAT_KIND_UNKNOWN
                && chat_kind != crate::models::CHANNEL_CHAT_KIND_UNKNOWN
            {
                None
            } else {
                row.conversation_id.clone()
            };
            sqlx::query(
                "UPDATE channel_sessions \
                 SET last_activity = ?, chat_kind = ?, conversation_id = ? \
                 WHERE channel_session_id = ?",
            )
                .bind(now)
                .bind(&chat_kind)
                .bind(&conversation_id)
                .bind(&row.channel_session_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(ChannelSessionRow {
                last_activity: now,
                chat_kind,
                conversation_id,
                ..row
            });
        }

        // Defensive recovery for a database whose binding was removed outside
        // the repository: bind the earliest matching legacy row instead of
        // authorizing a second session for the same scope.
        let legacy = sqlx::query_as::<_, ChannelSessionRow>(
            "SELECT * FROM channel_sessions \
             WHERE channel_user_id = ? AND chat_id = ? AND channel_plugin_id = ? \
             ORDER BY id ASC LIMIT 1",
        )
        .bind(channel_user_id.as_str())
        .bind(chat_id)
        .bind(channel_plugin_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = legacy {
            sqlx::query(
                "INSERT INTO channel_session_bindings \
                    (channel_plugin_id, channel_user_id, chat_id, channel_session_id, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(channel_plugin_id)
            .bind(channel_user_id.as_str())
            .bind(chat_id)
            .bind(&row.channel_session_id)
            .bind(row.created_at)
            .execute(&mut *tx)
            .await?;
            let now = nomifun_common::now_ms();
            let chat_kind = merge_chat_kind(&row.chat_kind, &new_row.chat_kind)?;
            let conversation_id = if row.chat_kind == crate::models::CHANNEL_CHAT_KIND_UNKNOWN
                && chat_kind != crate::models::CHANNEL_CHAT_KIND_UNKNOWN
            {
                None
            } else {
                row.conversation_id.clone()
            };
            sqlx::query(
                "UPDATE channel_sessions \
                 SET last_activity = ?, chat_kind = ?, conversation_id = ? \
                 WHERE channel_session_id = ?",
            )
            .bind(now)
            .bind(&chat_kind)
            .bind(&conversation_id)
            .bind(&row.channel_session_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(ChannelSessionRow {
                last_activity: now,
                chat_kind,
                conversation_id,
                ..row
            });
        }

        // Insert new session.
        let inserted = sqlx::query_as::<_, ChannelSessionRow>(
            "INSERT INTO channel_sessions \
                (channel_session_id, channel_user_id, agent_type, conversation_id, workspace, \
                 chat_id, channel_plugin_id, chat_kind, created_at, last_activity) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING *",
        )
        .bind(session_id.as_str())
        .bind(&new_row.channel_user_id)
        .bind(&new_row.agent_type)
        .bind(&conversation_id)
        .bind(&new_row.workspace)
        .bind(&new_row.chat_id)
        .bind(&new_row.channel_plugin_id)
        .bind(&new_row.chat_kind)
        .bind(new_row.created_at)
        .bind(new_row.last_activity)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO channel_session_bindings \
                (channel_plugin_id, channel_user_id, chat_id, channel_session_id, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(channel_plugin_id)
        .bind(channel_user_id.as_str())
        .bind(chat_id)
        .bind(&inserted.channel_session_id)
        .bind(inserted.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(inserted)
    }

    async fn update_session_activity(
        &self,
        channel_session_id: &str,
        last_activity: nomifun_common::TimestampMs,
    ) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE channel_sessions SET last_activity = ? WHERE channel_session_id = ?",
        )
            .bind(last_activity)
            .bind(channel_session_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Session '{channel_session_id}' not found")));
        }
        Ok(())
    }

    async fn update_session_conversation(&self, channel_session_id: &str, conversation_id: &str) -> Result<(), DbError> {
        let now = nomifun_common::now_ms();
        let mut tx = self.pool.begin().await?;
        let conversation_id =
            lock_conversation(&mut tx, Some(conversation_id), "channel session update")
                .await?
                .expect("Some input returns Some");
        let result = sqlx::query(
            "UPDATE channel_sessions \
             SET conversation_id = ?, last_activity = ? \
             WHERE channel_session_id = ?",
        )
        .bind(conversation_id)
        .bind(now)
        .bind(channel_session_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Session '{channel_session_id}' not found")));
        }
        tx.commit().await?;
        Ok(())
    }

    async fn update_session_agent_type(&self, channel_session_id: &str, agent_type: &str) -> Result<(), DbError> {
        validate_agent_type(agent_type, "channel session update")?;
        let now = nomifun_common::now_ms();
        let result = sqlx::query(
            "UPDATE channel_sessions \
             SET agent_type = ?, last_activity = ? \
             WHERE channel_session_id = ?",
        )
        .bind(agent_type)
        .bind(now)
        .bind(channel_session_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Session '{channel_session_id}' not found")));
        }
        Ok(())
    }

    async fn delete_sessions_by_user(&self, channel_user_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM channel_session_bindings WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM channel_sessions WHERE channel_user_id = ?")
            .bind(channel_user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn delete_sessions_by_channel(
        &self,
        channel_plugin_id: &str,
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE channel_pending_prompts \
             SET state = 'cancelled', settled_at = ? \
             WHERE channel_plugin_id = ? AND state = 'queued'",
        )
        .bind(nomifun_common::now_ms())
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM channel_session_bindings WHERE channel_plugin_id = ?")
            .bind(channel_plugin_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM channel_sessions WHERE channel_plugin_id = ?")
            .bind(channel_plugin_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn delete_group_sessions_by_channel(
        &self,
        channel_plugin_id: &str,
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let now = nomifun_common::now_ms();

        sqlx::query(
            "UPDATE channel_pending_prompts AS prompt \
             SET state = 'cancelled', settled_at = ? \
             WHERE prompt.channel_plugin_id = ? AND prompt.state = 'queued' \
               AND EXISTS ( \
                   SELECT 1 FROM channel_sessions AS session \
                   WHERE session.channel_session_id = prompt.channel_session_id \
                     AND session.channel_plugin_id = ? \
                     AND session.chat_kind = 'group' \
               )",
        )
        .bind(now)
        .bind(channel_plugin_id)
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM channel_session_bindings \
             WHERE channel_plugin_id = ? \
               AND channel_session_id IN ( \
                   SELECT channel_session_id FROM channel_sessions \
                   WHERE channel_plugin_id = ? AND chat_kind = 'group' \
               )",
        )
        .bind(channel_plugin_id)
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM channel_sessions \
             WHERE channel_plugin_id = ? AND chat_kind = 'group'",
        )
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn delete_session_by_user_chat(
        &self,
        channel_user_id: &str,
        chat_id: &str,
        channel_plugin_id: &str,
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM channel_session_bindings \
             WHERE channel_user_id = ? AND chat_id = ? AND channel_plugin_id = ?",
        )
        .bind(channel_user_id)
        .bind(chat_id)
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM channel_sessions \
             WHERE channel_user_id = ? AND chat_id = ? AND channel_plugin_id = ?",
        )
        .bind(channel_user_id)
        .bind(chat_id)
        .bind(channel_plugin_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    // -- Durable inbound admission ---------------------------------------

    async fn claim_inbound_receipt(
        &self,
        row: &NewChannelInboundReceiptRow,
    ) -> Result<ChannelInboundClaim, DbError> {
        UserId::parse(&row.user_id).map_err(|error| {
            DbError::Conflict(format!(
                "channel inbound user_id '{}' is not a canonical UUIDv7: {error}",
                row.user_id
            ))
        })?;
        ChannelPluginId::parse(&row.channel_plugin_id).map_err(|error| {
            DbError::Conflict(format!(
                "channel inbound plugin_id '{}' is not a canonical UUIDv7: {error}",
                row.channel_plugin_id
            ))
        })?;
        if row.operation_key.len() != 83
            || !row.operation_key.starts_with("channel-inbound:v1:")
            || !row.operation_key[19..].bytes().all(|byte| byte.is_ascii_hexdigit())
            || row.operation_key[19..]
                .bytes()
                .any(|byte| byte.is_ascii_uppercase())
        {
            return Err(DbError::Conflict(
                "channel inbound operation key must be channel-inbound:v1 plus 64 lowercase hex characters"
                    .to_owned(),
            ));
        }
        if row.payload_hash.len() != 64
            || !row.payload_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || row.payload_hash.bytes().any(|byte| byte.is_ascii_uppercase())
        {
            return Err(DbError::Conflict(
                "channel inbound payload hash must contain 64 lowercase hex characters".to_owned(),
            ));
        }
        if row.platform.is_empty()
            || row.platform.len() > 64
            || row.chat_id.is_empty()
            || row.chat_id.len() > 512
            || row.provider_event_id.is_empty()
            || row.provider_event_id.len() > 512
        {
            return Err(DbError::Conflict(
                "channel inbound platform/chat/event identity is empty or exceeds its bound"
                    .to_owned(),
            ));
        }
        let inserted = sqlx::query_as::<_, ChannelInboundReceiptRow>(
            "INSERT INTO channel_inbound_receipts \
                (operation_key, user_scope_id, user_id, channel_plugin_scope_id, \
                 channel_plugin_id, platform, chat_id, provider_event_id, payload_hash, \
                 status, phase, owner_generation, created_at, updated_at) \
             SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, 'accepted', 'claimed', 1, ?, ? \
             WHERE EXISTS(SELECT 1 FROM users WHERE user_id = ?) \
               AND EXISTS(SELECT 1 FROM channel_plugins WHERE channel_plugin_id = ?) \
             ON CONFLICT(operation_key) DO NOTHING \
             RETURNING *",
        )
        .bind(&row.operation_key)
        .bind(&row.user_id)
        .bind(&row.user_id)
        .bind(&row.channel_plugin_id)
        .bind(&row.channel_plugin_id)
        .bind(&row.platform)
        .bind(&row.chat_id)
        .bind(&row.provider_event_id)
        .bind(&row.payload_hash)
        .bind(row.created_at)
        .bind(row.created_at)
        .bind(&row.user_id)
        .bind(&row.channel_plugin_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(receipt) = inserted {
            return Ok(ChannelInboundClaim::Owner(receipt));
        }

        let existing = sqlx::query_as::<_, ChannelInboundReceiptRow>(
            "SELECT * FROM channel_inbound_receipts WHERE operation_key = ?",
        )
        .bind(&row.operation_key)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::Conflict(
                "channel inbound owner or plugin no longer exists; refusing admission".to_owned(),
            )
        })?;
        if existing.user_scope_id != row.user_id
            || existing.channel_plugin_scope_id != row.channel_plugin_id
            || existing.platform != row.platform
            || existing.chat_id != row.chat_id
            || existing.provider_event_id != row.provider_event_id
            || existing.payload_hash != row.payload_hash
        {
            return Err(DbError::Conflict(
                "channel inbound operation identity was reused with a different payload or scope"
                    .to_owned(),
            ));
        }

        // Fail closed forever. A claimed row may represent a process that is
        // merely suspended, so wall-clock age is never execution authority.
        Ok(ChannelInboundClaim::Replay(existing))
    }

    async fn begin_inbound_effects(
        &self,
        operation_key: &str,
        payload_hash: &str,
        owner_generation: i64,
        now: nomifun_common::TimestampMs,
    ) -> Result<bool, DbError> {
        let changed = sqlx::query(
            "UPDATE channel_inbound_receipts \
             SET phase = 'effects_started', updated_at = ? \
             WHERE operation_key = ? AND payload_hash = ? \
               AND status = 'accepted' AND phase = 'claimed' \
               AND owner_generation = ?",
        )
        .bind(now)
        .bind(operation_key)
        .bind(payload_hash)
        .bind(owner_generation)
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() == 1 {
            return Ok(true);
        }

        let existing = sqlx::query_as::<_, ChannelInboundReceiptRow>(
            "SELECT * FROM channel_inbound_receipts WHERE operation_key = ?",
        )
        .bind(operation_key)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::NotFound("channel inbound receipt".to_owned()))?;
        if existing.payload_hash != payload_hash {
            return Err(DbError::Conflict(
                "channel inbound operation identity was reused with a different payload".to_owned(),
            ));
        }
        Ok(false)
    }

    async fn settle_inbound_receipt(
        &self,
        operation_key: &str,
        payload_hash: &str,
        owner_generation: i64,
        status: &str,
        params: &SettleChannelInboundReceiptParams,
        now: nomifun_common::TimestampMs,
    ) -> Result<ChannelInboundReceiptRow, DbError> {
        if !matches!(status, "completed" | "failed") {
            return Err(DbError::Conflict(format!(
                "channel inbound settlement status '{status}' is not supported"
            )));
        }
        if let Some(conversation_id) = params.conversation_id.as_deref() {
            ConversationId::parse(conversation_id).map_err(|error| {
                DbError::Conflict(format!(
                    "channel inbound conversation_id '{conversation_id}' is not canonical: {error}"
                ))
            })?;
        }
        if let Some(message_id) = params.message_id.as_deref() {
            MessageId::parse(message_id).map_err(|error| {
                DbError::Conflict(format!(
                    "channel inbound message_id '{message_id}' is not canonical: {error}"
                ))
            })?;
        }
        if let Some(outcome) = params.outcome_json.as_deref() {
            let value: serde_json::Value = serde_json::from_str(outcome).map_err(|error| {
                DbError::Conflict(format!("channel inbound outcome is invalid JSON: {error}"))
            })?;
            if !value.is_object() {
                return Err(DbError::Conflict(
                    "channel inbound outcome must be a JSON object".to_owned(),
                ));
            }
        }

        let settled = sqlx::query_as::<_, ChannelInboundReceiptRow>(
            "UPDATE channel_inbound_receipts \
             SET status = ?, phase = 'settled', \
                 conversation_scope_id = ?, message_scope_id = ?, \
                 conversation_id = CASE \
                     WHEN ? IS NOT NULL AND EXISTS(\
                         SELECT 1 FROM conversations WHERE conversation_id = ?\
                     ) THEN ? ELSE NULL END, \
                 message_id = CASE \
                     WHEN ? IS NOT NULL AND EXISTS(\
                         SELECT 1 FROM messages WHERE message_id = ?\
                     ) THEN ? ELSE NULL END, \
                 outcome_json = ?, error_text = ?, \
                 updated_at = ?, completed_at = ? \
             WHERE operation_key = ? AND payload_hash = ? \
               AND status = 'accepted' AND phase = 'effects_started' \
               AND owner_generation = ? \
             RETURNING *",
        )
        .bind(status)
        .bind(&params.conversation_id)
        .bind(&params.message_id)
        .bind(&params.conversation_id)
        .bind(&params.conversation_id)
        .bind(&params.conversation_id)
        .bind(&params.message_id)
        .bind(&params.message_id)
        .bind(&params.message_id)
        .bind(&params.outcome_json)
        .bind(&params.error_text)
        .bind(now)
        .bind(now)
        .bind(operation_key)
        .bind(payload_hash)
        .bind(owner_generation)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(receipt) = settled {
            return Ok(receipt);
        }

        let existing = sqlx::query_as::<_, ChannelInboundReceiptRow>(
            "SELECT * FROM channel_inbound_receipts WHERE operation_key = ?",
        )
        .bind(operation_key)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::NotFound("channel inbound receipt".to_owned()))?;
        if existing.payload_hash == payload_hash
            && existing.owner_generation == owner_generation
            && existing.status == status
            && existing.phase == "settled"
            && existing.conversation_scope_id == params.conversation_id
            && existing.message_scope_id == params.message_id
            && existing.outcome_json == params.outcome_json
            && existing.error_text == params.error_text
        {
            return Ok(existing);
        }
        Err(DbError::Conflict(
            "channel inbound receipt cannot be settled by this owner generation".to_owned(),
        ))
    }

    // -- Pairing Codes ------------------------------------------------

    async fn create_pairing(&self, row: &NewChannelPairingCodeRow) -> Result<ChannelPairingCodeRow, DbError> {
        let mut tx = self.pool.begin().await?;
        lock_channel_plugin(
            &mut tx,
            row.channel_plugin_id.as_deref(),
            "channel pairing",
        )
        .await?;
        let inserted = sqlx::query_as::<_, ChannelPairingCodeRow>(
            "INSERT INTO channel_pairing_codes \
                (code, platform_user_id, platform_type, channel_plugin_id, display_name, \
                 requested_at, expires_at, status) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING *",
        )
        .bind(&row.code)
        .bind(&row.platform_user_id)
        .bind(&row.platform_type)
        .bind(&row.channel_plugin_id)
        .bind(&row.display_name)
        .bind(row.requested_at)
        .bind(row.expires_at)
        .bind(&row.status)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                DbError::Conflict(format!("Pairing code '{}' already exists", row.code))
            } else {
                DbError::Query(e)
            }
        })?;
        tx.commit().await?;
        Ok(inserted)
    }

    async fn get_pending_pairings(&self) -> Result<Vec<ChannelPairingCodeRow>, DbError> {
        let rows = sqlx::query_as::<_, ChannelPairingCodeRow>(
            "SELECT * FROM channel_pairing_codes \
             WHERE status = 'pending' \
             ORDER BY requested_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get_pairing_by_code(&self, code: &str) -> Result<Option<ChannelPairingCodeRow>, DbError> {
        let row = sqlx::query_as::<_, ChannelPairingCodeRow>("SELECT * FROM channel_pairing_codes WHERE code = ?")
            .bind(code)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn update_pairing_status(&self, code: &str, status: &str) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE channel_pairing_codes SET status = ? WHERE code = ?")
            .bind(status)
            .bind(code)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Pairing code '{code}' not found")));
        }
        Ok(())
    }

    async fn approve_pairing_and_retire_non_direct_sessions(
        &self,
        code: &str,
        now: nomifun_common::TimestampMs,
    ) -> Result<PairingApprovalOutcome, DbError> {
        let mut tx = self.pool.begin().await?;
        let pairing = sqlx::query_as::<_, ChannelPairingCodeRow>(
            "SELECT * FROM channel_pairing_codes WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(pairing) = pairing else {
            return Ok(PairingApprovalOutcome::NotFound);
        };
        if pairing.status != "pending" {
            return Ok(PairingApprovalOutcome::AlreadyProcessed);
        }
        if pairing.expires_at <= now {
            sqlx::query(
                "UPDATE channel_pairing_codes SET status = 'expired' \
                 WHERE code = ? AND status = 'pending' AND expires_at <= ?",
            )
            .bind(code)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(PairingApprovalOutcome::Expired);
        }

        let channel_plugin_id = pairing.channel_plugin_id.as_deref().ok_or_else(|| {
            DbError::Conflict(format!(
                "Pairing code '{code}' is not scoped to a channel plugin"
            ))
        })?;
        lock_channel_plugin(&mut tx, Some(channel_plugin_id), "pairing approval").await?;

        // The upsert deliberately promotes only auto_group. A second pairing
        // for an already-approved identity preserves the existing conflict
        // semantics and rolls the entire transition back.
        let channel_user_id = ChannelUserId::new().into_string();
        let user = sqlx::query_as::<_, ChannelUserRow>(
            "INSERT INTO channel_users \
                (channel_user_id, platform_user_id, platform_type, channel_plugin_id, \
                 display_name, authorization_kind, authorized_at, last_active) \
             VALUES (?, ?, ?, ?, ?, 'approved', ?, NULL) \
             ON CONFLICT(platform_user_id, platform_type, channel_plugin_id) DO UPDATE SET \
                 display_name = COALESCE(excluded.display_name, channel_users.display_name), \
                 authorization_kind = 'approved', \
                 authorized_at = excluded.authorized_at \
             WHERE channel_users.authorization_kind = 'auto_group' \
             RETURNING *",
        )
        .bind(channel_user_id)
        .bind(&pairing.platform_user_id)
        .bind(&pairing.platform_type)
        .bind(channel_plugin_id)
        .bind(&pairing.display_name)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| {
            if is_unique_violation(&error) {
                DbError::Conflict(format!(
                    "User '{}' on platform '{}' already exists",
                    pairing.platform_user_id, pairing.platform_type
                ))
            } else {
                DbError::Query(error)
            }
        })?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "User '{}' on platform '{}' already exists",
                pairing.platform_user_id, pairing.platform_type
            ))
        })?;

        let pairing_update = sqlx::query(
            "UPDATE channel_pairing_codes SET status = 'approved' \
             WHERE code = ? AND status = 'pending' AND expires_at > ?",
        )
        .bind(code)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if pairing_update.rows_affected() != 1 {
            return Err(DbError::Conflict(format!(
                "Pairing code '{code}' changed during approval"
            )));
        }

        sqlx::query(
            "UPDATE channel_pending_prompts AS prompt \
             SET state = 'cancelled', settled_at = ? \
             WHERE prompt.channel_plugin_id = ? AND prompt.state = 'queued' \
               AND EXISTS ( \
                   SELECT 1 FROM channel_sessions AS session \
                   WHERE session.channel_session_id = prompt.channel_session_id \
                     AND session.channel_plugin_id = ? \
                     AND session.channel_user_id = ? \
                     AND session.chat_kind IN ('group', 'unknown') \
               )",
        )
        .bind(now)
        .bind(channel_plugin_id)
        .bind(channel_plugin_id)
        .bind(&user.channel_user_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM channel_session_bindings \
             WHERE channel_plugin_id = ? AND channel_user_id = ? \
               AND channel_session_id IN ( \
                   SELECT channel_session_id FROM channel_sessions \
                   WHERE channel_plugin_id = ? AND channel_user_id = ? \
                     AND chat_kind IN ('group', 'unknown') \
               )",
        )
        .bind(channel_plugin_id)
        .bind(&user.channel_user_id)
        .bind(channel_plugin_id)
        .bind(&user.channel_user_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM channel_sessions \
             WHERE channel_plugin_id = ? AND channel_user_id = ? \
               AND chat_kind IN ('group', 'unknown')",
        )
        .bind(channel_plugin_id)
        .bind(&user.channel_user_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(PairingApprovalOutcome::Approved(user))
    }

    async fn cleanup_expired_pairings(&self, now: nomifun_common::TimestampMs) -> Result<u64, DbError> {
        let result = sqlx::query(
            "UPDATE channel_pairing_codes \
             SET status = 'expired' \
             WHERE status = 'pending' AND expires_at <= ?",
        )
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // -- Busy-time pending prompt queue (spec D1) ---------------------

    async fn enqueue_pending_prompt(
        &self,
        row: &NewChannelPendingPromptRow,
        now: nomifun_common::TimestampMs,
    ) -> Result<PendingPromptEnqueue, DbError> {
        ChannelPluginId::parse(&row.channel_plugin_id).map_err(|error| {
            DbError::Conflict(format!(
                "pending prompt channel plugin id is invalid: {error}"
            ))
        })?;
        ChannelSessionId::parse(&row.channel_session_id).map_err(|error| {
            DbError::Conflict(format!(
                "pending prompt channel session id is invalid: {error}"
            ))
        })?;
        ConversationId::parse(&row.conversation_id).map_err(|error| {
            DbError::Conflict(format!("pending prompt conversation id is invalid: {error}"))
        })?;
        if row.chat_id.trim().is_empty() || row.text.trim().is_empty() {
            return Err(DbError::Conflict(
                "pending prompt requires a chat id and non-empty text".to_owned(),
            ));
        }
        if row.idempotency_key.trim().is_empty() {
            return Err(DbError::Conflict(
                "pending prompt requires an idempotency key".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        // The zero-row write upgrades this transaction to the SQLite writer
        // before the cap read, so concurrent enqueues of the same conversation
        // serialize and the COUNT + INSERT pair is atomic.
        sqlx::query(
            "UPDATE channel_pending_prompts SET state = state \
             WHERE conversation_id = ? AND state = 'queued' AND 0",
        )
        .bind(&row.conversation_id)
        .execute(&mut *tx)
        .await?;
        let queued: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channel_pending_prompts \
             WHERE conversation_id = ? AND state = 'queued'",
        )
        .bind(&row.conversation_id)
        .fetch_one(&mut *tx)
        .await?;
        if queued >= crate::repository::channel::PENDING_PROMPT_QUEUE_LIMIT {
            tx.rollback().await?;
            return Ok(PendingPromptEnqueue::QueueFull);
        }
        let prompt_id = nomifun_common::ChannelPendingPromptId::new().into_string();
        let inserted = sqlx::query_as::<_, ChannelPendingPromptRow>(
            "INSERT INTO channel_pending_prompts \
                (prompt_id, channel_plugin_id, chat_id, channel_session_id, conversation_id, \
                 text, idempotency_key, state, attempts, queued_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', 0, ?) \
             RETURNING prompt_id, channel_plugin_id, chat_id, channel_session_id, \
                       conversation_id, text, idempotency_key, state, attempts, \
                       queued_at, settled_at",
        )
        .bind(&prompt_id)
        .bind(&row.channel_plugin_id)
        .bind(&row.chat_id)
        .bind(&row.channel_session_id)
        .bind(&row.conversation_id)
        .bind(&row.text)
        .bind(&row.idempotency_key)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(PendingPromptEnqueue::Queued {
            row: inserted,
            position: queued + 1,
        })
    }

    async fn peek_next_queued(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ChannelPendingPromptRow>, DbError> {
        let row = sqlx::query_as::<_, ChannelPendingPromptRow>(
            "SELECT prompt_id, channel_plugin_id, chat_id, channel_session_id, \
                    conversation_id, text, idempotency_key, state, attempts, \
                    queued_at, settled_at \
             FROM channel_pending_prompts \
             WHERE conversation_id = ? AND state = 'queued' \
             ORDER BY id ASC LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn settle_prompt(
        &self,
        prompt_id: &str,
        state: &str,
        now: nomifun_common::TimestampMs,
    ) -> Result<(), DbError> {
        if !matches!(state, "delivered" | "expired" | "cancelled" | "failed") {
            return Err(DbError::Conflict(format!(
                "pending prompt cannot settle into state '{state}'"
            )));
        }
        let result = sqlx::query(
            "UPDATE channel_pending_prompts \
             SET state = ?, settled_at = ? \
             WHERE prompt_id = ? AND state = 'queued'",
        )
        .bind(state)
        .bind(now)
        .bind(prompt_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "pending prompt '{prompt_id}' is not queued; terminal states are absorbing"
            )));
        }
        Ok(())
    }

    async fn increment_prompt_attempts(&self, prompt_id: &str) -> Result<i64, DbError> {
        let attempts: Option<i64> = sqlx::query_scalar(
            "UPDATE channel_pending_prompts \
             SET attempts = attempts + 1 \
             WHERE prompt_id = ? AND state = 'queued' \
             RETURNING attempts",
        )
        .bind(prompt_id)
        .fetch_optional(&self.pool)
        .await?;
        attempts.ok_or_else(|| {
            DbError::Conflict(format!(
                "pending prompt '{prompt_id}' is not queued; retries only apply to queued prompts"
            ))
        })
    }

    async fn expire_stale(
        &self,
        before_ms: nomifun_common::TimestampMs,
        now: nomifun_common::TimestampMs,
    ) -> Result<Vec<ChannelPendingPromptRow>, DbError> {
        let rows = sqlx::query_as::<_, ChannelPendingPromptRow>(
            "UPDATE channel_pending_prompts \
             SET state = 'expired', settled_at = ? \
             WHERE state = 'queued' AND queued_at < ? \
             RETURNING prompt_id, channel_plugin_id, chat_id, channel_session_id, \
                       conversation_id, text, idempotency_key, state, attempts, \
                       queued_at, settled_at",
        )
        .bind(now)
        .bind(before_ms)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn cancel_chat_queue(
        &self,
        channel_plugin_id: &str,
        chat_id: &str,
        now: nomifun_common::TimestampMs,
    ) -> Result<u64, DbError> {
        let result = sqlx::query(
            "UPDATE channel_pending_prompts \
             SET state = 'cancelled', settled_at = ? \
             WHERE channel_plugin_id = ? AND chat_id = ? AND state = 'queued'",
        )
        .bind(now)
        .bind(channel_plugin_id)
        .bind(chat_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    async fn list_queued_conversations(&self) -> Result<Vec<String>, DbError> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT conversation_id FROM channel_pending_prompts \
             WHERE state = 'queued' ORDER BY conversation_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

/// Checks whether a sqlx error indicates a UNIQUE constraint violation.
fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db_err) => db_err.message().contains("UNIQUE constraint failed"),
        _ => false,
    }
}

/// Checks whether a sqlx error carries the owner-domain guard-trigger abort
/// (migration 020: cs-domain bots must never carry a companion binding).
fn is_owner_domain_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db_err) => db_err
            .message()
            .contains("customer-service channel bots cannot carry a companion binding"),
        _ => false,
    }
}

fn owner_domain_conflict() -> DbError {
    DbError::Conflict(
        "customer-service channel bots cannot carry a companion binding".to_owned(),
    )
}

#[cfg(test)]
mod tests {
    const MISSING_ID: &str = "0190f5fe-7c00-7a00-8000-000000000999";

    use super::*;
    use crate::init_database_memory;

    async fn setup() -> (SqliteChannelRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteChannelRepository::new(db.pool().clone());
        (repo, db)
    }

    fn sample_plugin() -> NewChannelPluginRow {
        let now = nomifun_common::now_ms();
        NewChannelPluginRow {
            r#type: "telegram".into(),
            name: "My Telegram Bot".into(),
            enabled: false,
            config: r#"{"credentials":{"token":"enc_xxx"}}"#.into(),
            status: None,
            last_connected: None,
            companion_id: None,
            bot_key: None,
            owner_domain: crate::models::default_owner_domain(),
            group_access_mode: crate::models::default_group_access_mode(),
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_user(channel_plugin_id: &str) -> NewChannelUserRow {
        let now = nomifun_common::now_ms();
        NewChannelUserRow {
            platform_user_id: "tg_12345".into(),
            platform_type: "telegram".into(),
            channel_plugin_id: Some(channel_plugin_id.to_owned()),
            display_name: Some("Alice".into()),
            authorization_kind: crate::models::default_channel_user_authorization_kind(),
            authorized_at: now,
            last_active: None,
        }
    }

    fn sample_session(
        channel_user_id: &str,
        channel_plugin_id: &str,
        chat_id: &str,
    ) -> NewChannelSessionRow {
        let now = nomifun_common::now_ms();
        NewChannelSessionRow {
            channel_session_id: nomifun_common::ChannelSessionId::new().into_string(),
            channel_user_id: channel_user_id.to_owned(),
            agent_type: "acp".into(),
            conversation_id: None,
            workspace: None,
            chat_id: Some(chat_id.into()),
            channel_plugin_id: Some(channel_plugin_id.to_owned()),
            chat_kind: crate::models::default_channel_chat_kind(),
            created_at: now,
            last_activity: now,
        }
    }

    fn sample_pairing() -> NewChannelPairingCodeRow {
        let now = nomifun_common::now_ms();
        NewChannelPairingCodeRow {
            code: "123456".into(),
            platform_user_id: "tg_99".into(),
            platform_type: "telegram".into(),
            channel_plugin_id: None,
            display_name: Some("Bob".into()),
            requested_at: now,
            expires_at: now + 600_000,
            status: "pending".into(),
        }
    }

    fn sample_inbound_receipt(
        operation_suffix: char,
        chat_id: &str,
        provider_event_id: &str,
        payload_suffix: char,
        created_at: i64,
    ) -> NewChannelInboundReceiptRow {
        NewChannelInboundReceiptRow {
            operation_key: format!(
                "channel-inbound:v1:{}",
                operation_suffix.to_string().repeat(64)
            ),
            user_id: UserId::new().into_string(),
            channel_plugin_id: ChannelPluginId::new().into_string(),
            platform: "telegram".into(),
            chat_id: chat_id.into(),
            provider_event_id: provider_event_id.into(),
            payload_hash: payload_suffix.to_string().repeat(64),
            created_at,
        }
    }

    async fn make_inbound_claimable(
        repo: &SqliteChannelRepository,
        db: &crate::Database,
        mut row: NewChannelInboundReceiptRow,
    ) -> NewChannelInboundReceiptRow {
        let plugin = seed_channel(repo, "Inbound Test Bot").await;
        row.user_id = sqlx::query_scalar(
            "SELECT owner_user_id FROM installation_identity \
             WHERE singleton_key = 'installation'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        row.channel_plugin_id = plugin.channel_plugin_id;
        row
    }

    async fn seed_channel(repo: &SqliteChannelRepository, name: &str) -> ChannelPluginRow {
        let mut plugin = sample_plugin();
        plugin.name = name.into();
        repo.create_plugin(&plugin).await.unwrap()
    }

    async fn seed_user(
        repo: &SqliteChannelRepository,
        channel_plugin_id: &str,
    ) -> ChannelUserRow {
        repo.create_user(&sample_user(channel_plugin_id))
            .await
            .unwrap()
    }

    // -- Plugin tests -----------------------------------------------------

    #[tokio::test]
    async fn get_all_plugins_empty() {
        let (repo, _db) = setup().await;
        let plugins = repo.get_all_plugins().await.unwrap();
        assert!(plugins.is_empty());
    }

    #[tokio::test]
    async fn create_and_get_plugin() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();

        let found = repo
            .get_plugin(&plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.channel_plugin_id, plugin.channel_plugin_id);
        assert_eq!(found.r#type, "telegram");
        assert_eq!(found.name, "My Telegram Bot");
        assert!(!found.enabled);
    }

    #[tokio::test]
    async fn update_plugin_updates_existing() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();

        let updated = ChannelPluginRow {
            name: "Updated Bot".into(),
            enabled: true,
            updated_at: nomifun_common::now_ms(),
            ..plugin
        };
        repo.update_plugin(&updated).await.unwrap();

        let found = repo
            .get_plugin(&updated.channel_plugin_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.name, "Updated Bot");
        assert!(found.enabled);
    }

    #[tokio::test]
    async fn get_all_plugins_returns_multiple() {
        let (repo, _db) = setup().await;
        repo.create_plugin(&sample_plugin()).await.unwrap();

        let now = nomifun_common::now_ms();
        let lark = NewChannelPluginRow {
            r#type: "lark".into(),
            name: "Lark Bot".into(),
            enabled: true,
            config: "{}".into(),
            status: Some("running".into()),
            last_connected: Some(now),
            companion_id: None,
            bot_key: None,
            owner_domain: crate::models::default_owner_domain(),
            group_access_mode: crate::models::default_group_access_mode(),
            created_at: now,
            updated_at: now,
        };
        repo.create_plugin(&lark).await.unwrap();

        let all = repo.get_all_plugins().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn update_plugin_status_sets_fields() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();

        let now = nomifun_common::now_ms();
        repo.update_plugin_status(
            &plugin.channel_plugin_id,
            &UpdatePluginStatusParams {
                status: Some("running".into()),
                last_connected: Some(now),
                enabled: Some(true),
            },
        )
        .await
        .unwrap();

        let found = repo
            .get_plugin(&plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status.as_deref(), Some("running"));
        assert_eq!(found.last_connected, Some(now));
        assert!(found.enabled);
    }

    #[tokio::test]
    async fn update_plugin_status_not_found() {
        let (repo, _db) = setup().await;
        let missing_id = ChannelPluginId::new();
        let err = repo
            .update_plugin_status(
                missing_id.as_str(),
                &UpdatePluginStatusParams {
                    status: Some("error".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_plugin_status_empty_params_is_noop() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();
        // No fields to update → no-op, no error.
        repo.update_plugin_status(
            &plugin.channel_plugin_id,
            &UpdatePluginStatusParams::default(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn delete_plugin_removes_row() {
        let (repo, db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();
        let other_plugin = seed_channel(&repo, "Other channel").await;
        let owned_user = seed_user(&repo, &plugin.channel_plugin_id).await;
        let mut unscoped_user = sample_user(&other_plugin.channel_plugin_id);
        unscoped_user.platform_user_id = "tg_unscoped".into();
        unscoped_user.channel_plugin_id = None;
        let other_user = repo.create_user(&unscoped_user).await.unwrap();
        let owned_session = sample_session(
            &owned_user.channel_user_id,
            &plugin.channel_plugin_id,
            "owned",
        );
        repo.get_or_create_session(
            &owned_user.channel_user_id,
            "owned",
            &plugin.channel_plugin_id,
            &owned_session,
        )
            .await
            .unwrap();
        let retained_session = sample_session(
            &other_user.channel_user_id,
            &plugin.channel_plugin_id,
            "retained",
        );
        let retained_session = repo
            .get_or_create_session(
                &other_user.channel_user_id,
                "retained",
                &plugin.channel_plugin_id,
                &retained_session,
            )
            .await
            .unwrap();
        let mut pairing = sample_pairing();
        pairing.code = "654321".into();
        pairing.channel_plugin_id = Some(plugin.channel_plugin_id.clone());
        repo.create_pairing(&pairing).await.unwrap();

        repo.delete_plugin(&plugin.channel_plugin_id).await.unwrap();

        assert!(
            repo.get_plugin(&plugin.channel_plugin_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            repo.get_user_by_platform(
                "tg_12345",
                "telegram",
                &plugin.channel_plugin_id,
            )
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            repo.get_session(&owned_session.channel_session_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            repo.get_pairing_by_code("654321")
                .await
                .unwrap()
                .is_none()
        );
        let retained = repo
            .get_session(&retained_session.channel_session_id)
            .await
            .unwrap()
            .unwrap();
        assert!(retained.channel_plugin_id.is_none());

        let remaining_user_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channel_users WHERE channel_user_id = ?")
                .bind(&other_user.channel_user_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(remaining_user_count, 1);
    }

    #[tokio::test]
    async fn delete_plugin_not_found() {
        let (repo, _db) = setup().await;
        let missing_id = ChannelPluginId::new();
        let err = repo.delete_plugin(missing_id.as_str()).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn same_bot_key_on_two_rows_conflicts() {
        let (repo, _db) = setup().await;
        let now = nomifun_common::now_ms();
        let companion_a = CompanionId::new().into_string();
        let companion_b = CompanionId::new().into_string();
        let bot = |name: &str, companion: &str| NewChannelPluginRow {
            r#type: "lark".into(),
            name: name.into(),
            enabled: true,
            config: "enc".into(),
            status: None,
            last_connected: None,
            companion_id: Some(companion.into()),
            bot_key: Some("cli_same_app".into()),
            owner_domain: crate::models::default_owner_domain(),
            group_access_mode: crate::models::default_group_access_mode(),
            created_at: now,
            updated_at: now,
        };
        let first = repo
            .create_plugin(&bot("Lark Bot A", &companion_a))
            .await
            .unwrap();

        // Same lark app on a second row (= bound to another companion) must fail.
        let err = repo
            .create_plugin(&bot("Lark Bot B", &companion_b))
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));

        // Updating the same row keeps working.
        repo.update_plugin(&ChannelPluginRow {
            companion_id: Some(companion_b),
            updated_at: nomifun_common::now_ms(),
            ..first
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn different_bot_keys_same_platform_coexist() {
        let (repo, _db) = setup().await;
        let now = nomifun_common::now_ms();
        for (name, key) in [("Lark Bot A", "cli_app_a"), ("Lark Bot B", "cli_app_b")] {
            repo.create_plugin(&NewChannelPluginRow {
                r#type: "lark".into(),
                name: name.into(),
                enabled: true,
                config: "enc".into(),
                status: None,
                last_connected: None,
                companion_id: None,
                bot_key: Some(key.into()),
                owner_domain: crate::models::default_owner_domain(),
                group_access_mode: crate::models::default_group_access_mode(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        }
        assert_eq!(repo.get_all_plugins().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn update_plugin_companion_roundtrip_and_clear() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();
        let companion_id = CompanionId::new().into_string();

        repo.update_plugin_companion(&plugin.channel_plugin_id, Some(&companion_id))
            .await
            .unwrap();
        assert_eq!(
            repo.get_plugin(&plugin.channel_plugin_id)
                .await
                .unwrap()
                .unwrap()
                .companion_id
                .as_deref(),
            Some(companion_id.as_str())
        );

        repo.update_plugin_companion(&plugin.channel_plugin_id, None)
            .await
            .unwrap();
        assert!(
            repo.get_plugin(&plugin.channel_plugin_id)
                .await
                .unwrap()
                .unwrap()
                .companion_id
                .is_none()
        );

        let missing_id = ChannelPluginId::new();
        let err = repo
            .update_plugin_companion(missing_id.as_str(), Some(&companion_id))
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    // -- Owner-domain tests (migration 020) ---------------------------------

    fn cs_plugin(name: &str) -> NewChannelPluginRow {
        NewChannelPluginRow {
            name: name.into(),
            owner_domain: crate::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE.into(),
            ..sample_plugin()
        }
    }

    #[tokio::test]
    async fn create_plugin_defaults_to_companion_domain() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();
        assert_eq!(plugin.owner_domain, "companion");
    }

    #[tokio::test]
    async fn create_customer_service_plugin_roundtrips_domain() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&cs_plugin("CS Bot")).await.unwrap();
        assert_eq!(plugin.owner_domain, "customer_service");
        assert_eq!(
            repo.get_plugin(&plugin.channel_plugin_id)
                .await
                .unwrap()
                .unwrap()
                .owner_domain,
            "customer_service"
        );
    }

    #[tokio::test]
    async fn create_plugin_rejects_unknown_owner_domain() {
        let (repo, _db) = setup().await;
        let mut plugin = sample_plugin();
        plugin.owner_domain = "somebody_else".into();
        let err = repo.create_plugin(&plugin).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn create_customer_service_plugin_with_companion_is_rejected() {
        let (repo, _db) = setup().await;
        let mut plugin = cs_plugin("CS Bot");
        plugin.companion_id = Some(CompanionId::new().into_string());
        let err = repo.create_plugin(&plugin).await.unwrap_err();
        assert!(
            matches!(&err, DbError::Conflict(message) if message.contains("companion binding")),
            "trigger must abort a cs-domain insert carrying companion_id: {err:?}"
        );
    }

    #[tokio::test]
    async fn customer_service_plugin_cannot_gain_companion_binding() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&cs_plugin("CS Bot")).await.unwrap();
        let companion_id = CompanionId::new().into_string();

        // Direct binding write is aborted by the update guard trigger.
        let err = repo
            .update_plugin_companion(&plugin.channel_plugin_id, Some(&companion_id))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, DbError::Conflict(message) if message.contains("companion binding")),
            "update guard must abort: {err:?}"
        );

        // Full-row update carrying both is equally aborted.
        let err = repo
            .update_plugin(&ChannelPluginRow {
                companion_id: Some(companion_id),
                updated_at: nomifun_common::now_ms(),
                ..plugin
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn companion_plugin_cannot_switch_to_customer_service_while_bound() {
        let (repo, _db) = setup().await;
        let mut plugin = sample_plugin();
        plugin.companion_id = Some(CompanionId::new().into_string());
        let created = repo.create_plugin(&plugin).await.unwrap();

        let err = repo
            .update_plugin(&ChannelPluginRow {
                owner_domain: crate::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE.into(),
                updated_at: nomifun_common::now_ms(),
                ..created
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn list_plugins_by_owner_domain_filters() {
        let (repo, _db) = setup().await;
        repo.create_plugin(&sample_plugin()).await.unwrap();
        let cs = repo.create_plugin(&cs_plugin("CS Bot")).await.unwrap();

        let companion_rows = repo.list_plugins_by_owner_domain("companion").await.unwrap();
        assert_eq!(companion_rows.len(), 1);
        assert_eq!(companion_rows[0].owner_domain, "companion");

        let cs_rows = repo
            .list_plugins_by_owner_domain("customer_service")
            .await
            .unwrap();
        assert_eq!(cs_rows.len(), 1);
        assert_eq!(cs_rows[0].channel_plugin_id, cs.channel_plugin_id);

        let err = repo.list_plugins_by_owner_domain("bogus").await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }


    #[tokio::test]
    async fn update_plugin_bot_key_backfills() {
        let (repo, _db) = setup().await;
        let plugin = repo.create_plugin(&sample_plugin()).await.unwrap();

        repo.update_plugin_bot_key(&plugin.channel_plugin_id, "123456")
            .await
            .unwrap();
        assert_eq!(
            repo.get_plugin(&plugin.channel_plugin_id)
                .await
                .unwrap()
                .unwrap()
                .bot_key
                .as_deref(),
            Some("123456")
        );
    }

    // -- User tests -------------------------------------------------------

    #[tokio::test]
    async fn get_all_users_empty() {
        let (repo, _db) = setup().await;
        let users = repo.get_all_users().await.unwrap();
        assert!(users.is_empty());
    }

    #[tokio::test]
    async fn create_and_get_user_by_platform() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = repo
            .create_user(&sample_user(&plugin.channel_plugin_id))
            .await
            .unwrap();

        let found = repo
            .get_user_by_platform("tg_12345", "telegram", &plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.channel_user_id, user.channel_user_id);
        assert_eq!(found.display_name.as_deref(), Some("Alice"));
    }

    #[tokio::test]
    async fn create_duplicate_user_returns_conflict() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = sample_user(&plugin.channel_plugin_id);
        repo.create_user(&user).await.unwrap();

        let err = repo.create_user(&user).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn get_user_by_platform_not_found() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        assert!(
            repo.get_user_by_platform("nope", "telegram", &plugin.channel_plugin_id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn same_platform_user_two_channels_are_independent() {
        let (repo, _db) = setup().await;
        let first_plugin = seed_channel(&repo, "Lark Stub A").await;
        let second_plugin = seed_channel(&repo, "Lark Stub B").await;

        let mk = |channel_plugin_id: &str| NewChannelUserRow {
            platform_user_id: "ou_same".into(),
            platform_type: "lark".into(),
            channel_plugin_id: Some(channel_plugin_id.to_owned()),
            display_name: None,
            authorization_kind: crate::models::default_channel_user_authorization_kind(),
            authorized_at: nomifun_common::now_ms(),
            last_active: None,
        };
        let first_user = repo
            .create_user(&mk(&first_plugin.channel_plugin_id))
            .await
            .unwrap();
        let second_user = repo
            .create_user(&mk(&second_plugin.channel_plugin_id))
            .await
            .unwrap();

        assert_eq!(
            repo.get_user_by_platform("ou_same", "lark", &first_plugin.channel_plugin_id)
                .await
                .unwrap()
                .unwrap()
                .channel_user_id,
            first_user.channel_user_id
        );
        assert_eq!(
            repo.get_user_by_platform("ou_same", "lark", &second_plugin.channel_plugin_id)
                .await
                .unwrap()
                .unwrap()
                .channel_user_id,
            second_user.channel_user_id
        );
    }

    #[tokio::test]
    async fn deleting_channel_removes_scoped_user() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Lark Stub").await;
        repo.create_user(&NewChannelUserRow {
                platform_user_id: "ou_x".into(),
                platform_type: "lark".into(),
                channel_plugin_id: Some(plugin.channel_plugin_id.clone()),
                display_name: None,
                authorization_kind: crate::models::default_channel_user_authorization_kind(),
                authorized_at: nomifun_common::now_ms(),
                last_active: None,
            })
            .await
            .unwrap();

        repo.delete_plugin(&plugin.channel_plugin_id).await.unwrap();
        assert!(
            repo.get_all_users()
                .await
                .unwrap()
                .iter()
                .all(|user| user.platform_user_id != "ou_x")
        );
    }

    #[tokio::test]
    async fn update_user_last_active_updates_timestamp() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, &plugin.channel_plugin_id).await;

        let new_ts = nomifun_common::now_ms() + 5000;
        repo.update_user_last_active(&user.channel_user_id, new_ts)
            .await
            .unwrap();

        let found = repo
            .get_user_by_platform("tg_12345", "telegram", &plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.last_active, Some(new_ts));
    }

    #[tokio::test]
    async fn update_user_last_active_not_found() {
        let (repo, _db) = setup().await;
        let missing_id = ChannelUserId::new();
        let err = repo
            .update_user_last_active(missing_id.as_str(), 123)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_user_removes_row() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, &plugin.channel_plugin_id).await;
        repo.delete_user(&user.channel_user_id).await.unwrap();
        assert!(
            repo.get_user_by_platform("tg_12345", "telegram", &plugin.channel_plugin_id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_user_not_found() {
        let (repo, _db) = setup().await;
        let missing_id = ChannelUserId::new();
        let err = repo.delete_user(missing_id.as_str()).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_user_cascades_sessions() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, &plugin.channel_plugin_id).await;

        let session = sample_session(
            &user.channel_user_id,
            &plugin.channel_plugin_id,
            "chat-abc",
        );
        repo.get_or_create_session(
            &user.channel_user_id,
            "chat-abc",
            &plugin.channel_plugin_id,
            &session,
        )
            .await
            .unwrap();

        // Sessions exist before delete.
        assert_eq!(repo.get_all_sessions().await.unwrap().len(), 1);

        repo.delete_user(&user.channel_user_id).await.unwrap();

        assert!(repo.get_all_sessions().await.unwrap().is_empty());
    }

    // -- Session tests ------------------------------------------------

    #[tokio::test]
    async fn get_all_sessions_empty() {
        let (repo, _db) = setup().await;
        assert!(repo.get_all_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_or_create_session_creates_new() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let new = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        let result = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &new)
            .await
            .unwrap();
        assert_eq!(result.channel_session_id, new.channel_session_id);
        assert_eq!(result.channel_user_id, user.channel_user_id.as_str());
        assert_eq!(result.chat_id.as_deref(), Some("chat-abc"));
    }

    #[tokio::test]
    async fn get_or_create_session_reuses_existing() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let new = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        let first = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &new)
            .await
            .unwrap();

        // A different proposed business id still reuses the persisted session.
        let another = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        let second = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &another)
            .await
            .unwrap();
        assert_eq!(second.channel_session_id, first.channel_session_id);
        // last_activity should be updated.
        assert!(second.last_activity >= first.last_activity);
    }

    #[tokio::test]
    async fn concurrent_get_or_create_session_uses_one_canonical_binding() {
        let (repo, db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;
        let first = sample_session(
            user.channel_user_id.as_str(),
            plugin.channel_plugin_id.as_str(),
            "chat-race",
        );
        let second = sample_session(
            user.channel_user_id.as_str(),
            plugin.channel_plugin_id.as_str(),
            "chat-race",
        );
        let first_id = first.channel_session_id.clone();
        let second_id = second.channel_session_id.clone();

        let repo_a = repo.clone();
        let repo_b = repo.clone();
        let user_id_a = user.channel_user_id.clone();
        let user_id_b = user.channel_user_id.clone();
        let plugin_id_a = plugin.channel_plugin_id.clone();
        let plugin_id_b = plugin.channel_plugin_id.clone();
        let (result_a, result_b) = tokio::join!(
            async move {
                repo_a
                    .get_or_create_session(&user_id_a, "chat-race", &plugin_id_a, &first)
                    .await
                    .unwrap()
            },
            async move {
                repo_b
                    .get_or_create_session(&user_id_b, "chat-race", &plugin_id_b, &second)
                    .await
                    .unwrap()
            }
        );

        assert_eq!(result_a.channel_session_id, result_b.channel_session_id);
        assert!(
            result_a.channel_session_id == first_id
                || result_a.channel_session_id == second_id
        );
        let sessions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channel_sessions \
             WHERE channel_plugin_id = ? AND channel_user_id = ? AND chat_id = 'chat-race'",
        )
        .bind(plugin.channel_plugin_id.as_str())
        .bind(user.channel_user_id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap();
        let bindings: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channel_session_bindings \
             WHERE channel_plugin_id = ? AND channel_user_id = ? AND chat_id = 'chat-race'",
        )
        .bind(plugin.channel_plugin_id.as_str())
        .bind(user.channel_user_id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(sessions, 1);
        assert_eq!(bindings, 1);
    }

    #[tokio::test]
    async fn per_chat_isolation_different_chats() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let s1 = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &s1)
            .await
            .unwrap();

        let s2 = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-xyz");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-xyz", plugin.channel_plugin_id.as_str(), &s2)
            .await
            .unwrap();

        assert_eq!(repo.get_all_sessions().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn get_session_by_business_id() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let new = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        let created = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &new)
            .await
            .unwrap();

        let found = repo
            .get_session(&created.channel_session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.channel_session_id, created.channel_session_id);
        assert_eq!(found.agent_type, "acp");
    }

    #[tokio::test]
    async fn get_session_not_found() {
        let (repo, _db) = setup().await;
        assert!(repo.get_session("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_session_activity_updates_timestamp() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let new = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        let created = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &new)
            .await
            .unwrap();

        let new_ts = nomifun_common::now_ms() + 5000;
        repo.update_session_activity(&created.channel_session_id, new_ts)
            .await
            .unwrap();

        let found = repo
            .get_session(&created.channel_session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.last_activity, new_ts);
    }

    #[tokio::test]
    async fn update_session_activity_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.update_session_activity("nope", 123).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_sessions_by_user_removes_all() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let s1 = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &s1)
            .await
            .unwrap();

        let s2 = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-xyz");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-xyz", plugin.channel_plugin_id.as_str(), &s2)
            .await
            .unwrap();

        repo.delete_sessions_by_user(user.channel_user_id.as_str()).await.unwrap();
        assert!(repo.get_all_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_sessions_by_user_no_sessions_is_ok() {
        let (repo, _db) = setup().await;
        // No sessions exist for this user —should not error.
        repo.delete_sessions_by_user(MISSING_ID).await.unwrap();
    }

    /// Helper to create an installation-owned stub conversation for
    /// channel-session logical-reference tests. Channel sessions may point at a
    /// host-capable Conversation, so the fixture must use the one principal
    /// that is allowed to own host execution.
    async fn create_stub_conversation(pool: &SqlitePool, conv_id: &str) {
        let now = nomifun_common::now_ms();
        let installation_owner = crate::installation_owner_id(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO conversations (conversation_id, user_id, name, type, created_at, updated_at) \
             VALUES (?1, ?2, 'Test Conv', 'nomi', ?3, ?3)",
        )
        .bind(conv_id)
        .bind(installation_owner)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn update_session_conversation_persists() {
        let conversation_id = nomifun_common::ConversationId::new().into_string();
        let (repo, db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let new = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        let created = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &new)
            .await
            .unwrap();

        create_stub_conversation(db.pool(), &conversation_id).await;

        repo.update_session_conversation(&created.channel_session_id, &conversation_id)
            .await
            .unwrap();

        let found = repo
            .get_session(&created.channel_session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.conversation_id, Some(conversation_id));
    }

    #[tokio::test]
    async fn update_session_conversation_not_found() {
        let (repo, db) = setup().await;
        let conversation_id = nomifun_common::ConversationId::new();
        create_stub_conversation(db.pool(), conversation_id.as_str()).await;
        let err = repo
            .update_session_conversation("nope", conversation_id.as_str())
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_session_agent_type_persists() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let new = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        let created = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &new)
            .await
            .unwrap();

        assert_eq!(
            repo.get_session(&created.channel_session_id)
                .await
                .unwrap()
                .unwrap()
                .agent_type,
            "acp"
        );

        repo.update_session_agent_type(&created.channel_session_id, "acp")
            .await
            .unwrap();

        let found = repo
            .get_session(&created.channel_session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.agent_type, "acp");
    }

    #[tokio::test]
    async fn update_session_agent_type_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.update_session_agent_type("nope", "acp").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_session_by_user_chat_removes_only_target() {
        let (repo, _db) = setup().await;
        let plugin = seed_channel(&repo, "Telegram Stub").await;
        let user = seed_user(&repo, plugin.channel_plugin_id.as_str()).await;

        let s1 = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-abc");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str(), &s1)
            .await
            .unwrap();

        let s2 = sample_session(user.channel_user_id.as_str(), plugin.channel_plugin_id.as_str(), "chat-xyz");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-xyz", plugin.channel_plugin_id.as_str(), &s2)
            .await
            .unwrap();

        repo.delete_session_by_user_chat(user.channel_user_id.as_str(), "chat-abc", plugin.channel_plugin_id.as_str())
            .await
            .unwrap();

        let remaining = repo.get_all_sessions().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].chat_id.as_deref(), Some("chat-xyz"));
    }

    #[tokio::test]
    async fn delete_session_by_user_chat_no_match_is_ok() {
        let (repo, _db) = setup().await;
        // No sessions exist —should not error.
        repo.delete_session_by_user_chat(MISSING_ID, "chat-abc", MISSING_ID)
            .await
            .unwrap();
    }

    // -- Durable inbound admission tests --------------------------------

    #[tokio::test]
    async fn inbound_same_provider_id_in_different_chat_scopes_does_not_collide() {
        let (repo, db) = setup().await;
        let first = make_inbound_claimable(
            &repo,
            &db,
            sample_inbound_receipt('a', "chat-a", "provider-42", '1', 1_000),
        )
        .await;
        let mut second = first.clone();
        second.operation_key = format!("channel-inbound:v1:{}", "b".repeat(64));
        second.chat_id = "chat-b".into();

        assert!(matches!(
            repo.claim_inbound_receipt(&first).await.unwrap(),
            ChannelInboundClaim::Owner(_)
        ));
        assert!(matches!(
            repo.claim_inbound_receipt(&second).await.unwrap(),
            ChannelInboundClaim::Owner(_)
        ));
    }

    #[tokio::test]
    async fn inbound_same_operation_key_with_different_payload_is_a_conflict() {
        let (repo, db) = setup().await;
        let first = make_inbound_claimable(
            &repo,
            &db,
            sample_inbound_receipt('a', "chat-a", "provider-42", '1', 1_000),
        )
        .await;
        repo.claim_inbound_receipt(&first).await.unwrap();
        let mut changed = first.clone();
        changed.payload_hash = "2".repeat(64);
        changed.created_at = 1_001;

        let error = repo
            .claim_inbound_receipt(&changed)
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn inbound_claim_never_times_out_or_transfers_execution_authority() {
        let (repo, db) = setup().await;
        let first = make_inbound_claimable(
            &repo,
            &db,
            sample_inbound_receipt('a', "chat-a", "provider-42", '1', 1_000),
        )
        .await;
        let owner = match repo.claim_inbound_receipt(&first).await.unwrap() {
            ChannelInboundClaim::Owner(receipt) => receipt,
            ChannelInboundClaim::Replay(_) => panic!("first claim must own"),
        };

        // Even a process paused far beyond the former 30-second lease cannot
        // transfer execution authority. This is deliberately clock-free.
        let mut much_later = first.clone();
        much_later.created_at = 9_000_000;
        assert!(matches!(
            repo.claim_inbound_receipt(&much_later).await.unwrap(),
            ChannelInboundClaim::Replay(receipt)
                if receipt.phase == "claimed"
                    && receipt.owner_generation == owner.owner_generation
        ));

        assert!(
            repo.begin_inbound_effects(
                &owner.operation_key,
                &owner.payload_hash,
                owner.owner_generation,
                9_000_001,
            )
            .await
            .unwrap()
        );
        much_later.created_at = i64::MAX - 1;
        assert!(matches!(
            repo.claim_inbound_receipt(&much_later).await.unwrap(),
            ChannelInboundClaim::Replay(receipt)
                if receipt.phase == "effects_started"
                    && receipt.owner_generation == owner.owner_generation
        ));
    }

    #[tokio::test]
    async fn concurrent_inbound_duplicate_has_exactly_one_owner() {
        let (repo, db) = setup().await;
        let receipt = make_inbound_claimable(
            &repo,
            &db,
            sample_inbound_receipt('a', "chat-a", "provider-42", '1', 1_000),
        )
        .await;
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let repo = repo.clone();
            let receipt = receipt.clone();
            tasks.push(tokio::spawn(async move {
                repo.claim_inbound_receipt(&receipt).await.unwrap()
            }));
        }

        let mut owners = 0;
        let mut replays = 0;
        for task in tasks {
            match task.await.unwrap() {
                ChannelInboundClaim::Owner(_) => owners += 1,
                ChannelInboundClaim::Replay(_) => replays += 1,
            }
        }
        assert_eq!(owners, 1);
        assert_eq!(replays, 15);
    }

    #[tokio::test]
    async fn inbound_settlement_is_absorbing_and_lost_ack_retry_is_idempotent() {
        let (repo, db) = setup().await;
        let input = make_inbound_claimable(
            &repo,
            &db,
            sample_inbound_receipt('a', "chat-a", "provider-42", '1', 1_000),
        )
        .await;
        let owner = match repo.claim_inbound_receipt(&input).await.unwrap() {
            ChannelInboundClaim::Owner(receipt) => receipt,
            ChannelInboundClaim::Replay(_) => panic!("first claim must own"),
        };
        assert!(
            repo.begin_inbound_effects(
                &owner.operation_key,
                &owner.payload_hash,
                owner.owner_generation,
                1_100,
            )
            .await
            .unwrap()
        );
        let params = SettleChannelInboundReceiptParams {
            outcome_json: Some(r#"{"kind":"action"}"#.into()),
            ..Default::default()
        };
        let settled = repo
            .settle_inbound_receipt(
                &owner.operation_key,
                &owner.payload_hash,
                owner.owner_generation,
                "completed",
                &params,
                1_200,
            )
            .await
            .unwrap();
        let retried = repo
            .settle_inbound_receipt(
                &owner.operation_key,
                &owner.payload_hash,
                owner.owner_generation,
                "completed",
                &params,
                1_300,
            )
            .await
            .unwrap();
        assert_eq!(retried, settled);

        let delete = sqlx::query(
            "DELETE FROM channel_inbound_receipts WHERE operation_key = ?",
        )
        .bind(&owner.operation_key)
        .execute(db.pool())
        .await;
        assert!(delete.is_err(), "retained receipt must reject deletion");
    }

    #[tokio::test]
    async fn deleting_plugin_clears_projection_but_preserves_replay_authority() {
        let (repo, db) = setup().await;
        let plugin = seed_channel(&repo, "Disposable Bot").await;
        let owner_user_id: String = sqlx::query_scalar(
            "SELECT owner_user_id FROM installation_identity \
             WHERE singleton_key = 'installation'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        let mut input = sample_inbound_receipt('a', "chat-a", "provider-42", '1', 1_000);
        input.user_id = owner_user_id;
        input.channel_plugin_id = plugin.channel_plugin_id.clone();
        let claimed = match repo.claim_inbound_receipt(&input).await.unwrap() {
            ChannelInboundClaim::Owner(receipt) => receipt,
            ChannelInboundClaim::Replay(_) => panic!("first claim must own"),
        };
        assert!(
            repo.begin_inbound_effects(
                &claimed.operation_key,
                &claimed.payload_hash,
                claimed.owner_generation,
                1_100,
            )
            .await
            .unwrap()
        );

        repo.delete_plugin(&plugin.channel_plugin_id).await.unwrap();
        let retained = sqlx::query_as::<_, ChannelInboundReceiptRow>(
            "SELECT * FROM channel_inbound_receipts WHERE operation_key = ?",
        )
        .bind(&claimed.operation_key)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(
            retained.channel_plugin_scope_id,
            plugin.channel_plugin_id
        );
        assert!(retained.channel_plugin_id.is_none());
        assert!(matches!(
            repo.claim_inbound_receipt(&input).await.unwrap(),
            ChannelInboundClaim::Replay(receipt)
                if receipt.operation_key == claimed.operation_key
                    && receipt.phase == "effects_started"
        ));
    }

    #[tokio::test]
    async fn same_chat_two_channels_get_isolated_sessions() {
        let (repo, _db) = setup().await;
        let first_plugin = seed_channel(&repo, "Telegram Stub A").await;
        let second_plugin = seed_channel(&repo, "Telegram Stub B").await;
        let mut unscoped = sample_user(first_plugin.channel_plugin_id.as_str());
        unscoped.channel_plugin_id = None;
        let user = repo.create_user(&unscoped).await.unwrap();

        let s1 = sample_session(user.channel_user_id.as_str(), first_plugin.channel_plugin_id.as_str(), "chat-abc");
        let first = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", first_plugin.channel_plugin_id.as_str(), &s1)
            .await
            .unwrap();

        // Same user + same chat through another bot → a second session.
        let s2 = sample_session(user.channel_user_id.as_str(), second_plugin.channel_plugin_id.as_str(), "chat-abc");
        let created = repo
            .get_or_create_session(user.channel_user_id.as_str(), "chat-abc", second_plugin.channel_plugin_id.as_str(), &s2)
            .await
            .unwrap();
        assert_eq!(created.channel_session_id, s2.channel_session_id);
        assert_eq!(repo.get_all_sessions().await.unwrap().len(), 2);

        // Reuse matches per channel.
        let reuse_candidate = sample_session(
            user.channel_user_id.as_str(),
            first_plugin.channel_plugin_id.as_str(),
            "chat-abc",
        );
        let reused = repo
            .get_or_create_session(
                user.channel_user_id.as_str(),
                "chat-abc",
                first_plugin.channel_plugin_id.as_str(),
                &reuse_candidate,
            )
            .await
            .unwrap();
        assert_eq!(reused.channel_session_id, first.channel_session_id);
    }

    #[tokio::test]
    async fn delete_sessions_by_channel_only_hits_that_channel() {
        let (repo, db) = setup().await;
        let first_plugin = seed_channel(&repo, "Telegram Stub A").await;
        let second_plugin = seed_channel(&repo, "Telegram Stub B").await;
        let mut unscoped = sample_user(first_plugin.channel_plugin_id.as_str());
        unscoped.channel_plugin_id = None;
        let user = repo.create_user(&unscoped).await.unwrap();

        let s1 = sample_session(user.channel_user_id.as_str(), first_plugin.channel_plugin_id.as_str(), "chat-abc");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-abc", first_plugin.channel_plugin_id.as_str(), &s1)
            .await
            .unwrap();
        let s2 = sample_session(user.channel_user_id.as_str(), second_plugin.channel_plugin_id.as_str(), "chat-abc");
        repo.get_or_create_session(user.channel_user_id.as_str(), "chat-abc", second_plugin.channel_plugin_id.as_str(), &s2)
            .await
            .unwrap();

        for (prompt_id, plugin_id, session_id, state) in [
            (
                "0190f5fe-7c00-7a00-8abc-0123456789d1",
                first_plugin.channel_plugin_id.as_str(),
                s1.channel_session_id.as_str(),
                "queued",
            ),
            (
                "0190f5fe-7c00-7a00-8abc-0123456789d2",
                first_plugin.channel_plugin_id.as_str(),
                s1.channel_session_id.as_str(),
                "delivered",
            ),
            (
                "0190f5fe-7c00-7a00-8abc-0123456789d3",
                second_plugin.channel_plugin_id.as_str(),
                s2.channel_session_id.as_str(),
                "queued",
            ),
        ] {
            sqlx::query(
                "INSERT INTO channel_pending_prompts \
                    (prompt_id, channel_plugin_id, chat_id, channel_session_id, conversation_id, \
                     text, idempotency_key, state, queued_at) \
                 VALUES (?, ?, 'chat-abc', ?, '0190f5fe-7c00-7a00-8abc-0123456789ee', \
                         'text', ?, ?, 1)",
            )
            .bind(prompt_id)
            .bind(plugin_id)
            .bind(session_id)
            .bind(format!("key-{prompt_id}"))
            .bind(state)
            .execute(db.pool())
            .await
            .unwrap();
        }

        repo.delete_sessions_by_channel(first_plugin.channel_plugin_id.as_str())
            .await
            .unwrap();

        let remaining = repo.get_all_sessions().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].channel_plugin_id,
            Some(second_plugin.channel_plugin_id.clone())
        );
        let prompt_states: Vec<(String, String, Option<i64>)> = sqlx::query_as(
            "SELECT prompt_id, state, settled_at FROM channel_pending_prompts ORDER BY prompt_id",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(prompt_states[0].1, "cancelled");
        assert!(prompt_states[0].2.is_some());
        assert_eq!(prompt_states[1].1, "delivered");
        assert_eq!(prompt_states[2].1, "queued");
    }

    // -- Pairing tests ------------------------------------------------

    #[tokio::test]
    async fn create_and_get_pairing() {
        let (repo, _db) = setup().await;
        let pairing = sample_pairing();
        let created = repo.create_pairing(&pairing).await.unwrap();

        let found = repo.get_pairing_by_code("123456").await.unwrap().unwrap();
        assert_eq!(found.code, created.code);
        assert_eq!(found.platform_user_id, "tg_99");
        assert_eq!(found.status, "pending");
    }

    #[tokio::test]
    async fn create_duplicate_pairing_returns_conflict() {
        let (repo, _db) = setup().await;
        repo.create_pairing(&sample_pairing()).await.unwrap();
        let err = repo.create_pairing(&sample_pairing()).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn get_pending_pairings_filters_by_status() {
        let (repo, _db) = setup().await;
        let p1 = sample_pairing();
        repo.create_pairing(&p1).await.unwrap();

        let p2 = NewChannelPairingCodeRow {
            code: "654321".into(),
            status: "approved".into(),
            ..sample_pairing()
        };
        repo.create_pairing(&p2).await.unwrap();

        let pending = repo.get_pending_pairings().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].code, "123456");
    }

    #[tokio::test]
    async fn get_pairing_by_code_not_found() {
        let (repo, _db) = setup().await;
        assert!(repo.get_pairing_by_code("000000").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_pairing_status_changes_status() {
        let (repo, _db) = setup().await;
        repo.create_pairing(&sample_pairing()).await.unwrap();

        repo.update_pairing_status("123456", "approved").await.unwrap();

        let found = repo.get_pairing_by_code("123456").await.unwrap().unwrap();
        assert_eq!(found.status, "approved");
    }

    #[tokio::test]
    async fn update_pairing_status_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.update_pairing_status("000000", "approved").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn cleanup_expired_pairings_marks_expired() {
        let (repo, _db) = setup().await;
        let now = nomifun_common::now_ms();

        // Create an already-expired pairing.
        let expired = NewChannelPairingCodeRow {
            code: "111111".into(),
            expires_at: now - 1000,
            ..sample_pairing()
        };
        repo.create_pairing(&expired).await.unwrap();

        // Create a still-valid pairing.
        let valid = NewChannelPairingCodeRow {
            code: "222222".into(),
            expires_at: now + 600_000,
            ..sample_pairing()
        };
        repo.create_pairing(&valid).await.unwrap();

        let cleaned = repo.cleanup_expired_pairings(now).await.unwrap();
        assert_eq!(cleaned, 1);

        let found_expired = repo.get_pairing_by_code("111111").await.unwrap().unwrap();
        assert_eq!(found_expired.status, "expired");

        let found_valid = repo.get_pairing_by_code("222222").await.unwrap().unwrap();
        assert_eq!(found_valid.status, "pending");
    }

    #[tokio::test]
    async fn cleanup_expired_pairings_skips_non_pending() {
        let (repo, _db) = setup().await;
        let now = nomifun_common::now_ms();

        // Create an expired pairing that is already approved.
        let approved = NewChannelPairingCodeRow {
            code: "333333".into(),
            expires_at: now - 1000,
            status: "approved".into(),
            ..sample_pairing()
        };
        repo.create_pairing(&approved).await.unwrap();

        let cleaned = repo.cleanup_expired_pairings(now).await.unwrap();
        assert_eq!(cleaned, 0);

        let found = repo.get_pairing_by_code("333333").await.unwrap().unwrap();
        assert_eq!(found.status, "approved");
    }
}
