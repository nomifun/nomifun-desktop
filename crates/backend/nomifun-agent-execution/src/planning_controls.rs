//! Cancellation capabilities for in-flight model-only planning calls. SQLite
//! remains the authority for admission and settlement; these handles contain no
//! plan, checkpoint, scheduler state, or replayable work.
use std::sync::{Arc, Mutex};

use nomifun_common::AppError;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub(crate) struct PlanningControls(Arc<Mutex<Vec<Arc<PlanningOperation>>>>,
    #[cfg(any(test,feature="test-support"))] Arc<Mutex<std::collections::BTreeMap<String,Arc<InitialPlanningBarrier>>>>,
    #[cfg(test)] Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)] Arc<std::sync::atomic::AtomicBool>);

#[cfg(any(test,feature="test-support"))]
struct InitialPlanningBarrier {entered:Arc<tokio::sync::Notify>,release:Arc<tokio::sync::Notify>}

struct PlanningOperation {
    execution_id: String,
    version: i64,
    delegation_step: Option<String>,
    initial: bool,
    cancellation: CancellationToken,
}

pub(crate) struct PlanningGuard {
    controls: PlanningControls,
    operation: Arc<PlanningOperation>,
}

#[derive(Clone, Copy)]
pub(crate) struct PlanningTarget<'a> {
    pub(crate) version: i64,
    pub(crate) delegation: Option<(&'a str, &'a str)>,
}

impl PlanningControls {
    #[cfg(test)]
    pub(crate) fn note_database_retry(&self) {self.2.fetch_add(1,std::sync::atomic::Ordering::Relaxed);}
    #[cfg(test)]
    pub(crate) fn database_retries(&self)->usize {self.2.load(std::sync::atomic::Ordering::Relaxed)}
    #[cfg(test)]
    pub(crate) fn lose_next_plan_ack(&self) {self.3.store(true,std::sync::atomic::Ordering::SeqCst);}
    #[cfg(test)]
    pub(crate) fn take_lost_plan_ack(&self)->bool {self.3.swap(false,std::sync::atomic::Ordering::SeqCst)}
    pub(crate) fn has_live_owner(&self,execution_id:&str)->bool {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter()
            .any(|operation|operation.execution_id==execution_id&&!operation.cancellation.is_cancelled())
    }

    #[cfg(any(test,feature="test-support"))]
    pub(crate) fn hold_initial_for_test(&self,goal:&str)->Result<(Arc<tokio::sync::Notify>,Arc<tokio::sync::Notify>),AppError> {
        let mut barriers=self.1.lock().map_err(|_|AppError::Internal("initial planning test barrier poisoned".into()))?;
        if goal.is_empty()||barriers.contains_key(goal) {return Err(AppError::Conflict("initial planning test barrier requires its unique original goal".into()));}
        let barrier=Arc::new(InitialPlanningBarrier {entered:Default::default(),release:Default::default()});
        barriers.insert(goal.into(),barrier.clone());Ok((barrier.entered.clone(),barrier.release.clone()))
    }

    #[cfg(any(test,feature="test-support"))]
    pub(crate) async fn wait_initial_for_test(&self,goal:&str,owner:&PlanningGuard)->Result<bool,AppError> {
        let barrier=self.1.lock().map_err(|_|AppError::Internal("initial planning test barrier poisoned".into()))?.remove(goal);
        if let Some(barrier)=barrier {
            barrier.entered.notify_one();
            tokio::select! {biased;
                _=owner.cancelled()=>return Ok(false),
                _=barrier.release.notified()=>{},
            }
        }
        Ok(!owner.is_cancelled())
    }

    pub(crate) fn observations(&self) -> Vec<(String, i64, bool)> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter()
            .map(|operation| (operation.execution_id.clone(), operation.version, operation.cancellation.is_cancelled())).collect()
    }

    pub(crate) fn register(
        &self, execution_id: &str, version: i64, delegation_step: Option<&str>,
        initial: bool, cancellation: CancellationToken,
    ) -> Result<Option<PlanningGuard>, AppError> {
        let mut operations = self.0.lock().map_err(|_| AppError::Internal("planning control lock poisoned".into()))?;
        if initial && operations.iter().any(|operation| operation.initial && operation.execution_id == execution_id) {
            return Ok(None);
        }
        let operation = Arc::new(PlanningOperation {
            execution_id: execution_id.to_owned(), version,
            delegation_step: delegation_step.map(str::to_owned), initial, cancellation,
        });
        operations.push(operation.clone());
        Ok(Some(PlanningGuard { controls: self.clone(), operation }))
    }

    /// Signal only jobs admitted before this committed fence. A newer explicit
    /// epoch of the same Execution cannot inherit an old Stop capability.
    pub(crate) fn stop_through(&self, execution_id: &str, version: i64, steps: Option<&[String]>) {
        let operations = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for operation in operations.iter().filter(|operation| {
            operation.execution_id == execution_id && operation.version <= version
                && steps.is_none_or(|steps| operation.delegation_step.as_ref().is_some_and(|step| steps.contains(step)))
        }) {
            operation.cancellation.cancel();
        }
    }
}

impl PlanningGuard {
    pub(crate) async fn cancelled(&self) { self.operation.cancellation.cancelled().await; }
    pub(crate) fn is_cancelled(&self) -> bool { self.operation.cancellation.is_cancelled() }
}

impl Drop for PlanningGuard {
    fn drop(&mut self) {
        self.controls.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|operation| !Arc::ptr_eq(operation, &self.operation));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_signals_original_scope_without_sibling_or_successor_or_lingering_cache() {
        let controls = PlanningControls::default();
        let register = |execution, version, step| controls.register(execution, version, step, false, CancellationToken::new()).unwrap().unwrap();
        let old = register("a", 3, Some("step-a"));
        let sibling = register("a", 3, Some("step-b"));
        let other = register("b", 3, None);
        let successor = register("a", 5, Some("step-a"));
        controls.stop_through("a", 4, Some(&["step-a".into()]));
        old.cancelled().await;
        assert!(controls.observations().contains(&("a".into(), 3, true)));
        assert!(!sibling.is_cancelled()); assert!(!other.is_cancelled()); assert!(!successor.is_cancelled());
        controls.stop_through("a", 4, None);
        assert!(sibling.is_cancelled()); assert!(!other.is_cancelled()); assert!(!successor.is_cancelled());
        drop((old, sibling, other, successor));
        assert!(controls.0.lock().unwrap().is_empty());
        assert!(controls.observations().is_empty());
    }

    #[test]
    fn initial_planning_has_one_original_owner_and_shutdown_propagates() {
        let controls = PlanningControls::default(); let shutdown = CancellationToken::new();
        let first = controls.register("a", 1, None, true, shutdown.child_token()).unwrap().unwrap();
        assert!(controls.has_live_owner("a"));assert!(!controls.has_live_owner("foreign"));
        assert!(controls.register("a", 1, None, true, shutdown.child_token()).unwrap().is_none());
        shutdown.cancel(); assert!(first.is_cancelled());assert!(!controls.has_live_owner("a")); drop(first);
        assert!(controls.register("a", 2, None, true, CancellationToken::new()).unwrap().is_some());
        assert!(controls.0.lock().unwrap().is_empty());
    }
}
