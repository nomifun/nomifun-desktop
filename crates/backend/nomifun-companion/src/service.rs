//! `CompanionService` bundles the shared config, the companion registry, the store,
//! collector stats, learner and the companion-thread manager into the single
//! object the routes layer talks to.

use std::path::PathBuf;
use std::sync::Arc;

use nomifun_common::{
    AppError, CompanionId, ProviderId, ProviderUsage, ProviderUsageFeature, SharedProviderLifecycleBarrier,
};
use nomifun_db::{IProviderModelRepository, IProviderRepository};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::collector::{self, Collector, SharedConfig, SharedEventStoreLock};
use crate::archiver::Archiver;
use crate::companion::CompanionThreads;
use crate::events::CompanionEventEmitter;
use crate::evolution::{EvolutionEngine, EvolveRun, NoopTranscriptSource};
use crate::gamify::level_for_xp;
use crate::learner::{CompanionCompleter, CompanionLearnResult, Learner};
use crate::memory_search::{MemorySearchQuery, MemoryStatusFilter};
use crate::profile::{CompanionProfileConfig, SharedCompanionConfig};
use crate::registry::{CompanionRegistry, json_merge_patch};
use crate::session_port::{CompanionHostPorts, CompanionRole};
use crate::store::{
    CompanionThread, MemoryActor, MemoryBatchAction, MemoryFilter, MemoryListSort, MemoryPage,
    CompanionMemory, CompanionSkill, CompanionStore,
    memory_contents_similar,
};
use nomifun_skill_library::skill_service::{self, SkillPaths, SkillScope};
use nomifun_skill_library::constants::SKILL_MANIFEST_FILE;

fn managed_workspace_deletion_targets(root: &std::path::Path, managed: &[String])
    -> Result<std::collections::BTreeSet<PathBuf>, AppError> {
    use nomifun_common::paths::{WorkspaceDirectoryCheck, canonical_existing_workspace_directory, simplified};
    let root = canonical_existing_workspace_directory(root, WorkspaceDirectoryCheck::Runtime)?;
    let mut targets = std::collections::BTreeSet::new();
    for path in managed {
        // Host guards return real paths. Normalize both spellings before the
        // lexical check, then resolve each existing directory again before
        // the final domain deletion fence permits removing it.
        let target = simplified(std::path::Path::new(path));
        if !target.is_absolute() || target == root || !target.starts_with(&root) {
            return Err(AppError::Conflict("host returned a workspace outside the managed Companion root".into()));
        }
        match std::fs::symlink_metadata(&target) {
            Ok(_) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(AppError::Internal(format!("inspect settled Companion workspace: {error}"))),
        }
        let target = canonical_existing_workspace_directory(&target, WorkspaceDirectoryCheck::Runtime)?;
        if target == root || !target.starts_with(&root) {
            return Err(AppError::Conflict("host returned a workspace outside the managed Companion root".into()));
        }
        targets.insert(target);
    }
    Ok(targets)
}

/// Map the stored owner to the extension skill scope. `None` is only the
/// vestigial legacy row the boot re-homing has not claimed, whose body still
/// lives in the shared tree.
fn scope_for(companion_id: Option<&str>) -> SkillScope {
    companion_id
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
    /// Effective displayed emotion, including an optional profile override.
    pub mood: String,
    /// Latest learning output, retained while a manual emotion is selected.
    pub automatic_mood: String,
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

#[derive(Debug, Clone, Serialize)]
pub struct CompanionFactRecallPage {
    pub hits: Vec<crate::AgentMemorySearchHit>,
    pub has_more: bool,
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
                && head.companion_id == memory.companion_id
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
    async fn on_companion_model_changed(
        &self,
        _companion_id: &str,
        _model: Option<&nomifun_common::ProviderWithModel>,
    ) {
    }
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
    /// Host shutdown only, after ingress and background consumers have stopped.
    /// This closes the separate memory database without deleting any data.
    pub async fn close_storage(&self) {
        self.store.close().await;
    }

    /// Installation owner whose persistent Companion dataset this service
    /// instance controls. Agent adapters use it to reject a forged resource
    /// binding before resolving any Companion identity.
    pub fn authoritative_user_id(&self) -> &str {
        self.authoritative_user_id.as_ref()
    }

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
        provider_model_repo: Option<Arc<dyn IProviderModelRepository>>,
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
            provider_model_repo,
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
        for profile in registry.list().await {
            store.seed_companion_state(&profile.companion_id, "memory_owner_epoch", &profile.created_at.to_string()).await?;
        }
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

    /// Late-wire the companion thread manager through typed Session ports.
    pub fn attach_companion(&self, ports: CompanionHostPorts) {
        // Also wire the real transcript source so skill drafting rehydrates the actual
        // (redacted) session transcript from the conversation store — the durable single
        // source of truth — instead of degrading to tool-name steps.
        self.evolution.set_transcript(ports.transcript);
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
                port: ports.archive,
                run_lock: Arc::new(Mutex::new(())),
            });
            if self.archiver.set(archiver.clone()).is_ok() {
                archiver.spawn();
            }
        }
        let _ = self.companion.set(CompanionThreads {
            ensure_lock: Mutex::new(()),
            authoritative_user_id: self.authoritative_user_id.clone(),
            store: self.store.clone(),
            config: self.config.clone(),
            registry: self.registry.clone(),
            sessions: ports.sessions,
            skill_paths: self.skill_paths.clone(),
        });
    }

    /// Late-wire the delete-cascade hooks (depends on services built after
    /// the companion service in app startup, e.g. `KnowledgeService`). First call
    /// wins; later calls are ignored (`OnceLock` semantics).
    pub fn set_cleanup_hooks(&self, hooks: Vec<Arc<dyn CompanionCleanupHook>>) {
        let _ = self.cleanup_hooks.set(hooks);
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

    /// Every desktop-companion reference to `provider_id`, slot by slot: each
    /// companion's chat / 备用 / 视觉 models, its own 学习 / 进化 models (one
    /// install-wide pair until 2026-08) and its 语音识别 / 语音合成 selections —
    /// i.e. exactly `CompanionProfileConfig::provider_model_slots`.
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
            for (slot, model) in p.provider_model_slots() {
                if model.provider_id != provider_id.as_str() {
                    continue;
                }
                let suffix = slot_display_label(slot);
                out.push(ProviderUsage {
                    feature: ProviderUsageFeature::DesktopCompanion,
                    label: if suffix.is_empty() {
                        p.name.clone()
                    } else {
                        format!("{}·{suffix}", p.name)
                    },
                    target_id: Some(p.companion_id.clone()),
                });
            }
        }
        out
    }

    /// Detach mutable model selections while the deletion coordinator holds
    /// the shared Provider lifecycle write guard. `None` removes the Provider's
    /// references; `Some` removes only that exact model. Existing conversations
    /// and their frozen model bindings are deliberately left untouched.
    pub async fn detach_model_references_under_provider_guard(
        &self,
        provider_id: &str,
        model: Option<&str>,
    ) -> Result<usize, AppError> {
        let provider_id = ProviderId::try_from(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider id: {error}")))?;
        if model.is_some_and(|model| model.trim().is_empty()) {
            return Err(AppError::BadRequest("model must not be empty".into()));
        }
        let mut changed = 0;
        for profile in self.registry.list().await {
            if let Some(detached) = self
                .registry
                .detach_model_references_under_guard(
                    &profile.companion_id,
                    provider_id.as_str(),
                    model,
                )
                .await?
            {
                // Emit each successful save immediately: if another profile's
                // save fails, already persisted changes still reach subscribers.
                self.emitter
                    .emit_companion_updated(&detached.companion_id, &detached);
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Create a companion. The first companion ever created automatically becomes the
    /// default companion (shared config saved + broadcast).
    pub async fn create_companion(&self, name: &str, character: &str) -> Result<CompanionProfileConfig, AppError> {
        let profile = self.registry.create(name, character).await?;
        self.store.seed_companion_state(&profile.companion_id, "memory_owner_epoch", &profile.created_at.to_string()).await?;
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
        // Ensure the default desktop receive Session, but only when its
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

    pub fn companion_workspaces_root(&self) -> PathBuf {
        self.registry.workspaces_dir()
    }

    pub async fn companion_workspace_path(&self, id: &str) -> Result<PathBuf, AppError> {
        let profile = self.get_companion(id).await?;
        match self.companion_workspace_component(id).await? {
            Some(component) => Ok(self.registry.workspaces_dir().join(component)),
            None => Ok(crate::companion::compute_desired_workspace_dir(&self.registry.workspaces_dir(), &profile)),
        }
    }

    pub async fn companion_workspace_component(&self, id: &str) -> Result<Option<String>, AppError> {
        self.get_companion(id).await?;
        let component = self.store.get_companion_state(id, crate::companion::WORKSPACE_COMPONENT_KEY).await?;
        if let Some(value) = component.as_deref() {
            crate::companion::validate_workspace_component(value)
                .map_err(|error| AppError::Conflict(format!("invalid registered Companion workspace: {error}")))?;
        }
        Ok(component)
    }

    /// Server-only adoption after the host verifies canonical creation
    /// provenance. No HTTP or model Action can register a directory. CAS-if-absent
    /// returns the winner and never rewrites an existing location on rename.
    pub async fn register_companion_workspace_component(&self, id: &str, component: &str) -> Result<String, AppError> {
        self.get_companion(id).await?;
        crate::companion::validate_workspace_component(component)?;
        self.store.seed_companion_state(id, crate::companion::WORKSPACE_COMPONENT_KEY, component).await?;
        self.companion_workspace_component(id).await?
            .ok_or_else(|| AppError::Conflict("Companion workspace registration disappeared".into()))
    }

    /// RFC 7396 partial update of one companion's profile. Profile model changes
    /// apply to future AgentSessions and remote-channel reconstruction; an
    /// existing local AgentSession retains its frozen model.
    pub async fn patch_companion(&self, id: &str, patch: serde_json::Value) -> Result<CompanionProfileConfig, AppError> {
        let manager = self.companion().ok();
        let _lifecycle = match manager { Some(manager) => Some(manager.ensure_lock.lock().await), None => None };
        // Snapshot the pre-patch model so we can tell whether this patch
        // actually changed it (RFC 7396 patches need not mention `model`).
        let prev = self.registry.get(id).await;
        let prev_model = prev.as_ref().and_then(|p| p.model.clone());
        let mood_guard = if patch.get("mood_override").is_some() {
            Some(self.registry.lock_mood_updates().await)
        } else {
            None
        };
        // Read before committing so a damaged/closed runtime store cannot make
        // PATCH fail after the new preference has already been persisted.
        let automatic_mood = if patch.get("mood_override").is_some() {
            self.get_companion(id).await?;
            Some(self.store.get_companion_state(id, crate::store::MOOD_KEY).await?
                .unwrap_or_else(|| "content".into()))
        } else {
            None
        };
        let profile = self.registry.patch(id, patch).await?;
        self.emitter.emit_companion_updated(&profile.companion_id, &profile);
        if let Some(automatic_mood) = automatic_mood {
            self.emitter.emit_mood_changed(id, profile.effective_mood(&automatic_mood));
        }
        drop(mood_guard);

        let model_changed = prev_model.as_ref() != profile.model.as_ref();
        if model_changed {
            // 通知宿主：模型已切换（唯一事实源）。当前用于清理该伙伴绑定的
            // IM 渠道会话，使其下轮重建拾取新模型（或正确地因未配置而拒绝）。
            // best-effort，不阻断 patch。
            if let Some(hooks) = self.cleanup_hooks.get() {
                for hook in hooks {
                    hook.on_companion_model_changed(
                        &profile.companion_id,
                        profile.model.as_ref(),
                    )
                    .await;
                }
            }
        }
        // Existing AgentSessions retain their frozen workspace and Skill
        // snapshot. Name/Skill profile changes apply to a future Session.
        Ok(profile)
    }

    /// Delete a companion: cascade-delete its companion conversations, clear its
    /// per-companion store rows, remove the profile, and re-point the default companion
    /// if it was this one.
    pub async fn delete_companion(&self, id: &str) -> Result<(), AppError> {
        let manager = self.companion().ok();
        // Same domain lifecycle fence as ensure/new/delete. The host owns the
        // required canonical affiliation and producer/effect settlement fence.
        let _lifecycle = match manager { Some(manager) => Some(manager.ensure_lock.lock().await), None => None };
        self.get_companion(id).await?;
        let workspace = self.companion_workspace_path(id).await?;
        // Resolve the target once, before acquiring the host directory fence.
        // An originally absent directory must not be removed if it appears later.
        let existing_workspace = match std::fs::symlink_metadata(&workspace) {
            Ok(_) => {
                let root = nomifun_common::paths::canonical_existing_workspace_directory(
                    &self.registry.workspaces_dir(), nomifun_common::paths::WorkspaceDirectoryCheck::Runtime)?;
                let target = nomifun_common::paths::canonical_existing_workspace_directory(
                    &workspace, nomifun_common::paths::WorkspaceDirectoryCheck::Runtime)?;
                if target == root || !target.starts_with(&root) {
                    return Err(AppError::Conflict("Companion workspace escapes its owner directory".into()));
                }
                Some(target)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(AppError::Internal(format!("inspect Companion workspace: {error}"))),
        };
        let workspace_argument = existing_workspace.as_ref().map(|path| path.to_string_lossy().into_owned());
        let deletion_guard = match manager {
            Some(manager) => Some(manager.sessions.delete_companion_sessions(self.authoritative_user_id.as_ref(), id,
                workspace_argument.as_deref()).await?),
            None if existing_workspace.is_some() || !self.store.list_companion_threads(Some(id)).await?.is_empty() => {
                return Err(AppError::Internal("canonical Companion Session cleanup is not configured".into()));
            }
            None => None,
        };
        // Only canonical provenance verified by the host grants directory
        // ownership. Its returned Guard also covers older frozen roots created
        // before stable workspace naming and keeps every directory fence alive.
        let managed = deletion_guard.as_ref().map(|guard| guard.managed_workspaces()).unwrap_or_default();
        if !managed.is_empty() {
            for target in managed_workspace_deletion_targets(&self.registry.workspaces_dir(), managed)? {
                match std::fs::remove_dir_all(&target) {
                    Ok(()) => {},
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(error) => return Err(AppError::Internal(format!("remove settled Companion workspace: {error}"))),
                }
            }
        }
        // T3.3 knowledge binding cleanup lives in the cleanup hooks below
        // (the app assembly registers a KnowledgeService-backed hook that
        // drops the ('companion', id) binding row).
        self.remove_companion_skill_trees(id)?;
        self.store.delete_companion_rows(id).await?;
        self.registry.remove(id).await?;
        drop(deletion_guard);
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
        let mood_guard = self.registry.lock_mood_updates().await;
        let profile = self.get_companion(id).await?;
        let automatic_mood = self.store.get_companion_state(id, crate::store::MOOD_KEY).await?
            .unwrap_or_else(|| "content".into());
        let mood = profile.effective_mood(&automatic_mood).to_owned();
        drop(mood_guard);
        let cfg = self.config.read().await.clone();
        let xp = self.store.get_companion_state_i64(id, "xp").await?;
        Ok(CompanionStatus {
            companion_id: Some(profile.companion_id),
            xp,
            level: level_for_xp(xp),
            mood,
            automatic_mood,
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

    // ----- companion Sessions and independent receive routes -----

    /// Idempotent ensure of the desktop receive Session.
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

    pub async fn ensure_session_for_role(
        &self, companion_id: &str, role: CompanionRole, robot_id: Option<&str>,
    ) -> Result<CompanionThread, AppError> {
        self.companion()?.ensure_for_role(companion_id, role, robot_id, None).await
    }

    pub async fn desktop_role_launch_configuration(
        &self, companion_id: &str,
    ) -> Result<crate::CompanionRoleLaunchConfiguration, AppError> {
        self.companion()?.desktop_role_launch_configuration(companion_id).await
    }

    pub async fn create_session_for_role(
        &self, companion_id: &str, role: CompanionRole, robot_id: Option<&str>, idempotency_key: &str,
    ) -> Result<CompanionThread, AppError> {
        self.companion()?.create_for_role(companion_id, role, robot_id, idempotency_key, None).await
    }

    pub async fn new_companion_thread(
        &self, companion_id: &str, creation_key: &str, title: Option<String>,
    ) -> Result<CompanionThread, AppError> {
        self.companion()?.create_for_role(companion_id, CompanionRole::Companion, None, creation_key, title).await
    }

    pub async fn list_companion_threads(&self, companion_id: &str) -> Result<Vec<CompanionThread>, AppError> {
        self.get_companion(companion_id).await?;
        self.companion()?.list(companion_id).await
    }

    pub async fn set_companion_active_thread(&self, companion_id: &str, session_id: &str) -> Result<(), AppError> {
        self.companion()?.set_active(companion_id, session_id).await?;
        let profile = self.get_companion(companion_id).await?;
        self.emitter.emit_companion_updated(companion_id, &profile);
        Ok(())
    }

    pub async fn delete_companion_thread(&self, companion_id: &str, session_id: &str) -> Result<(), AppError> {
        self.companion()?.delete(companion_id, session_id).await?;
        let profile = self.get_companion(companion_id).await?;
        self.emitter.emit_companion_updated(companion_id, &profile);
        Ok(())
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
        let (attempted, failed) = self.set_learning_enabled_for_every_companion(true).await;
        if !failed.is_empty() {
            // Leave the consent flag UNSET so a retry finishes the roster. Setting it
            // here would turn this path into a permanent no-op with those companions
            // never enabled, and the retry is idempotent for the ones that landed.
            return Err(AppError::Internal(format!(
                "collection defaults applied, but learning could not be enabled for {}/{} companions: {}",
                failed.len(),
                attempted,
                failed.join(", ")
            )));
        }
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
    ///
    /// Because of that split there are exactly two failure shapes, and the caller
    /// must be able to tell them apart: `patch_config` failing means nothing
    /// happened, whereas a per-companion failure means collection is already off.
    /// Reporting the second as a bare error reads as "nothing happened" and invites
    /// the user to hit the switch again while their events have in fact already
    /// stopped being recorded — so the error says which half landed.
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
        let (attempted, failed) = self.set_learning_enabled_for_every_companion(false).await;
        if !failed.is_empty() {
            return Err(AppError::Internal(format!(
                "collection is now off, but learning could not be stopped for {}/{} companions: {}",
                failed.len(),
                attempted,
                failed.join(", ")
            )));
        }
        Ok(cfg)
    }

    /// Flip `learn.enabled` + `evolve.enabled` on every companion, emitting one
    /// profile-updated event each so live surfaces follow.
    ///
    /// Best effort by design, returning `(attempted, failed_ids)`. One unwritable
    /// profile must not leave the rest of the roster untouched: aborting on the
    /// first error is the worst of both outcomes for a kill switch — it stops some
    /// companions, reports total failure, and leaves the user unable to tell which.
    /// Every companion is attempted and the failures are named instead.
    async fn set_learning_enabled_for_every_companion(&self, enabled: bool) -> (usize, Vec<String>) {
        let patch = serde_json::json!({
            "learn": { "enabled": enabled },
            "evolve": { "enabled": enabled }
        });
        let ids = self.registry.ids().await;
        let attempted = ids.len();
        let mut failed = Vec::new();
        for companion_id in ids {
            if let Err(error) = self.patch_companion(&companion_id, patch.clone()).await {
                tracing::warn!(
                    companion_id = %companion_id,
                    enabled,
                    error = %error,
                    "companion learning toggle failed; continuing with the rest of the roster"
                );
                failed.push(companion_id);
            }
        }
        (attempted, failed)
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
            automatic_mood: "content".into(),
            // No companion exists to own anything: the only rows left are
            // vestigial unowned ones, so the unscoped count IS the honest total.
            memories_active: self.store.count_memories("active", None).await?,
            memories_archived: self.store.count_memories("archived", None).await?,
            model_configured: false,
            collect_any_enabled: cfg.collect.any_enabled(),
        })
    }

    // ----- memories -----

    pub async fn current_memory_context(
        &self, companion_id: &str, max_bytes: usize,
    ) -> Result<crate::CompanionMemoryContext, AppError> {
        self.get_companion(companion_id).await?;
        self.store.current_memory_context(companion_id, max_bytes).await
    }

    pub async fn memory_message_is_retracted(
        &self, companion_id: &str, session_id: &str, message_id: &str,
    ) -> Result<bool, AppError> {
        self.get_companion(companion_id).await?;
        self.store.memory_message_is_retracted(companion_id, session_id, message_id).await
    }

    pub async fn list_memories(&self, filter: &MemoryFilter) -> Result<Vec<CompanionMemory>, AppError> {
        self.store.list_memories(filter).await
    }

    /// Return the bounded, durable memory projection used by an explicitly
    /// bound Agent capability. Durable source filters and ranking run before
    /// the model-only projection, whose encoded size owns this result budget.
    pub async fn recall_memories_for_agent(
        &self,
        companion_id: &str,
        per_kind: i64,
        char_budget: usize,
    ) -> Result<Vec<crate::AgentMemoryProjection>, AppError> {
        self.get_companion(companion_id).await?;
        if !(1..=20).contains(&per_kind) {
            return Err(AppError::BadRequest(
                "agent memory recall per_kind must be between 1 and 20".into(),
            ));
        }
        if !(1..=64 * 1024).contains(&char_budget) {
            return Err(AppError::BadRequest(
                "agent memory recall char_budget must be between 1 and 65536".into(),
            ));
        }
        let memories = self.store
            .memories_for_injection(companion_id, per_kind, char_budget)
            .await?;
        let mut used = 0_usize;
        let mut bounded = Vec::new();
        for memory in memories.into_iter().map(crate::AgentMemoryProjection::from) {
            let size = serde_json::to_vec(&memory)
                .map_err(|error| AppError::Internal(error.to_string()))?.len();
            if used.saturating_add(size) <= char_budget {
                used += size;
                bounded.push(memory);
            }
        }
        Ok(bounded)
    }

    pub async fn recall_memory_search_for_agent(
        &self, companion_id: &str, mut query: MemorySearchQuery, char_budget: usize,
    ) -> Result<CompanionFactRecallPage, AppError> {
        self.get_companion(companion_id).await?;
        if !(1..=64 * 1024).contains(&char_budget) || !(1..=50).contains(&query.limit)
            || query.queries.len() != 1 || query.queries[0].trim().is_empty() || query.queries[0].chars().count() > 1024
        {
            return Err(AppError::BadRequest("memory recall query exceeds its bounded schema".into()));
        }
        // The selected resource is authoritative. Callers cannot widen scope by
        // constructing an unscoped administrative MemorySearchQuery.
        query.companion_id = Some(CompanionId::try_from(companion_id)
            .map_err(|error| AppError::BadRequest(error.to_string()))?);
        query.status = MemoryStatusFilter::Active;
        let query_limit = query.limit;
        query.limit = query_limit + 1;
        let hits = self.store.search_memories(query).await?;
        let found = hits.len();
        let mut used = 0_usize;
        let mut bounded = Vec::new();
        // Source/session filters have already examined the full durable
        // lineage. Only the bounded presentation is charged to model context.
        for hit in hits.into_iter().take(query_limit).map(crate::AgentMemorySearchHit::from) {
            let size = serde_json::to_vec(&hit).map_err(|error| AppError::Internal(error.to_string()))?.len();
            if used.saturating_add(size) <= char_budget {
                used += size;
                bounded.push(hit);
            }
        }
        Ok(CompanionFactRecallPage { has_more: bounded.len() < found, hits: bounded })
    }

    pub async fn recall_companion_history_for_agent(
        &self, companion_id: &str, query: crate::CompanionHistoryRecallQuery,
    ) -> Result<crate::CompanionHistoryRecallPage, AppError> {
        self.get_companion(companion_id).await?;
        if query.query.trim().is_empty() || query.query.chars().count() > 1024
            || !(1..=50).contains(&query.limit) || !(256..=64 * 1024).contains(&query.byte_budget)
            || query.since_ts.is_some_and(|ts| ts < 0) || query.until_ts.is_some_and(|ts| ts < 0)
            || matches!((query.since_ts, query.until_ts), (Some(start), Some(end)) if start > end)
        {
            return Err(AppError::BadRequest("history recall query exceeds its bounded schema".into()));
        }
        if let Some(session_id) = query.source_session_id.as_deref() {
            nomifun_common::ConversationId::try_from(session_id)
                .map_err(|error| AppError::BadRequest(format!("invalid source_session_id: {error}")))?;
        }
        if let Some(cursor) = query.before.as_ref() {
            if cursor.recorded_at < 0 || cursor.source_seq == 0 {
                return Err(AppError::BadRequest("invalid history recall cursor position".into()));
            }
            nomifun_common::ConversationId::try_from(cursor.session_id.as_str())
                .map_err(|error| AppError::BadRequest(format!("invalid cursor Session id: {error}")))?;
        }
        self.companion()?.sessions.recall_companion_history(self.authoritative_user_id.as_ref(), companion_id, query).await
    }

    pub async fn add_memory_for_agent_with_sources(
        &self, companion_id: &str, kind: &str, content: &str, tags: &[String],
        source_session_id: &str, source_operation_id: &str,
        sources: &[crate::store::CompanionMemoryMessageSource],
    ) -> Result<CompanionMemory, AppError> {
        if sources.is_empty() {
            return Err(AppError::BadRequest("Agent memory writes require verified canonical human input sources".into()));
        }
        self.get_companion(companion_id).await?;
        if !crate::store::MEMORY_KINDS.contains(&kind) || content.trim().is_empty() {
            return Err(AppError::BadRequest("invalid memory kind or empty content".into()));
        }
        let memory = self.store.add_agent_memory_with_sources(kind, content.trim(), tags, companion_id,
            source_session_id, source_operation_id, sources).await?;
        if memory.source_session_id.as_deref() == Some(source_session_id)
            && memory.source_operation_id.as_deref() == Some(source_operation_id)
        { self.emitter.emit_memory_created(&memory); }
        else { self.emitter.emit_memory_updated(&memory); }
        Ok(memory)
    }

    pub async fn update_memory_for_agent_cas_with_sources(
        &self, companion_id: &str, memory_id: &str, expected_revision: i64, kind: &str, content: &str,
        sources: &[crate::store::CompanionMemoryMessageSource],
    ) -> Result<CompanionMemory, AppError> {
        if sources.is_empty() {
            return Err(AppError::BadRequest("Agent memory corrections require verified canonical human input sources".into()));
        }
        self.get_companion(companion_id).await?;
        self.require_exact_companion_memory(companion_id, memory_id).await?;
        let old = self.store.get_memory(memory_id).await?.ok_or_else(|| AppError::NotFound("memory not found".into()))?;
        if old.kind != kind {
            return Err(AppError::BadRequest("a memory correction cannot change its kind".into()));
        }
        let updated = self.store.update_memory_cas_with_sources(memory_id, expected_revision, Some(content), None, None,
            &MemoryActor::Companion(companion_id.to_owned()), sources).await?;
        self.emitter.emit_memory_updated(&updated);
        Ok(updated)
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
        owner: Option<String>,
        sort: &str,
        limit: i64,
        offset: i64,
    ) -> Result<MemoryListPage, AppError> {
        let companion_id = owner
            .as_deref()
            .map(|id| {
                CompanionId::try_from(id).map_err(|error| {
                    AppError::BadRequest(format!("invalid companion_id: {error}"))
                })
            })
            .transpose()?;
        let query = MemorySearchQuery {
            queries: vec![q.to_owned()],
            kind,
            status,
            companion_id,
            limit: 500,
            ..Default::default()
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
    /// `actor` owns the whole batch: one foreign id rolls all of it back.
    pub async fn batch_memories(
        &self,
        ids: &[String],
        action: &MemoryBatchAction,
        actor: &MemoryActor,
    ) -> Result<(), AppError> {
        self.store.batch_update_memories(ids, action, actor).await?;
        for id in ids {
            match self.store.get_memory(id).await {
                Ok(Some(updated)) => self.emitter.emit_memory_updated(&updated),
                Ok(None) => self.emitter.emit_memory_deleted(id),
                Err(_) => {}
            }
        }
        Ok(())
    }

    pub async fn batch_memories_cas(
        &self, ids: &[String], action: &MemoryBatchAction, actor: &MemoryActor,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
    ) -> Result<(), AppError> {
        self.store.batch_update_memories_cas(ids, action, actor, expected_revisions).await?;
        for id in ids {
            match self.store.get_memory(id).await? {
                Some(memory) => self.emitter.emit_memory_updated(&memory),
                None => self.emitter.emit_memory_deleted(id),
            }
        }
        Ok(())
    }

    /// Merge-assistant dry run for ONE companion: suspected-duplicate groups over
    /// the active layer it can see (per kind + scope, normalized-similarity
    /// clustering).
    ///
    /// Scoped in the store, not here and certainly not on the client: this feeds a
    /// surface that belongs to a single companion, and memory is owned, so another
    /// companion's memory text has no business being on that wire.
    pub async fn memory_merge_suggestions(&self, companion_id: &str) -> Result<Vec<MemoryMergeGroup>, AppError> {
        // Existence gate: an unknown companion must 404 rather than read as
        // "nothing to merge".
        self.get_companion(companion_id).await?;
        let active = self.store.dump_active_memories_visible_to(companion_id).await?;
        Ok(group_similar_memories(active))
    }

    /// Merge-assistant confirm: persist the merged memory, archive the source
    /// group (audit-tagged `superseded_by:{id}`), and notify open surfaces.
    pub async fn merge_memories(
        &self,
        group: &[String],
        merged_content: &str,
        kind: &str,
        actor: &MemoryActor,
    ) -> Result<CompanionMemory, AppError> {
        let merged = self.store.merge_memories(group, merged_content, kind, actor).await?;
        self.emitter.emit_memory_created(&merged);
        for id in group {
            if let Ok(Some(archived)) = self.store.get_memory(id).await {
                self.emitter.emit_memory_updated(&archived);
            }
        }
        Ok(merged)
    }

    pub async fn merge_memories_cas(
        &self, group: &[String], merged_content: &str, kind: &str, actor: &MemoryActor,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
    ) -> Result<CompanionMemory, AppError> {
        let merged = self.store.merge_memories_cas(group, merged_content, kind, actor, expected_revisions).await?;
        self.emitter.emit_memory_created(&merged);
        for id in group {
            if let Some(memory) = self.store.get_memory(id).await? { self.emitter.emit_memory_updated(&memory); }
        }
        Ok(merged)
    }

    /// Agent-facing merge over one explicitly bound companion. Unlike the
    /// administrative merge surface, this refuses vestigial unowned rows as
    /// well as rows owned by another companion before entering the atomic
    /// store transition.
    pub async fn merge_companion_memories_for_agent(
        &self,
        companion_id: &str,
        group: &[String],
        merged_content: &str,
        kind: &str,
    ) -> Result<CompanionMemory, AppError> {
        self.get_companion(companion_id).await?;
        for memory_id in group {
            self.require_exact_companion_memory(companion_id, memory_id)
                .await?;
        }
        self.merge_memories(
            group,
            merged_content,
            kind,
            &MemoryActor::Companion(companion_id.to_owned()),
        )
        .await
    }

    // ----- session-window day digests (伙伴会话归档回看) -----

    /// The complete day index of this companion's history: every LOCAL calendar
    /// day its Sessions hold visible messages on, plus every day that has an
    /// archived digest, newest first.
    ///
    /// Strictly read-only. The conversation is RESOLVED from the stored pointer,
    /// never minted: `create_companion_thread` 400s for a companion with no model
    /// configured, and a history reader must never fail for that reason. A
    /// companion that has never chatted — or whose conversation was deleted
    /// out-of-band, leaving a dangling pointer — yields an empty index rather
    /// than an error.
    pub async fn history_day_index(&self, companion_id: &str) -> Result<Vec<CompanionHistoryDay>, AppError> {
        self.history_day_index_for_session(companion_id, None).await
    }

    pub async fn history_day_index_for_session(
        &self, companion_id: &str, conversation_id: Option<&str>,
    ) -> Result<Vec<CompanionHistoryDay>, AppError> {
        // Existence gate: an unknown companion must 404, not read as "no history".
        self.get_companion(companion_id).await?;
        if let Some(session_id) = conversation_id {
            self.companion()?.assert_owned(companion_id, session_id).await?;
        }
        let digest_days: std::collections::HashSet<String> = match conversation_id {
            Some(session_id) => self.store.archived_digest_days_for_session(companion_id, session_id).await?,
            None => self.store.archived_digest_days(companion_id).await?,
        }
            .into_iter()
            .collect();
        let mut counts = Vec::new();
        let registered = self.store.list_companion_threads(Some(companion_id)).await?;
        let threads = if registered.is_empty() { Vec::new() } else { self.list_companion_threads(companion_id).await? };
        for thread in threads.into_iter().filter(|thread| conversation_id.is_none_or(|id| id == thread.conversation_id)) {
            match self.companion()?.sessions.message_local_day_index(
                self.authoritative_user_id.as_ref(), &thread.conversation_id,
            ).await {
                Ok(buckets) => counts.extend(buckets),
                Err(AppError::NotFound(_)) => {},
                Err(error) => return Err(error),
            }
        }
        // A day carrying only a digest (its messages were cleared) must still be
        // reachable, so the index is the union of both sources.
        let mut days: std::collections::BTreeMap<String, i64> = digest_days
            .iter()
            .map(|day| (day.clone(), 0))
            .collect();
        for bucket in counts {
            *days.entry(bucket.day).or_default() += bucket.message_count;
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

    pub async fn list_day_digests_for_session(
        &self, companion_id: &str, conversation_id: &str, since: &str, until: &str, limit: i64,
    ) -> Result<Vec<crate::store::SessionWindow>, AppError> {
        self.companion()?.assert_owned(companion_id, conversation_id).await?;
        self.store.digests_for_session(companion_id, conversation_id, since, until, limit).await
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

    pub async fn digests_on_this_day_for_session(
        &self, companion_id: &str, conversation_id: &str, mmdd: &str, exclude_day: &str, limit: i64,
    ) -> Result<Vec<crate::store::SessionWindow>, AppError> {
        self.companion()?.assert_owned(companion_id, conversation_id).await?;
        self.store.digests_on_day_of_year_for_session(companion_id, conversation_id, mmdd, exclude_day, limit).await
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
            .insert_memory_scoped(kind, content, tags, 0.8, "manual", Some(&owner))
            .await?;
        self.emitter.emit_memory_created(&mem);
        Ok(mem)
    }

    /// Edit a memory's content / pin / lifecycle. Ownership is immutable: there
    /// is no re-homing wire any more, so an edit can never move a memory between
    /// companions — and `actor` decides which rows are addressable at all.
    pub async fn update_memory(
        &self,
        memory_id: &str,
        content: Option<&str>,
        pinned: Option<bool>,
        status: Option<&str>,
        actor: &MemoryActor,
    ) -> Result<(), AppError> {
        if let Some(status) = status {
            if status != "active" && status != "archived" {
                return Err(AppError::BadRequest(format!("invalid memory status '{status}'")));
            }
        }
        self.store
            .update_memory(memory_id, content, pinned, status, actor)
            .await?;
        // Notify open surfaces with the post-edit row (best-effort; a missing
        // row already errored above).
        if let Ok(Some(updated)) = self.store.get_memory(memory_id).await {
            self.emitter.emit_memory_updated(&updated);
        }
        Ok(())
    }

    pub async fn update_memory_cas(
        &self, memory_id: &str, expected_revision: i64, content: Option<&str>, pinned: Option<bool>,
        status: Option<&str>, actor: &MemoryActor,
    ) -> Result<CompanionMemory, AppError> {
        if status.is_some_and(|status| !matches!(status, "active" | "archived")) {
            return Err(AppError::BadRequest("invalid memory status".into()));
        }
        let updated = self.store.update_memory_cas(memory_id, expected_revision, content, pinned, status, actor).await?;
        self.emitter.emit_memory_updated(&updated);
        Ok(updated)
    }

    /// Evolve one durable memory in place for the explicitly bound companion.
    /// The store preserves identity, kind, lifecycle, pin and ownership while
    /// atomically replacing/redacting content and refreshing its FTS row.
    pub async fn evolve_companion_memory_for_agent(
        &self,
        companion_id: &str,
        memory_id: &str,
        content: &str,
    ) -> Result<CompanionMemory, AppError> {
        self.get_companion(companion_id).await?;
        self.require_exact_companion_memory(companion_id, memory_id)
            .await?;
        self.update_memory(
            memory_id,
            Some(content),
            None,
            None,
            &MemoryActor::Companion(companion_id.to_owned()),
        )
        .await?;
        self.require_exact_companion_memory(companion_id, memory_id)
            .await
    }

    async fn require_exact_companion_memory(
        &self,
        companion_id: &str,
        memory_id: &str,
    ) -> Result<CompanionMemory, AppError> {
        match self.store.get_memory(memory_id).await? {
            Some(memory) if memory.companion_id.as_deref() == Some(companion_id) => Ok(memory),
            Some(_) | None => Err(AppError::NotFound(format!(
                "memory '{memory_id}' not found"
            ))),
        }
    }

    pub async fn delete_memory(&self, memory_id: &str, actor: &MemoryActor) -> Result<(), AppError> {
        self.store.delete_memory(memory_id, actor).await?;
        self.emitter.emit_memory_deleted(memory_id);
        Ok(())
    }

    pub async fn delete_memory_cas(&self, memory_id: &str, expected_revision: i64, actor: &MemoryActor) -> Result<(), AppError> {
        self.store.delete_memory_cas(memory_id, expected_revision, actor).await?;
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
            let scope = scope_for(skill.companion_id.as_deref());
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
        let scope = scope_for(skill.companion_id.as_deref());
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
        let scope = scope_for(skill.companion_id.as_deref());
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

    /// Run one explicit skill-evolution pass for exactly one live companion.
    ///
    /// The Agent capability adapter uses this same owner as the background
    /// scheduler; it does not construct a second evolution engine or fall back
    /// to the installation's default companion.
    pub async fn run_evolve_now(
        &self,
        companion_id: &str,
    ) -> Result<EvolveRun, AppError> {
        self.evolution.run_for(companion_id).await
    }

    /// Build the current persona/memory prompt for one explicitly selected
    /// companion resource.
    ///
    /// Unknown or deleted ids remain errors. This is intentionally stricter
    /// than a display-oriented optional lookup because an Agent resource
    /// binding must never silently switch to another companion.
    pub async fn build_bound_system_prompt(
        &self,
        companion_id: &str,
        channel_platform: Option<&str>,
    ) -> Result<String, AppError> {
        let profile = self.get_companion(companion_id).await?;
        let smart = self.config.read().await.smart_collaboration;
        Ok(crate::companion::build_companion_system_prompt(
            &self.store,
            &profile,
            channel_platform,
            smart,
        )
        .await)
    }

    pub(crate) async fn context_role(
        &self, companion_id:&str, session_id:&str,
        expected_snapshot:&nomifun_agent_contracts::ResolvedSnapshotRef,
    )->Result<Option<crate::CompanionRole>,AppError>{
        self.companion()?.sessions.context_role(self.authoritative_user_id.as_ref(),companion_id,session_id,expected_snapshot).await
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

/// Display suffix for one profile Provider slot in the deletion-blocking usage
/// report. Empty = the companion's own chat model, which needs no suffix.
fn slot_display_label(slot: &str) -> &'static str {
    match slot {
        "learn" => "学习模型",
        "evolve" => "进化模型",
        "fallback" => "备用模型",
        "vision" => "视觉模型",
        "asr" => "语音识别模型",
        "tts" => "语音合成模型",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_realtime::BroadcastEventBus;

    #[test]
    fn companion_workspace_cleanup_normalizes_real_paths_and_rejects_outside_roots() {
        let temporary = tempfile::tempdir().unwrap();
        let owner = temporary.path().join("managed-owner");
        let leaf = owner.join("owned-leaf");
        let outside = temporary.path().join("outside");
        std::fs::create_dir_all(&leaf).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(leaf.join("owned.txt"), "owned").unwrap();
        std::fs::write(outside.join("private.txt"), "preserve").unwrap();
        let owner_verbatim = std::fs::canonicalize(&owner).unwrap();
        let leaf_verbatim = std::fs::canonicalize(&leaf).unwrap();
        let owner_simple = nomifun_common::paths::canonicalize_simplified(&owner).unwrap();
        let leaf_simple = nomifun_common::paths::canonicalize_simplified(&leaf).unwrap();
        // Real host Guards contain paths resolved while the directory exists.
        // Simulate a directory disappearing after that proof, rather than an
        // unproven missing path built from TEMP's possible Windows 8.3 alias.
        let removed = owner.join("already-removed");
        std::fs::create_dir(&removed).unwrap();
        let removed_verbatim = std::fs::canonicalize(&removed).unwrap();
        let removed_simple = nomifun_common::paths::canonicalize_simplified(&removed).unwrap();
        std::fs::remove_dir(&removed).unwrap();
        let outside_missing = nomifun_common::paths::canonicalize_simplified(&outside).unwrap().join("never-owned");
        #[cfg(windows)]
        assert_ne!(leaf_verbatim, leaf_simple, "the real filesystem fixture must exercise Windows verbatim and simplified spellings");
        let managed = vec![leaf_verbatim.to_string_lossy().into_owned(), leaf_simple.to_string_lossy().into_owned()];
        for root in [&owner_verbatim, &owner_simple] {
            let targets = managed_workspace_deletion_targets(root, &managed).unwrap();
            assert_eq!(targets, std::collections::BTreeSet::from([leaf_simple.clone()]));
            for rejected in [
                root.to_string_lossy().into_owned(),
                outside.to_string_lossy().into_owned(),
                "relative-leaf".into(),
                leaf.join("..").join("..").join("outside").to_string_lossy().into_owned(),
                outside_missing.to_string_lossy().into_owned(),
            ] {
                assert!(managed_workspace_deletion_targets(root, &[rejected.clone()]).is_err(), "reject outside or non-absolute target {rejected}");
            }
            assert!(managed_workspace_deletion_targets(root, &[
                removed_verbatim.to_string_lossy().into_owned(), removed_simple.to_string_lossy().into_owned(),
            ]).unwrap().is_empty(), "a proven owned leaf removed after Guard acquisition remains an idempotent no-op");
        }
        assert!(leaf.join("owned.txt").is_file(), "preflight itself never deletes files");
        assert_eq!(std::fs::read_to_string(outside.join("private.txt")).unwrap(), "preserve");
    }

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
        async fn complete(&self, _p: &str, _m: &str, _s: &str, _u: &str, _t: Option<u32>) -> Result<String, AppError> {
            Ok("{}".into())
        }
    }

    async fn service(data_dir: &std::path::Path) -> Arc<CompanionService> {
        CompanionService::start(
            data_dir,
            Arc::new(BroadcastEventBus::new(16)),
            "owner-a",
            Arc::new(NoopCompleter),
            Arc::new(nomifun_skill_library::skill_service::resolve_skill_paths(data_dir, data_dir)),
        )
        .await
        .unwrap()
    }

    fn attach_canonical_test_host(svc: &Arc<CompanionService>) -> Arc<dyn crate::CompanionSessionPort> {
        struct UnusedArchive;
        #[async_trait::async_trait]
        impl crate::archiver::ArchiveConversationPort for UnusedArchive {
            async fn window_messages(&self, _: &str, _: i64, _: &str) -> Result<crate::archiver::ArchiveWindow, AppError> {
                Err(AppError::Internal("archive is outside this lifecycle fixture".into()))
            }
            async fn reset_context(&self, _: &str, _: &str, _: &nomifun_agent_session::ContextClearBoundary) -> Result<(), AppError> {
                Err(AppError::Internal("archive is outside this lifecycle fixture".into()))
            }
        }
        let sessions = crate::companion::thread_management_tests::canonical_host();
        svc.attach_companion(CompanionHostPorts { sessions: sessions.clone(), archive: Arc::new(UnusedArchive), transcript: Arc::new(NoopTranscriptSource) });
        sessions
    }

    #[tokio::test]
    async fn bound_persona_context_preserves_identity_without_advertising_tool_grants() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let selected = svc.create_companion("Selected", "ink").await.unwrap();
        let other = svc.create_companion("Other", "ink").await.unwrap();
        svc.patch_companion(
            &selected.companion_id,
            serde_json::json!({"persona": {"preset": "calm", "custom": "叫主人老大"}}),
        )
        .await
        .unwrap();
        svc.config.write().await.smart_collaboration = true;
        let owner = crate::agent_capability::CompanionAgentCapabilityOwner::new(
            "owner-a",
            Arc::clone(&svc),
        );
        // Persona read is available even when the frozen Agent grants none of
        // the companion's memory/actions or any platform-management tools.
        let resources = owner
            .resolve_scene_bindings(
                "owner-a",
                &selected.companion_id,
                &std::collections::BTreeSet::new(),
            )
            .await
            .unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0].operations,
            std::collections::BTreeSet::from(["read".to_owned()]),
        );
        let context = owner.persona_context("owner-a", &resources).await.unwrap();
        assert_eq!(context.companion_id, selected.companion_id);
        assert!(context.system_prompt.contains("你是 Selected"));
        assert!(context.system_prompt.contains("沉稳温柔"));
        assert!(context.system_prompt.contains("叫主人老大"));
        assert!(!context.system_prompt.contains(&other.companion_id));
        assert!(context.system_prompt.contains("当前运行时的已授权工具目录和工具说明为准"));
        assert!(context.system_prompt.contains("当前提供委派工具时"));
        assert!(!context.system_prompt.contains("nomi_"));
        assert!(!context.system_prompt.contains("完整的工具能力"));
    }

    #[tokio::test]
    async fn agent_recall_keeps_short_facts_with_large_durable_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let companion = svc.create_companion("Lineage", "ink").await.unwrap();
        let memory = svc.add_memory("preference", "Use lineageproof coffee.", &[],
            Some(&companion.companion_id)).await.unwrap();
        let source_session = nomifun_common::ConversationId::new().into_string();
        let sources = (0..1024).map(|_| crate::store::CompanionMemoryMessageSource {
            session_id: source_session.clone(), message_id: nomifun_common::generate_id(),
        }).collect::<Vec<_>>();
        let events = (0..1024).map(|_| nomifun_common::generate_id()).collect::<Vec<_>>();
        sqlx::query("UPDATE companion_memories SET source_event_ids = ?, source_message_refs = ? WHERE memory_id = ?")
            .bind(serde_json::to_string(&events).unwrap())
            .bind(serde_json::to_string(&sources).unwrap())
            .bind(&memory.memory_id).execute(svc.store.pool()).await.unwrap();
        let filter = MemoryFilter { companion_id: Some(companion.companion_id.clone()), ..Default::default() };
        let full_before = svc.list_memories(&filter).await.unwrap();
        assert_eq!(full_before.len(), 1);
        assert!(serde_json::to_vec(&full_before[0]).unwrap().len() > 64 * 1024);

        let default = svc.recall_memories_for_agent(&companion.companion_id, 8, 1200).await.unwrap();
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].memory_id, memory.memory_id);
        assert!(serde_json::to_vec(&default).unwrap().len() <= 1200);
        let query = svc.recall_memory_search_for_agent(&companion.companion_id, MemorySearchQuery {
            queries: vec!["lineageproof".into()], source_session_id: Some(source_session),
            ..Default::default()
        }, 1200).await.unwrap();
        assert_eq!(query.hits.len(), 1, "durable source filtering must precede model projection");
        assert_eq!(query.hits[0].memory.memory_id, memory.memory_id);
        assert!(!query.has_more);
        assert!(serde_json::to_vec(&query).unwrap().len() <= 1200);
        for value in [serde_json::to_value(&default[0]).unwrap(), serde_json::to_value(&query.hits[0].memory).unwrap()] {
            assert!(value.get("source_event_ids").is_none());
            assert!(value.get("source_message_refs").is_none());
            assert_eq!(value["revision"], memory.revision);
            assert_eq!(value["content"], memory.content);
        }
        assert_eq!(svc.list_memories(&filter).await.unwrap(), full_before,
            "Agent presentation must retain all lineage for REST, export and retraction");
    }

    #[tokio::test]
    async fn agent_memory_owner_recalls_writes_merges_evolves_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let first_service = service(dir.path()).await;
        let primary = first_service.create_companion("Primary", "ink").await.unwrap();
        let other = first_service.create_companion("Other", "ink").await.unwrap();
        let first = first_service
            .add_memory(
                "preference",
                "Prefer release notes with concrete verification evidence.",
                &["release".to_owned()],
                Some(&primary.companion_id),
            )
            .await
            .unwrap();
        let second = first_service
            .add_memory(
                "preference",
                "Keep unrelated working-tree changes intact.",
                &["git".to_owned()],
                Some(&primary.companion_id),
            )
            .await
            .unwrap();
        let foreign = first_service
            .add_memory(
                "preference",
                "This belongs to another companion.",
                &[],
                Some(&other.companion_id),
            )
            .await
            .unwrap();

        let recalled = first_service
            .recall_memories_for_agent(&primary.companion_id, 20, 48 * 1024)
            .await
            .unwrap();
        assert!(recalled.iter().any(|memory| memory.memory_id == first.memory_id));
        assert!(recalled.iter().any(|memory| memory.memory_id == second.memory_id));
        assert!(!recalled.iter().any(|memory| memory.memory_id == foreign.memory_id));

        let merged = first_service
            .merge_companion_memories_for_agent(
                &primary.companion_id,
                &[first.memory_id.clone(), second.memory_id.clone()],
                "Release notes must cite concrete checks and preserve unrelated changes.",
                "preference",
            )
            .await
            .unwrap();
        assert_eq!(merged.source, "merge");
        assert_eq!(merged.companion_id.as_deref(), Some(primary.companion_id.as_str()));
        let evolved = first_service
            .evolve_companion_memory_for_agent(
                &primary.companion_id,
                &merged.memory_id,
                "Release notes must cite executed checks, preserve unrelated changes, and name remaining blockers.",
            )
            .await
            .unwrap();
        assert_eq!(evolved.memory_id, merged.memory_id);
        assert!(evolved.content.contains("remaining blockers"));

        assert!(
            first_service
                .evolve_companion_memory_for_agent(
                    &primary.companion_id,
                    &foreign.memory_id,
                    "must not cross owners",
                )
                .await
                .is_err()
        );
        assert!(
            first_service
                .merge_companion_memories_for_agent(
                    &primary.companion_id,
                    &[evolved.memory_id.clone(), foreign.memory_id],
                    "must not cross owners",
                    "preference",
                )
                .await
                .is_err()
        );

        let restarted = service(dir.path()).await;
        let after_restart = restarted
            .recall_memories_for_agent(&primary.companion_id, 20, 48 * 1024)
            .await
            .unwrap();
        assert!(after_restart.iter().any(|memory| {
            memory.memory_id == evolved.memory_id && memory.content == evolved.content
        }));
        assert!(!after_restart.iter().any(|memory| memory.memory_id == first.memory_id));
        assert!(!after_restart.iter().any(|memory| memory.memory_id == second.memory_id));
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
            Arc::new(nomifun_skill_library::skill_service::resolve_skill_paths(
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

    async fn model_detachment_service(
        data_dir: &std::path::Path,
    ) -> (
        Arc<CompanionService>,
        SharedProviderLifecycleBarrier,
        Arc<BroadcastEventBus>,
    ) {
        let barrier = Arc::new(nomifun_common::ProviderLifecycleBarrier::new());
        let bus = Arc::new(BroadcastEventBus::new(32));
        let svc = CompanionService::start_with_provider_lifecycle(
            data_dir,
            Arc::clone(&bus),
            "owner-a",
            Arc::new(NoopCompleter),
            Arc::new(nomifun_skill_library::skill_service::resolve_skill_paths(data_dir, data_dir)),
            None,
            None,
            Some(Arc::clone(&barrier)),
        )
        .await
        .unwrap();
        (svc, barrier, bus)
    }

    #[tokio::test]
    async fn model_detachment_clears_all_seven_slots_under_the_provider_write_guard() {
        struct ModelChangeHook(std::sync::atomic::AtomicUsize);
        #[async_trait::async_trait]
        impl CompanionCleanupHook for ModelChangeHook {
            async fn on_companion_deleted(&self, _: &str) {}
            async fn on_companion_model_changed(
                &self,
                _: &str,
                _: Option<&nomifun_common::ProviderWithModel>,
            ) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let (svc, barrier, bus) = model_detachment_service(dir.path()).await;
        let selected = svc.create_companion("Selected", "ink").await.unwrap();
        let untouched = svc.create_companion("Untouched", "boo").await.unwrap();
        let provider_id = provider_fixture(20);
        let model = serde_json::json!({"provider_id": provider_id, "model": "removed"});
        let before = svc.patch_companion(&selected.companion_id, serde_json::json!({
            "model": model, "learn": {"model": model}, "evolve": {"model": model},
            "fallback_model": model, "vision_model": model,
            "voice": {"asr": model, "tts": {
                "provider_id": provider_id, "model": "removed", "voice": "selected-voice"
            }},
            "persona": {"custom": "Preserve this preference"}, "order_index": 7
        })).await.unwrap();
        assert_eq!(before.provider_model_slots().len(), 7);
        let hook = Arc::new(ModelChangeHook(std::sync::atomic::AtomicUsize::new(0)));
        svc.set_cleanup_hooks(vec![hook.clone() as Arc<dyn CompanionCleanupHook>]);

        let conversation = conversation_fixture(20);
        svc.store.insert_companion_thread(&conversation, &selected.companion_id, "Existing").await.unwrap();
        crate::companion::set_active_thread_ptr(&svc.store, &selected.companion_id, Some(&conversation)).await.unwrap();
        let window = svc.store.ensure_open_window(&selected.companion_id, &conversation, 1).await.unwrap();
        svc.store.close_window(&window.session_window_id, "archived", Some("Existing digest"), None, 5).await.unwrap();
        let archived_before = serde_json::to_value(svc.store.list_digests(&selected.companion_id, 10).await.unwrap()).unwrap();
        let shared_before = svc.get_config().await;
        let mut events = bus.subscribe_user();

        let _write_guard = barrier.write().await;
        let changed = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            svc.detach_model_references_under_provider_guard(&provider_id, None),
        ).await.expect("detachment must not reacquire the lifecycle read lock").unwrap();
        assert_eq!(changed, 1);
        let actual = svc.get_companion(&selected.companion_id).await.unwrap();
        let mut expected = before;
        expected.model = None;
        expected.learn.model = None;
        expected.evolve.model = None;
        expected.fallback_model = None;
        expected.vision_model = None;
        expected.voice.asr = None;
        expected.voice.tts = None;
        assert_eq!(actual, expected);
        assert_eq!(svc.get_companion(&untouched.companion_id).await.unwrap(), untouched);
        let profile_dir = dir.path().join(crate::COMPANION_COMPANIONS_REL_DIR).join(&selected.companion_id);
        assert_eq!(CompanionProfileConfig::load(&profile_dir).unwrap().unwrap(), expected);
        let event = events.try_recv().unwrap();
        assert_eq!(event.user_id, "owner-a");
        assert_eq!(event.event.name, "companion.config-updated");
        assert_eq!(event.event.data["scope"], selected.companion_id);
        assert!(event.event.data["model"].is_null());
        assert!(matches!(events.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
        assert_eq!(svc.detach_model_references_under_provider_guard(&provider_id, None).await.unwrap(), 0);
        assert!(matches!(events.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));

        assert_eq!(hook.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(svc.get_config().await, shared_before);
        assert_eq!(svc.store.thread_companion_id(&conversation).await.unwrap().as_deref(), Some(selected.companion_id.as_str()));
        assert_eq!(crate::companion::active_thread_ptr(&svc.store, &selected.companion_id).await.unwrap().as_deref(), Some(conversation.as_str()));
        assert_eq!(serde_json::to_value(svc.store.list_digests(&selected.companion_id, 10).await.unwrap()).unwrap(), archived_before);
    }

    #[tokio::test]
    async fn model_detachment_matches_the_exact_provider_and_model_without_replacing_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, barrier, _) = model_detachment_service(dir.path()).await;
        let profile = svc.create_companion("Exact", "ink").await.unwrap();
        let provider_id = provider_fixture(21);
        let other_provider = provider_fixture(22);
        let before = svc.patch_companion(&profile.companion_id, serde_json::json!({
            "model": {"provider_id": provider_id, "model": "removed"},
            "learn": {"model": {"provider_id": provider_id, "model": "sibling"}},
            "evolve": {"model": {"provider_id": other_provider, "model": "removed"}},
            "fallback_model": {"provider_id": provider_id, "model": "removed-preview"},
            "vision_model": {"provider_id": provider_id, "model": "Removed"},
            "voice": {
                "asr": {"provider_id": provider_id, "model": "removed"},
                "tts": {"provider_id": provider_id, "model": "removed", "voice": "custom"}
            }
        })).await.unwrap();
        let _write_guard = barrier.write().await;
        assert_eq!(svc.detach_model_references_under_provider_guard(&provider_id, Some("removed")).await.unwrap(), 1);
        let mut expected = before;
        expected.model = None;
        expected.voice.asr = None;
        expected.voice.tts = None;
        assert_eq!(svc.get_companion(&profile.companion_id).await.unwrap(), expected);
        assert_eq!(svc.detach_model_references_under_provider_guard(&provider_id, Some("removed")).await.unwrap(), 0);
        assert_eq!(svc.detach_model_references_under_provider_guard(&provider_id, None).await.unwrap(), 1);
        expected.learn.model = None;
        expected.fallback_model = None;
        expected.vision_model = None;
        assert_eq!(svc.get_companion(&profile.companion_id).await.unwrap(), expected);
        assert_eq!(expected.evolve.model.unwrap().provider_id, other_provider);
    }

    #[tokio::test]
    async fn model_detachment_failed_save_preserves_all_selections_and_emits_no_update() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, barrier, bus) = model_detachment_service(dir.path()).await;
        let profile = svc.create_companion("Fail closed", "ink").await.unwrap();
        let provider_id = provider_fixture(23);
        let before = svc.patch_companion(&profile.companion_id, serde_json::json!({
            "model": {"provider_id": provider_id, "model": "removed"},
            "fallback_model": {"provider_id": provider_id, "model": "sibling"},
            "voice": {"tts": {"provider_id": provider_id, "model": "voice", "voice": "keep"}}
        })).await.unwrap();
        let profile_dir = dir.path().join(crate::COMPANION_COMPANIONS_REL_DIR).join(&profile.companion_id);
        let config_path = CompanionProfileConfig::config_path(&profile_dir);
        let saved = std::fs::read(&config_path).unwrap();
        std::fs::remove_file(&config_path).unwrap();
        std::fs::create_dir(&config_path).unwrap();
        let mut events = bus.subscribe_user();
        let _write_guard = barrier.write().await;
        let error = svc.detach_model_references_under_provider_guard(&provider_id, Some("removed")).await.unwrap_err();
        assert!(error.to_string().contains("save companion profile"), "{error}");
        assert_eq!(svc.get_companion(&profile.companion_id).await.unwrap(), before);
        assert!(config_path.is_dir());
        assert!(matches!(events.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));

        std::fs::remove_dir(&config_path).unwrap();
        std::fs::write(&config_path, saved).unwrap();
        assert_eq!(CompanionProfileConfig::load(&profile_dir).unwrap().unwrap(), before);
        assert_eq!(svc.detach_model_references_under_provider_guard(&provider_id, Some("removed")).await.unwrap(), 1);
        let mut expected = before;
        expected.model = None;
        assert_eq!(CompanionProfileConfig::load(&profile_dir).unwrap().unwrap(), expected);
    }

    #[tokio::test]
    async fn model_detachment_rejects_invalid_identifiers_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, barrier, bus) = model_detachment_service(dir.path()).await;
        let profile = svc.create_companion("Keep", "ink").await.unwrap();
        let mut events = bus.subscribe_user();
        let provider_id = provider_fixture(24);
        let _write_guard = barrier.write().await;
        for (provider, model) in [(MALFORMED_PROVIDER_ID, None), (&provider_id, Some("  "))] {
            assert!(matches!(svc.detach_model_references_under_provider_guard(provider, model).await, Err(AppError::BadRequest(_))));
        }
        assert_eq!(svc.get_companion(&profile.companion_id).await.unwrap(), profile);
        assert!(matches!(events.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn accepting_create_skill_promotes_draft_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let companion = svc.registry.create("测试", "ink").await.unwrap();
        let cid = companion.companion_id;

        // A reviewed draft: SKILL.md on disk (draft dir) + a draft registry row.
        let input = nomifun_skill_library::skill_service::SkillDraftInput {
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
                companion_id: Some(cid.clone()),
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
        let input = nomifun_skill_library::skill_service::SkillDraftInput {
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
        let input = nomifun_skill_library::skill_service::SkillDraftInput {
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
                companion_id: Some(cid.to_owned()),
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
                companion_id: Some(cid.clone()),
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
        let input = nomifun_skill_library::skill_service::SkillDraftInput {
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
                companion_id: Some(cid.clone()),
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
                companion_id: Some(cid.clone()),
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
    /// regression net for the list query — a resurrected `OR companion_id IS NULL`
    /// or a copy path would show up here as a second row.
    #[tokio::test]
    async fn a_companions_skill_list_is_exactly_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.registry.create("A", "ink").await.unwrap().companion_id;
        let b = svc.registry.create("B", "ink").await.unwrap().companion_id;
        let input = nomifun_skill_library::skill_service::SkillDraftInput {
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
            companion_id: Some(a.clone()),
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
        skill.companion_id = None;
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

    /// A kill switch that gives up half way is the worst outcome: it stops some
    /// companions, reports total failure, and leaves the user unable to tell which.
    /// So one unwritable profile must not spare the rest of the roster, and the
    /// error must say that collection已经 stopped — reporting a bare failure reads as
    /// "nothing happened" and invites a second press while events have in fact
    /// already stopped being recorded.
    #[tokio::test]
    async fn disable_all_stops_the_rest_of_the_roster_when_one_profile_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        svc.patch_config(serde_json::json!({
            "collect": { "tool_calls": true, "companion_dialogues": true }
        }))
        .await
        .unwrap();
        let mut ids = Vec::new();
        for name in ["甲", "乙", "丙"] {
            let companion = svc.create_companion(name, "ink").await.unwrap();
            svc.patch_companion(
                &companion.companion_id,
                serde_json::json!({"learn": { "enabled": true }, "evolve": { "enabled": true }}),
            )
            .await
            .unwrap();
            ids.push(companion.companion_id);
        }

        // Block exactly one profile. `save_bytes_atomic` creates its temp file inside
        // the companion's own directory, so a non-writable directory fails the save —
        // and `registry::patch` saves BEFORE inserting, so that companion keeps
        // `learn.enabled = true` in memory too.
        let blocked = ids[1].clone();
        let blocked_dir = dir
            .path()
            .join(crate::COMPANION_COMPANIONS_REL_DIR)
            .join(&blocked);
        let original = std::fs::metadata(&blocked_dir).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&blocked_dir, readonly).unwrap();

        // A root runner writes straight through the permission bits, so the failure
        // this test needs cannot be expressed there. Say so instead of asserting it.
        let probe = blocked_dir.join(".write-probe");
        if std::fs::File::create(&probe).is_ok() {
            let _ = std::fs::remove_file(&probe);
            std::fs::set_permissions(&blocked_dir, original).unwrap();
            eprintln!("skipped: this runner writes through a read-only directory (root?)");
            return;
        }

        let error = svc.disable_all().await.unwrap_err().to_string();
        std::fs::set_permissions(&blocked_dir, original).unwrap();

        // The half that landed, and the user must be told it landed.
        {
            let cfg = svc.config.read().await;
            assert!(!cfg.collect.tool_calls);
            assert!(!cfg.collect.companion_dialogues);
        }
        assert!(
            error.contains("collection is now off"),
            "the error must not read as 'nothing happened': {error}"
        );
        assert!(error.contains(&blocked), "the error must name the companion that failed: {error}");

        // Best effort: a mid-roster failure does not spare the companions after it.
        for id in [&ids[0], &ids[2]] {
            let profile = svc.get_companion(id).await.unwrap();
            assert!(!profile.learn.enabled, "the rest of the roster must still stop");
            assert!(!profile.evolve.enabled);
        }
        // And the blocked one is honestly still running — nothing was faked.
        let still_on = svc.get_companion(&blocked).await.unwrap();
        assert!(still_on.learn.enabled, "a failed save must not report success in memory");
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
        let sessions = attach_canonical_test_host(&svc);
        let mut canonical_ids = Vec::new();
        for (companion_id, key) in [(&a.companion_id, "a-chat"), (&a.companion_id, "a-associated"), (&b.companion_id, "b-chat")] {
            let request = serde_json::from_value::<nomifun_api_types::CreateConversationRequest>(serde_json::json!({
                "type": "nomi", "name": key, "extra": {},
            })).unwrap();
            let created = sessions.create(svc.authoritative_user_id.as_ref(), request, vec![], crate::CompanionSessionCreateContext {
                companion_id: companion_id.clone(), role: CompanionRole::Companion, robot_id: None,
                idempotency_key: key.into(), mcp_server_ids: None,
            }).await.unwrap();
            canonical_ids.push(created.conversation_id);
        }
        assert_eq!(svc.get_config().await.default_companion_id.as_deref(), Some(a.companion_id.as_str()));

        // Give A per-companion rows that must vanish with it.
        svc.store.add_companion_xp(&a.companion_id, 42).await.unwrap();
        let conversation = canonical_ids[0].clone();
        svc.store.insert_companion_thread(&conversation, &a.companion_id, "甲聊").await.unwrap();
        svc.store.add_companion_xp(&b.companion_id, 7).await.unwrap();

        svc.delete_companion(&a.companion_id).await.unwrap();

        assert!(matches!(svc.get_companion(&a.companion_id).await, Err(AppError::NotFound(_))));
        assert_eq!(svc.store.get_companion_state_i64(&a.companion_id, "xp").await.unwrap(), 0);
        assert!(!svc.store.is_companion_thread(&conversation).await.unwrap());
        for id in &canonical_ids[..2] {
            assert!(matches!(sessions.get(svc.authoritative_user_id.as_ref(), id).await, Err(AppError::NotFound(_))));
        }
        assert!(sessions.get(svc.authoritative_user_id.as_ref(), &canonical_ids[2]).await.is_ok(), "another Companion's canonical Session survives");
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
    async fn rename_then_new_session_keeps_stable_identity_and_delete_cleans_old_frozen_roots() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let profile = svc.create_companion("名字A", "ink").await.unwrap();
        let sessions = attach_canonical_test_host(&svc);
        let legacy_root = svc.companion_workspaces_root().join(format!("{}_名字A", profile.seq));
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::write(legacy_root.join("old.txt"), "existing current-generation work").unwrap();
        let legacy = sessions.create(svc.authoritative_user_id.as_ref(), serde_json::from_value(serde_json::json!({
            "type": "nomi", "name": "旧的正式伙伴会话", "extra": { "workspace": legacy_root.to_string_lossy() },
        })).unwrap(), vec![], crate::CompanionSessionCreateContext {
            companion_id: profile.companion_id.clone(), role: CompanionRole::Companion, robot_id: None,
            idempotency_key: "before-stable-directory-identity".into(), mcp_server_ids: None,
        }).await.unwrap();
        svc.store.insert_companion_thread(&legacy.conversation_id, &profile.companion_id, &legacy.name).await.unwrap();
        let component = legacy_root.file_name().unwrap().to_str().unwrap();
        assert_eq!(svc.register_companion_workspace_component(&profile.companion_id, component).await.unwrap(), component);
        assert_eq!(svc.register_companion_workspace_component(&profile.companion_id, "another-proposed-location").await.unwrap(), component,
            "registration is CAS-if-absent, never a path rewrite");
        assert!(svc.register_companion_workspace_component(&profile.companion_id, "../escape").await.is_err());

        let older_second_root = svc.companion_workspaces_root().join(format!("{}_名字B", profile.seq));
        std::fs::create_dir_all(&older_second_root).unwrap();
        std::fs::write(older_second_root.join("other-old.txt"), "another frozen current-generation root").unwrap();
        sessions.create(svc.authoritative_user_id.as_ref(), serde_json::from_value(serde_json::json!({
            "type": "nomi", "name": "另一旧会话", "extra": { "workspace": older_second_root.to_string_lossy() },
        })).unwrap(), vec![], crate::CompanionSessionCreateContext {
            companion_id: profile.companion_id.clone(), role: CompanionRole::Companion, robot_id: None,
            idempotency_key: "second-current-generation-frozen-root".into(), mcp_server_ids: None,
        }).await.unwrap();

        let stable = svc.companion_workspace_path(&profile.companion_id).await.unwrap();
        let provider = nomifun_common::ProviderId::new().into_string();
        svc.patch_companion(&profile.companion_id, serde_json::json!({
            "name": "名字B", "model": {"provider_id": provider, "model": "fixture"},
        })).await.unwrap();
        assert_eq!(svc.companion_workspace_path(&profile.companion_id).await.unwrap(), stable);
        std::fs::create_dir_all(&svc.skill_paths.builtin_skills_dir).unwrap();
        let new_thread = svc.new_companion_thread(&profile.companion_id, "after-rename", None).await.unwrap();
        assert_eq!(sessions.get(svc.authoritative_user_id.as_ref(), &new_thread.conversation_id).await.unwrap().extra["workspace"], stable.to_string_lossy().as_ref());
        assert_eq!(sessions.get(svc.authoritative_user_id.as_ref(), &legacy.conversation_id).await.unwrap().extra["workspace"], legacy_root.to_string_lossy().as_ref(), "an existing frozen Session is not moved");
        std::fs::write(stable.join("new.txt"), "new shared work").unwrap();
        svc.delete_companion(&profile.companion_id).await.unwrap();
        assert!(!legacy_root.exists(), "canonical provenance includes the older named root");
        assert!(!older_second_root.exists(), "all authenticated frozen roots are settled and cleaned");
        assert!(svc.store.get_companion_state(&profile.companion_id, crate::companion::WORKSPACE_COMPONENT_KEY).await.unwrap().is_none());
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
            Arc::new(nomifun_skill_library::skill_service::resolve_skill_paths(
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
    async fn mood_override_survives_restart_and_clearing_restores_latest_automatic_mood() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.create_companion("甲", "ink").await.unwrap();
        let b = svc.create_companion("乙", "ink").await.unwrap();
        svc.store.set_companion_state(&a.companion_id, crate::store::MOOD_KEY, "sleepy").await.unwrap();
        let patched = svc.patch_companion(&a.companion_id, serde_json::json!({"mood_override": "happy"})).await.unwrap();
        assert_eq!(patched.mood_override, Some(crate::profile::CompanionMood::Happy));
        let status = svc.companion_status(&a.companion_id).await.unwrap();
        assert_eq!(status.mood, "happy");
        assert_eq!(status.automatic_mood, "sleepy");
        assert_eq!(svc.companion_status(&b.companion_id).await.unwrap().mood, "content");

        svc.close_storage().await;
        drop(svc);
        let svc = service(dir.path()).await;
        assert_eq!(svc.get_companion(&a.companion_id).await.unwrap().mood_override, Some(crate::profile::CompanionMood::Happy));
        assert_eq!(svc.companion_status(&a.companion_id).await.unwrap().mood, "happy");
        // Learning still owns and updates its original runtime row while the
        // manual preference keeps the displayed emotion stable.
        svc.store.set_companion_state(&a.companion_id, crate::store::MOOD_KEY, "worried").await.unwrap();
        let status = svc.companion_status(&a.companion_id).await.unwrap();
        assert_eq!(status.mood, "happy");
        assert_eq!(status.automatic_mood, "worried");

        let cleared = svc.patch_companion(&a.companion_id, serde_json::json!({"mood_override": null})).await.unwrap();
        assert_eq!(cleared.mood_override, None);
        let status = svc.companion_status(&a.companion_id).await.unwrap();
        assert_eq!(status.mood, "worried");
        assert_eq!(status.automatic_mood, "worried");
        assert_eq!(svc.store.get_companion_state(&a.companion_id, crate::store::MOOD_KEY).await.unwrap().as_deref(), Some("worried"));
    }

    #[tokio::test]
    async fn invalid_mood_override_patch_preserves_the_entire_profile() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.create_companion("甲", "ink").await.unwrap();
        let profile_dir = dir.path().join(crate::COMPANION_COMPANIONS_REL_DIR).join(&a.companion_id);
        let before = std::fs::read(CompanionProfileConfig::config_path(&profile_dir)).unwrap();
        for invalid in [serde_json::json!("auto"), serde_json::json!("angry"), serde_json::json!(1), serde_json::json!({})] {
            let result = svc.patch_companion(&a.companion_id, serde_json::json!({"name": "不应保存", "mood_override": invalid})).await;
            assert!(matches!(result, Err(AppError::BadRequest(_))));
            assert_eq!(svc.get_companion(&a.companion_id).await.unwrap(), a);
            assert_eq!(std::fs::read(CompanionProfileConfig::config_path(&profile_dir)).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn mood_override_patch_fails_before_saving_when_runtime_store_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path()).await;
        let a = svc.create_companion("甲", "ink").await.unwrap();
        svc.close_storage().await;
        assert!(svc.patch_companion(&a.companion_id, serde_json::json!({"mood_override": "happy"})).await.is_err());
        assert_eq!(svc.get_companion(&a.companion_id).await.unwrap(), a);
        let profile_dir = dir.path().join(crate::COMPANION_COMPANIONS_REL_DIR).join(&a.companion_id);
        assert_eq!(CompanionProfileConfig::load(&profile_dir).unwrap().unwrap(), a);
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
        assert_eq!(first.companion_id.as_deref(), Some(a.as_str()));
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
        assert_eq!(bs.companion_id.as_deref(), Some(b.as_str()));
        assert_eq!(svc.store.count_memories("active", Some(&b)).await.unwrap(), 1);

        // Omitting the companion resolves the owner (oldest = 甲) rather than
        // writing a shared row.
        let resolved = svc.add_memory("task", "帮主人订咖啡豆", &[], None).await.unwrap();
        assert_eq!(resolved.companion_id.as_deref(), Some(a.as_str()));

        // Validation untouched, and an unknown owner is rejected before any write
        // (an orphaned reference would hard-fail the next boot).
        assert!(svc.add_memory("bogus", "x", &[], Some(&a)).await.is_err());
        assert!(svc.add_memory("task", "   ", &[], Some(&a)).await.is_err());
        assert!(svc.add_memory("task", "无主人", &[], Some(MALFORMED_COMPANION_ID)).await.is_err());
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
