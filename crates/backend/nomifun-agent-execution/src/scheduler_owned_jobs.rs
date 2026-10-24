//! Process-local ownership of the original Step future. The durable Attempt
//! and native Session remain the facts; this registry never admits work.
use super::*;

pub(super) struct OwnedStepJob {
    reserved_step_version:i64,
    attempt: Mutex<Option<String>>,
    lease: Mutex<Option<AgentExecutionLeaseToken>>,
}

impl ExecutionScheduler {
    pub(crate) fn has_step_job(&self, execution: &str, step: &str, attempt: Option<&str>,observed_step_version:Option<i64>) -> bool {
        self.inner.step_jobs.get(&(execution.to_owned(),step.to_owned())).is_some_and(|job|
            job.lease.lock().unwrap_or_else(|error|error.into_inner()).is_some()
                &&attempt.is_none_or(|id|match job.attempt.lock().unwrap_or_else(|error|error.into_inner()).as_deref() {
                    Some(original)=>original==id,
                    None=>observed_step_version.is_some_and(|version|job.reserved_step_version.checked_add(1)==Some(version)),
                }))
    }

    pub(super) fn bind_step_job_attempt(&self,execution:&str,step:&str,attempt:&str)->Result<(),AppError> {
        if let Some(job)=self.inner.step_jobs.get(&(execution.to_owned(),step.to_owned())) {
            let mut original=job.attempt.lock().unwrap_or_else(|error|error.into_inner());
            if original.as_deref().is_some_and(|id|id!=attempt) {
                return Err(AppError::Conflict("original Step job cannot adopt another Attempt".into()));
            }
            *original=Some(attempt.into());
        }
        Ok(())
    }

    /// Called only after the new generation acquired its real writer lease.
    /// The existing task accepts that generation's settlement responsibility;
    /// its original invocation/result does not move through a notification.
    pub(super) fn adopt_step_jobs(&self,execution:&str,lease:&AgentExecutionLeaseToken) {
        for job in self.inner.step_jobs.iter().filter(|job|job.key().0==execution) {
            *job.lease.lock().unwrap_or_else(|error|error.into_inner())=Some(lease.clone());
        }
    }

    pub(super) fn fence_step_jobs(&self,execution:&str,lease:&AgentExecutionLeaseToken) {
        for job in self.inner.step_jobs.iter().filter(|job|job.key().0==execution) {
            let mut assigned=job.lease.lock().unwrap_or_else(|error|error.into_inner());
            if assigned.as_ref().is_some_and(|current|current.owner()==lease.owner()) { *assigned=None; }
        }
    }

    pub(super) fn step_job_lease(&self,execution:&str,step:&str,fallback:&AgentExecutionLeaseToken)->Option<AgentExecutionLeaseToken> {
        self.inner.step_jobs.get(&(execution.to_owned(),step.to_owned())).map_or_else(||Some(fallback.clone()),
            |job|job.lease.lock().unwrap_or_else(|error|error.into_inner()).clone())
    }

    pub(super) fn owned_step_ids(&self,execution:&str)->HashSet<String> {
        self.inner.step_jobs.iter().filter(|job|job.key().0==execution).map(|job|job.key().1.clone()).collect()
    }

    pub(super) fn retain_step_job(&self,execution:String,step:String,reserved_step_version:i64,attempt:Option<String>,lease:AgentExecutionLeaseToken,
        jobs:&mut ScheduledAgentJobs,future:BoxFuture<'static,String>) {
        use dashmap::mapref::entry::Entry;
        let key=(execution,step.clone());
        let owned=Arc::new(OwnedStepJob {reserved_step_version,attempt:Mutex::new(attempt),lease:Mutex::new(Some(lease))});
        match self.inner.step_jobs.entry(key.clone()) {
            Entry::Occupied(_)=>return,
            Entry::Vacant(entry)=>{entry.insert(owned.clone());}
        }
        let scheduler=self.clone();
        let (completion,notification)=tokio::sync::oneshot::channel();
        self.inner.deps.lifecycle.spawn(async move {
            let result=std::panic::AssertUnwindSafe(future).catch_unwind().await;
            if result.is_err() {tracing::error!(execution_id=%key.0,step_id=%key.1,"original Step owner panicked; canonical receipt recovery required");}
            scheduler.inner.step_jobs.remove_if(&key,|_,current|Arc::ptr_eq(current,&owned));
            // The task has already settled or transferred its exact receipt
            // responsibility before this optional observer receives a notice.
            let _=completion.send(key.1.clone());
            if let Some(handle)=scheduler.inner.active.get(&key.0) {handle.wake.notify_one();}
        });
        jobs.push(async move {notification.await.unwrap_or(step)}.boxed());
    }
}
