use agent_client_protocol::schema::{
    ContentBlock, EmbeddedResourceResource, PermissionOption, PermissionOptionKind as SdkPermissionOptionKind,
    RequestPermissionRequest, SessionNotification, SessionUpdate, ToolCallContent as SdkToolCallContent,
    ToolCallLocation as SdkToolCallLocation, ToolCallStatus as SdkToolCallStatus,
    ToolCallUpdate as SdkToolCallUpdate, ToolKind as SdkToolKind,
};
use base64::Engine as _;
use nomi_agent::output::{
    ArtifactContract, ArtifactExpectation, ArtifactRequirement, artifact_contract,
    artifact_contract_with_input, is_context_only_image_tool,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;
use tracing::debug;

use super::permission::{
    AcpPermissionEventData, AcpPermissionOptionData, AcpPermissionOptionKind, AcpPermissionRequestData,
    AcpPermissionToolCall,
};
use super::session_updates::{AvailableCommandsEventData, PlanEventData, ThinkingEventData};
use super::tool_call::{
    AcpToolCallContentItem, AcpToolCallEventData, AcpToolCallKind, AcpToolCallLocationItem,
    AcpToolCallSessionUpdateKind, AcpToolCallStatus, AcpToolCallTextBlock, AcpToolCallTextBlockType,
    AcpToolCallUpdateData,
};
use crate::artifact_store::{ArtifactKind, ArtifactStore, ArtifactStoreError, PersistedArtifact};

use super::{AgentStreamEvent, ErrorEventData, TextEventData};

const MAX_ACP_ARTIFACT_PATHS: usize = 32;
const MAX_ACP_ARTIFACT_PATH_LENGTH: usize = 4096;
const MAX_ACP_ARTIFACT_JSON_NODES: usize = 512;
const MAX_ACP_ARTIFACT_JSON_DEPTH: usize = 12;

#[derive(Debug)]
struct ToolArtifactDeliveryState {
    contract: Option<ArtifactContract>,
    delivered_artifact: bool,
    failure: Option<String>,
    candidate_paths: Vec<ArtifactPathCandidate>,
    verified_artifacts: Vec<PersistedArtifact>,
    provider_failed: bool,
    started_at: Option<SystemTime>,
    last_status: Option<AcpToolCallStatus>,
}

impl Default for ToolArtifactDeliveryState {
    fn default() -> Self {
        Self {
            contract: None,
            delivered_artifact: false,
            failure: None,
            candidate_paths: Vec::new(),
            verified_artifacts: Vec::new(),
            provider_failed: false,
            started_at: None,
            last_status: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ArtifactPathCandidate {
    path: String,
    baseline: ArtifactPathBaseline,
    observed_before_terminal: bool,
}

#[derive(Debug, Clone)]
enum ArtifactPathBaseline {
    Absent,
    Present { size_bytes: u64, sha256: String },
    Error(String),
}

struct ToolDeliveryOutcome {
    failure: Option<String>,
    force_failed: bool,
    releasable_artifacts: Vec<PersistedArtifact>,
}

fn any_artifact_contract() -> ArtifactContract {
    ArtifactContract {
        expectation: ArtifactExpectation::Any,
        requirement: ArtifactRequirement::Any,
        requested_count: None,
    }
}

fn contract_accepts_artifact(contract: ArtifactContract, artifact: &PersistedArtifact) -> bool {
    let kind_matches = match contract.expectation {
        ArtifactExpectation::Image => artifact.kind == ArtifactKind::Image,
        ArtifactExpectation::Audio => artifact.kind == ArtifactKind::Audio,
        ArtifactExpectation::Video => artifact.kind == ArtifactKind::Video,
        ArtifactExpectation::File | ArtifactExpectation::Any => true,
        ArtifactExpectation::None => false,
    };
    kind_matches && contract.accepts_mime(&artifact.mime_type)
}

/// Turn-scoped ACP artifact delivery state.
///
/// ACP tool updates are partial and can arrive as a sequence such as
/// `in_progress(image bytes)` -> `completed(no content)`. Conversely, a
/// provider can emit invalid bytes and then a late `completed` update. Keeping
/// this state outside a single notification makes delivery failure absorbing
/// for the lifetime of the tool call and gives the prompt lifecycle a reliable
/// signal for choosing Error instead of Finish(EndTurn).
#[derive(Debug, Default)]
pub(crate) struct AcpArtifactDeliveryState {
    calls: HashMap<(String, String), ToolArtifactDeliveryState>,
    turn_failures: HashMap<String, String>,
}

impl AcpArtifactDeliveryState {
    pub(crate) fn begin_turn(&mut self, session_id: &str) {
        self.calls.retain(|(sid, _), _| sid != session_id);
        self.turn_failures.remove(session_id);
    }

    pub(crate) fn turn_failure(&self, session_id: &str) -> Option<String> {
        self.turn_failures.get(session_id).cloned()
    }

    /// Seal a prompt turn after the ACP `PromptResponse` arrives. EndTurn is a
    /// terminal boundary: an artifact-producing call left pending/in-progress
    /// without a receipt is not allowed to turn into a successful Finish.
    #[cfg(test)]
    pub(crate) fn finish_turn(&mut self, session_id: &str) -> Option<String> {
        self.finish_turn_with_store(session_id, None)
    }

    /// Production prompt sealing additionally re-verifies each published
    /// receipt. A later shell/edit call can delete even an immutable snapshot;
    /// the turn must fail instead of publishing a locator that no longer works.
    pub(crate) fn finish_turn_with_store(
        &mut self,
        session_id: &str,
        artifact_store: Option<&ArtifactStore>,
    ) -> Option<String> {
        // Local delivery-integrity failures (invalid bytes, failed atomic
        // persistence, false Completed) remain fatal for the whole turn.
        if let Some(error) = self.turn_failure(session_id) {
            return Some(error);
        }

        for ((sid, tool_call_id), state) in &self.calls {
            if sid != session_id || state.contract.is_none() {
                continue;
            }
            let reason = if state.provider_failed {
                Some("reported a failed terminal status")
            } else if state.last_status != Some(AcpToolCallStatus::Completed) {
                Some("did not reach a completed terminal status")
            } else if !state.delivered_artifact {
                Some("completed without a verified artifact delivery")
            } else {
                None
            };
            if let Some(reason) = reason {
                let error = format!("ACP artifact-producing tool `{tool_call_id}` {reason}");
                self.record_turn_failure(session_id, error.clone());
                return Some(error);
            }
            if let Some(store) = artifact_store {
                for artifact in &state.verified_artifacts {
                    if let Err(error) = store.reverify_receipt(artifact) {
                        let error = format!(
                            "ACP artifact-producing tool `{tool_call_id}` artifact {} failed final verification: {error}",
                            artifact.path
                        );
                        self.record_turn_failure(session_id, error.clone());
                        return Some(error);
                    }
                }
            }
        }
        None
    }

    fn record_turn_failure(&mut self, session_id: &str, error: String) {
        self.turn_failures.entry(session_id.to_owned()).or_insert(error);
    }

    fn observe_tool_metadata(
        &mut self,
        session_id: &str,
        tool_call_id: &str,
        contract: Option<ArtifactContract>,
        candidate_paths: impl IntoIterator<Item = String>,
        requested_status: Option<AcpToolCallStatus>,
        artifact_store: Option<&ArtifactStore>,
    ) -> (
        Option<ArtifactContract>,
        Vec<ArtifactPathCandidate>,
        Option<SystemTime>,
    ) {
        let state = self
            .calls
            .entry((session_id.to_owned(), tool_call_id.to_owned()))
            .or_default();
        if matches!(
            state.last_status,
            Some(AcpToolCallStatus::Completed | AcpToolCallStatus::Failed)
        ) && state.failure.is_none()
        {
            state.failure = Some(format!(
                "ACP artifact-producing tool `{tool_call_id}` reused a terminal tool-call id"
            ));
        }
        match (state.contract, contract) {
            (None, observed) => state.contract = observed,
            (Some(current), Some(observed)) => match current.merge(observed) {
                Ok(merged) => state.contract = Some(merged),
                Err(error) if state.failure.is_none() => {
                    state.failure = Some(format!(
                        "ACP artifact-producing tool `{tool_call_id}` changed its artifact contract: {error}"
                    ));
                }
                Err(_) => {}
            },
            (Some(_), None) => {}
        }
        let observed_before_terminal = !matches!(
            requested_status,
            Some(AcpToolCallStatus::Completed | AcpToolCallStatus::Failed)
        );
        if observed_before_terminal && state.started_at.is_none() {
            state.started_at = Some(SystemTime::now());
        }
        for path in candidate_paths {
            if path.trim().is_empty()
                || state
                    .candidate_paths
                    .iter()
                    .any(|candidate| candidate.path == path)
            {
                continue;
            }
            let baseline = if observed_before_terminal {
                artifact_store.map_or_else(
                    || ArtifactPathBaseline::Error("session has no workspace artifact store".to_owned()),
                    |store| capture_artifact_path_baseline(store, &path),
                )
            } else {
                ArtifactPathBaseline::Error(
                    "artifact path was first observed at a terminal update".to_owned(),
                )
            };
            state.candidate_paths.push(ArtifactPathCandidate {
                path,
                baseline,
                observed_before_terminal,
            });
        }
        (
            state.contract,
            state.candidate_paths.clone(),
            state.started_at,
        )
    }

    fn apply_tool_update(
        &mut self,
        session_id: &str,
        tool_call_id: &str,
        contract: Option<ArtifactContract>,
        verified_artifacts: &[PersistedArtifact],
        requested_status: Option<AcpToolCallStatus>,
        delivery_error: Option<String>,
    ) -> ToolDeliveryOutcome {
        let key = (session_id.to_owned(), tool_call_id.to_owned());
        let state = self.calls.entry(key).or_default();
        match (state.contract, contract) {
            (None, observed) => state.contract = observed,
            (Some(current), Some(observed)) => match current.merge(observed) {
                Ok(merged) => state.contract = Some(merged),
                Err(error) if state.failure.is_none() => {
                    state.failure = Some(format!(
                        "ACP artifact-producing tool `{tool_call_id}` changed its artifact contract: {error}"
                    ));
                }
                Err(_) => {}
            },
            (Some(_), None) => {}
        }
        let mismatched_artifact = state.contract.and_then(|contract| {
            state
                .verified_artifacts
                .iter()
                .chain(verified_artifacts)
                .find(|artifact| !contract_accepts_artifact(contract, artifact))
        });
        if let Some(artifact) = mismatched_artifact {
            if state.failure.is_none() {
                let label = state
                    .contract
                    .map_or("artifact", ArtifactContract::label);
                state.failure = Some(format!(
                    "ACP artifact-producing tool `{tool_call_id}` delivered {} ({:?}), expected {}",
                    artifact.mime_type,
                    artifact.kind,
                    label
                ));
            }
        } else {
            for artifact in verified_artifacts {
                if !state.verified_artifacts.iter().any(|existing| {
                    existing.path == artifact.path && existing.sha256 == artifact.sha256
                }) {
                    state.verified_artifacts.push(artifact.clone());
                }
            }
            state.delivered_artifact |= !verified_artifacts.is_empty();
        }
        state.provider_failed |= requested_status == Some(AcpToolCallStatus::Failed);
        if let Some(status) = requested_status {
            state.last_status = Some(status);
        }
        if state.failure.is_none() {
            state.failure = delivery_error;
        }
        if state.failure.is_none()
            && requested_status == Some(AcpToolCallStatus::Completed)
            && let Some(contract) = state.contract
            && !state.provider_failed
        {
            if !state.delivered_artifact {
                state.failure = Some(format!(
                    "ACP artifact-producing tool `{tool_call_id}` completed without a verified artifact receipt or verified workspace output path"
                ));
            } else if state.verified_artifacts.len() < contract.expected_count() {
                state.failure = Some(format!(
                    "ACP artifact-producing tool `{tool_call_id}` completed with {} verified artifact receipt(s), expected at least {} {} receipt(s)",
                    state.verified_artifacts.len(),
                    contract.expected_count(),
                    contract.label()
                ));
            }
        }

        let failure = state.failure.clone();
        let outcome = ToolDeliveryOutcome {
            force_failed: failure.is_some() || state.provider_failed,
            releasable_artifacts: if requested_status == Some(AcpToolCallStatus::Completed)
                && failure.is_none()
                && !state.provider_failed
            {
                state.verified_artifacts.clone()
            } else {
                Vec::new()
            },
            failure: failure.clone(),
        };
        if let Some(error) = failure.clone() {
            self.record_turn_failure(session_id, error);
        }
        outcome
    }
}

/// Convert an SDK [`SessionNotification`] into zero or more [`AgentStreamEvent`]s.
pub(crate) fn session_notification_to_events(notif: &SessionNotification) -> Vec<AgentStreamEvent> {
    session_notification_to_events_with_store(notif, None)
}

/// Translate an ACP notification while materializing inline outputs into a
/// verified workspace store. Production ACP sessions always provide a store;
/// the `None` wrapper remains for metadata-only internal trackers and tests.
pub(crate) fn session_notification_to_events_with_store(
    notif: &SessionNotification,
    artifact_store: Option<&ArtifactStore>,
) -> Vec<AgentStreamEvent> {
    let mut delivery_state = AcpArtifactDeliveryState::default();
    session_notification_to_events_with_delivery_state(notif, artifact_store, &mut delivery_state)
}

pub(crate) fn session_notification_to_events_with_delivery_state(
    notif: &SessionNotification,
    artifact_store: Option<&ArtifactStore>,
    delivery_state: &mut AcpArtifactDeliveryState,
) -> Vec<AgentStreamEvent> {
    let session_id = notif.session_id.to_string();
    let mut events = Vec::new();

    match &notif.update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            match map_agent_message_content(&chunk.content, artifact_store) {
                Ok(Some(content)) => events.push(AgentStreamEvent::Text(TextEventData { content })),
                Ok(None) => {}
                Err(error) => {
                    delivery_state.record_turn_failure(&session_id, error.clone());
                    events.push(AgentStreamEvent::Error(ErrorEventData::legacy(
                        format!("ACP artifact delivery failed: {error}"),
                        None,
                    )));
                }
            }
        }

        SessionUpdate::AgentThoughtChunk(chunk) => {
            if let ContentBlock::Text(text) = &chunk.content {
                events.push(AgentStreamEvent::Thinking(ThinkingEventData {
                    content: text.text.clone(),
                    subject: None,
                    duration: None,
                    status: Some("in_progress".into()),
                }));
            }
        }

        SessionUpdate::UserMessageChunk(_chunk) => {}

        SessionUpdate::ToolCall(tc) => {
            let tool_call_id = tc.tool_call_id.to_string();
            let requested_status = Some(map_sdk_tool_status(&tc.status));
            let detected_contract = tool_artifact_contract(
                Some(&tc.title),
                tc.raw_input.as_ref(),
                tc.raw_output.as_ref(),
            );
            let mut contract = detected_contract.contract;
            if contract.is_none() && tool_content_has_artifact_payload(&tc.content) {
                contract = Some(any_artifact_contract());
            }
            let output_candidates = output_candidate_paths(
                Some(&tc.content),
                &tc.locations,
                tc.raw_input.as_ref(),
                tc.raw_output.as_ref(),
            );
            let path_scan_error = detected_contract.error.or(output_candidates.error);
            let (contract, candidate_paths, started_at) = delivery_state.observe_tool_metadata(
                &session_id,
                &tool_call_id,
                contract,
                output_candidates.paths,
                requested_status,
                artifact_store,
            );
            let path_delivery = if let Some(contract) = contract {
                if let Some(error) = path_scan_error {
                    Err(error)
                } else if requested_status == Some(AcpToolCallStatus::Completed) {
                    verify_completed_path_artifacts(
                        artifact_store,
                        &candidate_paths,
                        started_at,
                        contract,
                    )
                } else {
                    Ok(Vec::new())
                }
            } else {
                Ok(Vec::new())
            };
            // Path candidates are preflighted before inline content is written;
            // an invalid/stale path therefore cannot leave a partial inline
            // batch behind.
            let mut mapped_content = if path_delivery.is_ok() {
                map_tool_call_content(&tc.content, artifact_store)
            } else {
                map_tool_call_content_without_artifact_writes(&tc.content)
            };
            match path_delivery {
                Ok(artifacts) => mapped_content.ensure_artifact_receipts(&artifacts),
                Err(error) => {
                    mapped_content.delivery_error.get_or_insert(error.clone());
                    mapped_content.ensure_error_item(&error);
                }
            }
            let verified_artifacts = mapped_content.verified_artifacts();
            let delivery = delivery_state.apply_tool_update(
                &session_id,
                &tool_call_id,
                contract,
                &verified_artifacts,
                requested_status,
                mapped_content.delivery_error.clone(),
            );
            if delivery.force_failed {
                mapped_content.remove_artifact_receipts();
            }
            if let Some(error) = delivery.failure.as_deref() {
                mapped_content.ensure_error_item(error);
            }
            mapped_content.ensure_artifact_receipts(&delivery.releasable_artifacts);
            events.push(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
                session_id,
                update: AcpToolCallUpdateData {
                    session_update: AcpToolCallSessionUpdateKind::ToolCall,
                    tool_call_id,
                    status: Some(if delivery.force_failed {
                        AcpToolCallStatus::Failed
                    } else {
                        requested_status.expect("tool call status is always present")
                    }),
                    title: Some(tc.title.clone()),
                    kind: Some(map_sdk_tool_kind(&tc.kind)),
                    raw_input: tc.raw_input.clone(),
                    raw_output: tc.raw_output.clone(),
                    content: mapped_content.items,
                    locations: map_tool_call_locations(&tc.locations),
                },
                meta: tc.meta.clone(),
            }));
        }

        SessionUpdate::ToolCallUpdate(tcu) => {
            let tool_call_id = tcu.tool_call_id.to_string();
            let requested_status = tcu.fields.status.as_ref().map(map_sdk_tool_status);
            let detected_contract = tool_artifact_contract(
                tcu.fields.title.as_deref(),
                tcu.fields.raw_input.as_ref(),
                tcu.fields.raw_output.as_ref(),
            );
            let mut contract = detected_contract.contract;
            if contract.is_none()
                && tcu
                    .fields
                    .content
                    .as_deref()
                    .is_some_and(tool_content_has_artifact_payload)
            {
                contract = Some(any_artifact_contract());
            }
            let output_candidates = output_candidate_paths(
                tcu.fields.content.as_deref(),
                tcu.fields.locations.as_deref().unwrap_or_default(),
                tcu.fields.raw_input.as_ref(),
                tcu.fields.raw_output.as_ref(),
            );
            let path_scan_error = detected_contract.error.or(output_candidates.error);
            let (contract, candidate_paths, started_at) = delivery_state.observe_tool_metadata(
                &session_id,
                &tool_call_id,
                contract,
                output_candidates.paths,
                requested_status,
                artifact_store,
            );
            let path_delivery = if let Some(contract) = contract {
                if let Some(error) = path_scan_error {
                    Err(error)
                } else if requested_status == Some(AcpToolCallStatus::Completed) {
                    verify_completed_path_artifacts(
                        artifact_store,
                        &candidate_paths,
                        started_at,
                        contract,
                    )
                } else {
                    Ok(Vec::new())
                }
            } else {
                Ok(Vec::new())
            };
            let mut mapped_content = tcu.fields.content.as_ref().map(|content| {
                if path_delivery.is_ok() {
                    map_tool_call_content(content, artifact_store)
                } else {
                    map_tool_call_content_without_artifact_writes(content)
                }
            });
            match path_delivery {
                Ok(artifacts) if !artifacts.is_empty() => mapped_content
                    .get_or_insert_with(MappedToolContent::default)
                    .ensure_artifact_receipts(&artifacts),
                Err(error) => {
                    let mapped = mapped_content.get_or_insert_with(MappedToolContent::default);
                    mapped.delivery_error.get_or_insert(error.clone());
                    mapped.ensure_error_item(&error);
                }
                Ok(_) => {}
            }
            let verified_artifacts = mapped_content
                .as_ref()
                .map(MappedToolContent::verified_artifacts)
                .unwrap_or_default();
            let delivery = delivery_state.apply_tool_update(
                &session_id,
                &tool_call_id,
                contract,
                &verified_artifacts,
                requested_status,
                mapped_content
                    .as_ref()
                    .and_then(|mapped| mapped.delivery_error.clone()),
            );
            if delivery.force_failed
                && let Some(mapped) = mapped_content.as_mut()
            {
                mapped.remove_artifact_receipts();
            }
            if let Some(error) = delivery.failure.as_deref()
                && let Some(mapped) = mapped_content.as_mut()
            {
                mapped.ensure_error_item(error);
            }
            if !delivery.releasable_artifacts.is_empty() {
                mapped_content
                    .get_or_insert_with(MappedToolContent::default)
                    .ensure_artifact_receipts(&delivery.releasable_artifacts);
            }
            let mapped_items = mapped_content
                .as_ref()
                .and_then(|mapped| mapped.items.clone())
                .or_else(|| {
                    delivery
                        .failure
                        .clone()
                        .map(|message| vec![AcpToolCallContentItem::ArtifactError { message }])
                });
            events.push(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
                session_id,
                update: AcpToolCallUpdateData {
                    session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                    tool_call_id,
                    status: if delivery.force_failed {
                        Some(AcpToolCallStatus::Failed)
                    } else {
                        requested_status
                    },
                    title: tcu.fields.title.clone(),
                    kind: tcu.fields.kind.as_ref().map(map_sdk_tool_kind),
                    raw_input: tcu.fields.raw_input.clone(),
                    raw_output: tcu.fields.raw_output.clone(),
                    content: mapped_items,
                    locations: tcu
                        .fields
                        .locations
                        .as_ref()
                        .and_then(|locations| map_tool_call_locations(locations)),
                },
                meta: tcu.meta.clone(),
            }));
        }

        SessionUpdate::Plan(plan) => {
            let entries: Vec<serde_json::Value> = plan
                .entries
                .iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default())
                .collect();

            events.push(AgentStreamEvent::Plan(PlanEventData {
                session_id: Some(session_id),
                source_call_id: None,
                entries,
            }));
        }

        SessionUpdate::AvailableCommandsUpdate(update) => {
            events.push(AgentStreamEvent::AvailableCommands(AvailableCommandsEventData {
                commands: update.available_commands.clone(),
            }));
        }

        SessionUpdate::CurrentModeUpdate(update) => {
            events.push(AgentStreamEvent::AcpModeInfo(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }

        SessionUpdate::ConfigOptionUpdate(update) => {
            events.push(AgentStreamEvent::AcpConfigOption(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }

        SessionUpdate::SessionInfoUpdate(update) => {
            events.push(AgentStreamEvent::AcpSessionInfo(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }

        SessionUpdate::UsageUpdate(update) => {
            events.push(AgentStreamEvent::AcpContextUsage(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }
        _ => {
            debug!("Unknown SessionUpdate variant received, skipping");
        }
    }

    events
}

pub(crate) fn permission_request_to_event_data(request: &RequestPermissionRequest) -> AcpPermissionEventData {
    AcpPermissionEventData::Request(AcpPermissionRequestData {
        session_id: request.session_id.to_string(),
        tool_call: map_permission_tool_call(&request.tool_call),
        options: request.options.iter().map(map_permission_option).collect(),
        meta: request.meta.clone(),
    })
}

fn map_sdk_tool_status(sdk: &SdkToolCallStatus) -> AcpToolCallStatus {
    match sdk {
        SdkToolCallStatus::Pending => AcpToolCallStatus::Pending,
        SdkToolCallStatus::InProgress => AcpToolCallStatus::InProgress,
        SdkToolCallStatus::Completed => AcpToolCallStatus::Completed,
        SdkToolCallStatus::Failed => AcpToolCallStatus::Failed,
        _ => AcpToolCallStatus::Pending,
    }
}

fn map_sdk_tool_kind(kind: &SdkToolKind) -> AcpToolCallKind {
    match kind {
        SdkToolKind::Read | SdkToolKind::Search => AcpToolCallKind::Read,
        SdkToolKind::Edit | SdkToolKind::Delete | SdkToolKind::Move => AcpToolCallKind::Edit,
        SdkToolKind::Execute
        | SdkToolKind::Think
        | SdkToolKind::Fetch
        | SdkToolKind::SwitchMode
        | SdkToolKind::Other
        | _ => AcpToolCallKind::Execute,
    }
}

fn map_sdk_permission_option_kind(kind: SdkPermissionOptionKind) -> AcpPermissionOptionKind {
    match kind {
        SdkPermissionOptionKind::AllowOnce => AcpPermissionOptionKind::AllowOnce,
        SdkPermissionOptionKind::AllowAlways => AcpPermissionOptionKind::AllowAlways,
        SdkPermissionOptionKind::RejectOnce => AcpPermissionOptionKind::RejectOnce,
        SdkPermissionOptionKind::RejectAlways => AcpPermissionOptionKind::RejectAlways,
        _ => AcpPermissionOptionKind::RejectOnce,
    }
}

fn map_permission_tool_call(tool_call: &SdkToolCallUpdate) -> AcpPermissionToolCall {
    AcpPermissionToolCall {
        tool_call_id: tool_call.tool_call_id.to_string(),
        status: tool_call.fields.status.as_ref().map(map_sdk_tool_status),
        title: tool_call.fields.title.clone(),
        kind: tool_call.fields.kind.as_ref().map(map_sdk_tool_kind),
        raw_input: tool_call.fields.raw_input.clone(),
        raw_output: tool_call.fields.raw_output.clone(),
        content: tool_call
            .fields
            .content
            .as_ref()
            .and_then(|content| map_tool_call_content(content, None).items),
        locations: tool_call
            .fields
            .locations
            .as_ref()
            .and_then(|locations| map_tool_call_locations(locations)),
        meta: tool_call.meta.clone(),
    }
}

fn map_permission_option(option: &PermissionOption) -> AcpPermissionOptionData {
    AcpPermissionOptionData {
        option_id: option.option_id.to_string(),
        name: option.name.clone(),
        kind: map_sdk_permission_option_kind(option.kind),
        meta: option.meta.clone(),
    }
}

#[derive(Default)]
struct MappedToolContent {
    items: Option<Vec<AcpToolCallContentItem>>,
    delivery_error: Option<String>,
}

impl MappedToolContent {
    fn remove_artifact_receipts(&mut self) {
        if let Some(items) = self.items.as_mut() {
            items.retain(|item| !matches!(item, AcpToolCallContentItem::Artifact { .. }));
        }
    }

    fn ensure_error_item(&mut self, error: &str) {
        let items = self.items.get_or_insert_with(Vec::new);
        if !items.iter().any(
            |item| matches!(item, AcpToolCallContentItem::ArtifactError { message } if message == error),
        ) {
            items.push(AcpToolCallContentItem::ArtifactError {
                message: error.to_owned(),
            });
        }
    }

    fn verified_artifacts(&self) -> Vec<PersistedArtifact> {
        self.items
            .as_ref()
            .into_iter()
            .flatten()
            .filter_map(|item| match item {
                AcpToolCallContentItem::Artifact { artifact, .. } => Some(artifact.clone()),
                _ => None,
            })
            .collect()
    }

    fn ensure_artifact_receipts(&mut self, artifacts: &[PersistedArtifact]) {
        if artifacts.is_empty() {
            return;
        }
        let items = self.items.get_or_insert_with(Vec::new);
        for artifact in artifacts {
            if items.iter().any(
                |item| matches!(item, AcpToolCallContentItem::Artifact { artifact: existing, .. }
                    if existing.path == artifact.path && existing.sha256 == artifact.sha256),
            ) {
                continue;
            }
            items.push(AcpToolCallContentItem::Artifact {
                artifact: artifact.clone(),
                source_uri: None,
            });
        }
    }
}

fn output_candidate_paths(
    content: Option<&[SdkToolCallContent]>,
    locations: &[SdkToolCallLocation],
    raw_input: Option<&serde_json::Value>,
    raw_output: Option<&serde_json::Value>,
) -> ArtifactPathScan {
    let mut output = ArtifactPathScan::default();
    for location in locations {
        push_artifact_path(
            &mut output,
            &location.path.to_string_lossy(),
            "ACP tool location",
        );
    }
    if let Some(content) = content {
        for item in content {
            if let SdkToolCallContent::Diff(diff) = item {
                push_artifact_path(&mut output, &diff.path.to_string_lossy(), "ACP diff path");
            }
        }
    }
    for (value, allow_file_path) in [(raw_input, false), (raw_output, true)] {
        let Some(value) = value else {
            continue;
        };
        let scan = scan_artifact_path_candidates(value, allow_file_path);
        output.saw_path_key |= scan.saw_path_key;
        if output.error.is_none() {
            output.error = scan.error;
        }
        for path in scan.paths {
            push_artifact_path(&mut output, &path, "ACP artifact output");
        }
    }
    output
}

fn tool_content_has_artifact_payload(content: &[SdkToolCallContent]) -> bool {
    content.iter().any(|item| {
        matches!(
            item,
            SdkToolCallContent::Content(content)
                if content_block_is_artifact_payload(&content.content)
        )
    })
}

#[derive(Debug, Default)]
struct ArtifactPathScan {
    paths: Vec<String>,
    saw_path_key: bool,
    error: Option<String>,
}

fn artifact_path_key(key: &str, allow_file_path: bool) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "outputpath"
            | "outputpaths"
            | "outputfile"
            | "outputfiles"
            | "artifactpath"
            | "artifactpaths"
            | "artifactfile"
            | "artifactfiles"
            | "resultpath"
            | "resultpaths"
            | "resultfile"
            | "resultfiles"
            | "savepath"
            | "savepaths"
            | "savefile"
            | "savefiles"
            | "destinationpath"
            | "destinationpaths"
            | "destinationfile"
            | "destinationfiles"
    ) || (allow_file_path
        && matches!(
            normalized.as_str(),
            "filepath" | "filepaths" | "path" | "paths"
        ))
}

fn record_artifact_path_error(scan: &mut ArtifactPathScan, error: impl Into<String>) {
    if scan.error.is_none() {
        scan.error = Some(error.into());
    }
}

fn push_artifact_path(scan: &mut ArtifactPathScan, path: &str, source: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        record_artifact_path_error(scan, format!("{source} contains an empty artifact path"));
        return;
    }
    if trimmed.len() > MAX_ACP_ARTIFACT_PATH_LENGTH {
        record_artifact_path_error(
            scan,
            format!(
                "{source} exceeds the {MAX_ACP_ARTIFACT_PATH_LENGTH}-byte artifact path limit"
            ),
        );
        return;
    }
    if trimmed.chars().any(char::is_control) {
        record_artifact_path_error(scan, format!("{source} contains control characters"));
        return;
    }
    let normalized = url::Url::parse(trimmed)
        .ok()
        .filter(|uri| uri.scheme() == "file")
        .and_then(|uri| uri.to_file_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.to_owned());
    if scan.paths.contains(&normalized) {
        return;
    }
    if scan.paths.len() >= MAX_ACP_ARTIFACT_PATHS {
        record_artifact_path_error(
            scan,
            format!("ACP artifact output exceeds the {MAX_ACP_ARTIFACT_PATHS}-path limit"),
        );
        return;
    }
    scan.paths.push(normalized);
}

fn scan_artifact_path_candidates(
    value: &serde_json::Value,
    allow_file_path: bool,
) -> ArtifactPathScan {
    fn enter_node(
        depth: usize,
        nodes: &mut usize,
        scan: &mut ArtifactPathScan,
    ) -> bool {
        if depth > MAX_ACP_ARTIFACT_JSON_DEPTH {
            record_artifact_path_error(
                scan,
                format!(
                    "ACP artifact output exceeds the JSON depth limit of {MAX_ACP_ARTIFACT_JSON_DEPTH}"
                ),
            );
            return false;
        }
        if *nodes >= MAX_ACP_ARTIFACT_JSON_NODES {
            record_artifact_path_error(
                scan,
                format!(
                    "ACP artifact output exceeds the {MAX_ACP_ARTIFACT_JSON_NODES}-node JSON limit"
                ),
            );
            return false;
        }
        *nodes += 1;
        true
    }

    fn collect_path_values(
        value: &serde_json::Value,
        depth: usize,
        nodes: &mut usize,
        scan: &mut ArtifactPathScan,
    ) {
        if !enter_node(depth, nodes, scan) {
            return;
        }
        match value {
            serde_json::Value::String(path) => {
                push_artifact_path(scan, path, "ACP explicit artifact path")
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    collect_path_values(value, depth + 1, nodes, scan);
                }
            }
            _ => record_artifact_path_error(
                scan,
                "ACP explicit artifact path must be a string or an array of strings",
            ),
        }
    }

    fn visit(
        value: &serde_json::Value,
        depth: usize,
        nodes: &mut usize,
        allow_file_path: bool,
        scan: &mut ArtifactPathScan,
    ) {
        if !enter_node(depth, nodes, scan) {
            return;
        }
        match value {
            serde_json::Value::Object(object) => {
                for (key, value) in object {
                    if artifact_path_key(key, allow_file_path) {
                        scan.saw_path_key = true;
                        collect_path_values(value, depth + 1, nodes, scan);
                    } else if value.is_object() || value.is_array() {
                        visit(value, depth + 1, nodes, allow_file_path, scan);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    visit(value, depth + 1, nodes, allow_file_path, scan);
                }
            }
            _ => {}
        }
    }

    let mut scan = ArtifactPathScan::default();
    let mut nodes = 0;
    visit(value, 0, &mut nodes, allow_file_path, &mut scan);
    scan
}

#[cfg(test)]
fn artifact_path_candidates(value: &serde_json::Value, allow_file_path: bool) -> Vec<String> {
    scan_artifact_path_candidates(value, allow_file_path).paths
}

fn capture_artifact_path_baseline(
    store: &ArtifactStore,
    path: &str,
) -> ArtifactPathBaseline {
    match store.verify_existing_path(path) {
        Ok(artifact) => ArtifactPathBaseline::Present {
            size_bytes: artifact.size_bytes,
            sha256: artifact.sha256,
        },
        Err(ArtifactStoreError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            ArtifactPathBaseline::Absent
        }
        Err(error) => ArtifactPathBaseline::Error(error.to_string()),
    }
}

fn verify_completed_path_artifacts(
    artifact_store: Option<&ArtifactStore>,
    candidate_paths: &[ArtifactPathCandidate],
    started_at: Option<SystemTime>,
    contract: ArtifactContract,
) -> Result<Vec<PersistedArtifact>, String> {
    if candidate_paths.is_empty() {
        return Ok(Vec::new());
    }
    let store = artifact_store.ok_or_else(|| "session has no workspace artifact store".to_owned())?;
    if started_at.is_none() {
        return Err(
            "ACP completed with an output path but no pre-terminal tool-call baseline; refusing a potentially stale file"
                .to_owned(),
        );
    }

    let mut verified_sources = Vec::with_capacity(candidate_paths.len());
    for candidate in candidate_paths {
        if !candidate.observed_before_terminal {
            return Err(format!(
                "ACP output path `{}` was first declared at completion and has no pre-call fingerprint",
                candidate.path
            ));
        }
        let artifact = store
            .verify_existing_path(&candidate.path)
            .map_err(|error| format!("ACP output path `{}` failed verification: {error}", candidate.path))?;
        if !contract_accepts_artifact(contract, &artifact) {
            return Err(format!(
                "ACP output path `{}` delivered {} ({:?}), expected {}",
                candidate.path,
                artifact.mime_type,
                artifact.kind,
                contract.label()
            ));
        }
        match &candidate.baseline {
            ArtifactPathBaseline::Absent => {}
            ArtifactPathBaseline::Present { size_bytes, sha256 } => {
                let content_changed = *size_bytes != artifact.size_bytes
                    || !sha256.eq_ignore_ascii_case(&artifact.sha256);
                if !content_changed {
                    return Err(format!(
                        "ACP output path `{}` is unchanged from its pre-call fingerprint",
                        candidate.path
                    ));
                }
            }
            ArtifactPathBaseline::Error(error) => {
                return Err(format!(
                    "ACP output path `{}` had an unverifiable pre-call baseline: {error}",
                    candidate.path
                ));
            }
        }
        if verified_sources
            .iter()
            .any(|known: &PersistedArtifact| known.path == artifact.path)
        {
            continue;
        }
        verified_sources.push(artifact);
    }
    let snapshots = store
        .import_existing_batch(verified_sources.iter().map(|artifact| artifact.path.as_str()))
        .map_err(|error| format!("ACP output path snapshot import failed: {error}"))?;
    if snapshots.len() != verified_sources.len()
        || snapshots
            .iter()
            .zip(&verified_sources)
            .any(|(snapshot, source)| {
                snapshot.size_bytes != source.size_bytes
                    || !snapshot.sha256.eq_ignore_ascii_case(&source.sha256)
            })
    {
        return Err(
            "ACP output path changed while its immutable artifact snapshot was being created"
                .to_owned(),
        );
    }
    Ok(snapshots)
}

fn map_tool_call_content_without_artifact_writes(content: &[SdkToolCallContent]) -> MappedToolContent {
    let mut items = Vec::new();
    for item in content {
        match item {
            SdkToolCallContent::Content(content) => match &content.content {
                ContentBlock::Text(text) => items.push(AcpToolCallContentItem::Content {
                    content: AcpToolCallTextBlock {
                        block_type: AcpToolCallTextBlockType::Text,
                        text: text.text.clone(),
                    },
                }),
                _ => {}
            },
            SdkToolCallContent::Diff(diff) => items.push(AcpToolCallContentItem::Diff {
                path: diff.path.to_string_lossy().into_owned(),
                old_text: diff.old_text.clone(),
                new_text: diff.new_text.clone(),
            }),
            SdkToolCallContent::Terminal(terminal) => {
                items.push(AcpToolCallContentItem::Terminal {
                    terminal_id: terminal.terminal_id.to_string(),
                });
            }
            _ => {}
        }
    }
    MappedToolContent {
        items: (!items.is_empty()).then_some(items),
        delivery_error: None,
    }
}

fn map_tool_call_content(
    content: &[SdkToolCallContent],
    artifact_store: Option<&ArtifactStore>,
) -> MappedToolContent {
    let mut items = Vec::new();
    let mut delivery_error = None;

    // Materialize every inline payload and local file ResourceLink as one
    // immutable batch. Preflight non-local artifact blocks before the batch
    // write as well: otherwise a valid image followed by an invalid/transient
    // ResourceLink could leave an orphaned image even though the ACP content
    // array is rejected as a whole.
    let mut batch_indexes = Vec::new();
    let mut inline_plans = Vec::new();
    let mut existing_plans = Vec::new();
    let mut batch_planning_error = None;
    let mut prevalidated_non_inline = HashMap::new();
    let mut non_inline_preflight_error = None;
    for (index, item) in content.iter().enumerate() {
        let SdkToolCallContent::Content(content) = item else {
            continue;
        };
        if let Some(plan) = existing_artifact_plan(&content.content) {
            batch_indexes.push(index);
            if batch_indexes.len() > MAX_ACP_ARTIFACT_PATHS {
                batch_planning_error.get_or_insert_with(|| {
                    format!(
                        "ACP artifact content exceeds the {MAX_ACP_ARTIFACT_PATHS}-item batch limit"
                    )
                });
            }
            match plan {
                Ok(plan) => {
                    if plan.path.to_string_lossy().len() > MAX_ACP_ARTIFACT_PATH_LENGTH {
                        batch_planning_error.get_or_insert_with(|| {
                            format!(
                                "ACP file ResourceLink exceeds the {MAX_ACP_ARTIFACT_PATH_LENGTH}-byte path limit"
                            )
                        });
                    }
                    existing_plans.push((index, plan));
                }
                Err(error) => {
                    batch_planning_error.get_or_insert(error);
                }
            }
            continue;
        }
        let Some(plan) = inline_artifact_plan(&content.content) else {
            if content_block_is_artifact_payload(&content.content) {
                let mapped = map_content_block(&content.content, artifact_store);
                if let Err(error) = &mapped {
                    non_inline_preflight_error.get_or_insert_with(|| error.clone());
                }
                prevalidated_non_inline.insert(index, mapped);
            }
            continue;
        };
        batch_indexes.push(index);
        if batch_indexes.len() > MAX_ACP_ARTIFACT_PATHS {
            batch_planning_error.get_or_insert_with(|| {
                format!(
                    "ACP artifact content exceeds the {MAX_ACP_ARTIFACT_PATHS}-item batch limit"
                )
            });
        }
        match plan {
            Ok(plan) => inline_plans.push((index, plan)),
            Err(error) => {
                batch_planning_error.get_or_insert(error);
            }
        }
    }

    let mut batch_receipts: HashMap<usize, Result<(PersistedArtifact, Option<String>), String>> =
        HashMap::new();
    if !batch_indexes.is_empty() {
        let batch_result = if let Some(error) = batch_planning_error.or(non_inline_preflight_error) {
            Err(error)
        } else if let Some(store) = artifact_store {
            store
                .persist_inline_and_existing_batch(
                    inline_plans
                        .iter()
                        .map(|(_, plan)| (plan.kind, plan.mime_type.as_str(), plan.data.as_str())),
                    existing_plans.iter().map(|(_, plan)| &plan.path),
                )
                .map_err(|error| error.to_string())
                .and_then(|artifacts| {
                    if artifacts.len() != inline_plans.len() + existing_plans.len() {
                        return Err("artifact batch receipt count did not match the requested payload count".to_owned());
                    }
                    let mut artifacts = artifacts.into_iter();
                    let mut receipts = Vec::with_capacity(batch_indexes.len());
                    for (index, plan) in &existing_plans {
                        let artifact = artifacts.next().ok_or_else(|| {
                            "artifact batch omitted an existing-path receipt".to_owned()
                        })?;
                        receipts.push((*index, (artifact, Some(plan.source_uri.clone()))));
                    }
                    for (index, plan) in &inline_plans {
                        let artifact = artifacts.next().ok_or_else(|| {
                            "artifact batch omitted an inline receipt".to_owned()
                        })?;
                        receipts.push((*index, (artifact, plan.source_uri.clone())));
                    }
                    Ok(receipts)
                })
        } else {
            Err("session has no workspace artifact store".to_owned())
        };

        match batch_result {
            Ok(receipts) => {
                batch_receipts.extend(receipts.into_iter().map(|(index, receipt)| (index, Ok(receipt))))
            }
            Err(error) => {
                batch_receipts.extend(
                    batch_indexes
                        .into_iter()
                        .map(|index| (index, Err(error.clone()))),
                );
            }
        }
    }

    for (index, item) in content.iter().enumerate() {
        let mapped = match item {
            SdkToolCallContent::Content(content) => {
                if let Some(receipt) = batch_receipts.remove(&index) {
                    receipt.map(|(artifact, source_uri)| AcpToolCallContentItem::Artifact {
                        artifact,
                        source_uri,
                    })
                } else if let Some(mapped) = prevalidated_non_inline.remove(&index) {
                    mapped
                } else {
                    map_content_block(&content.content, artifact_store)
                }
            }
            SdkToolCallContent::Diff(diff) => Ok(AcpToolCallContentItem::Diff {
                path: diff.path.to_string_lossy().into_owned(),
                old_text: diff.old_text.clone(),
                new_text: diff.new_text.clone(),
            }),
            SdkToolCallContent::Terminal(terminal) => Ok(AcpToolCallContentItem::Terminal {
                terminal_id: terminal.terminal_id.to_string(),
            }),
            _ => continue,
        };

        match mapped {
            Ok(item) => items.push(item),
            Err(error) => {
                if delivery_error.is_none() {
                    delivery_error = Some(error.clone());
                }
                items.push(AcpToolCallContentItem::ArtifactError { message: error });
            }
        }
    }

    MappedToolContent {
        items: (!items.is_empty()).then_some(items),
        delivery_error,
    }
}

struct InlineArtifactPlan {
    kind: ArtifactKind,
    mime_type: String,
    data: String,
    source_uri: Option<String>,
}

struct ExistingArtifactPlan {
    path: PathBuf,
    source_uri: String,
}

fn existing_artifact_plan(
    block: &ContentBlock,
) -> Option<Result<ExistingArtifactPlan, String>> {
    let ContentBlock::ResourceLink(resource) = block else {
        return None;
    };
    let (source_uri, parsed) = match validate_durable_resource_uri(&resource.uri) {
        Ok(parsed) => parsed,
        Err(error) => return Some(Err(error)),
    };
    if parsed.scheme() != "file" {
        return None;
    }
    Some(
        parsed
            .to_file_path()
            .map(|path| ExistingArtifactPlan { path, source_uri })
            .map_err(|_| "ACP file resource URI is not a valid local path".to_owned()),
    )
}

fn inline_artifact_plan(block: &ContentBlock) -> Option<Result<InlineArtifactPlan, String>> {
    match block {
        ContentBlock::Image(image) => Some(Ok(InlineArtifactPlan {
            kind: ArtifactKind::Image,
            mime_type: image.mime_type.clone(),
            data: image.data.clone(),
            source_uri: durable_source_uri(image.uri.as_deref()),
        })),
        ContentBlock::Audio(audio) => Some(Ok(InlineArtifactPlan {
            kind: ArtifactKind::Audio,
            mime_type: audio.mime_type.clone(),
            data: audio.data.clone(),
            source_uri: None,
        })),
        ContentBlock::Resource(resource) => Some(match &resource.resource {
            EmbeddedResourceResource::TextResourceContents(text) => Ok(InlineArtifactPlan {
                kind: ArtifactKind::Text,
                mime_type: text.mime_type.clone().unwrap_or_else(|| "text/plain".to_owned()),
                data: base64::engine::general_purpose::STANDARD.encode(text.text.as_bytes()),
                source_uri: durable_source_uri(Some(&text.uri)),
            }),
            EmbeddedResourceResource::BlobResourceContents(blob) => {
                let mime_type = blob
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_owned());
                Ok(InlineArtifactPlan {
                    kind: artifact_kind_for_mime(&mime_type),
                    mime_type,
                    data: blob.blob.clone(),
                    source_uri: durable_source_uri(Some(&blob.uri)),
                })
            }
            _ => Err("unsupported embedded ACP resource".to_owned()),
        }),
        _ => None,
    }
}

fn content_block_is_artifact_payload(block: &ContentBlock) -> bool {
    matches!(
        block,
        ContentBlock::Image(_)
            | ContentBlock::Audio(_)
            | ContentBlock::Resource(_)
            | ContentBlock::ResourceLink(_)
    )
}

#[derive(Debug, Default)]
struct ToolArtifactContract {
    contract: Option<ArtifactContract>,
    error: Option<String>,
}

impl ToolArtifactContract {
    fn merge_observed(&mut self, observed: Self) {
        if self.error.is_none() {
            self.error = observed.error;
        }
        match (self.contract, observed.contract) {
            (None, contract) => self.contract = contract,
            (Some(current), Some(observed)) => match current.merge(observed) {
                Ok(merged) => self.contract = Some(merged),
                Err(reason) => {
                    self.error.get_or_insert_with(|| {
                        format!("conflicting artifact contract: {reason}")
                    });
                }
            },
            (Some(_), None) => {}
        }
    }
}

fn identity_artifact_contract(
    identity: &str,
    count_input: Option<&serde_json::Value>,
) -> ToolArtifactContract {
    if let Some(input) = count_input {
        match artifact_contract_with_input(identity, input) {
            Ok(contract) => ToolArtifactContract {
                contract,
                error: None,
            },
            Err(error) => ToolArtifactContract {
                // Keep the identity-level obligation even when its count is
                // malformed; the metadata error is absorbing and fails the
                // call without losing the expected product/format.
                contract: artifact_contract(identity),
                error: Some(format!(
                    "invalid artifact contract for `{identity}`: {error}"
                )),
            },
        }
    } else {
        ToolArtifactContract {
            contract: artifact_contract(identity),
            error: None,
        }
    }
}

fn tool_artifact_contract(
    title: Option<&str>,
    raw_input: Option<&serde_json::Value>,
    raw_output: Option<&serde_json::Value>,
) -> ToolArtifactContract {
    // Preserve the most specific identity contract. Reducing this to a boolean
    // lets an image generator claim success with a valid text/audio/blob
    // receipt, which is still a failed image-generation task.
    let mut detected = ToolArtifactContract::default();
    if let Some(title) = title {
        detected.merge_observed(identity_artifact_contract(title, raw_input));
    }
    if let Some(input) = raw_input {
        for observed in value_artifact_contracts(input, true) {
            detected.merge_observed(observed);
        }
    }
    if let Some(output) = raw_output {
        for observed in value_artifact_contracts(output, false) {
            detected.merge_observed(observed);
        }
    }
    if detected.contract.is_some() {
        return detected;
    }

    // An explicit output/artifact/result destination is itself a
    // machine-readable delivery contract, including for third-party tools
    // whose names are unknown to us. Generic `path`/`filePath` fields are
    // deliberately excluded here: read/inspect tools commonly return them as
    // context and must not be mistaken for generators.
    if [raw_input, raw_output]
        .into_iter()
        .flatten()
        .any(|value| scan_artifact_path_candidates(value, false).saw_path_key)
    {
        detected.contract = Some(any_artifact_contract());
        return detected;
    }

    // A generic `path` appearing only in a terminal result is ambiguous. For
    // a known read/inspect/edit/command tool it is ordinary context; for an
    // unknown worker it may be the only machine-readable claim that a file was
    // produced. Fail the unknown case closed so a stale pre-existing path can
    // never turn a task green without a pre-terminal fingerprint.
    if raw_output.is_some_and(|value| scan_artifact_path_candidates(value, true).saw_path_key)
        && !tool_is_context_only(title, raw_input, raw_output)
    {
        detected.contract = Some(any_artifact_contract());
    }
    detected
}

#[cfg(test)]
fn tool_artifact_expectation(
    title: Option<&str>,
    raw_input: Option<&serde_json::Value>,
    raw_output: Option<&serde_json::Value>,
) -> ArtifactExpectation {
    tool_artifact_contract(title, raw_input, raw_output)
        .contract
        .map_or(ArtifactExpectation::None, |contract| contract.expectation)
}

#[cfg(test)]
fn tool_expects_artifact(
    title: Option<&str>,
    raw_input: Option<&serde_json::Value>,
    raw_output: Option<&serde_json::Value>,
) -> bool {
    tool_artifact_expectation(title, raw_input, raw_output) != ArtifactExpectation::None
}

fn tool_is_context_only(
    title: Option<&str>,
    raw_input: Option<&serde_json::Value>,
    raw_output: Option<&serde_json::Value>,
) -> bool {
    title.is_some_and(context_tool_label)
        || [raw_input, raw_output]
            .into_iter()
            .flatten()
            .any(value_declares_context_tool)
}

fn value_declares_context_tool(value: &serde_json::Value) -> bool {
    let serde_json::Value::Object(object) = value else {
        return false;
    };
    const IDENTITY_KEYS: &[&str] = &[
        "tool",
        "tool_name",
        "toolName",
        "name",
        "operation",
        "operation_name",
        "operationName",
    ];
    IDENTITY_KEYS.iter().any(|key| {
        object
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(context_tool_label)
    })
}

fn context_tool_label(label: &str) -> bool {
    if is_context_only_image_tool(label) {
        return true;
    }
    let mut normalized = String::with_capacity(label.len());
    let mut previous_was_lower_or_digit = false;
    for character in label.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase() && previous_was_lower_or_digit {
                normalized.push(' ');
            }
            normalized.push(character.to_ascii_lowercase());
            previous_was_lower_or_digit =
                character.is_ascii_lowercase() || character.is_ascii_digit();
        } else {
            normalized.push(' ');
            previous_was_lower_or_digit = false;
        }
    }
    normalized.split_whitespace().any(|word| {
        matches!(
            word,
            "read"
                | "view"
                | "viewer"
                | "inspect"
                | "search"
                | "find"
                | "list"
                | "stat"
                | "metadata"
                | "query"
                | "lookup"
                | "open"
                | "browse"
                | "browser"
                | "computer"
                | "execute"
                | "run"
                | "command"
                | "shell"
                | "terminal"
                | "edit"
                | "update"
                | "patch"
        )
    })
}

/// ACP does not expose a machine-readable output schema on tool-call updates,
/// so use only high-signal tool identity fields. Prompt/body text is
/// deliberately ignored to avoid classifying a shell command that merely
/// mentions an image as an image-producing tool.
fn value_artifact_contracts(
    value: &serde_json::Value,
    infer_count: bool,
) -> Vec<ToolArtifactContract> {
    let serde_json::Value::Object(object) = value else {
        return Vec::new();
    };
    const IDENTITY_KEYS: &[&str] = &[
        "tool",
        "tool_name",
        "toolName",
        "name",
        "operation",
        "operation_name",
        "operationName",
    ];
    IDENTITY_KEYS
        .iter()
        .filter_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
        .map(|identity| {
            identity_artifact_contract(identity, infer_count.then_some(value))
        })
        .filter(|detected| detected.contract.is_some() || detected.error.is_some())
        .collect()
}

#[cfg(test)]
fn artifact_tool_label(label: &str) -> bool {
    // Keep ACP and Nomi on the same identity-only classifier so camelCase,
    // concatenated MCP names and plurals cannot silently lose their artifact
    // contract.  In particular, ordinary ACP `Write file`/edit operations are
    // intentionally not generators: their diffs remain visible, while a true
    // export/generation tool must provide a durable receipt.
    artifact_contract(label).is_some()
}

fn map_content_block(
    block: &ContentBlock,
    artifact_store: Option<&ArtifactStore>,
) -> Result<AcpToolCallContentItem, String> {
    match block {
        ContentBlock::Text(text) => Ok(AcpToolCallContentItem::Content {
            content: AcpToolCallTextBlock {
                block_type: AcpToolCallTextBlockType::Text,
                text: text.text.clone(),
            },
        }),
        ContentBlock::Image(image) => {
            let artifact = persist_inline_required(
                artifact_store,
                ArtifactKind::Image,
                &image.mime_type,
                &image.data,
            )?;
            Ok(AcpToolCallContentItem::Artifact {
                artifact,
                source_uri: durable_source_uri(image.uri.as_deref()),
            })
        }
        ContentBlock::Audio(audio) => {
            let artifact = persist_inline_required(
                artifact_store,
                ArtifactKind::Audio,
                &audio.mime_type,
                &audio.data,
            )?;
            Ok(AcpToolCallContentItem::Artifact {
                artifact,
                source_uri: None,
            })
        }
        ContentBlock::ResourceLink(resource) => {
            let (uri, parsed) = validate_durable_resource_uri(&resource.uri)?;
            if parsed.scheme() == "file" {
                let path = parsed
                    .to_file_path()
                    .map_err(|_| "ACP file resource URI is not a valid local path".to_owned())?;
                let artifact = artifact_store
                    .ok_or_else(|| "session has no workspace artifact store".to_owned())?
                    .import_existing_path(path)
                    .map_err(|error| error.to_string())?;
                return Ok(AcpToolCallContentItem::Artifact {
                    artifact,
                    source_uri: Some(uri),
                });
            }
            Ok(AcpToolCallContentItem::ResourceLink {
                name: resource.name.clone(),
                uri,
                title: resource.title.clone(),
                description: resource.description.clone(),
                mime_type: resource.mime_type.clone(),
                size_bytes: resource.size,
            })
        }
        ContentBlock::Resource(resource) => match &resource.resource {
            EmbeddedResourceResource::TextResourceContents(text) => {
                let store = artifact_store.ok_or_else(|| "session has no workspace artifact store".to_owned())?;
                let artifact = store
                    .persist_text(text.mime_type.as_deref(), &text.text)
                    .map_err(|error| error.to_string())?;
                Ok(AcpToolCallContentItem::Artifact {
                    artifact,
                    source_uri: durable_source_uri(Some(&text.uri)),
                })
            }
            EmbeddedResourceResource::BlobResourceContents(blob) => {
                let mime = blob.mime_type.as_deref().unwrap_or("application/octet-stream");
                let kind = artifact_kind_for_mime(mime);
                let artifact = persist_inline_required(artifact_store, kind, mime, &blob.blob)?;
                Ok(AcpToolCallContentItem::Artifact {
                    artifact,
                    source_uri: durable_source_uri(Some(&blob.uri)),
                })
            }
            _ => Err("unsupported embedded ACP resource".to_owned()),
        },
        _ => Err("unsupported ACP content block".to_owned()),
    }
}

fn validate_durable_resource_uri(value: &str) -> Result<(String, url::Url), String> {
    let uri = value.trim();
    if uri.is_empty() || uri.len() > 8 * 1024 {
        return Err("ACP resource link has no durable addressable URI".to_owned());
    }
    let parsed = url::Url::parse(uri)
        .map_err(|_| "ACP resource link has no durable addressable URI".to_owned())?;
    if matches!(parsed.scheme(), "data" | "blob" | "javascript" | "about") {
        return Err("ACP resource link uses a transient or unsafe URI scheme".to_owned());
    }
    Ok((uri.to_owned(), parsed))
}

fn persist_inline_required(
    artifact_store: Option<&ArtifactStore>,
    kind: ArtifactKind,
    mime_type: &str,
    data: &str,
) -> Result<PersistedArtifact, String> {
    artifact_store
        .ok_or_else(|| "session has no workspace artifact store".to_owned())?
        .persist_inline(kind, mime_type, data)
        .map_err(|error| error.to_string())
}

fn artifact_kind_for_mime(mime: &str) -> ArtifactKind {
    if mime.starts_with("image/") {
        ArtifactKind::Image
    } else if mime.starts_with("audio/") {
        ArtifactKind::Audio
    } else if mime.starts_with("video/") {
        ArtifactKind::Video
    } else if mime.starts_with("text/") || mime == "application/json" {
        ArtifactKind::Text
    } else {
        ArtifactKind::File
    }
}

/// Preserve only short, addressable source URIs. Inline `data:` values have
/// already been persisted and must never be duplicated into event/history JSON.
fn durable_source_uri(uri: Option<&str>) -> Option<String> {
    uri.map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 8 * 1024)
        .filter(|value| !value.to_ascii_lowercase().starts_with("data:"))
        .map(str::to_owned)
}

fn map_agent_message_content(
    block: &ContentBlock,
    artifact_store: Option<&ArtifactStore>,
) -> Result<Option<String>, String> {
    if let ContentBlock::Text(text) = block {
        return Ok(Some(text.text.clone()));
    }
    let item = map_content_block(block, artifact_store)?;
    match item {
        AcpToolCallContentItem::Artifact { artifact, .. } => {
            let label = match artifact.kind {
                ArtifactKind::Image => "Generated image",
                ArtifactKind::Audio => "Generated audio",
                ArtifactKind::Video => "Generated video",
                ArtifactKind::Text => "Generated text artifact",
                ArtifactKind::File => "Generated file",
            };
            let target = url::Url::from_file_path(&artifact.path)
                .map(|url| url.to_string())
                .unwrap_or_else(|_| artifact.path.clone());
            let markdown = if artifact.kind == ArtifactKind::Image {
                format!("![{label}]({target})\n\n`{}`", artifact.path)
            } else {
                format!("[{label}]({target})\n\n`{}`", artifact.path)
            };
            Ok(Some(markdown))
        }
        AcpToolCallContentItem::ResourceLink { name, uri, .. } => {
            Ok(Some(format!("[{}]({uri})", escape_markdown_label(&name))))
        }
        AcpToolCallContentItem::Content { content } => Ok(Some(content.text)),
        AcpToolCallContentItem::Terminal { terminal_id } => {
            Ok(Some(format!("Terminal: `{terminal_id}`")))
        }
        AcpToolCallContentItem::Diff { path, .. } => Ok(Some(format!("Updated `{path}`"))),
        AcpToolCallContentItem::ArtifactError { message } => Err(message),
    }
}

fn escape_markdown_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]")
}

fn map_tool_call_locations(locations: &[SdkToolCallLocation]) -> Option<Vec<AcpToolCallLocationItem>> {
    (!locations.is_empty()).then(|| {
        locations
            .iter()
            .map(|loc| AcpToolCallLocationItem {
                path: loc.path.to_string_lossy().into_owned(),
            })
            .collect()
    })
}

#[cfg(test)]
mod artifact_contract_tests {
    use super::*;
    use serde_json::json;

    fn receipt(kind: ArtifactKind, mime_type: &str, suffix: &str) -> PersistedArtifact {
        PersistedArtifact {
            id: format!("artifact-{suffix}"),
            kind,
            mime_type: mime_type.to_owned(),
            path: format!("/workspace/nomifun-artifacts/{suffix}"),
            relative_path: format!("nomifun-artifacts/{suffix}"),
            size_bytes: 1,
            sha256: format!("sha256-{suffix}"),
        }
    }

    fn apply_completed_contract(
        session_id: &str,
        title: &str,
        raw_input: Option<&serde_json::Value>,
        artifacts: &[PersistedArtifact],
    ) -> (AcpArtifactDeliveryState, ToolDeliveryOutcome) {
        let detected = tool_artifact_contract(Some(title), raw_input, None);
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn(session_id);
        let (contract, _, _) = state.observe_tool_metadata(
            session_id,
            "call-artifact",
            detected.contract,
            std::iter::empty(),
            Some(AcpToolCallStatus::Completed),
            None,
        );
        let outcome = state.apply_tool_update(
            session_id,
            "call-artifact",
            contract,
            artifacts,
            Some(AcpToolCallStatus::Completed),
            detected.error,
        );
        (state, outcome)
    }

    #[test]
    fn explicit_plural_output_paths_create_an_artifact_contract() {
        let input = json!({
            "outputFiles": ["renders/one.png", "renders/two.png"],
        });

        assert!(tool_expects_artifact(None, Some(&input), None));
        assert_eq!(
            artifact_path_candidates(&input, false),
            vec!["renders/one.png", "renders/two.png"]
        );
    }

    #[test]
    fn generic_result_path_is_context_for_readers_but_a_contract_for_unknown_workers() {
        let output = json!({
            "path": "src/read_only.rs",
            "filePath": "src/also_read_only.rs",
        });

        assert!(!tool_expects_artifact(
            Some("Read file"),
            None,
            Some(&output)
        ));
        assert!(!tool_expects_artifact(
            Some("executeCommand"),
            None,
            Some(&output)
        ));
        assert!(tool_expects_artifact(
            Some("Worker"),
            None,
            Some(&output)
        ));
        assert!(artifact_path_candidates(&output, false).is_empty());
        assert_eq!(artifact_path_candidates(&output, true).len(), 2);
    }

    #[test]
    fn artifact_identity_contract_is_shared_with_nomi_and_excludes_code_edits() {
        for label in [
            "generateImage",
            "renderImages",
            "mcp__reports__exportReport",
            "textToSpeech",
            "createArtifacts",
        ] {
            assert!(artifact_tool_label(label), "{label} should require an artifact receipt");
        }

        for label in ["Write file", "editFile", "browserScreenshot", "readImage"] {
            assert!(
                !artifact_tool_label(label),
                "{label} is an edit/observation, not a generated artifact"
            );
        }
    }

    #[test]
    fn image_generator_rejects_non_image_receipts() {
        let contract = tool_artifact_contract(Some("generateImage"), None, None).contract;
        assert_eq!(
            contract.map(|contract| contract.expectation),
            Some(ArtifactExpectation::Image)
        );

        for (index, artifact) in [
            receipt(ArtifactKind::Text, "text/plain", "text.txt"),
            receipt(ArtifactKind::Audio, "audio/wav", "audio.wav"),
            receipt(
                ArtifactKind::File,
                "application/octet-stream",
                "opaque.bin",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let session_id = format!("session-{index}");
            let mut state = AcpArtifactDeliveryState::default();
            state.begin_turn(&session_id);
            let (stored_contract, _, _) = state.observe_tool_metadata(
                &session_id,
                "call-image",
                contract,
                std::iter::empty(),
                Some(AcpToolCallStatus::Completed),
                None,
            );
            let outcome = state.apply_tool_update(
                &session_id,
                "call-image",
                stored_contract,
                &[artifact],
                Some(AcpToolCallStatus::Completed),
                None,
            );

            assert!(outcome.force_failed);
            assert!(
                outcome
                    .failure
                    .as_deref()
                    .is_some_and(|error| error.contains("expected image artifact"))
            );
            assert!(state.finish_turn(&session_id).is_some());
        }
    }

    #[test]
    fn image_generator_rejects_receipt_with_image_mime_but_wrong_kind() {
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn("session-kind");
        let contract = tool_artifact_contract(Some("generateImage"), None, None).contract;
        let (contract, _, _) = state.observe_tool_metadata(
            "session-kind",
            "call-image",
            contract,
            std::iter::empty(),
            Some(AcpToolCallStatus::Completed),
            None,
        );
        let artifact = receipt(ArtifactKind::File, "image/png", "forged-kind.png");
        let outcome = state.apply_tool_update(
            "session-kind",
            "call-image",
            contract,
            &[artifact],
            Some(AcpToolCallStatus::Completed),
            None,
        );

        assert!(outcome.force_failed);
        assert!(state.finish_turn("session-kind").is_some());
    }

    #[test]
    fn exact_format_contracts_reject_other_valid_artifact_formats() {
        let cases = [
            (
                "renderWebp",
                receipt(ArtifactKind::Image, "image/png", "wrong.png"),
                "WebP image artifact",
            ),
            (
                "renderGif",
                receipt(ArtifactKind::Image, "image/webp", "wrong.webp"),
                "GIF image artifact",
            ),
            (
                "exportFlac",
                receipt(ArtifactKind::Audio, "audio/ogg", "wrong.ogg"),
                "FLAC audio artifact",
            ),
            (
                "exportOgg",
                receipt(ArtifactKind::Audio, "audio/flac", "wrong.flac"),
                "Ogg audio artifact",
            ),
            (
                "exportM4a",
                receipt(ArtifactKind::Audio, "audio/mpeg", "wrong.mp3"),
                "M4A audio artifact",
            ),
            (
                "exportMov",
                receipt(ArtifactKind::Video, "video/mp4", "wrong.mp4"),
                "QuickTime video artifact",
            ),
            (
                "exportZip",
                receipt(ArtifactKind::File, "application/pdf", "wrong.pdf"),
                "ZIP archive artifact",
            ),
            (
                "exportDocx",
                receipt(ArtifactKind::File, "application/zip", "generic.zip"),
                "DOCX document artifact",
            ),
            (
                "exportXlsx",
                receipt(
                    ArtifactKind::File,
                    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                    "wrong.pptx",
                ),
                "XLSX workbook artifact",
            ),
            (
                "exportPptx",
                receipt(
                    ArtifactKind::File,
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                    "wrong.docx",
                ),
                "PPTX presentation artifact",
            ),
        ];

        for (index, (identity, artifact, expected_label)) in cases.into_iter().enumerate() {
            let session_id = format!("session-exact-format-{index}");
            let (state, outcome) = apply_completed_contract(
                &session_id,
                identity,
                None,
                &[artifact],
            );

            assert!(outcome.force_failed, "{identity} must reject the wrong format");
            assert!(
                outcome
                    .failure
                    .as_deref()
                    .is_some_and(|error| error.contains(expected_label)),
                "{identity} should retain its exact expected format"
            );
            assert!(state.turn_failure(&session_id).is_some());
        }
    }

    #[test]
    fn exact_ooxml_contract_accepts_its_canonical_receipt_mime() {
        let session_id = "session-docx-exact-success";
        let (state, outcome) = apply_completed_contract(
            session_id,
            "exportDocx",
            None,
            &[receipt(
                ArtifactKind::File,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "report.docx",
            )],
        );

        assert!(!outcome.force_failed);
        assert_eq!(outcome.releasable_artifacts.len(), 1);
        assert!(state.turn_failure(session_id).is_none());
    }

    #[test]
    fn conflicting_identity_contracts_fail_closed() {
        let input = json!({ "tool_name": "generateAudio" });
        let contract = tool_artifact_contract(Some("generateImage"), Some(&input), None);
        assert_eq!(
            contract.contract.map(|contract| contract.expectation),
            Some(ArtifactExpectation::Image)
        );
        let contract_error = contract.error.expect("conflicting identities must be retained");
        assert!(contract_error.contains("image artifact"));
        assert!(contract_error.contains("audio artifact"));

        let session_id = "session-conflicting-contract";
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn(session_id);
        let (stored_contract, _, _) = state.observe_tool_metadata(
            session_id,
            "call-conflict",
            contract.contract,
            std::iter::empty(),
            Some(AcpToolCallStatus::Completed),
            None,
        );
        let outcome = state.apply_tool_update(
            session_id,
            "call-conflict",
            stored_contract,
            &[receipt(ArtifactKind::Image, "image/png", "image.png")],
            Some(AcpToolCallStatus::Completed),
            Some(contract_error),
        );

        assert!(outcome.force_failed);
        assert!(state.finish_turn(session_id).is_some());
    }

    #[test]
    fn a_late_exact_contract_revalidates_receipts_from_earlier_partial_updates() {
        let session_id = "session-late-exact-contract";
        let call_id = "call-render";
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn(session_id);

        let broad = artifact_contract("generate_image");
        let (broad, _, _) = state.observe_tool_metadata(
            session_id,
            call_id,
            broad,
            std::iter::empty(),
            Some(AcpToolCallStatus::InProgress),
            None,
        );
        let partial = state.apply_tool_update(
            session_id,
            call_id,
            broad,
            &[receipt(ArtifactKind::Image, "image/jpeg", "early.jpg")],
            Some(AcpToolCallStatus::InProgress),
            None,
        );
        assert!(!partial.force_failed);

        let exact = artifact_contract("render_png");
        let (exact, _, _) = state.observe_tool_metadata(
            session_id,
            call_id,
            exact,
            std::iter::empty(),
            Some(AcpToolCallStatus::Completed),
            None,
        );
        let completed = state.apply_tool_update(
            session_id,
            call_id,
            exact,
            &[],
            Some(AcpToolCallStatus::Completed),
            None,
        );

        assert!(completed.force_failed);
        assert!(
            completed
                .failure
                .as_deref()
                .is_some_and(|error| error.contains("expected PNG image artifact"))
        );
    }

    #[test]
    fn render_png_rejects_a_verified_jpeg_receipt() {
        let session_id = "session-render-png-jpeg";
        let (mut state, outcome) = apply_completed_contract(
            session_id,
            "renderPng",
            None,
            &[receipt(ArtifactKind::Image, "image/jpeg", "wrong.jpg")],
        );

        assert!(outcome.force_failed);
        assert!(
            outcome
                .failure
                .as_deref()
                .is_some_and(|error| error.contains("expected PNG image artifact"))
        );
        assert!(state.finish_turn(session_id).is_some());
    }

    #[test]
    fn generate_mp3_rejects_a_verified_wav_receipt() {
        let session_id = "session-generate-mp3-wav";
        let (mut state, outcome) = apply_completed_contract(
            session_id,
            "generateMp3",
            None,
            &[receipt(ArtifactKind::Audio, "audio/wav", "wrong.wav")],
        );

        assert!(outcome.force_failed);
        assert!(
            outcome
                .failure
                .as_deref()
                .is_some_and(|error| error.contains("expected MP3 audio artifact"))
        );
        assert!(state.finish_turn(session_id).is_some());
    }

    #[test]
    fn export_mp4_rejects_a_verified_webm_receipt() {
        let session_id = "session-export-mp4-webm";
        let (mut state, outcome) = apply_completed_contract(
            session_id,
            "exportMp4",
            None,
            &[receipt(ArtifactKind::Video, "video/webm", "wrong.webm")],
        );

        assert!(outcome.force_failed);
        assert!(
            outcome
                .failure
                .as_deref()
                .is_some_and(|error| error.contains("expected MP4 video artifact"))
        );
        assert!(state.finish_turn(session_id).is_some());
    }

    #[test]
    fn export_pdf_rejects_an_arbitrary_file_receipt() {
        let session_id = "session-export-pdf-file";
        let (mut state, outcome) = apply_completed_contract(
            session_id,
            "exportPdf",
            None,
            &[receipt(
                ArtifactKind::File,
                "application/octet-stream",
                "wrong.bin",
            )],
        );

        assert!(outcome.force_failed);
        assert!(
            outcome
                .failure
                .as_deref()
                .is_some_and(|error| error.contains("expected PDF artifact"))
        );
        assert!(state.finish_turn(session_id).is_some());
    }

    #[test]
    fn requested_image_count_rejects_one_receipt_and_accepts_four() {
        let input = json!({ "n": 4 });
        let failed_session = "session-image-count-short";
        let (mut failed_state, failed) = apply_completed_contract(
            failed_session,
            "generateImage",
            Some(&input),
            &[receipt(ArtifactKind::Image, "image/png", "only.png")],
        );
        assert!(failed.force_failed);
        assert!(
            failed
                .failure
                .as_deref()
                .is_some_and(|error| error.contains("expected at least 4"))
        );
        assert!(failed_state.finish_turn(failed_session).is_some());

        let artifacts = (0..4)
            .map(|index| {
                receipt(
                    ArtifactKind::Image,
                    "image/png",
                    &format!("image-{index}.png"),
                )
            })
            .collect::<Vec<_>>();
        let valid_session = "session-image-count-valid";
        let (mut valid_state, valid) = apply_completed_contract(
            valid_session,
            "generateImage",
            Some(&input),
            &artifacts,
        );
        assert!(!valid.force_failed, "{:?}", valid.failure);
        assert_eq!(valid.releasable_artifacts.len(), 4);
        assert!(valid_state.finish_turn(valid_session).is_none());
    }

    #[test]
    fn ordinary_read_and_edit_count_fields_do_not_create_artifact_contracts() {
        assert!(
            tool_artifact_contract(Some("Read file"), Some(&json!({ "n": 4 })), None)
                .contract
                .is_none()
        );
        assert!(
            tool_artifact_contract(
                None,
                Some(&json!({ "tool_name": "Edit", "count": 4 })),
                None,
            )
            .contract
            .is_none()
        );
    }

    #[test]
    fn terminal_tool_call_id_reuse_cannot_inherit_a_prior_receipt() {
        let session_id = "session-reused-call-id";
        let call_id = "call-image";
        let contract = tool_artifact_contract(Some("generateImage"), None, None).contract;
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn(session_id);

        let (first_contract, _, _) = state.observe_tool_metadata(
            session_id,
            call_id,
            contract,
            std::iter::empty(),
            Some(AcpToolCallStatus::Completed),
            None,
        );
        let first = state.apply_tool_update(
            session_id,
            call_id,
            first_contract,
            &[receipt(ArtifactKind::Image, "image/png", "first.png")],
            Some(AcpToolCallStatus::Completed),
            None,
        );
        assert!(!first.force_failed);

        let (reused_contract, _, _) = state.observe_tool_metadata(
            session_id,
            call_id,
            contract,
            std::iter::empty(),
            Some(AcpToolCallStatus::Completed),
            None,
        );
        let reused = state.apply_tool_update(
            session_id,
            call_id,
            reused_contract,
            &[],
            Some(AcpToolCallStatus::Completed),
            None,
        );

        assert!(reused.force_failed);
        assert!(
            reused
                .failure
                .as_deref()
                .is_some_and(|error| error.contains("reused a terminal tool-call id"))
        );
        assert!(state.finish_turn(session_id).is_some());
    }

    #[test]
    fn oversized_explicit_path_contract_fails_the_call_instead_of_verifying_a_prefix() {
        let paths = (0..=MAX_ACP_ARTIFACT_PATHS)
            .map(|index| serde_json::Value::String(format!("renders/{index}.png")))
            .collect::<Vec<_>>();
        let input = json!({ "outputPaths": paths });
        let scan = scan_artifact_path_candidates(&input, false);
        assert_eq!(scan.paths.len(), MAX_ACP_ARTIFACT_PATHS);
        assert!(
            scan.error
                .as_deref()
                .is_some_and(|error| error.contains("32-path limit"))
        );

        let session_id = "session-too-many-paths";
        let call_id = "call-export";
        let contract = tool_artifact_contract(Some("exportReport"), Some(&input), None).contract;
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn(session_id);
        let (contract, _, _) = state.observe_tool_metadata(
            session_id,
            call_id,
            contract,
            scan.paths,
            Some(AcpToolCallStatus::InProgress),
            None,
        );
        let outcome = state.apply_tool_update(
            session_id,
            call_id,
            contract,
            &[],
            Some(AcpToolCallStatus::InProgress),
            scan.error,
        );

        assert!(outcome.force_failed);
        assert!(state.finish_turn(session_id).is_some());
    }

    #[test]
    fn non_not_found_baseline_error_remains_absorbing_after_a_valid_file_appears() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(workspace.path());
        let output_path = workspace.path().join("render.png");

        // A directory (and, on real deployments, PermissionDenied or a Windows
        // sharing violation) is not proof that the output was absent before the
        // call. Preserve that pre-call error instead of treating it as NotFound.
        std::fs::create_dir(&output_path).unwrap();
        let baseline = capture_artifact_path_baseline(&store, &output_path.to_string_lossy());
        assert!(matches!(baseline, ArtifactPathBaseline::Error(_)));

        std::fs::remove_dir(&output_path).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(PNG)
            .unwrap();
        std::fs::write(&output_path, bytes).unwrap();
        let contract = artifact_contract("render_png").unwrap();
        let result = verify_completed_path_artifacts(
            Some(&store),
            &[ArtifactPathCandidate {
                path: output_path.to_string_lossy().into_owned(),
                baseline,
                observed_before_terminal: true,
            }],
            Some(SystemTime::now()),
            contract,
        );

        assert!(
            result
                .unwrap_err()
                .contains("unverifiable pre-call baseline"),
            "a later valid file must not erase the original baseline error"
        );
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }
}
