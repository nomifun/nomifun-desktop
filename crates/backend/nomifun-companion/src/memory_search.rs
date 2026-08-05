//! Unified full-text memory search over the `companion_memories_fts` FTS5
//! index — the single retrieval interface behind the `recall_memories` tool,
//! the REST list endpoint (q → relevance) and the second-wave in-session
//! summon track. Interface names (`MemorySearchQuery` / `MemorySearchHit` /
//! `CompanionStore::search_memories`) are a cross-track contract; do not
//! rename.
//!
//! Ranking: BM25 relevance first, fused with durable-importance boosts —
//! `rank = -bm25 + pinned*2.0 + importance*0.5 + strength*0.5` (bm25() is
//! smaller-is-better, so its negation is higher-is-better).
//!
//! The trigram tokenizer cannot MATCH patterns shorter than 3 characters, so
//! 1–2 char query terms (common Chinese words like 「咖啡」) fall back to a
//! LIKE substring scan with a neutral BM25 of 0 — still filtered, deduped and
//! boost-ranked like every other hit, just without a relevance component.

use std::collections::HashMap;

use nomifun_common::{AppError, CompanionId};
use sqlx::Row;

use crate::store::{CompanionMemory, CompanionStore, row_to_memory};

/// Row alias fixed by the cross-track interface contract.
pub type CompanionMemoryRow = CompanionMemory;

/// Lifecycle filter for a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryStatusFilter {
    Active,
    Archived,
    All,
}

/// One full-text memory search. `queries` are OR-merged (deduped by memory).
#[derive(Debug, Clone)]
pub struct MemorySearchQuery {
    pub queries: Vec<String>,
    pub kind: Option<String>,
    pub status: MemoryStatusFilter,
    /// Visibility scope: this companion's own memories plus any vestigial
    /// unowned row the boot migration has not re-homed yet. `None` = every
    /// memory (the owner's administrative view).
    pub companion_id: Option<CompanionId>,
    pub limit: usize,
}

impl Default for MemorySearchQuery {
    fn default() -> Self {
        Self {
            queries: Vec::new(),
            kind: None,
            status: MemoryStatusFilter::Active,
            companion_id: None,
            limit: 20,
        }
    }
}

/// One ranked hit. `snippet` carries `<b>…</b>` highlight markers (FTS hits
/// only; LIKE-fallback hits have no snippet).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemorySearchHit {
    pub memory: CompanionMemoryRow,
    pub rank: f64,
    pub snippet: Option<String>,
}

/// Hard cap on hits considered per query term and returned overall.
const SEARCH_LIMIT_CAP: usize = 500;
/// Default when the caller passes `limit: 0`.
const SEARCH_LIMIT_DEFAULT: usize = 20;

/// Below this many characters a term cannot produce a trigram MATCH.
const TRIGRAM_MIN_CHARS: usize = 3;

/// FTS5 string-literal escape: wrap in double quotes, double embedded quotes.
fn fts_phrase(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// `LIKE` pattern for substring match with `ESCAPE '\'`.
fn like_pattern(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    out.push('%');
    for c in term.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Boost-fused rank (see module docs). `bm25` is SQLite's smaller-is-better
/// value; LIKE-fallback hits pass 0.0.
fn fused_rank(bm25: f64, memory: &CompanionMemory) -> f64 {
    -bm25
        + if memory.pinned { 2.0 } else { 0.0 }
        + memory.importance * 0.5
        + memory.strength * 0.5
}

/// The shared `AND …` tail for both the MATCH and LIKE arms, with its binds.
struct FilterClause {
    sql: String,
    kind: Option<String>,
    status: Option<&'static str>,
    visible_to: Option<String>,
}

fn build_filter(q: &MemorySearchQuery) -> Result<FilterClause, AppError> {
    let mut sql = String::new();
    let kind = q.kind.clone().filter(|kind| !kind.is_empty());
    if kind.is_some() {
        sql.push_str(" AND m.kind = ?");
    }
    let status = match q.status {
        MemoryStatusFilter::Active => Some("active"),
        MemoryStatusFilter::Archived => Some("archived"),
        MemoryStatusFilter::All => None,
    };
    if status.is_some() {
        sql.push_str(" AND m.status = ?");
    }
    // Same visibility rule as `store::MEMORY_VISIBILITY_PREDICATE` (aliased `m.`
    // here): the companion's own memories plus the not-yet-re-homed unowned ones.
    let visible_to = q.companion_id.as_ref().map(|companion_id| {
        sql.push_str(" AND (m.scope_kind = 'user' OR m.scope_companion_id = ?)");
        companion_id.as_str().to_owned()
    });
    Ok(FilterClause { sql, kind, status, visible_to })
}

fn bind_filter<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    filter: &'q FilterClause,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    if let Some(kind) = &filter.kind {
        query = query.bind(kind);
    }
    if let Some(status) = filter.status {
        query = query.bind(status);
    }
    if let Some(visible_to) = &filter.visible_to {
        query = query.bind(visible_to);
    }
    query
}

impl CompanionStore {
    /// Multi-term OR full-text search with status/kind/scope filtering and
    /// boost-fused ranking. Cross-track interface — signature is contractual.
    pub async fn search_memories(
        &self,
        q: MemorySearchQuery,
    ) -> Result<Vec<MemorySearchHit>, AppError> {
        let terms: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            q.queries
                .iter()
                .map(|term| term.trim())
                .filter(|term| !term.is_empty())
                .filter(|term| seen.insert(term.to_owned()))
                .map(str::to_owned)
                .collect()
        };
        if terms.is_empty() {
            return Err(AppError::BadRequest(
                "memory search queries must contain at least one non-empty term".into(),
            ));
        }
        let limit = if q.limit == 0 {
            SEARCH_LIMIT_DEFAULT
        } else {
            q.limit.min(SEARCH_LIMIT_CAP)
        };
        let filter = build_filter(&q)?;

        // memory_id → (row, best bm25, best snippet). Smaller bm25 = better.
        let mut merged: HashMap<String, (CompanionMemory, f64, Option<String>)> = HashMap::new();
        for term in &terms {
            let (rows, is_fts) = if term.chars().count() >= TRIGRAM_MIN_CHARS {
                let sql = format!(
                    "SELECT m.*, bm25(companion_memories_fts) AS bm25_score, \
                            snippet(companion_memories_fts, 0, '<b>', '</b>', '…', 12) AS snip \
                     FROM companion_memories_fts \
                     JOIN companion_memories m ON m.id = companion_memories_fts.rowid \
                     WHERE companion_memories_fts MATCH ?{} \
                     ORDER BY bm25(companion_memories_fts) LIMIT ?",
                    filter.sql
                );
                let phrase = fts_phrase(term);
                let query = sqlx::query(&sql).bind(&phrase);
                let query = bind_filter(query, &filter).bind(limit as i64);
                (query.fetch_all(self.pool()).await.map_err(crate::store::db_err)?, true)
            } else {
                // Sub-trigram term: LIKE substring fallback (see module docs).
                let sql = format!(
                    "SELECT m.*, 0.0 AS bm25_score, NULL AS snip \
                     FROM companion_memories m \
                     WHERE m.content LIKE ? ESCAPE '\\'{} \
                     LIMIT ?",
                    filter.sql
                );
                let pattern = like_pattern(term);
                let query = sqlx::query(&sql).bind(&pattern);
                let query = bind_filter(query, &filter).bind(limit as i64);
                (query.fetch_all(self.pool()).await.map_err(crate::store::db_err)?, false)
            };
            for row in &rows {
                let memory = row_to_memory(row)?;
                let bm25: f64 = row.try_get("bm25_score").unwrap_or(0.0);
                let snippet: Option<String> = if is_fts { row.try_get("snip").ok() } else { None };
                match merged.get_mut(&memory.memory_id) {
                    Some((_, best, best_snippet)) => {
                        if bm25 < *best {
                            *best = bm25;
                            if snippet.is_some() {
                                *best_snippet = snippet;
                            }
                        } else if best_snippet.is_none() && snippet.is_some() {
                            *best_snippet = snippet;
                        }
                    }
                    None => {
                        merged.insert(memory.memory_id.clone(), (memory, bm25, snippet));
                    }
                }
            }
        }

        let mut hits: Vec<MemorySearchHit> = merged
            .into_values()
            .map(|(memory, bm25, snippet)| MemorySearchHit {
                rank: fused_rank(bm25, &memory),
                memory,
                snippet,
            })
            .collect();
        hits.sort_by(|a, b| {
            // Fused rank first; on exact ties the plan's precedence applies
            // explicitly (importance over strength — the 0.5/0.5 weights alone
            // cannot express it), then recency, then id for determinism.
            b.rank
                .partial_cmp(&a.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.memory
                        .importance
                        .partial_cmp(&a.memory.importance)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    b.memory
                        .strength
                        .partial_cmp(&a.memory.strength)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.memory.updated_at.cmp(&a.memory.updated_at))
                .then_with(|| a.memory.memory_id.cmp(&b.memory.memory_id))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryScope;
    use nomifun_common::CompanionMemoryId;

    fn companion_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        CompanionId::try_from(raw.as_str()).unwrap().into_string()
    }

    fn query(terms: &[&str]) -> MemorySearchQuery {
        MemorySearchQuery {
            queries: terms.iter().map(|t| (*t).to_owned()).collect(),
            ..MemorySearchQuery::default()
        }
    }

    /// A raw fixture with independent importance/strength/pinned (the public
    /// insert path couples strength to importance).
    fn raw_memory(content: &str, kind: &str, importance: f64, strength: f64, pinned: bool) -> CompanionMemory {
        CompanionMemory {
            memory_id: CompanionMemoryId::new().into_string(),
            kind: kind.into(),
            content: content.into(),
            tags: vec![],
            importance,
            strength,
            pinned,
            source: "manual".into(),
            status: "active".into(),
            created_at: 1,
            updated_at: 1,
            last_reinforced_at: 1,
            scope_kind: "user".into(),
            scope_companion_id: None,
        }
    }

    #[tokio::test]
    async fn chinese_substring_hits_via_trigram_and_short_term_fallback() {
        let store = CompanionStore::open_memory().await.unwrap();
        store
            .insert_memory("preference", "主人喜欢深烘焙咖啡豆", &[], 0.8, "manual")
            .await
            .unwrap();

        // 3+ chars → trigram MATCH, snippet carries highlight markers.
        let hits = store.search_memories(query(&["咖啡豆"])).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.as_deref().unwrap_or_default().contains("<b>"), "{hits:?}");

        // 2 chars is below the trigram floor → LIKE fallback still hits.
        let hits = store.search_memories(query(&["咖啡"])).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.content, "主人喜欢深烘焙咖啡豆");

        let miss = store.search_memories(query(&["不存在的词xyz"])).await.unwrap();
        assert!(miss.is_empty());
    }

    #[tokio::test]
    async fn multi_query_or_merges_and_dedups() {
        let store = CompanionStore::open_memory().await.unwrap();
        store.insert_memory("preference", "主人喜欢喝拿铁", &[], 0.8, "manual").await.unwrap();
        store.insert_memory("knowledge", "主人在用 Rust 写后端", &[], 0.8, "manual").await.unwrap();

        let hits = store.search_memories(query(&["拿铁", "Rust"])).await.unwrap();
        assert_eq!(hits.len(), 2);

        // Overlapping terms must not duplicate a hit.
        let hits = store.search_memories(query(&["拿铁", "喜欢喝拿铁"])).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn status_filter_selects_lifecycle_layer() {
        let store = CompanionStore::open_memory().await.unwrap();
        let active = store.insert_memory("episode", "上周去了咖啡展", &[], 0.8, "manual").await.unwrap();
        let archived = store.insert_memory("episode", "去年逛过咖啡庄园", &[], 0.8, "manual").await.unwrap();
        store.archive_memories(std::slice::from_ref(&archived.memory_id)).await.unwrap();

        let q_archived = MemorySearchQuery { status: MemoryStatusFilter::Archived, ..query(&["咖啡"]) };
        let hits = store.search_memories(q_archived).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.memory_id, archived.memory_id);

        let hits = store.search_memories(query(&["咖啡"])).await.unwrap();
        assert_eq!(hits.len(), 1, "default Active must exclude archived");
        assert_eq!(hits[0].memory.memory_id, active.memory_id);

        let q_all = MemorySearchQuery { status: MemoryStatusFilter::All, ..query(&["咖啡"]) };
        assert_eq!(store.search_memories(q_all).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn kind_and_visibility_filters() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = companion_fixture(1);
        let stranger = companion_fixture(2);
        // One vestigial unowned row (a pre-re-homing legacy memory) + one owned.
        store.insert_memory("preference", "主人喜欢手冲咖啡", &[], 0.8, "manual").await.unwrap();
        store
            .insert_memory_scoped("task", "帮主人试三种咖啡豆", &[], 0.8, "chat", MemoryScope::Companion(owner.clone()))
            .await
            .unwrap();

        // kind filter
        let q_kind = MemorySearchQuery { kind: Some("task".into()), ..query(&["咖啡"]) };
        let hits = store.search_memories(q_kind).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.kind, "task");

        // visibility: the owner sees its own + the unowned row; a stranger sees
        // only the unowned one — never another companion's memory.
        let visible = MemorySearchQuery {
            companion_id: Some(CompanionId::try_from(owner.as_str()).unwrap()),
            ..query(&["咖啡"])
        };
        assert_eq!(store.search_memories(visible).await.unwrap().len(), 2);
        let strangers = MemorySearchQuery {
            companion_id: Some(CompanionId::try_from(stranger.as_str()).unwrap()),
            ..query(&["咖啡"])
        };
        let hits = store.search_memories(strangers).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.content, "主人喜欢手冲咖啡");
    }

    #[tokio::test]
    async fn rank_fuses_pinned_importance_strength_over_equal_bm25() {
        let store = CompanionStore::open_memory().await.unwrap();
        // Identical content → identical BM25; only the boosts differ.
        let low = raw_memory("咖啡烘焙偏好档案", "preference", 0.2, 0.2, false);
        let strong = raw_memory("咖啡烘焙偏好档案", "preference", 0.2, 0.9, false);
        let important = raw_memory("咖啡烘焙偏好档案", "preference", 0.9, 0.2, false);
        let pinned = raw_memory("咖啡烘焙偏好档案", "preference", 0.2, 0.2, true);
        for m in [&low, &strong, &important, &pinned] {
            store.insert_memory_raw(m).await.unwrap();
        }

        let hits = store.search_memories(query(&["咖啡烘焙"])).await.unwrap();
        let order: Vec<&str> = hits.iter().map(|h| h.memory.memory_id.as_str()).collect();
        assert_eq!(
            order,
            vec![
                pinned.memory_id.as_str(),
                important.memory_id.as_str(), // ties strength on weight, wins over low
                strong.memory_id.as_str(),
                low.memory_id.as_str(),
            ],
            "pinned > higher importance/strength > baseline"
        );
        assert!(hits[0].rank > hits[1].rank && hits[1].rank >= hits[2].rank && hits[2].rank > hits[3].rank);
    }

    #[tokio::test]
    async fn limit_caps_result_count() {
        let store = CompanionStore::open_memory().await.unwrap();
        for i in 0..5 {
            store
                .insert_memory("knowledge", &format!("咖啡笔记第{i}篇：烘焙曲线"), &[], 0.8, "manual")
                .await
                .unwrap();
        }
        let q = MemorySearchQuery { limit: 3, ..query(&["咖啡笔记"]) };
        assert_eq!(store.search_memories(q).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn empty_queries_are_rejected() {
        let store = CompanionStore::open_memory().await.unwrap();
        for queries in [vec![], vec!["".to_owned()], vec!["   ".to_owned()]] {
            let q = MemorySearchQuery { queries, ..MemorySearchQuery::default() };
            assert!(
                matches!(store.search_memories(q).await, Err(AppError::BadRequest(_))),
                "empty query set must be a BadRequest"
            );
        }
    }
}
