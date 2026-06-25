use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::IdmmInterventionRow;
use crate::repository::idmm_intervention::{IIdmmInterventionRepository, PER_TARGET_CAP};

#[derive(Clone, Debug)]
pub struct SqliteIdmmInterventionRepository {
    pool: SqlitePool,
}

impl SqliteIdmmInterventionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IIdmmInterventionRepository for SqliteIdmmInterventionRepository {
    async fn insert(&self, row: &IdmmInterventionRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO idmm_interventions (\
                id, target_kind, target_id, watch, at, signal, tier_used, category, \
                action, detail, reason, confidence, bypass_model, outcome\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.target_kind)
        .bind(&row.target_id)
        .bind(&row.watch)
        .bind(row.at)
        .bind(&row.signal)
        .bind(&row.tier_used)
        .bind(&row.category)
        .bind(&row.action)
        .bind(&row.detail)
        .bind(&row.reason)
        .bind(row.confidence)
        .bind(&row.bypass_model)
        .bind(&row.outcome)
        .execute(&self.pool)
        .await?;

        // 激进淘汰:每写入即把该 target 裁到最近 PER_TARGET_CAP 条(数据可丢)。
        sqlx::query(
            "DELETE FROM idmm_interventions \
              WHERE target_kind = ?1 AND target_id = ?2 \
                AND id NOT IN (\
                  SELECT id FROM idmm_interventions \
                   WHERE target_kind = ?1 AND target_id = ?2 \
                   ORDER BY at DESC, id DESC LIMIT ?3\
                )",
        )
        .bind(&row.target_kind)
        .bind(&row.target_id)
        .bind(PER_TARGET_CAP)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn list_for_target(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: i64,
    ) -> Result<Vec<IdmmInterventionRow>, DbError> {
        let rows = sqlx::query_as::<_, IdmmInterventionRow>(
            "SELECT * FROM idmm_interventions \
              WHERE target_kind = ? AND target_id = ? \
              ORDER BY at DESC, id DESC LIMIT ?",
        )
        .bind(target_kind)
        .bind(target_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete_for_target(&self, target_kind: &str, target_id: &str) -> Result<u64, DbError> {
        let result = sqlx::query("DELETE FROM idmm_interventions WHERE target_kind = ? AND target_id = ?")
            .bind(target_kind)
            .bind(target_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    async fn list_recent(&self, limit: i64) -> Result<Vec<IdmmInterventionRow>, DbError> {
        let rows = sqlx::query_as::<_, IdmmInterventionRow>(
            "SELECT * FROM idmm_interventions ORDER BY at DESC, id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn clear_all(&self) -> Result<u64, DbError> {
        let result = sqlx::query("DELETE FROM idmm_interventions").execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    async fn sweep(&self, cutoff_ms: i64, global_cap: i64) -> Result<u64, DbError> {
        // 先按 TTL 删旧。
        let by_ttl = sqlx::query("DELETE FROM idmm_interventions WHERE at < ?")
            .bind(cutoff_ms)
            .execute(&self.pool)
            .await?
            .rows_affected();

        // 再按全局硬上限兜底:只留最近 global_cap 条。
        let by_cap = sqlx::query(
            "DELETE FROM idmm_interventions \
              WHERE id NOT IN (\
                SELECT id FROM idmm_interventions ORDER BY at DESC, id DESC LIMIT ?\
              )",
        )
        .bind(global_cap)
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(by_ttl + by_cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    async fn setup() -> (SqliteIdmmInterventionRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteIdmmInterventionRepository::new(db.pool().clone());
        (repo, db)
    }

    fn sample_row(id: &str, target_kind: &str, target_id: &str, at: i64) -> IdmmInterventionRow {
        IdmmInterventionRow {
            id: id.to_string(),
            target_kind: target_kind.to_string(),
            target_id: target_id.to_string(),
            watch: "decision".to_string(),
            at,
            signal: "decision".to_string(),
            tier_used: "rule".to_string(),
            category: Some("option".to_string()),
            action: "answer_choice".to_string(),
            detail: Some("选了方案A".to_string()),
            reason: Some("规则匹配".to_string()),
            confidence: None,
            bypass_model: None,
            outcome: "applied".to_string(),
        }
    }

    #[tokio::test]
    async fn insert_then_list_returns_recent_first() {
        let (repo, _db) = setup().await;
        repo.insert(&sample_row("idmmrec_a", "conversation", "c1", 10))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_b", "conversation", "c1", 30))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_c", "conversation", "c1", 20))
            .await
            .unwrap();

        let rows = repo.list_for_target("conversation", "c1", 100).await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        // 按 at DESC:30 -> 20 -> 10。
        assert_eq!(ids, vec!["idmmrec_b", "idmmrec_c", "idmmrec_a"]);
    }

    #[tokio::test]
    async fn insert_prunes_to_per_target_cap() {
        let (repo, _db) = setup().await;
        // 插 35 条,at 递增(at=i 对应 id idmmrec_i)。
        for i in 0..35 {
            repo.insert(&sample_row(&format!("idmmrec_{i:02}"), "conversation", "c1", i))
                .await
                .unwrap();
        }

        let rows = repo.list_for_target("conversation", "c1", 100).await.unwrap();
        assert_eq!(rows.len(), PER_TARGET_CAP as usize);
        assert_eq!(rows.len(), 30);

        // 最旧 5 条(at 0..=4)应已被裁掉。
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        for i in 0..5 {
            let stale = format!("idmmrec_{i:02}");
            assert!(!ids.contains(&stale), "oldest id {stale} should have been evicted");
        }
        // 最新一条仍在。
        assert!(ids.contains(&"idmmrec_34".to_string()));
        // 最旧的留存项是 at=5。
        let oldest = rows.last().unwrap();
        assert_eq!(oldest.id, "idmmrec_05");
    }

    #[tokio::test]
    async fn delete_for_target_removes_only_that_target() {
        let (repo, _db) = setup().await;
        repo.insert(&sample_row("idmmrec_c1a", "conversation", "c1", 10))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_c1b", "conversation", "c1", 20))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_t1a", "terminal", "1", 15))
            .await
            .unwrap();

        let removed = repo.delete_for_target("conversation", "c1").await.unwrap();
        assert_eq!(removed, 2);

        assert!(repo.list_for_target("conversation", "c1", 100).await.unwrap().is_empty());
        let remaining = repo.list_for_target("terminal", "1", 100).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "idmmrec_t1a");
    }

    #[tokio::test]
    async fn sweep_removes_older_than_cutoff() {
        let (repo, _db) = setup().await;
        repo.insert(&sample_row("idmmrec_old", "conversation", "c1", 100))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_new", "conversation", "c1", 1000))
            .await
            .unwrap();

        // cutoff=500:删 at<500(old),留 new。global_cap 足够大不触发硬上限。
        let removed = repo.sweep(500, 2000).await.unwrap();
        assert_eq!(removed, 1);

        let rows = repo.list_for_target("conversation", "c1", 100).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "idmmrec_new");
    }

    #[tokio::test]
    async fn list_recent_is_cross_target_recent_first_capped() {
        let (repo, _db) = setup().await;
        // 跨多个 target 写入,at 交错。
        repo.insert(&sample_row("idmmrec_c1a", "conversation", "c1", 10))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_t1a", "terminal", "1", 40))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_c2a", "conversation", "c2", 20))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_t1b", "terminal", "1", 30))
            .await
            .unwrap();

        // 跨全部 target 按 at DESC:40 -> 30 -> 20 -> 10。
        let rows = repo.list_recent(100).await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["idmmrec_t1a", "idmmrec_t1b", "idmmrec_c2a", "idmmrec_c1a"]);

        // limit 封顶,仍取最近的。
        let capped = repo.list_recent(2).await.unwrap();
        let ids: Vec<&str> = capped.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["idmmrec_t1a", "idmmrec_t1b"]);
    }

    #[tokio::test]
    async fn clear_all_empties_table_and_returns_count() {
        let (repo, _db) = setup().await;
        repo.insert(&sample_row("idmmrec_c1a", "conversation", "c1", 10))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_t1a", "terminal", "1", 20))
            .await
            .unwrap();
        repo.insert(&sample_row("idmmrec_c2a", "conversation", "c2", 30))
            .await
            .unwrap();

        let removed = repo.clear_all().await.unwrap();
        assert_eq!(removed, 3);

        assert!(repo.list_recent(100).await.unwrap().is_empty());
    }
}
