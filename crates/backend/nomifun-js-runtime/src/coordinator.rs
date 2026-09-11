use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CanonicalErrorCode, RuntimeSwitchDecision, RuntimeSwitchParticipantKind,
    RuntimeSwitchParticipantResult, RuntimeSwitchValidationResult,
};
use tokio::sync::Mutex;

use crate::{
    JavaScriptRuntimeError, ResolvedNodeRuntime, RuntimeAuthority,
    RuntimeCandidate, RuntimeSwitchFence, VersionedRuntimeSelection,
};

pub const ERR_RUNTIME_SWITCH_INTERRUPTED: &str =
    "JAVASCRIPT_RUNTIME_SWITCH_INTERRUPTED";

#[derive(Clone, Debug)]
pub struct BeginRuntimeSwitchCommand {
    pub expected_revision: u64,
    pub owner_user_id: String,
    pub selected_runtime: Option<ResolvedNodeRuntime>,
    pub candidate: RuntimeCandidate,
    pub acknowledge_non_recommended: bool,
}

#[derive(Clone, Debug)]
pub struct DecideRuntimeSwitchCommand {
    pub expected_revision: u64,
    pub candidate: RuntimeCandidate,
    pub decision: RuntimeSwitchDecision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeQuiesceResult {
    pub old_runtime_process_tree_zero: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeParticipantValidation {
    pub foundation_hello_passed: bool,
    pub participants: Vec<RuntimeSwitchParticipantResult>,
}

#[async_trait]
pub trait RuntimeSwitchParticipant: Send + Sync {
    async fn quiesce_and_stop(
        &self,
        owner_user_id: &str,
        selected: Option<&ResolvedNodeRuntime>,
    ) -> Result<RuntimeQuiesceResult, JavaScriptRuntimeError>;

    async fn validate_candidate(
        &self,
        owner_user_id: &str,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<RuntimeParticipantValidation, JavaScriptRuntimeError>;

    /// Prepare every fallible in-memory factory change while the old
    /// selection is still authoritative and the global write fence is held.
    async fn prepare_candidate(
        &self,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<(), JavaScriptRuntimeError>;

    /// Reconcile non-authoritative projections after the selection CAS. A
    /// failure is recorded against the new Runtime; it does not roll back the
    /// committed global selection.
    async fn finalize_candidate(
        &self,
        _candidate: &ResolvedNodeRuntime,
    ) -> Result<(), JavaScriptRuntimeError> {
        Ok(())
    }

    async fn restore_selected(
        &self,
        selected: Option<&ResolvedNodeRuntime>,
    ) -> Result<(), JavaScriptRuntimeError>;
}

#[async_trait]
pub trait RuntimeSwitchCoordinator: Send + Sync {
    async fn begin_switch(
        &self,
        command: BeginRuntimeSwitchCommand,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError>;

    async fn decide_switch(
        &self,
        command: DecideRuntimeSwitchCommand,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError>;

    async fn recover_interrupted_switch(
        &self,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError>;
}

struct PendingRuntimeSwitch {
    selected: Option<ResolvedNodeRuntime>,
    candidate: ResolvedNodeRuntime,
    phase: PendingRuntimeSwitchPhase,
    _fence: RuntimeSwitchFence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingRuntimeSwitchPhase {
    AwaitingDecision,
    RecoveryRequired,
}

pub struct CoordinatedRuntimeSwitch {
    authority: Arc<RuntimeAuthority>,
    participants: Vec<Arc<dyn RuntimeSwitchParticipant>>,
    operation: Mutex<()>,
    pending: Mutex<Option<PendingRuntimeSwitch>>,
}

impl std::fmt::Debug for CoordinatedRuntimeSwitch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoordinatedRuntimeSwitch")
            .field("participant_count", &self.participants.len())
            .finish_non_exhaustive()
    }
}

impl CoordinatedRuntimeSwitch {
    pub fn new(
        authority: Arc<RuntimeAuthority>,
        participants: Vec<Arc<dyn RuntimeSwitchParticipant>>,
    ) -> Result<Arc<Self>, JavaScriptRuntimeError> {
        if participants.is_empty() {
            return Err(JavaScriptRuntimeError::SwitchNotCovered(
                "Runtime switch coordinator has no participants".to_owned(),
            ));
        }
        Ok(Arc::new(Self {
            authority,
            participants,
            operation: Mutex::new(()),
            pending: Mutex::new(None),
        }))
    }

    async fn restore_all(
        &self,
        selected: Option<&ResolvedNodeRuntime>,
    ) -> Result<(), JavaScriptRuntimeError> {
        let mut first_error = None;
        for participant in self.participants.iter().rev() {
            if let Err(error) = participant.restore_selected(selected).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn prepare_all(
        &self,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<(), JavaScriptRuntimeError> {
        for participant in &self.participants {
            participant.prepare_candidate(candidate).await?;
        }
        Ok(())
    }

    async fn finalize_all(
        &self,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<(), JavaScriptRuntimeError> {
        let mut first_error = None;
        for participant in &self.participants {
            if let Err(error) = participant.finalize_candidate(candidate).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn restore_and_abort(
        &self,
        versioned: &VersionedRuntimeSelection,
        mut pending: PendingRuntimeSwitch,
        last_error: Option<CanonicalErrorCode>,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        if let Err(error) = self.restore_all(pending.selected.as_ref()).await {
            pending.phase = PendingRuntimeSwitchPhase::RecoveryRequired;
            *self.pending.lock().await = Some(pending);
            return Err(error);
        }
        match self
            .authority
            .manager()
            .abort_pending(
                versioned.revision,
                pending
                    .candidate
                    .fingerprint
                    .runtime_installation_id
                    .as_ref(),
                pending
                    .candidate
                    .fingerprint
                    .executable_digest
                    .as_ref(),
                last_error,
            )
            .await
        {
            Ok(aborted) => Ok(aborted),
            Err(error) => {
                pending.phase = PendingRuntimeSwitchPhase::RecoveryRequired;
                *self.pending.lock().await = Some(pending);
                Err(error)
            }
        }
    }

    async fn fail_before_commit(
        &self,
        versioned: &VersionedRuntimeSelection,
        pending: PendingRuntimeSwitch,
        error: JavaScriptRuntimeError,
    ) -> JavaScriptRuntimeError {
        match self
            .restore_and_abort(
                versioned,
                pending,
                Some(CanonicalErrorCode::from(error.code())),
            )
            .await
        {
            Ok(_) => error,
            Err(recovery) => recovery,
        }
    }

    async fn validate_participants(
        &self,
        owner_user_id: &str,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<RuntimeParticipantValidation, JavaScriptRuntimeError> {
        let mut results = Vec::new();
        let mut foundation_hello_passed = true;
        for participant in &self.participants {
            let validation = participant
                .validate_candidate(owner_user_id, candidate)
                .await?;
            foundation_hello_passed &= validation.foundation_hello_passed;
            results.extend(validation.participants);
        }
        let foundation_count = results
            .iter()
            .filter(|result| {
                result.kind == RuntimeSwitchParticipantKind::BuildFoundation
            })
            .count();
        if foundation_count != 1 {
            return Err(JavaScriptRuntimeError::SwitchNotCovered(format!(
                "Runtime switch requires exactly one Build Foundation result, observed {foundation_count}"
            )));
        }
        Ok(RuntimeParticipantValidation {
            foundation_hello_passed,
            participants: results,
        })
    }
}

#[async_trait]
impl RuntimeSwitchCoordinator for CoordinatedRuntimeSwitch {
    async fn begin_switch(
        &self,
        command: BeginRuntimeSwitchCommand,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let _operation = self.operation.lock().await;
        if self.pending.lock().await.is_some() {
            return Err(JavaScriptRuntimeError::SwitchAlreadyPending);
        }
        let candidate = ResolvedNodeRuntime {
            fingerprint: command.candidate.fingerprint,
            executable_path: command.candidate.executable_path,
        };
        let fence = self.authority.acquire_switch_fence().await;
        let pending_record = self
            .authority
            .manager()
            .begin_pending(
                command.expected_revision,
                candidate.fingerprint.clone(),
                candidate.executable_path.clone(),
                command.acknowledge_non_recommended,
            )
            .await?;
        let mut switch = PendingRuntimeSwitch {
            selected: command.selected_runtime,
            candidate,
            phase: PendingRuntimeSwitchPhase::RecoveryRequired,
            _fence: fence,
        };
        let mut old_runtime_process_tree_zero = true;
        for participant in &self.participants {
            match participant
                .quiesce_and_stop(
                    &command.owner_user_id,
                    switch.selected.as_ref(),
                )
                .await
            {
                Ok(proof) => {
                    old_runtime_process_tree_zero &=
                        proof.old_runtime_process_tree_zero;
                }
                Err(error) => {
                    return Err(
                        self.fail_before_commit(
                            &pending_record,
                            switch,
                            error,
                        )
                        .await,
                    );
                }
            }
        }

        let results = match self
            .validate_participants(
                &command.owner_user_id,
                &switch.candidate,
            )
            .await
        {
            Ok(results) => results,
            Err(error) => {
                return Err(
                    self.fail_before_commit(
                        &pending_record,
                        switch,
                        error,
                    )
                    .await,
                );
            }
        };
        let validation = RuntimeSwitchValidationResult {
            candidate: switch.candidate.fingerprint.clone(),
            foundation_hello_passed: results.foundation_hello_passed,
            old_runtime_process_tree_zero,
            participants: results.participants,
            completed_at_ms: now_ms(),
        };
        let validated = match self
            .authority
            .manager()
            .record_validation(
                pending_record.revision,
                switch
                    .candidate
                    .fingerprint
                    .runtime_installation_id
                    .as_ref(),
                switch
                    .candidate
                    .fingerprint
                    .executable_digest
                    .as_ref(),
                validation.clone(),
            )
            .await
        {
            Ok(validated) => validated,
            Err(error) => {
                return Err(
                    self.fail_before_commit(
                        &pending_record,
                        switch,
                        error,
                    )
                    .await,
                );
            }
        };

        if validation.requires_user_decision() {
            switch.phase = PendingRuntimeSwitchPhase::AwaitingDecision;
            *self.pending.lock().await = Some(switch);
            return Ok(validated);
        }

        if let Err(error) = self.prepare_all(&switch.candidate).await {
            return Err(
                self.fail_before_commit(&validated, switch, error)
                    .await,
            );
        }
        let committed = self
            .authority
            .manager()
            .commit_pending(
                validated.revision,
                switch
                    .candidate
                    .fingerprint
                    .runtime_installation_id
                    .as_ref(),
                switch
                    .candidate
                    .fingerprint
                    .executable_digest
                    .as_ref(),
            )
            .await;
        let committed = match committed {
            Ok(committed) => committed,
            Err(error) => {
                return Err(
                    self.fail_before_commit(&validated, switch, error)
                        .await,
                );
            }
        };
        // The durable selection is now authoritative. Release the global
        // write fence before reconciling non-authoritative projections: a
        // finalizer may legitimately read the committed Runtime, and keeping
        // the fence here would self-deadlock that read.
        let committed_candidate = switch.candidate.clone();
        drop(switch);
        if let Err(error) = self.finalize_all(&committed_candidate).await {
            let _ = self
                .authority
                .manager()
                .record_error(
                    committed.revision,
                    CanonicalErrorCode::from(error.code()),
                )
                .await;
            return Err(error);
        }
        Ok(committed)
    }

    async fn decide_switch(
        &self,
        command: DecideRuntimeSwitchCommand,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let _operation = self.operation.lock().await;
        let current = self.authority.manager().snapshot().await?;
        if current.revision != command.expected_revision {
            return Err(JavaScriptRuntimeError::SelectionRevisionConflict {
                expected: command.expected_revision,
                observed: current.revision,
            });
        }
        let stored_candidate = current
            .selection
            .pending_candidate
            .as_ref()
            .ok_or(JavaScriptRuntimeError::NoPendingCandidate)?;
        if stored_candidate != &command.candidate.fingerprint
            || current.pending_candidate_executable_path.as_deref()
                != Some(command.candidate.executable_path.as_path())
        {
            return Err(JavaScriptRuntimeError::CandidateStale);
        }
        let pending = self
            .pending
            .lock()
            .await
            .take()
            .ok_or(JavaScriptRuntimeError::NoPendingCandidate)?;
        if pending.phase != PendingRuntimeSwitchPhase::AwaitingDecision {
            *self.pending.lock().await = Some(pending);
            return Err(JavaScriptRuntimeError::SwitchBusy(
                "Runtime switch recovery is still required".to_owned(),
            ));
        }
        let observed = ResolvedNodeRuntime {
            fingerprint: command.candidate.fingerprint,
            executable_path: command.candidate.executable_path,
        };
        if pending.candidate != observed {
            *self.pending.lock().await = Some(pending);
            return Err(JavaScriptRuntimeError::CandidateStale);
        }

        match command.decision {
            RuntimeSwitchDecision::CommitCandidate => {
                if let Err(error) =
                    self.prepare_all(&pending.candidate).await
                {
                    return Err(
                        self.fail_before_commit(&current, pending, error)
                            .await,
                    );
                }
                let committed = self
                    .authority
                    .manager()
                    .commit_pending(
                        command.expected_revision,
                        pending
                            .candidate
                            .fingerprint
                            .runtime_installation_id
                            .as_ref(),
                        pending
                            .candidate
                            .fingerprint
                            .executable_digest
                            .as_ref(),
                    )
                    .await;
                let committed = match committed {
                    Ok(committed) => committed,
                    Err(error) => {
                        return Err(
                            self.fail_before_commit(
                                &current,
                                pending,
                                error,
                            )
                            .await,
                        );
                    }
                };
                // See the automatic-commit path above: finalizers run after
                // the selection CAS and after the write fence is released.
                let committed_candidate = pending.candidate.clone();
                drop(pending);
                if let Err(error) =
                    self.finalize_all(&committed_candidate).await
                {
                    let _ = self
                        .authority
                        .manager()
                        .record_error(
                            committed.revision,
                            CanonicalErrorCode::from(error.code()),
                        )
                        .await;
                    return Err(error);
                }
                Ok(committed)
            }
            RuntimeSwitchDecision::AbortAndRestoreSelected => {
                self.restore_and_abort(&current, pending, None).await
            }
        }
    }

    async fn recover_interrupted_switch(
        &self,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let _operation = self.operation.lock().await;
        if let Some(pending) = self.pending.lock().await.take() {
            if pending.phase == PendingRuntimeSwitchPhase::AwaitingDecision {
                *self.pending.lock().await = Some(pending);
                return self.authority.manager().snapshot().await;
            }
            let current = self.authority.manager().snapshot().await?;
            return self
                .restore_and_abort(
                    &current,
                    pending,
                    Some(CanonicalErrorCode::from(
                        ERR_RUNTIME_SWITCH_INTERRUPTED,
                    )),
                )
                .await;
        }
        let current = self.authority.manager().snapshot().await?;
        let Some(candidate) = current.selection.pending_candidate.as_ref() else {
            return Ok(current);
        };
        if current.pending_candidate_executable_path.is_none() {
            return Err(JavaScriptRuntimeError::Contract(
                "interrupted Runtime candidate has no executable path"
                    .to_owned(),
            ));
        }
        let selected = resolved_selected(&current)?;
        let fence = self.authority.acquire_switch_fence().await;
        self.restore_and_abort(
            &current,
            PendingRuntimeSwitch {
                selected,
                candidate: ResolvedNodeRuntime {
                    fingerprint: candidate.clone(),
                    executable_path: current
                        .pending_candidate_executable_path
                        .clone()
                        .expect("checked above"),
                },
                phase: PendingRuntimeSwitchPhase::RecoveryRequired,
                _fence: fence,
            },
            Some(CanonicalErrorCode::from(
                ERR_RUNTIME_SWITCH_INTERRUPTED,
            )),
        )
            .await
    }
}

fn resolved_selected(
    current: &VersionedRuntimeSelection,
) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
    match (
        current.selection.selected_runtime.clone(),
        current.selected_executable_path.clone(),
    ) {
        (Some(fingerprint), Some(executable_path)) => {
            Ok(Some(ResolvedNodeRuntime {
                fingerprint,
                executable_path,
            }))
        }
        (None, None) => Ok(None),
        _ => Err(JavaScriptRuntimeError::Contract(
            "selected Runtime binding is incomplete".to_owned(),
        )),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};

    use nomifun_agent_contracts::{
        DigestHex, JAVASCRIPT_HOST_PROTOCOL_VERSION,
        JAVASCRIPT_SDK_CONTRACT_VERSION, NodeProbeDisposition,
        NodeRuntimeFingerprint, NodeRuntimeProbeResult,
        NodeRuntimeSourceKind, RuntimeSelectionRecord,
        RuntimeSwitchParticipantOutcome, RuntimeTarget, VersionString,
    };

    use super::*;
    use crate::{
        CommittedRuntimeProvider, JavaScriptWorkKind, NodeDiscoveryRequest,
        NodeProbeCandidate, NodeResolution, NodeRuntimeManager,
        NodeRuntimeProbePort, RuntimeSelectionStore,
        RuntimeSelectionStoreError,
    };

    struct MemoryStore {
        value: Mutex<VersionedRuntimeSelection>,
    }

    impl Default for MemoryStore {
        fn default() -> Self {
            Self {
                value: Mutex::new(VersionedRuntimeSelection::empty()),
            }
        }
    }

    #[async_trait]
    impl RuntimeSelectionStore for MemoryStore {
        async fn load(
            &self,
        ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
            Ok(self.value.lock().await.clone())
        }

        async fn save_cas(
            &self,
            expected_revision: u64,
            selection: &RuntimeSelectionRecord,
            selected_executable_path: Option<&Path>,
            pending_candidate_executable_path: Option<&Path>,
            updated_at_ms: i64,
        ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
            let mut value = self.value.lock().await;
            if value.revision != expected_revision {
                return Err(RuntimeSelectionStoreError::Conflict("stale".into()));
            }
            *value = VersionedRuntimeSelection {
                selection: selection.clone(),
                selected_executable_path:
                    selected_executable_path.map(Path::to_path_buf),
                pending_candidate_executable_path:
                    pending_candidate_executable_path.map(Path::to_path_buf),
                revision: expected_revision + 1,
                updated_at_ms,
            };
            Ok(value.clone())
        }
    }

    struct UnusedProbe;

    #[async_trait]
    impl NodeRuntimeProbePort for UnusedProbe {
        async fn resolve(
            &self,
            _request: NodeDiscoveryRequest,
        ) -> Result<NodeResolution, JavaScriptRuntimeError> {
            unreachable!()
        }

        async fn probe(
            &self,
            _candidate: &NodeProbeCandidate,
        ) -> NodeRuntimeProbeResult {
            unreachable!()
        }
    }

    struct Participant {
        outcome: RuntimeSwitchParticipantOutcome,
        restore_fails: Option<Arc<AtomicBool>>,
        finalize_authority: Option<Arc<RuntimeAuthority>>,
    }

    #[async_trait]
    impl RuntimeSwitchParticipant for Participant {
        async fn quiesce_and_stop(
            &self,
            _owner_user_id: &str,
            _selected: Option<&ResolvedNodeRuntime>,
        ) -> Result<RuntimeQuiesceResult, JavaScriptRuntimeError> {
            Ok(RuntimeQuiesceResult {
                old_runtime_process_tree_zero: true,
            })
        }

        async fn validate_candidate(
            &self,
            _owner_user_id: &str,
            _candidate: &ResolvedNodeRuntime,
        ) -> Result<RuntimeParticipantValidation, JavaScriptRuntimeError>
        {
            Ok(RuntimeParticipantValidation {
                foundation_hello_passed: true,
                participants: vec![RuntimeSwitchParticipantResult {
                    kind: RuntimeSwitchParticipantKind::BuildFoundation,
                    owner_id: "build-foundation".to_owned(),
                    outcome: self.outcome,
                    error_code: (self.outcome
                        == RuntimeSwitchParticipantOutcome::Failed)
                        .then(|| CanonicalErrorCode::from("BUILD_FAILED")),
                }],
            })
        }

        async fn prepare_candidate(
            &self,
            _candidate: &ResolvedNodeRuntime,
        ) -> Result<(), JavaScriptRuntimeError> {
            Ok(())
        }

        async fn finalize_candidate(
            &self,
            _candidate: &ResolvedNodeRuntime,
        ) -> Result<(), JavaScriptRuntimeError> {
            if let Some(authority) = self.finalize_authority.as_ref() {
                let lease = authority
                    .acquire_use(JavaScriptWorkKind::BuildHost)
                    .await?;
                drop(lease);
            }
            Ok(())
        }

        async fn restore_selected(
            &self,
            _selected: Option<&ResolvedNodeRuntime>,
        ) -> Result<(), JavaScriptRuntimeError> {
            if self
                .restore_fails
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire))
            {
                Err(JavaScriptRuntimeError::SwitchNotCovered(
                    "injected restore failure".to_owned(),
                ))
            } else {
                Ok(())
            }
        }
    }

    fn candidate(digest: char) -> RuntimeCandidate {
        RuntimeCandidate {
            fingerprint: NodeRuntimeFingerprint {
                runtime_installation_id: format!("node-{digest}").into(),
                source_kind: NodeRuntimeSourceKind::Managed,
                node_version: VersionString::from("24.8.0"),
                node_major: 24,
                runtime_target: RuntimeTarget::from(crate::current_runtime_target()),
                executable_digest: DigestHex::from(
                    digest.to_string().repeat(64),
                ),
                javascript_host_protocol_version:
                    JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                javascript_sdk_contract_version:
                    JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            },
            executable_path: std::env::temp_dir()
                .join(format!("nomifun-js-runtime-coordinator-{digest}"))
                .join(if cfg!(windows) { "node.exe" } else { "node" }),
            disposition: NodeProbeDisposition::CompatibleRecommended,
        }
    }

    fn coordinator(
        outcome: RuntimeSwitchParticipantOutcome,
    ) -> Arc<CoordinatedRuntimeSwitch> {
        let manager = Arc::new(NodeRuntimeManager::new(Arc::new(
            MemoryStore::default(),
        )));
        let authority =
            RuntimeAuthority::new(manager, Arc::new(UnusedProbe));
        CoordinatedRuntimeSwitch::new(
            authority,
            vec![Arc::new(Participant {
                outcome,
                restore_fails: None,
                finalize_authority: None,
            })],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn all_passed_commits_after_pending_and_validation_revisions() {
        let coordinator =
            coordinator(RuntimeSwitchParticipantOutcome::Passed);
        let candidate = candidate('a');
        let committed = coordinator
            .begin_switch(BeginRuntimeSwitchCommand {
                expected_revision: 0,
                owner_user_id: "owner".to_owned(),
                selected_runtime: None,
                candidate: candidate.clone(),
                acknowledge_non_recommended: false,
            })
            .await
            .unwrap();
        assert_eq!(committed.revision, 3);
        assert_eq!(
            committed.selection.selected_runtime,
            Some(candidate.fingerprint)
        );
        assert!(committed.selection.pending_candidate.is_none());
    }

    #[tokio::test]
    async fn committed_selection_releases_write_fence_before_finalizers() {
        let manager = Arc::new(NodeRuntimeManager::new(Arc::new(
            MemoryStore::default(),
        )));
        let authority =
            RuntimeAuthority::new(manager, Arc::new(UnusedProbe));
        let coordinator = CoordinatedRuntimeSwitch::new(
            Arc::clone(&authority),
            vec![Arc::new(Participant {
                outcome: RuntimeSwitchParticipantOutcome::Passed,
                restore_fails: None,
                finalize_authority: Some(authority),
            })],
        )
        .unwrap();
        let committed = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            coordinator.begin_switch(BeginRuntimeSwitchCommand {
                expected_revision: 0,
                owner_user_id: "owner".to_owned(),
                selected_runtime: None,
                candidate: candidate('a'),
                acknowledge_non_recommended: false,
            }),
        )
        .await
        .expect("finalizer must not self-deadlock on committed Runtime")
        .unwrap();
        assert_eq!(committed.revision, 3);
    }

    #[tokio::test]
    async fn partial_failure_stays_pending_until_explicit_abort() {
        let coordinator =
            coordinator(RuntimeSwitchParticipantOutcome::NotCovered);
        let candidate_a = candidate('a');
        let pending = coordinator
            .begin_switch(BeginRuntimeSwitchCommand {
                expected_revision: 0,
                owner_user_id: "owner".to_owned(),
                selected_runtime: None,
                candidate: candidate_a.clone(),
                acknowledge_non_recommended: false,
            })
            .await
            .unwrap();
        assert_eq!(pending.revision, 2);
        assert!(pending.selection.validation_result.is_some());

        let aborted = coordinator
            .decide_switch(DecideRuntimeSwitchCommand {
                expected_revision: pending.revision,
                candidate: candidate_a,
                decision: RuntimeSwitchDecision::AbortAndRestoreSelected,
            })
            .await
            .unwrap();
        assert_eq!(aborted.revision, 3);
        assert!(aborted.selection.selected_runtime.is_none());
        assert!(aborted.selection.pending_candidate.is_none());
    }

    #[tokio::test]
    async fn stale_decision_keeps_the_exact_pending_switch() {
        let coordinator =
            coordinator(RuntimeSwitchParticipantOutcome::NotCovered);
        let candidate_a = candidate('a');
        let pending = coordinator
            .begin_switch(BeginRuntimeSwitchCommand {
                expected_revision: 0,
                owner_user_id: "owner".to_owned(),
                selected_runtime: None,
                candidate: candidate_a.clone(),
                acknowledge_non_recommended: false,
            })
            .await
            .unwrap();
        let error = coordinator
            .decide_switch(DecideRuntimeSwitchCommand {
                expected_revision: pending.revision,
                candidate: candidate('b'),
                decision: RuntimeSwitchDecision::CommitCandidate,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, JavaScriptRuntimeError::CandidateStale));

        let aborted = coordinator
            .decide_switch(DecideRuntimeSwitchCommand {
                expected_revision: pending.revision,
                candidate: candidate_a,
                decision: RuntimeSwitchDecision::AbortAndRestoreSelected,
            })
            .await
            .unwrap();
        assert!(aborted.selection.pending_candidate.is_none());
    }

    #[tokio::test]
    async fn failed_restore_retains_the_write_fence_until_recovery() {
        let manager = Arc::new(NodeRuntimeManager::new(Arc::new(
            MemoryStore::default(),
        )));
        let selected_candidate = candidate('a');
        let selected = manager
            .select_initial(
                0,
                selected_candidate.fingerprint.clone(),
                selected_candidate.executable_path.clone(),
            )
            .await
            .unwrap();
        let authority =
            RuntimeAuthority::new(manager, Arc::new(UnusedProbe));
        let restore_fails = Arc::new(AtomicBool::new(true));
        let coordinator = CoordinatedRuntimeSwitch::new(
            Arc::clone(&authority),
            vec![Arc::new(Participant {
                outcome: RuntimeSwitchParticipantOutcome::NotCovered,
                restore_fails: Some(Arc::clone(&restore_fails)),
                finalize_authority: None,
            })],
        )
        .unwrap();
        let replacement = candidate('b');
        let pending = coordinator
            .begin_switch(BeginRuntimeSwitchCommand {
                expected_revision: 1,
                owner_user_id: "owner".to_owned(),
                selected_runtime: Some(selected.clone()),
                candidate: replacement.clone(),
                acknowledge_non_recommended: false,
            })
            .await
            .unwrap();
        let error = coordinator
            .decide_switch(DecideRuntimeSwitchCommand {
                expected_revision: pending.revision,
                candidate: replacement,
                decision: RuntimeSwitchDecision::AbortAndRestoreSelected,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            JavaScriptRuntimeError::SwitchNotCovered(_)
        ));

        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(20),
            authority.acquire_use(JavaScriptWorkKind::BuildHost),
        )
        .await;
        assert!(
            blocked.is_err(),
            "failed restore must keep the global write fence"
        );

        restore_fails.store(false, Ordering::Release);
        let recovered = coordinator
            .recover_interrupted_switch()
            .await
            .unwrap();
        assert!(recovered.selection.pending_candidate.is_none());
        let lease = authority
            .acquire_use(JavaScriptWorkKind::BuildHost)
            .await
            .unwrap();
        assert_eq!(lease.runtime(), &selected);
    }
}
