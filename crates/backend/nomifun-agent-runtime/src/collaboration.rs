//! Collaboration controls observe the original host-owned delegated work.
//! They never create another execution ledger or finish the caller's Turn.
use async_trait::async_trait;
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::{ChatCausality, ChatToolDefinition};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::AgentEngineError;

pub(crate) const STATUS: &str = "agent_status";
pub(crate) const WAIT: &str = "agent_wait";
pub(crate) const MESSAGE: &str = "agent_message";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationSelection {
    pub execution_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_page: Option<AgentCollaborationResultPage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationResultPage {
    pub execution_id: String,
    pub step_id: String,
    pub source_digest: String,
    pub offset: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentChildrenWaitMode {
    Any,
    #[default]
    All,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChildrenWait {
    pub execution_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<String>,
    #[serde(default)]
    pub mode: AgentChildrenWaitMode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationSnapshot {
    pub cursor: String,
    /// Ordinary tool output, prepared from exact canonical sources by the
    /// host. This is not checkpoint data or a second result projection.
    pub result: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationMessage {
    pub execution_id: String,
    pub step_id: String,
    pub content: String,
    pub idempotency_key: String,
}

/// Bounded references derived from accepted owner receipts and successful
/// status tool results. Bodies remain only in the original event journal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationProgress {
    pub pending_child_executions: std::collections::BTreeSet<String>,
    pub results: std::collections::BTreeMap<String, AgentChildResultConsumption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_cursor: Option<String>,
    #[serde(default,skip_serializing_if="std::collections::BTreeMap::is_empty")]
    /// One watermark of each selected Step's complete immutable source set.
    /// Deletion has a distinct key and clears the Execution's old Step index.
    pub workspace_observations: std::collections::BTreeMap<String,std::collections::BTreeMap<String,String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChildResultConsumption {
    pub sequence: u64,
    pub source_digests: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub result_pages: std::collections::BTreeMap<String, AgentChildResultCoverage>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_terminal_observed: bool,
}

/// Continuous delivered bytes of one immutable canonical result; never its body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChildResultCoverage {
    pub source_digest: String,
    pub sha256: String,
    pub total_bytes: u64,
    pub contiguous_bytes: u64,
}

impl AgentCollaborationProgress {
    pub fn is_empty(&self) -> bool { self.pending_child_executions.is_empty() && self.results.is_empty()
        && self.wake_cursor.is_none() && self.last_read_cursor.is_none() && self.workspace_observations.is_empty() }

    pub fn validate(&self) -> Result<(), AgentEngineError> {
        if self.pending_child_executions.len() > 64 || self.results.len() > 64
            || self.pending_child_executions.iter().chain(self.results.keys()).any(|id| !valid_id(id))
            || self.results.values().any(|result| result.source_digests.len() > 128
                || result.source_digests.iter().any(|(id, digest)| !valid_id(id) || digest.len() != 64
                    || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
                || result.result_pages.len() > 128 || result.result_pages.iter().any(|(id,page)|
                    !valid_id(id) || !valid_digest(&page.source_digest) || !valid_digest(&page.sha256)
                        || page.contiguous_bytes > page.total_bytes)) {
            return Err(AgentEngineError::InvalidContract("invalid bounded collaboration progress references".into()));
        }
        if self.workspace_observations.len()>64 || self.workspace_observations.iter().any(|(execution,steps)|
            !valid_id(execution) || (!self.results.contains_key(execution)&&!self.pending_child_executions.contains(execution))
                || steps.len()>128 || steps.iter().any(|(key,digest)|
                (key!="@deleted" && key.strip_prefix("step:").is_none_or(|step|!valid_id(step))) || !valid_digest(digest))) {
            return Err(AgentEngineError::InvalidContract("invalid bounded child workspace observation references".into()));
        }
        if self.wake_cursor.iter().chain(self.last_read_cursor.iter()).any(|cursor| cursor.is_empty() || cursor.len() > 8192 || cursor.contains('\0')) {
            return Err(AgentEngineError::InvalidContract("invalid child consumption cursor".into()));
        }
        Ok(())
    }

    pub(crate) fn accept(&mut self, result: &crate::AgentToolResult) -> Result<(), AgentEngineError> {
        let receipt: nomi_types::agent::AgentExecutionReceipt = serde_json::from_str(&result.output_text())
            .map_err(|_| AgentEngineError::InvalidContract("accepted delegation has no typed Execution receipt".into()))?;
        if !valid_id(&receipt.execution_id) || receipt.step_ids.iter().any(|id| !valid_id(id)) {
            return Err(AgentEngineError::InvalidContract("delegation receipt identities are invalid".into()));
        }
        let already_read = self.results.get(&receipt.execution_id).is_some_and(|consumption|
            !consumption.source_digests.is_empty() && (receipt.step_ids.is_empty()
                || receipt.step_ids.iter().all(|id| consumption.source_digests.contains_key(id))));
        if !already_read { self.pending_child_executions.insert(receipt.execution_id); }
        self.validate()
    }

    pub(crate) fn accept_message(&mut self, result: &crate::AgentToolResult) -> Result<(), AgentEngineError> {
        #[derive(Deserialize)]
        struct Receipt { execution_id: String, sequence: Option<u64> }
        let receipt: Receipt = serde_json::from_str(&result.output_text())
            .map_err(|_| AgentEngineError::InvalidContract("accepted child guidance has no exact Execution receipt".into()))?;
        if !valid_id(&receipt.execution_id) { return Err(AgentEngineError::InvalidContract("child guidance Execution identity is invalid".into())); }
        let already_consumed = !self.pending_child_executions.contains(&receipt.execution_id)
            && self.results.get(&receipt.execution_id).is_some_and(|read|
                receipt.sequence.is_some_and(|sequence|sequence <= read.sequence));
        if !already_consumed {
            if let Some(read)=self.results.get_mut(&receipt.execution_id) {read.all_terminal_observed=false;}
            self.pending_child_executions.insert(receipt.execution_id);
            self.wake_cursor = None;
        }
        self.validate()
    }

    pub(crate) fn observe_status(&mut self, snapshot: &AgentCollaborationSnapshot) -> Result<bool, AgentEngineError> {
        let mut workspace_changed=false;
        let rows = snapshot.result.0.get("executions").and_then(serde_json::Value::as_array)
            .ok_or_else(|| AgentEngineError::InvalidContract("collaboration status has no exact Execution observations".into()))?;
        for row in rows {
            let id = row["execution_id"].as_str().ok_or_else(|| AgentEngineError::InvalidContract("collaboration status has no Execution identity".into()))?;
            if !self.pending_child_executions.contains(id) && !self.results.contains_key(id) { continue; }
            self.pending_child_executions.insert(id.into());
            let sequence = row["sequence"].as_u64().filter(|sequence|*sequence>0)
                .ok_or_else(|| AgentEngineError::InvalidContract("collaboration status has no canonical sequence".into()))?;
            if row["deleted"]==true {
                let receipt=&row["execution_terminal_receipt"];
                if receipt["execution_id"].as_str()!=Some(id) || receipt["event_type"].as_str()!=Some("deleted")
                    || receipt["deleted"]!=true || receipt["status"]!=row["status"]
                    || receipt["status"].as_str().is_none_or(str::is_empty)
                    || receipt["execution_event_sequence"].as_u64().is_none_or(|value|value==0||value>sequence) {
                    return Err(AgentEngineError::InvalidContract("deleted delegated work has no exact owner deletion receipt".into()));
                }
                let digest=nomifun_agent_contracts::digest_payload(receipt)
                    .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
                // Revocation invalidates earlier completion assumptions even
                // when it proves no file mutation. Keep its real domain source
                // distinct from native workspace-effect identities.
                let invalidation=nomifun_agent_contracts::digest_payload(&serde_json::json!({
                    "kind":"execution_deleted","receipt":receipt}))
                    .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
                let observed=self.workspace_observations.entry(id.into()).or_default();
                workspace_changed |= observed.get("@deleted").is_none_or(|old|old!=invalidation.as_ref());
                observed.clear();
                observed.insert("@deleted".into(),invalidation.as_ref().into());
                let consumed=self.results.entry(id.into()).or_default();
                consumed.sequence=sequence;
                consumed.source_digests.clear();
                consumed.result_pages.clear();
                consumed.source_digests.insert("@execution".into(),digest.as_ref().into());
                consumed.all_terminal_observed=true;
                self.pending_child_executions.remove(id);
                continue;
            }
            let ids = |key: &str| -> Result<std::collections::BTreeSet<String>, AgentEngineError> {
                let list = row[key].as_array().ok_or_else(|| AgentEngineError::InvalidContract("collaboration status lacks its complete owned Step identities".into()))?;
                let values = list.iter().map(|value| value.as_str().filter(|value| valid_id(value))
                    .map(str::to_owned).ok_or_else(|| AgentEngineError::InvalidContract("invalid owned child identity".into())))
                    .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
                if values.len() != list.len() || values.len() > 128 { return Err(AgentEngineError::InvalidContract("duplicate or excessive owned child identities".into())); }
                Ok(values)
            };
            let owned = ids("owned_step_ids")?;
            let terminal = ids("owned_terminal_step_ids")?;
            if !terminal.is_subset(&owned) { return Err(AgentEngineError::InvalidContract("terminal child is outside the original delegation".into())); }
            let observations=self.workspace_observations.entry(id.into()).or_default();
            observations.retain(|key,_|key.strip_prefix("step:").is_some_and(|step|owned.contains(step)));
            let consumed = self.results.entry(id.into()).or_default();
            consumed.all_terminal_observed = !owned.is_empty() && owned==terminal;
            if consumed.sequence != sequence { consumed.sequence = sequence; consumed.source_digests.clear(); }
            consumed.result_pages.retain(|step,_|owned.contains(step));
            if owned.is_empty() {
                let receipt=&row["execution_terminal_receipt"];
                if !receipt.is_null() {
                    if receipt["execution_id"].as_str()!=Some(id)
                        || receipt["execution_event_sequence"].as_u64().is_none_or(|value|value==0||value>sequence)
                        || receipt["event_type"].as_str()!=Some("status_changed")
                        || receipt["status"]!=row["status"]
                        || !matches!(receipt["status"].as_str(),Some("failed"|"cancelled"|"completed_with_failures")) {
                        return Err(AgentEngineError::InvalidContract("empty delegated group has no exact failure or cancellation settlement".into()));
                    }
                    let digest=nomifun_agent_contracts::digest_payload(receipt)
                        .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
                    consumed.source_digests.insert("@execution".into(),digest.as_ref().into());
                    self.pending_child_executions.remove(id);
                }
                continue;
            }
            for child in row["children"].as_array().ok_or_else(|| AgentEngineError::InvalidContract("collaboration status has no returned children".into()))? {
                let step = child["step_id"].as_str().ok_or_else(|| AgentEngineError::InvalidContract("returned child has no identity".into()))?;
                if !owned.contains(step) { return Err(AgentEngineError::InvalidContract("returned child is outside the original delegation".into())); }
                let mut source_digests=std::collections::BTreeSet::<String>::new();
                if let Some(sources)=child.get("workspace_change_sources") {
                    let sources=sources.as_array().ok_or_else(||AgentEngineError::InvalidContract("child workspace change sources are not an exact source array".into()))?;
                    for source in sources {
                        validate_workspace_source(source)?;
                        let digest=nomifun_agent_contracts::digest_payload(source)
                            .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
                        source_digests.insert(digest.as_ref().into());
                    }
                }
                // The owner returns this Step's complete exact source set,
                // including older Attempts. Retain one ordered-set watermark
                // rather than pinning every historical native reference.
                let digest=nomifun_agent_contracts::digest_payload(&source_digests)
                    .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
                let previous=observations.insert(format!("step:{step}"),digest.as_ref().into());
                workspace_changed |= previous.as_ref().map_or(!source_digests.is_empty(),|old|old!=digest.as_ref());
                if !terminal.contains(step) { continue; }
                let receipt = &child["terminal_receipt"];
                if receipt["execution_id"].as_str() != Some(id) || receipt["step_id"].as_str() != Some(step)
                    || receipt["execution_event_sequence"].as_u64().is_none_or(|value| value > sequence)
                    || !matches!(receipt["status"].as_str(), Some("completed"|"failed"|"cancelled"|"skipped")) {
                    return Err(AgentEngineError::InvalidContract("terminal child has no exact domain settlement receipt".into()));
                }
                let identity = serde_json::json!({"terminal_receipt":receipt,"attempt_id":child["attempt_id"],"source":child["source"]});
                let digest = nomifun_agent_contracts::digest_payload(&identity)
                    .map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?;
                if !result_delivered(consumed,step,child)? {
                    consumed.source_digests.remove(step);
                    continue;
                }
                consumed.source_digests.insert(step.into(), digest.as_ref().into());
            }
            if !owned.is_empty() && terminal == owned && owned.iter().all(|step| consumed.source_digests.contains_key(step)) {
                self.pending_child_executions.remove(id);
            }
        }
        self.wake_cursor = None;
        self.last_read_cursor = Some(snapshot.cursor.clone());
        self.validate()?;
        Ok(workspace_changed)
    }

    pub(crate) fn wait(&self) -> Option<AgentChildrenWait> {
        (!self.pending_child_executions.is_empty()).then(|| AgentChildrenWait {
            execution_ids:self.pending_child_executions.iter().cloned().collect(),after_cursor:self.last_read_cursor.clone(),mode:AgentChildrenWaitMode::All,
        })
    }

    pub(crate) fn has_ready_unread_results(&self)->bool {
        self.pending_child_executions.iter().any(|id|self.results.get(id).is_some_and(|read|read.all_terminal_observed))
    }
}

fn validate_workspace_source(value:&serde_json::Value)->Result<(),AgentEngineError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Source {agent_session_id:String,turn_operation_id:String,terminal_event_id:String}
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WorkspaceSource {attempt_id:String,source:Source}
    let source:WorkspaceSource=serde_json::from_value(value.clone()).map_err(|error|
        AgentEngineError::InvalidContract(format!("child workspace change has no exact original native source: {error}")))?;
    if !valid_id(&source.attempt_id) || [&source.source.agent_session_id,&source.source.turn_operation_id,&source.source.terminal_event_id]
        .iter().any(|value|value.is_empty()||value.len()>1024||value.trim()!=value.as_str()||value.chars().any(char::is_control)) {
        return Err(AgentEngineError::InvalidContract("child workspace change native identities are invalid".into()));
    }
    Ok(())
}

fn result_delivered(consumed:&mut AgentChildResultConsumption,step:&str,child:&serde_json::Value)
    -> Result<bool,AgentEngineError> {
    let Some(page)=child.get("result_page") else { return Ok(true); };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Page { source_digest:String,sha256:String,total_bytes:u64,offset:u64,next_offset:Option<u64> }
    let page:Page=serde_json::from_value(page.clone()).map_err(|error|
        AgentEngineError::InvalidContract(format!("invalid exact result page metadata: {error}")))?;
    let body=child["result"].as_str().ok_or_else(||AgentEngineError::InvalidContract("result page has no UTF-8 body".into()))?;
    let identity=if child.get("source").is_some() {
        serde_json::json!({"attempt_id":child["attempt_id"],"source":child["source"]})
    } else { serde_json::json!({"attempt_id":child["attempt_id"],"terminal_receipt":child["terminal_receipt"]}) };
    let source=nomifun_agent_contracts::digest_payload(&identity)
        .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
    let end=page.offset.checked_add(body.len() as u64).ok_or_else(||
        AgentEngineError::InvalidContract("result page byte range overflow".into()))?;
    if !valid_digest(&page.sha256) || page.source_digest!=source.as_ref()
        || end>page.total_bytes || (end<page.total_bytes && page.next_offset!=Some(end))
        || (end==page.total_bytes && page.next_offset.is_some()) {
        return Err(AgentEngineError::InvalidContract("result page does not match its immutable source or exact byte range".into()));
    }
    let coverage=consumed.result_pages.entry(step.into()).or_default();
    if coverage.source_digest!=page.source_digest || coverage.sha256!=page.sha256 || coverage.total_bytes!=page.total_bytes {
        *coverage=AgentChildResultCoverage {source_digest:page.source_digest,sha256:page.sha256,total_bytes:page.total_bytes,contiguous_bytes:0};
    }
    if page.offset<=coverage.contiguous_bytes {coverage.contiguous_bytes=coverage.contiguous_bytes.max(end);}
    Ok(coverage.contiguous_bytes==coverage.total_bytes)
}

pub(crate) fn snapshot_from_result(result: &crate::AgentToolResult) -> Result<AgentCollaborationSnapshot, AgentEngineError> {
    let snapshot = serde_json::from_str(&result.output_text())
        .map_err(|_| AgentEngineError::ReplayContract("child status result is not a typed owner snapshot".into()))?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

#[async_trait]
pub trait AgentCollaborationPort: Send + Sync + std::fmt::Debug {
    /// Only work owned by this exact source Session and Turn may be returned.
    async fn status(&self, causality: &ChatCausality, selection: &AgentCollaborationSelection,
        cancellation: CancellationToken) -> Result<AgentCollaborationSnapshot, AgentEngineError>;
    /// Subscribe before rereading canonical facts. Spurious notifications and
    /// timeouts are not ready results; never start, resume or retry children.
    async fn wait(&self, causality: &ChatCausality, request: &AgentChildrenWait,
        cancellation: CancellationToken) -> Result<AgentCollaborationSnapshot, AgentEngineError>;
    /// A retained host command preserves its exact target and idempotency key
    /// if its observer disappears. Delivery does not prove child completion.
    async fn send(&self, causality: &ChatCausality, call_id: &nomifun_chat_model_broker::ToolCallId, message: &AgentCollaborationMessage,
        cancellation: CancellationToken) -> Result<StrictJsonValue, AgentEngineError>;
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.trim() == value && !value.chars().any(char::is_control)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl AgentCollaborationSelection {
    pub fn validate(&self) -> Result<(), AgentEngineError> {
        let ids = self.execution_ids.iter().collect::<std::collections::BTreeSet<_>>();
        if self.execution_ids.is_empty() || self.execution_ids.len() > 64
            || ids.len() != self.execution_ids.len() || self.execution_ids.iter().any(|id| !valid_id(id)) {
            return Err(AgentEngineError::InvalidContract("collaboration selection must contain bounded distinct execution identities".into()));
        }
        let steps = self.step_ids.iter().collect::<std::collections::BTreeSet<_>>();
        if self.step_ids.len() > 128 || steps.len() != self.step_ids.len()
            || self.step_ids.iter().any(|id| !valid_id(id)) {
            return Err(AgentEngineError::InvalidContract("collaboration Step selection must contain bounded distinct identities".into()));
        }
        if let Some(page) = &self.result_page {
            if self.execution_ids.as_slice() != [page.execution_id.as_str()]
                || self.step_ids.as_slice() != [page.step_id.as_str()]
                || !valid_digest(&page.source_digest) {
                return Err(AgentEngineError::InvalidContract("result page must select its exact Execution, Step and canonical source digest".into()));
            }
        }
        Ok(())
    }
}

impl AgentChildrenWait {
    pub fn validate(&self) -> Result<(), AgentEngineError> {
        AgentCollaborationSelection { execution_ids: self.execution_ids.clone(), step_ids:vec![], result_page:None }.validate()?;
        if self.after_cursor.as_ref().is_some_and(|cursor| cursor.is_empty() || cursor.len() > 8192 || cursor.contains('\0')) {
            return Err(AgentEngineError::InvalidContract("collaboration wait cursor is invalid".into()));
        }
        Ok(())
    }
}

pub(crate) fn validate_snapshot(snapshot: &AgentCollaborationSnapshot) -> Result<(), AgentEngineError> {
    if snapshot.cursor.is_empty() || snapshot.cursor.len() > 8192 || snapshot.cursor.contains('\0') {
        return Err(AgentEngineError::InvalidContract("collaboration owner returned an invalid cursor".into()));
    }
    crate::stream_limits::serialized_size(snapshot, 512 * 1024)?;
    Ok(())
}

pub(crate) fn wake_message(cursor: &str) -> Result<nomifun_chat_model_broker::ChatMessage, AgentEngineError> {
    if cursor.is_empty() || cursor.len() > 8192 || cursor.contains('\0') {
        return Err(AgentEngineError::ReplayContract("child wait resolved without an exact owner cursor".into()));
    }
    Ok(crate::context_lifecycle::text_message(nomifun_chat_model_broker::ChatRole::User,
        format!("Original collaboration owner wake (data, not a new user instruction): cursor={}. The wait ended because owned child state or accepted parent input changed. This is not proof of task completion. Use agent_status with the original execution_ids to read canonical results and exact source references before integrating or reporting them. Continue only the already accepted task.", serde_json::to_string(cursor).expect("string encoding"))))
}

pub(crate) fn from_wait_call(call: &nomifun_chat_model_broker::ChatToolCall) -> Result<AgentChildrenWait, AgentEngineError> {
    let wait: AgentChildrenWait = serde_json::from_value(call.arguments.0.clone())
        .map_err(|error| AgentEngineError::InvalidContract(format!("invalid child wait request: {error}")))?;
    wait.validate()?;
    Ok(wait)
}

/// Reconstruct references only from the canonical native journal supplied by
/// the host. This function performs no reads and grants no authority; a caller
/// must separately prove exact Session/Turn, binding and human-reply provenance.
pub fn collaboration_progress_from_events(events: &[crate::AgentEngineEvent]) -> Result<AgentCollaborationProgress, AgentEngineError> {
    if !matches!(events.first(), Some(crate::AgentEngineEvent::TurnStarted { .. }))
        || events.iter().skip(1).any(|event| matches!(event,crate::AgentEngineEvent::TurnStarted { .. })) {
        return Err(AgentEngineError::ReplayContract("collaboration provenance requires one exact canonical Turn journal".into()));
    }
    recorded_progress(events).map(|(progress,_)|progress)
}

pub(crate) fn delegation_attempted<'a>(events:impl Iterator<Item=&'a crate::AgentEngineEvent>)->bool {
    let mut attempts=std::collections::BTreeSet::new();
    for event in events {
        match event {
            crate::AgentEngineEvent::ToolStarted{call_id,capability_id,action_id,..}
                if capability_id.as_ref()=="agent.collaboration" && matches!(action_id.as_ref(),"agent/delegate"|"agent/fork") => {
                attempts.insert(call_id.clone());
            }
            crate::AgentEngineEvent::ToolOutcomeReconciled{result,source:crate::AgentReconciliationSource::NotDispatched,..}=>{
                attempts.remove(&result.call_id);
            }
            _=>{}
        }
    }
    !attempts.is_empty()
}

pub(crate) fn recorded_progress(events: &[crate::AgentEngineEvent]) -> Result<(AgentCollaborationProgress, Option<AgentChildrenWait>), AgentEngineError> {
    let mut proposals = std::collections::BTreeMap::new();
    let mut admitted = std::collections::BTreeSet::new();
    let mut progress = AgentCollaborationProgress::default();
    let mut pending = None;
    for (index,event) in events.iter().enumerate() {
        match event {
            crate::AgentEngineEvent::CollaborationInherited { progress: inherited } => {
                if index != 1 || !matches!(events.first(), Some(crate::AgentEngineEvent::TurnStarted { .. })) {
                    return Err(AgentEngineError::ReplayContract("collaboration inheritance is not the original Turn entry".into()));
                }
                inherited.validate()?;
                progress = inherited.clone();
            }
            crate::AgentEngineEvent::ToolCallCompleted { call, .. } => {
                proposals.insert(call.call_id.clone(), call);
            }
            crate::AgentEngineEvent::ToolStarted { call_id, capability_id, action_id, .. }
                if capability_id.as_ref() == "agent.collaboration" && matches!(action_id.as_ref(), "agent/delegate"|"agent/fork") => {
                admitted.insert(call_id.clone());
            }
            crate::AgentEngineEvent::ToolCompleted { result, .. }
            | crate::AgentEngineEvent::ToolOutcomeReconciled { result, .. } if !result.is_error => {
                if let Some(call) = proposals.get(&result.call_id) {
                    if call.name == WAIT { pending = Some(from_wait_call(call)?); }
                    else if call.name == STATUS { progress.observe_status(&snapshot_from_result(result)?)?; }
                    else if call.name == MESSAGE { progress.accept_message(result)?; }
                    else if admitted.contains(&result.call_id) { progress.accept(result)?; }
                }
            }
            crate::AgentEngineEvent::AwaitingChildren { request, .. } => {
                if pending.as_ref() != Some(request) && progress.wait().as_ref() != Some(request) {
                    return Err(AgentEngineError::ReplayContract("child wait has no exact accepted native control".into()));
                }
                pending = Some(request.clone());
            }
            crate::AgentEngineEvent::ChildrenWaitResolved { cursor, .. } => {
                if pending.is_none() {
                    return Err(AgentEngineError::ReplayContract("child wake has no pending native wait".into()));
                }
                wake_message(cursor)?;
                progress.wake_cursor = Some(cursor.clone());
                pending = None;
            }
            _ => {}
        }
    }
    Ok((progress, pending))
}

pub(crate) async fn execute(
    call: &nomifun_chat_model_broker::ChatToolCall, port: &dyn AgentCollaborationPort,
    causality: &ChatCausality, cancellation: CancellationToken,
    awaiting_children: &mut Option<AgentChildrenWait>,
) -> Result<crate::AgentToolResult, AgentEngineError> {
    let value = match call.name.as_str() {
        STATUS => {
            let selection: AgentCollaborationSelection = serde_json::from_value(call.arguments.0.clone())
                .map_err(|error| AgentEngineError::InvalidContract(format!("invalid collaboration selection: {error}")))?;
            selection.validate()?;
            let snapshot = port.status(causality, &selection, cancellation).await?;
            validate_snapshot(&snapshot)?;
            serde_json::to_value(snapshot).map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?
        }
        WAIT => {
            let wait = from_wait_call(call)?;
            // Validate ownership now; waiting itself begins only after the ACK
            // and native checkpoint have settled in the original journal.
            let selection = AgentCollaborationSelection { execution_ids: wait.execution_ids.clone(), step_ids:vec![], result_page:None };
            let snapshot = port.status(causality, &selection, cancellation).await?;
            validate_snapshot(&snapshot)?;
            let cursor = snapshot.cursor;
            *awaiting_children = Some(wait);
            serde_json::json!({"state":"awaiting_children","cursor":cursor,
                "message":"Child wait accepted. The original Turn remains active; read exact results with agent_status after the owner wakes this wait."})
        }
        MESSAGE => {
            let message: AgentCollaborationMessage = serde_json::from_value(call.arguments.0.clone())
                .map_err(|error| AgentEngineError::InvalidContract(format!("invalid collaboration message: {error}")))?;
            if !valid_id(&message.execution_id) || !valid_id(&message.step_id)
                || message.content.trim().is_empty() || message.content.len() > 64 * 1024
                || !valid_id(&message.idempotency_key) || message.idempotency_key.len() > 128 {
                return Err(AgentEngineError::InvalidContract("collaboration message is outside its bounded contract".into()));
            }
            port.send(causality, &call.call_id, &message, cancellation).await?.0
        }
        _ => return Err(AgentEngineError::InvalidContract("unknown collaboration control".into())),
    };
    Ok(crate::AgentToolResult::text(call.call_id.clone(), value.to_string(), false))
}

pub(crate) fn definitions() -> Vec<ChatToolDefinition> {
    let ids = serde_json::json!({"type":"array","minItems":1,"maxItems":64,"uniqueItems":true,
        "items":{"type":"string","minLength":1,"maxLength":256}});
    let definition = |name: &str, description: &str, schema| ChatToolDefinition {
        name:name.into(),description:description.into(),input_schema:StrictJsonValue(schema),deferred:false,
    };
    vec![
        definition(STATUS,"Read status and exact canonical results of delegated executions owned by this source Turn. Read here after agent_wait wakes. Optionally select step_ids. Large results include result_page metadata: continue in order by copying execution_id, step_id, source_digest and next_offset into result_page.offset, selecting only that execution_id and step_id. Read all bytes before integrating or reporting completion. This does not create, resume, retry or cancel work.",
            serde_json::json!({"type":"object","additionalProperties":false,"required":["execution_ids"],"properties":{"execution_ids":ids,
                "step_ids":{"type":"array","maxItems":128,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":256}},
                "result_page":{"type":"object","additionalProperties":false,"required":["execution_id","step_id","source_digest","offset"],"properties":{
                    "execution_id":{"type":"string","minLength":1,"maxLength":256},"step_id":{"type":"string","minLength":1,"maxLength":256},
                    "source_digest":{"type":"string","minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"},"offset":{"type":"integer","minimum":0}}}}})),
        definition(WAIT,"Wait without model polling for owned delegated executions. Submit alone. The original Turn, budget and resources remain owned; this is not task completion or user-input handoff. When awakened, call agent_status to read exact results before integrating them. mode=all waits for all selected work; any waits for a selected terminal result. Copy after_cursor from agent_status when waiting for subsequent changes.",
            serde_json::json!({"type":"object","additionalProperties":false,"required":["execution_ids"],"properties":{
                "execution_ids":ids,"after_cursor":{"type":"string","minLength":1,"maxLength":8192},"mode":{"type":"string","enum":["any","all"]}}})),
        definition(MESSAGE,"Send guidance to one exact owned delegated Step. This does not start work or prove completion. Keep the same idempotency_key when checking an uncertain delivery; never address a sibling owned by another source.",
            serde_json::json!({"type":"object","additionalProperties":false,"required":["execution_id","step_id","content","idempotency_key"],"properties":{
                "execution_id":{"type":"string","minLength":1,"maxLength":256},"step_id":{"type":"string","minLength":1,"maxLength":256},
                "content":{"type":"string","minLength":1,"maxLength":16384},"idempotency_key":{"type":"string","minLength":1,"maxLength":128}}})),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn accepted() -> AgentCollaborationProgress {
        let mut progress = AgentCollaborationProgress::default();
        progress.accept(&crate::AgentToolResult::text("spawn".into(), json!({"execution_id":"execution","status":"running","message":"Exact accepted delegation receipt"}).to_string(), false)).unwrap();
        progress
    }

    fn page(sequence:u64, selected:&[&str], terminal:&[&str]) -> AgentCollaborationSnapshot {
        AgentCollaborationSnapshot { cursor:format!("source:{sequence}"), result:StrictJsonValue(json!({"executions":[{
            "execution_id":"execution","sequence":sequence,"owned_step_ids":["a","b"],"owned_terminal_step_ids":terminal,
            "children":selected.iter().map(|id|json!({"step_id":id,"attempt_id":format!("attempt-{sequence}-{id}"),
                "source":{"agent_session_id":format!("session-{id}"),"turn_operation_id":format!("turn-{sequence}-{id}"),"terminal_event_id":format!("terminal-{sequence}-{id}")},
                "terminal_receipt":{"execution_id":"execution","execution_event_sequence":sequence,"step_id":id,"status":"completed"},
                "workspace_change_sources":[],
                "result":"PRIVATE_RESULT_BODY_NOT_CHECKPOINT_DATA"})).collect::<Vec<_>>()
        }]})) }
    }

    #[test]
    fn partial_status_pages_consume_only_returned_exact_results_and_never_copy_bodies() {
        let mut progress=accepted();
        progress.observe_status(&page(7,&["a"],&["a","b"])).unwrap();
        assert!(progress.pending_child_executions.contains("execution"));
        assert!(!serde_json::to_string(&progress).unwrap().contains("PRIVATE_RESULT_BODY"));
        progress.observe_status(&page(7,&["b"],&["a","b"])).unwrap();
        assert!(progress.pending_child_executions.is_empty());
        progress.validate().unwrap();
    }

    #[test]
    fn changed_execution_sequence_cannot_reuse_an_old_attempt_consumption() {
        let mut progress=accepted();
        progress.observe_status(&page(7,&["a"],&["a","b"])).unwrap();
        progress.observe_status(&page(8,&["b"],&["a","b"])).unwrap();
        assert!(progress.pending_child_executions.contains("execution"));
        assert!(!progress.results["execution"].source_digests.contains_key("a"));
        progress.observe_status(&page(8,&["a"],&["a","b"])).unwrap();
        assert!(progress.pending_child_executions.is_empty());
    }

    #[test]
    fn guidance_followup_reopens_only_unconsumed_receipt_sequences_and_replay_does_not_restart_work() {
        let mut progress=accepted();
        progress.observe_status(&page(7,&["a","b"],&["a","b"])).unwrap();
        let receipt=|sequence|crate::AgentToolResult::text("guide".into(),json!({"execution_id":"execution","sequence":sequence,"accepted":true}).to_string(),false);
        progress.accept_message(&receipt(7)).unwrap();
        assert!(progress.pending_child_executions.is_empty());
        progress.accept_message(&receipt(6)).unwrap();
        assert!(progress.pending_child_executions.is_empty(),"an older original command receipt cannot reopen already consumed work");
        progress.accept_message(&receipt(8)).unwrap();
        assert!(progress.pending_child_executions.contains("execution"));
        progress.observe_status(&page(8,&["a","b"],&["a","b"])).unwrap();
        progress.accept_message(&receipt(8)).unwrap();
        assert!(progress.pending_child_executions.is_empty());
    }

    #[test]
    fn attention_status_restores_coordination_without_claiming_child_completion() {
        let mut progress=accepted();
        progress.wake_cursor=Some("attention:6".into());
        let mut snapshot=page(7,&["a"],&["a"]);
        snapshot.result.0["executions"][0]["children"][0]["status"]=json!("waiting_input");
        snapshot.result.0["executions"][0]["owned_terminal_step_ids"]=json!([]);
        progress.observe_status(&snapshot).unwrap();
        assert!(progress.wake_cursor.is_none());
        assert_eq!(progress.wait().unwrap().after_cursor.as_deref(),Some("source:7"));
        assert!(progress.pending_child_executions.contains("execution"));
    }

    #[test]
    fn terminal_status_without_its_exact_receipt_never_consumes_a_result() {
        let mut progress=accepted();
        let mut snapshot=page(7,&["a","b"],&["a","b"]);
        snapshot.result.0["executions"][0]["children"][0]["terminal_receipt"]=serde_json::Value::Null;
        assert!(progress.observe_status(&snapshot).is_err());
        assert!(progress.pending_child_executions.contains("execution"));
    }

    #[test]
    fn an_empty_delegation_needs_its_real_failure_receipt_and_is_never_vacuously_completed() {
        let mut progress=accepted();
        let mut snapshot=AgentCollaborationSnapshot{cursor:"failed:7".into(),result:StrictJsonValue(json!({"executions":[{
            "execution_id":"execution","sequence":7,"status":"planning","owned_step_ids":[],"owned_terminal_step_ids":[],"children":[]
        }]}))};
        progress.observe_status(&snapshot).unwrap();
        assert!(progress.pending_child_executions.contains("execution"));
        let row=&mut snapshot.result.0["executions"][0];
        row["status"]=json!("failed");
        row["execution_terminal_receipt"]=json!({"execution_id":"execution","execution_event_sequence":7,"status":"failed","event_type":"status_changed"});
        progress.observe_status(&snapshot).unwrap();
        assert!(progress.pending_child_executions.is_empty());
        assert!(progress.results["execution"].source_digests.contains_key("@execution"));
    }

    fn body_page(offset:u64,fragment:&str,total:u64) -> AgentCollaborationSnapshot {
        let mut snapshot=page(7,&["a","b"],&["a","b"]);
        let child=&mut snapshot.result.0["executions"][0]["children"][0];
        let source=nomifun_agent_contracts::digest_payload(&json!({"attempt_id":child["attempt_id"],"source":child["source"]})).unwrap();
        let end=offset+fragment.len() as u64;
        child["result"]=json!(fragment);
        child["result_page"]=json!({"source_digest":source,"sha256":nomifun_agent_contracts::digest_bytes("中文abc".as_bytes()),
            "offset":offset,"total_bytes":total,"next_offset":(end<total).then_some(end)});
        snapshot
    }

    #[test]
    fn result_pages_require_contiguous_original_bytes_and_checkpoint_only_coverage() {
        let mut progress=accepted();
        progress.observe_status(&body_page(6,"abc",9)).unwrap();
        assert!(progress.pending_child_executions.contains("execution"),"last page alone cannot satisfy the result obligation");
        assert_eq!(progress.results["execution"].result_pages["a"].contiguous_bytes,0);
        progress.observe_status(&body_page(0,"中文",9)).unwrap();
        assert!(progress.pending_child_executions.contains("execution"));
        let saved=serde_json::to_string(&progress).unwrap();
        assert!(!saved.contains("中文"));
        let mut resumed:AgentCollaborationProgress=serde_json::from_str(&saved).unwrap();
        resumed.observe_status(&body_page(6,"abc",9)).unwrap();
        assert!(resumed.pending_child_executions.is_empty());
        resumed.observe_status(&body_page(0,"中文",9)).unwrap();
        assert!(resumed.pending_child_executions.is_empty(),"re-reading earlier bytes preserves delivered coverage");
        assert_eq!(resumed.results["execution"].result_pages["a"].contiguous_bytes,9);
        resumed.validate().unwrap();
    }

    #[test]
    fn changed_result_source_discards_partial_coverage_and_wrong_digest_is_rejected() {
        let mut progress=accepted();
        progress.observe_status(&body_page(0,"中文",9)).unwrap();
        let mut changed=body_page(6,"abc",9);
        changed.result.0["executions"][0]["children"][0]["source"]["terminal_event_id"]=json!("new-terminal");
        assert!(progress.observe_status(&changed).is_err());
        let child=&mut changed.result.0["executions"][0]["children"][0];
        child["result_page"]["source_digest"]=json!(nomifun_agent_contracts::digest_payload(&json!({"attempt_id":child["attempt_id"],"source":child["source"]})).unwrap());
        progress.observe_status(&changed).unwrap();
        assert_eq!(progress.results["execution"].result_pages["a"].contiguous_bytes,0);
        assert!(progress.pending_child_executions.contains("execution"));
    }

    #[test]
    fn exact_child_workspace_source_invalidates_once_even_after_aggregate_sequence_changes() {
        let mut progress=accepted();
        let mut snapshot=page(7,&["a","b"],&["a","b"]);
        let child=&mut snapshot.result.0["executions"][0]["children"][0];
        child["workspace_change_sources"]=json!([{"attempt_id":child["attempt_id"],"source":child["source"]}]);
        assert!(progress.observe_status(&snapshot).unwrap());
        assert!(!progress.observe_status(&snapshot).unwrap());
        snapshot.result.0["executions"][0]["sequence"]=json!(8);
        assert!(!progress.observe_status(&snapshot).unwrap(),"an unrelated aggregate event does not change the immutable workspace source");
        snapshot.result.0["executions"][0]["children"][0]["workspace_change_sources"][0]["source"]["terminal_event_id"]=json!("a-new-written-turn");
        assert!(progress.observe_status(&snapshot).unwrap());
        assert_eq!(progress.workspace_observations["execution"].len(),2);
    }

    #[test]
    fn historical_attempt_workspace_changes_survive_a_current_cancelled_attempt_without_native_session() {
        let mut snapshot=page(7,&["a","b"],&["a","b"]);
        let child=&mut snapshot.result.0["executions"][0]["children"][0];
        child["attempt_id"]=json!("current-never-started");child["status"]=json!("cancelled");
        child["terminal_receipt"]["status"]=json!("cancelled");
        child.as_object_mut().unwrap().remove("source");child.as_object_mut().unwrap().remove("result");
        child["workspace_change_sources"]=json!([
            {"attempt_id":"old-failed-writer","source":{"agent_session_id":"original-child","turn_operation_id":"failed-turn","terminal_event_id":"failed-terminal"}},
            {"attempt_id":"old-cancelled-writer","source":{"agent_session_id":"original-child","turn_operation_id":"cancelled-turn","terminal_event_id":"cancelled-terminal"}}
        ]);
        let mut progress=accepted();
        assert!(progress.observe_status(&snapshot).unwrap());
        assert_eq!(progress.workspace_observations["execution"].len(),2);
        assert!(progress.pending_child_executions.is_empty());
        assert!(!progress.observe_status(&snapshot).unwrap());
        let before=progress.workspace_observations.clone();
        let sources=snapshot.result.0["executions"][0]["children"][0]["workspace_change_sources"].as_array_mut().unwrap();
        sources.reverse();sources.push(sources[0].clone());
        assert!(!progress.observe_status(&snapshot).unwrap(),"source ordering and duplicate exact references do not change a set watermark");
        let mut resumed:AgentCollaborationProgress=serde_json::from_slice(&serde_json::to_vec(&progress).unwrap()).unwrap();
        assert_eq!(resumed.workspace_observations,before);
        assert!(!resumed.observe_status(&snapshot).unwrap(),"checkpoint recovery does not invalidate the same complete source set again");
        snapshot.result.0["executions"][0]["children"][0]["workspace_change_sources"].as_array_mut().unwrap().push(json!({
            "attempt_id":"another-writer","source":{"agent_session_id":"original-child","turn_operation_id":"another-turn","terminal_event_id":"another-terminal"}}));
        assert!(resumed.observe_status(&snapshot).unwrap(),"one genuinely new exact source changes the watermark");
        assert!(snapshot.result.0["executions"][0]["children"][0]["source"].is_null(),"historical mutations must never become the current result's native source");
        let mut invalid=snapshot.clone();
        invalid.result.0["executions"][0]["children"][0]["workspace_change_sources"][0]["source"]["terminal_event_id"]=json!("");
        assert!(progress.observe_status(&invalid).is_err());
    }

    #[test]
    fn many_historical_attempts_keep_one_bounded_workspace_watermark_per_active_step() {
        let owned=(0..128).map(|index|format!("step-{index:03}")).collect::<Vec<_>>();
        let mut progress=accepted();
        let mut last=None;
        for (index,step) in owned.iter().enumerate() {
            let sources=(0..32).map(|attempt|json!({"attempt_id":format!("attempt-{index}-{attempt}"),"source":{
                "agent_session_id":format!("session-{index}"),"turn_operation_id":format!("writer-{index}-{attempt}"),
                "terminal_event_id":format!("closed-{index}-{attempt}")}})).collect::<Vec<_>>();
            let snapshot=AgentCollaborationSnapshot {cursor:"bounded:7".into(),result:StrictJsonValue(json!({"executions":[{
                "execution_id":"execution","sequence":7,"owned_step_ids":owned,"owned_terminal_step_ids":owned,
                "children":[{"step_id":step,"attempt_id":format!("never-started-{index}"),"status":"cancelled",
                    "terminal_receipt":{"execution_id":"execution","execution_event_sequence":7,"step_id":step,"status":"cancelled"},
                    "workspace_change_sources":sources}]}]}))};
            validate_snapshot(&snapshot).unwrap();
            assert!(progress.observe_status(&snapshot).unwrap());
            last=Some(snapshot);
        }
        assert!(progress.pending_child_executions.is_empty());
        assert_eq!(progress.workspace_observations["execution"].len(),128);
        let encoded=serde_json::to_vec(&progress).unwrap();
        assert!(encoded.len()<64*1024,"4096 historical source refs must not exhaust the 256KiB native checkpoint: {}",encoded.len());
        assert!(!String::from_utf8(encoded.clone()).unwrap().contains("writer-127-31"));
        let mut recovered:AgentCollaborationProgress=serde_json::from_slice(&encoded).unwrap();
        recovered.validate().unwrap();
        let mut last=last.unwrap();
        assert!(!recovered.observe_status(&last).unwrap());
        let row=&mut last.result.0["executions"][0];row["sequence"]=json!(8);
        row["owned_step_ids"]=json!([owned[127]]);row["owned_terminal_step_ids"]=json!([owned[127]]);
        assert!(!recovered.observe_status(&last).unwrap());
        assert_eq!(recovered.workspace_observations["execution"].keys().cloned().collect::<Vec<_>>(),["step:step-127"]);
    }

    #[test]
    fn explicit_deletion_releases_the_obligation_only_with_its_original_domain_receipt() {
        let mut snapshot=page(7,&["a"],&[]);
        let row=&mut snapshot.result.0["executions"][0];
        row["deleted"]=json!(true);
        row["status"]=json!("running");
        let mut progress=accepted();
        assert!(progress.observe_status(&snapshot).is_err());
        snapshot.result.0["executions"][0]["execution_terminal_receipt"]=json!({"execution_id":"execution",
            "execution_event_sequence":7,"event_type":"deleted","deleted":true,"status":"running"});
        assert!(progress.observe_status(&snapshot).unwrap(),"a newly observed user revocation invalidates old completion/freshness without asserting a mutation");
        assert!(progress.pending_child_executions.is_empty());
        assert!(progress.results["execution"].source_digests.contains_key("@execution"));
        assert!(!progress.observe_status(&snapshot).unwrap(),"the same original revocation is invalidated only once");
        assert_eq!(progress.workspace_observations["execution"].keys().cloned().collect::<Vec<_>>(),["@deleted"]);
        assert!(snapshot.result.0["executions"][0]["status"]=="running","observing deletion does not manufacture task success or cleanup completion");
    }
}
