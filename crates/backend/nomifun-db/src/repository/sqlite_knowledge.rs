use nomifun_common::{KnowledgeBaseId, KnowledgeBindingId};
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{CreateKnowledgeTagParams, KnowledgeBaseRow, KnowledgeBindingRow, KnowledgeTagRow, UpdateKnowledgeTagParams};
use crate::repository::knowledge::IKnowledgeRepository;

#[derive(Clone, Debug)]
pub struct SqliteKnowledgeRepository {
    pool: SqlitePool,
}

impl SqliteKnowledgeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}


/// Map a binding `target_kind` to the `knowledge_bindings` column that carries
/// its `target_id`. Returns `None` for an unrecognized kind so callers can
/// reject it without risking a write to the wrong column.
fn target_column(target_kind: &str) -> Option<&'static str> {
    match target_kind {
        "workpath" => Some("target_workpath"),
        "conversation" => Some("target_conv_id"),
        "terminal" => Some("target_term_id"),
        "companion" => Some("target_companion_id"),
        _ => None,
    }
}

#[async_trait::async_trait]
impl IKnowledgeRepository for SqliteKnowledgeRepository {
    async fn insert_base(&self, row: &KnowledgeBaseRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO knowledge_bases (\
                id, name, description, root_path, managed, extra, created_at, updated_at, tags\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.id.as_str())
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.root_path)
        .bind(row.managed)
        .bind(&row.extra)
        .bind(row.created_at)
        .bind(row.updated_at)
        .bind(&row.tags)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_base(&self, row: &KnowledgeBaseRow) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE knowledge_bases SET name = ?, description = ?, extra = ?, tags = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.extra)
        .bind(&row.tags)
        .bind(row.updated_at)
        .bind(row.id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("knowledge base {}", row.id)));
        }
        Ok(())
    }

    async fn delete_base(&self, id: &str) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM knowledge_bases WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("knowledge base {id}")));
        }
        Ok(())
    }

    async fn get_base(&self, id: &str) -> Result<Option<KnowledgeBaseRow>, DbError> {
        let row = sqlx::query_as::<_, KnowledgeBaseRow>("SELECT * FROM knowledge_bases WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list_bases(&self) -> Result<Vec<KnowledgeBaseRow>, DbError> {
        let rows = sqlx::query_as::<_, KnowledgeBaseRow>("SELECT * FROM knowledge_bases ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    async fn get_binding(
        &self,
        target_kind: &str,
        target_id: &str,
    ) -> Result<Option<(KnowledgeBindingRow, Vec<String>)>, DbError> {
        let Some(column) = target_column(target_kind) else {
            return Ok(None);
        };
        // The kind is fixed to a static column name above, never user input,
        // so this format! cannot inject. Also filter on target_kind so a stray
        // value in the wrong column can never satisfy the lookup.
        let sql = format!(
            "SELECT * FROM knowledge_bindings WHERE target_kind = ? AND {column} = ?"
        );
        let row = sqlx::query_as::<_, KnowledgeBindingRow>(&sql)
            .bind(target_kind)
            .bind(target_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let kb_ids = self.fetch_kb_ids(row.binding_id.as_str()).await?;
        Ok(Some((row, kb_ids)))
    }

    async fn set_binding(
        &self,
        target_kind: &str,
        target_id: &str,
        kb_ids: &[String],
        enabled: bool,
        writeback: bool,
        writeback_mode: &str,
        writeback_eagerness: &str,
        channel_write_enabled: bool,
        updated_at: nomifun_common::TimestampMs,
    ) -> Result<String, DbError> {
        let Some(column) = target_column(target_kind) else {
            return Err(DbError::NotFound(format!(
                "unknown knowledge binding kind {target_kind}"
            )));
        };

        let mut tx = self.pool.begin().await?;

        // 1. Upsert the main row. SELECT the existing binding_id via the typed
        //    target column (each kind has a partial UNIQUE index, so at most
        //    one row matches), then UPDATE or INSERT accordingly.
        let select_sql = format!(
            "SELECT binding_id FROM knowledge_bindings WHERE target_kind = ? AND {column} = ?"
        );
        let existing: Option<String> = sqlx::query_scalar(&select_sql)
            .bind(target_kind)
            .bind(target_id)
            .fetch_optional(&mut *tx)
            .await?;

        let binding_id = if let Some(binding_id) = existing {
            let binding_id = KnowledgeBindingId::parse(binding_id)
                .map_err(|error| DbError::Query(sqlx::Error::Decode(Box::new(error))))?;
            sqlx::query(
                "UPDATE knowledge_bindings \
                 SET enabled = ?, writeback = ?, writeback_mode = ?, writeback_eagerness = ?, \
                     channel_write_enabled = ?, updated_at = ? \
                 WHERE binding_id = ?",
            )
            .bind(enabled)
            .bind(writeback)
            .bind(writeback_mode)
            .bind(writeback_eagerness)
            .bind(channel_write_enabled)
            .bind(updated_at)
            .bind(binding_id.as_str())
            .execute(&mut *tx)
            .await?;
            binding_id
        } else {
            // The other three target columns stay NULL; the CHECK enforces
            // exactly-one-non-null matching target_kind.
            let insert_sql = format!(
                "INSERT INTO knowledge_bindings \
                    (binding_id, target_kind, {column}, enabled, writeback, writeback_mode, writeback_eagerness, \
                     channel_write_enabled, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
            );
            let binding_id = KnowledgeBindingId::new();
            sqlx::query(&insert_sql)
            .bind(binding_id.as_str())
            .bind(target_kind)
            .bind(target_id)
            .bind(enabled)
            .bind(writeback)
            .bind(writeback_mode)
            .bind(writeback_eagerness)
            .bind(channel_write_enabled)
            .bind(updated_at)
            .execute(&mut *tx)
            .await?;
            binding_id
        };

        // 2. Replace the junction rows for this binding, preserving kb_ids order.
        sqlx::query("DELETE FROM knowledge_binding_bases WHERE binding_id = ?")
            .bind(binding_id.as_str())
            .execute(&mut *tx)
            .await?;
        for (position, kb_id) in kb_ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO knowledge_binding_bases (binding_id, kb_id, position) VALUES (?, ?, ?)",
            )
            .bind(binding_id.as_str())
            .bind(kb_id)
            .bind(position as i64)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(binding_id.into_string())
    }

    async fn delete_binding(&self, target_kind: &str, target_id: &str) -> Result<(), DbError> {
        let Some(column) = target_column(target_kind) else {
            return Ok(());
        };
        // The junction rows are removed by FK CASCADE on knowledge_binding_bases.
        let sql = format!(
            "DELETE FROM knowledge_bindings WHERE target_kind = ? AND {column} = ?"
        );
        sqlx::query(&sql)
            .bind(target_kind)
            .bind(target_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_bindings_using_kb(&self, kb_id: &str) -> Result<Vec<KnowledgeBindingRow>, DbError> {
        let rows = sqlx::query_as::<_, KnowledgeBindingRow>(
            "SELECT b.* FROM knowledge_bindings b \
             JOIN knowledge_binding_bases j ON j.binding_id = b.binding_id \
             WHERE j.kb_id = ? \
             ORDER BY b.target_kind ASC, b.binding_id ASC",
        )
        .bind(kb_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Knowledge tags ────────────────────────────────────────────────────

    async fn list_knowledge_tags(&self) -> Result<Vec<KnowledgeTagRow>, DbError> {
        let rows = sqlx::query_as::<_, KnowledgeTagRow>(
            "SELECT * FROM knowledge_tags ORDER BY sort_order ASC, key ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn create_knowledge_tag(&self, params: CreateKnowledgeTagParams) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO knowledge_tags (key, label, color, sort_order, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&params.key)
        .bind(&params.label)
        .bind(&params.color)
        .bind(params.sort_order)
        .bind(params.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_knowledge_tag(&self, key: &str, params: UpdateKnowledgeTagParams) -> Result<(), DbError> {
        // Build a dynamic SET clause from the provided fields.
        let mut sets: Vec<&str> = Vec::new();
        if params.label.is_some() {
            sets.push("label = ?");
        }
        if params.color.is_some() {
            sets.push("color = ?");
        }
        if params.sort_order.is_some() {
            sets.push("sort_order = ?");
        }
        if sets.is_empty() {
            // Nothing to update; verify the key exists.
            let exists = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM knowledge_tags WHERE key = ?",
            )
            .bind(key)
            .fetch_one(&self.pool)
            .await?;
            if exists == 0 {
                return Err(DbError::NotFound(format!("knowledge tag {key}")));
            }
            return Ok(());
        }
        let sql = format!("UPDATE knowledge_tags SET {} WHERE key = ?", sets.join(", "));
        let mut query = sqlx::query(&sql);
        if let Some(ref label) = params.label {
            query = query.bind(label);
        }
        if let Some(ref color) = params.color {
            query = query.bind(color.as_deref());
        }
        if let Some(sort_order) = params.sort_order {
            query = query.bind(sort_order);
        }
        query = query.bind(key);
        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("knowledge tag {key}")));
        }
        Ok(())
    }

    async fn delete_knowledge_tag(&self, key: &str) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM knowledge_tags WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("knowledge tag {key}")));
        }
        Ok(())
    }
}

impl SqliteKnowledgeRepository {
    /// Reassemble a binding's `kb_id` list from the junction, ordered by
    /// `position` (the original `kb_ids` array order).
    async fn fetch_kb_ids(&self, binding_id: &str) -> Result<Vec<String>, DbError> {
        let kb_ids = sqlx::query_scalar::<_, String>(
            "SELECT kb_id FROM knowledge_binding_bases WHERE binding_id = ? ORDER BY position ASC, kb_id ASC",
        )
        .bind(binding_id)
        .fetch_all(&self.pool)
        .await?;
        kb_ids
            .into_iter()
            .map(|id| {
                KnowledgeBaseId::parse(id)
                    .map(KnowledgeBaseId::into_string)
                    .map_err(|error| DbError::Query(sqlx::Error::Decode(Box::new(error))))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::init_database_memory;

    const CONVERSATION_ID: &str =
        "conv_019abcde-f012-7abc-8abc-0123456789ab";
    const OTHER_CONVERSATION_ID: &str =
        "conv_019abcde-f012-7abc-8abc-0123456789ac";
    const KB_A: &str = "kb_019abcde-f012-7abc-8abc-0123456789ab";
    const KB_B: &str = "kb_019abcde-f012-7abc-8abc-0123456789ac";
    const KB_T: &str = "kb_019abcde-f012-7abc-8abc-0123456789ad";
    const KB_MISSING: &str = "kb_019abcde-f012-7abc-8abc-0123456789ae";

    fn make_base(id: &str) -> KnowledgeBaseRow {
        KnowledgeBaseRow {
            id: KnowledgeBaseId::parse(id).unwrap(),
            name: format!("kb-{id}"),
            description: String::new(),
            root_path: format!("/tmp/{id}"),
            managed: true,
            extra: "{}".into(),
            created_at: 1,
            updated_at: 1,
            tags: None,
        }
    }

    #[tokio::test]
    async fn base_crud_roundtrip() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());

        repo.insert_base(&make_base(KB_A)).await.unwrap();
        repo.insert_base(&make_base(KB_B)).await.unwrap();
        assert_eq!(repo.list_bases().await.unwrap().len(), 2);

        let mut row = repo.get_base(KB_A).await.unwrap().unwrap();
        row.name = "renamed".into();
        // `extra` is mutable through update (URL-source config lives there).
        row.extra = r#"{"source":{"kind":"url","mode":"live"}}"#.into();
        row.updated_at = 2;
        repo.update_base(&row).await.unwrap();
        let got = repo.get_base(KB_A).await.unwrap().unwrap();
        assert_eq!(got.name, "renamed");
        assert_eq!(got.extra, r#"{"source":{"kind":"url","mode":"live"}}"#);

        repo.delete_base(KB_A).await.unwrap();
        assert!(repo.get_base(KB_A).await.unwrap().is_none());
        assert!(matches!(repo.delete_base(KB_A).await, Err(DbError::NotFound(_))));
    }

    /// Insert a conversation so the conversation-kind binding's FK + CHECK are
    /// satisfied (target_conv_id REFERENCES conversations(id) ON DELETE CASCADE).
    async fn seed_conversation(pool: &SqlitePool, id: &str) {
        let installation_owner = crate::installation_owner_id(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, status, created_at, updated_at) \
             VALUES (?, ?, 'c', 'gemini', 'pending', 1, 1)",
        )
        .bind(id)
        .bind(installation_owner)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn binding_set_get_roundtrip() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());
        seed_conversation(db.pool(), CONVERSATION_ID).await;
        repo.insert_base(&make_base(KB_A)).await.unwrap();
        repo.insert_base(&make_base(KB_B)).await.unwrap();

        assert!(repo
            .get_binding("conversation", CONVERSATION_ID)
            .await
            .unwrap()
            .is_none());

        // Initial set: one base, disabled writeback, staged mode.
        let id1 = repo
            .set_binding(
                "conversation",
                CONVERSATION_ID,
                &[KB_A.to_owned()],
                true,
                false,
                "staged",
                "conservative",
                false,
                1,
            )
            .await
            .unwrap();
        assert!(nomifun_common::KnowledgeBindingId::parse(id1.clone()).is_ok());

        let (row, kb_ids) = repo
            .get_binding("conversation", CONVERSATION_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.binding_id.to_string(), id1);
        assert_eq!(row.target_kind, "conversation");
        assert_eq!(row.target_id().as_deref(), Some(CONVERSATION_ID));
        assert_eq!(
            row.target_conv_id.as_ref().map(ToString::to_string).as_deref(),
            Some(CONVERSATION_ID)
        );
        assert!(row.target_workpath.is_none() && row.target_term_id.is_none() && row.target_companion_id.is_none());
        assert!(row.enabled);
        assert!(!row.writeback);
        assert_eq!(row.writeback_mode, "staged");
        assert_eq!(row.writeback_eagerness, "conservative");
        assert_eq!(kb_ids, vec![KB_A.to_owned()]);

        // Update: same target reuses binding_id; junction replaced + reordered.
        let id2 = repo
            .set_binding(
                "conversation",
                CONVERSATION_ID,
                &[KB_B.to_owned(), KB_A.to_owned()],
                true,
                true,
                "direct",
                "aggressive",
                false,
                2,
            )
            .await
            .unwrap();
        assert_eq!(id2, id1, "same target must reuse the surrogate binding_id");

        let (row, kb_ids) = repo
            .get_binding("conversation", CONVERSATION_ID)
            .await
            .unwrap()
            .unwrap();
        assert!(row.writeback);
        assert_eq!(row.writeback_mode, "direct");
        assert_eq!(row.writeback_eagerness, "aggressive");
        assert_eq!(row.updated_at, 2);
        // Order from kb_ids slice is preserved via position.
        assert_eq!(kb_ids, vec![KB_B.to_owned(), KB_A.to_owned()]);

        repo.delete_binding("conversation", CONVERSATION_ID)
            .await
            .unwrap();
        assert!(repo
            .get_binding("conversation", CONVERSATION_ID)
            .await
            .unwrap()
            .is_none());
        // Deleting an absent binding is a no-op, not an error.
        repo.delete_binding("conversation", CONVERSATION_ID)
            .await
            .unwrap();
    }

    /// A workpath binding is keyed by a path string (not an entity, no FK), so
    /// it exercises the non-FK target column + the workpath partial UNIQUE.
    #[tokio::test]
    async fn binding_workpath_kind_and_empty_kb_ids() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());

        let bid = repo
            .set_binding("workpath", "/work/proj", &[], false, false, "staged", "conservative", false, 5)
            .await
            .unwrap();
        assert!(nomifun_common::KnowledgeBindingId::parse(bid).is_ok());

        let (row, kb_ids) = repo.get_binding("workpath", "/work/proj").await.unwrap().unwrap();
        assert_eq!(row.target_kind, "workpath");
        assert_eq!(row.target_workpath.as_deref(), Some("/work/proj"));
        assert!(row.target_conv_id.is_none());
        assert!(!row.enabled);
        assert!(kb_ids.is_empty(), "empty kb_ids slice yields no junction rows");

        // A different target_id is an independent binding.
        assert!(repo.get_binding("workpath", "/other").await.unwrap().is_none());
    }

    /// Deleting the conversation cascades the binding row away (target_conv_id
    /// FK ON DELETE CASCADE), and the junction follows via its own CASCADE.
    #[tokio::test]
    async fn deleting_conversation_cascades_binding() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());
        seed_conversation(db.pool(), OTHER_CONVERSATION_ID).await;
        repo.insert_base(&make_base(KB_A)).await.unwrap();

        let bid = repo
            .set_binding("conversation", OTHER_CONVERSATION_ID, &[KB_A.to_owned()], true, false, "staged", "conservative", false, 1)
            .await
            .unwrap();

        sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(OTHER_CONVERSATION_ID)
            .execute(db.pool())
            .await
            .unwrap();

        assert!(repo
            .get_binding("conversation", OTHER_CONVERSATION_ID)
            .await
            .unwrap()
            .is_none());
        let orphans = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_binding_bases WHERE binding_id = ?",
        )
        .bind(bid)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(orphans, 0, "junction rows must cascade with the binding");
    }

    /// An unknown kind is rejected on write and resolves to None on read,
    /// never silently writing to or matching the wrong column.
    #[tokio::test]
    async fn unknown_kind_is_rejected() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());

        assert!(matches!(
            repo.set_binding("bogus", "x", &[], true, false, "staged", "conservative", false, 1).await,
            Err(DbError::NotFound(_))
        ));
        assert!(repo.get_binding("bogus", "x").await.unwrap().is_none());
        // delete of an unknown kind is a no-op.
        repo.delete_binding("bogus", "x").await.unwrap();
    }

    /// `channel_write_enabled` (migration 009) persists and updates.
    #[tokio::test]
    async fn binding_persists_channel_write_enabled() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());
        repo.insert_base(&make_base(KB_A)).await.unwrap();

        // Default (false) on a write without the flag set.
        repo.set_binding("workpath", "/wp", &[KB_A.to_owned()], true, true, "staged", "conservative", false, 1)
            .await
            .unwrap();
        let (row, _) = repo.get_binding("workpath", "/wp").await.unwrap().unwrap();
        assert!(!row.channel_write_enabled);

        // Re-enable on update.
        repo.set_binding("workpath", "/wp", &[KB_A.to_owned()], true, true, "staged", "conservative", true, 2)
            .await
            .unwrap();
        let (row, _) = repo.get_binding("workpath", "/wp").await.unwrap().unwrap();
        assert!(row.channel_write_enabled, "channel_write_enabled must persist + update");
    }

    #[tokio::test]
    async fn list_bindings_using_kb_returns_all_consumers() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());
        repo.insert_base(&make_base(KB_A)).await.unwrap();
        repo.insert_base(&make_base(KB_B)).await.unwrap();

        // Two workpath bindings use kb_a (one enabled, one disabled); one uses kb_b only.
        repo.set_binding("workpath", "/p1", &[KB_A.to_owned()], true, false, "staged", "conservative", false, 1)
            .await
            .unwrap();
        repo.set_binding("workpath", "/p2", &[KB_A.to_owned(), KB_B.to_owned()], false, false, "staged", "conservative", false, 1)
            .await
            .unwrap();
        repo.set_binding("workpath", "/p3", &[KB_B.to_owned()], true, false, "staged", "conservative", false, 1)
            .await
            .unwrap();

        let mut using_a = repo.list_bindings_using_kb(KB_A).await.unwrap();
        assert_eq!(using_a.len(), 2, "p1 + p2 mount kb_a");
        using_a.sort_by(|x, y| x.target_workpath.cmp(&y.target_workpath));
        assert_eq!(using_a[0].target_workpath.as_deref(), Some("/p1"));
        assert!(using_a[0].enabled);
        assert_eq!(using_a[1].target_workpath.as_deref(), Some("/p2"));
        assert!(!using_a[1].enabled, "disabled binding still listed");

        assert_eq!(repo.list_bindings_using_kb(KB_B).await.unwrap().len(), 2, "p2 + p3 mount kb_b");
        assert!(repo.list_bindings_using_kb(KB_MISSING).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn knowledge_tags_crud_roundtrip() {
        use crate::models::{CreateKnowledgeTagParams, UpdateKnowledgeTagParams};

        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());

        // Initially empty.
        assert!(repo.list_knowledge_tags().await.unwrap().is_empty());

        // Create.
        repo.create_knowledge_tag(CreateKnowledgeTagParams {
            key: "research".into(),
            label: "研发".into(),
            color: Some("#4d9fff".into()),
            sort_order: 0,
            created_at: 1,
        })
        .await
        .unwrap();

        let tags = repo.list_knowledge_tags().await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].key, "research");
        assert_eq!(tags[0].label, "研发");
        assert_eq!(tags[0].color.as_deref(), Some("#4d9fff"));
        assert_eq!(tags[0].sort_order, 0);
        assert_eq!(tags[0].created_at, 1);

        // Update label only.
        repo.update_knowledge_tag("research", UpdateKnowledgeTagParams {
            label: Some("研发线".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        let tags = repo.list_knowledge_tags().await.unwrap();
        assert_eq!(tags[0].label, "研发线");
        assert_eq!(tags[0].color.as_deref(), Some("#4d9fff"), "untouched field preserved");

        // Update color to None.
        repo.update_knowledge_tag("research", UpdateKnowledgeTagParams {
            color: Some(None),
            ..Default::default()
        })
        .await
        .unwrap();
        let tags = repo.list_knowledge_tags().await.unwrap();
        assert!(tags[0].color.is_none(), "color cleared");

        // Delete.
        repo.delete_knowledge_tag("research").await.unwrap();
        assert!(repo.list_knowledge_tags().await.unwrap().is_empty());

        // Delete absent → NotFound.
        assert!(matches!(
            repo.delete_knowledge_tag("research").await,
            Err(DbError::NotFound(_))
        ));

        // Update absent → NotFound.
        assert!(matches!(
            repo.update_knowledge_tag("missing", UpdateKnowledgeTagParams::default()).await,
            Err(DbError::NotFound(_))
        ));
    }

    /// The `tags` column on `knowledge_bases` is read/written through the
    /// existing base CRUD methods.
    #[tokio::test]
    async fn base_tags_column_roundtrip() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteKnowledgeRepository::new(db.pool().clone());

        // Insert with tags = None (default for old rows).
        repo.insert_base(&make_base(KB_T)).await.unwrap();
        let row = repo.get_base(KB_T).await.unwrap().unwrap();
        assert!(row.tags.is_none(), "NULL maps to None");

        // Update with a JSON tags value.
        let mut row = row;
        row.tags = Some(r#"["research","ops"]"#.into());
        row.updated_at = 2;
        repo.update_base(&row).await.unwrap();
        let got = repo.get_base(KB_T).await.unwrap().unwrap();
        assert_eq!(got.tags.as_deref(), Some(r#"["research","ops"]"#));

        // list_bases also returns the tags column.
        let all = repo.list_bases().await.unwrap();
        assert_eq!(all[0].tags.as_deref(), Some(r#"["research","ops"]"#));
    }

    #[tokio::test]
    async fn malformed_stored_knowledge_entity_ids_are_rejected_on_read() {
        let db = init_database_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO knowledge_bases \
             (id, name, description, root_path, managed, extra, created_at, updated_at) \
             VALUES ('kb_1', 'bad', '', '/tmp/bad', 0, '{}', 1, 1)",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let repo = SqliteKnowledgeRepository::new(db.pool().clone());
        assert!(repo.list_bases().await.is_err());
    }
}
