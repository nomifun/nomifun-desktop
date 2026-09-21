//! Reusable turn admission and retained task witnesses for hosted tools.
//! Joining tasks proves only dispatch completion; resource owners supply the
//! additional settlement checks. This is not a durable effect receipt.
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures_util::FutureExt;
use nomifun_common::AppError;

use crate::engine_tasks::{EngineOwnedTask, EngineTaskGroup};

#[async_trait]
pub trait EngineEffectSettlement: Send + Sync {
    /// Must reject active or uncertain owner effects, including dropped calls.
    /// A cancelled waiter must not discard the owner's only cleanup witness.
    async fn ensure_settled(&self) -> Result<(), AppError>;

    /// Distinct from cleanup: may this original user source be automatically
    /// replayed or destructively edited? Owners without this proof fail closed.
    async fn ensure_source_replay_safe(&self, _source: &str) -> Result<(), AppError> {
        Err(AppError::Conflict(
            "Effect owner has no source-replay proof".into(),
        ))
    }
}

#[derive(Default)]
struct Admission {
    open: bool,
    retired: bool,
    settled: bool,
    settlement_failed: bool,
}

pub struct EngineEffectScope {
    admission: Mutex<Admission>,
    tasks: EngineTaskGroup,
    witnesses: Vec<Arc<dyn EngineEffectSettlement>>,
    settlement: tokio::sync::Mutex<()>,
}

fn failure() -> AppError {
    AppError::Conflict("Hosted tool effects are closed or not proven settled".into())
}

/// Isolate one cleanup callback, including a panic while constructing its
/// future. This does not retain work, retry effects or establish settlement:
/// the caller must keep ownership and reject the boundary on any error.
/// Only unwinding panics are caught; process aborts remain crash recovery.
pub async fn guard_effect_settlement<T, F, Fut>(operation: F) -> Result<T, AppError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    AssertUnwindSafe(async { operation().await })
        .catch_unwind()
        .await
        .map_err(|_| failure())?
}

impl EngineEffectScope {
    pub fn new(witnesses: Vec<Arc<dyn EngineEffectSettlement>>) -> Result<Self, AppError> {
        if witnesses.len() > 64 {
            return Err(failure());
        }
        Ok(Self {
            admission: Mutex::new(Admission {
                settled: true,
                ..Default::default()
            }),
            tasks: EngineTaskGroup::new(1024)?,
            witnesses,
            settlement: tokio::sync::Mutex::new(()),
        })
    }

    /// The engine calls this only after its previous turn fence completed.
    pub fn begin_turn(&self) -> Result<(), AppError> {
        let mut state = self.admission.lock().map_err(|_| failure())?;
        if state.retired || state.open || !state.settled || !self.tasks.is_quiescent()? {
            return Err(failure());
        }
        state.open = true;
        state.settled = false;
        Ok(())
    }

    pub fn spawn<T, F>(&self, operation: F) -> Result<EngineOwnedTask<T>, AppError>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let state = self.admission.lock().map_err(|_| failure())?;
        if !state.open || state.retired {
            return Err(failure());
        }
        // Keep admission locked until the task's completion witness is owned.
        self.tasks.spawn(operation)
    }

    pub fn close_session(&self) -> Result<(), AppError> {
        let mut state = self.admission.lock().map_err(|_| failure())?;
        state.open = false;
        state.retired = true;
        Ok(())
    }

    fn fail_settlement(&self) -> Result<(), AppError> {
        let mut state = self.admission.lock().map_err(|_| failure())?;
        state.open = false;
        state.retired = true;
        state.settled = false;
        state.settlement_failed = true;
        Ok(())
    }

    pub fn close_turn(&self) -> Result<(), AppError> {
        self.admission.lock().map_err(|_| failure())?.open = false;
        Ok(())
    }

    /// A provider-boundary fence, not a settlement proof. In particular, a
    /// cancelled tool waiter must not be followed by another model round just
    /// because its retained task happened to finish before the history read.
    pub fn ensure_turn_open(&self) -> Result<(), AppError> {
        let state = self.admission.lock().map_err(|_| failure())?;
        if !state.open || state.retired {
            return Err(failure());
        }
        Ok(())
    }

    /// Cancellation of this waiter does not consume task handles or permit
    /// the next turn. Every owner is checked even if another one fails or
    /// unwinds. A failing callback never becomes a successful cleanup proof.
    pub async fn settle_turn(&self) -> Result<(), AppError> {
        let _settlement = self.settlement.lock().await;
        let previously_failed = {
            let mut state = self.admission.lock().map_err(|_| failure())?;
            state.open = false;
            state.settled = false;
            state.settlement_failed
        };
        let mut failed =
            guard_effect_settlement(|| self.tasks.join()).await.is_err() || previously_failed;
        if failed {
            self.fail_settlement()?;
        }
        for witness in &self.witnesses {
            if guard_effect_settlement(|| witness.ensure_settled())
                .await
                .is_err()
            {
                failed = true;
                // Persist the failed fence before awaiting the next owner.
                // Dropping this waiter must not erase an observed failure.
                self.fail_settlement()?;
            }
        }
        let mut state = self.admission.lock().map_err(|_| failure())?;
        if failed {
            state.retired = true;
            return Err(failure());
        }
        state.settled = true;
        Ok(())
    }

    pub async fn settle_session(&self) -> Result<(), AppError> {
        self.close_session()?;
        self.settle_turn().await
    }

    /// Caller must hold its turn boundary. Never query while dispatch is open:
    /// an observation before a still-running tool is not a replay permit.
    pub async fn ensure_source_replay_safe(&self, source: &str) -> Result<(), AppError> {
        let _settlement = self.settlement.lock().await;
        {
            let state = self.admission.lock().map_err(|_| failure())?;
            if state.retired || state.open || !state.settled || !self.tasks.is_quiescent()? {
                return Err(failure());
            }
        }
        for witness in &self.witnesses {
            witness.ensure_source_replay_safe(source).await?;
        }
        Ok(())
    }
}
