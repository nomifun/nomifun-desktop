//! The scheduled learning loop: every tick, for every companion whose 定时学习 is
//! enabled and due (and which is not in its 休眠时段), read the events IT has not
//! consumed yet, run one LLM distillation call, and apply the output to that
//! companion (memories / reinforcement / supersedes / mood / diary / XP).
//!
//! One schedule, one model and one cursor served the whole roster until 2026-08.
//! Everything the loop reads now comes off [`CompanionProfileConfig::learn`], and
//! everything it writes is owned by the companion whose config drove the run —
//! there is no "default companion collects for everyone" step left.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use nomifun_ai_agent::nomi_config;
use nomifun_ai_agent::{one_shot_completion, resolve_provider_config, user_message};
use nomifun_common::{AppError, now_ms};
use nomifun_model_invoke::ModelInvokeService;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::collector::{LEARN_CURSOR_KEY, SharedEventStoreLock, read_events_since};
use crate::events::CompanionEventEmitter;
use crate::prompt::{self, LEARN_MAX_TOKENS};
use crate::registry::CompanionRegistry;
use crate::store::{CompanionStore, MOOD_KEY, MemoryFilter};

const MAX_EVENTS_PER_RUN: usize = 300;
const TICK_SECONDS: u64 = 60;
/// After this many consecutive scheduled runs fail to parse, the batch is
/// abandoned (cursor advanced) instead of re-burning tokens forever.
const PARSE_FAIL_GIVE_UP_RUNS: i64 = 3;
/// Per-companion `companion_runtime_state` keys owned by this loop.
const LAST_LEARN_TS_KEY: &str = "last_learn_ts";
const PARSE_FAIL_STREAK_KEY: &str = "learn_parse_fail_streak";

/// Re-entrancy guards, one per companion.
///
/// A single process-wide `Mutex` would make "run now" on companion A return
/// `Conflict` just because B's scheduled tick happened to be mid-flight, and each
/// run holds an LLM call — the one thing a per-companion feature must not
/// serialize. The map only ever grows by roster size, and each entry is a bare
/// `Mutex<()>`.
#[derive(Default)]
pub struct CompanionRunLocks(Mutex<HashMap<String, Arc<Mutex<()>>>>);

impl CompanionRunLocks {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn for_companion(&self, companion_id: &str) -> Arc<Mutex<()>> {
        self.0
            .lock()
            .await
            .entry(companion_id.to_owned())
            .or_default()
            .clone()
    }
}

/// Ephemeral outcome of one learning pass. It is returned to an explicit
/// caller and broadcast to live companion surfaces, but is deliberately not
/// persisted as run history.
#[derive(Debug, Clone, Serialize)]
pub struct CompanionLearnResult {
    pub status: String,
    pub events_processed: i64,
    pub memories_added: i64,
    pub error: Option<String>,
    /// Nomi's one-line diary for this pass, used by the live companion bubble.
    pub summary: Option<String>,
}

/// LLM seam so tests can run the learner without a live provider.
/// (Companion chat runs on the real agent engine; this trait only serves
/// the scheduled learning distillation calls.)
#[async_trait::async_trait]
pub trait CompanionCompleter: Send + Sync {
    async fn complete(&self, provider_id: &str, model: &str, system: &str, user: &str, max_tokens: u32)
    -> Result<String, AppError>;
}

/// Production completer: provider row → nomi Config → one-shot completion.
pub struct LiveCompanionCompleter {
    pub model_invoke: Arc<ModelInvokeService>,
    pub workspace: PathBuf,
}

impl LiveCompanionCompleter {
    async fn resolve(&self, provider_id: &str, model: &str) -> Result<nomi_config::config::Config, AppError> {
        resolve_provider_config(
            self.model_invoke.as_ref(),
            provider_id,
            model,
            &self.workspace,
        )
        .await
    }
}

#[async_trait::async_trait]
impl CompanionCompleter for LiveCompanionCompleter {
    async fn complete(
        &self,
        provider_id: &str,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<String, AppError> {
        let cfg = self.resolve(provider_id, model).await?;
        one_shot_completion(&cfg, system, vec![user_message(user)], max_tokens).await
    }
}

pub struct Learner {
    pub companion_dir: PathBuf,
    pub store: CompanionStore,
    /// The roster IS the schedule: each companion's own `learn` block decides
    /// whether, how often and with which model it distills.
    pub registry: Arc<CompanionRegistry>,
    pub completer: Arc<dyn CompanionCompleter>,
    pub emitter: CompanionEventEmitter,
    pub event_store_lock: SharedEventStoreLock,
    /// Re-entrancy guards shared between the tick loop and "run now".
    pub run_locks: Arc<CompanionRunLocks>,
}

impl Learner {
    /// Spawn the periodic tick loop. Companions are visited sequentially so an
    /// N-companion roster cannot fire N concurrent LLM calls on one tick.
    pub fn spawn(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(TICK_SECONDS));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                for profile in self.registry.list().await {
                    if !profile.learn.enabled {
                        continue;
                    }
                    // 休眠时段 means "leave me alone", not just "don't pop a
                    // bubble": a background LLM run costs the owner money and
                    // writes memories, so it waits for the window to pass. (IM
                    // auto-replies are deliberately NOT gated — silently not
                    // answering a message would be a surprise.)
                    if profile.appearance.in_quiet_hours_now() {
                        continue;
                    }
                    let last_run = self
                        .store
                        .get_companion_state_i64(&profile.companion_id, LAST_LEARN_TS_KEY)
                        .await
                        .unwrap_or(0);
                    let interval_minutes = profile.learn.effective_interval_minutes() as i64;
                    if now_ms() - last_run < interval_minutes * 60_000 {
                        continue;
                    }
                    if let Err(e) = self.run_for(&profile.companion_id).await {
                        tracing::warn!(
                            companion_id = %profile.companion_id,
                            error = %e,
                            "companion scheduled learn run failed"
                        );
                    }
                }
            }
        });
    }

    /// Execute one learning pass for `companion_id` and return its transient
    /// result. An unknown companion is a `NotFound` before anything is written or
    /// any token spent — the roster can change under a queued tick.
    pub async fn run_for(&self, companion_id: &str) -> Result<CompanionLearnResult, AppError> {
        let profile = self
            .registry
            .get(companion_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("companion '{companion_id}' not found")))?;
        let owner = profile.companion_id.clone();
        let lock = self.run_locks.for_companion(&owner).await;
        let Ok(_guard) = lock.try_lock() else {
            return Err(AppError::Conflict(
                "a learn run is already in progress for this companion".into(),
            ));
        };
        let started_at = now_ms();
        // Stamp first so a crashed/failed run doesn't hot-loop the scheduler.
        self.store
            .set_companion_state(&owner, LAST_LEARN_TS_KEY, &started_at.to_string())
            .await?;

        let mut run = CompanionLearnResult {
            status: "ok".into(),
            events_processed: 0,
            memories_added: 0,
            error: None,
            summary: None,
        };

        let Some(model) = profile.learn.model.clone() else {
            run.status = "model_unconfigured".into();
            return Ok(run);
        };

        let cursor = self
            .store
            .get_companion_state_i64(&owner, LEARN_CURSOR_KEY)
            .await?;
        let (events, truncated) = {
            let _event_guard = self.event_store_lock.read().await;
            read_events_since(&self.companion_dir, cursor, MAX_EVENTS_PER_RUN)?
        };
        if events.is_empty() {
            run.status = "no_events".into();
            return Ok(run);
        }
        // Taken from the UNFILTERED batch: the cursor must advance past every
        // record that was read, including the ones dropped just below.
        let new_cursor = events.last().map(|e| e.ts).unwrap_or(cursor);

        // Historical `companion.reply` records. Replies are no longer collected,
        // but day-files written before that change still hold them until retention
        // ages them out, and they must never reach the prompt: LEARN_SYSTEM no
        // longer carries the rule that forbade reading a reply as owner intent, so
        // a leftover one would arrive with nothing left to stop the model treating
        // the companion's own words as the owner's facts or wishes.
        let events: Vec<_> = events
            .into_iter()
            .filter(|event| event.name != "companion.reply")
            .collect();
        if events.is_empty() {
            // Nothing but stale replies. Advance anyway — leaving the cursor here
            // would re-read the same records every run and stall learning until
            // retention finally deletes them.
            self.store
                .set_companion_state(&owner, LEARN_CURSOR_KEY, &new_cursor.to_string())
                .await?;
            run.status = "no_events".into();
            return Ok(run);
        }
        run.events_processed = events.len() as i64;

        // 选项A：学习产出只由「记忆主人」窗口呈现。主人就是本轮配置的来源 ——
        // 记忆写给谁、气泡/心情落在谁头上、XP 记在谁账上，全是同一个伙伴，
        // 三者不允许分叉。
        self.emitter.emit_learn_started(&owner);

        // Existing-memory digest for reinforcement/conflict matching — only the
        // owner's own memories: the learner must never reinforce or supersede
        // another companion's row.
        let existing = self
            .store
            .list_memories(&MemoryFilter {
                status: Some("active".into()),
                companion_id: Some(owner.clone()),
                limit: 120,
                ..Default::default()
            })
            .await?;
        let event_lines: Vec<String> = events
            .iter()
            .map(|event| {
                serde_json::to_string(event).map_err(|error| {
                    AppError::Internal(format!("serialize collected event: {error}"))
                })
            })
            .collect::<Result<_, _>>()?;
        let user_prompt = prompt::build_learn_prompt(&existing, &event_lines, truncated);

        // One retry on parse failure (the model occasionally wraps in prose).
        let mut parsed = None;
        let mut last_err = String::new();
        let mut provider_failed = false;
        for attempt in 0..2 {
            match self
                .completer
                .complete(&model.provider_id, &model.model, prompt::LEARN_SYSTEM, &user_prompt, LEARN_MAX_TOKENS)
                .await
            {
                Ok(raw) => match prompt::parse_learn_output(&raw) {
                    Ok(out) => {
                        parsed = Some(out);
                        break;
                    }
                    Err(e) => {
                        last_err = e;
                        tracing::debug!(attempt, error = %last_err, "companion learn output unparseable");
                    }
                },
                Err(e) => {
                    last_err = e.to_string();
                    provider_failed = true;
                    break; // provider failure: don't burn a retry
                }
            }
        }

        let Some(output) = parsed else {
            run.status = "error".into();
            run.error = Some(last_err);
            // Provider failure is transient: keep the cursor so the same
            // events retry once the provider recovers. Parse failure is the
            // model misformatting — retry the batch a few scheduled runs,
            // then advance past it so a consistently-confused model can't
            // re-burn tokens on the same batch forever.
            if !provider_failed {
                let streak = self
                    .store
                    .get_companion_state_i64(&owner, PARSE_FAIL_STREAK_KEY)
                    .await
                    .unwrap_or(0)
                    + 1;
                if streak >= PARSE_FAIL_GIVE_UP_RUNS {
                    self.store
                        .set_companion_state(&owner, LEARN_CURSOR_KEY, &new_cursor.to_string())
                        .await?;
                    self.store
                        .set_companion_state(&owner, PARSE_FAIL_STREAK_KEY, "0")
                        .await?;
                    tracing::warn!(companion_id = %owner, events = run.events_processed, "companion learn batch abandoned after repeated parse failures");
                } else {
                    self.store
                        .set_companion_state(&owner, PARSE_FAIL_STREAK_KEY, &streak.to_string())
                        .await?;
                }
            }
            self.emitter.emit_learn_finished(&owner, &run);
            return Ok(run);
        };
        let _ = self
            .store
            .set_companion_state(&owner, PARSE_FAIL_STREAK_KEY, "0")
            .await;

        // Apply: decay first, then reinforce/supersede/insert.
        let _ = self.store.decay_memories().await;
        // The two id lists come back from the MODEL, and `reinforce_memories` /
        // `archive_memories` address rows by id alone — so they are filtered to
        // the ids this run actually showed it (the owner's own active memories).
        // 共享记忆删除后这道过滤是必需的：事件流里可能出现别的伙伴的 memory_id
        // （例如 owner agent 调用过 nomi_memory_list 这个跨伙伴视图，其输出被采集
        // 成了事件），模型把它抄进 supersede_memory_ids 就会静默归档别人的记忆。
        let owned_ids: std::collections::HashSet<&str> =
            existing.iter().map(|m| m.memory_id.as_str()).collect();
        let keep_owned = |ids: &[String], what: &str| -> Vec<String> {
            let (mine, foreign): (Vec<String>, Vec<String>) =
                ids.iter().cloned().partition(|id| owned_ids.contains(id.as_str()));
            if !foreign.is_empty() {
                tracing::warn!(
                    companion_id = %owner,
                    dropped = foreign.len(),
                    "companion learn output referenced {what} outside this companion's memories; ignored"
                );
            }
            mine
        };
        self.store
            .reinforce_memories(&keep_owned(&output.reinforce_memory_ids, "reinforce_memory_ids"))
            .await?;
        self.store
            .archive_memories(&keep_owned(&output.supersede_memory_ids, "supersede_memory_ids"))
            .await?;

        // 每条蒸馏记忆都落在 owner 名下（共享记忆概念已删除）。
        for m in &output.memories {
            if self
                .store
                .find_similar_active(&m.kind, &m.content, &owner)
                .await?
                .is_some()
            {
                continue;
            }
            self.store
                .insert_memory_scoped(
                    &m.kind,
                    &m.content,
                    &m.tags,
                    m.importance,
                    "learn",
                    Some(&owner),
                )
                .await?;
            run.memories_added += 1;
        }
        // Mood belongs to the companion that just learned. It was a single global
        // `companion_state` row until 2026-08, which made the whole family share
        // one mood — the last run to finish overwrote everyone's.
        if let Some(mood) = &output.mood {
            self.store.set_companion_state(&owner, MOOD_KEY, mood).await?;
            self.emitter.emit_mood_changed(&owner, mood);
        }
        run.summary = output.diary;

        // XP: 1 per event + 5 per new memory, credited to the companion that did
        // the learning. It used to be granted to EVERY companion on the theory
        // that one shared loop was a family achievement; with a loop per companion
        // that would hand every sibling XP for work they did not do.
        let _ = self
            .store
            .add_companion_xp(&owner, run.events_processed + run.memories_added * 5)
            .await;

        self.store
            .set_companion_state(&owner, LEARN_CURSOR_KEY, &new_cursor.to_string())
            .await?;
        self.emitter.emit_learn_finished(&owner, &run);
        Ok(run)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{CollectedEvent, EVOLVE_CURSOR_KEY, append_event};
    use crate::profile::CompanionLearnConfig;
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::BroadcastEventBus;
    use tokio::sync::RwLock;

    struct CannedCompleter(String);

    #[async_trait::async_trait]
    impl CompanionCompleter for CannedCompleter {
        async fn complete(&self, _p: &str, _m: &str, _s: &str, _u: &str, _t: u32) -> Result<String, AppError> {
            Ok(self.0.clone())
        }
    }

    /// Records every user prompt it is handed, so a test can assert what did —
    /// and did not — reach the model.
    struct RecordingCompleter {
        reply: String,
        prompts: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl CompanionCompleter for RecordingCompleter {
        async fn complete(&self, _p: &str, _m: &str, _s: &str, user: &str, _t: u32) -> Result<String, AppError> {
            self.prompts.lock().unwrap().push(user.to_owned());
            Ok(self.reply.clone())
        }
    }

    fn test_learn_config() -> CompanionLearnConfig {
        CompanionLearnConfig {
            enabled: true,
            interval_minutes: 60,
            model: Some(nomifun_common::ProviderWithModel {
                provider_id: nomifun_common::ProviderId::new().into_string(),
                model: "test-model".into(),
                use_model: None,
            }),
        }
    }

    /// Learner over a temp dir with one registered companion whose 定时学习 is
    /// configured. Returns the learner + that companion's id.
    async fn make_learner(dir: &std::path::Path, reply: &str) -> (Learner, String) {
        let registry = Arc::new(
            CompanionRegistry::scan(dir.join("companions"), dir.join("shared"))
                .unwrap(),
        );
        let companion = registry.create("测试宠", "ink").await.unwrap();
        registry
            .patch(
                &companion.companion_id,
                serde_json::json!({"learn": serde_json::to_value(test_learn_config()).unwrap()}),
            )
            .await
            .unwrap();
        let learner = Learner {
            companion_dir: dir.to_path_buf(),
            store: CompanionStore::open_memory().await.unwrap(),
            registry,
            completer: Arc::new(CannedCompleter(reply.to_owned())),
            emitter: CompanionEventEmitter::new(Arc::new(BroadcastEventBus::new(16)), "owner-a"),
            event_store_lock: Arc::new(RwLock::new(())),
            run_locks: Arc::new(CompanionRunLocks::new()),
        };
        (learner, companion.companion_id)
    }

    fn seed_event(dir: &std::path::Path) {
        append_event(
            dir,
            &CollectedEvent {
                event_id: nomifun_common::generate_id(),
                ts: now_ms(),
                source: "chat_user_messages".into(),
                name: "message.userCreated".into(),
                data: serde_json::json!({"content": "帮我看看 Rust 编译错误"}),
            },
        )
        .unwrap();
    }

    /// A `companion.reply` record of the kind day-files written before replies
    /// stopped being collected still hold. Returns its timestamp so a test can
    /// assert exactly where the cursor landed.
    fn seed_stale_reply(dir: &std::path::Path, content: &str) -> i64 {
        let ts = now_ms();
        append_event(
            dir,
            &CollectedEvent {
                event_id: nomifun_common::generate_id(),
                ts,
                source: "companion_dialogues".into(),
                name: "companion.reply".into(),
                data: serde_json::json!({"content": content}),
            },
        )
        .unwrap();
        ts
    }

    #[tokio::test]
    async fn stale_replies_never_reach_the_prompt() {
        // LEARN_SYSTEM no longer carries the rule that forbade reading a reply as
        // owner intent, so a leftover reply reaching the model would arrive with
        // nothing to stop it being distilled as the owner's own wish.
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        seed_stale_reply(dir.path(), "我帮你把配置改成深色主题了");
        let reply = r#"{"memories":[{"kind":"profile","content":"主人是 Rust 工程师","importance":0.9}],
            "mood":"content","diary":"今天陪主人修了 bug～"}"#;
        let (learner, companion_id) = make_learner(dir.path(), reply).await;
        let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let learner = Learner {
            completer: Arc::new(RecordingCompleter {
                reply: reply.to_owned(),
                prompts: prompts.clone(),
            }),
            ..learner
        };

        let run = learner.run_for(&companion_id).await.unwrap();
        assert_eq!(run.status, "ok");
        // Only the owner's own message counts as processed.
        assert_eq!(run.events_processed, 1);

        let sent = prompts.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("帮我看看 Rust 编译错误"));
        assert!(
            !sent[0].contains("我帮你把配置改成深色主题了"),
            "a stale companion.reply must not reach the prompt"
        );
        assert!(!sent[0].contains("companion.reply"));
    }

    #[tokio::test]
    async fn a_batch_of_only_stale_replies_advances_the_cursor() {
        // Otherwise the same records are re-read on every scheduled run and
        // learning stalls until retention finally deletes them.
        let dir = tempfile::tempdir().unwrap();
        seed_stale_reply(dir.path(), "好的呀");
        let last_ts = seed_stale_reply(dir.path(), "已经记下了");
        let (learner, companion_id) = make_learner(dir.path(), "{}").await;
        let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let learner = Learner {
            completer: Arc::new(RecordingCompleter {
                reply: "{}".to_owned(),
                prompts: prompts.clone(),
            }),
            ..learner
        };

        let run = learner.run_for(&companion_id).await.unwrap();
        assert_eq!(run.status, "no_events");
        assert_eq!(run.memories_added, 0);
        assert!(prompts.lock().unwrap().is_empty(), "the model must not be called at all");
        // Exactly the batch's last timestamp, not merely "nonzero": a cursor that
        // advanced to the wrong place would re-read or skip real events.
        assert_eq!(
            learner.store.get_companion_state_i64(&companion_id, LEARN_CURSOR_KEY).await.unwrap(),
            last_ts,
            "the cursor must advance to the end of a batch of stale replies"
        );
    }

    #[tokio::test]
    async fn run_for_applies_learn_output_to_that_companion() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let reply = r#"{"memories":[{"kind":"profile","content":"主人是 Rust 工程师","importance":0.9}],
            "mood":"content","diary":"今天陪主人修了 bug～"}"#;
        let (learner, companion_id) = make_learner(dir.path(), reply).await;
        let run = learner.run_for(&companion_id).await.unwrap();
        assert_eq!(run.status, "ok");
        assert_eq!(run.events_processed, 1);
        assert_eq!(run.memories_added, 1);
        // Mood and cursor are this companion's own rows, not global ones.
        assert_eq!(
            learner.store.get_companion_state(&companion_id, MOOD_KEY).await.unwrap().as_deref(),
            Some("content")
        );
        assert_eq!(learner.store.get_state(MOOD_KEY).await.unwrap(), None);
        assert!(
            learner.store.get_companion_state_i64(&companion_id, LEARN_CURSOR_KEY).await.unwrap() > 0
        );
        assert_eq!(learner.store.get_state_i64(LEARN_CURSOR_KEY).await.unwrap(), 0);
        // XP goes to the companion that learned (1 event + 1*5).
        assert_eq!(learner.store.get_companion_state_i64(&companion_id, "xp").await.unwrap(), 6);
        // Cursor advanced: a second run sees no events.
        let run2 = learner.run_for(&companion_id).await.unwrap();
        assert_eq!(run2.status, "no_events");
    }

    /// XP used to be granted to EVERY companion on the theory that one shared
    /// learn loop was a family achievement. With a loop per companion that would
    /// credit siblings for work they never did — and nothing else guards it.
    #[tokio::test]
    async fn learning_credits_only_the_companion_that_learned() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let reply = r#"{"memories":[{"kind":"profile","content":"主人写 Rust","importance":0.9}],
            "mood":"proud","diary":"d"}"#;
        let (learner, learned) = make_learner(dir.path(), reply).await;
        let idle = learner.registry.create("旁观者", "ink").await.unwrap().companion_id;

        assert_eq!(learner.run_for(&learned).await.unwrap().status, "ok");
        assert_eq!(learner.store.get_companion_state_i64(&learned, "xp").await.unwrap(), 6);
        assert_eq!(
            learner.store.get_companion_state_i64(&idle, "xp").await.unwrap(),
            0,
            "a companion that did not run must not be credited"
        );
        assert_eq!(
            learner.store.get_companion_state(&idle, MOOD_KEY).await.unwrap(),
            None,
            "one companion's mood must not become everyone's"
        );
        // The idle companion's cursor is untouched, so the same events are still
        // waiting for it if its own loop is ever enabled.
        assert_eq!(
            learner.store.get_companion_state_i64(&idle, LEARN_CURSOR_KEY).await.unwrap(),
            0
        );
    }

    /// 记忆有主人之后，模型回传的 id 不能再无条件生效：`reinforce_memories` /
    /// `archive_memories` 只按 memory_id 定位行，所以一个从事件流里抄来的、属于
    /// **别的伙伴**的 id 会静默改掉别人的记忆。这里把两条列表都钉住。
    #[tokio::test]
    async fn run_for_ignores_memory_ids_outside_this_companions_own_rows() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let (learner, owner) = make_learner(dir.path(), "{}").await;
        let stranger = learner.registry.create("别的伙伴", "ink").await.unwrap().companion_id;

        // 一条属于 stranger 的活跃记忆 + 一条属于 owner 的活跃记忆。
        let theirs = learner
            .store
            .insert_memory_scoped("profile", "别的伙伴的画像", &[], 0.9, "chat", Some(&stranger))
            .await
            .unwrap();
        let mine = learner
            .store
            .insert_memory_scoped("profile", "我记得主人写 Rust", &[], 0.9, "chat", Some(&owner))
            .await
            .unwrap();

        // 模型把两条 id 都塞进 supersede/reinforce：只有自己的那条可以生效。
        let reply = format!(
            r#"{{"memories":[],"supersede_memory_ids":["{}"],"reinforce_memory_ids":["{}"],"mood":"content","diary":"d"}}"#,
            theirs.memory_id, theirs.memory_id
        );
        let learner = Learner { completer: Arc::new(CannedCompleter(reply)), ..learner };
        assert_eq!(learner.run_for(&owner).await.unwrap().status, "ok");
        let untouched = learner.store.get_memory(&theirs.memory_id).await.unwrap().unwrap();
        assert_eq!(untouched.status, "active", "another companion's memory must not be archived");
        assert_eq!(
            untouched.strength, theirs.strength,
            "another companion's memory must not be reinforced"
        );

        // 自己的那条照常生效（过滤没有把功能一起关掉）。
        let reply = format!(
            r#"{{"memories":[],"supersede_memory_ids":["{}"],"mood":"content","diary":"d"}}"#,
            mine.memory_id
        );
        let learner = Learner { completer: Arc::new(CannedCompleter(reply)), ..learner };
        learner.store.set_companion_state(&owner, LEARN_CURSOR_KEY, "0").await.unwrap();
        assert_eq!(learner.run_for(&owner).await.unwrap().status, "ok");
        assert_eq!(
            learner.store.get_memory(&mine.memory_id).await.unwrap().unwrap().status,
            "archived",
            "the companion's own memory is still supersedable"
        );
    }

    #[tokio::test]
    async fn run_for_records_error_on_garbage_output() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let (learner, id) = make_learner(dir.path(), "我不会输出 JSON").await;
        let run = learner.run_for(&id).await.unwrap();
        assert_eq!(run.status, "error");
        assert!(run.error.is_some());
    }

    #[tokio::test]
    async fn run_for_skips_when_model_unconfigured() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let (learner, id) = make_learner(dir.path(), "{}").await;
        learner
            .registry
            .patch(&id, serde_json::json!({"learn": {"model": null}}))
            .await
            .unwrap();
        let run = learner.run_for(&id).await.unwrap();
        assert_eq!(run.status, "model_unconfigured");
    }

    /// A companion can be deleted while a tick is already queued for it. The run
    /// must then write nothing, burn no token, and above all not advance any
    /// cursor — the events belong to whoever is still enabled.
    #[tokio::test]
    async fn run_for_an_unknown_companion_writes_nothing_and_keeps_the_cursor() {
        struct ExplodingCompleter;
        #[async_trait::async_trait]
        impl CompanionCompleter for ExplodingCompleter {
            async fn complete(&self, _: &str, _: &str, _: &str, _: &str, _: u32) -> Result<String, AppError> {
                panic!("the learner must not call the model for a companion that is gone");
            }
        }

        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let (learner, only) = make_learner(dir.path(), "{}").await;
        learner.registry.remove(&only).await.unwrap();
        let learner = Learner { completer: Arc::new(ExplodingCompleter), ..learner };

        assert!(matches!(
            learner.run_for(&only).await,
            Err(AppError::NotFound(_))
        ));
        assert_eq!(learner.store.count_memories("active", None).await.unwrap(), 0);
        assert_eq!(
            learner.store.get_companion_state_i64(&only, LEARN_CURSOR_KEY).await.unwrap(),
            0
        );

        // 建好新伙伴之后，同一批事件确实还会被学习（游标是每伙伴的，没被烧掉）。
        let reply = r#"{"memories":[{"kind":"profile","content":"主人在写 Rust","importance":0.9}],
            "mood":"content","diary":"补上了"}"#;
        let learner = Learner { completer: Arc::new(CannedCompleter(reply.to_owned())), ..learner };
        let owner = learner.registry.create("迟到的伙伴", "ink").await.unwrap().companion_id;
        learner
            .registry
            .patch(
                &owner,
                serde_json::json!({"learn": serde_json::to_value(test_learn_config()).unwrap()}),
            )
            .await
            .unwrap();
        let run = learner.run_for(&owner).await.unwrap();
        assert_eq!(run.status, "ok");
        assert_eq!(run.memories_added, 1);
        let mine = learner
            .store
            .list_memories(&MemoryFilter {
                companion_id: Some(owner.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].companion_id.as_deref(), Some(owner.as_str()));
    }

    /// 休眠时段 gates the scheduled tick, not `run_for` — an explicit "run now"
    /// from the UI is the owner asking, and must still work. The gate itself lives
    /// on the profile so both loops and the renderer share one definition.
    #[tokio::test]
    async fn quiet_hours_gate_the_schedule_but_not_an_explicit_run() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let (learner, id) = make_learner(dir.path(), r#"{"memories":[],"mood":"calm","diary":"d"}"#).await;
        // A window covering the entire day: whatever the wall clock says, this
        // companion is asleep.
        learner
            .registry
            .patch(&id, serde_json::json!({"appearance": {"quiet_start": "00:00", "quiet_end": "23:59"}}))
            .await
            .unwrap();
        let profile = learner.registry.get(&id).await.unwrap();
        assert!(
            profile.appearance.in_quiet_hours_now()
                || chrono::Local::now().format("%H:%M").to_string() == "23:59",
            "an all-day window must read as quiet"
        );
        // The explicit path still runs.
        assert_eq!(learner.run_for(&id).await.unwrap().status, "ok");
    }

    /// Two companions, each with its own cursor over the SAME event spool: the
    /// second one must still see events the first has already consumed.
    #[tokio::test]
    async fn each_companion_reads_the_spool_from_its_own_cursor() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let reply = r#"{"memories":[{"kind":"profile","content":"主人写 Rust","importance":0.9}],"mood":"ok","diary":"d"}"#;
        let (learner, first) = make_learner(dir.path(), reply).await;
        let second = learner.registry.create("第二只", "ink").await.unwrap().companion_id;
        learner
            .registry
            .patch(
                &second,
                serde_json::json!({"learn": serde_json::to_value(test_learn_config()).unwrap()}),
            )
            .await
            .unwrap();

        assert_eq!(learner.run_for(&first).await.unwrap().events_processed, 1);
        assert_eq!(
            learner.run_for(&second).await.unwrap().events_processed,
            1,
            "the second companion's cursor is its own; the first consuming the batch must not hide it"
        );
        assert_eq!(learner.store.count_memories("active", Some(&first)).await.unwrap(), 1);
        assert_eq!(learner.store.count_memories("active", Some(&second)).await.unwrap(), 1);
    }

    /// The boot migration that gives every companion the old install-wide
    /// cursors. Seeding to 0 instead would make every companion re-distill the
    /// whole retained history on first launch: duplicate memories, unexpected bill.
    #[tokio::test]
    async fn per_companion_cursors_are_seeded_from_the_retired_global_ones() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let (learner, id) = make_learner(dir.path(), "{}").await;
        let store = &learner.store;
        store.set_state(LEARN_CURSOR_KEY, "900").await.unwrap();
        store.set_state(EVOLVE_CURSOR_KEY, "400").await.unwrap();
        store.set_state(MOOD_KEY, "happy").await.unwrap();
        let other = learner.registry.create("第二只", "ink").await.unwrap().companion_id;

        let ids = vec![id.clone(), other.clone()];
        assert!(store.seed_companion_state_from_global(&ids).await.unwrap() > 0);
        for who in &ids {
            assert_eq!(store.get_companion_state_i64(who, LEARN_CURSOR_KEY).await.unwrap(), 900);
            assert_eq!(store.get_companion_state_i64(who, EVOLVE_CURSOR_KEY).await.unwrap(), 400);
            assert_eq!(store.get_companion_state(who, MOOD_KEY).await.unwrap().as_deref(), Some("happy"));
        }

        // Idempotent: a companion that has moved on keeps ITS value.
        store.set_companion_state(&id, LEARN_CURSOR_KEY, "1500").await.unwrap();
        store.seed_companion_state_from_global(&ids).await.unwrap();
        assert_eq!(store.get_companion_state_i64(&id, LEARN_CURSOR_KEY).await.unwrap(), 1500);
        assert_eq!(store.get_companion_state_i64(&other, LEARN_CURSOR_KEY).await.unwrap(), 900);
    }

    #[derive(Default)]
    struct RecordingBroadcaster {
        events: std::sync::Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
    }
    impl nomifun_realtime::UserEventSink for RecordingBroadcaster {
        fn send_to_user(&self, _user_id: &str, e: WebSocketMessage<serde_json::Value>) {
            self.events.lock().unwrap().push(e);
        }
    }

    /// Every live event a run emits is scoped to the companion that ran — never to
    /// a separately resolved "default" companion, which is what used to make one
    /// window report another's learning.
    #[tokio::test]
    async fn learn_events_are_scoped_to_the_companion_that_ran() {
        let dir = tempfile::tempdir().unwrap();
        seed_event(dir.path());
        let reply = r#"{"memories":[{"kind":"profile","content":"主人是 Rust 工程师","importance":0.9}],
            "mood":"content","diary":"今天陪主人修了 bug～"}"#;

        let registry = Arc::new(
            CompanionRegistry::scan(
                dir.path().join("companions"),
                dir.path().join("shared"),
            )
            .unwrap(),
        );
        let _a = registry.create("甲", "ink").await.unwrap();
        let b = registry.create("乙", "ink").await.unwrap();
        registry
            .patch(
                &b.companion_id,
                serde_json::json!({"learn": serde_json::to_value(test_learn_config()).unwrap()}),
            )
            .await
            .unwrap();

        let bc = Arc::new(RecordingBroadcaster::default());
        let learner = Learner {
            companion_dir: dir.path().to_path_buf(),
            store: CompanionStore::open_memory().await.unwrap(),
            registry,
            completer: Arc::new(CannedCompleter(reply.to_owned())),
            emitter: CompanionEventEmitter::new(bc.clone(), "owner-a"),
            event_store_lock: Arc::new(RwLock::new(())),
            run_locks: Arc::new(CompanionRunLocks::new()),
        };
        learner.run_for(&b.companion_id).await.unwrap();

        let events = bc.events.lock().unwrap().clone();
        for name in [
            "companion.mood-changed",
            "companion.learn-finished",
            "companion.learn-started",
        ] {
            let evs: Vec<_> = events.iter().filter(|e| e.name == name).collect();
            assert!(!evs.is_empty(), "expected at least one {name} event");
            for e in evs {
                assert_eq!(
                    e.data.get("companion_id").and_then(|v| v.as_str()),
                    Some(b.companion_id.as_str()),
                    "{name} 必须 scope 到真正跑了这一轮的 乙"
                );
            }
        }

        let finished = events
            .iter()
            .find(|event| event.name == "companion.learn-finished")
            .unwrap();
        let payload = finished.data.as_object().unwrap();
        let actual_keys: std::collections::BTreeSet<&str> =
            payload.keys().map(String::as_str).collect();
        let expected_keys: std::collections::BTreeSet<&str> = [
            "companion_id",
            "error",
            "events_processed",
            "memories_added",
            "status",
            "summary",
        ]
        .into_iter()
        .collect();
        assert_eq!(actual_keys, expected_keys, "live result must not expose persisted run fields");
    }
}
