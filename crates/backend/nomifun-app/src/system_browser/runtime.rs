//! Agent-run ownership for explicitly connected and user-authorized browser tabs.
use super::*;
use nomifun_browser_platform::system_browser::{
    SystemBrowserBinding, SystemBrowserHost, SystemBrowserTurn, SystemBrowserWorkspace,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{
    Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

type Error = SystemBrowserRuntimeError;
#[derive(Clone)]
enum Job {
    Invoke(Shared<BoxFuture<'static, Result<Value, Error>>>),
    Detach(Shared<BoxFuture<'static, Result<(), Error>>>),
}
const ACTIVE: u8 = 0;
const SETTLING: u8 = 1;
const SETTLED: u8 = 2;
const FINISHED: u8 = 3;

#[async_trait]
pub trait SystemBrowserOwnerVerifier: Send + Sync {
    async fn verify(&self, user_id: &str, conversation_id: &str) -> Result<(), Error>;
}

#[derive(Default)]
struct State {
    closed: bool,
    runs: BTreeMap<Owner, Arc<Run>>,
    claims: BTreeMap<String, String>,
}
#[derive(Default)]
pub(super) struct Runtime {
    state: Mutex<State>,
}
struct Workspace {
    service: Arc<SystemBrowserService>,
    owner: Owner,
}
struct FrozenConnection {
    incarnation: String,
    connection: Arc<dyn SystemBrowserConnection>,
    tabs: BTreeMap<String, SystemBrowserTab>,
}
struct Run {
    id: String,
    owner: Owner,
    service: Weak<SystemBrowserService>,
    runtime: Weak<Runtime>,
    frozen: Option<FrozenConnection>,
    phase: AtomicU8,
    cancel: CancellationToken,
    operation: tokio::sync::Mutex<()>,
    job: Mutex<Option<Job>>,
    touched: Arc<Mutex<BTreeSet<String>>>,
    orphaned: AtomicBool,
    wake: tokio::sync::Notify,
}
struct Turn(Arc<Run>);

impl SystemBrowserService {
    pub fn install_owner_verifier(
        &self,
        verifier: Arc<dyn SystemBrowserOwnerVerifier>,
    ) -> Result<(), Error> {
        self.owner_verifier.set(verifier).map_err(|_| Error::Busy)
    }
    async fn verify_owner(&self, owner: &Owner) -> Result<(), Error> {
        if self.closed.is_cancelled() {
            return Err(Error::Disconnected);
        }
        self.owner_verifier
            .get()
            .ok_or(Error::Unavailable)?
            .verify(&owner.0, &owner.1)
            .await
    }
}
#[async_trait]
impl SystemBrowserHost for SystemBrowserService {
    fn binding(&self) -> SystemBrowserBinding {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        for source in [
            AttachedBrowser::automation_digest().into_bytes(),
            include_bytes!("runtime.rs").to_vec(),
            include_bytes!("../system_browser.rs").to_vec(),
            nomifun_browser_platform::system_browser::input_schema()
                .to_string()
                .into_bytes(),
        ] {
            hash.update((source.len() as u64).to_le_bytes());
            hash.update(source);
        }
        SystemBrowserBinding {
            schema_version: 1,
            runtime_digest: format!("{:x}", hash.finalize()),
        }
    }
    async fn workspace(
        &self,
        user: &str,
        conversation: &str,
    ) -> Result<Arc<dyn SystemBrowserWorkspace>, Error> {
        let owner = (user.to_owned(), conversation.to_owned());
        self.verify_owner(&owner).await?;
        Ok(Arc::new(Workspace {
            service: self.weak.upgrade().ok_or(Error::Disconnected)?,
            owner,
        }))
    }
}
#[async_trait]
impl SystemBrowserWorkspace for Workspace {
    async fn begin_run(&self) -> Result<Arc<dyn SystemBrowserTurn>, Error> {
        self.service.verify_owner(&self.owner).await?;
        let frozen = {
            let state = self.service.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(Error::Disconnected);
            }
            state.entries.get(&self.owner).and_then(|entry| {
                let data = entry.data.lock().unwrap_or_else(|e| e.into_inner());
                (data.state == SystemBrowserState::Connected)
                    .then(|| data.connection.clone())
                    .flatten()
                    .filter(|connection| connection.is_connected())
                    .map(|connection| FrozenConnection {
                        incarnation: entry.incarnation.clone(),
                        connection,
                        tabs: data.tabs.clone(),
                    })
            })
        };
        let runtime = self.service.runtime.clone();
        let run = {
            let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(Error::Disconnected);
            }
            if state.runs.contains_key(&self.owner) {
                return Err(Error::Busy);
            }
            if state.runs.len() >= MAX_CONNECTIONS {
                return Err(Error::Busy);
            }
            let run = Arc::new(Run {
                id: uuid::Uuid::now_v7().to_string(),
                owner: self.owner.clone(),
                service: Arc::downgrade(&self.service),
                runtime: Arc::downgrade(&runtime),
                frozen,
                phase: AtomicU8::new(ACTIVE),
                cancel: CancellationToken::new(),
                operation: tokio::sync::Mutex::new(()),
                job: Mutex::new(None),
                touched: Arc::new(Mutex::new(BTreeSet::new())),
                orphaned: AtomicBool::new(false),
                wake: tokio::sync::Notify::new(),
            });
            state.runs.insert(self.owner.clone(), run.clone());
            run
        };
        // Established before returning the handle: caller Drop cannot discard
        // an accepted operation or its final detach/claim cleanup authority.
        tokio::spawn(custodian(run.clone(), runtime));
        Ok(Arc::new(Turn(run)))
    }
}
impl Run {
    fn current(&self) -> Result<Arc<Runtime>, Error> {
        let runtime = self.runtime.upgrade().ok_or(Error::StaleRun)?;
        let valid = runtime
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .runs
            .get(&self.owner)
            .is_some_and(|run| run.id == self.id);
        if !valid {
            return Err(Error::StaleRun);
        }
        Ok(runtime)
    }
    fn connection(&self) -> Result<&FrozenConnection, Error> {
        let frozen = self.frozen.as_ref().ok_or(Error::Unavailable)?;
        let service = self.service.upgrade().ok_or(Error::Disconnected)?;
        let entry = service
            .entry(&self.owner.0, &self.owner.1, &frozen.incarnation, false)
            .map_err(|_| Error::Disconnected)?;
        let connected = service.connected(&entry).map_err(|_| Error::Disconnected)?;
        if !Arc::ptr_eq(&connected, &frozen.connection) {
            return Err(Error::Disconnected);
        }
        Ok(frozen)
    }
    async fn join_job(&self) -> Result<(), Error> {
        let job = self.job.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(job) = job {
            // An invocation error still proves its work ended. A detach error
            // does not prove cleanup and must retain the run and its claims.
            let result = match job {
                Job::Invoke(job) => {
                    let _ = job.await;
                    Ok(())
                }
                Job::Detach(job) => job.await,
            };
            self.job.lock().unwrap_or_else(|e| e.into_inner()).take();
            result?;
        }
        Ok(())
    }
    async fn settle(&self) -> Result<(), Error> {
        self.cancel.cancel();
        self.phase
            .compare_exchange(ACTIVE, SETTLING, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        let _operation = self.operation.lock().await;
        if self.phase.load(Ordering::Acquire) >= SETTLED {
            return Ok(());
        }
        self.current()?;
        self.join_job().await?;
        let tabs: Vec<_> = self
            .touched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        if let Some(frozen) = &self.frozen {
            if !tabs.is_empty() {
                let connection = frozen.connection.clone();
                let touched = self.touched.clone();
                let worker = tokio::spawn(async move {
                    let mut failed = None;
                    for tab in tabs {
                        match connection.settle(&tab).await {
                            Ok(()) => {
                                touched
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&tab);
                            }
                            Err(error) => failed = Some(error),
                        }
                    }
                    failed.map_or(Ok(()), Err)
                });
                let completion = async move { worker.await.unwrap_or(Err(Error::ExecutionFailed)) }
                    .boxed()
                    .shared();
                *self.job.lock().unwrap_or_else(|e| e.into_inner()) = Some(Job::Detach(completion));
                self.join_job().await?;
            }
        }
        self.phase.store(SETTLED, Ordering::Release);
        Ok(())
    }
    async fn finish(&self) -> Result<(), Error> {
        let _operation = self.operation.lock().await;
        let phase = self.phase.load(Ordering::Acquire);
        if phase == FINISHED {
            return Ok(());
        }
        if phase != SETTLED {
            return Err(Error::Busy);
        }
        let runtime = self.current()?;
        let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
        state.claims.retain(|_, owner| owner != &self.id);
        state.runs.remove(&self.owner);
        self.phase.store(FINISHED, Ordering::Release);
        self.wake.notify_one();
        Ok(())
    }
}
#[async_trait]
impl SystemBrowserTurn for Turn {
    fn cancel(&self) {
        self.0.cancel.cancel();
    }
    async fn invoke(&self, command: SystemBrowserCommand) -> Result<Value, Error> {
        let run = &self.0;
        let _operation = run.operation.lock().await;
        let runtime = run.current()?;
        if run.phase.load(Ordering::Acquire) != ACTIVE {
            return Err(Error::StaleRun);
        }
        if run.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        run.join_job().await?;
        if run.phase.load(Ordering::Acquire) != ACTIVE || run.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let frozen = run.connection()?;
        if frozen.tabs.is_empty() {
            return Err(Error::Unavailable);
        }
        let tab = match &command {
            SystemBrowserCommand::Tabs {} => {
                let tabs = frozen
                    .tabs
                    .values()
                    .map(|tab| {
                        json!({"tab_id":tab.tab_id,"title":tab.title,
                    "url":nomifun_browser_platform::url_projection::project_metadata_url(&tab.url)})
                    })
                    .collect::<Vec<_>>();
                return Ok(json!({"tabs":tabs,"untrusted_page_content":true}));
            }
            SystemBrowserCommand::Observe { tab_id }
            | SystemBrowserCommand::Navigate { tab_id, .. }
            | SystemBrowserCommand::Dialog { tab_id, .. }
            | SystemBrowserCommand::Click { tab_id, .. }
            | SystemBrowserCommand::Type { tab_id, .. }
            | SystemBrowserCommand::Press { tab_id, .. }
            | SystemBrowserCommand::Scroll { tab_id, .. } => tab_id.clone(),
        };
        if !frozen.tabs.contains_key(&tab) {
            return Err(Error::TabDenied);
        }
        let key = frozen.connection.target_key(&tab)?;
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::InvalidInput);
        }
        {
            let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(Error::Disconnected);
            }
            if state.claims.get(&key).is_some_and(|claim| claim != &run.id) {
                return Err(Error::Busy);
            }
            state.claims.insert(key, run.id.clone());
        }
        run.touched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tab.clone());
        let connection = frozen.connection.clone();
        let cancel = run.cancel.clone();
        let handle = tokio::spawn(async move { connection.invoke(&tab, command, &cancel).await });
        let job = async move { handle.await.unwrap_or(Err(Error::ExecutionFailed)) }
            .boxed()
            .shared();
        *run.job.lock().unwrap_or_else(|e| e.into_inner()) = Some(Job::Invoke(job.clone()));
        let result = job.await;
        run.job.lock().unwrap_or_else(|e| e.into_inner()).take();
        result
    }
    async fn settle(&self) -> Result<(), Error> {
        self.0.settle().await
    }
    async fn finish(&self) -> Result<(), Error> {
        self.0.finish().await
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        if self.0.phase.load(Ordering::Acquire) != FINISHED {
            self.0.orphaned.store(true, Ordering::Release);
            self.0.cancel.cancel();
            self.0.wake.notify_one();
        }
    }
}
async fn custodian(run: Arc<Run>, _runtime: Arc<Runtime>) {
    loop {
        if run.phase.load(Ordering::Acquire) == FINISHED {
            return;
        }
        if run.orphaned.load(Ordering::Acquire) {
            break;
        }
        run.wake.notified().await;
    }
    let mut retry_seconds = 1;
    loop {
        if run.settle().await.is_ok() && run.finish().await.is_ok() {
            return;
        }
        tracing::warn!("System browser orphan cleanup retained; exact tab detach will be retried");
        tokio::time::sleep(std::time::Duration::from_secs(retry_seconds)).await;
        retry_seconds = (retry_seconds * 2).min(30);
    }
}
impl Runtime {
    pub(super) async fn shutdown(&self) -> Result<(), Error> {
        let runs = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state.runs.values().cloned().collect::<Vec<_>>()
        };
        for run in &runs {
            run.cancel.cancel();
        }
        let mut failed = None;
        for run in runs {
            if let Err(error) = run.settle().await {
                failed = Some(error);
            }
        }
        failed.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
