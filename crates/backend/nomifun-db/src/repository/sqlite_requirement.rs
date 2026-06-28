use nomifun_common::{TimestampMs, now_ms};
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{RequirementRow, RequirementRowUpdate, RequirementTagRow};
use crate::repository::bind::{BindValue, bind_value, bind_value_as, bind_value_scalar};
use crate::repository::requirement::{IRequirementRepository, ListRequirementsParams};

#[derive(Clone, Debug)]
pub struct SqliteRequirementRepository {
    pool: SqlitePool,
}

impl SqliteRequirementRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Build a safe `ORDER BY` clause for the requirements list.
///
/// `order_by` is matched against a hard-coded whitelist of columns — user input
/// only *selects* a fixed column name and is NEVER interpolated into SQL, so a
/// value like `"title; DROP TABLE …"` simply misses the whitelist and falls
/// back to the default queue order. `order` is constrained to `ASC|DESC`
/// (default `DESC` for an explicit sort). For non-unique sort columns an
/// `id <dir>` tiebreaker is appended so pagination is deterministic.
fn build_order_clause(order_by: Option<&str>, order: Option<&str>) -> String {
    const DEFAULT: &str = "ORDER BY sort_seq ASC, priority DESC, created_at ASC";
    let col = match order_by {
        Some("id") => "id",
        Some("created_at") => "created_at",
        Some("updated_at") => "updated_at",
        Some("status") => "status",
        // Unknown column or no explicit sort → default queue order.
        _ => return DEFAULT.to_string(),
    };
    let dir = match order.map(str::to_ascii_lowercase).as_deref() {
        Some("asc") => "ASC",
        _ => "DESC",
    };
    if col == "id" {
        // `id` is unique — no tiebreaker needed.
        format!("ORDER BY id {dir}")
    } else {
        format!("ORDER BY {col} {dir}, id {dir}")
    }
}

#[async_trait::async_trait]
impl IRequirementRepository for SqliteRequirementRepository {
    async fn insert(&self, row: &RequirementRow) -> Result<i64, DbError> {
        // `id` is allocated by SQLite (INTEGER PK AUTOINCREMENT) and never bound;
        // the caller receives the assigned id via last_insert_rowid().
        let result = sqlx::query(
            "INSERT INTO requirements (\
                title, content, tag, order_key, sort_seq, status, priority, \
                completion_note, owner_session_id, owner_kind, claimed_at, lease_expires_at, \
                started_at, completed_at, attempt_count, created_by, extra, created_at, updated_at\
            ) VALUES (\
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?\
            )",
        )
        .bind(&row.title)
        .bind(&row.content)
        .bind(&row.tag)
        .bind(&row.order_key)
        .bind(&row.sort_seq)
        .bind(&row.status)
        .bind(row.priority)
        .bind(&row.completion_note)
        .bind(row.owner_session_id)
        .bind(&row.owner_kind)
        .bind(row.claimed_at)
        .bind(row.lease_expires_at)
        .bind(row.started_at)
        .bind(row.completed_at)
        .bind(row.attempt_count)
        .bind(&row.created_by)
        .bind(&row.extra)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn update(&self, id: i64, params: &RequirementRowUpdate) -> Result<(), DbError> {
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

        push_str!(title);
        push_str!(content);
        push_str!(tag);
        push_str!(order_key);
        push_str!(sort_seq);
        push_str!(status);
        push_i64!(priority);
        push_opt_str!(completion_note);
        push_opt_i64!(owner_session_id);
        push_opt_str!(owner_kind);
        push_opt_i64!(claimed_at);
        push_opt_i64!(lease_expires_at);
        push_opt_i64!(started_at);
        push_opt_i64!(completed_at);
        push_i64!(attempt_count);
        push_str!(extra);

        if set_parts.is_empty() {
            return Ok(());
        }

        set_parts.push("updated_at = ?".to_string());
        binds.push(BindValue::I64(now_ms()));

        let sql = format!("UPDATE requirements SET {} WHERE id = ?", set_parts.join(", "));
        let mut query = sqlx::query(&sql);
        for bind in &binds {
            query = bind_value(query, bind);
        }
        query = query.bind(id);

        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("requirement '{id}'")));
        }
        Ok(())
    }

    async fn delete(&self, id: i64) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM requirements WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("requirement '{id}'")));
        }
        Ok(())
    }

    async fn get_by_id(&self, id: i64) -> Result<Option<RequirementRow>, DbError> {
        let row = sqlx::query_as::<_, RequirementRow>("SELECT * FROM requirements WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list(&self, params: &ListRequirementsParams) -> Result<(Vec<RequirementRow>, u64), DbError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut binds: Vec<BindValue> = Vec::new();

        if let Some(tag) = &params.tag {
            where_parts.push("tag = ?".to_string());
            binds.push(BindValue::Str(tag.clone()));
        }
        if let Some(status) = &params.status {
            where_parts.push("status = ?".to_string());
            binds.push(BindValue::Str(status.clone()));
        }
        if let Some(owner) = &params.owner_session_id {
            where_parts.push("owner_session_id = ?".to_string());
            binds.push(BindValue::I64(*owner));
        }
        if let Some(kind) = &params.owner_kind {
            where_parts.push("owner_kind = ?".to_string());
            binds.push(BindValue::Str(kind.clone()));
        }
        if let Some(q) = &params.q
            && !q.is_empty()
        {
            // Escape LIKE metacharacters so a user typing `%` or `_` searches
            // literally rather than as wildcards. `\` is the ESCAPE char.
            let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            let like = format!("%{escaped}%");
            where_parts.push("(title LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\')".to_string());
            binds.push(BindValue::Str(like.clone()));
            binds.push(BindValue::Str(like));
        }

        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_parts.join(" AND "))
        };

        // total count
        let count_sql = format!("SELECT COUNT(*) FROM requirements{where_clause}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        for bind in &binds {
            count_query = bind_value_scalar(count_query, bind);
        }
        let total: i64 = count_query.fetch_one(&self.pool).await?;

        // page
        let page = params.page.unwrap_or(1).max(1);
        let page_size = params.page_size.unwrap_or(20).clamp(1, 200);
        let offset = (page - 1) * page_size;

        let page_sql = format!(
            "SELECT * FROM requirements{where_clause} {order_clause} LIMIT ? OFFSET ?",
            order_clause = build_order_clause(params.order_by.as_deref(), params.order.as_deref())
        );
        let mut page_query = sqlx::query_as::<_, RequirementRow>(&page_sql);
        for bind in &binds {
            page_query = bind_value_as(page_query, bind);
        }
        page_query = page_query.bind(page_size as i64).bind(offset as i64);
        let rows = page_query.fetch_all(&self.pool).await?;

        Ok((rows, total as u64))
    }

    async fn list_by_tag(&self, tag: &str) -> Result<Vec<RequirementRow>, DbError> {
        let rows = sqlx::query_as::<_, RequirementRow>(
            "SELECT * FROM requirements WHERE tag = ? \
             ORDER BY sort_seq ASC, priority DESC, created_at ASC",
        )
        .bind(tag)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn tag_status_counts(&self) -> Result<Vec<(String, String, i64)>, DbError> {
        let rows = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT tag, status, COUNT(*) as cnt FROM requirements GROUP BY tag, status ORDER BY tag ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn claim_next(
        &self,
        tag: &str,
        owner_session_id: i64,
        owner_kind: &str,
        lease_ms: i64,
        now: TimestampMs,
    ) -> Result<Option<RequirementRow>, DbError> {
        let row = sqlx::query_as::<_, RequirementRow>(
            "UPDATE requirements \
             SET status='in_progress', \
                 owner_session_id=?1, owner_kind=?2, \
                 claimed_at=?3, started_at=COALESCE(started_at, ?3), \
                 lease_expires_at=?3 + ?4, \
                 attempt_count=attempt_count + 1, \
                 updated_at=?3 \
             WHERE id = ( \
                 SELECT id FROM requirements \
                 WHERE tag = ?5 AND status = 'pending' \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM requirement_tags t WHERE t.tag = ?5 AND t.paused = 1 \
                   ) \
                 ORDER BY sort_seq ASC, priority DESC, created_at ASC \
                 LIMIT 1 \
             ) \
             RETURNING *",
        )
        .bind(owner_session_id)
        .bind(owner_kind)
        .bind(now)
        .bind(lease_ms)
        .bind(tag)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn renew_lease(&self, id: i64, owner: i64, lease_ms: i64, now: TimestampMs) -> Result<bool, DbError> {
        let result = sqlx::query(
            "UPDATE requirements SET lease_expires_at = ?1 + ?2, updated_at = ?1 \
             WHERE id = ?3 AND owner_session_id = ?4 AND status = 'in_progress'",
        )
        .bind(now)
        .bind(lease_ms)
        .bind(id)
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn sweep_expired_leases(
        &self,
        active_sessions: &[(String, i64)],
        now: TimestampMs,
    ) -> Result<u64, DbError> {
        // Exclude active sessions by the FULL dual-domain key `(owner_kind,
        // owner_session_id)`: a `(?,?)` tuple per active session, ORed and
        // negated. A kind-less `owner_session_id NOT IN (...)` would let an
        // active `conv#5` keep a stale `term#5` claim alive (numeric collision,
        // spec §2.2). When there are no active sessions the clause is omitted.
        let active_clause = if active_sessions.is_empty() {
            String::new()
        } else {
            let one = "(owner_kind = ? AND owner_session_id = ?)";
            let ors = vec![one; active_sessions.len()].join(" OR ");
            format!(" AND NOT ({ors})")
        };

        let sql = format!(
            "UPDATE requirements \
             SET status='pending', owner_session_id=NULL, owner_kind=NULL, \
                 claimed_at=NULL, lease_expires_at=NULL, updated_at=? \
             WHERE status='in_progress' \
               AND lease_expires_at IS NOT NULL \
               AND lease_expires_at < ?{active_clause}"
        );

        let mut query = sqlx::query(&sql).bind(now).bind(now);
        for (kind, id) in active_sessions {
            query = query.bind(kind).bind(id);
        }
        let result = query.execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    // ── AutoWork tag-level pause (Step 1) ───────────────────────────────

    async fn pause_tag(
        &self,
        tag: &str,
        reason: &str,
        req_id: Option<i64>,
        now: TimestampMs,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO requirement_tags (tag, paused, paused_reason, paused_req_id, paused_at) \
             VALUES (?1, 1, ?2, ?3, ?4) \
             ON CONFLICT(tag) DO UPDATE SET \
                 paused = 1, paused_reason = ?2, paused_req_id = ?3, paused_at = ?4",
        )
        .bind(tag)
        .bind(reason)
        .bind(req_id)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn resume_tag(&self, tag: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE requirement_tags SET paused = 0 WHERE tag = ?1")
            .bind(tag)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn is_tag_paused(&self, tag: &str) -> Result<bool, DbError> {
        let paused: Option<i64> = sqlx::query_scalar("SELECT paused FROM requirement_tags WHERE tag = ?1")
            .bind(tag)
            .fetch_optional(&self.pool)
            .await?;
        Ok(paused.unwrap_or(0) != 0)
    }

    async fn get_tag_state(&self, tag: &str) -> Result<Option<RequirementTagRow>, DbError> {
        let row = sqlx::query_as::<_, RequirementTagRow>("SELECT * FROM requirement_tags WHERE tag = ?1")
            .bind(tag)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn unclaim(&self, id: i64, owner: i64) -> Result<bool, DbError> {
        let result = sqlx::query(
            "UPDATE requirements \
             SET status='pending', owner_session_id=NULL, owner_kind=NULL, \
                 claimed_at=NULL, lease_expires_at=NULL, \
                 attempt_count = MAX(attempt_count - 1, 0), \
                 updated_at=?1 \
             WHERE id=?2 AND status='in_progress' AND owner_session_id=?3",
        )
        .bind(now_ms())
        .bind(id)
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    async fn setup() -> (SqliteRequirementRepository, crate::Database) {
        let db = init_database_memory().await.expect("init db");
        let repo = SqliteRequirementRepository::new(db.pool().clone());
        // A user is seeded for realism; `owner_session_id` has no FK (dual-domain
        // owner token, now an i64), so claims do not require a real conversation.
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
             VALUES ('user_1', 'tester', 'hash', 0, 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        (repo, db)
    }

    /// A conversation owner id used by claim tests. owner_session_id has no FK,
    /// so this is just an arbitrary i64.
    const CONV_OWNER: i64 = 1001;

    fn make_row(tag: &str, sort_seq: &str) -> RequirementRow {
        let now = now_ms();
        RequirementRow {
            // id is allocated by SQLite on insert(); the value here is ignored.
            id: 0,
            title: format!("Req {tag}/{sort_seq}"),
            content: "do the thing".into(),
            tag: tag.into(),
            order_key: sort_seq.into(),
            sort_seq: sort_seq.into(),
            status: "pending".into(),
            priority: 0,
            completion_note: None,
            owner_session_id: None,
            owner_kind: None,
            claimed_at: None,
            lease_expires_at: None,
            started_at: None,
            completed_at: None,
            attempt_count: 0,
            created_by: "user".into(),
            extra: "{}".into(),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn insert_and_get() {
        let (repo, _db) = setup().await;
        let id = repo.insert(&make_row("t", "00000001")).await.unwrap();
        assert!(id > 0);
        let found = repo.get_by_id(id).await.unwrap().expect("found");
        assert_eq!(found.id, id);
        assert_eq!(found.status, "pending");
        assert_eq!(found.tag, "t");
    }

    #[tokio::test]
    async fn update_nonexistent_is_not_found() {
        let (repo, _db) = setup().await;
        let params = RequirementRowUpdate {
            title: Some("x".into()),
            ..Default::default()
        };
        let err = repo.update(999_999, &params).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_filters_and_paginates() {
        let (repo, _db) = setup().await;
        let id1 = repo.insert(&make_row("alpha", "00000001")).await.unwrap();
        repo.insert(&make_row("alpha", "00000002")).await.unwrap();
        repo.insert(&make_row("beta", "00000001")).await.unwrap();

        let (rows, total) = repo
            .list(&ListRequirementsParams {
                tag: Some("alpha".into()),
                page_size: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id1); // lowest sort_seq first
    }

    #[tokio::test]
    async fn list_sorts_by_whitelisted_field_and_direction() {
        let (repo, _db) = setup().await;

        // Three rows with distinct created_at / updated_at / status, inserted in
        // id order (id1 < id2 < id3). sort_seq is deliberately NOT id-aligned so
        // a fallback to the default order would be distinguishable.
        let mut r1 = make_row("s", "00000003");
        r1.created_at = 100;
        r1.updated_at = 300;
        r1.status = "pending".into();
        let mut r2 = make_row("s", "00000001");
        r2.created_at = 300;
        r2.updated_at = 100;
        r2.status = "done".into();
        let mut r3 = make_row("s", "00000002");
        r3.created_at = 200;
        r3.updated_at = 200;
        r3.status = "cancelled".into();
        let id1 = repo.insert(&r1).await.unwrap();
        let id2 = repo.insert(&r2).await.unwrap();
        let id3 = repo.insert(&r3).await.unwrap();

        let sort = |order_by: &str, order: &str| ListRequirementsParams {
            order_by: Some(order_by.into()),
            order: Some(order.into()),
            page_size: Some(100),
            ..Default::default()
        };
        let ids = |rows: &[RequirementRow]| rows.iter().map(|r| r.id).collect::<Vec<_>>();

        // id asc / desc
        let (rows, _) = repo.list(&sort("id", "asc")).await.unwrap();
        assert_eq!(ids(&rows), vec![id1, id2, id3]);
        let (rows, _) = repo.list(&sort("id", "desc")).await.unwrap();
        assert_eq!(ids(&rows), vec![id3, id2, id1]);

        // created_at desc → r2(300) > r3(200) > r1(100)
        let (rows, _) = repo.list(&sort("created_at", "desc")).await.unwrap();
        assert_eq!(ids(&rows), vec![id2, id3, id1]);

        // updated_at asc → r2(100) < r3(200) < r1(300)
        let (rows, _) = repo.list(&sort("updated_at", "asc")).await.unwrap();
        assert_eq!(ids(&rows), vec![id2, id3, id1]);

        // status asc (alphabetical): cancelled(r3) < done(r2) < pending(r1)
        let (rows, _) = repo.list(&sort("status", "asc")).await.unwrap();
        assert_eq!(ids(&rows), vec![id3, id2, id1]);
    }

    #[tokio::test]
    async fn list_status_sort_breaks_ties_by_id() {
        let (repo, _db) = setup().await;
        // All three share status "pending" (make_row default) → the secondary
        // `id` tiebreaker decides, in the SAME direction as the primary sort.
        let id1 = repo.insert(&make_row("s", "1")).await.unwrap();
        let id2 = repo.insert(&make_row("s", "2")).await.unwrap();
        let id3 = repo.insert(&make_row("s", "3")).await.unwrap();

        let (rows, _) = repo
            .list(&ListRequirementsParams {
                order_by: Some("status".into()),
                order: Some("desc".into()),
                page_size: Some(100),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![id3, id2, id1]);
    }

    #[tokio::test]
    async fn list_unknown_order_by_falls_back_and_resists_injection() {
        let (repo, _db) = setup().await;
        // Default order is sort_seq ASC; insert so the lower sort_seq has the
        // HIGHER id, making the fallback distinguishable from id-order.
        repo.insert(&make_row("s", "00000002")).await.unwrap();
        let id_low_seq = repo.insert(&make_row("s", "00000001")).await.unwrap();

        let (rows, total) = repo
            .list(&ListRequirementsParams {
                // Not in the whitelist + an injection attempt: must be ignored.
                order_by: Some("title; DROP TABLE requirements".into()),
                order: Some("asc".into()),
                page_size: Some(100),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(total, 2, "table intact — the injection string never reached SQL");
        assert_eq!(rows[0].id, id_low_seq, "fell back to default sort_seq ASC order");
    }

    #[tokio::test]
    async fn claim_next_returns_lowest_order_then_none() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("t", "00000002")).await.unwrap();
        let id1 = repo.insert(&make_row("t", "00000001")).await.unwrap();

        let first = repo
            .claim_next("t", CONV_OWNER, "conversation", 60_000, now_ms())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.id, id1);
        assert_eq!(first.status, "in_progress");
        assert_eq!(first.owner_session_id, Some(CONV_OWNER));
        assert_eq!(first.owner_kind.as_deref(), Some("conversation"));
        assert_eq!(first.attempt_count, 1);

        let second = repo
            .claim_next("t", CONV_OWNER, "conversation", 60_000, now_ms())
            .await
            .unwrap()
            .unwrap();
        assert_ne!(second.id, id1);

        let third = repo.claim_next("t", CONV_OWNER, "conversation", 60_000, now_ms()).await.unwrap();
        assert!(third.is_none(), "tag drained");
    }

    #[tokio::test]
    async fn claim_allows_non_conversation_owner() {
        // Regression for terminal AutoWork: the claim owner may be a terminal id
        // recorded with owner_kind='terminal'. owner_session_id carries no FK
        // (dual-domain), so the claim must succeed and record both the owner
        // token and its kind (paired, satisfying the table's CHECK).
        let (repo, _db) = setup().await;
        repo.insert(&make_row("t", "00000001")).await.unwrap();
        let term_owner: i64 = 7;
        let claimed = repo
            .claim_next("t", term_owner, "terminal", 60_000, now_ms())
            .await
            .expect("claim must not error on a non-conversation owner")
            .expect("a pending requirement is available to claim");
        assert_eq!(claimed.status, "in_progress");
        assert_eq!(claimed.owner_session_id, Some(term_owner));
        assert_eq!(claimed.owner_kind.as_deref(), Some("terminal"));
    }

    #[tokio::test]
    async fn concurrent_claims_each_row_once() {
        let (repo, db) = setup().await;
        for i in 0..20 {
            repo.insert(&make_row("t", &format!("{i:08}"))).await.unwrap();
        }

        // N concurrent claimers against the same tag + pool.
        let mut handles = Vec::new();
        for c in 0..8 {
            let r = SqliteRequirementRepository::new(db.pool().clone());
            let owner: i64 = 2000 + c;
            handles.push(tokio::spawn(async move {
                let mut got = Vec::new();
                while let Some(row) = r
                    .claim_next("t", owner, "conversation", 60_000, now_ms())
                    .await
                    .unwrap()
                {
                    got.push(row.id);
                }
                got
            }));
        }

        let mut all_claimed = Vec::new();
        for h in handles {
            all_claimed.extend(h.await.unwrap());
        }
        all_claimed.sort();
        all_claimed.dedup();
        // All 20 claimed, each exactly once (dedup removed nothing).
        assert_eq!(all_claimed.len(), 20, "every requirement claimed exactly once");
    }

    #[tokio::test]
    async fn sweep_repends_expired_when_conversation_inactive() {
        let (repo, _db) = setup().await;
        let id1 = repo.insert(&make_row("t", "00000001")).await.unwrap();
        // Claim with a lease that is already in the past.
        let past = now_ms() - 10_000;
        let claimed = repo
            .claim_next("t", CONV_OWNER, "conversation", 1, past)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.status, "in_progress");

        // Active set does NOT include CONV_OWNER → it should be re-pended.
        let reset = repo.sweep_expired_leases(&[], now_ms()).await.unwrap();
        assert_eq!(reset, 1);
        let row = repo.get_by_id(id1).await.unwrap().unwrap();
        assert_eq!(row.status, "pending");
        assert!(row.owner_session_id.is_none());
        assert!(row.owner_kind.is_none());

        // Re-claim, then sweep with CONV_OWNER ACTIVE → must be retained.
        let past2 = now_ms() - 10_000;
        repo.claim_next("t", CONV_OWNER, "conversation", 1, past2)
            .await
            .unwrap()
            .unwrap();
        let reset2 = repo
            .sweep_expired_leases(&[("conversation".to_string(), CONV_OWNER)], now_ms())
            .await
            .unwrap();
        assert_eq!(reset2, 0);
        let row2 = repo.get_by_id(id1).await.unwrap().unwrap();
        assert_eq!(row2.status, "in_progress");
    }

    #[tokio::test]
    async fn sweep_active_conversation_does_not_protect_same_numbered_terminal() {
        // Cross-domain (spec §2.2): the sweeper excludes active sessions by the
        // FULL `(owner_kind, owner_session_id)` key. An expired lease owned by
        // TERMINAL #5 must be re-pended even when CONVERSATION #5 (same number)
        // is in the active set — a kind-less `NOT IN (5)` would wrongly retain it.
        let (repo, _db) = setup().await;
        let id = repo.insert(&make_row("t", "00000001")).await.unwrap();
        let past = now_ms() - 10_000;
        // Claim it for TERMINAL #5 with an already-expired lease.
        repo.claim_next("t", 5, "terminal", 1, past).await.unwrap().unwrap();

        // Sweep with CONVERSATION #5 active. The terminal#5 lease must NOT be
        // protected by the numerically-equal active conversation.
        let reset = repo
            .sweep_expired_leases(&[("conversation".to_string(), 5)], now_ms())
            .await
            .unwrap();
        assert_eq!(reset, 1, "term#5's expired lease is re-pended despite active conv#5");
        let row = repo.get_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "pending");
        assert!(row.owner_session_id.is_none());

        // Positive control: an active TERMINAL #5 DOES protect its own lease.
        repo.claim_next("t", 5, "terminal", 1, now_ms() - 10_000)
            .await
            .unwrap()
            .unwrap();
        let reset2 = repo
            .sweep_expired_leases(&[("terminal".to_string(), 5)], now_ms())
            .await
            .unwrap();
        assert_eq!(reset2, 0, "an active terminal#5 retains its own expired lease");
    }

    #[tokio::test]
    async fn pause_resume_roundtrip() {
        let (repo, _db) = setup().await;
        assert!(!repo.is_tag_paused("t").await.unwrap(), "absent tag = not paused");
        assert!(repo.get_tag_state("t").await.unwrap().is_none());

        // paused_req_id is an FK → requirements(id); the triggering requirement
        // must exist before it can be recorded on the pause row.
        let req_x = repo.insert(&make_row("t", "00000001")).await.unwrap();
        repo.pause_tag("t", "requirement_failed", Some(req_x), now_ms())
            .await
            .unwrap();
        assert!(repo.is_tag_paused("t").await.unwrap());
        let st = repo.get_tag_state("t").await.unwrap().expect("state row exists");
        assert!(st.is_paused());
        assert_eq!(st.paused_reason.as_deref(), Some("requirement_failed"));
        assert_eq!(st.paused_req_id, Some(req_x));
        assert!(st.paused_at.is_some());

        // Idempotent re-pause updates reason/req_id without erroring.
        repo.pause_tag("t", "manual", None, now_ms()).await.unwrap();
        let st2 = repo.get_tag_state("t").await.unwrap().unwrap();
        assert_eq!(st2.paused_reason.as_deref(), Some("manual"));

        repo.resume_tag("t").await.unwrap();
        assert!(!repo.is_tag_paused("t").await.unwrap());
    }

    #[tokio::test]
    async fn claim_next_skips_paused_tag() {
        let (repo, _db) = setup().await;
        repo.insert(&make_row("t", "00000001")).await.unwrap();
        // None trigger id: this test only exercises the pause→skip→resume gate,
        // and paused_req_id is an FK → requirements(id), so a bare placeholder id
        // would violate it. The recorded trigger is irrelevant here.
        repo.pause_tag("t", "requirement_failed", None, now_ms())
            .await
            .unwrap();

        let claimed = repo.claim_next("t", CONV_OWNER, "conversation", 60_000, now_ms()).await.unwrap();
        assert!(claimed.is_none(), "paused tag must not yield a claim");

        repo.resume_tag("t").await.unwrap();
        let claimed2 = repo.claim_next("t", CONV_OWNER, "conversation", 60_000, now_ms()).await.unwrap();
        assert!(claimed2.is_some(), "resumed tag claims again");
    }

    #[tokio::test]
    async fn unclaim_decrements_attempt_and_repends() {
        let (repo, _db) = setup().await;
        let id1 = repo.insert(&make_row("t", "00000001")).await.unwrap();
        let claimed = repo
            .claim_next("t", CONV_OWNER, "conversation", 60_000, now_ms())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.attempt_count, 1);

        // Wrong owner → no-op (row stays claimed).
        assert!(!repo.unclaim(id1, 9999).await.unwrap());
        assert_eq!(repo.get_by_id(id1).await.unwrap().unwrap().status, "in_progress");

        // Correct owner → revert to pending, attempt decremented (NOT consumed).
        assert!(repo.unclaim(id1, CONV_OWNER).await.unwrap());
        let row = repo.get_by_id(id1).await.unwrap().unwrap();
        assert_eq!(row.status, "pending");
        assert_eq!(row.attempt_count, 0, "unclaim must not consume an attempt");
        assert!(row.owner_session_id.is_none());
        assert!(row.owner_kind.is_none());
        assert!(row.lease_expires_at.is_none());
    }

    // ── Adversarial review (spec §2.2): renew_lease / unclaim are kind-LESS ──
    //
    // Both guards are `id = ? AND owner_session_id = ?` with NO owner_kind. This
    // test CHARACTERIZES that gap: a row owned by TERMINAL #5 is mutated by a
    // call that passes only the numeric owner `5` — i.e. the DB primitive itself
    // does not isolate domains. This is NOT a reachable production hole because:
    //   (a) the row is pinned by its UNIQUE requirement id (`id = ?`), and
    //   (b) the only callers (orchestrator run_loop) always pass the req_id they
    //       themselves just claimed via claim_next(kind, owner), so owner_kind
    //       always matches in the self-flow,
    // but it documents that the kind-less guard relies on caller discipline +
    // the unique-id pin rather than on its own domain check. If a future caller
    // ever passes a cross-domain (req_id, owner) pair, these primitives would
    // mutate the wrong domain's claim. Defense-in-depth fix = add
    // `AND owner_kind = ?` to both guards (cheap, closes the class entirely).
    #[tokio::test]
    async fn renew_and_unclaim_guards_do_not_consult_owner_kind() {
        let (repo, _db) = setup().await;
        let id = repo.insert(&make_row("t", "00000001")).await.unwrap();
        // Claim it for TERMINAL #5.
        let claimed = repo.claim_next("t", 5, "terminal", 60_000, now_ms()).await.unwrap().unwrap();
        assert_eq!(claimed.owner_kind.as_deref(), Some("terminal"));
        assert_eq!(claimed.owner_session_id, Some(5));

        // renew_lease(req_id, 5) succeeds even though the caller carries no kind:
        // the guard matches purely on (unique id, numeric owner). A conversation
        // path passing owner 5 would renew a TERMINAL-owned lease.
        let renewed = repo.renew_lease(id, 5, 60_000, now_ms()).await.unwrap();
        assert!(
            renewed,
            "renew_lease matched a terminal-owned row on the bare numeric owner — \
             the guard is kind-less (characterization, see test doc)"
        );

        // unclaim(req_id, 5) likewise reverts the TERMINAL-owned claim with no
        // kind check.
        let unclaimed = repo.unclaim(id, 5).await.unwrap();
        assert!(
            unclaimed,
            "unclaim matched a terminal-owned row on the bare numeric owner — kind-less guard"
        );
        let row = repo.get_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "pending", "the terminal-owned claim was reverted by a kind-less unclaim");
    }
}
