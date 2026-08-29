use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use nomifun_agent_contracts::{
    AgentSessionId, RuntimeBindingContract, RuntimeBindingId, RuntimeCommand,
    RuntimeCommandContext, RuntimeSessionDisposeParams,
};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::client::{ClientLimits, CodexRuntimeClient};
use crate::credential::InheritedHandleCredential;
use crate::error::RuntimeError;
use crate::native_action::RuntimeIngressPort;
use crate::process::{
    ManagedRuntimeProcess, ProcessTreeDisposeReport, RuntimeProcessConfig,
};
use crate::profile::PinnedRuntimeProfile;
use crate::release::{RuntimeHelloExpectation, RuntimeReleaseDescriptor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposeRpcOutcome {
    Acked,
    TimedOut,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeDisposeReport {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
    pub rpc: DisposeRpcOutcome,
    pub process_tree: ProcessTreeDisposeReport,
}

pub struct RuntimeLaunchRequest {
    pub process: RuntimeProcessConfig,
    pub credential: InheritedHandleCredential,
    pub release: RuntimeReleaseDescriptor,
    pub hello_expectation: RuntimeHelloExpectation,
    pub profile: PinnedRuntimeProfile,
    pub open_command: RuntimeCommand,
    pub ingress: Arc<dyn RuntimeIngressPort>,
    pub client_limits: ClientLimits,
    pub dispose_timeout: Duration,
}

pub struct CodexRuntimeSupervisor {
    sessions: Mutex<BTreeMap<RuntimeBindingId, Arc<ManagedRuntimeSession>>>,
    opening: Mutex<BTreeSet<RuntimeBindingId>>,
}

impl Default for CodexRuntimeSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexRuntimeSupervisor {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(BTreeMap::new()),
            opening: Mutex::new(BTreeSet::new()),
        }
    }

    pub async fn launch(
        &self,
        request: RuntimeLaunchRequest,
    ) -> Result<Arc<ManagedRuntimeSession>, RuntimeError> {
        let context = open_context(&request.open_command)?.clone();
        {
            let sessions = self.sessions.lock().await;
            if sessions.contains_key(&context.runtime_binding_id) {
                return Err(RuntimeError::SessionAlreadyExists);
            }
        }
        {
            let mut opening = self.opening.lock().await;
            if !opening.insert(context.runtime_binding_id.clone()) {
                return Err(RuntimeError::SessionAlreadyExists);
            }
        }

        match ManagedRuntimeSession::launch(request, context.clone()).await {
            Ok(session) => {
                self.sessions
                    .lock()
                    .await
                    .insert(context.runtime_binding_id.clone(), Arc::clone(&session));
                self.opening.lock().await.remove(&context.runtime_binding_id);
                Ok(session)
            }
            Err(error) => {
                self.opening.lock().await.remove(&context.runtime_binding_id);
                Err(error)
            }
        }
    }

    pub async fn session(
        &self,
        runtime_binding_id: &RuntimeBindingId,
    ) -> Option<Arc<ManagedRuntimeSession>> {
        self.sessions.lock().await.get(runtime_binding_id).cloned()
    }

    pub async fn dispose(
        &self,
        params: RuntimeSessionDisposeParams,
    ) -> Result<RuntimeDisposeReport, RuntimeError> {
        let session = self
            .session(&params.runtime_binding_id)
            .await
            .ok_or(RuntimeError::SessionNotFound)?;
        session.dispose(params).await
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    pub async fn evict_disposed(&self, runtime_binding_id: &RuntimeBindingId) -> bool {
        let Some(session) = self.session(runtime_binding_id).await else {
            return false;
        };
        if !session.is_disposed().await {
            return false;
        }
        self.sessions.lock().await.remove(runtime_binding_id);
        true
    }
}

pub struct ManagedRuntimeSession {
    binding: RuntimeBindingContract,
    client: Arc<CodexRuntimeClient>,
    process: Mutex<ManagedRuntimeProcess>,
    ingress_cancellation: CancellationToken,
    ingress_task: Mutex<Option<JoinHandle<Result<(), RuntimeError>>>>,
    dispose_state: Mutex<DisposeState>,
    dispose_changed: Notify,
    dispose_timeout: Duration,
}

impl std::fmt::Debug for ManagedRuntimeSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedRuntimeSession")
            .field("binding", &self.binding)
            .field("dispose_timeout", &self.dispose_timeout)
            .finish_non_exhaustive()
    }
}

impl ManagedRuntimeSession {
    pub fn binding(&self) -> &RuntimeBindingContract {
        &self.binding
    }

    pub fn client(&self) -> &Arc<CodexRuntimeClient> {
        &self.client
    }

    pub async fn is_disposed(&self) -> bool {
        let state = self.dispose_state.lock().await;
        matches!(&*state, DisposeState::Disposed(_))
    }

    pub async fn dispose(
        &self,
        params: RuntimeSessionDisposeParams,
    ) -> Result<RuntimeDisposeReport, RuntimeError> {
        if params.agent_session_id != self.binding.agent_session_id
            || params.runtime_binding_id != self.binding.runtime_binding_id
        {
            return Err(RuntimeError::Protocol(
                "dispose identity differs from the managed runtime session".to_owned(),
            ));
        }

        loop {
            let notified = self.dispose_changed.notified();
            tokio::pin!(notified);
            let _ = notified.as_mut().enable();
            let leader = {
                let mut state = self.dispose_state.lock().await;
                match &*state {
                    DisposeState::Live => {
                        *state = DisposeState::Disposing;
                        true
                    }
                    DisposeState::Disposing => false,
                    DisposeState::Disposed(report) => return Ok(report.clone()),
                }
            };
            if !leader {
                notified.as_mut().await;
                continue;
            }

            let result = self.perform_dispose(params.clone()).await;
            let mut state = self.dispose_state.lock().await;
            match &result {
                Ok(report) => *state = DisposeState::Disposed(report.clone()),
                Err(_) => *state = DisposeState::Live,
            }
            drop(state);
            self.dispose_changed.notify_waiters();
            return result;
        }
    }

    async fn launch(
        request: RuntimeLaunchRequest,
        context: RuntimeCommandContext,
    ) -> Result<Arc<Self>, RuntimeError> {
        request.process.validate_release(&request.release)?;
        if request.hello_expectation.runtime_release_digest != request.release.payload_digest {
            return Err(RuntimeError::ReleaseManifest(
                "runtime hello expectation is not bound to the selected release input".to_owned(),
            ));
        }
        if let RuntimeCommand::Create(params) = &request.open_command {
            request.profile.validate_create(params)?;
        } else if context.runtime_profile_digest != request.profile.profile_digest {
            return Err(RuntimeError::Protocol(
                "open command profile digest does not match the compiled profile".to_owned(),
            ));
        }

        let spawned = ManagedRuntimeProcess::spawn(&request.process)?;
        let crate::process::SpawnedRuntimeProcess {
            mut process,
            stdin,
            stdout,
            credential_channel,
            credential_handle,
        } = spawned;
        let client = match CodexRuntimeClient::connect(
            stdout,
            stdin,
            credential_channel,
            credential_handle,
            request.credential,
            request.hello_expectation,
            request.client_limits,
        )
        .await
        {
            Ok(client) => client,
            Err(error) => {
                let _ = process.dispose_tree().await;
                return Err(error);
            }
        };

        let hello = match client.hello() {
            Some(hello) => hello,
            None => {
                client.close().await;
                let _ = process.dispose_tree().await;
                return Err(RuntimeError::HelloRejected(
                    "runtime client completed without hello".to_owned(),
                ));
            }
        };
        if let Err(error) = request.profile.validate_hello(&hello) {
            client.close().await;
            let _ = process.dispose_tree().await;
            return Err(error);
        }

        let ingress_cancellation = CancellationToken::new();
        let ingress_task = tokio::spawn({
            let client = Arc::clone(&client);
            let ingress = request.ingress;
            let cancellation = ingress_cancellation.clone();
            async move {
                let result = client.serve_ingress(ingress, cancellation).await;
                if result.is_err() {
                    client.close().await;
                }
                result
            }
        });

        let binding = match client.open(&request.open_command).await {
            Ok(binding) => binding,
            Err(error) => {
                ingress_cancellation.cancel();
                ingress_task.abort();
                let _ = ingress_task.await;
                client.close().await;
                let _ = process.dispose_tree().await;
                return Err(error);
            }
        };
        if let Err(error) = validate_open_binding(
            &binding,
            &context,
            &request.profile,
            &hello,
        ) {
            ingress_cancellation.cancel();
            ingress_task.abort();
            let _ = ingress_task.await;
            client.close().await;
            let _ = process.dispose_tree().await;
            return Err(error);
        }

        Ok(Arc::new(Self {
            binding,
            client,
            process: Mutex::new(process),
            ingress_cancellation,
            ingress_task: Mutex::new(Some(ingress_task)),
            dispose_state: Mutex::new(DisposeState::Live),
            dispose_changed: Notify::new(),
            dispose_timeout: request.dispose_timeout,
        }))
    }

    async fn perform_dispose(
        &self,
        params: RuntimeSessionDisposeParams,
    ) -> Result<RuntimeDisposeReport, RuntimeError> {
        let command = RuntimeCommand::SessionDispose(params.clone());
        let rpc = match tokio::time::timeout(self.dispose_timeout, self.client.dispose(&command))
            .await
        {
            Ok(Ok(ack))
                if ack.agent_session_id == params.agent_session_id
                    && ack.runtime_binding_id == params.runtime_binding_id
                    && ack.disposed =>
            {
                DisposeRpcOutcome::Acked
            }
            Ok(Ok(_)) => DisposeRpcOutcome::Failed(
                "session_dispose ACK identity or disposed flag mismatch".to_owned(),
            ),
            Ok(Err(error)) => DisposeRpcOutcome::Failed(error.to_string()),
            Err(_) => DisposeRpcOutcome::TimedOut,
        };

        self.ingress_cancellation.cancel();
        self.client.close().await;
        if let Some(task) = self.ingress_task.lock().await.take() {
            task.abort();
            let _ = task.await;
        }

        let process_tree = tokio::time::timeout(self.dispose_timeout, async {
            self.process.lock().await.dispose_tree().await
        })
        .await
        .map_err(|_| RuntimeError::Timeout("runtime process-tree dispose".to_owned()))??;

        Ok(RuntimeDisposeReport {
            agent_session_id: params.agent_session_id,
            runtime_binding_id: params.runtime_binding_id,
            rpc,
            process_tree,
        })
    }
}

#[derive(Clone, Debug)]
enum DisposeState {
    Live,
    Disposing,
    Disposed(RuntimeDisposeReport),
}

fn open_context(command: &RuntimeCommand) -> Result<&RuntimeCommandContext, RuntimeError> {
    match command {
        RuntimeCommand::Create(params) => Ok(&params.context),
        RuntimeCommand::Resume(params) => Ok(&params.context),
        RuntimeCommand::Fork(params) => Ok(&params.child_context),
        _ => Err(RuntimeError::Protocol(
            "runtime launch requires create, resume, or fork".to_owned(),
        )),
    }
}

fn validate_open_binding(
    binding: &RuntimeBindingContract,
    context: &RuntimeCommandContext,
    profile: &PinnedRuntimeProfile,
    hello: &nomifun_agent_contracts::RuntimeHelloPayload,
) -> Result<(), RuntimeError> {
    if binding.runtime_binding_id != context.runtime_binding_id
        || binding.agent_session_id != context.agent_session_id
        || binding.resolved_snapshot_ref != context.resolved_snapshot_ref
        || binding.runtime_profile_digest != context.runtime_profile_digest
        || binding.active_set_generation != context.active_set_generation
        || binding.profile_kind != profile.kind
        || binding.runtime_release_digest != hello.runtime_release_digest
        || binding.runtime_build_digest != hello.runtime_build_digest
        || binding.protocol_version != hello.protocol_version
    {
        return Err(RuntimeError::Protocol(
            "runtime open ACK does not match command context, profile, and hello".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::{
        DigestHex, EventId, ResolvedSnapshotId, ResolvedSnapshotRef, RuntimeProfileKind,
        RuntimeTarget, VersionString,
    };

    use super::*;

    #[test]
    fn binding_validation_checks_hello_and_frozen_context() {
        let context = RuntimeCommandContext {
            agent_session_id: AgentSessionId::from("session"),
            runtime_binding_id: RuntimeBindingId::from("binding"),
            operation_id: nomifun_agent_contracts::OperationId::from("operation"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot"),
                snapshot_digest: DigestHex::from("snapshot-digest"),
            },
            runtime_profile_digest: DigestHex::from("profile-digest"),
            active_set_generation: 4,
        };
        let profile = PinnedRuntimeProfile {
            kind: RuntimeProfileKind::ManagedMinimal,
            runtime_protocol_version: VersionString::from("1.0.0"),
            profile_digest: context.runtime_profile_digest.clone(),
            enabled_runtime_features: BTreeSet::new(),
            initial_capabilities: BTreeSet::new(),
            on_demand_capabilities: BTreeSet::new(),
            typed_resource_bindings: Vec::new(),
        };
        let hello = nomifun_agent_contracts::RuntimeHelloPayload {
            runtime_release_digest: DigestHex::from("release"),
            runtime_build_digest: DigestHex::from("build"),
            fork_commit: crate::release::FROZEN_CODEX_COMMIT.to_owned(),
            tracked_upstream_commit: crate::release::FROZEN_CODEX_COMMIT.to_owned(),
            protocol_version: VersionString::from("1.0.0"),
            protocol_schema_digest: DigestHex::from("schema"),
            runtime_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            supported_profiles: BTreeSet::from([RuntimeProfileKind::ManagedMinimal]),
            native_features: BTreeSet::new(),
            native_actions: BTreeSet::new(),
            full_auto: nomifun_agent_contracts::FullAutoExecutionWire::fixed(),
            rpc_allowlist: nomifun_agent_contracts::RuntimeRpcAllowlist::frozen(),
        };
        let binding = RuntimeBindingContract {
            runtime_binding_id: context.runtime_binding_id.clone(),
            agent_session_id: context.agent_session_id.clone(),
            resolved_snapshot_ref: context.resolved_snapshot_ref.clone(),
            runtime_release_digest: hello.runtime_release_digest.clone(),
            runtime_build_digest: hello.runtime_build_digest.clone(),
            protocol_version: hello.protocol_version.clone(),
            profile_kind: profile.kind,
            runtime_profile_digest: context.runtime_profile_digest.clone(),
            active_set_generation: context.active_set_generation,
            runtime_bound_event_id: EventId::from("runtime-bound"),
            through_seq: 0,
        };
        validate_open_binding(&binding, &context, &profile, &hello).unwrap();

        let mut wrong = binding;
        wrong.runtime_build_digest = DigestHex::from("wrong");
        assert!(validate_open_binding(&wrong, &context, &profile, &hello).is_err());
    }
}
