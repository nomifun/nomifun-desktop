use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use nomifun_ai_agent::IWorkerTaskManager;
use nomifun_ai_agent::types::SendMessageData;
use nomifun_common::ConversationStatus;
use nomifun_conversation::ConversationService;
use nomifun_conversation::runtime_state::TurnClaim;
use nomifun_realtime::EventBroadcaster;
use tokio::sync::Notify;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::mailbox::Mailbox;
use crate::scheduler::TeammateManager;
use crate::session::TeamSession;
use crate::types::TeammateStatus;

/// Cap on team-agent turns running concurrently across the whole team. Without
/// it, a settled team can fire every agent's expensive LLM turn at once — a
/// provider-rate-limit / resource storm. Generous enough not to serialise small
/// teams. (TIER-3 hardening, §3.4)
const MAX_CONCURRENT_TEAM_TURNS: usize = 4;

/// Registry of per-agent Notify handles. Used by any trigger source to poke
/// an agent's event loop without needing to know its internals.
pub struct EventLoopRegistry {
    notifiers: DashMap<String, Arc<Notify>>,
    handles: DashMap<String, JoinHandle<()>>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    /// Shared across all agents' loops so the concurrent-turn cap is team-wide.
    turn_semaphore: Arc<Semaphore>,
}

impl Default for EventLoopRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLoopRegistry {
    pub fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        Self {
            notifiers: DashMap::new(),
            handles: DashMap::new(),
            shutdown_tx,
            shutdown_rx,
            turn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_TEAM_TURNS)),
        }
    }

    /// Check if an event loop is registered for this slot.
    pub fn has(&self, slot_id: &str) -> bool {
        self.notifiers.contains_key(slot_id)
    }

    /// Poke the named agent's event loop so it drains its mailbox.
    pub fn notify(&self, slot_id: &str) {
        if let Some(n) = self.notifiers.get(slot_id) {
            n.notify_one();
        }
    }

    /// Register and spawn an event loop for one agent.
    pub fn spawn(&self, slot_id: &str, ctx: AgentLoopContext) {
        let notify = Arc::new(Notify::new());
        self.notifiers.insert(slot_id.to_owned(), notify.clone());
        let handle = tokio::spawn(run_event_loop(
            notify,
            self.shutdown_rx.clone(),
            self.turn_semaphore.clone(),
            ctx,
        ));
        self.handles.insert(slot_id.to_owned(), handle);
    }

    /// Remove an agent's event loop (agent removed from team).
    pub fn remove(&self, slot_id: &str) {
        self.notifiers.remove(slot_id);
        if let Some((_, handle)) = self.handles.remove(slot_id) {
            handle.abort();
        }
    }

    /// Shut down all event loops.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        for entry in self.handles.iter() {
            entry.value().abort();
        }
        self.handles.clear();
        self.notifiers.clear();
    }
}

/// Context shared across all iterations of one agent's event loop.
pub struct AgentLoopContext {
    pub team_id: String,
    pub slot_id: String,
    pub user_id: String,
    pub session: Arc<TeamSession>,
    pub scheduler: Arc<TeammateManager>,
    pub mailbox: Arc<Mailbox>,
    pub task_manager: Arc<dyn IWorkerTaskManager>,
    pub conversation_service: ConversationService,
    pub broadcaster: Arc<dyn EventBroadcaster>,
    /// Used to notify other agents' event loops (e.g. leader after all-settled).
    pub registry: Arc<EventLoopRegistry>,
}

struct TurnExecution {
    finish_ok: bool,
    claim: TurnClaim,
}

/// The event loop for one agent slot. Spawned as a tokio task.
///
/// Flow:
/// 1. Wait for signal (notify) or shutdown.
/// 2. Drain loop: compute_wake_input → has messages → send_message (blocking) → finalize → repeat.
/// 3. When mailbox empty → back to step 1.
async fn run_event_loop(
    notify: Arc<Notify>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    turn_semaphore: Arc<Semaphore>,
    ctx: AgentLoopContext,
) {
    info!(
        team_id = %ctx.team_id,
        slot_id = %ctx.slot_id,
        "agent event loop started"
    );

    loop {
        // Step 1: wait for signal or shutdown
        tokio::select! {
            biased;
            _ = shutdown_rx.wait_for(|v| *v) => {
                info!(
                    team_id = %ctx.team_id,
                    slot_id = %ctx.slot_id,
                    "agent event loop shutting down"
                );
                return;
            }
            _ = notify.notified() => {}
        }

        // Drain loop: keep processing until mailbox is empty
        loop {
            if *shutdown_rx.borrow() {
                return;
            }

            let input = match ctx.session.compute_wake_input(&ctx.slot_id).await {
                Ok(Some(input)) => input,
                Ok(None) => break,
                Err(e) => {
                    warn!(
                        team_id = %ctx.team_id,
                        slot_id = %ctx.slot_id,
                        error = %e,
                        "event loop: compute_wake_input failed"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    break;
                }
            };

            if !input.should_send {
                break;
            }

            // Bound concurrent team turns: hold a permit only across the
            // (expensive) turn execution, so at most MAX_CONCURRENT_TEAM_TURNS
            // agents call the model at once. Released before finalize.
            let outcome = {
                let _permit = turn_semaphore.acquire().await.ok();
                execute_turn(&ctx, &input).await
            };
            match outcome {
                Some(turn) => finalize_turn(&ctx, turn, &input.conversation_id).await,
                None => break, // Turn not started (guard/warmup); retry on next signal
            }
        }
    }
}

/// Execute one agent turn: warmup → guard → set Working → StreamRelay → send_message (blocking).
/// Returns `Some(true)` on success, `Some(false)` on error,
/// `None` if the turn was not started (guard hit, warmup fail, etc.).
async fn execute_turn(ctx: &AgentLoopContext, input: &crate::session::WakeInput) -> Option<TurnExecution> {
    ctx.session.mirror_unread_to_conversation(input).await;

    // Ensure agent task exists
    let handle = match ctx.task_manager.get_task(&input.conversation_id) {
        Some(h) => h,
        None => {
            if let Err(e) = ctx
                .conversation_service
                .warmup(&ctx.user_id, &input.conversation_id, &ctx.task_manager)
                .await
            {
                warn!(
                    team_id = %ctx.team_id,
                    slot_id = %ctx.slot_id,
                    conversation_id = %input.conversation_id,
                    error = %e,
                    "event loop: warmup failed"
                );
                return None;
            }
            match ctx.task_manager.get_task(&input.conversation_id) {
                Some(h) => h,
                None => {
                    warn!(
                        team_id = %ctx.team_id,
                        slot_id = %ctx.slot_id,
                        conversation_id = %input.conversation_id,
                        "event loop: no task after warmup"
                    );
                    return None;
                }
            }
        }
    };

    // Guard: skip if already running
    if handle.status() == Some(ConversationStatus::Running) {
        return None;
    }
    let claim = match ctx
        .conversation_service
        .runtime_state()
        .try_claim_turn(&input.conversation_id)
    {
        Ok(claim) => claim,
        Err(e) => {
            warn!(
                team_id = %ctx.team_id,
                slot_id = %ctx.slot_id,
                conversation_id = %input.conversation_id,
                error = %e,
                "event loop: runtime turn claim rejected"
            );
            return None;
        }
    };

    // Point-of-no-return: set Working. Runtime state, not DB status, is the turn guard.
    let _ = ctx.scheduler.set_status(&ctx.slot_id, TeammateStatus::Working).await;
    let repo = ctx.conversation_service.conversation_repo();

    // StreamRelay for response persistence + WebSocket forwarding
    let msg_id = ConversationService::mint_msg_id();
    let rx = handle.subscribe();
    let relay = nomifun_conversation::stream_relay::StreamRelay::new(
        input.conversation_id.clone(),
        msg_id.clone(),
        ctx.user_id.clone(),
        Arc::clone(repo),
        ctx.broadcaster.clone(),
        None,
    );
    tokio::spawn(async move { relay.consume(rx).await });

    // Collect files from unread messages (user-attached files)
    let files: Vec<String> = input
        .unread
        .iter()
        .filter_map(|m| m.files.as_ref())
        .flatten()
        .cloned()
        .collect();

    let data = SendMessageData {
        content: input.first_message.clone(),
        msg_id,
        files,
        inject_skills: Vec::new(),
        origin: None,
    };

    let turn_ok = match handle.send_message(data).await {
        Ok(()) => true,
        Err(e) => {
            warn!(
                team_id = %ctx.team_id,
                slot_id = %ctx.slot_id,
                conversation_id = %input.conversation_id,
                error = %e,
                "event loop: send_message failed"
            );
            false
        }
    };

    // Mark messages as read regardless of turn outcome
    let msg_ids: Vec<i64> = input.unread.iter().map(|m| m.id).collect();
    if !msg_ids.is_empty()
        && let Err(e) = ctx.mailbox.mark_read_batch(&msg_ids).await
    {
        warn!(
            team_id = %ctx.team_id,
            slot_id = %ctx.slot_id,
            error = %e,
            "event loop: mark_read_batch failed (non-fatal)"
        );
    }

    Some(TurnExecution {
        finish_ok: turn_ok,
        claim,
    })
}

/// Finalize a completed turn: release runtime claim, mark idle (or error), cascade to leader.
async fn finalize_turn(ctx: &AgentLoopContext, mut turn: TurnExecution, _conversation_id: &str) {
    turn.claim.release();

    if !turn.finish_ok {
        let _ = ctx.scheduler.set_status(&ctx.slot_id, TeammateStatus::Error).await;
    }
    match ctx.scheduler.finalize_turn(&ctx.slot_id, &[]).await {
        Ok(Some(wake_target)) => {
            if wake_target != ctx.slot_id {
                ctx.registry.notify(&wake_target);
            }
        }
        Ok(None) => {}
        Err(e) => {
            warn!(
                team_id = %ctx.team_id,
                slot_id = %ctx.slot_id,
                error = %e,
                "event loop: finalize_turn failed"
            );
        }
    }
}
