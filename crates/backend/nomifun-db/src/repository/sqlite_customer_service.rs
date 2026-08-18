use nomifun_common::text_search::NoteQueryTerms;
use nomifun_common::{TimestampMs, validate_uuidv7};
use sqlx::{Row, SqlitePool};

use crate::error::DbError;
use crate::models::{
    CsAgentRow, CsAuditEventRow, CsChannelBindingRow, CsDialogueRow, CsMessageRow, CsNoteRow,
    NewCsAgentRow,
};
use crate::repository::customer_service::{
    CsDialogueKey, ICustomerServiceRepository, UpdateCsAgentParams,
};
use crate::repository::customer_service_search::{
    CsNoteSearchHit, fts_index_delete, fts_index_insert, list_note_topics, note_search_text,
    search_notes_hybrid,
};

const AGENT_COLUMNS: &str = "cs_agent_id, name, greeting, persona, service_policy, provider_id, \
     model, knowledge_base_ids, enabled, max_concurrent, audit_retention_days, created_at, updated_at";
const DIALOGUE_COLUMNS: &str = "cs_dialogue_id, cs_agent_id, channel_plugin_id, channel_user_id, \
     chat_id, state, created_at, last_activity";
const MESSAGE_COLUMNS: &str = "cs_message_id, cs_dialogue_id, role, content, created_at";
const NOTE_COLUMNS: &str =
    "cs_note_id, cs_agent_id, kind, content, aliases, enabled, created_at, updated_at";

fn canonical_id(kind: &str, value: &str) -> Result<(), DbError> {
    validate_uuidv7(value)
        .map(|_| ())
        .map_err(|error| DbError::Conflict(format!("invalid {kind} '{value}': {error}")))
}

#[derive(Clone, Debug)]
pub struct SqliteCustomerServiceRepository {
    pool: SqlitePool,
}

impl SqliteCustomerServiceRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ICustomerServiceRepository for SqliteCustomerServiceRepository {
    // ── cs_agents ────────────────────────────────────────────────────

    async fn create_agent(&self, row: &NewCsAgentRow) -> Result<CsAgentRow, DbError> {
        canonical_id("cs_agent_id", &row.cs_agent_id)?;
        let sql = format!(
            "INSERT INTO cs_agents ({AGENT_COLUMNS}) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING {AGENT_COLUMNS}"
        );
        let inserted = sqlx::query_as::<_, CsAgentRow>(&sql)
            .bind(&row.cs_agent_id)
            .bind(&row.name)
            .bind(&row.greeting)
            .bind(&row.persona)
            .bind(&row.service_policy)
            .bind(&row.provider_id)
            .bind(&row.model)
            .bind(&row.knowledge_base_ids)
            .bind(row.enabled)
            .bind(row.max_concurrent)
            .bind(row.audit_retention_days)
            .bind(row.created_at)
            .bind(row.updated_at)
            .fetch_one(&self.pool)
            .await?;
        Ok(inserted)
    }

    async fn get_agent(&self, cs_agent_id: &str) -> Result<Option<CsAgentRow>, DbError> {
        let sql = format!("SELECT {AGENT_COLUMNS} FROM cs_agents WHERE cs_agent_id = ?");
        Ok(sqlx::query_as::<_, CsAgentRow>(&sql)
            .bind(cs_agent_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    async fn list_agents(&self) -> Result<Vec<CsAgentRow>, DbError> {
        let sql = format!("SELECT {AGENT_COLUMNS} FROM cs_agents ORDER BY created_at DESC, id DESC");
        Ok(sqlx::query_as::<_, CsAgentRow>(&sql)
            .fetch_all(&self.pool)
            .await?)
    }

    async fn update_agent(
        &self,
        cs_agent_id: &str,
        params: &UpdateCsAgentParams,
        now: TimestampMs,
    ) -> Result<CsAgentRow, DbError> {
        if let Some(Some(provider_id)) = &params.provider_id {
            canonical_id("provider_id", provider_id)?;
        }
        let sql = format!(
            "UPDATE cs_agents SET \
                name = COALESCE(?, name), \
                greeting = COALESCE(?, greeting), \
                persona = COALESCE(?, persona), \
                service_policy = COALESCE(?, service_policy), \
                provider_id = CASE WHEN ? THEN ? ELSE provider_id END, \
                model = CASE WHEN ? THEN ? ELSE model END, \
                knowledge_base_ids = COALESCE(?, knowledge_base_ids), \
                enabled = COALESCE(?, enabled), \
                max_concurrent = COALESCE(?, max_concurrent), \
                audit_retention_days = COALESCE(?, audit_retention_days), \
                updated_at = ? \
             WHERE cs_agent_id = ? \
             RETURNING {AGENT_COLUMNS}"
        );
        let updated = sqlx::query_as::<_, CsAgentRow>(&sql)
            .bind(&params.name)
            .bind(&params.greeting)
            .bind(&params.persona)
            .bind(&params.service_policy)
            .bind(params.provider_id.is_some())
            .bind(params.provider_id.clone().flatten())
            .bind(params.model.is_some())
            .bind(params.model.clone().flatten())
            .bind(&params.knowledge_base_ids)
            .bind(params.enabled)
            .bind(params.max_concurrent)
            .bind(params.audit_retention_days)
            .bind(now)
            .bind(cs_agent_id)
            .fetch_optional(&self.pool)
            .await?;
        updated.ok_or_else(|| DbError::NotFound(format!("cs agent {cs_agent_id}")))
    }

    async fn delete_agent(&self, cs_agent_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE cs_agents SET updated_at = updated_at WHERE cs_agent_id = ?",
        )
        .bind(cs_agent_id)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("cs agent {cs_agent_id}")));
        }
        sqlx::query("DELETE FROM cs_channel_bindings WHERE cs_agent_id = ?")
            .bind(cs_agent_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM cs_messages WHERE cs_dialogue_id IN \
             (SELECT cs_dialogue_id FROM cs_dialogues WHERE cs_agent_id = ?)",
        )
        .bind(cs_agent_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM cs_dialogues WHERE cs_agent_id = ?")
            .bind(cs_agent_id)
            .execute(&mut *tx)
            .await?;
        // Private notes cascade; shared notes (NULL owner) survive.
        sqlx::query("DELETE FROM cs_notes WHERE cs_agent_id = ?")
            .bind(cs_agent_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cs_agents WHERE cs_agent_id = ?")
            .bind(cs_agent_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // ── cs_channel_bindings ──────────────────────────────────────────

    async fn replace_agent_bindings(
        &self,
        cs_agent_id: &str,
        channel_plugin_ids: &[String],
        now: TimestampMs,
    ) -> Result<Vec<CsChannelBindingRow>, DbError> {
        canonical_id("cs_agent_id", cs_agent_id)?;
        for plugin_id in channel_plugin_ids {
            canonical_id("channel_plugin_id", plugin_id)?;
        }
        let mut tx = self.pool.begin().await?;
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cs_agents WHERE cs_agent_id = ?")
                .bind(cs_agent_id)
                .fetch_one(&mut *tx)
                .await?;
        if exists == 0 {
            return Err(DbError::NotFound(format!("cs agent {cs_agent_id}")));
        }
        sqlx::query("DELETE FROM cs_channel_bindings WHERE cs_agent_id = ?")
            .bind(cs_agent_id)
            .execute(&mut *tx)
            .await?;
        for plugin_id in channel_plugin_ids {
            // A plugin listed here is stolen from any other agent: the UNIQUE
            // index on channel_plugin_id is the authority (同 bot 重绑替换).
            sqlx::query("DELETE FROM cs_channel_bindings WHERE channel_plugin_id = ?")
                .bind(plugin_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO cs_channel_bindings (cs_agent_id, channel_plugin_id, created_at) \
                 VALUES (?, ?, ?)",
            )
            .bind(cs_agent_id)
            .bind(plugin_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        let rows = sqlx::query_as::<_, CsChannelBindingRow>(
            "SELECT cs_agent_id, channel_plugin_id, created_at \
             FROM cs_channel_bindings WHERE cs_agent_id = ? ORDER BY id",
        )
        .bind(cs_agent_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows)
    }

    async fn list_agent_bindings(
        &self,
        cs_agent_id: &str,
    ) -> Result<Vec<CsChannelBindingRow>, DbError> {
        Ok(sqlx::query_as::<_, CsChannelBindingRow>(
            "SELECT cs_agent_id, channel_plugin_id, created_at \
             FROM cs_channel_bindings WHERE cs_agent_id = ? ORDER BY id",
        )
        .bind(cs_agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn binding_for_plugin(
        &self,
        channel_plugin_id: &str,
    ) -> Result<Option<CsChannelBindingRow>, DbError> {
        Ok(sqlx::query_as::<_, CsChannelBindingRow>(
            "SELECT cs_agent_id, channel_plugin_id, created_at \
             FROM cs_channel_bindings WHERE channel_plugin_id = ?",
        )
        .bind(channel_plugin_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    // ── cs_dialogues / cs_messages ───────────────────────────────────

    async fn get_or_create_dialogue(
        &self,
        cs_agent_id: &str,
        key: &CsDialogueKey,
        now: TimestampMs,
    ) -> Result<CsDialogueRow, DbError> {
        canonical_id("cs_agent_id", cs_agent_id)?;
        canonical_id("channel_plugin_id", &key.channel_plugin_id)?;
        canonical_id("channel_user_id", &key.channel_user_id)?;
        let cs_dialogue_id = nomifun_common::generate_id();
        // Upsert on the identity triple: a replayed visitor keeps the lane,
        // gets a fresh last_activity, and follows the bot's CURRENT agent.
        let sql = format!(
            "INSERT INTO cs_dialogues \
                 (cs_dialogue_id, cs_agent_id, channel_plugin_id, channel_user_id, chat_id, \
                  state, created_at, last_activity) \
             VALUES (?, ?, ?, ?, ?, 'open', ?, ?) \
             ON CONFLICT(channel_plugin_id, channel_user_id, chat_id) DO UPDATE SET \
                 cs_agent_id = excluded.cs_agent_id, \
                 state = 'open', \
                 last_activity = excluded.last_activity \
             RETURNING {DIALOGUE_COLUMNS}"
        );
        Ok(sqlx::query_as::<_, CsDialogueRow>(&sql)
            .bind(&cs_dialogue_id)
            .bind(cs_agent_id)
            .bind(&key.channel_plugin_id)
            .bind(&key.channel_user_id)
            .bind(&key.chat_id)
            .bind(now)
            .bind(now)
            .fetch_one(&self.pool)
            .await?)
    }

    async fn get_dialogue(&self, cs_dialogue_id: &str) -> Result<Option<CsDialogueRow>, DbError> {
        let sql = format!("SELECT {DIALOGUE_COLUMNS} FROM cs_dialogues WHERE cs_dialogue_id = ?");
        Ok(sqlx::query_as::<_, CsDialogueRow>(&sql)
            .bind(cs_dialogue_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    async fn list_dialogues(&self, cs_agent_id: &str) -> Result<Vec<CsDialogueRow>, DbError> {
        let sql = format!(
            "SELECT {DIALOGUE_COLUMNS} FROM cs_dialogues \
             WHERE cs_agent_id = ? ORDER BY last_activity DESC, id DESC"
        );
        Ok(sqlx::query_as::<_, CsDialogueRow>(&sql)
            .bind(cs_agent_id)
            .fetch_all(&self.pool)
            .await?)
    }

    async fn append_message(
        &self,
        cs_dialogue_id: &str,
        role: &str,
        content: &str,
        now: TimestampMs,
    ) -> Result<CsMessageRow, DbError> {
        canonical_id("cs_dialogue_id", cs_dialogue_id)?;
        let cs_message_id = nomifun_common::generate_id();
        let mut tx = self.pool.begin().await?;
        let touched = sqlx::query(
            "UPDATE cs_dialogues SET last_activity = ? WHERE cs_dialogue_id = ?",
        )
        .bind(now)
        .bind(cs_dialogue_id)
        .execute(&mut *tx)
        .await?;
        if touched.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("cs dialogue {cs_dialogue_id}")));
        }
        let sql = format!(
            "INSERT INTO cs_messages (cs_message_id, cs_dialogue_id, role, content, created_at) \
             VALUES (?, ?, ?, ?, ?) RETURNING {MESSAGE_COLUMNS}"
        );
        let inserted = sqlx::query_as::<_, CsMessageRow>(&sql)
            .bind(&cs_message_id)
            .bind(cs_dialogue_id)
            .bind(role)
            .bind(content)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(inserted)
    }

    async fn recent_messages(
        &self,
        cs_dialogue_id: &str,
        limit: usize,
        char_budget: usize,
    ) -> Result<Vec<CsMessageRow>, DbError> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM cs_messages \
             WHERE cs_dialogue_id = ? ORDER BY id DESC LIMIT ?"
        );
        let newest_first = sqlx::query_as::<_, CsMessageRow>(&sql)
            .bind(cs_dialogue_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        // Newest messages win the budget; then restore chronological order.
        let mut kept: Vec<CsMessageRow> = Vec::with_capacity(newest_first.len());
        let mut used: usize = 0;
        for message in newest_first {
            let cost = message.content.chars().count();
            if !kept.is_empty() && used + cost > char_budget {
                break;
            }
            used += cost;
            kept.push(message);
        }
        kept.reverse();
        Ok(kept)
    }

    async fn list_messages(&self, cs_dialogue_id: &str) -> Result<Vec<CsMessageRow>, DbError> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM cs_messages WHERE cs_dialogue_id = ? ORDER BY id"
        );
        Ok(sqlx::query_as::<_, CsMessageRow>(&sql)
            .bind(cs_dialogue_id)
            .fetch_all(&self.pool)
            .await?)
    }

    // ── cs_notes ─────────────────────────────────────────────────────

    async fn create_note(&self, row: &CsNoteRow) -> Result<CsNoteRow, DbError> {
        canonical_id("cs_note_id", &row.cs_note_id)?;
        if let Some(agent_id) = &row.cs_agent_id {
            canonical_id("cs_agent_id", agent_id)?;
        }
        // Row insert and index insert share one transaction: a half-applied
        // write would leave a note that exists but cannot be found, which is
        // the exact failure mode this whole change exists to remove.
        let search_text = note_search_text(&row.content, &row.aliases);
        let mut tx = self.pool.begin().await?;
        let sql = format!(
            "INSERT INTO cs_notes ({NOTE_COLUMNS}, search_text) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id, {NOTE_COLUMNS}"
        );
        let inserted = sqlx::query(&sql)
            .bind(&row.cs_note_id)
            .bind(&row.cs_agent_id)
            .bind(&row.kind)
            .bind(&row.content)
            .bind(&row.aliases)
            .bind(row.enabled)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(&search_text)
            .fetch_one(&mut *tx)
            .await?;
        let rowid: i64 = inserted.try_get("id")?;
        fts_index_insert(&mut tx, rowid, &search_text).await?;
        tx.commit().await?;
        Ok(CsNoteRow {
            cs_note_id: inserted.try_get("cs_note_id")?,
            cs_agent_id: inserted.try_get("cs_agent_id")?,
            kind: inserted.try_get("kind")?,
            content: inserted.try_get("content")?,
            aliases: inserted.try_get("aliases")?,
            enabled: inserted.try_get("enabled")?,
            created_at: inserted.try_get("created_at")?,
            updated_at: inserted.try_get("updated_at")?,
        })
    }

    async fn list_notes(&self, cs_agent_id: Option<&str>) -> Result<Vec<CsNoteRow>, DbError> {
        let rows = match cs_agent_id {
            Some(agent_id) => {
                let sql = format!(
                    "SELECT {NOTE_COLUMNS} FROM cs_notes \
                     WHERE cs_agent_id = ? OR cs_agent_id IS NULL \
                     ORDER BY created_at DESC, id DESC"
                );
                sqlx::query_as::<_, CsNoteRow>(&sql)
                    .bind(agent_id)
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                let sql = format!(
                    "SELECT {NOTE_COLUMNS} FROM cs_notes ORDER BY created_at DESC, id DESC"
                );
                sqlx::query_as::<_, CsNoteRow>(&sql).fetch_all(&self.pool).await?
            }
        };
        Ok(rows)
    }

    async fn search_notes(
        &self,
        cs_agent_id: &str,
        terms: &NoteQueryTerms,
        limit: usize,
    ) -> Result<Vec<CsNoteSearchHit>, DbError> {
        search_notes_hybrid(&self.pool, cs_agent_id, terms, limit).await
    }

    async fn note_topics(&self, cs_agent_id: &str, limit: usize) -> Result<Vec<String>, DbError> {
        list_note_topics(&self.pool, cs_agent_id, limit).await
    }

    async fn update_note(
        &self,
        cs_note_id: &str,
        kind: Option<&str>,
        content: Option<&str>,
        aliases: Option<&str>,
        enabled: Option<bool>,
        now: TimestampMs,
    ) -> Result<CsNoteRow, DbError> {
        let mut tx = self.pool.begin().await?;

        // Read the CURRENT row first. The fts5 'delete' command must be given
        // the value that was originally indexed; handing it the new value
        // raises "database disk image is malformed" while a later
        // 'integrity-check' still reports PASSED, so the note silently drops
        // out of the index. Read old -> delete(old) -> update -> insert(new),
        // in this order, inside one transaction.
        let existing = sqlx::query("SELECT id, content, aliases, search_text FROM cs_notes WHERE cs_note_id = ?")
            .bind(cs_note_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("cs note {cs_note_id}")))?;
        let rowid: i64 = existing.try_get("id")?;
        let old_search_text: String = existing.try_get("search_text")?;
        let old_content: String = existing.try_get("content")?;
        let old_aliases: String = existing.try_get("aliases")?;

        let new_content = content.unwrap_or(&old_content);
        let new_aliases = aliases.unwrap_or(&old_aliases);
        let new_search_text = note_search_text(new_content, new_aliases);

        fts_index_delete(&mut tx, rowid, &old_search_text).await?;
        let sql = format!(
            "UPDATE cs_notes SET \
                kind = COALESCE(?, kind), \
                content = COALESCE(?, content), \
                aliases = COALESCE(?, aliases), \
                enabled = COALESCE(?, enabled), \
                search_text = ?, \
                updated_at = ? \
             WHERE cs_note_id = ? RETURNING {NOTE_COLUMNS}"
        );
        let updated = sqlx::query_as::<_, CsNoteRow>(&sql)
            .bind(kind)
            .bind(content)
            .bind(aliases)
            .bind(enabled)
            .bind(&new_search_text)
            .bind(now)
            .bind(cs_note_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("cs note {cs_note_id}")))?;
        fts_index_insert(&mut tx, rowid, &new_search_text).await?;
        tx.commit().await?;
        Ok(updated)
    }

    async fn delete_note(&self, cs_note_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        // Same contract as update: de-index with the OLD value before the row
        // disappears, otherwise the index keeps a phantom entry pointing at a
        // rowid that no longer exists.
        let existing = sqlx::query("SELECT id, search_text FROM cs_notes WHERE cs_note_id = ?")
            .bind(cs_note_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("cs note {cs_note_id}")))?;
        let rowid: i64 = existing.try_get("id")?;
        let old_search_text: String = existing.try_get("search_text")?;
        fts_index_delete(&mut tx, rowid, &old_search_text).await?;
        sqlx::query("DELETE FROM cs_notes WHERE cs_note_id = ?")
            .bind(cs_note_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // ── cs_audit_events ──────────────────────────────────────────────

    async fn insert_audit_event(&self, row: &CsAuditEventRow) -> Result<(), DbError> {
        canonical_id("cs_agent_id", &row.cs_agent_id)?;
        sqlx::query(
            "INSERT INTO cs_audit_events (cs_agent_id, kind, platform, detail, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&row.cs_agent_id)
        .bind(&row.kind)
        .bind(&row.platform)
        .bind(&row.detail)
        .bind(row.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_audit_events(
        &self,
        cs_agent_id: &str,
        limit: usize,
    ) -> Result<Vec<CsAuditEventRow>, DbError> {
        Ok(sqlx::query_as::<_, CsAuditEventRow>(
            "SELECT cs_agent_id, kind, platform, detail, created_at \
             FROM cs_audit_events WHERE cs_agent_id = ? \
             ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(cs_agent_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn cleanup_audit_events(&self, now: TimestampMs) -> Result<u64, DbError> {
        const DAY_MS: i64 = 24 * 60 * 60 * 1000;
        let result = sqlx::query(
            "DELETE FROM cs_audit_events WHERE id IN (\
                 SELECT event.id FROM cs_audit_events event \
                 JOIN cs_agents agent ON agent.cs_agent_id = event.cs_agent_id \
                 WHERE event.created_at < ? - agent.audit_retention_days * ?\
             )",
        )
        .bind(now)
        .bind(DAY_MS)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;
    use nomifun_common::text_search::expand_query;
    use nomifun_common::{ChannelPluginId, ChannelUserId, generate_id};

    async fn repo() -> (crate::Database, SqliteCustomerServiceRepository) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteCustomerServiceRepository::new(db.pool().clone());
        (db, repo)
    }

    fn new_agent(name: &str) -> NewCsAgentRow {
        NewCsAgentRow {
            cs_agent_id: generate_id(),
            name: name.into(),
            greeting: "您好，我是客服".into(),
            persona: "耐心友好".into(),
            service_policy: "只回答业务问题".into(),
            provider_id: None,
            model: Some("model-a".into()),
            knowledge_base_ids: "[]".into(),
            enabled: true,
            max_concurrent: 8,
            audit_retention_days: 30,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn dialogue_key() -> CsDialogueKey {
        CsDialogueKey {
            channel_plugin_id: ChannelPluginId::new().into_string(),
            channel_user_id: ChannelUserId::new().into_string(),
            chat_id: "chat-1".into(),
        }
    }

    #[tokio::test]
    async fn agent_crud_roundtrip() {
        let (_db, repo) = repo().await;
        let created = repo.create_agent(&new_agent("小助")).await.unwrap();
        assert_eq!(created.name, "小助");
        assert_eq!(created.max_concurrent, 8);

        let fetched = repo.get_agent(&created.cs_agent_id).await.unwrap().unwrap();
        assert_eq!(fetched.cs_agent_id, created.cs_agent_id);

        let updated = repo
            .update_agent(
                &created.cs_agent_id,
                &UpdateCsAgentParams {
                    name: Some("小助2".into()),
                    enabled: Some(false),
                    model: Some(None),
                    ..Default::default()
                },
                7,
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "小助2");
        assert!(!updated.enabled);
        assert_eq!(updated.model, None);
        assert_eq!(updated.greeting, created.greeting, "unspecified fields keep values");
        assert_eq!(updated.updated_at, 7);

        assert_eq!(repo.list_agents().await.unwrap().len(), 1);
        repo.delete_agent(&created.cs_agent_id).await.unwrap();
        assert!(repo.get_agent(&created.cs_agent_id).await.unwrap().is_none());
        assert!(matches!(
            repo.delete_agent(&created.cs_agent_id).await.unwrap_err(),
            DbError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn agent_rejects_invalid_business_id() {
        let (_db, repo) = repo().await;
        let mut row = new_agent("bad");
        row.cs_agent_id = "not-a-uuid".into();
        assert!(matches!(
            repo.create_agent(&row).await.unwrap_err(),
            DbError::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn binding_replace_is_full_put_and_steals_plugins() {
        let (_db, repo) = repo().await;
        let agent_a = repo.create_agent(&new_agent("A")).await.unwrap();
        let agent_b = repo.create_agent(&new_agent("B")).await.unwrap();
        let plugin_1 = ChannelPluginId::new().into_string();
        let plugin_2 = ChannelPluginId::new().into_string();

        let rows = repo
            .replace_agent_bindings(&agent_a.cs_agent_id, &[plugin_1.clone(), plugin_2.clone()], 1)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);

        // Rebinding plugin_1 to B steals it from A (同 bot 重绑替换).
        repo.replace_agent_bindings(&agent_b.cs_agent_id, std::slice::from_ref(&plugin_1), 2)
            .await
            .unwrap();
        let owner = repo.binding_for_plugin(&plugin_1).await.unwrap().unwrap();
        assert_eq!(owner.cs_agent_id, agent_b.cs_agent_id);
        let a_rows = repo.list_agent_bindings(&agent_a.cs_agent_id).await.unwrap();
        assert_eq!(a_rows.len(), 1);
        assert_eq!(a_rows[0].channel_plugin_id, plugin_2);

        // Empty PUT clears the set.
        repo.replace_agent_bindings(&agent_a.cs_agent_id, &[], 3).await.unwrap();
        assert!(repo.list_agent_bindings(&agent_a.cs_agent_id).await.unwrap().is_empty());
        assert!(repo.binding_for_plugin(&plugin_2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn dialogue_get_or_create_upserts_on_identity_triple() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();
        let other = repo.create_agent(&new_agent("B")).await.unwrap();
        let key = dialogue_key();

        let first = repo.get_or_create_dialogue(&agent.cs_agent_id, &key, 10).await.unwrap();
        let second = repo.get_or_create_dialogue(&agent.cs_agent_id, &key, 20).await.unwrap();
        assert_eq!(first.cs_dialogue_id, second.cs_dialogue_id, "same lane");
        assert_eq!(second.last_activity, 20);
        assert_eq!(second.created_at, 10, "created_at is immutable");

        // Bot rebound to another agent: the lane follows the current agent.
        let third = repo.get_or_create_dialogue(&other.cs_agent_id, &key, 30).await.unwrap();
        assert_eq!(third.cs_dialogue_id, first.cs_dialogue_id);
        assert_eq!(third.cs_agent_id, other.cs_agent_id);

        // A different chat in the same plugin gets its own lane.
        let mut other_chat = key.clone();
        other_chat.chat_id = "chat-2".into();
        let lane2 = repo.get_or_create_dialogue(&agent.cs_agent_id, &other_chat, 40).await.unwrap();
        assert_ne!(lane2.cs_dialogue_id, first.cs_dialogue_id);
    }

    #[tokio::test]
    async fn messages_append_window_and_budget() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();
        let dialogue = repo
            .get_or_create_dialogue(&agent.cs_agent_id, &dialogue_key(), 1)
            .await
            .unwrap();

        for index in 0..5 {
            repo.append_message(
                &dialogue.cs_dialogue_id,
                if index % 2 == 0 { "visitor" } else { "agent" },
                &format!("msg-{index}"),
                10 + index,
            )
            .await
            .unwrap();
        }

        let all = repo.list_messages(&dialogue.cs_dialogue_id).await.unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].content, "msg-0");
        assert_eq!(all[4].content, "msg-4");

        // Limit window: newest 3, chronological order.
        let window = repo.recent_messages(&dialogue.cs_dialogue_id, 3, 10_000).await.unwrap();
        assert_eq!(
            window.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(),
            vec!["msg-2", "msg-3", "msg-4"]
        );

        // Char budget: each message is 5 chars; budget 11 keeps newest two
        // (the newest always survives even under a tiny budget).
        let tight = repo.recent_messages(&dialogue.cs_dialogue_id, 30, 11).await.unwrap();
        assert_eq!(
            tight.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(),
            vec!["msg-3", "msg-4"]
        );
        let tiny = repo.recent_messages(&dialogue.cs_dialogue_id, 30, 1).await.unwrap();
        assert_eq!(tiny.len(), 1, "newest message always survives");

        // Appending bumps last_activity.
        let refreshed = repo.get_dialogue(&dialogue.cs_dialogue_id).await.unwrap().unwrap();
        assert_eq!(refreshed.last_activity, 14);

        // Unknown dialogue → NotFound.
        assert!(matches!(
            repo.append_message(&generate_id(), "visitor", "x", 1).await.unwrap_err(),
            DbError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn notes_scope_shared_plus_private_and_search() {
        let (_db, repo) = repo().await;
        let agent_a = repo.create_agent(&new_agent("A")).await.unwrap();
        let agent_b = repo.create_agent(&new_agent("B")).await.unwrap();

        let make_note = |owner: Option<String>, content: &str, enabled: bool| CsNoteRow {
            cs_note_id: generate_id(),
            cs_agent_id: owner,
            kind: "faq".into(),
            content: content.into(),
            aliases: String::new(),
            enabled,
            created_at: 1,
            updated_at: 1,
        };
        repo.create_note(&make_note(None, "shared 退货政策", true)).await.unwrap();
        repo.create_note(&make_note(Some(agent_a.cs_agent_id.clone()), "A 私有 发货时间", true))
            .await
            .unwrap();
        repo.create_note(&make_note(Some(agent_b.cs_agent_id.clone()), "B 私有 发货时间", true))
            .await
            .unwrap();
        repo.create_note(&make_note(Some(agent_a.cs_agent_id.clone()), "A 停用 发货时间", false))
            .await
            .unwrap();

        // Visible scope = shared + own private.
        let visible = repo.list_notes(Some(&agent_a.cs_agent_id)).await.unwrap();
        assert_eq!(visible.len(), 3);
        assert!(visible.iter().all(|note| note.cs_agent_id.as_deref() != Some(agent_b.cs_agent_id.as_str())));
        assert_eq!(repo.list_notes(None).await.unwrap().len(), 4);

        // Search: enabled only, scope-filtered. The disabled note and the other
        // agent's note must stay invisible — losing either filter while
        // swapping the index would surface retired or foreign answers.
        let hits = repo
            .search_notes(&agent_a.cs_agent_id, &expand_query("发货时间"), 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].note.content, "A 私有 发货时间");
        let shared_hits = repo
            .search_notes(&agent_b.cs_agent_id, &expand_query("退货政策"), 10)
            .await
            .unwrap();
        assert_eq!(shared_hits.len(), 1);

        // FTS5/LIKE metacharacters are literals, never wildcards or operators.
        // Unquoted, several of these raise SQLite syntax errors rather than
        // returning nothing, which would surface as a failed reply.
        for hostile in ["%", "_", "*", "\"", "AND", "OR", "(", "a-b", "NEAR(x y)"] {
            let result = repo
                .search_notes(&agent_a.cs_agent_id, &expand_query(hostile), 10)
                .await;
            assert!(result.is_ok(), "{hostile:?} must not error: {result:?}");
        }

        // Update and delete.
        let note = &repo.list_notes(Some(&agent_a.cs_agent_id)).await.unwrap()[0];
        let updated = repo
            .update_note(&note.cs_note_id, Some("policy"), None, None, Some(false), 9)
            .await
            .unwrap();
        assert_eq!(updated.kind, "policy");
        assert!(!updated.enabled);
        repo.delete_note(&note.cs_note_id).await.unwrap();
        assert!(matches!(
            repo.delete_note(&note.cs_note_id).await.unwrap_err(),
            DbError::NotFound(_)
        ));
    }

    /// The regression suite for the reported bug.
    ///
    /// Each row is a real visitor phrasing that must reach the FAQ. Under the
    /// old `content LIKE '%query%'` path the first four rows FAILED — a single
    /// inserted space or a rephrase broke contiguity — which is exactly the
    /// defect this table now pins.
    #[tokio::test]
    async fn visitor_phrasings_reach_the_expected_note() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();
        let owner = Some(agent.cs_agent_id.clone());

        let note = |content: &str, aliases: &str| CsNoteRow {
            cs_note_id: generate_id(),
            cs_agent_id: owner.clone(),
            kind: "faq".into(),
            content: content.into(),
            aliases: aliases.into(),
            enabled: true,
            created_at: 1,
            updated_at: 1,
        };
        // Aliases carry the synonym cases: pure lexical matching cannot bridge
        // a paraphrase that shares no vocabulary with the note.
        let intro = repo
            .create_note(&note(
                "Q：NomiFun是什么？\nA：NomiFun 是本地优先的开源 AI 工作空间。",
                "这个软件\n产品简介",
            ))
            .await
            .unwrap();
        let install = repo
            .create_note(&note("Q：怎么安装？\nA：下载安装包后双击运行。", ""))
            .await
            .unwrap();
        let price = repo
            .create_note(&note("Q：收费吗？\nA：开源免费，MIT 协议。", "要钱\n价格\n多少钱"))
            .await
            .unwrap();

        let cases: &[(&str, Option<&str>)] = &[
            // The four originally reported failures.
            ("@xxx  NomiFun是什么", Some(&intro.cs_note_id)),
            ("@xxx  NomiFun 是什么", Some(&intro.cs_note_id)),
            ("@xxx  nomifun是什么", Some(&intro.cs_note_id)),
            ("@xxx 介绍一下 NomiFun", Some(&intro.cs_note_id)),
            // Case and full-width folding.
            ("@xxx NOMIFUN 能干什么？", Some(&intro.cs_note_id)),
            ("@xxx ＮomiFun是什么？", Some(&intro.cs_note_id)),
            // Paraphrase with no shared vocabulary — alias channel.
            ("@xxx 这个软件是干什么的", Some(&intro.cs_note_id)),
            // English query against a Chinese note.
            ("@xxx what is nomifun", Some(&intro.cs_note_id)),
            // Other notes, including particle stripping (免费吗 -> 免费).
            ("@xxx 怎么安装", Some(&install.cs_note_id)),
            ("@xxx 安装包在哪下载", Some(&install.cs_note_id)),
            ("@xxx 免费吗", Some(&price.cs_note_id)),
            ("@xxx 要钱吗", Some(&price.cs_note_id)),
            ("@xxx 多少钱？", Some(&price.cs_note_id)),
            // A true miss must stay a miss: recall must not become "everything".
            ("@xxx 完全无关的问题zzz", None),
            ("@xxx", None),
            ("@xxx 是什么", None),
        ];

        for (query, expected) in cases {
            let hits = repo
                .search_notes(&agent.cs_agent_id, &expand_query(query), 10)
                .await
                .unwrap();
            match expected {
                Some(note_id) => {
                    assert!(
                        hits.iter().any(|hit| hit.note.cs_note_id == **note_id),
                        "{query:?} must find the expected note, got {:?}",
                        hits.iter().map(|h| &h.note.content).collect::<Vec<_>>()
                    );
                }
                None => assert!(hits.is_empty(), "{query:?} must find nothing, got {hits:?}"),
            }
        }
    }

    /// The index is maintained by hand (v3 forbids triggers), and the fts5
    /// `'delete'` command corrupts the index SILENTLY when handed the wrong
    /// value — a later `'integrity-check'` still passes. So the write paths
    /// keeping the index in step is itself the invariant worth pinning: a drift
    /// here reproduces the original "note exists but cannot be found" bug.
    #[tokio::test]
    async fn write_paths_keep_the_full_text_index_in_step() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();
        let created = repo
            .create_note(&CsNoteRow {
                cs_note_id: generate_id(),
                cs_agent_id: Some(agent.cs_agent_id.clone()),
                kind: "faq".into(),
                content: "Q：退款要多久？\nA：三个工作日内到账。".into(),
                aliases: String::new(),
                enabled: true,
                created_at: 1,
                updated_at: 1,
            })
            .await
            .unwrap();

        let find = |needle: &'static str, agent_id: String| {
            let repo = repo.clone();
            async move {
                repo.search_notes(&agent_id, &expand_query(needle), 10).await.unwrap().len()
            }
        };

        assert_eq!(find("退款要多久", agent.cs_agent_id.clone()).await, 1, "create must index");

        // After an edit the old text must be gone and the new text findable. If
        // 'delete' had been given the new value instead of the old one, the row
        // would vanish from the index entirely and BOTH counts would be 0.
        repo.update_note(
            &created.cs_note_id,
            None,
            Some("Q：换货怎么处理？\nA：联系客服登记。"),
            None,
            None,
            2,
        )
        .await
        .unwrap();
        assert_eq!(find("退款要多久", agent.cs_agent_id.clone()).await, 0, "stale text must be de-indexed");
        assert_eq!(find("换货怎么处理", agent.cs_agent_id.clone()).await, 1, "new text must be indexed");

        // Adding an alias makes a previously-unmatchable phrasing reachable
        // without touching the note body.
        repo.update_note(&created.cs_note_id, None, None, Some("退换\n售后"), None, 3)
            .await
            .unwrap();
        assert_eq!(find("售后", agent.cs_agent_id.clone()).await, 1, "alias must be searchable");
        assert_eq!(find("换货怎么处理", agent.cs_agent_id.clone()).await, 1, "body still indexed");

        // Delete removes it from the index, not just the table.
        repo.delete_note(&created.cs_note_id).await.unwrap();
        assert_eq!(find("换货怎么处理", agent.cs_agent_id.clone()).await, 0, "delete must de-index");
    }

    /// A disabled note must be invisible to search even though it is indexed,
    /// and re-enabling must restore it. The `enabled` filter lives in the
    /// retrieval SQL rather than the index, so it is the easiest guard to lose.
    #[tokio::test]
    async fn disabled_notes_are_never_searchable() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();
        let created = repo
            .create_note(&CsNoteRow {
                cs_note_id: generate_id(),
                cs_agent_id: Some(agent.cs_agent_id.clone()),
                kind: "faq".into(),
                content: "Q：营业时间？\nA：全年无休。".into(),
                aliases: String::new(),
                enabled: true,
                created_at: 1,
                updated_at: 1,
            })
            .await
            .unwrap();
        let count = |agent_id: String| {
            let repo = repo.clone();
            async move {
                repo.search_notes(&agent_id, &expand_query("营业时间"), 10).await.unwrap().len()
            }
        };
        assert_eq!(count(agent.cs_agent_id.clone()).await, 1);

        repo.update_note(&created.cs_note_id, None, None, None, Some(false), 2).await.unwrap();
        assert_eq!(count(agent.cs_agent_id.clone()).await, 0, "disabled must not surface");

        repo.update_note(&created.cs_note_id, None, None, None, Some(true), 3).await.unwrap();
        assert_eq!(count(agent.cs_agent_id.clone()).await, 1, "re-enabling restores it");
    }

    /// The backfill is what makes migration 035 safe for existing notes: rows
    /// created before it have an empty `search_text` and are absent from the
    /// index, so without it every pre-existing note would become unfindable —
    /// turning a recall bug into a total recall outage.
    #[tokio::test]
    async fn backfill_indexes_rows_written_before_the_index_existed() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();

        // Simulate a pre-migration row: inserted directly, so neither
        // `search_text` nor the index knows about it.
        sqlx::query(
            "INSERT INTO cs_notes (cs_note_id, cs_agent_id, kind, content, aliases, enabled, created_at, updated_at, search_text) \
             VALUES (?, ?, 'faq', ?, '', 1, 1, 1, '')",
        )
        .bind(generate_id())
        .bind(&agent.cs_agent_id)
        .bind("Q：ＮomiFun是什么？\nA：本地优先的开源工作空间。")
        .execute(&repo.pool)
        .await
        .unwrap();

        let terms = expand_query("nomifun是什么");
        assert!(
            repo.search_notes(&agent.cs_agent_id, &terms, 10).await.unwrap().is_empty(),
            "a pre-migration row starts out unindexed"
        );

        let rewritten = crate::repository::customer_service_search::backfill_note_search_text(&repo.pool)
            .await
            .unwrap();
        assert_eq!(rewritten, 1);
        assert_eq!(
            repo.search_notes(&agent.cs_agent_id, &terms, 10).await.unwrap().len(),
            1,
            "backfill must make it findable, including the full-width variant"
        );

        // Idempotent: a second run rewrites nothing.
        let again = crate::repository::customer_service_search::backfill_note_search_text(&repo.pool)
            .await
            .unwrap();
        assert_eq!(again, 0);
    }

    /// Topics back the "nothing matched, but here is what exists" reply, so the
    /// model can re-query instead of telling the visitor there is no answer.
    #[tokio::test]
    async fn note_topics_summarize_visible_notes() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();
        for (content, enabled) in [
            ("Q：怎么退货？\nA：七天无理由。", true),
            ("Q：怎么换货？\nA：联系客服。", true),
            ("Q：已下架的问题？\nA：无。", false),
        ] {
            repo.create_note(&CsNoteRow {
                cs_note_id: generate_id(),
                cs_agent_id: Some(agent.cs_agent_id.clone()),
                kind: "faq".into(),
                content: content.into(),
                aliases: String::new(),
                enabled,
                created_at: 1,
                updated_at: 1,
            })
            .await
            .unwrap();
        }
        let topics = repo.note_topics(&agent.cs_agent_id, 10).await.unwrap();
        assert!(topics.iter().any(|t| t == "怎么退货？"), "{topics:?}");
        assert!(topics.iter().any(|t| t == "怎么换货？"), "{topics:?}");
        assert!(
            !topics.iter().any(|t| t.contains("已下架")),
            "disabled notes must not be advertised: {topics:?}"
        );
    }

    #[tokio::test]
    async fn delete_agent_cascades_own_rows_keeps_shared_notes() {
        let (_db, repo) = repo().await;
        let agent = repo.create_agent(&new_agent("A")).await.unwrap();
        let plugin = ChannelPluginId::new().into_string();
        repo.replace_agent_bindings(&agent.cs_agent_id, std::slice::from_ref(&plugin), 1)
            .await
            .unwrap();
        let dialogue = repo
            .get_or_create_dialogue(&agent.cs_agent_id, &dialogue_key(), 1)
            .await
            .unwrap();
        repo.append_message(&dialogue.cs_dialogue_id, "visitor", "hi", 2).await.unwrap();
        repo.create_note(&CsNoteRow {
            cs_note_id: generate_id(),
            cs_agent_id: Some(agent.cs_agent_id.clone()),
            kind: "faq".into(),
            content: "private".into(),
            aliases: String::new(),
            enabled: true,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
        repo.create_note(&CsNoteRow {
            cs_note_id: generate_id(),
            cs_agent_id: None,
            kind: "faq".into(),
            content: "shared".into(),
            aliases: String::new(),
            enabled: true,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();

        repo.delete_agent(&agent.cs_agent_id).await.unwrap();
        assert!(repo.binding_for_plugin(&plugin).await.unwrap().is_none());
        assert!(repo.get_dialogue(&dialogue.cs_dialogue_id).await.unwrap().is_none());
        assert!(repo.list_messages(&dialogue.cs_dialogue_id).await.unwrap().is_empty());
        let remaining = repo.list_notes(None).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].content, "shared");
    }

    #[tokio::test]
    async fn audit_insert_list_and_retention_cleanup() {
        let (_db, repo) = repo().await;
        let mut short_lived = new_agent("short");
        short_lived.audit_retention_days = 1;
        let agent = repo.create_agent(&short_lived).await.unwrap();

        const DAY_MS: i64 = 24 * 60 * 60 * 1000;
        let now = 10 * DAY_MS;
        for (kind, at) in [("turn", now - 3 * DAY_MS), ("turn", now - 1), ("turn_error", now)] {
            repo.insert_audit_event(&CsAuditEventRow {
                cs_agent_id: agent.cs_agent_id.clone(),
                kind: kind.into(),
                platform: "telegram".into(),
                detail: "d".into(),
                created_at: at,
            })
            .await
            .unwrap();
        }
        assert_eq!(repo.list_audit_events(&agent.cs_agent_id, 10).await.unwrap().len(), 3);

        let removed = repo.cleanup_audit_events(now).await.unwrap();
        assert_eq!(removed, 1, "only the 3-day-old event exceeds 1-day retention");
        let events = repo.list_audit_events(&agent.cs_agent_id, 10).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "turn_error", "newest first");
    }
}
