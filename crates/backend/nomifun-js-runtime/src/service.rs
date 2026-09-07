use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    NodeProbeDisposition, NodeRuntimeFingerprint, NodeRuntimeProbeResult,
    NodeRuntimeSourceKind, RuntimeSelectionRecord, RuntimeSwitchDecision,
    RuntimeSwitchParticipantKind, RuntimeSwitchParticipantOutcome,
    RuntimeSwitchValidationResult,
};
use nomifun_api_types::{
    BeginJavascriptRuntimeSwitchRequest,
    ConfirmJavascriptRuntimeDownloadRequest,
    DecideJavascriptRuntimeSwitchRequest,
    JavascriptRuntimeCompatibilityDto, JavascriptRuntimeDownloadDto,
    JavascriptRuntimeDownloadOfferDto, JavascriptRuntimeDownloadStateDto,
    JavascriptRuntimeProbeDto, JavascriptRuntimeRefDto,
    JavascriptRuntimeSourceDto, JavascriptRuntimeStatusDto,
    ProbeJavascriptRuntimeRequest, RuntimeSwitchDecisionDto,
    RuntimeSwitchParticipantDto, RuntimeSwitchParticipantKindDto,
    RuntimeSwitchParticipantStatusDto,
};
use tokio::sync::{Mutex, RwLock};

use crate::{
    JavaScriptRuntimeError, ManagedRuntimeOffer, ManagedRuntimeProvider,
    NodeDiscoveryRequest, NodeProbeCandidate, NodeResolution,
    NodeRuntimeManager, NodeRuntimeResolver, VersionedRuntimeSelection,
};

#[async_trait]
pub trait NodeRuntimeProbePort: Send + Sync {
    async fn resolve(
        &self,
        request: NodeDiscoveryRequest,
    ) -> Result<NodeResolution, JavaScriptRuntimeError>;

    async fn probe(&self, candidate: &NodeProbeCandidate)
    -> NodeRuntimeProbeResult;
}

#[derive(Clone, Debug, Default)]
pub struct SystemNodeRuntimeProbePort {
    resolver: NodeRuntimeResolver,
}

#[async_trait]
impl NodeRuntimeProbePort for SystemNodeRuntimeProbePort {
    async fn resolve(
        &self,
        request: NodeDiscoveryRequest,
    ) -> Result<NodeResolution, JavaScriptRuntimeError> {
        self.resolver.resolve(request).await
    }

    async fn probe(
        &self,
        candidate: &NodeProbeCandidate,
    ) -> NodeRuntimeProbeResult {
        self.resolver.probe(candidate).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCandidate {
    pub fingerprint: NodeRuntimeFingerprint,
    pub executable_path: PathBuf,
    pub disposition: NodeProbeDisposition,
}

#[derive(Clone, Debug)]
pub struct RuntimeSwitchValidationRequest {
    pub owner_user_id: String,
    pub selected_runtime: Option<NodeRuntimeFingerprint>,
    pub candidate: RuntimeCandidate,
}

#[async_trait]
pub trait RuntimeSwitchValidator: Send + Sync {
    async fn validate(
        &self,
        request: RuntimeSwitchValidationRequest,
    ) -> Result<RuntimeSwitchValidationResult, JavaScriptRuntimeError>;
}

#[derive(Debug)]
struct RuntimeInventory {
    initialized: bool,
    saved_manual_path: Option<PathBuf>,
    probes: Vec<NodeRuntimeProbeResult>,
    candidates: BTreeMap<String, RuntimeCandidate>,
    offer: Option<ManagedRuntimeOffer>,
    inventory_error_code: Option<String>,
    download: JavascriptRuntimeDownloadDto,
}

impl Default for RuntimeInventory {
    fn default() -> Self {
        Self {
            initialized: false,
            saved_manual_path: None,
            probes: Vec::new(),
            candidates: BTreeMap::new(),
            offer: None,
            inventory_error_code: None,
            download: JavascriptRuntimeDownloadDto {
                download_revision: 0,
                state: JavascriptRuntimeDownloadStateDto::NotInstalled,
                downloaded_bytes: None,
                total_bytes: None,
                runtime: None,
                error_code: None,
            },
        }
    }
}

pub struct JavaScriptRuntimeService {
    manager: Arc<NodeRuntimeManager>,
    probe: Arc<dyn NodeRuntimeProbePort>,
    managed: Arc<dyn ManagedRuntimeProvider>,
    validator: Arc<dyn RuntimeSwitchValidator>,
    inventory: RwLock<RuntimeInventory>,
    operation_gate: Mutex<()>,
}

impl std::fmt::Debug for JavaScriptRuntimeService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JavaScriptRuntimeService")
            .finish_non_exhaustive()
    }
}

impl JavaScriptRuntimeService {
    pub fn new(
        manager: Arc<NodeRuntimeManager>,
        probe: Arc<dyn NodeRuntimeProbePort>,
        managed: Arc<dyn ManagedRuntimeProvider>,
        validator: Arc<dyn RuntimeSwitchValidator>,
    ) -> Self {
        Self {
            manager,
            probe,
            managed,
            validator,
            inventory: RwLock::new(RuntimeInventory::default()),
            operation_gate: Mutex::new(()),
        }
    }

    pub async fn status(
        &self,
    ) -> Result<JavascriptRuntimeStatusDto, JavaScriptRuntimeError> {
        if !self.inventory.read().await.initialized {
            self.refresh_inventory(None).await?;
        }
        let selection = self.manager.snapshot().await?;
        self.project_status(selection).await
    }

    pub async fn probe(
        &self,
        request: ProbeJavascriptRuntimeRequest,
    ) -> Result<JavascriptRuntimeStatusDto, JavaScriptRuntimeError> {
        let _gate = self.operation_gate.lock().await;
        let expected_revision = match &request {
            ProbeJavascriptRuntimeRequest::AutoDiscover {
                expected_selection_revision,
            }
            | ProbeJavascriptRuntimeRequest::ManualPath {
                expected_selection_revision,
                ..
            }
            | ProbeJavascriptRuntimeRequest::ManagedInstallation {
                expected_selection_revision,
                ..
            } => *expected_selection_revision,
        };
        let selection = self.manager.snapshot().await?;
        require_revision(&selection, expected_revision)?;

        match request {
            ProbeJavascriptRuntimeRequest::AutoDiscover { .. } => {
                self.refresh_inventory(None).await?;
            }
            ProbeJavascriptRuntimeRequest::ManualPath {
                executable_path, ..
            } => {
                let path = PathBuf::from(executable_path);
                if !path.is_absolute() {
                    return Err(JavaScriptRuntimeError::RelativePath(path));
                }
                self.refresh_inventory(Some(path.clone())).await?;
                self.inventory.write().await.saved_manual_path = Some(path);
            }
            ProbeJavascriptRuntimeRequest::ManagedInstallation {
                runtime_installation_id,
                expected_executable_digest,
                ..
            } => {
                let mut candidate = self
                    .candidate(
                        &runtime_installation_id,
                        &expected_executable_digest,
                    )
                    .await;
                if candidate.is_none() {
                    self.refresh_inventory(None).await?;
                    candidate = self
                        .candidate(
                            &runtime_installation_id,
                            &expected_executable_digest,
                        )
                        .await;
                }
                let candidate =
                    candidate.ok_or(JavaScriptRuntimeError::CandidateUnavailable)?;
                if candidate.fingerprint.source_kind
                    != NodeRuntimeSourceKind::Managed
                {
                    return Err(JavaScriptRuntimeError::CandidateMismatch);
                }
                self.reprobe_exact(&candidate).await?;
            }
        }
        self.status().await
    }

    pub async fn confirm_download(
        self: &Arc<Self>,
        request: ConfirmJavascriptRuntimeDownloadRequest,
    ) -> Result<JavascriptRuntimeStatusDto, JavaScriptRuntimeError> {
        let _gate = self.operation_gate.lock().await;
        let selection = self.manager.snapshot().await?;
        require_revision(&selection, request.expected_selection_revision)?;

        let offer = match self.inventory.read().await.offer.clone() {
            Some(offer) => offer,
            None => {
                let offer = self.managed.resolve_offer().await?;
                self.inventory.write().await.offer = Some(offer.clone());
                offer
            }
        };
        if offer.offer_digest.as_ref() != request.expected_offer_digest {
            return Err(JavaScriptRuntimeError::DownloadOfferStale);
        }
        {
            let mut inventory = self.inventory.write().await;
            if inventory.download.state
                == JavascriptRuntimeDownloadStateDto::Downloading
            {
                return Err(JavaScriptRuntimeError::DownloadAlreadyRunning);
            }
            inventory.download = next_download_state(
                &inventory.download,
                JavascriptRuntimeDownloadStateDto::Downloading,
                offer.archive_size_bytes,
                None,
                None,
            )?;
        }
        let service = Arc::clone(self);
        tokio::spawn(async move {
            service.finish_download(offer).await;
        });
        self.status().await
    }

    pub async fn begin_switch(
        &self,
        owner_user_id: &str,
        request: BeginJavascriptRuntimeSwitchRequest,
    ) -> Result<JavascriptRuntimeStatusDto, JavaScriptRuntimeError> {
        let _gate = self.operation_gate.lock().await;
        let current = self.manager.snapshot().await?;
        require_revision(&current, request.expected_selection_revision)?;
        require_selected_identity(&current.selection, &request)?;
        if current.selection.pending_candidate.is_some() {
            return Err(JavaScriptRuntimeError::SwitchAlreadyPending);
        }
        if self.inventory.read().await.download.state
            == JavascriptRuntimeDownloadStateDto::Downloading
        {
            return Err(JavaScriptRuntimeError::DownloadAlreadyRunning);
        }

        let candidate = self
            .candidate(
                &request.candidate_runtime_id,
                &request.expected_candidate_executable_digest,
            )
            .await
            .ok_or(JavaScriptRuntimeError::CandidateUnavailable)?;
        let candidate = self.reprobe_exact(&candidate).await?;
        let warning_acknowledged = current
            .selection
            .non_recommended_warning_acknowledged
            .contains(&candidate.fingerprint.runtime_installation_id);
        let non_recommended = candidate.disposition
            == NodeProbeDisposition::CompatibleNonRecommended;
        if non_recommended
            && !warning_acknowledged
            && !request.acknowledge_non_recommended_runtime
        {
            return Err(
                JavaScriptRuntimeError::NonRecommendedConfirmationRequired,
            );
        }

        let validation = self
            .validator
            .validate(RuntimeSwitchValidationRequest {
                owner_user_id: owner_user_id.to_owned(),
                selected_runtime: current.selection.selected_runtime.clone(),
                candidate: candidate.clone(),
            })
            .await?;
        if validation.candidate != candidate.fingerprint {
            return Err(JavaScriptRuntimeError::CandidateMismatch);
        }
        let saved = self
            .manager
            .commit_validated_switch(
                request.expected_selection_revision,
                candidate.fingerprint,
                candidate.executable_path,
                validation,
                non_recommended
                    && request.acknowledge_non_recommended_runtime,
            )
            .await?;
        self.project_status(saved).await
    }

    pub async fn decide_switch(
        &self,
        request: DecideJavascriptRuntimeSwitchRequest,
    ) -> Result<JavascriptRuntimeStatusDto, JavaScriptRuntimeError> {
        let _gate = self.operation_gate.lock().await;
        let decision = match request.decision {
            RuntimeSwitchDecisionDto::CommitCandidate => {
                RuntimeSwitchDecision::CommitCandidate
            }
            RuntimeSwitchDecisionDto::AbortAndRestoreSelected => {
                RuntimeSwitchDecision::AbortAndRestoreSelected
            }
        };
        let saved = self
            .manager
            .decide(
                request.expected_selection_revision,
                &request.candidate_runtime_id,
                &request.expected_candidate_executable_digest,
                decision,
            )
            .await?;
        self.project_status(saved).await
    }

    async fn refresh_inventory(
        &self,
        explicit_path: Option<PathBuf>,
    ) -> Result<(), JavaScriptRuntimeError> {
        let selection = self.manager.snapshot().await?;
        let saved_manual_path = self
            .inventory
            .read()
            .await
            .saved_manual_path
            .clone()
            .or_else(|| persisted_manual_path(&selection));
        let managed_paths = self.managed.installed_executables().await;
        let managed_paths = match managed_paths {
            Ok(paths) => paths,
            Err(error) => {
                self.inventory.write().await.inventory_error_code =
                    Some(error.code().to_owned());
                Vec::new()
            }
        };
        let request = NodeDiscoveryRequest {
            explicit_path: explicit_path.clone(),
            saved_path: saved_manual_path
                .filter(|saved| explicit_path.as_ref() != Some(saved)),
            managed_paths,
        };
        let (resolution, offer) = tokio::join!(
            self.probe.resolve(request),
            self.managed.resolve_offer()
        );
        let resolution = resolution?;
        let mut inventory = self.inventory.write().await;
        inventory.initialized = true;
        inventory.probes = resolution.probes;
        inventory.candidates = candidate_index(&inventory.probes);
        match offer {
            Ok(offer) => {
                inventory.offer = Some(offer);
                inventory.inventory_error_code = None;
            }
            Err(error) => {
                inventory.offer = None;
                inventory.inventory_error_code =
                    Some(error.code().to_owned());
            }
        }
        Ok(())
    }

    async fn candidate(
        &self,
        runtime_id: &str,
        executable_digest: &str,
    ) -> Option<RuntimeCandidate> {
        self.inventory
            .read()
            .await
            .candidates
            .get(runtime_id)
            .filter(|candidate| {
                candidate.fingerprint.executable_digest.as_ref()
                    == executable_digest
            })
            .cloned()
    }

    async fn reprobe_exact(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<RuntimeCandidate, JavaScriptRuntimeError> {
        let probe = self
            .probe
            .probe(&NodeProbeCandidate::new(
                candidate.fingerprint.source_kind,
                candidate.executable_path.clone(),
            ))
            .await;
        let observed = probe
            .fingerprint
            .clone()
            .ok_or(JavaScriptRuntimeError::CandidateStale)?;
        if observed != candidate.fingerprint
            || !matches!(
                probe.disposition,
                NodeProbeDisposition::CompatibleRecommended
                    | NodeProbeDisposition::CompatibleNonRecommended
            )
        {
            return Err(JavaScriptRuntimeError::CandidateStale);
        }
        let exact = RuntimeCandidate {
            fingerprint: observed,
            executable_path: PathBuf::from(&probe.executable_path),
            disposition: probe.disposition,
        };
        let mut inventory = self.inventory.write().await;
        merge_probe(&mut inventory.probes, probe);
        inventory.candidates = candidate_index(&inventory.probes);
        Ok(exact)
    }

    async fn finish_download(&self, offer: ManagedRuntimeOffer) {
        match self.managed.install_offer(&offer).await {
            Ok(fingerprint) => {
                let paths = self
                    .managed
                    .installed_executables()
                    .await
                    .unwrap_or_default();
                let mut matching_probe = None;
                for path in paths {
                    let probe = self
                        .probe
                        .probe(&NodeProbeCandidate::new(
                            NodeRuntimeSourceKind::Managed,
                            path,
                        ))
                        .await;
                    if probe.fingerprint.as_ref() == Some(&fingerprint) {
                        matching_probe = Some(probe);
                        break;
                    }
                }
                let mut inventory = self.inventory.write().await;
                if let Some(probe) = matching_probe {
                    merge_probe(&mut inventory.probes, probe);
                    inventory.candidates = candidate_index(&inventory.probes);
                    inventory.download = next_download_state(
                        &inventory.download,
                        JavascriptRuntimeDownloadStateDto::Ready,
                        offer.archive_size_bytes,
                        Some(runtime_ref(&fingerprint)),
                        None,
                    )
                    .unwrap_or_else(|error| failed_download(
                        &inventory.download,
                        error.code(),
                    ));
                } else {
                    inventory.download = failed_download(
                        &inventory.download,
                        JavaScriptRuntimeError::CandidateStale.code(),
                    );
                }
            }
            Err(error) => {
                let mut inventory = self.inventory.write().await;
                inventory.download =
                    failed_download(&inventory.download, error.code());
            }
        }
    }

    async fn project_status(
        &self,
        versioned: VersionedRuntimeSelection,
    ) -> Result<JavascriptRuntimeStatusDto, JavaScriptRuntimeError> {
        versioned.validate()?;
        let inventory = self.inventory.read().await;
        let validation = versioned.selection.validation_result.as_ref();
        let switch_participants = validation
            .map(|validation| {
                validation
                    .participants
                    .iter()
                    .map(participant_dto)
                    .collect()
            })
            .unwrap_or_default();
        let requires_switch_decision = validation
            .is_some_and(RuntimeSwitchValidationResult::requires_user_decision);
        Ok(JavascriptRuntimeStatusDto {
            selection_revision: versioned.revision,
            selected: versioned
                .selection
                .selected_runtime
                .as_ref()
                .map(runtime_ref),
            pending_candidate: versioned
                .selection
                .pending_candidate
                .as_ref()
                .map(runtime_ref),
            probes: inventory.probes.iter().map(probe_dto).collect(),
            switch_participants,
            requires_switch_decision,
            last_error_code: versioned
                .selection
                .last_error
                .as_ref()
                .map(|code| code.as_ref().to_owned())
                .or_else(|| inventory.inventory_error_code.clone()),
            non_recommended_warning_acknowledged: versioned
                .selection
                .non_recommended_warning_acknowledged
                .iter()
                .map(|id| id.as_ref().to_owned())
                .collect(),
            download_offer: inventory.offer.as_ref().map(offer_dto),
            download: inventory.download.clone(),
        })
    }
}

fn require_revision(
    current: &VersionedRuntimeSelection,
    expected: u64,
) -> Result<(), JavaScriptRuntimeError> {
    if current.revision == expected {
        Ok(())
    } else {
        Err(JavaScriptRuntimeError::SelectionRevisionConflict {
            expected,
            observed: current.revision,
        })
    }
}

fn persisted_manual_path(
    selection: &VersionedRuntimeSelection,
) -> Option<PathBuf> {
    [
        (
            selection.selection.selected_runtime.as_ref(),
            selection.selected_executable_path.as_ref(),
        ),
        (
            selection.selection.pending_candidate.as_ref(),
            selection.pending_candidate_executable_path.as_ref(),
        ),
    ]
    .into_iter()
    .find_map(|(runtime, path)| {
        (runtime.is_some_and(|runtime| {
            runtime.source_kind == NodeRuntimeSourceKind::ManualPath
        }))
        .then(|| path.cloned())
        .flatten()
    })
}

fn require_selected_identity(
    selection: &RuntimeSelectionRecord,
    request: &BeginJavascriptRuntimeSwitchRequest,
) -> Result<(), JavaScriptRuntimeError> {
    match (
        selection.selected_runtime.as_ref(),
        request.expected_selected_runtime_id.as_deref(),
        request.expected_selected_executable_digest.as_deref(),
    ) {
        (None, None, None) => Ok(()),
        (Some(selected), Some(runtime_id), Some(digest))
            if selected.runtime_installation_id.as_ref() == runtime_id
                && selected.executable_digest.as_ref() == digest =>
        {
            Ok(())
        }
        _ => Err(JavaScriptRuntimeError::SelectedRuntimeMismatch),
    }
}

fn candidate_index(
    probes: &[NodeRuntimeProbeResult],
) -> BTreeMap<String, RuntimeCandidate> {
    probes
        .iter()
        .filter_map(|probe| {
            let fingerprint = probe.fingerprint.clone()?;
            Some((
                fingerprint.runtime_installation_id.as_ref().to_owned(),
                RuntimeCandidate {
                    fingerprint,
                    executable_path: PathBuf::from(&probe.executable_path),
                    disposition: probe.disposition,
                },
            ))
        })
        .collect()
}

fn merge_probe(
    probes: &mut Vec<NodeRuntimeProbeResult>,
    incoming: NodeRuntimeProbeResult,
) {
    probes.retain(|probe| {
        probe.executable_path != incoming.executable_path
            && probe
                .fingerprint
                .as_ref()
                .zip(incoming.fingerprint.as_ref())
                .is_none_or(|(current, next)| {
                    current.runtime_installation_id
                        != next.runtime_installation_id
                })
    });
    probes.push(incoming);
    probes.sort_by(|left, right| {
        source_rank(left.source_kind)
            .cmp(&source_rank(right.source_kind))
            .then_with(|| left.executable_path.cmp(&right.executable_path))
    });
}

fn source_rank(source: NodeRuntimeSourceKind) -> u8 {
    match source {
        NodeRuntimeSourceKind::ManualPath => 0,
        NodeRuntimeSourceKind::ProcessPath => 1,
        NodeRuntimeSourceKind::Managed => 2,
    }
}

fn runtime_ref(runtime: &NodeRuntimeFingerprint) -> JavascriptRuntimeRefDto {
    JavascriptRuntimeRefDto {
        runtime_installation_id: runtime.runtime_installation_id.as_ref().to_owned(),
        node_version: runtime.node_version.as_ref().to_owned(),
        runtime_target: runtime.runtime_target.as_ref().to_owned(),
        executable_digest: runtime.executable_digest.as_ref().to_owned(),
    }
}

fn probe_dto(probe: &NodeRuntimeProbeResult) -> JavascriptRuntimeProbeDto {
    JavascriptRuntimeProbeDto {
        source: match probe.source_kind {
            NodeRuntimeSourceKind::ManualPath => {
                JavascriptRuntimeSourceDto::ManualPath
            }
            NodeRuntimeSourceKind::ProcessPath => {
                JavascriptRuntimeSourceDto::ProcessPath
            }
            NodeRuntimeSourceKind::Managed => {
                JavascriptRuntimeSourceDto::Managed
            }
        },
        executable_path: probe.executable_path.clone(),
        compatibility: match probe.disposition {
            NodeProbeDisposition::CompatibleRecommended => {
                JavascriptRuntimeCompatibilityDto::Recommended
            }
            NodeProbeDisposition::CompatibleNonRecommended => {
                JavascriptRuntimeCompatibilityDto::Compatible
            }
            NodeProbeDisposition::Incompatible => {
                JavascriptRuntimeCompatibilityDto::Incompatible
            }
        },
        runtime: probe.fingerprint.as_ref().map(runtime_ref),
        error_code: probe
            .error_code
            .as_ref()
            .map(|code| code.as_ref().to_owned()),
    }
}

fn participant_dto(
    participant: &nomifun_agent_contracts::RuntimeSwitchParticipantResult,
) -> RuntimeSwitchParticipantDto {
    RuntimeSwitchParticipantDto {
        kind: match participant.kind {
            RuntimeSwitchParticipantKind::PluginMount => {
                RuntimeSwitchParticipantKindDto::PluginMount
            }
            RuntimeSwitchParticipantKind::MiniappService => {
                RuntimeSwitchParticipantKindDto::MiniappService
            }
            RuntimeSwitchParticipantKind::BuildFoundation => {
                RuntimeSwitchParticipantKindDto::BuildFoundation
            }
        },
        owner_id: participant.owner_id.clone(),
        status: match participant.outcome {
            RuntimeSwitchParticipantOutcome::Passed => {
                RuntimeSwitchParticipantStatusDto::Passed
            }
            RuntimeSwitchParticipantOutcome::Failed => {
                RuntimeSwitchParticipantStatusDto::Failed
            }
            RuntimeSwitchParticipantOutcome::NotCovered => {
                RuntimeSwitchParticipantStatusDto::NotCovered
            }
        },
        error_code: participant
            .error_code
            .as_ref()
            .map(|code| code.as_ref().to_owned()),
    }
}

fn offer_dto(offer: &ManagedRuntimeOffer) -> JavascriptRuntimeDownloadOfferDto {
    JavascriptRuntimeDownloadOfferDto {
        offer_digest: offer.offer_digest.as_ref().to_owned(),
        node_version: offer.node_version.clone(),
        runtime_target: offer.runtime_target.as_ref().to_owned(),
        archive_file_name: offer.archive_file_name.clone(),
        archive_size_bytes: offer.archive_size_bytes,
    }
}

fn next_download_state(
    current: &JavascriptRuntimeDownloadDto,
    state: JavascriptRuntimeDownloadStateDto,
    total_bytes: Option<u64>,
    runtime: Option<JavascriptRuntimeRefDto>,
    error_code: Option<String>,
) -> Result<JavascriptRuntimeDownloadDto, JavaScriptRuntimeError> {
    let download_revision = current
        .download_revision
        .checked_add(1)
        .ok_or_else(|| {
            JavaScriptRuntimeError::Contract(
                "Runtime download revision overflow".into(),
            )
        })?;
    Ok(JavascriptRuntimeDownloadDto {
        download_revision,
        state,
        downloaded_bytes: None,
        total_bytes,
        runtime,
        error_code,
    })
}

fn failed_download(
    current: &JavascriptRuntimeDownloadDto,
    error_code: &str,
) -> JavascriptRuntimeDownloadDto {
    next_download_state(
        current,
        JavascriptRuntimeDownloadStateDto::Failed,
        current.total_bytes,
        None,
        Some(error_code.to_owned()),
    )
    .unwrap_or_else(|_| JavascriptRuntimeDownloadDto {
        download_revision: current.download_revision,
        state: JavascriptRuntimeDownloadStateDto::Failed,
        downloaded_bytes: None,
        total_bytes: current.total_bytes,
        runtime: None,
        error_code: Some(error_code.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nomifun_agent_contracts::{
        CanonicalErrorCode, DigestHex, JAVASCRIPT_HOST_PROTOCOL_VERSION,
        JAVASCRIPT_SDK_CONTRACT_VERSION, RECOMMENDED_NODE_LTS_MAJOR,
        RuntimeInstallationId, RuntimeSwitchParticipantResult, RuntimeTarget,
        VersionString,
    };
    use tokio::sync::Mutex;

    use super::*;
    use crate::{RuntimeSelectionStore, RuntimeSelectionStoreError};

    #[derive(Default)]
    struct MemoryStore {
        value: Mutex<VersionedRuntimeSelection>,
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
            selected_executable_path: Option<&std::path::Path>,
            pending_candidate_executable_path: Option<&std::path::Path>,
            updated_at_ms: i64,
        ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
            let mut value = self.value.lock().await;
            if value.revision != expected_revision {
                return Err(RuntimeSelectionStoreError::Conflict(
                    "stale".into(),
                ));
            }
            *value = VersionedRuntimeSelection {
                selection: selection.clone(),
                selected_executable_path:
                    selected_executable_path.map(std::path::Path::to_path_buf),
                pending_candidate_executable_path:
                    pending_candidate_executable_path
                        .map(std::path::Path::to_path_buf),
                revision: expected_revision + 1,
                updated_at_ms,
            };
            Ok(value.clone())
        }
    }

    #[derive(Clone)]
    struct FakeProbe {
        candidate: RuntimeCandidate,
    }

    #[async_trait]
    impl NodeRuntimeProbePort for FakeProbe {
        async fn resolve(
            &self,
            _request: NodeDiscoveryRequest,
        ) -> Result<NodeResolution, JavaScriptRuntimeError> {
            Ok(NodeResolution {
                selected: Some(self.candidate.fingerprint.clone()),
                probes: vec![probe_result(&self.candidate)],
            })
        }

        async fn probe(
            &self,
            _candidate: &NodeProbeCandidate,
        ) -> NodeRuntimeProbeResult {
            probe_result(&self.candidate)
        }
    }

    struct FakeManaged {
        offer: ManagedRuntimeOffer,
        candidate: RuntimeCandidate,
    }

    #[async_trait]
    impl ManagedRuntimeProvider for FakeManaged {
        async fn resolve_offer(
            &self,
        ) -> Result<ManagedRuntimeOffer, JavaScriptRuntimeError> {
            Ok(self.offer.clone())
        }

        async fn install_offer(
            &self,
            offer: &ManagedRuntimeOffer,
        ) -> Result<NodeRuntimeFingerprint, JavaScriptRuntimeError> {
            if offer.offer_digest != self.offer.offer_digest {
                return Err(JavaScriptRuntimeError::DownloadOfferStale);
            }
            Ok(self.candidate.fingerprint.clone())
        }

        async fn installed_executables(
            &self,
        ) -> Result<Vec<PathBuf>, JavaScriptRuntimeError> {
            Ok(vec![self.candidate.executable_path.clone()])
        }
    }

    struct FakeValidator {
        outcome: RuntimeSwitchParticipantOutcome,
    }

    #[async_trait]
    impl RuntimeSwitchValidator for FakeValidator {
        async fn validate(
            &self,
            request: RuntimeSwitchValidationRequest,
        ) -> Result<RuntimeSwitchValidationResult, JavaScriptRuntimeError> {
            Ok(RuntimeSwitchValidationResult {
                candidate: request.candidate.fingerprint,
                foundation_hello_passed: true,
                old_runtime_process_tree_zero: true,
                participants: vec![RuntimeSwitchParticipantResult {
                    kind: RuntimeSwitchParticipantKind::MiniappService,
                    owner_id: "miniapp-production-host".into(),
                    outcome: self.outcome,
                    error_code: (self.outcome
                        == RuntimeSwitchParticipantOutcome::Failed)
                        .then(|| CanonicalErrorCode::from("MINIAPP_FAILED")),
                }],
                completed_at_ms: 1,
            })
        }
    }

    fn candidate(source: NodeRuntimeSourceKind, major: u16) -> RuntimeCandidate {
        let digest = if major == RECOMMENDED_NODE_LTS_MAJOR {
            "a"
        } else {
            "b"
        };
        RuntimeCandidate {
            fingerprint: NodeRuntimeFingerprint {
                runtime_installation_id: RuntimeInstallationId::from(
                    format!("node-{digest}"),
                ),
                source_kind: source,
                node_version: VersionString::from(format!("{major}.1.0")),
                node_major: major,
                runtime_target: RuntimeTarget::from(
                    "x86_64-pc-windows-msvc",
                ),
                executable_digest: DigestHex::from(digest.repeat(64)),
                javascript_host_protocol_version:
                    JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                javascript_sdk_contract_version:
                    JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            },
            executable_path: PathBuf::from(r"C:\node\node.exe"),
            disposition: if major == RECOMMENDED_NODE_LTS_MAJOR {
                NodeProbeDisposition::CompatibleRecommended
            } else {
                NodeProbeDisposition::CompatibleNonRecommended
            },
        }
    }

    fn probe_result(candidate: &RuntimeCandidate) -> NodeRuntimeProbeResult {
        NodeRuntimeProbeResult {
            source_kind: candidate.fingerprint.source_kind,
            executable_path: candidate.executable_path.display().to_string(),
            disposition: candidate.disposition,
            fingerprint: Some(candidate.fingerprint.clone()),
            error_code: None,
        }
    }

    fn offer(candidate: &RuntimeCandidate) -> ManagedRuntimeOffer {
        ManagedRuntimeOffer {
            offer_digest: DigestHex::from("c".repeat(64)),
            node_version: candidate.fingerprint.node_version.as_ref().to_owned(),
            runtime_target: candidate.fingerprint.runtime_target.clone(),
            archive_file_name: "node-v24.1.0-win-x64.zip".into(),
            archive_sha256: "d".repeat(64),
            archive_size_bytes: Some(10),
        }
    }

    fn service(
        candidate: RuntimeCandidate,
        outcome: RuntimeSwitchParticipantOutcome,
    ) -> Arc<JavaScriptRuntimeService> {
        Arc::new(JavaScriptRuntimeService::new(
            Arc::new(NodeRuntimeManager::new(Arc::new(
                MemoryStore::default(),
            ))),
            Arc::new(FakeProbe {
                candidate: candidate.clone(),
            }),
            Arc::new(FakeManaged {
                offer: offer(&candidate),
                candidate,
            }),
            Arc::new(FakeValidator { outcome }),
        ))
    }

    #[tokio::test]
    async fn client_can_only_switch_to_an_exact_server_probe() {
        let candidate = candidate(NodeRuntimeSourceKind::ProcessPath, 24);
        let service = service(
            candidate.clone(),
            RuntimeSwitchParticipantOutcome::NotCovered,
        );
        service
            .probe(ProbeJavascriptRuntimeRequest::AutoDiscover {
                expected_selection_revision: 0,
            })
            .await
            .unwrap();
        let error = service
            .begin_switch(
                "owner",
                BeginJavascriptRuntimeSwitchRequest {
                    expected_selection_revision: 0,
                    expected_selected_runtime_id: None,
                    expected_selected_executable_digest: None,
                    candidate_runtime_id: candidate
                        .fingerprint
                        .runtime_installation_id
                        .as_ref()
                        .to_owned(),
                    expected_candidate_executable_digest: "f".repeat(64),
                    acknowledge_non_recommended_runtime: false,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            JavaScriptRuntimeError::CandidateUnavailable
        ));
        assert_eq!(service.status().await.unwrap().selection_revision, 0);
    }

    #[tokio::test]
    async fn non_recommended_runtime_requires_one_time_confirmation() {
        let candidate = candidate(NodeRuntimeSourceKind::ManualPath, 22);
        let service = service(
            candidate.clone(),
            RuntimeSwitchParticipantOutcome::NotCovered,
        );
        service
            .probe(ProbeJavascriptRuntimeRequest::AutoDiscover {
                expected_selection_revision: 0,
            })
            .await
            .unwrap();
        let request = BeginJavascriptRuntimeSwitchRequest {
            expected_selection_revision: 0,
            expected_selected_runtime_id: None,
            expected_selected_executable_digest: None,
            candidate_runtime_id: candidate
                .fingerprint
                .runtime_installation_id
                .as_ref()
                .to_owned(),
            expected_candidate_executable_digest: candidate
                .fingerprint
                .executable_digest
                .as_ref()
                .to_owned(),
            acknowledge_non_recommended_runtime: false,
        };
        assert!(matches!(
            service.begin_switch("owner", request.clone()).await,
            Err(JavaScriptRuntimeError::NonRecommendedConfirmationRequired)
        ));
        let status = service
            .begin_switch(
                "owner",
                BeginJavascriptRuntimeSwitchRequest {
                    acknowledge_non_recommended_runtime: true,
                    ..request
                },
            )
            .await
            .unwrap();
        assert!(status.requires_switch_decision);
        assert!(status
            .non_recommended_warning_acknowledged
            .contains(&candidate
                .fingerprint
                .runtime_installation_id
                .as_ref()
                .to_owned()));
    }

    #[tokio::test]
    async fn managed_download_accepts_only_the_server_offer_digest() {
        let candidate = candidate(NodeRuntimeSourceKind::Managed, 24);
        let service = service(
            candidate,
            RuntimeSwitchParticipantOutcome::NotCovered,
        );
        service
            .probe(ProbeJavascriptRuntimeRequest::AutoDiscover {
                expected_selection_revision: 0,
            })
            .await
            .unwrap();
        let error = service
            .confirm_download(ConfirmJavascriptRuntimeDownloadRequest {
                expected_selection_revision: 0,
                expected_offer_digest: "e".repeat(64),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            JavaScriptRuntimeError::DownloadOfferStale
        ));
    }

    #[tokio::test]
    async fn not_covered_validation_stays_pending_until_exact_decision() {
        let candidate = candidate(NodeRuntimeSourceKind::ProcessPath, 24);
        let service = service(
            candidate.clone(),
            RuntimeSwitchParticipantOutcome::NotCovered,
        );
        service
            .probe(ProbeJavascriptRuntimeRequest::AutoDiscover {
                expected_selection_revision: 0,
            })
            .await
            .unwrap();
        let pending = service
            .begin_switch(
                "owner",
                BeginJavascriptRuntimeSwitchRequest {
                    expected_selection_revision: 0,
                    expected_selected_runtime_id: None,
                    expected_selected_executable_digest: None,
                    candidate_runtime_id: candidate
                        .fingerprint
                        .runtime_installation_id
                        .as_ref()
                        .to_owned(),
                    expected_candidate_executable_digest: candidate
                        .fingerprint
                        .executable_digest
                        .as_ref()
                        .to_owned(),
                    acknowledge_non_recommended_runtime: false,
                },
            )
            .await
            .unwrap();
        assert!(pending.selected.is_none());
        assert_eq!(
            pending.pending_candidate.as_ref().unwrap().runtime_installation_id,
            candidate.fingerprint.runtime_installation_id.as_ref()
        );

        let committed = service
            .decide_switch(DecideJavascriptRuntimeSwitchRequest {
                expected_selection_revision: pending.selection_revision,
                candidate_runtime_id: candidate
                    .fingerprint
                    .runtime_installation_id
                    .as_ref()
                    .to_owned(),
                expected_candidate_executable_digest: candidate
                    .fingerprint
                    .executable_digest
                    .as_ref()
                    .to_owned(),
                decision: RuntimeSwitchDecisionDto::CommitCandidate,
            })
            .await
            .unwrap();
        assert_eq!(
            committed.selected.unwrap().runtime_installation_id,
            candidate.fingerprint.runtime_installation_id.as_ref()
        );
        assert!(committed.pending_candidate.is_none());
    }
}
