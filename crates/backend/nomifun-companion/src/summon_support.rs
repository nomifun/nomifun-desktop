//! In-session companion summon support (spec §设计 B2/B3, plan Task 2).
//!
//! Three pieces the summoned work session consumes, all read-only towards the
//! companion's memory store:
//!
//! 1. [`resolve_summon_context`] — resolves the summon's hand-picked
//!    `memory_ids` against the live store each turn (budgeted at
//!    [`SUMMON_CONTEXT_BUDGET`] chars, truncated at entry boundaries) so
//!    memory edits propagate without copying content into the conversation
//!    row. Exposed to the engine via [`SummonContextResolver`]
//!    (`nomi_agent::summon_tools::SummonContextSink`).
//! 2. [`SummonMemorySink`] — a read-only `CompanionMemorySink` over the
//!    A-track `CompanionStore::search_memories` contract, visibility locked to
//!    the summoned companion (shared + its own private memories). `save` and
//!    `recent_events` refuse: `save_memory` is never registered in a summoned
//!    session, and this sink fails closed even if it were.
//! 3. [`SummonSuggestionSink`] — the `propose_companion_memory` handler:
//!    writes a `companion_suggestions` card (kind
//!    [`SUMMON_MEMORY_SUGGESTION_KIND`], provenance `source="summon"` + the
//!    originating conversation id inside `action`), never touching
//!    `companion_memories`. The memory only materializes when the owner
//!    accepts the card (`CompanionService::decide_suggestion`).

use async_trait::async_trait;
use nomifun_ai_agent::{CompanionMemorySink, SummonContextSink, SummonProposalSink};
use nomifun_api_types::SummonConfig;
use nomifun_common::{AppError, CompanionId};

use crate::companion::format_date;
use crate::events::CompanionEventEmitter;
use crate::memory_search::{MemorySearchQuery, MemoryStatusFilter};
use crate::store::{CompanionStore, MEMORY_KINDS};

/// Character budget for the injected memory-snapshot section (spec §B2).
pub const SUMMON_CONTEXT_BUDGET: usize = 8000;

/// `companion_suggestions.kind` for memories proposed from a summoned session.
pub const SUMMON_MEMORY_SUGGESTION_KIND: &str = "companion_memory";

/// Resolve the summon's selected memory ids into the injectable snapshot
/// section. Live per-turn resolution: archived memories still resolve (tagged
/// `[已归档]`), deleted ids silently drop out, and the whole section obeys the
/// character budget with entry-boundary truncation. Empty selection → empty
/// string (the contributor's no-op path).
pub async fn resolve_summon_context(
    store: &CompanionStore,
    config: &SummonConfig,
) -> Result<String, AppError> {
    if config.memory_ids.is_empty() {
        return Ok(String::new());
    }
    let mut entries: Vec<String> = Vec::with_capacity(config.memory_ids.len());
    for memory_id in &config.memory_ids {
        let Some(memory) = store.get_memory(memory_id).await? else {
            continue; // deleted since selection — the snapshot just narrows
        };
        entries.push(format!(
            "- [{}|{}{}] {}\n",
            format_date(memory.created_at),
            memory.kind,
            if memory.status == "archived" { "|已归档" } else { "" },
            memory.content
        ));
    }
    if entries.is_empty() {
        return Ok(String::new());
    }

    let header = "## 召唤的伙伴记忆（只读参考）\n\
                  以下记忆由主人为本次任务挑选，实时读取自伙伴的记忆库，仅供参考，不可直接修改。\
                  需要补查时用 recall_memories；发现长期有价值的新事实时用 propose_companion_memory \
                  提议（主人确认后才入库）。\n";
    let mut out = String::from(header);
    let mut used = header.chars().count();
    let mut omitted = 0usize;
    for entry in &entries {
        let cost = entry.chars().count();
        if used + cost > SUMMON_CONTEXT_BUDGET {
            omitted += 1;
            continue;
        }
        out.push_str(entry);
        used += cost;
    }
    if omitted > 0 {
        out.push_str(&format!("（记忆快照超出 {SUMMON_CONTEXT_BUDGET} 字符预算，已省略 {omitted} 条。）\n"));
    }
    Ok(out)
}

/// Per-turn snapshot resolver handed to the engine's `SummonContextContributor`.
/// Resolver failures are logged and become `None` — a snapshot must never fail
/// a turn.
pub struct SummonContextResolver {
    store: CompanionStore,
    config: SummonConfig,
}

impl SummonContextResolver {
    pub fn new(store: CompanionStore, config: SummonConfig) -> Self {
        Self { store, config }
    }
}

#[async_trait]
impl SummonContextSink for SummonContextResolver {
    async fn resolve_context(&self) -> Option<String> {
        match resolve_summon_context(&self.store, &self.config).await {
            Ok(section) if section.is_empty() => None,
            Ok(section) => Some(section),
            Err(error) => {
                tracing::warn!(
                    target: "nomifun_companion",
                    error = %error,
                    companion_id = %self.config.companion_id,
                    "summon memory snapshot resolution failed; skipping this turn"
                );
                None
            }
        }
    }
}

/// Read-only `CompanionMemorySink` for a summoned work session: recall reuses
/// the A-track `search_memories` contract with visibility locked to the
/// summoned companion; every write path refuses.
pub struct SummonMemorySink {
    store: CompanionStore,
    companion_id: CompanionId,
}

impl SummonMemorySink {
    pub fn new(store: CompanionStore, companion_id: CompanionId) -> Self {
        Self { store, companion_id }
    }
}

#[async_trait]
impl CompanionMemorySink for SummonMemorySink {
    async fn recall(
        &self,
        _conversation_id: &str,
        queries: &[String],
        kind: Option<&str>,
        include_archived: bool,
        limit: usize,
    ) -> Result<String, String> {
        // Scope is fixed at construction to the summoned companion — the work
        // conversation id must never influence visibility.
        let query = MemorySearchQuery {
            queries: queries.to_vec(),
            kind: kind.map(str::to_owned),
            scope: None,
            status: if include_archived { MemoryStatusFilter::All } else { MemoryStatusFilter::Active },
            companion_id: Some(self.companion_id.clone()),
            limit: if limit == 0 { 20 } else { limit },
        };
        let hits = self.store.search_memories(query).await.map_err(|e| e.to_string())?;
        if hits.is_empty() {
            return Ok("没有找到相关记忆。".into());
        }
        let mut out = String::new();
        for hit in hits {
            let m = &hit.memory;
            out.push_str(&format!(
                "- [{}|{}|id:{}{}] {}\n",
                format_date(m.created_at),
                m.kind,
                m.memory_id,
                if m.status == "archived" { "|已归档" } else { "" },
                m.content
            ));
        }
        Ok(out)
    }

    async fn save(
        &self,
        _conversation_id: &str,
        _kind: &str,
        _content: &str,
        _tags: &[String],
    ) -> Result<String, String> {
        Err("召唤会话对伙伴记忆只读：请改用 propose_companion_memory 提议，由主人确认后入库。".into())
    }

    async fn recent_events(&self, _limit: usize) -> Result<String, String> {
        Err("召唤会话不提供最近事件。".into())
    }
}

/// `propose_companion_memory` backend: candidate memories become suggestion
/// cards, never direct memory writes (spec §B3 确认式回写).
pub struct SummonSuggestionSink {
    store: CompanionStore,
    emitter: CompanionEventEmitter,
    companion_id: CompanionId,
}

impl SummonSuggestionSink {
    pub fn new(
        store: CompanionStore,
        emitter: CompanionEventEmitter,
        companion_id: CompanionId,
    ) -> Self {
        Self { store, emitter, companion_id }
    }
}

#[async_trait]
impl SummonProposalSink for SummonSuggestionSink {
    async fn propose(
        &self,
        conversation_id: &str,
        kind: &str,
        content: &str,
        reason: &str,
    ) -> Result<String, String> {
        if !MEMORY_KINDS.contains(&kind) {
            return Err(format!("kind 必须是 {MEMORY_KINDS:?} 之一"));
        }
        let content = content.trim();
        if content.is_empty() {
            return Err("content 不能为空".into());
        }
        let reason = reason.trim();
        // Already an active memory → no card needed.
        match self.store.find_similar_active(kind, content).await {
            Ok(Some(_)) => return Ok("伙伴已有相似的活跃记忆，无需重复提议。".into()),
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        let title = format!("伙伴记忆提议（{kind}）");
        let body = format!("{content}\n\n提议理由：{reason}");
        // Pending-card dedup backstop (same pattern as the learner): touch the
        // existing card so repeated evidence re-floats it instead of stacking.
        match self
            .store
            .find_similar_suggestion(SUMMON_MEMORY_SUGGESTION_KIND, &title, &body)
            .await
        {
            Ok(Some(existing_id)) => {
                if let Err(e) = self.store.touch_suggestion(&existing_id).await {
                    tracing::warn!(target: "nomifun_companion", error = %e, suggestion_id = %existing_id, "touch duplicate summon memory proposal failed");
                }
                return Ok("已有相同提议等待主人确认，无需重复提交。".into());
            }
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        let action = serde_json::json!({
            "type": SUMMON_MEMORY_SUGGESTION_KIND,
            "companion_id": self.companion_id.as_str(),
            "memory_kind": kind,
            "content": content,
            "reason": reason,
            "source": "summon",
            "source_conversation_id": conversation_id,
        });
        let created = self
            .store
            .insert_suggestion(SUMMON_MEMORY_SUGGESTION_KIND, &title, &body, Some(&action))
            .await
            .map_err(|e| e.to_string())?;
        self.emitter.emit_suggestion_created(self.companion_id.as_str(), &created);
        Ok(format!("已生成建议卡（{kind}），等待主人确认后写入伙伴记忆：{content}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryScope;
    use nomifun_realtime::BroadcastEventBus;
    use std::sync::Arc;

    fn companion_fixture(sequence: u64) -> CompanionId {
        CompanionId::try_from(format!("0190f5fe-7c00-7a00-8abc-{sequence:012}").as_str()).unwrap()
    }

    fn summon_config(companion_id: &CompanionId, memory_ids: Vec<String>) -> SummonConfig {
        SummonConfig {
            companion_id: companion_id.as_str().to_owned(),
            memory_ids,
            skill_exclusions: vec![],
            summoned_at: 1,
        }
    }

    fn emitter() -> CompanionEventEmitter {
        CompanionEventEmitter::new(Arc::new(BroadcastEventBus::new(16)), "owner-a")
    }

    #[tokio::test]
    async fn resolve_summon_context_is_empty_without_selection() {
        let store = CompanionStore::open_memory().await.unwrap();
        let companion = companion_fixture(1);
        let out = resolve_summon_context(&store, &summon_config(&companion, vec![]))
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn resolve_summon_context_tags_archived_and_skips_deleted() {
        let store = CompanionStore::open_memory().await.unwrap();
        let companion = companion_fixture(1);
        let active = store
            .insert_memory("preference", "主人喜欢深烘焙咖啡豆", &[], 0.8, "manual")
            .await
            .unwrap();
        let archived = store
            .insert_memory("episode", "去年逛过咖啡庄园", &[], 0.8, "manual")
            .await
            .unwrap();
        store.archive_memories(std::slice::from_ref(&archived.memory_id)).await.unwrap();

        let config = summon_config(
            &companion,
            vec![
                active.memory_id.clone(),
                archived.memory_id.clone(),
                // Deleted/unknown id resolves to nothing, silently.
                "0190f5fe-7c00-7a00-8abc-00000000dead".into(),
            ],
        );
        let out = resolve_summon_context(&store, &config).await.unwrap();
        assert!(out.contains("召唤的伙伴记忆（只读参考）"));
        assert!(out.contains("主人喜欢深烘焙咖啡豆"));
        assert!(out.contains("去年逛过咖啡庄园"));
        assert!(out.contains("已归档"), "archived entries must be tagged: {out}");
        assert!(!out.contains("超出"), "no truncation note under budget");
    }

    #[tokio::test]
    async fn resolve_summon_context_truncates_at_entry_boundary() {
        let store = CompanionStore::open_memory().await.unwrap();
        let companion = companion_fixture(1);
        let mut ids = Vec::new();
        for i in 0..5 {
            let big = format!("咖啡笔记{i}：{}", "很长的内容".repeat(500)); // ~2500+ chars each
            let m = store.insert_memory("knowledge", &big, &[], 0.8, "manual").await.unwrap();
            ids.push(m.memory_id);
        }
        let out = resolve_summon_context(&store, &summon_config(&companion, ids))
            .await
            .unwrap();
        assert!(
            out.chars().count() <= SUMMON_CONTEXT_BUDGET + 100,
            "must stay near budget (note line may exceed slightly): {}",
            out.chars().count()
        );
        assert!(out.contains("已省略"), "truncation note required: {out}");
        // Entry-boundary truncation: every included entry is complete.
        for line in out.lines().filter(|l| l.starts_with("- [")) {
            assert!(line.ends_with("很长的内容") || line.contains("很长的内容"));
        }
    }

    #[tokio::test]
    async fn summon_memory_sink_is_scoped_and_read_only() {
        let store = CompanionStore::open_memory().await.unwrap();
        let summoned = companion_fixture(1);
        let stranger = companion_fixture(2);
        store.insert_memory("preference", "主人喜欢手冲咖啡", &[], 0.8, "manual").await.unwrap();
        store
            .insert_memory_scoped(
                "task",
                "帮主人试三种咖啡豆",
                &[],
                0.8,
                "chat",
                MemoryScope::Companion(summoned.as_str().to_owned()),
            )
            .await
            .unwrap();
        store
            .insert_memory_scoped(
                "task",
                "别的伙伴的咖啡私事",
                &[],
                0.8,
                "chat",
                MemoryScope::Companion(stranger.as_str().to_owned()),
            )
            .await
            .unwrap();

        let sink = SummonMemorySink::new(store.clone(), summoned.clone());
        let out = sink
            .recall("conv_w", &["咖啡".into()], None, false, 20)
            .await
            .unwrap();
        assert!(out.contains("主人喜欢手冲咖啡"), "shared memories visible: {out}");
        assert!(out.contains("帮主人试三种咖啡豆"), "own private memories visible: {out}");
        assert!(
            !out.contains("别的伙伴的咖啡私事"),
            "another companion's private memories must stay invisible: {out}"
        );

        let err = sink.save("conv_w", "preference", "x", &[]).await.unwrap_err();
        assert!(err.contains("propose_companion_memory"), "{err}");
        assert!(sink.recent_events(5).await.is_err());
    }

    #[tokio::test]
    async fn propose_writes_suggestion_card_not_memories() {
        let store = CompanionStore::open_memory().await.unwrap();
        let companion = companion_fixture(1);
        let sink = SummonSuggestionSink::new(store.clone(), emitter(), companion.clone());

        let out = sink
            .propose("conv_w", "preference", "主人喜欢 TDD 流程", "多次强调")
            .await
            .unwrap();
        assert!(out.contains("建议卡"), "{out}");

        let suggestions = store.list_suggestions(Some("new"), 10).await.unwrap();
        assert_eq!(suggestions.len(), 1);
        let card = &suggestions[0];
        assert_eq!(card.kind, SUMMON_MEMORY_SUGGESTION_KIND);
        let action = card.action.as_ref().unwrap();
        assert_eq!(action["source"], "summon");
        assert_eq!(action["source_conversation_id"], "conv_w");
        assert_eq!(action["memory_kind"], "preference");
        assert_eq!(action["companion_id"], companion.as_str());

        // The proposal must NOT touch companion_memories.
        assert_eq!(store.count_memories("active").await.unwrap(), 0);

        // Duplicate proposal dedups onto the pending card.
        let again = sink
            .propose("conv_w", "preference", "主人喜欢 TDD 流程", "多次强调")
            .await
            .unwrap();
        assert!(again.contains("无需重复"), "{again}");
        assert_eq!(store.list_suggestions(Some("new"), 10).await.unwrap().len(), 1);

        // Invalid kind refuses.
        assert!(sink.propose("conv_w", "bogus", "x", "y").await.is_err());
        assert!(sink.propose("conv_w", "task", "  ", "y").await.is_err());
    }
}
