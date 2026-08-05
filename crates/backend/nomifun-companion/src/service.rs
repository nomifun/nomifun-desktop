//! `CompanionService` bundles the shared config, the companion registry, the store,
//! collector stats, learner and the companion-thread manager into the single
//! object the routes layer talks to.

use std::path::PathBuf;
use std::sync::Arc;

use nomifun_common::{
    AppError, CompanionId, ProviderId, ProviderUsage, ProviderUsageFeature, SharedProviderLifecycleBarrier,
};
use nomifun_db::IProviderRepository;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::collector::{self, Collector, SharedConfig, SharedEventStoreLock};
use crate::archiver::Archiver;
use crate::companion::{CompanionThreads, build_companion_system_prompt};
use crate::events::CompanionEventEmitter;
use crate::evolution::{EvolutionEngine, NoopTranscriptSource};
use crate::gamify::level_for_xp;
use crate::learner::{CompanionCompleter, CompanionLearnResult, Learner};
use crate::memory_search::{MemorySearchQuery, MemoryStatusFilter};
use crate::profile::{CompanionProfileConfig, SharedCompanionConfig};
use crate::registry::{CompanionRegistry, json_merge_patch};
use crate::skill_sink::CompanionSkillStoreSink;
use crate::store::{
    CompanionThread, MemoryBatchAction, MemoryFilter, MemoryListSort, MemoryPage, MemoryScope,
    CompanionMemory, CompanionSkill, CompanionStore,
    memory_contents_similar,
};
use nomifun_extension::skill_service::{self, SkillPaths, SkillScope};
use nomifun_extension::constants::SKILL_MANIFEST_FILE;

/// Map the stored owner to the extension skill scope. `None` is only the
/// vestigial legacy row the boot re-homing has not claimed, whose body still
/// lives in the shared tree.
fn scope_for(scope_companion_id: Option<&str>) -> SkillScope {
    scope_companion_id
        .map(|id| SkillScope::Companion(id.to_owned()))
        .unwrap_or(SkillScope::Shared)
}

/// A skill registry row + its SKILL.md `description` (frontmatter), flattened for the UI list.
#[derive(Debug, Clone, Serialize)]
pub struct CompanionSkillView {
    #[serde(flatten)]
    pub skill: CompanionSkill,
    pub description: String,
}

/// One page of skill list rows enriched with their SKILL.md descriptions.
#[derive(Debug, Clone, Serialize)]
pub struct CompanionSkillViewPage {
    pub items: Vec<CompanionSkillView>,
    pub total: i64,
}

/// A skill registry row + its raw SKILL.md body, for the in-app editor.
#[derive(Debug, Clone, Serialize)]
pub struct CompanionSkillContent {
    pub skill: CompanionSkill,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct CompanionStatus {
    /// Which companion this status describes; `None` for the shared-only fallback.
    pub companion_id: Option<String>,
    pub xp: i64,
    pub level: i64,
    pub mood: String,
    pub memories_active: i64,
    pub memories_archived: i64,
    pub model_configured: bool,
    pub collect_any_enabled: bool,
}

/// "What I learned this week" digest for ONE companion: the skills it distilled
/// and the memories it recorded in the window. How many of those skills are
/// currently active was the 专精 badge's number and is deliberately not here.
#[derive(Debug, Serialize)]
pub struct CompanionWeeklyDigest {
    pub since_ms: i64,
    pub skills_learned: i64,
    pub memories_added: i64,
    pub new_skill_names: Vec<String>,
}

/// One day of a companion's readable history: the LOCAL calendar day, how many
/// visible messages it holds, and whether 会话归档 left a diary on it. The day key
/// is `YYYYMMDD`, identical in shape and timezone to `session_day`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompanionHistoryDay {
    pub day: String,
    pub message_count: i64,
    pub has_digest: bool,
}

#[derive(Debug, Serialize)]
pub struct SourceStats {
    pub source: String,
    pub today: u64,
    pub total: u64,
}

/// One suspected-duplicate cluster for the merge assistant: active memories of
/// one kind + one scope whose contents are normalized-similar. v1 carries no
/// LLM-drafted merged text (YAGNI) — the UI pre-fills from the members and the
/// user edits before confirming.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryMergeGroup {
    pub memories: Vec<CompanionMemory>,
}

/// One row of the REST memory list: the memory plus FTS extras (highlight
/// snippet + fused rank) when the page came from a full-text query.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryListItem {
    #[serde(flatten)]
    pub memory: CompanionMemory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<f64>,
}

/// The REST memory-list page (superset of the legacy `MemoryPage` wire shape).
#[derive(Debug, Clone, Serialize)]
pub struct MemoryListPage {
    pub items: Vec<MemoryListItem>,
    pub total: i64,
}

/// Cluster active memories into suspected-duplicate groups (same kind + same
/// OWNER, contents pairwise similar to an existing member). Groups of one are
/// dropped — there is nothing to merge. Grouping by owner is what stops the
/// merge assistant from ever offering to fuse two companions' memories into one.
fn group_similar_memories(memories: Vec<CompanionMemory>) -> Vec<MemoryMergeGroup> {
    let mut groups: Vec<Vec<CompanionMemory>> = Vec::new();
    for memory in memories {
        let slot = groups.iter_mut().find(|group| {
            let head = &group[0];
            head.kind == memory.kind
                && head.scope_kind == memory.scope_kind
                && head.scope_companion_id == memory.scope_companion_id
                && group
                    .iter()
                    .any(|member| memory_contents_similar(&member.content, &memory.content))
        });
        match slot {
            Some(group) => group.push(memory),
            None => groups.push(vec![memory]),
        }
    }
    groups
        .into_iter()
        .filter(|group| group.len() >= 2)
        .map(|memories| MemoryMergeGroup { memories })
        .collect()
}

/// Post-delete cascade hook for companion removal. Registered by the app assembly
/// (e.g. knowledge-binding cleanup wrapping `KnowledgeService`) so this crate
/// stays free of those dependencies. Implementations must swallow their own
/// failures (warn, never panic) — the companion is already gone when they run.
#[async_trait::async_trait]
pub trait CompanionCleanupHook: Send + Sync {
    async fn on_companion_deleted(&self, companion_id: &str);
    /// Called after a companion's chat model changed (best-effort). Lets the host
    /// react to the single-source-of-truth model switch — e.g. clear the IM
    /// channel sessions bound to this companion so they recreate with the new model
    /// on the next inbound message. Default no-op so existing hooks (knowledge
    /// cleanup) need not implement it.
    async fn on_companion_model_changed(&self, _companion_id: &str) {}
}

pub struct CompanionService {
    /// Canonical owner of host-control-plane companion conversations. Resolved
    /// once from the authoritative user repository and propagated explicitly to
    /// every conversation adapter; never inferred from a username or DB literal.
    authoritative_user_id: Arc<str>,
    /// Shared multi-companion home (`{data_dir}/companion/shared`): config + events + db.
    shared_dir: PathBuf,
    /// Cached ML assets (`{data_dir}/companion/models`): the MODNet matting model
    /// proxied + served locally (see [`crate::matting_model`]).
    models_dir: PathBuf,
    /// Serializes first-time matting-model downloads (one fetch for N callers).
    model_lock: Mutex<()>,
    /// Shared custom-figure library home (`{data_dir}/companion/figures`).
    figures_dir: PathBuf,
    /// Serializes figure-library index read-modify-write.
    figures_lock: Mutex<()>,
    config: SharedConfig,
    event_store_lock: SharedEventStoreLock,
    registry: Arc<CompanionRegistry>,
    pub(crate) store: CompanionStore,
    emitter: CompanionEventEmitter,
    learner: Arc<Learner>,
    /// Skill-evolution engine; held so on-demand drafting (learn-by-demonstration) can
    /// reach it, not just the background tick.
    evolution: Arc<EvolutionEngine>,
    /// Resolved skill paths (`{data_dir}/skills/...`), shared with the evolution
    /// engine and the agent-facing skill sink.
    skill_paths: Arc<SkillPaths>,
    /// Companion thread management (real nomi conversations). Unset when the
    /// host wires the companion without a conversation service (tests).
    companion: tokio::sync::OnceCell<CompanionThreads>,
    /// Session-window archiver; late-wired (with a real conversation port) in
    /// [`Self::attach_companion`], since it needs the conversation service.
    /// Held so a future "archive now" can reach it. Unset in tests.
    archiver: std::sync::OnceLock<Arc<Archiver>>,
    /// Delete-cascade hooks, late-wired by the app assembly (same pattern as
    /// `companion`). Empty when never set (tests).
    cleanup_hooks: std::sync::OnceLock<Vec<Arc<dyn CompanionCleanupHook>>>,
    provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
}

impl CompanionService {
    /// Construct the service from the v3 companion layout, open the shared
    /// store, scan the companion roster, and spawn background tasks.
    pub async fn start(
        data_dir: &std::path::Path,
        bus: Arc<nomifun_realtime::BroadcastEventBus>,
        owner_id: &str,
        completer: Arc<dyn CompanionCompleter>,
        skill_paths: Arc<SkillPaths>,
    ) -> Result<Arc<Self>, AppError> {
        Self::start_with_provider_lifecycle(
            data_dir,
            bus,
            owner_id,
            completer,
            skill_paths,
            None,
            None,
        )
        .await
    }

    pub async fn start_with_provider_lifecycle(
        data_dir: &std::path::Path,
        bus: Arc<nomifun_realtime::BroadcastEventBus>,
        owner_id: &str,
        completer: Arc<dyn CompanionCompleter>,
        skill_paths: Arc<SkillPaths>,
        provider_repo: Option<Arc<dyn IProviderRepository>>,
        provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
    ) -> Result<Arc<Self>, AppError> {
        let owner_id = owner_id.trim();
        if owner_id.is_empty() {
            return Err(AppError::Internal(
                "authoritative companion owner id must not be empty".into(),
            ));
        }
        let authoritative_user_id: Arc<str> = Arc::from(owner_id);
        let shared_dir = data_dir.join(crate::COMPANION_SHARED_REL_DIR);
        let companions_dir = data_dir.join(crate::COMPANION_COMPANIONS_REL_DIR);
        let models_dir = data_dir.join(crate::COMPANION_MODELS_REL_DIR);
        let figures_dir = data_dir.join(crate::COMPANION_FIGURES_REL_DIR);
        // 学习 / 进化 moved off this file onto each companion. The retired blocks are
        // read out here and consumed by the boot migration below; the file is only
        // rewritten without them once that has durably succeeded.
        let loaded_config = SharedCompanionConfig::load_migrating(&shared_dir)?;
        let config: SharedConfig = Arc::new(RwLock::new(loaded_config.config));
        let event_store_lock: SharedEventStoreLock = Arc::new(RwLock::new(()));
        let registry = Arc::new(CompanionRegistry::scan_with_provider_lifecycle(
            companions_dir,
            shared_dir.clone(),
            provider_repo.clone(),
            provider_lifecycle.clone(),
        )?);
        // Boot migration, part 1 of 2: every existing companion inherits the
        // retired install-wide 学习/进化 values, so nobody's behaviour changes on
        // upgrade. Must precede the provider audit and the retention pass below,
        // both of which read the per-companion config.
        registry
            .seed_learn_evolve_from_retired(&loaded_config.retired_learn_evolve)
            .await?;
        if loaded_config.needs_rewrite {
            config
                .read()
                .await
                .save(&shared_dir)
                .map_err(|error| {
                    AppError::Internal(format!(
                        "rewrite shared companion config without the retired learn/evolve blocks: {error}"
                    ))
                })?;
        }
        let live_companion_ids = registry
            .ids()
            .await
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        if let Some(default_companion_id) = config.read().await.default_companion_id.as_deref()
            && !live_companion_ids.contains(default_companion_id)
        {
            return Err(AppError::Internal(format!(
                "shared companion config references missing default companion '{}'",
                default_companion_id
            )));
        }
        // Existing side-store references are hard bindings too. Audit them
        // before any background worker can consume a model. Normal Provider
        // deletion takes the write side of the same barrier and therefore
        // cannot race this check.
        let _provider_guard = if let Some(barrier) = provider_lifecycle.as_ref() {
            Some(barrier.read().await)
        } else {
            None
        };
        registry.validate_provider_references_under_guard().await?;
        drop(_provider_guard);
        // The persistent v3 side-store is authoritative. Never hide a corrupt,
        // locked, or pre-v3 database behind a throwaway in-memory replacement:
        // doing so would make writes appear successful and then disappear.
        //
        // The owner is resolved from the roster that was just scanned, BEFORE the
        // store opens, because opening it runs the one-time re-homing of the
        // vestigial shared memories and skills onto that companion. It is always a
        // live roster member, so the reference audit right below still passes.
        let row_owner = {
            let default_companion_id = config.read().await.default_companion_id.clone();
            registry
                .resolve_row_owner(default_companion_id.as_deref())
                .await
        };
        let store = CompanionStore::open(&shared_dir, row_owner.as_deref()).await?;
        store.validate_companion_references(&live_companion_ids).await?;
        // Boot migration, part 2 of 2: the runtime state that became per-companion
        // with the settings — above all the two event cursors. A companion left at
        // the absent-row default of 0 would re-distill the whole retained history
        // on its first run (duplicate memories, unexpected LLM bill), and would
        // also pin the retention watermark to 0 forever.
        store
            .seed_companion_state_from_global(&registry.ids().await)
            .await?;
        collector::validate_event_store(&shared_dir)?;
        let startup_config = config.read().await.clone();
        let protected_after_ts =
            collector::active_consumer_watermark(&store, &registry.list().await).await?;
        collector::prune_event_store(
            &shared_dir,
            startup_config.collect.event_retention_days,
            startup_config.collect.event_max_storage_mb,
            protected_after_ts,
            0,
        )?;
        crate::figures::validate_store(&figures_dir)?;
        let live_figure_ids = crate::figures::id_set(&figures_dir)?;
        registry.validate_figure_references(&live_figure_ids).await?;
        let skills = store.list_all_skills().await?;
        // The rows were re-homed by the store's boot migration; their bodies still
        // sit in the legacy shared tree, and the audit right below is fail-closed.
        crate::skill_io::rehome_unowned_skill_dirs(&skill_paths, &skills).await?;
        crate::skill_io::validate_store(&skill_paths, &skills).await?;
        let emitter = CompanionEventEmitter::new(bus.clone(), authoritative_user_id.to_string());

        Collector::with_event_store_lock(
            shared_dir.clone(),
            config.clone(),
            store.clone(),
            registry.clone(),
            event_store_lock.clone(),
        )
        .spawn(bus);

        // One lock map shared by both loops' "run now" entry points and their
        // ticks, keyed by companion so one companion's run cannot serialize another's.
        let learner = Arc::new(Learner {
            companion_dir: shared_dir.clone(),
            store: store.clone(),
            registry: registry.clone(),
            completer: completer.clone(),
            emitter: emitter.clone(),
            run_locks: Arc::new(crate::learner::CompanionRunLocks::new()),
            event_store_lock: event_store_lock.clone(),
        });
        learner.clone().spawn();

        // Skill self-evolution engine (design §5): independent background loop that
        // mines repeated tool sequences and drafts reviewable skills. Shares the
        // collector event stream + completer with the learner but runs its own tick.
        let evolution = Arc::new(EvolutionEngine {
            companion_dir: shared_dir.clone(),
            store: store.clone(),
            registry: registry.clone(),
            completer: completer.clone(),
            emitter: emitter.clone(),
            skill_paths: skill_paths.clone(),
            // Real conversation-store-backed source is late-wired in `attach_companion`
            // (the conversation service is built after this). Noop = drafts degrade to
            // tool-name steps until then.
            transcript: std::sync::RwLock::new(Arc::new(NoopTranscriptSource)),
            run_locks: Arc::new(crate::learner::CompanionRunLocks::new()),
            event_store_lock: event_store_lock.clone(),
        });
        evolution.clone().spawn();

        Ok(Arc::new(Self {
            authoritative_user_id,
            shared_dir,
            models_dir,
            model_lock: Mutex::new(()),
            figures_dir,
            figures_lock: Mutex::new(()),
            config,
            event_store_lock,
            registry,
            store,
            emitter,
            learner,
            evolution,
            skill_paths,
            companion: tokio::sync::OnceCell::new(),
            archiver: std::sync::OnceLock::new(),
            cleanup_hooks: std::sync::OnceLock::new(),
            provider_lifecycle,
        }))
    }

    /// Late-wire the companion thread manager (depends on the conversation
    /// service, which is built after the companion service in app startup).
    pub fn attach_companion(
        &self,
        conversations: Arc<nomifun_conversation::ConversationService>,
        runtime_registry: Arc<dyn nomifun_ai_agent::AgentRuntimeRegistry>,
    ) {
        // Also wire the real transcript source so skill drafting rehydrates the actual
        // (redacted) session transcript from the conversation store — the durable single
        // source of truth — instead of degrading to tool-name steps.
        self.evolution.set_transcript(Arc::new(crate::evolution::ConversationTranscriptSource::new(
            conversations.conversation_repo().clone(),
        )));
        // Spawn the session-window archiver now that a real conversation port
        // exists. The loop no-ops every tick while `archive.enabled` is false
        // (opt-in), so an unconfigured install pays nothing. `OnceLock::set`
        // guards against a double-spawn if attach is ever called twice.
        if self.archiver.get().is_none() {
            let archiver = Arc::new(Archiver {
                store: self.store.clone(),
                config: self.config.clone(),
                registry: self.registry.clone(),
                // Reuse the learn completer + model — one background LLM config.
                completer: self.learner.completer.clone(),
                port: Arc::new(crate::archive_port::ConversationArchivePort::new(
                    self.authoritative_user_id.clone(),
                    conversations.clone(),
                )),
                run_lock: Arc::new(Mutex::new(())),
            });
            if self.archiver.set(archiver.clone()).is_ok() {
                archiver.spawn();
            }
        }
        let _ = self.companion.set(CompanionThreads {
            authoritative_user_id: self.authoritative_user_id.clone(),
            store: self.store.clone(),
            config: self.config.clone(),
            registry: self.registry.clone(),
            conversations,
            runtime_registry,
            skill_paths: self.skill_paths.clone(),
        });
    }

    /// Late-wire the delete-cascade hooks (depends on services built after
    /// the companion service in app startup, e.g. `KnowledgeService`). First call
    /// wins; later calls are ignored (`OnceLock` semantics).
    pub fn set_cleanup_hooks(&self, hooks: Vec<Arc<dyn CompanionCleanupHook>>) {
        let _ = self.cleanup_hooks.set(hooks);
    }

    /// Build the `CompanionMemorySink` the agent factory needs — gives every
    /// companion_session conversation the recall/save/recent-events tools.
    pub fn memory_sink(&self) -> Arc<dyn nomifun_ai_agent::CompanionMemorySink> {
        Arc::new(crate::companion::CompanionStoreSink {
            store: self.store.clone(),
            config: self.config.clone(),
            registry: self.registry.clone(),
            emitter: self.emitter.clone(),
            companion_dir: self.shared_dir.clone(),
            event_store_lock: self.event_store_lock.clone(),
        })
    }

    /// Build the `CompanionSkillSink` the agent factory needs — gives companion_session
    /// conversations the `companion_skill` tool + the per-turn when_to_use injection
    /// over the owning companion's self-evolved skills (design §7).
    pub fn skill_sink(&self) -> Arc<dyn nomifun_ai_agent::CompanionSkillSink> {
        Arc::new(CompanionSkillStoreSink {
            store: self.store.clone(),
            config: self.config.clone(),
            registry: self.registry.clone(),
            skill_paths: self.skill_paths.clone(),
        })
    }

    fn parse_summon_companion_id(companion_id: &str) -> Result<nomifun_common::CompanionId, AppError> {
        nomifun_common::CompanionId::try_from(companion_id)
            .map_err(|error| AppError::BadRequest(format!("invalid summon companion id: {error}")))
    }

    fn companion(&self) -> Result<&CompanionThreads, AppError> {
        self.companion
            .get()
            .ok_or_else(|| AppError::Internal("companion threads not wired".into()))
    }

    // ----- companions -----

    /// All companion profiles, oldest first.
    pub async fn list_companions(&self) -> Vec<CompanionProfileConfig> {
        self.registry.list().await
    }

    /// Every desktop-companion reference to `provider_id`: per companion, its chat
    /// model plus its own 学习 / 进化 models (one install-wide pair until 2026-08).
    /// Malformed provider IDs never match.
    /// The provider deletion coordinator invokes this while holding the shared
    /// lifecycle write guard; all parent checks below therefore observe the
    /// same deletion-critical snapshot.
    pub async fn providers_in_use(&self, provider_id: &str) -> Vec<ProviderUsage> {
        let Ok(provider_id) = ProviderId::try_from(provider_id) else {
            return Vec::new();
        };
        if let Err(error) = self
            .registry
            .validate_provider_references_under_guard()
            .await
        {
            return vec![ProviderUsage {
                feature: ProviderUsageFeature::DesktopCompanion,
                label: format!("桌面伙伴 Provider 引用审计失败（{error}）"),
                target_id: None,
            }];
        }
        let mut out = Vec::new();
        for p in self.list_companions().await {
            for (model, what) in [
                (p.model.as_ref(), None),
                (p.learn.model.as_ref(), Some("学习模型")),
                (p.evolve.model.as_ref(), Some("进化模型")),
            ] {
                if model.is_some_and(|model| model.provider_id == provider_id.as_str()) {
                    out.push(ProviderUsage {
                        feature: ProviderUsageFeature::DesktopCompanion,
                        label: match what {
                            None => p.name.clone(),
                            Some(what) => format!("{}·{what}", p.name),
                        },
                        target_id: Some(p.companion_id.clone()),
                    });
                }
            }
        }
        out
    }

    /// Create a companion. The first companion ever created automatically becomes the
    /// default companion (shared config saved + broadcast).
    pub async fn create_companion(&self, name: &str, character: &str) -> Result<CompanionProfileConfig, AppError> {
        let profile = self.registry.create(name, character).await?;
        // A brand-new companion starts reading the shared event spool from NOW.
        // Left at the absent-row default of 0 it would distill the entire retained
        // history on its first run — weeks of the owner's events re-summarised into
        // duplicate memories, on the owner's token budget — and would hold the
        // retention watermark at 0 until it caught up.
        for key in [collector::LEARN_CURSOR_KEY, collector::EVOLVE_CURSOR_KEY] {
            if let Err(error) = self
                .store
                .seed_companion_state(&profile.companion_id, key, &nomifun_common::now_ms().to_string())
                .await
            {
                tracing::warn!(
                    error = %error,
                    companion_id = %profile.companion_id,
                    key,
                    "seed new companion event cursor failed; it will start from the oldest retained event"
                );
            }
        }
        let updated_shared = {
            let mut cfg = self.config.write().await;
            if cfg.default_companion_id.is_none() {
                cfg.default_companion_id = Some(profile.companion_id.clone());
                if let Err(e) = cfg.save(&self.shared_dir) {
                    // The pointer survives in memory; warn rather than fail
                    // the creation (the companion itself is already persisted).
                    tracing::warn!(error = %e, "save shared companion config (default_companion_id) failed");
                }
                Some(cfg.clone())
            } else {
                None
            }
        };
        if let Some(cfg) = updated_shared {
            self.emitter.emit_shared_config_updated(&cfg);
        }
        self.emitter.emit_companion_created(&profile);
        // Auto-create the companion's single companion session, but only when its
        // model is already configured (a session can't be minted without one)
        // and the companion manager is wired (it isn't in tests). Best-effort:
        // a failure here must never fail companion creation — the session is lazily
        // ensured later when the UI calls POST .../companion/threads after a
        // model is set.
        if profile.model.is_some()
            && let Ok(companion) = self.companion()
            && let Err(e) = companion.create(&profile.companion_id, None).await
        {
            tracing::warn!(error = %e, companion_id = %profile.companion_id, "auto-create companion session failed; will be ensured lazily");
        }
        Ok(profile)
    }

    pub async fn get_companion(&self, id: &str) -> Result<CompanionProfileConfig, AppError> {
        self.registry
            .get(id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("companion '{id}' not found")))
    }

    /// Apply a server-resolved preset without replacing companion identity or
    /// learned state. The frozen snapshot is persisted on the profile so new
    /// companion sessions and remote channel turns reuse the same capability
    /// template even if the source preset is edited later.
    pub async fn apply_preset_snapshot(
        &self,
        id: &str,
        mut snapshot: nomifun_api_types::ResolvedPresetSnapshot,
    ) -> Result<CompanionProfileConfig, AppError> {
        if snapshot.target != nomifun_api_types::PresetTarget::Companion {
            return Err(AppError::BadRequest(
                "preset snapshot target must be companion".into(),
            ));
        }
        let resolved_model = snapshot.resolved_model.take();
        let mut patch = serde_json::json!({ "applied_preset": snapshot });
        if let Some(model) = resolved_model {
            if let Some(provider_id) = model.provider_id {
                patch["model"] = serde_json::json!({
                    "provider_id": provider_id,
                    "model": model.model,
                });
            }
        }
        let profile = self.patch_companion(id, patch).await?;
        self.propagate_preset_to_companion(&profile).await;
        Ok(profile)
    }

    /// RFC 7396 partial update of one companion's profile. When the patch changes the
    /// model into a new configured value, the new model (唯一事实源 =
    /// profile.model) is propagated to the companion's single companion conversation
    /// row so the next turn uses it — the conversation row `model` was only a
    /// create-time snapshot. If the companion had no session yet but the model just
    /// became configured, the session is auto-ensured (idempotent). All of the
    /// companion-side work is best-effort: it never fails the patch.
    pub async fn patch_companion(&self, id: &str, patch: serde_json::Value) -> Result<CompanionProfileConfig, AppError> {
        // Snapshot the pre-patch model so we can tell whether this patch
        // actually changed it (RFC 7396 patches need not mention `model`).
        let prev = self.registry.get(id).await;
        let prev_model = prev.as_ref().and_then(|p| p.model.clone());
        let prev_name = prev.as_ref().map(|p| p.name.clone());
        let prev_skills = prev.as_ref().map(|p| p.skills.clone());
        let profile = self.registry.patch(id, patch).await?;
        self.emitter.emit_companion_updated(&profile.companion_id, &profile);

        let model_changed = prev_model.as_ref() != profile.model.as_ref();
        if model_changed {
            if profile.model.is_some() {
                self.propagate_model_to_companion(&profile).await;
            }
            // 通知宿主：模型已切换（唯一事实源）。当前用于清理该伙伴绑定的
            // IM 渠道会话，使其下轮重建拾取新模型（或正确地因未配置而拒绝）。
            // best-effort，不阻断 patch。
            if let Some(hooks) = self.cleanup_hooks.get() {
                for hook in hooks {
                    hook.on_companion_model_changed(&profile.companion_id).await;
                }
            }
        }
        // 改名跟随：名字变了就把已存在的伙伴会话工作区目录迁到新 pretty 名
        // （best-effort，不为改名新建会话；agent 运行中占用则保留旧名下次再迁）。
        if prev_name.as_deref() != Some(profile.name.as_str()) {
            self.reconcile_companion_workspace(&profile).await;
        }
        if prev_skills.as_ref() != Some(&profile.skills) {
            self.reconcile_companion_skills(&profile).await;
        }
        Ok(profile)
    }

    /// Best-effort：把伙伴「已存在」会话的工作区目录收敛到当前名字。无会话则跳过
    /// （下次 create() 自然用新名）；companion 未接线（测试）则跳过。绝不阻断 patch。
    async fn reconcile_companion_workspace(&self, profile: &CompanionProfileConfig) {
        let Ok(companion) = self.companion() else { return };
        let threads = match companion.list(&profile.companion_id).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, companion_id = %profile.companion_id, "list threads for workspace reconcile failed");
                return;
            }
        };
        if let Some(thread) = threads.into_iter().next() {
            companion.reconcile_thread_workspace(profile, &thread.conversation_id).await;
        }
    }

    /// Best-effort: apply the profile's catalog Skill configuration to the
    /// existing companion conversation. A new conversation is handled by
    /// `CompanionThreads::create`; there is nothing to reconcile when no thread
    /// exists yet.
    async fn reconcile_companion_skills(&self, profile: &CompanionProfileConfig) {
        let Ok(companion) = self.companion() else { return };
        let threads = match companion.list(&profile.companion_id).await {
            Ok(threads) => threads,
            Err(error) => {
                tracing::warn!(error = %error, companion_id = %profile.companion_id, "list threads for skill reconcile failed");
                return;
            }
        };
        if let Some(thread) = threads.into_iter().next() {
            companion
                .reconcile_profile_skills(profile, &thread.conversation_id)
                .await;
        }
    }

    /// Best-effort: push the companion's configured model onto its single companion
    /// conversation row, auto-ensuring the session first if the model just
    /// became configured (so setting a model immediately gives the partner a
    /// usable session). Swallows every error (companion may be unwired in
    /// tests); a failure here must not fail the patch that triggered it.
    async fn propagate_model_to_companion(&self, profile: &CompanionProfileConfig) {
        let Some(model) = profile.model.as_ref() else { return };
        let Ok(companion) = self.companion() else { return };
        // Idempotent ensure: returns the existing session, or mints one now
        // that the model is configured. This also yields the conversation id
        // to retarget.
        let conversation_id = match companion.create(&profile.companion_id, None).await {
            Ok(thread) => thread.conversation_id,
            Err(e) => {
                tracing::warn!(error = %e, companion_id = %profile.companion_id, "ensure companion session for model propagation failed");
                return;
            }
        };
        if let Err(e) = companion.set_model(&profile.companion_id, &conversation_id, model).await {
            tracing::warn!(error = %e, companion_id = %profile.companion_id, "propagate model to companion conversation failed");
        }
    }

    /// Best-effort live propagation for an existing companion session. New
    /// sessions already consume `profile.applied_preset` in `create()`.
    async fn propagate_preset_to_companion(&self, profile: &CompanionProfileConfig) {
        let Some(snapshot) = profile.applied_preset.as_ref() else { return };
        let Ok(companion) = self.companion() else { return };
        let conversation_id = match companion.create(&profile.companion_id, None).await {
            Ok(thread) => thread.conversation_id,
            Err(error) => {
                tracing::warn!(%error, companion_id = %profile.companion_id, "ensure companion session for preset propagation failed");
                return;
            }
        };
        let system_prompt = build_companion_system_prompt(
            &self.store,
            profile,
            None,
            self.config.read().await.smart_collaboration,
        )
        .await;
        if let Err(error) = companion
            .set_preset(&profile.companion_id, &conversation_id, system_prompt, snapshot)
            .await
        {
            tracing::warn!(%error, companion_id = %profile.companion_id, "propagate preset to companion conversation failed");
        }
    }

    /// Delete a companion: cascade-delete its companion conversations, clear its
    /// per-companion store rows, remove the profile, and re-point the default companion
    /// if it was this one.
    pub async fn delete_companion(&self, id: &str) -> Result<(), AppError> {
        // Existence gate first so a bad id 404s before any side effect.
        self.get_companion(id).await?;
        // Cascade the companion's companion threads through the full delete path
        // (kills running agents + removes the real conversations). When the
        // companion manager isn't wired (tests), the thread rows still go
        // away below via delete_companion_rows. Any failure aborts the delete:
        // proceeding would drop the companion (and its thread registry rows) while
        // the conversations live on as orphans — the user can simply retry.
        let threads = self.store.list_companion_threads(Some(id)).await?;
        if let Ok(companion) = self.companion() {
            for t in &threads {
                companion.delete(id, &t.conversation_id).await.map_err(|e| {
                    AppError::Internal(format!(
                        "cascade-delete companion thread '{}' failed, companion kept: {e}",
                        t.conversation_id
                    ))
                })?;
            }
        }
        // T3.3 knowledge binding cleanup lives in the cleanup hooks below
        // (the app assembly registers a KnowledgeService-backed hook that
        // drops the ('companion', id) binding row).
        self.remove_companion_skill_trees(id)?;
        self.store.delete_companion_rows(id).await?;
        self.registry.remove(id).await?;
        // Post-removal cascade hooks. The companion is already gone, so a failing
        // hook must never fail the delete — implementations warn internally.
        if let Some(hooks) = self.cleanup_hooks.get() {
            for hook in hooks {
                hook.on_companion_deleted(id).await;
            }
        }
        // Default pointer: hand it to the oldest surviving companion (or clear it).
        let updated_shared = {
            let mut cfg = self.config.write().await;
            if cfg.default_companion_id.as_deref() == Some(id) {
                cfg.default_companion_id = self
                    .registry
                    .list()
                    .await
                    .first()
                    .map(|p| p.companion_id.clone());
                if let Err(e) = cfg.save(&self.shared_dir) {
                    tracing::warn!(error = %e, "save shared companion config (default_companion_id) failed");
                }
                Some(cfg.clone())
            } else {
                None
            }
        };
        if let Some(cfg) = updated_shared {
            self.emitter.emit_shared_config_updated(&cfg);
        }
        self.emitter.emit_companion_deleted(id);
        Ok(())
    }

    /// Companion-scoped skill bodies are durable children of the companion
    /// logical owner. Delete both active and draft trees before dropping the
    /// SQLite rows; if filesystem cleanup fails, the registry profile remains
    /// and the delete can be retried without hiding orphaned files.
    fn remove_companion_skill_trees(&self, companion_id: &str) -> Result<(), AppError> {
        crate::skill_io::remove_companion_trees(&self.skill_paths, companion_id)
    }

    /// One companion's status: its own xp/level/mood, its own memory counters,
    /// that companion's companion model flag.
    pub async fn companion_status(&self, id: &str) -> Result<CompanionStatus, AppError> {
        let profile = self.get_companion(id).await?;
        let cfg = self.config.read().await.clone();
        let xp = self.store.get_companion_state_i64(id, "xp").await?;
        Ok(CompanionStatus {
            companion_id: Some(profile.companion_id),
            xp,
            level: level_for_xp(xp),
            // Mood is this companion's own since 2026-08 (it was one global row, so
            // whichever loop finished last set the whole family's mood).
            mood: self
                .store
                .get_companion_state(id, crate::store::MOOD_KEY)
                .await?
                .unwrap_or_else(|| "content".into()),
            // Memory is owned per companion, and this snapshot is rendered per
            // companion: count only what THIS companion can read.
            memories_active: self.store.count_memories("active", Some(id)).await?,
            memories_archived: self.store.count_memories("archived", Some(id)).await?,
            model_configured: profile.model.is_some(),
            collect_any_enabled: cfg.collect.any_enabled(),
        })
    }

    /// Aggregate "what I learned this week" for the Overview digest card.
    pub async fn weekly_digest(&self, companion_id: &str, since_ms: i64) -> Result<CompanionWeeklyDigest, AppError> {
        let skills_learned = self.store.count_skills_since(companion_id, since_ms).await?;
        let memories_added = self.store.count_memories_since(since_ms, companion_id).await?;
        let new_skill_names = self.store.list_skill_names_since(companion_id, since_ms, 12).await?;
        Ok(CompanionWeeklyDigest {
            since_ms,
            skills_learned,
            memories_added,
            new_skill_names,
        })
    }

    /// The effective default companion: the shared pointer, or the oldest
    /// companion when no pointer has been configured. Startup rejects a stored
    /// dangling pointer, so a configured value is always live here.
    pub async fn default_companion_id(&self) -> Option<String> {
        let configured = self.config.read().await.default_companion_id.clone();
        if let Some(configured) = configured {
            return Some(configured);
        }
        self.registry.list().await.first().map(|p| p.companion_id.clone())
    }

    // ----- DIY custom figure (spec §3 存储与回显) -----

    /// Ingest an uploaded figure image for one companion (two-phase upload: the
    /// file already sits under the temp upload root). Unknown companion → 404
    /// before any filesystem work; validation failures map to 400/403.
    pub async fn ingest_figure(&self, companion_id: &str, source_path: &str) -> Result<(), AppError> {
        self.get_companion(companion_id).await?;
        crate::figure::ingest_figure(self.registry.companions_dir(), companion_id, std::path::Path::new(source_path))
    }

    /// One companion's stored figure bytes + mtime (unix seconds, the ETag input).
    /// Unknown companion or missing figure file → 404.
    pub async fn read_figure(&self, companion_id: &str) -> Result<(Vec<u8>, u64), AppError> {
        self.get_companion(companion_id).await?;
        crate::figure::read_figure(self.registry.companions_dir(), companion_id)
            ?
            .ok_or_else(|| AppError::NotFound(format!("companion '{companion_id}' has no custom figure")))
    }

    // ----- matting model proxy (fixes the DIY 30s-timeout dead-end) -----

    /// Ensure the MODNet matting model is cached on disk and return its bytes.
    /// Downloads from a mirror on first use (uncapped, so a slow 25 MB transfer
    /// completes instead of being killed by the old in-worker 30 s timeout).
    pub async fn matting_model_bytes(&self) -> Result<Vec<u8>, AppError> {
        let path = crate::matting_model::ensure_model(&self.models_dir, &self.model_lock).await?;
        tokio::fs::read(&path)
            .await
            .map_err(|e| AppError::Internal(format!("read matting model: {e}")))
    }

    // ----- custom-figure library (decoupled from companions) -----

    /// All saved library figures, newest first.
    pub async fn list_figures(&self) -> Result<Vec<crate::figures::FigureMeta>, AppError> {
        crate::figures::list(&self.figures_dir)
    }

    /// Ingest an uploaded cutout as a new reusable library figure.
    pub async fn create_figure(
        &self,
        source_path: &str,
        name: &str,
        aspect: f32,
        head_box: crate::profile::HeadBox,
        size_tier: &str,
    ) -> Result<crate::figures::FigureMeta, AppError> {
        let _guard = self.figures_lock.lock().await;
        crate::figures::create(
            &self.figures_dir,
            std::path::Path::new(source_path),
            name,
            aspect,
            head_box,
            size_tier,
        )
    }

    /// One library figure's image bytes + mtime (unix seconds). Unknown id → 404.
    pub async fn read_figure_image(&self, figure_id: &str) -> Result<(Vec<u8>, u64), AppError> {
        crate::figures::read_image(&self.figures_dir, figure_id)
            ?
            .ok_or_else(|| AppError::NotFound(format!("figure '{figure_id}' not found")))
    }

    /// Update editable library-figure metadata. Framing/size changes are synced
    /// into active custom companions that reference the library figure.
    pub async fn update_figure(
        &self,
        figure_id: &str,
        update: crate::figures::FigureUpdate,
    ) -> Result<crate::figures::FigureMeta, AppError> {
        let sync_users = update.head_box.is_some() || update.size_tier.is_some();
        let updated = {
            let _guard = self.figures_lock.lock().await;
            crate::figures::update(&self.figures_dir, figure_id, update)?
        };
        if sync_users {
            self.sync_figure_to_active_companions(&updated).await;
        }
        Ok(updated)
    }

    async fn sync_figure_to_active_companions(&self, figure: &crate::figures::FigureMeta) {
        let users: Vec<_> = self
            .registry
            .list()
            .await
            .into_iter()
            .filter(|p| {
                p.character == "custom"
                    && p.appearance
                        .custom_figure
                        .as_ref()
                        .and_then(|cf| cf.figure_id.as_deref())
                        == Some(figure.figure_id.as_str())
            })
            .collect();
        for profile in users {
            let patch = serde_json::json!({
                "appearance": {"custom_figure": {
                    "aspect": figure.aspect,
                    "head_box": {"x": figure.head_box.x, "y": figure.head_box.y, "w": figure.head_box.w, "h": figure.head_box.h},
                    "size_tier": figure.size_tier.clone(),
                    "figure_id": figure.figure_id.clone(),
                }},
            });
            if let Err(e) = self.patch_companion(&profile.companion_id, patch).await {
                tracing::warn!(error = %e, companion_id = %profile.companion_id, figure_id = %figure.figure_id, "sync updated library figure metadata to companion failed");
            }
        }
    }

    /// Number of hard profile references to this library figure. A stored
    /// `figure_id` remains a logical binding even while a built-in character is
    /// selected; callers must explicitly clear or replace it before deletion.
    async fn figure_user_count(&self, figure_id: &str) -> usize {
        self.registry
            .list()
            .await
            .iter()
            .filter(|p| {
                p.appearance
                    .custom_figure
                    .as_ref()
                    .and_then(|cf| cf.figure_id.as_deref())
                    == Some(figure_id)
            })
            .count()
    }

    /// Delete a library figure (image + index entry). Unknown id → 404. A figure
    /// still referenced by a companion is refused (`Conflict`): deleting it would leave
    /// that companion's `custom_figure.figure_id` dangling and its window image 404ing.
    /// Only unused figures may be deleted.
    pub async fn delete_figure(&self, figure_id: &str) -> Result<(), AppError> {
        let _guard = self.figures_lock.lock().await;
        let users = self.figure_user_count(figure_id).await;
        if users > 0 {
            return Err(AppError::Conflict(format!(
                "figure '{figure_id}' is used by {users} companion(s) and cannot be deleted"
            )));
        }
        crate::figures::remove(&self.figures_dir, figure_id)
    }

    // ----- companion session (per companion, single session) -----

    /// Idempotent ensure of the companion's single companion session.
    pub async fn create_companion_thread(
        &self,
        companion_id: &str,
        title: Option<String>,
    ) -> Result<CompanionThread, AppError> {
        self.companion()?.create(companion_id, title).await
    }

    pub async fn companion_active_thread(&self, companion_id: &str) -> Result<Option<String>, AppError> {
        self.companion()?.active_thread_id(companion_id).await
    }

    // ----- shared config -----

    pub async fn get_config(&self) -> SharedCompanionConfig {
        self.config.read().await.clone()
    }

    /// RFC 7396-style partial update: merge `patch` over the current shared
    /// config under the write lock. Lets concurrent writers (settings
    /// toggles, default-companion switches) update disjoint fields without
    /// clobbering each other the way full-object PUTs do.
    pub async fn patch_config(&self, patch: serde_json::Value) -> Result<SharedCompanionConfig, AppError> {
        if !patch.is_object() {
            return Err(AppError::BadRequest("config patch must be a JSON object".into()));
        }
        // Provider lifecycle barrier precedes the shared-config lock. Provider
        // deletion holds the write side, then scans this config.
        let _provider_guard = if let Some(barrier) = self.provider_lifecycle.as_ref() {
            Some(barrier.read().await)
        } else {
            None
        };
        // Event-store lock precedes config everywhere. Keeping it across the
        // merge, save and optional prune prevents a collector append or
        // competing policy PATCH from observing a stale capacity.
        let _event_guard = self.event_store_lock.write().await;
        let (merged, storage_policy_changed) = {
            let mut cfg = self.config.write().await;
            let mut value = serde_json::to_value(&*cfg)
                .map_err(|e| AppError::Internal(format!("serialize shared companion config: {e}")))?;
            json_merge_patch(&mut value, &patch);
            let merged: SharedCompanionConfig =
                serde_json::from_value(value).map_err(|e| AppError::BadRequest(format!("invalid config patch: {e}")))?;
            merged
                .collect
                .validate_storage_policy()
                .map_err(AppError::BadRequest)?;
            self.validate_default_companion_reference(&merged).await?;
            let storage_policy_changed = cfg.collect.event_retention_days
                != merged.collect.event_retention_days
                || cfg.collect.event_max_storage_mb != merged.collect.event_max_storage_mb;
            // Persist the policy before performing any destructive cleanup.
            // If this save fails, the PATCH has no in-memory effect and no raw
            // event file has been removed. Cleanup errors after a successful
            // save are retryable maintenance failures: the collector already
            // enforces the committed hard cap before every subsequent append.
            merged
                .save(&self.shared_dir)
                .map_err(|e| AppError::Internal(format!("save shared companion config: {e}")))?;
            *cfg = merged.clone();
            (merged, storage_policy_changed)
        };
        if storage_policy_changed {
            match collector::active_consumer_watermark(&self.store, &self.registry.list().await)
                .await
            {
                Ok(protected_after_ts) => {
                    if let Err(error) = collector::prune_event_store(
                        &self.shared_dir,
                        merged.collect.event_retention_days,
                        merged.collect.event_max_storage_mb,
                        protected_after_ts,
                        0,
                    ) {
                        tracing::warn!(
                            error = %error,
                            "companion event cleanup after committed storage-policy update failed; will retry"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "companion event cleanup could not read consumer cursors after committed storage-policy update; will retry"
                    );
                }
            }
        }
        self.emitter.emit_shared_config_updated(&merged);
        Ok(merged)
    }

    async fn validate_default_companion_reference(
        &self,
        config: &SharedCompanionConfig,
    ) -> Result<(), AppError> {
        let Some(default_companion_id) = config.default_companion_id.as_deref() else {
            return Ok(());
        };
        if self.registry.get(default_companion_id).await.is_none() {
            return Err(AppError::NotFound(format!(
                "default companion '{}' not found",
                default_companion_id
            )));
        }
        Ok(())
    }


    /// First-launch consent: apply self-evolution default-ON exactly once (design §9, 默认开).
    /// Turns work-source collection ON via `patch_config` (atomic save + emit + live Arc
    /// propagation) and 学习 + 进化 ON for every companion in the roster, guarded by a
    /// one-time global KV flag so it NEVER re-applies and never re-enables after the user
    /// later turns things off. Raw `Default` impls stay `false` (existing users are never
    /// silently enabled by a serde back-fill).
    pub async fn apply_default_on_consent(&self) -> Result<SharedCompanionConfig, AppError> {
        const CONSENT_KEY: &str = "self_evolution_consent";
        if self.store.get_state(CONSENT_KEY).await?.is_some() {
            return Ok(self.config.read().await.clone()); // idempotent: already consented
        }
        // Default-on set applied once on first consent. Skill mining keys off
        // `tool_calls` and memory distillation off owner-authored inputs.
        let patch = serde_json::json!({
            "collect": {
                "tool_calls": true,
                "chat_user_messages": true,
                "requirements": true
            }
        });
        let cfg = self.patch_config(patch).await?;
        self.set_learning_enabled_for_every_companion(true).await?;
        self.store.set_state(CONSENT_KEY, "1").await?;
        Ok(cfg)
    }

    /// Master kill switch (design §9, 一键全关): stop ALL collection (incl. `companion_dialogues`,
    /// which `any_enabled()` deliberately excludes) plus every companion's learning and
    /// evolution. Leaves models/intervals intact so re-enable needs no reconfiguration, and
    /// does NOT clear the consent flag (a user who explicitly disabled is never silently
    /// re-enabled). Already-collected events remain governed by the automatic retention and
    /// capacity policy.
    ///
    /// The two halves cannot be one atomic write any more — collection is one shared
    /// file, learning is N profiles. Collection is turned off FIRST so the worst
    /// interleaving leaves loops running over a spool nothing is adding to, rather
    /// than collection running with no consumer to advance the retention watermark.
    pub async fn disable_all(&self) -> Result<SharedCompanionConfig, AppError> {
        let patch = serde_json::json!({
            "collect": {
                "chat_user_messages": false,
                "requirements": false,
                "terminal_sessions": false,
                "tool_calls": false,
                "companion_dialogues": false
            }
        });
        let cfg = self.patch_config(patch).await?;
        self.set_learning_enabled_for_every_companion(false).await?;
        Ok(cfg)
    }

    /// Flip `learn.enabled` + `evolve.enabled` on every companion, emitting one
    /// profile-updated event each so live surfaces follow.
    async fn set_learning_enabled_for_every_companion(
        &self,
        enabled: bool,
    ) -> Result<(), AppError> {
        let patch = serde_json::json!({
            "learn": { "enabled": enabled },
            "evolve": { "enabled": enabled }
        });
        for companion_id in self.registry.ids().await {
            self.patch_companion(&companion_id, patch.clone()).await?;
        }
        Ok(())
    }

    // ----- status -----

    /// Aggregate status: the default companion's status. With no companions at
    /// all, return a zeroed shared-only snapshot (xp 0 / level 1 / no model).
    pub async fn status(&self) -> Result<CompanionStatus, AppError> {
        if let Some(companion_id) = self.default_companion_id().await {
            return self.companion_status(&companion_id).await;
        }
        let cfg = self.config.read().await.clone();
        Ok(CompanionStatus {
            companion_id: None,
            xp: 0,
            level: level_for_xp(0),
            // No companion exists to have a mood of its own.
            mood: "content".into(),
            // No companion exists to own anything: the only rows left are
            // vestigial unowned ones, so the unscoped count IS the honest total.
            memories_active: self.store.count_memories("active", None).await?,
            memories_archived: self.store.count_memories("archived", None).await?,
            model_configured: false,
            collect_any_enabled: cfg.collect.any_enabled(),
        })
    }

    // ----- memories -----

    pub async fn list_memories(&self, filter: &MemoryFilter) -> Result<Vec<CompanionMemory>, AppError> {
        self.store.list_memories(filter).await
    }

    /// Non-FTS list with an explicit sort (the REST `sort` param without `q`).
    pub async fn list_memory_page_sorted(&self, filter: &MemoryFilter, sort: MemoryListSort) -> Result<MemoryPage, AppError> {
        self.store.list_memory_page_sorted(filter, sort).await
    }

    /// FTS-backed memory list page (`q` present): full-text hits with snippet +
    /// rank, re-sorted per `sort` ('relevance' keeps the fused-rank order),
    /// paginated in memory over a capped hit set.
    pub async fn search_memory_page(
        &self,
        q: &str,
        kind: Option<String>,
        status: MemoryStatusFilter,
        scope_companion_id: Option<String>,
        sort: &str,
        limit: i64,
        offset: i64,
    ) -> Result<MemoryListPage, AppError> {
        let companion_id = scope_companion_id
            .as_deref()
            .map(|id| {
                CompanionId::try_from(id).map_err(|error| {
                    AppError::BadRequest(format!("invalid scope_companion_id: {error}"))
                })
            })
            .transpose()?;
        let query = MemorySearchQuery {
            queries: vec![q.to_owned()],
            kind,
            status,
            companion_id,
            limit: 500,
        };
        let mut hits = self.store.search_memories(query).await?;
        match sort {
            "time" => hits.sort_by(|a, b| b.memory.updated_at.cmp(&a.memory.updated_at)),
            "importance" => hits.sort_by(|a, b| {
                (b.memory.pinned as i64)
                    .cmp(&(a.memory.pinned as i64))
                    .then_with(|| {
                        b.memory
                            .importance
                            .partial_cmp(&a.memory.importance)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| b.memory.updated_at.cmp(&a.memory.updated_at))
            }),
            _ => {} // relevance: keep the search ranking
        }
        let total = hits.len() as i64;
        let limit = if limit <= 0 { 100 } else { limit.min(500) } as usize;
        let offset = offset.max(0) as usize;
        let items = hits
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|hit| MemoryListItem {
                memory: hit.memory,
                snippet: hit.snippet,
                rank: Some(hit.rank),
            })
            .collect();
        Ok(MemoryListPage { items, total })
    }

    /// Atomic batch memory operation + live per-row surface refresh events.
    pub async fn batch_memories(&self, ids: &[String], action: &MemoryBatchAction) -> Result<(), AppError> {
        self.store.batch_update_memories(ids, action).await?;
        for id in ids {
            match self.store.get_memory(id).await {
                Ok(Some(updated)) => self.emitter.emit_memory_updated(&updated),
                Ok(None) => self.emitter.emit_memory_deleted(id),
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// Merge-assistant dry run: suspected-duplicate groups over the ACTIVE
    /// layer (per kind + scope, normalized-similarity clustering).
    pub async fn memory_merge_suggestions(&self) -> Result<Vec<MemoryMergeGroup>, AppError> {
        let active: Vec<CompanionMemory> = self
            .store
            .dump_memories_all()
            .await?
            .into_iter()
            .filter(|memory| memory.status == "active")
            .collect();
        Ok(group_similar_memories(active))
    }

    /// Merge-assistant confirm: persist the merged memory, archive the source
    /// group (audit-tagged `superseded_by:{id}`), and notify open surfaces.
    pub async fn merge_memories(&self, group: &[String], merged_content: &str, kind: &str) -> Result<CompanionMemory, AppError> {
        let merged = self.store.merge_memories(group, merged_content, kind).await?;
        self.emitter.emit_memory_created(&merged);
        for id in group {
            if let Ok(Some(archived)) = self.store.get_memory(id).await {
                self.emitter.emit_memory_updated(&archived);
            }
        }
        Ok(merged)
    }

    // ----- session-window day digests (伙伴会话归档回看) -----

    /// The complete day index of this companion's history: every LOCAL calendar
    /// day its conversation holds visible messages on, plus every day that has an
    /// archived digest, newest first.
    ///
    /// Strictly read-only. The conversation is RESOLVED from the stored pointer,
    /// never minted: `create_companion_thread` 400s for a companion with no model
    /// configured, and a history reader must never fail for that reason. A
    /// companion that has never chatted — or whose conversation was deleted
    /// out-of-band, leaving a dangling pointer — yields an empty index rather
    /// than an error.
    pub async fn history_day_index(&self, companion_id: &str) -> Result<Vec<CompanionHistoryDay>, AppError> {
        // Existence gate: an unknown companion must 404, not read as "no history".
        self.get_companion(companion_id).await?;
        let digest_days: std::collections::HashSet<String> = self
            .store
            .archived_digest_days(companion_id)
            .await?
            .into_iter()
            .collect();
        let counts = match crate::companion::active_thread_ptr(&self.store, companion_id).await? {
            Some(conversation_id) => {
                match self
                    .companion()?
                    .conversations
                    .message_local_day_index(self.authoritative_user_id.as_ref(), &conversation_id)
                    .await
                {
                    Ok(buckets) => buckets,
                    // Dangling pointer to a conversation deleted out-of-band: the
                    // digests this companion already produced are still real history.
                    Err(AppError::NotFound(_)) => Vec::new(),
                    Err(error) => return Err(error),
                }
            }
            None => Vec::new(),
        };
        // A day carrying only a digest (its messages were cleared) must still be
        // reachable, so the index is the union of both sources.
        let mut days: std::collections::BTreeMap<String, i64> = digest_days
            .iter()
            .map(|day| (day.clone(), 0))
            .collect();
        for bucket in counts {
            days.insert(bucket.day, bucket.message_count);
        }
        Ok(days
            .into_iter()
            .rev()
            .map(|(day, message_count)| CompanionHistoryDay {
                has_digest: digest_days.contains(&day),
                day,
                message_count,
            })
            .collect())
    }

    /// Archived day-digests for one companion. `since`/`until` are inclusive
    /// `YYYYMMDD` bounds (empty = open). When both are empty, returns the most
    /// recent `limit` digests (newest first); otherwise the range (ascending).
    pub async fn list_day_digests(
        &self,
        companion_id: &str,
        since: &str,
        until: &str,
        limit: i64,
    ) -> Result<Vec<crate::store::SessionWindow>, AppError> {
        if since.is_empty() && until.is_empty() {
            self.store.list_digests(companion_id, limit).await
        } else {
            self.store.digests_in_range(companion_id, since, until).await
        }
    }

    /// "去年今日" — archived digests whose day-of-year (`MMDD`) matches, excluding
    /// today. `mmdd` is a 4-char `MMDD`.
    pub async fn digests_on_this_day(
        &self,
        companion_id: &str,
        mmdd: &str,
        exclude_day: &str,
        limit: i64,
    ) -> Result<Vec<crate::store::SessionWindow>, AppError> {
        self.store.digests_on_day_of_year(companion_id, mmdd, exclude_day, limit).await
    }
    /// The single owner every ownerless memory write lands on: the explicit
    /// default companion, else the oldest companion, else `None` on an empty
    /// roster (no legal owner — the caller must refuse rather than write an
    /// orphan). 共享记忆已删除，所以任何写入方都必须先问过这里。
    pub async fn resolve_memory_owner(&self) -> Option<String> {
        let default_companion_id = self.config.read().await.default_companion_id.clone();
        self.registry
            .resolve_row_owner(default_companion_id.as_deref())
            .await
    }

    /// Add a memory owned by `companion_id`, or by the resolved owner when the
    /// caller has no companion of its own (the MCP owner-agent write path).
    pub async fn add_memory(
        &self,
        kind: &str,
        content: &str,
        tags: &[String],
        companion_id: Option<&str>,
    ) -> Result<CompanionMemory, AppError> {
        if !crate::store::MEMORY_KINDS.contains(&kind) {
            return Err(AppError::BadRequest(format!("invalid memory kind '{kind}'")));
        }
        let content = content.trim();
        if content.is_empty() {
            return Err(AppError::BadRequest("memory content is empty".into()));
        }
        // A memory may only be owned by a LIVE companion: an unknown owner would
        // become an orphaned reference and hard-fail the next boot.
        let owner = match companion_id {
            Some(companion_id) => {
                self.get_companion(companion_id).await?;
                companion_id.to_owned()
            }
            None => self.resolve_memory_owner().await.ok_or_else(|| {
                AppError::BadRequest("还没有伙伴，无法保存记忆：请先创建一个伙伴。".into())
            })?,
        };
        // Dedup-merge (parity with the companion sink's save): a similar active
        // memory OF THIS OWNER is reinforced and returned instead of inserting a
        // near-duplicate. Owner-scoped, so one companion's add is never folded
        // into another companion's memory.
        if let Some(id) = self.store.find_similar_active(kind, content, &owner).await? {
            self.store.reinforce_memories(std::slice::from_ref(&id)).await?;
            if let Some(existing) = self.store.get_memory(&id).await? {
                return Ok(existing);
            }
        }
        let mem = self
            .store
            .insert_memory_scoped(kind, content, tags, 0.8, "manual", MemoryScope::Companion(owner))
            .await?;
        self.emitter.emit_memory_created(&mem);
        Ok(mem)
    }

    /// Edit a memory's content / pin / lifecycle. Ownership is immutable: there
    /// is no re-homing wire any more, so an edit can never move a memory between
    /// companions.
    pub async fn update_memory(
        &self,
        memory_id: &str,
        content: Option<&str>,
        pinned: Option<bool>,
        status: Option<&str>,
    ) -> Result<(), AppError> {
        if let Some(status) = status {
            if status != "active" && status != "archived" {
                return Err(AppError::BadRequest(format!("invalid memory status '{status}'")));
            }
        }
        self.store
            .update_memory(memory_id, content, pinned, status)
            .await?;
        // Notify open surfaces with the post-edit row (best-effort; a missing
        // row already errored above).
        if let Ok(Some(updated)) = self.store.get_memory(memory_id).await {
            self.emitter.emit_memory_updated(&updated);
        }
        Ok(())
    }

    pub async fn delete_memory(&self, memory_id: &str) -> Result<(), AppError> {
        self.store.delete_memory(memory_id).await?;
        self.emitter.emit_memory_deleted(memory_id);
        Ok(())
    }

    /// List one page of companion skills for the UI. Only skills on the selected page
    /// have their SKILL.md frontmatter read from disk. Each row gets its
    /// SKILL.md `description` read from disk. A missing or malformed durable
    /// body is a side-store integrity failure, not an empty description.
    pub async fn list_companion_skill_page(
        &self,
        companion_id: &str,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<CompanionSkillViewPage, AppError> {
        let page = self
            .store
            .list_skill_page(companion_id, status, limit, offset)
            .await?;
        Ok(CompanionSkillViewPage {
            items: self.skill_views(page.items).await?,
            total: page.total,
        })
    }

    async fn skill_views(
        &self,
        skills: Vec<CompanionSkill>,
    ) -> Result<Vec<CompanionSkillView>, AppError> {
        let mut out = Vec::with_capacity(skills.len());
        for skill in skills {
            let scope = scope_for(skill.scope_companion_id.as_deref());
            let draft = skill.status == "draft";
            let dir = skill_service::skill_dir_for(
                &self.skill_paths,
                &scope,
                &skill.skill_name,
                draft,
            )
            .map_err(|error| {
                AppError::Internal(format!(
                    "resolve durable skill '{}' path: {error}",
                    skill.skill_name
                ))
            })?;
            let (_, description) = skill_service::read_skill_info(&dir)
                .await
                .map_err(|error| {
                    AppError::Internal(format!(
                        "read durable skill '{}' from {}: {error}",
                        skill.skill_name,
                        dir.display()
                    ))
                })?;
            out.push(CompanionSkillView { skill, description });
        }
        Ok(out)
    }

    /// Read one skill's registry row + raw SKILL.md body for the in-app editor.
    pub async fn get_companion_skill_content(
        &self,
        companion_id: &str,
        companion_skill_id: &str,
    ) -> Result<CompanionSkillContent, AppError> {
        let skill = self
            .store
            .get_owned_skill(companion_id, companion_skill_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "companion skill {companion_skill_id} not found"
                ))
            })?;
        let scope = scope_for(skill.scope_companion_id.as_deref());
        let draft = skill.status == "draft";
        let dir = skill_service::skill_dir_for(
            &self.skill_paths,
            &scope,
            &skill.skill_name,
            draft,
        )
            .map_err(|e| AppError::Internal(format!("resolve skill dir: {e}")))?;
        let content = tokio::fs::read_to_string(dir.join(SKILL_MANIFEST_FILE))
            .await
            .map_err(|e| AppError::Internal(format!("read durable skill content: {e}")))?;
        Ok(CompanionSkillContent { skill, content })
    }

    /// Edit a skill's SKILL.md body in place. `content` must be a full valid SKILL.md
    /// (frontmatter + non-empty description) — `write_skill` rejects otherwise → BadRequest.
    pub async fn write_companion_skill_content(
        &self,
        companion_id: &str,
        companion_skill_id: &str,
        content: &str,
    ) -> Result<(), AppError> {
        let skill = self
            .store
            .get_owned_skill(companion_id, companion_skill_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "companion skill {companion_skill_id} not found"
                ))
            })?;
        let scope = scope_for(skill.scope_companion_id.as_deref());
        let draft = skill.status == "draft";
        crate::skill_io::write_skill(
            &self.skill_paths,
            &scope,
            draft,
            &skill.skill_name,
            content,
        )
        .await?;
        Ok(())
    }

    /// Review a draft skill. accept → promote draft SKILL.md to active + status active +
    /// emit skill-learned; reject → status archived + record reject feedback. IDEMPOTENT:
    /// a re-decide on a non-draft row is a no-op returning the row (the `newly`-gate analogue).
    pub async fn decide_companion_skill(
        &self,
        companion_id: &str,
        companion_skill_id: &str,
        accept: bool,
        reason: Option<&str>,
    ) -> Result<CompanionSkill, AppError> {
        let skill = self
            .store
            .get_owned_skill(companion_id, companion_skill_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "companion skill {companion_skill_id} not found"
                ))
            })?;
        if skill.status != "draft" {
            return Ok(skill); // idempotent: already decided
        }
        let scope = SkillScope::Companion(companion_id.to_owned());
        let (draft_dir, active_dir) =
            crate::skill_io::promote_draft(&self.skill_paths, &scope, &skill.skill_name).await?;
        if let Err(error) = self
            .store
            .set_skill_status(
                companion_skill_id,
                if accept { "active" } else { "archived" },
            )
            .await
        {
            let rollback = crate::skill_io::rollback_promotion(&draft_dir, &active_dir);
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => AppError::Internal(format!(
                    "{error}; additionally failed to roll back skill promotion: {rollback_error}"
                )),
            });
        }
        if accept {
            self.emitter.emit_skill_learned(
                companion_id,
                companion_skill_id,
                &skill.skill_name,
            );
        } else {
            let feedback_id =
                nomifun_common::CompanionEvolutionFeedbackId::new().into_string();
            let signature_snapshot = (!skill.signature.is_empty())
                .then_some(skill.signature.as_str());
            self.store
                .record_feedback(
                    &feedback_id,
                    companion_skill_id,
                    &skill.skill_name,
                    skill.skill_pattern_id.as_deref(),
                    signature_snapshot,
                    "reject",
                    reason,
                    nomifun_common::now_ms(),
                )
                .await?;
            // Suppress the originating mined pattern from re-proposal (纠偏回流).
            if let Some(skill_pattern_id) = skill.skill_pattern_id.as_deref() {
                self.store
                    .mark_pattern_status(skill_pattern_id, "rejected")
                    .await?;
            }
        }
        Ok(self
            .store
            .get_owned_skill(companion_id, companion_skill_id)
            .await?
            .unwrap_or(skill))
    }

    /// Learn-by-demonstration (P5 T2-B): reconstruct a tool-name sequence from `conversation_id`'s
    /// collected tool-calls and draft a reviewable skill from it. Requires `collect.tool_calls` to
    /// have been on for that session. Returns the drafted skill name.
    pub async fn draft_skill_from_session(&self, companion_id: &str, conversation_id: &str) -> Result<Option<String>, AppError> {
        let events = {
            let _event_guard = self.event_store_lock.read().await;
            crate::collector::read_recent_events(&self.shared_dir, 1000)?
        };
        let mut steps: Vec<String> = Vec::new();
        let mut call_ids: Vec<String> = Vec::new();
        let mut start_ts = i64::MAX;
        let mut end_ts = 0i64;
        for ev in events.iter() {
            if ev.source != "tool_calls" {
                continue;
            }
            let conv = ev.data.get("conversation_id");
            let matches = conv
                .and_then(|value| value.as_str())
                .is_some_and(|value| value == conversation_id);
            if !matches {
                continue;
            }
            if let Some(name) = ev.data.get("name").and_then(|n| n.as_str()) {
                if steps.last().map(|s| s != name).unwrap_or(true) {
                    steps.push(name.to_owned());
                }
            }
            if let Some(cid) = ev.data.get("call_id").and_then(|c| c.as_str()).filter(|c| !c.is_empty()) {
                call_ids.push(cid.to_owned());
            }
            start_ts = start_ts.min(ev.ts);
            end_ts = end_ts.max(ev.ts);
        }
        if steps.len() < 2 {
            return Err(AppError::BadRequest("这个会话还没有足够的工具操作可以学习成技能".into()));
        }
        // Whole-session anchor: rehydrate this conversation's real transcript for richer drafting.
        let anchor = crate::evolution::TranscriptAnchor {
            conversation_id: conversation_id.to_owned(),
            start_ts: if start_ts == i64::MAX { 0 } else { start_ts },
            end_ts,
            pad_turns: 0,
            call_ids,
        };
        self.evolution.draft_from_episode(steps, anchor, companion_id).await
    }

    // ----- learning -----

    /// "Run now" for ONE companion: it distills from its own cursor into its own
    /// memories. Companion-scoped rather than a single global run because the run
    /// lock is per companion — asking A to learn must not be refused just because
    /// B's scheduled tick is mid-flight.
    pub async fn run_learn_now(
        &self,
        companion_id: &str,
    ) -> Result<CompanionLearnResult, AppError> {
        self.learner.run_for(companion_id).await
    }

    // ----- events -----

    pub async fn event_stats(&self) -> Result<Vec<SourceStats>, AppError> {
        let stats = {
            let _event_guard = self.event_store_lock.read().await;
            collector::event_stats(&self.shared_dir)?
        };
        Ok(stats
            .into_iter()
            .map(|(source, (today, total))| SourceStats { source, today, total })
            .collect())
    }

    pub async fn event_storage(&self) -> Result<collector::EventStorageStatus, AppError> {
        let _event_guard = self.event_store_lock.read().await;
        let config = self.config.read().await;
        collector::event_storage_status(
            &self.shared_dir,
            config.collect.event_retention_days,
            config.collect.event_max_storage_mb,
        )
    }

    pub async fn export_memory_bundle(
        &self,
        dest_path: &std::path::Path,
        include_events: bool,
    ) -> Result<crate::export::ExportSummary, AppError> {
        let _event_guard = self.event_store_lock.read().await;
        crate::export::export_memory_bundle(
            &self.store,
            &self.shared_dir,
            dest_path,
            include_events,
        )
        .await
    }

    /// The durable per-companion homes (`{companions_dir}/{companion_id}/`).
    pub(crate) fn companions_dir(&self) -> &std::path::Path {
        self.registry.companions_dir()
    }

    /// The filesystem homes a companion export reads beside the store.
    fn bundle_homes(&self) -> crate::export::CompanionBundleHomes {
        crate::export::CompanionBundleHomes {
            companions_dir: self.registry.companions_dir().to_path_buf(),
            figures_dir: self.figures_dir.clone(),
            skill_paths: self.skill_paths.clone(),
        }
    }

    /// Package one companion, including — unless `scope` says otherwise — the
    /// memories, skills and custom figure that make it that companion.
    pub async fn export_companion_bundle(
        &self,
        companion_id: &str,
        dest_path: &std::path::Path,
        knowledge_names: &[String],
        scope: crate::export::CompanionBundleScope,
    ) -> Result<crate::export::ExportSummary, AppError> {
        // Existence gate: an unknown companion must 404 before any file is written.
        let profile = self.get_companion(companion_id).await?;
        crate::export::export_companion_bundle(
            &self.store,
            &self.bundle_homes(),
            &profile,
            dest_path,
            knowledge_names,
            scope,
        )
        .await
    }

    pub async fn import_bundle(
        &self,
        src_path: &std::path::Path,
    ) -> Result<crate::export::ImportOutcome, AppError> {
        let outcome = crate::export::import_bundle_with_event_lock(
            &self.store,
            self,
            &self.skill_paths,
            &self.shared_dir,
            src_path,
            self.event_store_lock.clone(),
            self.config.clone(),
        )
        .await?;
        Ok(outcome)
    }
}

/// The factory-facing persona prompt provider: Channel Agent sessions
/// carry `companion_session` but no persisted `system_prompt`, so the nomi factory
/// asks the bound companion for a fresh persona (with current memory snapshot) at
/// every agent build. The persona is built **only** for an explicitly-bound, live
/// companion; `companion_id: None` or a dead id yields no persona — an unbound
/// channel is hosted by no companion (no default-companion fallback; 历史债
/// 「渠道与远程连接默认由默认伙伴接待」已废除，连接由用户为每个伙伴显式配置).
#[async_trait::async_trait]
impl nomifun_ai_agent::CompanionPromptProvider for CompanionService {
    async fn build_system_prompt(&self, companion_id: Option<&str>, channel_platform: Option<&str>) -> Option<String> {
        let companion_id = CompanionId::try_from(companion_id?).ok()?;
        let profile = self.registry.get(companion_id.as_str()).await?;
        let smart = self.config.read().await.smart_collaboration;
        Some(crate::companion::build_companion_system_prompt(&self.store, &profile, channel_platform, smart).await)
    }
}

/// In-session companion summon provider (spec §设计 B): the nomi factory's
/// seam into the companion domain for `extra.summon` sessions — read-only
/// sinks over the store, the per-turn snapshot resolver, and manifest-owned
/// workspace skill materialization/unload.
#[async_trait::async_trait]
impl nomifun_ai_agent::CompanionSummonProvider for CompanionService {
    async fn companion_name(&self, companion_id: &str) -> Option<String> {
        self.registry.get(companion_id).await.map(|profile| profile.name)
    }

    fn summon_memory_sink(
        &self,
        companion_id: &str,
    ) -> Result<Arc<dyn nomifun_ai_agent::CompanionMemorySink>, AppError> {
        Ok(Arc::new(crate::summon_support::SummonMemorySink::new(
            self.store.clone(),
            Self::parse_summon_companion_id(companion_id)?,
        )))
    }


    fn summon_context_sink(
        &self,
        config: &nomifun_api_types::SummonConfig,
    ) -> Result<Arc<dyn nomifun_ai_agent::SummonContextSink>, AppError> {
        Self::parse_summon_companion_id(&config.companion_id)?;
        Ok(Arc::new(crate::summon_support::SummonContextResolver::new(
            self.store.clone(),
            config.clone(),
        )))
    }

    async fn sync_summon_workspace_skills(
        &self,
        conversation_id: &str,
        workspace: &std::path::Path,
        companion_id: &str,
        skill_exclusions: &[String],
    ) -> Result<Vec<String>, AppError> {
        let profile = self
            .registry
            .get(companion_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("companion '{companion_id}' not found")))?;
        let names: Vec<String> =
            crate::companion::effective_skill_names(&self.skill_paths, &profile)
                .await?
                .into_iter()
                .filter(|name| !skill_exclusions.iter().any(|excluded| excluded == name))
                .collect();
        Ok(crate::companion::sync_managed_workspace_skills(
            &self.skill_paths,
            conversation_id,
            workspace,
            &names,
        )
        .await)
    }

    async fn clear_summon_workspace_skills(
        &self,
        conversation_id: &str,
        workspace: &std::path::Path,
    ) -> Result<(), AppError> {
        crate::companion::sync_managed_workspace_skills(
            &self.skill_paths,
            conversation_id,
            workspace,
            &[],
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_realtime::BroadcastEventBus;

    fn companion_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        nomifun_common::CompanionId::try_from(raw.as_str()).unwrap().into_string()
    }

    fn conversation_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        nomifun_common::ConversationId::try_from(raw.as_str()).unwrap().into_string()
    }

    fn provider_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        nomifun_common::ProviderId::try_from(raw.as_str()).unwrap().into_string()
    }

    const MALFORMED_COMPANION_ID: &str = "not-a-companion-id";
    const MALFORMED_PROVIDER_ID: &str = "not-a-provider-id";

    struct NoopCompleter;

    #[async_trait::async_trait]
    impl CompanionCompleter for NoopCompleter {
        async fn complete(&self, _p: &str, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<String, AppError> {
            Ok("{}".into())
        }
    }

    async fn service(data_dir: &std::path::Path) -> Arc<CompanionService> {
        CompanionService::start(
            data_dir,
            Arc::new(BroadcastEventBus::new(16)),
            "owner-a",
            Arc::new(NoopCompleter),
            Arc::new(nomifun_extension::skill_service::resolve_skill_paths(data_dir, data_dir)),
        )
        .await
        .unwrap()
    }

    /// THE upgrade test. An install that had learning on at a non-default
    /// interval, evolution on in 激进 mode, and non-zero global cursors must come
    /// out of boot with EXACTLY that behaviour on every companion — and with the
    /// cursors carried over, not reset to 0 (which would re-distill the whole
    /// retained event history on the next tick).
    ///
    /// Booting twice must change nothing.
    #[tokio::test]
    async fn boot_seeds_every_companion_from_the_retired_install_wide_config_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let shared_dir = dir.path().join(crate::COMPANION_SHARED_REL_DIR);

        // Two companions written the pre-migration way: no learn/evolve keys.
        let companions_dir = dir.path().join(crate::COMPANION_COMPANIONS_REL_DIR);
        let mut ids = Vec::new();
        for (index, name) in ["甲", "乙"].iter().enumerate() {
            let profile = CompanionProfileConfig::new(name, "ink", index as u64 + 1);
            let mut raw = serde_json::to_value(&profile).unwrap();
            let object = raw.as_object_mut().unwrap();
            object.remove("learn");
            object.remove("evolve");
            let home = companions_dir.join(&profile.companion_id);
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(
                CompanionProfileConfig::config_path(&home),
                serde_json::to_vec_pretty(&raw).unwrap(),
            )
            .unwrap();
            ids.push(profile.companion_id);
        }
        std::fs::write(
            shared_dir.join(crate::registry::SEQ_STATE_FILE),
            br#"{"last_companion_seq": 2}"#,
        )
        .unwrap_or_else(|_| {
            std::fs::create_dir_all(&shared_dir).unwrap();
            std::fs::write(
                shared_dir.join(crate::registry::SEQ_STATE_FILE),
                br#"{"last_companion_seq": 2}"#,
            )
            .unwrap()
        });

        // A pre-migration shared config with non-default install-wide settings.
        let mut shared = serde_json::to_value(SharedCompanionConfig::default()).unwrap();
        shared["collect"]["tool_calls"] = serde_json::json!(true);
        shared["learn"] = serde_json::json!({
            "enabled": true, "interval_minutes": 30, "model": null
        });
        shared["evolve"] = serde_json::json!({
            "enabled": true, "interval_minutes": 45, "model": null,
            "min_pattern_count": 4, "min_distinct_sessions": 5,
            "auto_activate": true, "auto_threshold": 0.7,
            "skill_half_life_days": 30.0, "skill_archive_threshold": 0.1
        });
        std::fs::write(
            SharedCompanionConfig::config_path(&shared_dir),
            serde_json::to_vec_pretty(&shared).unwrap(),
        )
        .unwrap();

        // Non-zero global cursors + a mood, as any real install has.
        {
            let store = CompanionStore::open(&shared_dir, Some(&ids[0])).await.unwrap();
            store.set_state(collector::LEARN_CURSOR_KEY, "17000").await.unwrap();
            store.set_state(collector::EVOLVE_CURSOR_KEY, "9000").await.unwrap();
            store.set_state(crate::store::MOOD_KEY, "sleepy").await.unwrap();
        }

        let svc = service(dir.path()).await;
        for id in &ids {
            let profile = svc.get_companion(id).await.unwrap();
            assert!(profile.learn.enabled, "learning must stay ON for {id}");
            assert_eq!(profile.learn.interval_minutes, 30, "the owner's cadence survives");
            assert!(profile.evolve.enabled);
            assert!(profile.evolve.auto_activate, "激进 survives");
            assert_eq!(profile.evolve.min_distinct_sessions, 5);
            assert_eq!(profile.evolve.interval_minutes, 45);
            // The tuning knobs carry over verbatim too.
            assert_eq!(profile.evolve.min_pattern_count, 4);

            assert_eq!(
                svc.store.get_companion_state_i64(id, collector::LEARN_CURSOR_KEY).await.unwrap(),
                17000,
                "a companion seeded at 0 would re-distill the whole event history"
            );
            assert_eq!(
                svc.store.get_companion_state_i64(id, collector::EVOLVE_CURSOR_KEY).await.unwrap(),
                9000
            );
            assert_eq!(svc.companion_status(id).await.unwrap().mood, "sleepy");
        }
        // The shared file no longer carries the moved blocks.
        let rewritten: serde_json::Value = serde_json::from_slice(
            &std::fs::read(SharedCompanionConfig::config_path(&shared_dir)).unwrap(),
        )
        .unwrap();
        assert!(rewritten.get("learn").is_none());
        assert!(rewritten.get("evolve").is_none());
        assert_eq!(rewritten["collect"]["tool_calls"], serde_json::json!(true));

        // Retention still protects both companions from their own lag.
        assert_eq!(
            collector::active_consumer_watermark(&svc.store, &svc.list_companions().await)
                .await
                .unwrap(),
            Some(9000)
        );

        // The owner then changes one companion's mind.
        svc.patch_companion(&ids[0], serde_json::json!({"learn": {"enabled": false}}))
            .await
            .unwrap();
        svc.store
            .set_companion_state(&ids[0], collector::LEARN_CURSOR_KEY, "99000")
            .await
            .unwrap();
        drop(svc);

        // Second boot: nothing is re-seeded, nothing is clobbered.
        let again = service(dir.path()).await;
        assert!(!again.get_companion(&ids[0]).await.unwrap().learn.enabled);
        assert_eq!(
            again.store.get_companion_state_i64(&ids[0], collector::LEARN_CURSOR_KEY).await.unwrap(),
            99000
        );
        assert!(again.get_companion(&ids[1]).await.unwrap().learn.enabled);
        assert_eq!(
            again.store.get_companion_state_i64(&ids[1], collector::LEARN_CURSOR_KEY).await.unwrap(),
            17000
        );
    }

    /// A companion created after the migration starts reading the spool from NOW.
    /// At the absent-row default of 0 its first run would re-summarise the entire
    /// retained history of a machine it has never seen.
    #[tokio::test]
    async fn a_new_companion_starts_its_cursors_at_now_not_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let before = nomifun_common::now_ms();
        let companion = svc.create_companion("新来的", "ink").await.unwrap();
        for key in [collector::LEARN_CURSOR_KEY, collector::EVOLVE_CURSOR_KEY] {
            let cursor = svc
                .store
                .get_companion_state_i64(&companion.companion_id, key)
                .await
                .unwrap();
            assert!(cursor >= before, "{key} must start at creation time, got {cursor}");
        }
    }

    #[tokio::test]
    async fn start_rejects_missing_authoritative_owner() {
        let dir = tempfile::tempdir().unwrap();
        let result = CompanionService::start(
            dir.path(),
            Arc::new(BroadcastEventBus::new(16)),
            "  ",
            Arc::new(NoopCompleter),
            Arc::new(nomifun_extension::skill_service::resolve_skill_paths(
                dir.path(),
                dir.path(),
            )),
        )
        .await;

        assert!(matches!(result, Err(AppError::Internal(message)) if message.contains("owner id")));
    }

    #[tokio::test]
    async fn providers_in_use_detects_companion_chat_model() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("大聪明", "ink").await.unwrap().companion_id;
        let provider_id = provider_fixture(1);
        svc.patch_companion(&cid, serde_json::json!({"model":{"provider_id": provider_id,"model":"m"}})).await.unwrap();

        let hits = svc.providers_in_use(&provider_id).await;
        assert!(hits.iter().any(|u| u.label == "大聪明" && u.target_id.as_deref() == Some(cid.as_str())));
        assert!(svc.providers_in_use(&provider_fixture(99)).await.is_empty());
    }

    #[tokio::test]
    async fn providers_in_use_detects_a_companions_learn_model() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let provider_id = provider_fixture(2);
        let companion = svc.create_companion("学习者", "ink").await.unwrap();
        svc.patch_companion(
            &companion.companion_id,
            serde_json::json!({"learn":{"model":{"provider_id": provider_id,"model":"m"}}}),
        )
        .await
        .unwrap();
        let hits = svc.providers_in_use(&provider_id).await;
        // The reference is now attributable to a companion, so deletion can name it.
        assert!(hits.iter().any(|u| {
            matches!(u.feature, nomifun_common::ProviderUsageFeature::DesktopCompanion)
                && u.label == "学习者·学习模型"
                && u.target_id.as_deref() == Some(companion.companion_id.as_str())
        }), "{hits:?}");
    }

    #[tokio::test]
    async fn providers_in_use_detects_a_companions_evolve_model() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let provider_id = provider_fixture(3);
        let companion = svc.create_companion("进化者", "ink").await.unwrap();
        svc.patch_companion(
            &companion.companion_id,
            serde_json::json!({"evolve":{"model":{"provider_id": provider_id,"model":"m"}}}),
        )
        .await
        .unwrap();
        let hits = svc.providers_in_use(&provider_id).await;
        assert!(hits.iter().any(|u| {
            u.label == "进化者·进化模型"
                && u.target_id.as_deref() == Some(companion.companion_id.as_str())
        }), "{hits:?}");
    }

    #[tokio::test]
    async fn providers_in_use_rejects_malformed_provider_id() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        svc.registry.create("未配置", "ink").await.unwrap();
        for malformed in ["", MALFORMED_PROVIDER_ID] {
            assert!(svc.providers_in_use(malformed).await.is_empty());
        }
    }

    #[tokio::test]
    async fn accepting_create_skill_promotes_draft_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let companion = svc.registry.create("测试", "ink").await.unwrap();
        let cid = companion.companion_id;

        // A reviewed draft: SKILL.md on disk (draft dir) + a draft registry row.
        let input = nomifun_extension::skill_service::SkillDraftInput {
            name: "demo".into(),
            description: "演示技能".into(),
            when_to_use: None,
            allowed_tools: None,
            paths: None,
            body: "步骤".into(),
        };
        let scope = SkillScope::Companion(cid.clone());
        skill_service::create_skill(&svc.skill_paths, &scope, true, &input).await.unwrap();
        let now = nomifun_common::now_ms();
        svc.store
            .insert_skill(&crate::store::CompanionSkill {
            companion_skill_id: nomifun_common::generate_id(),
                skill_name: "demo".into(),
                scope_companion_id: Some(cid.clone()),
                status: "draft".into(),
                source: "mined".into(),
                confidence: 0.9,
                provenance_event_ids: vec![],
                strength: 1.0,
                version: 1,
                skill_pattern_id: None,
                usage_count: 0,
                last_used_at: None,
                created_at: now,
                updated_at: now,
                signature: String::new(),
            })
            .await
            .unwrap();
        let demo_skill_id = svc.store.find_owned_skill_by_name(&cid, "demo").await.unwrap().unwrap().companion_skill_id;

        // Accept → promote draft to active. Reviewed on the 技能 surface: the
        // 建议 card that used to wrap this decision was retired.
        svc.decide_companion_skill(&cid, &demo_skill_id, true, None).await.unwrap();
        let active_md = svc.skill_paths.user_skills_dir.join("companion").join(&cid).join("demo").join("SKILL.md");
        let draft_dir = svc.skill_paths.user_skills_dir.join("_drafts").join(&cid).join("demo");
        assert!(active_md.exists(), "active SKILL.md missing at {}", active_md.display());
        assert!(!draft_dir.exists(), "promoted draft must be removed");
        assert_eq!(svc.store.find_owned_skill_by_name(&cid, "demo").await.unwrap().unwrap().status, "active");

        // Re-accept → idempotent: status unchanged.
        svc.decide_companion_skill(&cid, &demo_skill_id, true, None).await.unwrap();
        assert_eq!(svc.store.find_owned_skill_by_name(&cid, "demo").await.unwrap().unwrap().status, "active");
    }

    #[tokio::test]
    async fn delete_companion_removes_active_and_draft_skill_trees() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;
        let input = nomifun_extension::skill_service::SkillDraftInput {
            name: "cleanup".into(),
            description: "删除测试".into(),
            when_to_use: None,
            allowed_tools: None,
            paths: None,
            body: "步骤".into(),
        };
        let scope = SkillScope::Companion(cid.clone());
        skill_service::create_skill(&svc.skill_paths, &scope, false, &input)
            .await
            .unwrap();
        skill_service::create_skill(&svc.skill_paths, &scope, true, &input)
            .await
            .unwrap();
        let active_root =
            skill_service::companion_skills_root(&svc.skill_paths).join(&cid);
        let draft_root = skill_service::drafts_root(&svc.skill_paths).join(&cid);
        assert!(active_root.exists());
        assert!(draft_root.exists());

        svc.delete_companion(&cid).await.unwrap();

        assert!(!active_root.exists());
        assert!(!draft_root.exists());
    }

    /// Seed a draft skill (SKILL.md on disk + registry row).
    async fn seed_draft_skill(svc: &CompanionService, cid: &str, name: &str) {
        let input = nomifun_extension::skill_service::SkillDraftInput {
            name: name.into(),
            description: "原始描述".into(),
            when_to_use: None,
            allowed_tools: None,
            paths: None,
            body: "步骤".into(),
        };
        skill_service::create_skill(&svc.skill_paths, &SkillScope::Companion(cid.to_owned()), true, &input)
            .await
            .unwrap();
        let now = nomifun_common::now_ms();
        svc.store
            .insert_skill(&CompanionSkill {
            companion_skill_id: nomifun_common::generate_id(),
                skill_name: name.into(),
                scope_companion_id: Some(cid.to_owned()),
                status: "draft".into(),
                source: "mined".into(),
                confidence: 0.5,
                provenance_event_ids: vec![],
                strength: 1.0,
                version: 1,
                skill_pattern_id: None,
                usage_count: 0,
                last_used_at: None,
                created_at: now,
                updated_at: now,
                signature: String::new(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_companion_skills_fails_closed_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;
        seed_draft_skill(&svc, &cid, "alpha").await; // SKILL.md on disk
        // beta: registry row only, NO SKILL.md → fail closed.
        let now = nomifun_common::now_ms();
        svc.store
            .insert_skill(&CompanionSkill {
            companion_skill_id: nomifun_common::generate_id(),
                skill_name: "beta".into(),
                scope_companion_id: Some(cid.clone()),
                status: "draft".into(),
                source: "mined".into(),
                confidence: 0.5,
                provenance_event_ids: vec![],
                strength: 1.0,
                version: 1,
                skill_pattern_id: None,
                usage_count: 0,
                last_used_at: None,
                created_at: now,
                updated_at: now,
                signature: String::new(),
            })
            .await
            .unwrap();
        assert!(svc.list_companion_skill_page(&cid, None, 100, 0).await.is_err());
    }

    #[tokio::test]
    async fn list_companion_skill_page_enriches_only_current_page() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;
        seed_draft_skill(&svc, &cid, "alpha").await;
        seed_draft_skill(&svc, &cid, "beta").await;

        let page = svc
            .list_companion_skill_page(&cid, Some("draft"), 1, 0)
            .await
            .unwrap();

        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].description, "原始描述");
    }

    #[tokio::test]
    async fn get_and_write_skill_content_roundtrip_and_validate() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;
        seed_draft_skill(&svc, &cid, "demo").await;

        assert!(svc.get_companion_skill_content(&cid, &svc.store.find_owned_skill_by_name(&cid, "demo").await.unwrap().unwrap().companion_skill_id).await.unwrap().content.contains("原始描述"));
        // edit with a valid full SKILL.md
        let new_md = "---\nname: demo\ndescription: 改后描述\n---\n\n新步骤\n";
        svc.write_companion_skill_content(&cid, &svc.store.find_owned_skill_by_name(&cid, "demo").await.unwrap().unwrap().companion_skill_id, new_md).await.unwrap();
        assert!(svc.get_companion_skill_content(&cid, &svc.store.find_owned_skill_by_name(&cid, "demo").await.unwrap().unwrap().companion_skill_id).await.unwrap().content.contains("改后描述"));
        // empty description → BadRequest; missing skill → NotFound
        assert!(svc.write_companion_skill_content(&cid, &svc.store.find_owned_skill_by_name(&cid, "demo").await.unwrap().unwrap().companion_skill_id, "---\nname: demo\ndescription:\n---\nx").await.is_err());
        assert!(svc.get_companion_skill_content(&cid, &nomifun_common::generate_id()).await.is_err());
    }

    #[tokio::test]
    async fn decide_companion_skill_accept_reject_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;

        seed_draft_skill(&svc, &cid, "acc").await;
        let r = svc.decide_companion_skill(&cid, &svc.store.find_owned_skill_by_name(&cid, "acc").await.unwrap().unwrap().companion_skill_id, true, None).await.unwrap();
        assert_eq!(r.status, "active");
        assert!(svc.skill_paths.user_skills_dir.join("companion").join(&cid).join("acc").join("SKILL.md").exists());
        // re-accept on a non-draft row is an idempotent no-op
        assert_eq!(svc.decide_companion_skill(&cid, &svc.store.find_owned_skill_by_name(&cid, "acc").await.unwrap().unwrap().companion_skill_id, true, None).await.unwrap().status, "active");

        seed_draft_skill(&svc, &cid, "rej").await;
        let r3 = svc.decide_companion_skill(&cid, &svc.store.find_owned_skill_by_name(&cid, "rej").await.unwrap().unwrap().companion_skill_id, false, Some("太窄")).await.unwrap();
        assert_eq!(r3.status, "archived");
        assert!(
            svc.skill_paths
                .user_skills_dir
                .join("companion")
                .join(&cid)
                .join("rej")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            !svc.skill_paths
                .user_skills_dir
                .join("_drafts")
                .join(&cid)
                .join("rej")
                .exists()
        );
    }

    #[tokio::test]
    async fn rejecting_skill_suppresses_its_originating_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;
        let input = nomifun_extension::skill_service::SkillDraftInput {
            name: "rej-sig".into(),
            description: "d".into(),
            when_to_use: None,
            allowed_tools: None,
            paths: None,
            body: "b".into(),
        };
        skill_service::create_skill(&svc.skill_paths, &SkillScope::Companion(cid.clone()), true, &input).await.unwrap();
        let now = nomifun_common::now_ms();
        svc.store
            .insert_skill(&CompanionSkill {
            companion_skill_id: nomifun_common::generate_id(),
                skill_name: "rej-sig".into(),
                scope_companion_id: Some(cid.clone()),
                status: "draft".into(),
                source: "mined".into(),
                confidence: 0.5,
                provenance_event_ids: vec![],
                strength: 1.0,
                version: 1,
                skill_pattern_id: None,
                usage_count: 0,
                last_used_at: None,
                created_at: now,
                updated_at: now,
                signature: "sig-XYZ".into(),
            })
            .await
            .unwrap();
        assert!(!svc.store.is_signature_rejected("sig-XYZ").await.unwrap());
        svc.decide_companion_skill(&cid, &svc.store.find_owned_skill_by_name(&cid, "rej-sig").await.unwrap().unwrap().companion_skill_id, false, Some("不通用")).await.unwrap();
        assert!(
            svc.store.is_signature_rejected("sig-XYZ").await.unwrap(),
            "rejecting a skill must suppress its originating mined pattern"
        );
    }

    #[tokio::test]
    async fn weekly_digest_counts_recent_skills() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;
        let now = nomifun_common::now_ms();
        svc.store
            .insert_skill(&CompanionSkill {
            companion_skill_id: nomifun_common::generate_id(),
                skill_name: "recent".into(),
                scope_companion_id: Some(cid.clone()),
                status: "active".into(),
                source: "mined".into(),
                confidence: 0.5,
                provenance_event_ids: vec![],
                strength: 1.0,
                version: 1,
                skill_pattern_id: None,
                usage_count: 0,
                last_used_at: None,
                created_at: now,
                updated_at: now,
                signature: String::new(),
            })
            .await
            .unwrap();
        let digest = svc.weekly_digest(&cid, now - 7 * 86_400_000).await.unwrap();
        assert_eq!(digest.skills_learned, 1);
        assert!(digest.new_skill_names.contains(&"recent".to_string()));
    }

    #[tokio::test]
    async fn draft_from_session_requires_activity() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let cid = svc.registry.create("测试", "ink").await.unwrap().companion_id;
        // No collected tool-call activity for this conversation → BadRequest, not a panic.
        assert!(svc.draft_skill_from_session(&cid, &conversation_fixture(90)).await.is_err());
    }

    /// 赠送 (cross-companion gift) is gone, and with it every cross-companion
    /// skill read: a companion's list is exactly its own rows. This is the
    /// regression net for the list query — a resurrected `OR scope_kind = 'user'`
    /// or a copy path would show up here as a second row.
    #[tokio::test]
    async fn a_companions_skill_list_is_exactly_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.registry.create("A", "ink").await.unwrap().companion_id;
        let b = svc.registry.create("B", "ink").await.unwrap().companion_id;
        let input = nomifun_extension::skill_service::SkillDraftInput {
            name: "mine".into(),
            description: "d".into(),
            when_to_use: None,
            allowed_tools: None,
            paths: None,
            body: "b".into(),
        };
        skill_service::create_skill(&svc.skill_paths, &SkillScope::Companion(a.clone()), false, &input).await.unwrap();
        let now = nomifun_common::now_ms();
        let mut skill = CompanionSkill {
            companion_skill_id: nomifun_common::generate_id(),
            skill_name: "mine".into(),
            scope_companion_id: Some(a.clone()),
            status: "active".into(),
            source: "mined".into(),
            confidence: 0.7,
            provenance_event_ids: vec![],
            strength: 1.0,
            version: 1,
            skill_pattern_id: None,
            usage_count: 0,
            last_used_at: None,
            created_at: now,
            updated_at: now,
            signature: String::new(),
        };
        svc.store.insert_skill(&skill).await.unwrap();

        let mine = svc.list_companion_skill_page(&a, None, 50, 0).await.unwrap();
        assert_eq!(mine.total, 1);
        assert_eq!(mine.items[0].skill.skill_name, "mine");
        let theirs = svc.list_companion_skill_page(&b, None, 50, 0).await.unwrap();
        assert_eq!(theirs.total, 0, "another companion's skill must never be listed: {:?}", theirs.items);

        // And there is no way to write an ownerless (shared) row any more.
        skill.companion_skill_id = nomifun_common::generate_id();
        skill.scope_companion_id = None;
        assert!(svc.store.insert_skill(&skill).await.is_err(), "an ownerless skill must be refused");
    }

    /// The kill switch now spans two stores — one shared collect file and N
    /// profiles — so it must turn EVERY companion's loops off, not just the
    /// default one, and still preserve each companion's models and interval.
    #[tokio::test]
    async fn disable_all_turns_everything_off_on_every_companion_but_keeps_models() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let provider_id = provider_fixture(4);
        svc.patch_config(serde_json::json!({
            "collect": { "tool_calls": true, "chat_user_messages": true, "companion_dialogues": true }
        }))
        .await
        .unwrap();
        let mut ids = Vec::new();
        for name in ["甲", "乙"] {
            let companion = svc.create_companion(name, "ink").await.unwrap();
            svc.patch_companion(
                &companion.companion_id,
                serde_json::json!({
                    "learn": { "enabled": true, "interval_minutes": 30, "model": { "provider_id": provider_id, "model": "m" } },
                    "evolve": { "enabled": true, "model": { "provider_id": provider_id, "model": "m" } }
                }),
            )
            .await
            .unwrap();
            ids.push(companion.companion_id);
        }

        svc.disable_all().await.unwrap();
        {
            let cfg = svc.config.read().await;
            assert!(!cfg.collect.tool_calls);
            assert!(!cfg.collect.chat_user_messages);
            assert!(!cfg.collect.companion_dialogues, "kill switch must turn OFF companion_dialogues");
        }
        for id in &ids {
            let profile = svc.get_companion(id).await.unwrap();
            assert!(!profile.learn.enabled, "every companion's learning must stop");
            assert!(!profile.evolve.enabled, "every companion's evolution must stop");
            // models + interval preserved so re-enable needs no reconfig
            assert_eq!(profile.learn.model.as_ref().unwrap().provider_id, provider_id);
            assert_eq!(profile.learn.interval_minutes, 30);
            assert_eq!(profile.evolve.model.as_ref().unwrap().provider_id, provider_id);
        }
    }

    #[tokio::test]
    async fn consent_applies_once_and_never_reenables_after_disable() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let companion = svc.create_companion("甲", "ink").await.unwrap().companion_id;
        // fresh: work sources + learn + evolve all off (Default untouched)
        assert!(!svc.config.read().await.collect.tool_calls);
        assert!(!svc.get_companion(&companion).await.unwrap().learn.enabled);

        // first-launch consent → default-on applied + flag set
        svc.apply_default_on_consent().await.unwrap();
        {
            let cfg = svc.config.read().await;
            assert!(cfg.collect.tool_calls);
            assert!(cfg.collect.chat_user_messages);
            assert!(cfg.collect.requirements);
        }
        {
            let profile = svc.get_companion(&companion).await.unwrap();
            assert!(profile.learn.enabled);
            assert!(profile.evolve.enabled);
        }
        assert_eq!(svc.store.get_state("self_evolution_consent").await.unwrap().as_deref(), Some("1"));

        // user explicitly kills everything
        svc.disable_all().await.unwrap();
        assert!(!svc.config.read().await.collect.tool_calls);

        // re-consent must be an idempotent no-op (flag set) — NEVER silently re-enable
        svc.apply_default_on_consent().await.unwrap();
        assert!(!svc.config.read().await.collect.tool_calls, "must not re-enable after explicit disable");
        assert!(!svc.get_companion(&companion).await.unwrap().learn.enabled);
    }

    #[tokio::test]
    async fn config_writes_cannot_reset_seq_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.create_companion("甲", "ink").await.unwrap();
        let b = svc.create_companion("乙", "boo").await.unwrap();
        assert_eq!(b.seq, 2);
        svc.delete_companion(&b.companion_id).await.unwrap();

        // A merge patch of the shared config (the patch body simply has no
        // watermark field — it lives in the registry's own state file)…
        svc.patch_config(serde_json::json!({
            "default_companion_id": a.companion_id.clone(),
            "collect": {"tool_calls": true},
        }))
        .await
        .unwrap();

        // …cannot hand out the deleted companion's number again.
        let c = svc.create_companion("丙", "mochi").await.unwrap();
        assert_eq!(c.seq, 3);

        // The watermark file is independent of the shared config file, which
        // carries no watermark field at all.
        let shared_dir = dir.path().join(crate::COMPANION_SHARED_REL_DIR);
        assert_eq!(crate::registry::CompanionSeqState::load(&shared_dir).unwrap().last_companion_seq, 3);
        let cfg_raw = std::fs::read_to_string(SharedCompanionConfig::config_path(&shared_dir)).unwrap();
        assert!(!cfg_raw.contains("last_companion_seq"), "config.json must not carry the watermark: {cfg_raw}");
    }

    #[tokio::test]
    async fn failed_storage_policy_save_does_not_prune_raw_events_or_publish_config() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        crate::collector::append_event(
            &svc.shared_dir,
            &crate::collector::CollectedEvent {
                event_id: nomifun_common::generate_id(),
                ts: nomifun_common::now_ms() - 10 * 24 * 60 * 60 * 1000,
                source: "chat_user_messages".into(),
                name: "message.userCreated".into(),
                data: serde_json::json!({"content": "must survive a failed config save"}),
            },
        )
        .unwrap();
        assert_eq!(crate::collector::read_recent_events(&svc.shared_dir, 10).unwrap().len(), 1);

        // Make atomic replacement of config.json fail on every platform: a
        // directory cannot be replaced with the temporary config file.
        let config_path = SharedCompanionConfig::config_path(&svc.shared_dir);
        if config_path.exists() {
            std::fs::remove_file(&config_path).unwrap();
        }
        std::fs::create_dir(&config_path).unwrap();

        let error = svc
            .patch_config(serde_json::json!({
                "collect": {"event_retention_days": 7}
            }))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("save shared companion config"), "{error}");
        assert_eq!(
            svc.get_config().await.collect.event_retention_days,
            crate::config::DEFAULT_EVENT_RETENTION_DAYS
        );
        assert_eq!(
            crate::collector::read_recent_events(&svc.shared_dir, 10)
                .unwrap()
                .len(),
            1,
            "a failed PATCH must not perform the destructive cleanup it requested"
        );
    }

    #[tokio::test]
    async fn startup_prunes_expired_events_before_the_service_returns() {
        let dir = tempfile::tempdir().unwrap();
        let shared_dir = dir.path().join(crate::COMPANION_SHARED_REL_DIR);
        crate::collector::append_event(
            &shared_dir,
            &crate::collector::CollectedEvent {
                event_id: nomifun_common::generate_id(),
                ts: nomifun_common::now_ms() - 31 * 24 * 60 * 60 * 1000,
                source: "tool_calls".into(),
                name: "tool.call".into(),
                data: serde_json::json!({"name": "expired-before-start"}),
            },
        )
        .unwrap();
        assert_eq!(crate::collector::read_recent_events(&shared_dir, 10).unwrap().len(), 1);

        let svc = service(dir.path()).await;

        assert!(
            crate::collector::read_recent_events(&svc.shared_dir, 10)
                .unwrap()
                .is_empty(),
            "startup must enforce retention before callers can use the service"
        );
    }

    #[tokio::test]
    async fn lowering_retention_prunes_expired_events_before_patch_returns() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        crate::collector::append_event(
            &svc.shared_dir,
            &crate::collector::CollectedEvent {
                event_id: nomifun_common::generate_id(),
                ts: nomifun_common::now_ms() - 10 * 24 * 60 * 60 * 1000,
                source: "requirements".into(),
                name: "requirement.created".into(),
                data: serde_json::json!({"title": "expired-after-policy-change"}),
            },
        )
        .unwrap();

        let updated = svc
            .patch_config(serde_json::json!({
                "collect": {"event_retention_days": 7}
            }))
            .await
            .unwrap();

        assert_eq!(updated.collect.event_retention_days, 7);
        assert!(
            crate::collector::read_recent_events(&svc.shared_dir, 10)
                .unwrap()
                .is_empty(),
            "a successful policy PATCH must complete its immediate retention pass before returning"
        );
    }

    #[tokio::test]
    async fn create_companion_first_becomes_default() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        assert!(svc.list_companions().await.is_empty());
        assert_eq!(svc.default_companion_id().await, None);

        let first = svc.create_companion("毛球", "ink").await.unwrap();
        assert_eq!(svc.get_config().await.default_companion_id.as_deref(), Some(first.companion_id.as_str()));
        assert_eq!(svc.default_companion_id().await.as_deref(), Some(first.companion_id.as_str()));
        // Persisted, not just in memory.
        let on_disk =
            SharedCompanionConfig::load(&dir.path().join(crate::COMPANION_SHARED_REL_DIR)).unwrap();
        assert_eq!(on_disk.default_companion_id.as_deref(), Some(first.companion_id.as_str()));

        // A second companion never steals the default.
        let second = svc.create_companion("墨墨", "boo").await.unwrap();
        assert_eq!(svc.get_config().await.default_companion_id.as_deref(), Some(first.companion_id.as_str()));
        assert_ne!(first.companion_id, second.companion_id);
        assert_eq!(svc.list_companions().await.len(), 2);
    }

    #[tokio::test]
    async fn delete_companion_repoints_default_and_clears_rows() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.create_companion("甲", "ink").await.unwrap();
        let b = svc.create_companion("乙", "boo").await.unwrap();
        assert_eq!(svc.get_config().await.default_companion_id.as_deref(), Some(a.companion_id.as_str()));

        // Give A per-companion rows that must vanish with it.
        svc.store.add_companion_xp(&a.companion_id, 42).await.unwrap();
        let conversation = conversation_fixture(1);
        svc.store.insert_companion_thread(&conversation, &a.companion_id, "甲聊").await.unwrap();
        svc.store.add_companion_xp(&b.companion_id, 7).await.unwrap();

        svc.delete_companion(&a.companion_id).await.unwrap();

        assert!(matches!(svc.get_companion(&a.companion_id).await, Err(AppError::NotFound(_))));
        assert_eq!(svc.store.get_companion_state_i64(&a.companion_id, "xp").await.unwrap(), 0);
        assert!(!svc.store.is_companion_thread(&conversation).await.unwrap());
        // Default re-pointed to the survivor; survivor untouched.
        assert_eq!(svc.get_config().await.default_companion_id.as_deref(), Some(b.companion_id.as_str()));
        assert_eq!(svc.store.get_companion_state_i64(&b.companion_id, "xp").await.unwrap(), 7);

        // Deleting the last companion clears the default entirely.
        svc.delete_companion(&b.companion_id).await.unwrap();
        assert_eq!(svc.get_config().await.default_companion_id, None);
        assert_eq!(svc.default_companion_id().await, None);

        assert!(matches!(svc.delete_companion(&a.companion_id).await, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn delete_companion_invokes_cleanup_hooks() {
        struct RecordingHook(std::sync::Mutex<Vec<String>>);

        #[async_trait::async_trait]
        impl CompanionCleanupHook for RecordingHook {
            async fn on_companion_deleted(&self, companion_id: &str) {
                self.0.lock().unwrap().push(companion_id.to_owned());
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let hook = Arc::new(RecordingHook(std::sync::Mutex::new(Vec::new())));
        svc.set_cleanup_hooks(vec![hook.clone() as Arc<dyn CompanionCleanupHook>]);

        let p = svc.create_companion("丙", "ink").await.unwrap();
        svc.delete_companion(&p.companion_id).await.unwrap();
        assert_eq!(hook.0.lock().unwrap().as_slice(), &[p.companion_id.clone()]);

        // A failed delete (unknown id) must not fire the hooks again.
        assert!(matches!(svc.delete_companion(&p.companion_id).await, Err(AppError::NotFound(_))));
        assert_eq!(hook.0.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn status_uses_default_companion_and_survives_no_companions() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;

        // No companions yet: zeroed fallback.
        let empty = svc.status().await.unwrap();
        assert_eq!(empty.companion_id, None);
        assert_eq!(empty.xp, 0);
        assert_eq!(empty.level, 1);
        assert!(!empty.model_configured);

        let a = svc.create_companion("甲", "ink").await.unwrap();
        svc.store.add_companion_xp(&a.companion_id, 150).await.unwrap();
        let status = svc.status().await.unwrap();
        assert_eq!(status.companion_id.as_deref(), Some(a.companion_id.as_str()));
        assert_eq!(status.xp, 150);
        assert_eq!(status.level, 2);

        // Per-companion status for a second companion reads its own xp.
        let b = svc.create_companion("乙", "boo").await.unwrap();
        let sb = svc.companion_status(&b.companion_id).await.unwrap();
        assert_eq!(sb.companion_id.as_deref(), Some(b.companion_id.as_str()));
        assert_eq!(sb.xp, 0);
        assert!(matches!(svc.companion_status(MALFORMED_COMPANION_ID).await, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn start_rejects_dangling_default_companion_reference() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let _a = svc.create_companion("甲", "ink").await.unwrap();
        let _b = svc.create_companion("乙", "boo").await.unwrap();

        let shared_dir = dir.path().join(crate::COMPANION_SHARED_REL_DIR);
        let mut corrupt = svc.get_config().await;
        corrupt.default_companion_id = Some(companion_fixture(999));
        corrupt.save(&shared_dir).unwrap();
        drop(svc);
        let restarted = CompanionService::start(
            dir.path(),
            Arc::new(BroadcastEventBus::new(16)),
            "owner-a",
            Arc::new(NoopCompleter),
            Arc::new(nomifun_extension::skill_service::resolve_skill_paths(
                dir.path(),
                dir.path(),
            )),
        )
        .await;
        assert!(
            matches!(restarted, Err(AppError::Internal(message)) if message.contains("missing default companion"))
        );
    }

    #[tokio::test]
    async fn patch_companion_emits_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.create_companion("甲", "ink").await.unwrap();
        let patched = svc
            .patch_companion(&a.companion_id, serde_json::json!({"name": "新名", "appearance": {"companion_enabled": true}}))
            .await
            .unwrap();
        assert_eq!(patched.name, "新名");
        assert!(patched.appearance.companion_enabled);
        assert_eq!(svc.get_companion(&a.companion_id).await.unwrap().name, "新名");
    }

    #[tokio::test]
    async fn patch_companion_model_change_is_best_effort_when_companion_unwired() {
        // Setting a model triggers companion-session ensure + model
        // propagation, both best-effort. With no companion wired (this test
        // harness), the patch must still succeed and persist the model — the
        // model唯一事实源 (profile.model) is always written regardless.
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.create_companion("甲", "ink").await.unwrap();
        assert!(a.model.is_none());
        let provider_id = provider_fixture(5);

        let patched = svc
            .patch_companion(&a.companion_id, serde_json::json!({"model": {"provider_id": provider_id, "model": "claude-fable-5"}}))
            .await
            .unwrap();
        assert_eq!(patched.model.as_ref().unwrap().provider_id, provider_id);
        assert_eq!(svc.get_companion(&a.companion_id).await.unwrap().model.unwrap().model, "claude-fable-5");

        // A non-model patch on an already-configured companion also succeeds (no
        // spurious propagation path, model unchanged).
        let renamed = svc.patch_companion(&a.companion_id, serde_json::json!({"name": "甲改"})).await.unwrap();
        assert_eq!(renamed.name, "甲改");
        assert_eq!(renamed.model.as_ref().unwrap().provider_id, provider_id);
    }

    #[tokio::test]
    async fn add_memory_dedups_into_existing_active_memory() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;

        // No companion at all: there is no legal owner, so the add refuses
        // instead of writing an ownerless row.
        let ownerless = svc.add_memory("preference", "主人喜欢深色主题", &[], None).await;
        assert!(
            matches!(&ownerless, Err(AppError::BadRequest(message)) if message.contains("还没有伙伴")),
            "{ownerless:?}"
        );

        let a = svc.create_companion("甲", "ink").await.unwrap().companion_id;
        let b = svc.create_companion("乙", "ink").await.unwrap().companion_id;

        let first = svc
            .add_memory("preference", "主人喜欢深色主题", &["ui".into()], Some(&a))
            .await
            .unwrap();
        assert_eq!(first.scope_companion_id.as_deref(), Some(a.as_str()));
        assert_eq!(svc.store.count_memories("active", Some(&a)).await.unwrap(), 1);

        // Same content (modulo case/whitespace) merges: reinforced, no new row.
        let again = svc.add_memory("preference", " 主人喜欢深色主题 ", &[], Some(&a)).await.unwrap();
        assert_eq!(again.memory_id, first.memory_id);
        assert_eq!(svc.store.count_memories("active", Some(&a)).await.unwrap(), 1);
        assert!(again.strength > first.strength, "dedup hit must reinforce the existing memory");

        // Genuinely different content still inserts.
        let other = svc.add_memory("preference", "主人喜欢浅色代码字体", &[], Some(&a)).await.unwrap();
        assert_ne!(other.memory_id, first.memory_id);
        assert_eq!(svc.store.count_memories("active", Some(&a)).await.unwrap(), 2);

        // The dedup guard is OWNER-scoped: 乙 saying the same thing gets its own
        // row instead of being silently folded into 甲's memory.
        let bs = svc.add_memory("preference", "主人喜欢深色主题", &[], Some(&b)).await.unwrap();
        assert_ne!(bs.memory_id, first.memory_id);
        assert_eq!(bs.scope_companion_id.as_deref(), Some(b.as_str()));
        assert_eq!(svc.store.count_memories("active", Some(&b)).await.unwrap(), 1);

        // Omitting the companion resolves the owner (oldest = 甲) rather than
        // writing a shared row.
        let resolved = svc.add_memory("task", "帮主人订咖啡豆", &[], None).await.unwrap();
        assert_eq!(resolved.scope_companion_id.as_deref(), Some(a.as_str()));

        // Validation untouched, and an unknown owner is rejected before any write
        // (an orphaned reference would hard-fail the next boot).
        assert!(svc.add_memory("bogus", "x", &[], Some(&a)).await.is_err());
        assert!(svc.add_memory("task", "   ", &[], Some(&a)).await.is_err());
        assert!(svc.add_memory("task", "无主人", &[], Some(MALFORMED_COMPANION_ID)).await.is_err());
    }

    #[tokio::test]
    async fn companion_prompt_provider_builds_only_for_bound_companion() {
        use nomifun_ai_agent::CompanionPromptProvider;
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;

        // No companions: no persona.
        assert!(svc.build_system_prompt(None, None).await.is_none());

        let a = svc.create_companion("毛球", "ink").await.unwrap();
        let b = svc.create_companion("墨墨", "boo").await.unwrap();
        // No companion_id → NO persona (历史债「渠道默认由默认伙伴接待」已废除；不再回落默认伙伴).
        assert!(svc.build_system_prompt(None, None).await.is_none());
        // Explicit, live companion → its persona.
        let b_prompt = svc.build_system_prompt(Some(&b.companion_id), None).await.unwrap();
        assert!(b_prompt.contains("你是 墨墨"));
        // Dead explicit id → NO persona (no default fallback).
        assert!(svc.build_system_prompt(Some(MALFORMED_COMPANION_ID), None).await.is_none());
        let _ = a;
    }

    // ----- custom-figure library: in-use figures must not be deletable -----

    /// A real 7×5 lossless WebP (VP8L) — the same bytes the figures.rs/figure.rs
    /// tests use, so `create_figure`'s validator accepts it.
    fn webp_bytes() -> Vec<u8> {
        vec![
            0x52, 0x49, 0x46, 0x46, 0x1E, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x4C, 0x11, 0x00, 0x00, 0x00, 0x2F, 0x06, 0x00, 0x01, 0x00, 0x07, 0x50, 0x8A,
            0x2A, 0xD4, 0xA3, 0xFF, 0x81, 0x88, 0xE8, 0x7F, 0x00, 0x00,
        ]
    }

    /// A unique scratch dir under the upload sandbox root (`{temp}/nomifun`) —
    /// figure sources must canonicalize inside it (see
    /// [`crate::figure::validate_figure_source`]).
    fn upload_scratch() -> tempfile::TempDir {
        let root = std::env::temp_dir().join("nomifun");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new().prefix("companionsvc-fig-").tempdir_in(root).unwrap()
    }

    fn webp_source(upload: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
        let p = upload.path().join(name);
        std::fs::write(&p, webp_bytes()).unwrap();
        p
    }

    /// Patch links a companion to a library figure via `appearance.custom_figure.figure_id`.
    fn link_patch(fig: &crate::figures::FigureMeta) -> serde_json::Value {
        serde_json::json!({
            "character": "custom",
            "appearance": {"custom_figure": {
                "aspect": fig.aspect,
                "head_box": {"x": fig.head_box.x, "y": fig.head_box.y, "w": fig.head_box.w, "h": fig.head_box.h},
                "size_tier": fig.size_tier,
                "figure_id": fig.figure_id,
            }},
        })
    }

    #[tokio::test]
    async fn update_figure_syncs_active_companion_custom_figure_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let upload = upload_scratch();
        let svc = service(dir.path()).await;

        let fig = svc
            .create_figure(
                webp_source(&upload, "editable.webp").to_str().unwrap(),
                "旧形象",
                0.7,
                crate::profile::HeadBox { x: 0.3, y: 0.0, w: 0.4, h: 0.4 },
                "m",
            )
            .await
            .unwrap();
        let companion = svc.create_companion("可可", "custom").await.unwrap();
        svc.patch_companion(&companion.companion_id, link_patch(&fig)).await.unwrap();

        let next_head = crate::profile::HeadBox { x: 0.12, y: 0.18, w: 0.36, h: 0.42 };
        let updated = svc
            .update_figure(
                &fig.figure_id,
                crate::figures::FigureUpdate { name: Some("新形象".to_owned()), head_box: Some(next_head.clone()), size_tier: Some("l".to_owned()) },
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "新形象");
        assert_eq!(updated.head_box, next_head);
        assert_eq!(updated.size_tier, "l");

        let synced = svc.get_companion(&companion.companion_id).await.unwrap();
        let custom = synced.appearance.custom_figure.unwrap();
        assert_eq!(custom.figure_id.as_deref(), Some(fig.figure_id.as_str()));
        assert_eq!(custom.aspect, fig.aspect);
        assert_eq!(custom.head_box, next_head);
        assert_eq!(custom.size_tier, "l");
    }

    #[tokio::test]
    async fn update_figure_preserves_per_companion_size_px_override() {
        // The 总览 size slider writes a per-companion `size_px` override onto the
        // companion's custom_figure. Editing the LIBRARY figure (head_box/tier)
        // fans out via sync_figure_to_active_companions, whose RFC 7396 patch never
        // mentions size_px — so the per-companion override must survive the sync.
        let dir = tempfile::tempdir().unwrap();
        let upload = upload_scratch();
        let svc = service(dir.path()).await;

        let fig = svc
            .create_figure(
                webp_source(&upload, "sized.webp").to_str().unwrap(),
                "旧形象",
                0.7,
                crate::profile::HeadBox { x: 0.3, y: 0.0, w: 0.4, h: 0.4 },
                "m",
            )
            .await
            .unwrap();
        let companion = svc.create_companion("可可", "custom").await.unwrap();
        svc.patch_companion(&companion.companion_id, link_patch(&fig)).await.unwrap();
        // Slider sets a per-companion override (merge-patch, like the UI does).
        svc.patch_companion(
            &companion.companion_id,
            serde_json::json!({"appearance": {"custom_figure": {"size_px": 333.0}}}),
        )
        .await
        .unwrap();

        // Editing the library figure's tier fans out to the companion.
        svc.update_figure(
            &fig.figure_id,
            crate::figures::FigureUpdate { name: None, head_box: None, size_tier: Some("l".to_owned()) },
        )
        .await
        .unwrap();

        let synced = svc.get_companion(&companion.companion_id).await.unwrap();
        let custom = synced.appearance.custom_figure.unwrap();
        assert_eq!(custom.size_tier, "l"); // library tier change applied
        assert_eq!(custom.size_px, Some(333.0)); // per-companion override preserved
    }

    #[tokio::test]
    async fn delete_figure_refuses_while_a_companion_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        let upload = upload_scratch();
        let svc = service(dir.path()).await;

        let fig = svc
            .create_figure(
                webp_source(&upload, "a.webp").to_str().unwrap(),
                "阿狸",
                0.7,
                crate::profile::HeadBox { x: 0.3, y: 0.0, w: 0.4, h: 0.4 },
                "m",
            )
            .await
            .unwrap();
        let companion = svc.create_companion("毛球", "custom").await.unwrap();
        svc.patch_companion(&companion.companion_id, link_patch(&fig)).await.unwrap();

        // In use → delete is refused with Conflict, and the figure survives.
        assert!(matches!(svc.delete_figure(&fig.figure_id).await, Err(AppError::Conflict(_))));
        assert!(svc.list_figures().await.unwrap().iter().any(|f| f.figure_id == fig.figure_id));

        // Re-point the companion to a built-in character → figure is now unused → deletable.
        svc.patch_companion(&companion.companion_id, serde_json::json!({"character": "ink", "appearance": {"custom_figure": null}}))
            .await
            .unwrap();
        svc.delete_figure(&fig.figure_id).await.unwrap();
        assert!(svc.list_figures().await.unwrap().iter().all(|f| f.figure_id != fig.figure_id));
    }

    #[tokio::test]
    async fn delete_figure_allows_unused_and_after_only_user_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let upload = upload_scratch();
        let svc = service(dir.path()).await;

        // An unused figure deletes straight away.
        let unused = svc
            .create_figure(
                webp_source(&upload, "u.webp").to_str().unwrap(),
                "未用",
                1.0,
                crate::profile::HeadBox { x: 0.3, y: 0.0, w: 0.4, h: 0.4 },
                "m",
            )
            .await
            .unwrap();
        svc.delete_figure(&unused.figure_id).await.unwrap();
        assert!(svc.list_figures().await.unwrap().is_empty());

        // A figure freed by deleting its only user becomes deletable.
        let fig = svc
            .create_figure(
                webp_source(&upload, "b.webp").to_str().unwrap(),
                "在用",
                0.7,
                crate::profile::HeadBox { x: 0.3, y: 0.0, w: 0.4, h: 0.4 },
                "m",
            )
            .await
            .unwrap();
        let companion = svc.create_companion("墨墨", "custom").await.unwrap();
        svc.patch_companion(&companion.companion_id, link_patch(&fig)).await.unwrap();
        assert!(matches!(svc.delete_figure(&fig.figure_id).await, Err(AppError::Conflict(_))));

        svc.delete_companion(&companion.companion_id).await.unwrap();
        svc.delete_figure(&fig.figure_id).await.unwrap();
        assert!(svc.list_figures().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_figure_requires_explicitly_clearing_hidden_reference() {
        let dir = tempfile::tempdir().unwrap();
        let upload = upload_scratch();
        let svc = service(dir.path()).await;

        let fig = svc
            .create_figure(
                webp_source(&upload, "yx.webp").to_str().unwrap(),
                "云霄",
                0.56,
                crate::profile::HeadBox { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
                "l",
            )
            .await
            .unwrap();
        let companion = svc.create_companion("墨墨", "custom").await.unwrap();
        svc.patch_companion(&companion.companion_id, link_patch(&fig)).await.unwrap();
        // While the companion's character is `custom`, the figure is genuinely in use.
        assert!(matches!(svc.delete_figure(&fig.figure_id).await, Err(AppError::Conflict(_))));

        // Switching render mode alone does not erase the durable logical
        // reference, so deletion remains blocked.
        svc.patch_companion(&companion.companion_id, serde_json::json!({"character": "ink"}))
            .await
            .unwrap();
        assert!(matches!(svc.delete_figure(&fig.figure_id).await, Err(AppError::Conflict(_))));

        // Explicitly clear the binding, then deletion is safe.
        svc.patch_companion(
            &companion.companion_id,
            serde_json::json!({"appearance": {"custom_figure": null}}),
        )
        .await
        .unwrap();
        svc.delete_figure(&fig.figure_id).await.unwrap();
        assert!(svc.list_figures().await.unwrap().iter().all(|f| f.figure_id != fig.figure_id));
    }
}
