use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CanonicalErrorCode, NodeRuntimeFingerprint, RuntimeInstallationId,
    RuntimeSelectionRecord, RuntimeSwitchValidationResult,
};
use tokio::sync::Mutex;

use crate::{JavaScriptRuntimeError, RuntimeSelectionStoreError};
use crate::ResolvedNodeRuntime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedRuntimeSelection {
    pub selection: RuntimeSelectionRecord,
    pub selected_executable_path: Option<PathBuf>,
    pub pending_candidate_executable_path: Option<PathBuf>,
    pub revision: u64,
    pub updated_at_ms: i64,
}

impl VersionedRuntimeSelection {
    pub fn empty() -> Self {
        Self {
            selection: RuntimeSelectionRecord {
                selected_runtime: None,
                pending_candidate: None,
                validation_result: None,
                last_error: None,
                non_recommended_warning_acknowledged: Default::default(),
            },
            selected_executable_path: None,
            pending_candidate_executable_path: None,
            revision: 0,
            updated_at_ms: 0,
        }
    }

    pub fn validate(&self) -> Result<(), JavaScriptRuntimeError> {
        self.selection
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        validate_path_pair(
            self.selection.selected_runtime.as_ref(),
            self.selected_executable_path.as_deref(),
            "selected Runtime",
        )?;
        validate_path_pair(
            self.selection.pending_candidate.as_ref(),
            self.pending_candidate_executable_path.as_deref(),
            "pending Runtime",
        )
    }
}

#[async_trait]
pub trait RuntimeSelectionStore: Send + Sync {
    async fn load(
        &self,
    ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError>;

    async fn save_cas(
        &self,
        expected_revision: u64,
        selection: &RuntimeSelectionRecord,
        selected_executable_path: Option<&Path>,
        pending_candidate_executable_path: Option<&Path>,
        updated_at_ms: i64,
    ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError>;
}

pub struct NodeRuntimeManager {
    store: Arc<dyn RuntimeSelectionStore>,
    mutation_gate: Mutex<()>,
}

impl std::fmt::Debug for NodeRuntimeManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeRuntimeManager")
            .finish_non_exhaustive()
    }
}

impl NodeRuntimeManager {
    pub fn new(store: Arc<dyn RuntimeSelectionStore>) -> Self {
        Self {
            store,
            mutation_gate: Mutex::new(()),
        }
    }

    pub async fn snapshot(
        &self,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let versioned = self.store.load().await?;
        versioned.validate()?;
        Ok(versioned)
    }

    pub async fn selected_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        let current = self.snapshot().await?;
        match (
            current.selection.selected_runtime,
            current.selected_executable_path,
        ) {
            (Some(fingerprint), Some(executable_path)) => {
                Ok(Some(ResolvedNodeRuntime {
                    fingerprint,
                    executable_path,
                }))
            }
            (None, None) => Ok(None),
            _ => Err(JavaScriptRuntimeError::Contract(
                "selected Runtime fingerprint and executable path are incomplete"
                    .to_owned(),
            )),
        }
    }

    pub async fn select_initial(
        &self,
        expected_revision: u64,
        candidate: NodeRuntimeFingerprint,
        executable_path: PathBuf,
    ) -> Result<ResolvedNodeRuntime, JavaScriptRuntimeError> {
        candidate
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        if !executable_path.is_absolute() {
            return Err(JavaScriptRuntimeError::RelativePath(executable_path));
        }
        let _gate = self.mutation_gate.lock().await;
        let current = self.snapshot().await?;
        require_revision(&current, expected_revision)?;
        if current.selection.selected_runtime.is_some()
            || current.selection.pending_candidate.is_some()
        {
            return Err(JavaScriptRuntimeError::SelectedRuntimeMismatch);
        }
        let mut next = current.selection;
        next.selected_runtime = Some(candidate.clone());
        next.last_error = None;
        let saved = save_next(
            &*self.store,
            expected_revision,
            &next,
            Some(&executable_path),
            None,
        )
        .await?;
        if saved.selection.selected_runtime.as_ref() != Some(&candidate)
            || saved.selected_executable_path.as_ref() != Some(&executable_path)
        {
            return Err(JavaScriptRuntimeError::SelectionStore(
                RuntimeSelectionStoreError::Corrupt(
                    "initial Runtime selection returned a non-exact binding"
                        .to_owned(),
                ),
            ));
        }
        Ok(ResolvedNodeRuntime {
            fingerprint: candidate,
            executable_path,
        })
    }

    pub async fn clear_selected(
        &self,
        expected_revision: u64,
        error: CanonicalErrorCode,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let _gate = self.mutation_gate.lock().await;
        let current = self.snapshot().await?;
        require_revision(&current, expected_revision)?;
        let mut next = current.selection;
        next.selected_runtime = None;
        next.validation_result = None;
        next.last_error = Some(error);
        save_next(
            &*self.store,
            expected_revision,
            &next,
            None,
            current.pending_candidate_executable_path.as_deref(),
        )
        .await
    }

    pub async fn begin_pending(
        &self,
        expected_revision: u64,
        candidate: NodeRuntimeFingerprint,
        candidate_executable_path: PathBuf,
        acknowledge_non_recommended: bool,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        candidate
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        if !candidate_executable_path.is_absolute() {
            return Err(JavaScriptRuntimeError::RelativePath(
                candidate_executable_path,
            ));
        }
        let _gate = self.mutation_gate.lock().await;
        let current = self.snapshot().await?;
        require_revision(&current, expected_revision)?;
        if current.selection.pending_candidate.is_some() {
            return Err(JavaScriptRuntimeError::SwitchAlreadyPending);
        }

        let mut next = current.selection;
        if acknowledge_non_recommended {
            next.non_recommended_warning_acknowledged
                .insert(candidate.runtime_installation_id.clone());
        }
        next.last_error = None;
        next.pending_candidate = Some(candidate);
        next.validation_result = None;
        let selected_path = current.selected_executable_path;
        save_next(
            &*self.store,
            expected_revision,
            &next,
            selected_path.as_deref(),
            Some(&candidate_executable_path),
        )
        .await
    }

    pub async fn record_validation(
        &self,
        expected_revision: u64,
        candidate_runtime_id: &str,
        expected_candidate_executable_digest: &str,
        validation: RuntimeSwitchValidationResult,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        validation
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        let _gate = self.mutation_gate.lock().await;
        let current = self.snapshot().await?;
        require_revision(&current, expected_revision)?;
        let candidate = exact_pending(
            &current,
            candidate_runtime_id,
            expected_candidate_executable_digest,
        )?;
        if validation.candidate != *candidate {
            return Err(JavaScriptRuntimeError::CandidateMismatch);
        }

        let mut next = current.selection;
        next.validation_result = Some(validation);
        next.last_error = None;
        save_next(
            &*self.store,
            expected_revision,
            &next,
            current.selected_executable_path.as_deref(),
            current.pending_candidate_executable_path.as_deref(),
        )
        .await
    }

    pub async fn commit_pending(
        &self,
        expected_revision: u64,
        candidate_runtime_id: &str,
        expected_candidate_executable_digest: &str,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let _gate = self.mutation_gate.lock().await;
        let current = self.snapshot().await?;
        require_revision(&current, expected_revision)?;
        let candidate = exact_pending(
            &current,
            candidate_runtime_id,
            expected_candidate_executable_digest,
        )?
        .clone();
        let validation = current
            .selection
            .validation_result
            .as_ref()
            .ok_or(JavaScriptRuntimeError::ValidationRequired)?;
        if validation.candidate != candidate {
            return Err(JavaScriptRuntimeError::CandidateMismatch);
        }
        let candidate_path = current
            .pending_candidate_executable_path
            .clone()
            .ok_or_else(|| {
                JavaScriptRuntimeError::Contract(
                    "pending Runtime has no executable path".to_owned(),
                )
            })?;
        let mut next = current.selection;
        next.selected_runtime = Some(candidate);
        next.pending_candidate = None;
        next.validation_result = None;
        next.last_error = None;
        save_next(
            &*self.store,
            expected_revision,
            &next,
            Some(&candidate_path),
            None,
        )
        .await
    }

    pub async fn abort_pending(
        &self,
        expected_revision: u64,
        candidate_runtime_id: &str,
        expected_candidate_executable_digest: &str,
        last_error: Option<CanonicalErrorCode>,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let _gate = self.mutation_gate.lock().await;
        let current = self.snapshot().await?;
        require_revision(&current, expected_revision)?;
        exact_pending(
            &current,
            candidate_runtime_id,
            expected_candidate_executable_digest,
        )?;
        let mut next = current.selection;
        next.pending_candidate = None;
        next.validation_result = None;
        next.last_error = last_error;
        save_next(
            &*self.store,
            expected_revision,
            &next,
            current.selected_executable_path.as_deref(),
            None,
        )
        .await
    }

    pub async fn record_error(
        &self,
        expected_revision: u64,
        error: CanonicalErrorCode,
    ) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
        let _gate = self.mutation_gate.lock().await;
        let current = self.snapshot().await?;
        require_revision(&current, expected_revision)?;
        let mut next = current.selection;
        next.last_error = Some(error);
        save_next(
            &*self.store,
            expected_revision,
            &next,
            current.selected_executable_path.as_deref(),
            current.pending_candidate_executable_path.as_deref(),
        )
        .await
    }

    pub async fn warning_acknowledged(
        &self,
        runtime_id: &RuntimeInstallationId,
    ) -> Result<bool, JavaScriptRuntimeError> {
        Ok(self
            .snapshot()
            .await?
            .selection
            .non_recommended_warning_acknowledged
            .contains(runtime_id))
    }
}

fn exact_pending<'a>(
    current: &'a VersionedRuntimeSelection,
    candidate_runtime_id: &str,
    expected_candidate_executable_digest: &str,
) -> Result<&'a NodeRuntimeFingerprint, JavaScriptRuntimeError> {
    let candidate = current
        .selection
        .pending_candidate
        .as_ref()
        .ok_or(JavaScriptRuntimeError::NoPendingCandidate)?;
    if candidate.runtime_installation_id.as_ref() != candidate_runtime_id
        || candidate.executable_digest.as_ref()
            != expected_candidate_executable_digest
    {
        return Err(JavaScriptRuntimeError::CandidateMismatch);
    }
    Ok(candidate)
}

fn require_revision(
    current: &VersionedRuntimeSelection,
    expected_revision: u64,
) -> Result<(), JavaScriptRuntimeError> {
    if current.revision == expected_revision {
        Ok(())
    } else {
        Err(JavaScriptRuntimeError::SelectionRevisionConflict {
            expected: expected_revision,
            observed: current.revision,
        })
    }
}

async fn save_next(
    store: &dyn RuntimeSelectionStore,
    expected_revision: u64,
    selection: &RuntimeSelectionRecord,
    selected_executable_path: Option<&Path>,
    pending_candidate_executable_path: Option<&Path>,
) -> Result<VersionedRuntimeSelection, JavaScriptRuntimeError> {
    selection
        .validate()
        .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
    let saved = store
        .save_cas(
            expected_revision,
            selection,
            selected_executable_path,
            pending_candidate_executable_path,
            now_ms(),
        )
        .await?;
    let expected_next = expected_revision.checked_add(1).ok_or_else(|| {
        JavaScriptRuntimeError::Contract(
            "Runtime selection revision overflow".into(),
        )
    })?;
    if saved.revision != expected_next
        || saved.selection != *selection
        || saved.selected_executable_path.as_deref()
            != selected_executable_path
        || saved.pending_candidate_executable_path.as_deref()
            != pending_candidate_executable_path
    {
        return Err(JavaScriptRuntimeError::SelectionStore(
            RuntimeSelectionStoreError::Corrupt(
                "selection store returned a non-exact CAS result".into(),
            ),
        ));
    }
    saved.validate()?;
    Ok(saved)
}

fn validate_path_pair(
    runtime: Option<&NodeRuntimeFingerprint>,
    executable_path: Option<&Path>,
    label: &str,
) -> Result<(), JavaScriptRuntimeError> {
    match (runtime, executable_path) {
        (None, None) => Ok(()),
        (Some(_), Some(path)) if path.is_absolute() => Ok(()),
        (Some(_), Some(path)) => Err(JavaScriptRuntimeError::RelativePath(
            path.to_path_buf(),
        )),
        _ => Err(JavaScriptRuntimeError::Contract(format!(
            "{label} fingerprint and executable path must be stored together"
        ))),
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
    use std::sync::Arc;

    use nomifun_agent_contracts::{
        CanonicalErrorCode, DigestHex, JAVASCRIPT_HOST_PROTOCOL_VERSION,
        JAVASCRIPT_SDK_CONTRACT_VERSION, NodeRuntimeSourceKind,
        RuntimeSwitchParticipantKind, RuntimeSwitchParticipantOutcome,
        RuntimeSwitchParticipantResult, RuntimeTarget, VersionString,
    };
    use tokio::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        value: Mutex<VersionedRuntimeSelection>,
    }

    impl Default for VersionedRuntimeSelection {
        fn default() -> Self {
            Self::empty()
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
                return Err(RuntimeSelectionStoreError::Conflict(
                    "stale revision".into(),
                ));
            }
            let revision = expected_revision
                .checked_add(1)
                .ok_or_else(|| {
                    RuntimeSelectionStoreError::Conflict(
                        "revision overflow".into(),
                    )
                })?;
            *value = VersionedRuntimeSelection {
                selection: selection.clone(),
                selected_executable_path:
                    selected_executable_path.map(Path::to_path_buf),
                pending_candidate_executable_path:
                    pending_candidate_executable_path.map(Path::to_path_buf),
                revision,
                updated_at_ms,
            };
            Ok(value.clone())
        }
    }

    fn runtime(digest: char) -> NodeRuntimeFingerprint {
        NodeRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from(format!(
                "node-{digest}"
            )),
            source_kind: NodeRuntimeSourceKind::Managed,
            node_version: VersionString::from("24.8.0"),
            node_major: 24,
            runtime_target: RuntimeTarget::from(
                "x86_64-pc-windows-msvc",
            ),
            executable_digest: DigestHex::from(digest.to_string().repeat(64)),
            javascript_host_protocol_version:
                JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version:
                JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
        }
    }

    fn validation(
        candidate: NodeRuntimeFingerprint,
        outcome: RuntimeSwitchParticipantOutcome,
    ) -> RuntimeSwitchValidationResult {
        RuntimeSwitchValidationResult {
            candidate,
            foundation_hello_passed: true,
            old_runtime_process_tree_zero: true,
            participants: vec![RuntimeSwitchParticipantResult {
                kind: RuntimeSwitchParticipantKind::MiniappService,
                owner_id: "miniapp-production-host".into(),
                outcome,
                error_code: (outcome
                    == RuntimeSwitchParticipantOutcome::Failed)
                    .then(|| CanonicalErrorCode::from("MINIAPP_FAILED")),
            }],
            completed_at_ms: 1,
        }
    }

    #[tokio::test]
    async fn exact_validation_is_persisted_before_user_decision() {
        let manager =
            NodeRuntimeManager::new(Arc::new(MemoryStore::default()));
        let candidate = runtime('a');
        let pending = manager
            .begin_pending(
                0,
                candidate.clone(),
                PathBuf::from(r"C:\managed\node.exe"),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            pending.selection.pending_candidate,
            Some(candidate.clone())
        );
        assert!(pending.selection.selected_runtime.is_none());
        assert!(pending.selection.validation_result.is_none());

        let validated = manager
            .record_validation(
                1,
                candidate.runtime_installation_id.as_ref(),
                candidate.executable_digest.as_ref(),
                validation(
                    candidate.clone(),
                    RuntimeSwitchParticipantOutcome::NotCovered,
                ),
            )
            .await
            .unwrap();
        assert!(validated.selection.validation_result.is_some());

        let committed = manager
            .commit_pending(
                2,
                candidate.runtime_installation_id.as_ref(),
                candidate.executable_digest.as_ref(),
            )
            .await
            .unwrap();
        assert_eq!(committed.selection.selected_runtime, Some(candidate));
        assert!(committed.selection.pending_candidate.is_none());
    }

    #[tokio::test]
    async fn abort_pending_restores_the_previous_selection() {
        let manager =
            NodeRuntimeManager::new(Arc::new(MemoryStore::default()));
        let selected = runtime('a');
        manager
            .select_initial(
                0,
                selected.clone(),
                PathBuf::from(r"C:\managed\selected\node.exe"),
            )
            .await
            .unwrap();
        let candidate = runtime('b');
        manager
            .begin_pending(
                1,
                candidate.clone(),
                PathBuf::from(r"C:\managed\candidate\node.exe"),
                false,
            )
            .await
            .unwrap();
        manager
            .record_validation(
                2,
                candidate.runtime_installation_id.as_ref(),
                candidate.executable_digest.as_ref(),
                validation(
                    candidate.clone(),
                    RuntimeSwitchParticipantOutcome::Failed,
                ),
            )
            .await
            .unwrap();
        let aborted = manager
            .abort_pending(
                3,
                candidate.runtime_installation_id.as_ref(),
                candidate.executable_digest.as_ref(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(aborted.selection.selected_runtime, Some(selected));
        assert!(aborted.selection.pending_candidate.is_none());
        assert!(aborted.selection.validation_result.is_none());
    }

    #[tokio::test]
    async fn stale_revision_never_changes_selection() {
        let manager =
            NodeRuntimeManager::new(Arc::new(MemoryStore::default()));
        let candidate = runtime('a');
        manager
            .begin_pending(
                0,
                candidate.clone(),
                PathBuf::from(r"C:\managed\node.exe"),
                false,
            )
            .await
            .unwrap();
        let error = manager
            .begin_pending(
                0,
                runtime('b'),
                PathBuf::from(r"C:\managed\node-b.exe"),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            JavaScriptRuntimeError::SelectionRevisionConflict { .. }
        ));
        assert_eq!(
            manager
                .snapshot()
                .await
                .unwrap()
                .selection
                .pending_candidate,
            Some(candidate)
        );
    }
}
