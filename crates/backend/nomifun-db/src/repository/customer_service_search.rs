//! Hybrid recall over `cs_notes` — the retrieval half of customer-service note
//! search, and the external-content FTS5 index maintenance it depends on.
//!
//! # What replaced what
//!
//! Note search used to be `content LIKE '%query%'` ordered by `created_at
//! DESC`: one contiguous literal substring against a model-generated string,
//! with relevance absent from the ranking. It missed notes that plainly
//! existed — a note reading `Q：NomiFun是什么？` was not found by
//! `NomiFun 是什么`, because the inserted space broke contiguity.
//!
//! The replacement is a graded ladder over [`NoteQueryTerms`], which
//! [`nomifun_common::text_search::expand_query`] produces by normalizing and
//! splitting the query:
//!
//! 1. **FTS5 MATCH** on 3+ char terms, OR-combined, ranked by BM25.
//! 2. **LIKE scan** on sub-trigram terms, which `MATCH` cannot reach at all.
//! 3. **CJK bigrams**, consulted ONLY when 1 and 2 found nothing, capped and
//!    ranked by term overlap. Segregated because high-frequency two-character
//!    words like 「怎么」 match nearly any note: promoting them to a peer
//!    channel would trade a recall bug for a precision bug.
//!
//! Rungs 1 and 2 are merged (a query legitimately spans both channels — `AI`
//! is sub-trigram while `工作空间` is not); rung 3 is a fallback, not a peer.
//!
//! # The index is maintained by hand, and the delete contract bites
//!
//! The v3 schema forbids triggers, so every `cs_notes` write path calls
//! [`fts_index_insert`] / [`fts_index_delete`] explicitly — the same posture as
//! `nomifun-companion/src/store.rs:709-760`.
//!
//! fts5's `'delete'` command requires the value that was ORIGINALLY INDEXED.
//! Passing the new value instead raises `database disk image is malformed`,
//! **and a subsequent `'integrity-check'` still reports PASSED** — verified.
//! The note silently vanishes from the index, which is precisely the bug class
//! this module exists to eliminate. So an update must read the old row, delete
//! with the old value, update, and reinsert, inside one transaction; see
//! `SqliteCustomerServiceRepository::update_note`.

use nomifun_common::text_search::{
    NoteQueryTerms, fts_match_expression, like_pattern, normalize_for_search,
};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::CsNoteRow;

/// Hard cap on notes returned by one search, whatever the caller asks for.
/// Tool output is fed to a model, so an unbounded result set is a context leak.
pub const NOTE_SEARCH_LIMIT_CAP: usize = 25;

/// Cap on the low-precision bigram fallback rung. Deliberately tighter than
/// [`NOTE_SEARCH_LIMIT_CAP`]: these hits share as little as one two-character
/// word with the query, so a long list is noise rather than recall.
pub const BIGRAM_FALLBACK_LIMIT: usize = 5;

/// One ranked note hit.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CsNoteSearchHit {
    pub note: CsNoteRow,
    /// Higher is better. BM25-derived for FTS hits, overlap-derived for the
    /// LIKE and bigram channels (which have no BM25 of their own).
    pub rank: f64,
    /// Which ladder rung surfaced this note. Carried so the tool layer can tell
    /// the model that a hit is a weak fallback rather than a confident match.
    pub channel: NoteMatchChannel,
}

/// The ladder rung that surfaced a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteMatchChannel {
    /// FTS5 MATCH — a real relevance-ranked hit.
    FullText,
    /// LIKE substring scan for terms below the trigram floor.
    Substring,
    /// CJK bigram last resort. Weak by construction.
    Bigram,
}

/// The `search_text` column value for a note: its content plus the owner's
/// alternate phrasings, folded.
///
/// Materializing this is what makes the index and the query side fold
/// identically. FTS5's trigram tokenizer applies NO Unicode normalization, so
/// indexing raw `content` would leave `ＮomiFun`, `NOMIFUN` and `nomifun`
/// as distinct index terms and reintroduce the original bug for full-width and
/// mixed-case input.
///
/// `aliases` are folded into the SAME column rather than indexed separately
/// because they are additional ways of asking the same question: a hit on an
/// alias is a hit on the note, with no separate ranking meaning.
pub fn note_search_text(content: &str, aliases: &str) -> String {
    let mut combined = String::with_capacity(content.len() + aliases.len() + 1);
    combined.push_str(content);
    if !aliases.trim().is_empty() {
        combined.push('\n');
        combined.push_str(aliases);
    }
    normalize_for_search(&combined)
}

/// Index one `cs_notes` row into the FTS index.
///
/// `search_text` must be exactly what was stored in the row's `search_text`
/// column, or the later [`fts_index_delete`] cannot match it.
pub async fn fts_index_insert(
    tx: &mut Transaction<'_, Sqlite>,
    rowid: i64,
    search_text: &str,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO cs_notes_fts(rowid, search_text) VALUES(?, ?)")
        .bind(rowid)
        .bind(search_text)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Remove one row from the FTS index.
///
/// `old_search_text` MUST be the value currently indexed for `rowid`. See the
/// module docs: a mismatch corrupts the index without failing the integrity
/// check.
pub async fn fts_index_delete(
    tx: &mut Transaction<'_, Sqlite>,
    rowid: i64,
    old_search_text: &str,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO cs_notes_fts(cs_notes_fts, rowid, search_text) VALUES('delete', ?, ?)")
        .bind(rowid)
        .bind(old_search_text)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Rebuild the whole index from `cs_notes`.
///
/// Used by the boot backfill and as the recovery path for an index that has
/// drifted. One statement reindexes every row, so no per-row loop is needed.
pub async fn fts_rebuild(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::query("INSERT INTO cs_notes_fts(cs_notes_fts) VALUES('rebuild')")
        .execute(pool)
        .await?;
    Ok(())
}

/// Recompute `search_text` for every note whose folded text is stale, then
/// rebuild the index.
///
/// Idempotent, so it is safe to run on every boot. It exists because migration
/// 035 adds `search_text` with a `''` default: SQLite's own `lower()` cannot
/// fold CJK full-width forms, so the backfill must run through the single Rust
/// normalizer rather than forking the semantics into SQL.
///
/// Returns the number of rows rewritten.
pub async fn backfill_note_search_text(pool: &SqlitePool) -> Result<u64, DbError> {
    let rows = sqlx::query("SELECT id, content, aliases, search_text FROM cs_notes")
        .fetch_all(pool)
        .await?;

    let mut stale: Vec<(i64, String)> = Vec::new();
    for row in &rows {
        let id: i64 = row.try_get("id")?;
        let content: String = row.try_get("content")?;
        let aliases: String = row.try_get("aliases")?;
        let current: String = row.try_get("search_text")?;
        let expected = note_search_text(&content, &aliases);
        if current != expected {
            stale.push((id, expected));
        }
    }
    if stale.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    for (id, expected) in &stale {
        sqlx::query("UPDATE cs_notes SET search_text = ? WHERE id = ?")
            .bind(expected)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    // Rebuild rather than incrementally fix up: the pre-backfill index state is
    // unknown (rows may never have been indexed at all), and 'delete' requires
    // knowing the exact previously-indexed value, which we do not.
    fts_rebuild(pool).await?;
    Ok(stale.len() as u64)
}

/// Visibility predicate shared by every note read path: the agent's own notes
/// plus every shared note, enabled only.
///
/// Kept as one constant because losing either half is a silent correctness bug
/// — dropping `enabled` would surface retired answers, and dropping the
/// `IS NULL` arm would hide every shared note.
const NOTE_VISIBILITY: &str =
    "(n.cs_agent_id = ? OR n.cs_agent_id IS NULL) AND n.enabled = 1";

/// Column list aliased for the joined queries below.
const NOTE_COLUMNS_ALIASED: &str = "n.cs_note_id, n.cs_agent_id, n.kind, n.content, \
     n.aliases, n.enabled, n.created_at, n.updated_at";

fn row_to_note(row: &sqlx::sqlite::SqliteRow) -> Result<CsNoteRow, DbError> {
    Ok(CsNoteRow {
        cs_note_id: row.try_get("cs_note_id")?,
        cs_agent_id: row.try_get("cs_agent_id")?,
        kind: row.try_get("kind")?,
        content: row.try_get("content")?,
        aliases: row.try_get("aliases")?,
        enabled: row.try_get("enabled")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Fuse a BM25 score into a rank where higher is better.
///
/// `bm25()` is smaller-is-better, so it is negated (the same convention as the
/// companion store's memory search). FAQ notes get a small nudge because a
/// visitor question is more often answered by an FAQ than by an internal
/// script, and each additional matched term adds a little confidence.
fn fused_rank(bm25: f64, kind: &str, matched_terms: usize) -> f64 {
    -bm25 + if kind == "faq" { 0.5 } else { 0.0 } + (matched_terms as f64) * 0.25
}

/// Accumulator that keeps the best rank per note across channels.
#[derive(Default)]
struct HitAccumulator {
    hits: Vec<(String, CsNoteSearchHit)>,
}

impl HitAccumulator {
    fn offer(&mut self, note: CsNoteRow, rank: f64, channel: NoteMatchChannel) {
        let key = note.cs_note_id.clone();
        if let Some((_, existing)) = self.hits.iter_mut().find(|(id, _)| *id == key) {
            if rank > existing.rank {
                existing.rank = rank;
                existing.channel = channel;
            }
            return;
        }
        self.hits.push((key, CsNoteSearchHit { note, rank, channel }));
    }

    fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// Sort by rank, then by recency, then by id so the order is total and
    /// reproducible — a flaky ordering would make the eval suite flaky too.
    fn finish(mut self, limit: usize) -> Vec<CsNoteSearchHit> {
        self.hits.sort_by(|(_, a), (_, b)| {
            b.rank
                .partial_cmp(&a.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.note.updated_at.cmp(&a.note.updated_at))
                .then_with(|| a.note.cs_note_id.cmp(&b.note.cs_note_id))
        });
        self.hits.truncate(limit);
        self.hits.into_iter().map(|(_, hit)| hit).collect()
    }
}

/// Search the notes visible to one agent using the expanded query terms.
///
/// Returns an empty vector when `terms` carries no signal — a query that
/// expanded to nothing must match NOTHING, never everything.
pub async fn search_notes_hybrid(
    pool: &SqlitePool,
    cs_agent_id: &str,
    terms: &NoteQueryTerms,
    limit: usize,
) -> Result<Vec<CsNoteSearchHit>, DbError> {
    let limit = limit.clamp(1, NOTE_SEARCH_LIMIT_CAP);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut acc = HitAccumulator::default();

    // ── Rung 1: FTS5 MATCH, BM25-ranked.
    if let Some(expression) = fts_match_expression(&terms.fts) {
        let sql = format!(
            "SELECT {NOTE_COLUMNS_ALIASED}, bm25(cs_notes_fts) AS bm25_score \
             FROM cs_notes_fts \
             JOIN cs_notes n ON n.id = cs_notes_fts.rowid \
             WHERE cs_notes_fts MATCH ? AND {NOTE_VISIBILITY} \
             ORDER BY bm25(cs_notes_fts) LIMIT ?"
        );
        let rows = sqlx::query(&sql)
            .bind(&expression)
            .bind(cs_agent_id)
            .bind(NOTE_SEARCH_LIMIT_CAP as i64)
            .fetch_all(pool)
            .await?;
        for row in &rows {
            let note = row_to_note(row)?;
            let bm25: f64 = row.try_get("bm25_score").unwrap_or(0.0);
            let matched = count_matched_terms(&note, &terms.fts);
            let rank = fused_rank(bm25, &note.kind, matched);
            acc.offer(note, rank, NoteMatchChannel::FullText);
        }
    }

    // ── Rung 2: LIKE scan for sub-trigram terms MATCH cannot see.
    for term in &terms.like {
        let sql = format!(
            "SELECT {NOTE_COLUMNS_ALIASED} FROM cs_notes n \
             WHERE n.search_text LIKE ? ESCAPE '\\' AND {NOTE_VISIBILITY} \
             ORDER BY n.updated_at DESC, n.id DESC LIMIT ?"
        );
        let rows = sqlx::query(&sql)
            .bind(like_pattern(term))
            .bind(cs_agent_id)
            .bind(NOTE_SEARCH_LIMIT_CAP as i64)
            .fetch_all(pool)
            .await?;
        for row in &rows {
            let note = row_to_note(row)?;
            // No BM25 for a substring hit; rank on overlap alone, which keeps
            // it below any genuine full-text hit.
            let matched = count_matched_terms(&note, &terms.like);
            let rank = fused_rank(0.0, &note.kind, matched);
            acc.offer(note, rank, NoteMatchChannel::Substring);
        }
    }

    // ── Rung 3: bigram last resort, ONLY if nothing better was found.
    if acc.is_empty() && !terms.bigrams.is_empty() {
        let mut scored: Vec<(usize, CsNoteRow)> = Vec::new();
        let sql = format!(
            "SELECT {NOTE_COLUMNS_ALIASED} FROM cs_notes n WHERE {NOTE_VISIBILITY}"
        );
        let rows = sqlx::query(&sql).bind(cs_agent_id).fetch_all(pool).await?;
        for row in &rows {
            let note = row_to_note(row)?;
            let matched = count_matched_terms(&note, &terms.bigrams);
            if matched > 0 {
                scored.push((matched, note));
            }
        }
        // Rank purely by how many distinct bigrams overlap, so a note sharing
        // several two-character words outranks one sharing a single particle.
        scored.sort_by(|(a, an), (b, bn)| {
            b.cmp(a)
                .then_with(|| bn.updated_at.cmp(&an.updated_at))
                .then_with(|| an.cs_note_id.cmp(&bn.cs_note_id))
        });
        for (matched, note) in scored.into_iter().take(BIGRAM_FALLBACK_LIMIT) {
            let rank = matched as f64 * 0.25;
            acc.offer(note, rank, NoteMatchChannel::Bigram);
        }
    }

    Ok(acc.finish(limit))
}

/// How many distinct terms occur in the note's folded search text.
///
/// Recomputed from `content`/`aliases` rather than read from `search_text` so
/// the count is correct even if a caller passes a row fetched before a
/// backfill.
fn count_matched_terms(note: &CsNoteRow, terms: &[String]) -> usize {
    let haystack = note_search_text(&note.content, &note.aliases);
    terms.iter().filter(|term| haystack.contains(term.as_str())).count()
}

/// A compact topic index of the notes visible to one agent.
///
/// Returned instead of a bare "nothing found" so a model that guessed badly
/// can see what IS available and re-query. `one_shot.rs` allows up to
/// `MAX_TOOL_ROUNDS` tool calls per turn, so the retry is free — the model was
/// simply never told what else to try.
pub async fn list_note_topics(
    pool: &SqlitePool,
    cs_agent_id: &str,
    limit: usize,
) -> Result<Vec<String>, DbError> {
    let sql = format!(
        "SELECT {NOTE_COLUMNS_ALIASED} FROM cs_notes n WHERE {NOTE_VISIBILITY} \
         ORDER BY n.updated_at DESC, n.id DESC LIMIT ?"
    );
    let rows = sqlx::query(&sql)
        .bind(cs_agent_id)
        .bind(limit.clamp(1, NOTE_SEARCH_LIMIT_CAP) as i64)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| {
            let note = row_to_note(row)?;
            Ok(note_topic(&note.content))
        })
        .collect::<Result<Vec<String>, DbError>>()
}

/// A one-line topic label for a note: its first non-empty line, with a leading
/// `Q：`/`Q:` marker stripped, capped so a long note cannot dominate the list.
fn note_topic(content: &str) -> String {
    const MAX_TOPIC_CHARS: usize = 60;
    let first = content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let stripped = first
        .strip_prefix("Q：")
        .or_else(|| first.strip_prefix("Q:"))
        .or_else(|| first.strip_prefix("q："))
        .or_else(|| first.strip_prefix("q:"))
        .unwrap_or(first)
        .trim();
    if stripped.chars().count() <= MAX_TOPIC_CHARS {
        return stripped.to_owned();
    }
    let mut out: String = stripped.chars().take(MAX_TOPIC_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_text_folds_content_and_aliases_together() {
        let folded = note_search_text("Q：ＮomiFun是什么？", "这个软件\n介绍");
        assert!(folded.contains("nomifun"), "{folded}");
        assert!(folded.contains("这个软件"), "{folded}");
        // NFKC folded the full-width question mark.
        assert!(folded.contains('?'), "{folded}");
    }

    #[test]
    fn search_text_omits_blank_aliases_without_trailing_separator() {
        assert_eq!(note_search_text("abc", ""), "abc");
        assert_eq!(note_search_text("abc", "   "), "abc");
    }

    #[test]
    fn topic_strips_the_question_marker_and_caps_length() {
        assert_eq!(note_topic("Q：NomiFun是什么？\nA：……"), "NomiFun是什么？");
        assert_eq!(note_topic("Q: how to install\nA: ..."), "how to install");
        assert_eq!(note_topic("\n\n  第一行  \n第二行"), "第一行");
        assert_eq!(note_topic(""), "");
        let long = "很".repeat(100);
        let topic = note_topic(&long);
        assert!(topic.chars().count() <= 61, "{}", topic.chars().count());
        assert!(topic.ends_with('…'));
    }

    #[test]
    fn fused_rank_prefers_better_bm25_then_faq_then_overlap() {
        // bm25 is smaller-is-better, so -bm25 makes a lower raw score rank higher.
        assert!(fused_rank(-5.0, "faq", 1) > fused_rank(-1.0, "faq", 1));
        assert!(fused_rank(-1.0, "faq", 1) > fused_rank(-1.0, "script", 1));
        assert!(fused_rank(-1.0, "faq", 3) > fused_rank(-1.0, "faq", 1));
    }
}
