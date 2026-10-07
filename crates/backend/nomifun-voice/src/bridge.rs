use crate::{VoiceJournal, VoiceWorkContext, VoiceWorkPort};
use nomifun_voice_contracts::{WorkTarget, voice::*};
use std::collections::BTreeMap;
use std::sync::Arc;
#[path="bridge/target_proof.rs"]
mod target_proof;
use target_proof::{SourceTargetProof,ConfirmedSourceTarget};

/// Builds context from allowed canonical facts; provider opaque conversation state is never restored.
pub struct VoiceContextAssembler;
impl VoiceContextAssembler {
    pub fn instructions(context: &VoiceWorkContext) -> String {
        let mut text = String::from(
            "You are the voice interaction for the bound NomiFun AgentSession. Conversation and work are separate. Backchannels and tentative transcription never start or cancel work. Use the adapter-supported typed work tool or application delegation only for explicit stable intent. Acknowledge acceptance as received; claim applied/completed only from canonical receipts. Stopping speech or ending voice keeps work running. Approvals require a specific active canonical decision presentation; never treat yes/uh-huh as permission.\n",
        );
        text.push_str("Every typed mutation must cite the app-provided committed input source_ref. A vendor-echoed context key cannot confirm a human target. If delegation during active work is unclear, ask whether the user intends a correction or an explicit new task; do not turn an unapplied correction into new work.\n");
        text.push_str(&context.instructions);
        text
    }
    pub fn initial_facts(context:&VoiceWorkContext)->Vec<VerifiedVoiceFact>{
        let mut facts=context.initial_facts.clone();
        for receipt in &context.facts{
            facts.push(VerifiedVoiceFact{correlation_id:receipt.operation_key.clone(),upstream_trigger_id:None,
                canonical_receipt_id:receipt.receipt_id.clone(),content:serde_json::to_string(receipt).unwrap_or_default(),
                speak:false,output_generation:None,work_context:Some(VoiceWorkContextFact{target:context.observed_target.clone()}),
                speech_source:receipt.speech.as_ref().map(|speech|VoiceSpeechSourceRef{message_id:speech.message_id.clone(),revision:speech.revision,through_seq:speech.through_seq})});
        }
        for presentation in &context.approvals{
            facts.push(VerifiedVoiceFact{correlation_id:format!("approval:{}",presentation.target.presentation_id),upstream_trigger_id:None,
                canonical_receipt_id:format!("{}:{}",presentation.target.execution_id,presentation.target.request_event_sequence),
                content:serde_json::to_string(presentation).unwrap_or_default(),speak:false,output_generation:None,work_context:None,speech_source:None});
        }
        facts
    }
    pub fn tools() -> Vec<VoiceToolDefinition> {
        vec![VoiceToolDefinition { name: "nomi_work".into(), description: "Submit a complete explicit work request, correct/observe/cancel the previously observed exact task, or cancel one named queued input. Work receipts are authoritative; voice itself has no tool/approval authority.".into(),
            parameters: serde_json::json!({"type":"object","additionalProperties":false,"properties":{
                "action":{"type":"string","enum":["start","steer","observe","cancel","cancel_queued","revise_queued","answer_approval","pause_speech","mute_input"]},
                "answer":{"type":"object"},
                "text":{"type":"string","minLength":1},"target":{"type":"object"},"pending_input_id":{"type":"string"},"expected_revision":{"type":"integer","minimum":1},"muted":{"type":"boolean"},
                "source_ref":{"type":"object","additionalProperties":false,"properties":{"fragment_id":{"type":"string"},"revision":{"type":"integer","minimum":1}},"required":["fragment_id","revision"]}},"required":["action"]}) }]
    }
}

#[derive(Clone)]
struct FrozenCheckpoint {
    range: MediaRange,
    target: Option<WorkTarget>,
    receipt_id: String,
}
struct FrozenWindow {
    initial: Option<VoiceWorkContextFact>,
    checkpoints: Vec<FrozenCheckpoint>,
}
#[derive(Clone)]
struct TriggerSource {
    fragments: Vec<String>,
    receipt: VoiceWorkReceipt,
    admission_operation_key: String,
    admission_revisions:Vec<u64>,
    bound_target:Option<WorkTarget>,
    last_text: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceBridgeMediaIntent {
    InterruptOutput,
    MuteInput { muted: bool },
}
#[derive(Clone)]
struct PreparedProof {
    trigger_id: String,
    request: Option<VoiceWorkRequest>,
    revision: String,
    source_ids: Vec<String>,
    source_revisions:Vec<u64>,
    original_admission:Option<(String,Vec<u64>,Option<WorkTarget>)>,
    captured_target:Option<WorkTarget>,
    source_text: String,
    window_offset: Option<(String, u64)>,
    known_operation_key: Option<String>,
    payload_revision: Option<String>,
}
pub struct VoicePreparedWork {
    port: Arc<dyn VoiceWorkPort>,
    journal: VoiceJournal,
    owner: String,
    session: String,
    binding: u64,
    voice: String,
    epoch: u64,
    proof: PreparedProof,
    work_steering_policy:nomifun_voice_contracts::WorkSteeringPolicy,
}
pub struct VoiceExecutedWork {
    proof: PreparedProof,
    receipt: VoiceWorkReceipt,
}
impl VoiceExecutedWork {
    pub fn request_kind(&self)->Option<VoiceWorkRequestKind>{self.proof.request.as_ref().map(VoiceWorkRequest::kind)}
}
pub struct VoiceBridgeCompletion {
    pub receipt: VoiceWorkReceipt,
    pub revision_correction: Option<WorkTrigger>,
    pub revision_error: Option<VoiceError>,
}
impl VoicePreparedWork {
    pub async fn execute(self) -> Result<VoiceExecutedWork, VoiceError> {
        if let Some(operation) = &self.proof.known_operation_key {
            let mut receipt = self
                .port
                .lookup_operation(&self.owner, &self.session, operation)
                .await?
                .ok_or_else(||VoiceError::new(VoiceErrorKind::StaleBinding,"canonical receipt is no longer available; voice cache cannot renew its authority"))?;
            receipt.duplicate = true;
            return Ok(VoiceExecutedWork {
                proof: self.proof,
                receipt,
            });
        }
        let (link,fresh)=if self.proof.source_ids.is_empty(){self.journal.reserve_trigger(self.voice,self.epoch,self.proof.trigger_id.clone(),self.proof.revision.clone()).await?}
            else{self.journal.reserve_source_trigger(self.voice,self.epoch,self.proof.trigger_id.clone(),self.proof.revision.clone(),self.proof.revision.clone()).await?};
        if link.quarantined {
            return Err(VoiceError::new(
                VoiceErrorKind::JournalUnavailable,
                "voice work reference quarantined",
            ));
        }
        let receipt = match self
            .port
            .lookup_operation(&self.owner, &self.session, &link.operation_key)
            .await?
        {
            Some(mut receipt) => {
                receipt.duplicate = true;
                receipt
            }
            None if !fresh => {
                return Err(VoiceError::new(
                    VoiceErrorKind::JournalUnavailable,
                    "work admission outcome unconfirmed; original receipt must be resolved before retry",
                ));
            }
            None => {
                self.port
                    .interact_with_policy(
                        &self.owner,
                        &self.session,
                        self.binding,
                        &link.operation_key,
                        self.proof
                            .request
                            .clone()
                            .ok_or_else(|| invalid("prepared work lacks its immutable request"))?,
                        self.work_steering_policy,
                    )
                    .await?
            }
        };
        if let Some(reference)=receipt.canonical_reference(){self.journal.associate_receipt(link.operation_key,reference.into()).await?;}
        Ok(VoiceExecutedWork {
            proof: self.proof,
            receipt,
        })
    }
}

pub struct VoiceWorkBridge {
    port: Arc<dyn VoiceWorkPort>,
    journal: VoiceJournal,
    owner_id: String,
    agent_session_id: String,
    binding_version: u64,
    voice_session_id: String,
    epoch: u64,
    transcripts: BTreeMap<String, TranscriptFragment>,
    first_transcript_ordinal: BTreeMap<String, u64>,
    observed_target: Option<WorkTarget>,
    context_floor:u64,
    context_revision:u64,
    source_target_proofs:BTreeMap<String,SourceTargetProof>,
    confirmed_source_targets:BTreeMap<String,ConfirmedSourceTarget>,
    required_source:Option<nomifun_voice_contracts::VoiceInputSourceRef>,
    required_source_trigger:Option<WorkTrigger>,
    confirmed_source_trigger:Option<WorkTrigger>,
    trigger_receipts: BTreeMap<String, VoiceWorkReceipt>,
    trigger_payload_operations: BTreeMap<String, BTreeMap<String, String>>,
    trigger_sources: BTreeMap<String, TriggerSource>,
    pending_sources: BTreeMap<String, Vec<String>>,
    windows: BTreeMap<String, FrozenWindow>,
    last_delegation_offset: BTreeMap<String, u64>,
    approvals: BTreeMap<String, (nomifun_voice_contracts::VoiceApprovalPresentation, u64)>,
    transcript_ordinal: u64,
    work_steering_policy:nomifun_voice_contracts::WorkSteeringPolicy,
}
fn invalid(message: &str) -> VoiceError {
    VoiceError::new(VoiceErrorKind::Configuration, message)
}
fn substantive(text: &str) -> String {
    // Preserve interior punctuation and case: file names, code and quoted
    // values may change meaning. Only sentence-tail noise and repeated
    // whitespace are normalized for revision/idempotency comparison.
    text.trim()
        .trim_end_matches(['。', '.', '!', '！', '?', '？'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn intent_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(c, '。' | '.' | '!' | '！' | '?' | '？' | ',' | '，' | '、')
        })
        .flat_map(char::to_lowercase)
        .collect()
}
fn bound_admission_target(receipt:&VoiceWorkReceipt)->Option<WorkTarget> {
    let mut target=receipt.target.clone()?;
    if target.execution_generation>0 {target.execution_generation=receipt.first_claim_generation?;}
    Some(target)
}
impl VoiceWorkBridge {
    pub fn new(
        port: Arc<dyn VoiceWorkPort>,
        journal: VoiceJournal,
        owner_id: String,
        agent_session_id: String,
        binding_version: u64,
        voice_session_id: String,
        epoch: u64,
        context: VoiceWorkContext,
    ) -> Self {
        Self {
            port,
            journal,
            owner_id,
            agent_session_id,
            binding_version,
            voice_session_id,
            epoch,
            transcripts: BTreeMap::new(),
            first_transcript_ordinal: BTreeMap::new(),
            observed_target: context.observed_target,
            context_floor:context.context_floor,
            context_revision:0,
            source_target_proofs:BTreeMap::new(),
            confirmed_source_targets:BTreeMap::new(),
            required_source:None,
            required_source_trigger:None,
            confirmed_source_trigger:None,
            trigger_receipts: BTreeMap::new(),
            trigger_payload_operations: BTreeMap::new(),
            trigger_sources: BTreeMap::new(),
            pending_sources: BTreeMap::new(),
            windows: BTreeMap::new(),
            last_delegation_offset: BTreeMap::new(),
            approvals: context
                .approvals
                .into_iter()
                .map(|p| (p.target.presentation_id.clone(), (p, 0)))
                .collect(),
            transcript_ordinal: 0,
            work_steering_policy:Default::default(),
        }
    }
    pub fn observed_target(&self) -> Option<&WorkTarget> {
        self.observed_target.as_ref()
    }
    pub fn with_work_steering_policy(mut self,policy:nomifun_voice_contracts::WorkSteeringPolicy)->Self{self.work_steering_policy=policy;self}
    pub fn note_receipt(&mut self, receipt: &VoiceWorkReceipt) {
        for source in self.trigger_sources.values_mut() {
            if receipt.operation_key == source.admission_operation_key
                || receipt.operation_key == source.receipt.operation_key
            {
                if source.bound_target.is_none()&&source.receipt.pending_input_id.is_some()&&receipt.operation_key==source.admission_operation_key {
                    source.bound_target=bound_admission_target(receipt);
                }
                if let (Some(bound),Some(target))=(source.bound_target.as_mut(),receipt.target.as_ref()) {
                    if bound.execution_generation==0&&target.execution_generation>0&&receipt.first_claim_generation==Some(target.execution_generation)
                        &&bound.agent_session_id==target.agent_session_id&&bound.binding_version==target.binding_version&&bound.turn_operation_id==target.turn_operation_id {*bound=target.clone();}
                }
                let current_operation=source.receipt.operation_key.clone();
                source.receipt = receipt.clone();
                if receipt.operation_key==source.admission_operation_key {source.receipt.operation_key=current_operation;}
            }
        }
    }
    /// Presentation cache only. Existing transcript windows and accepted
    /// source affinities change only with their own proven identities.
    pub fn note_context_target(&mut self, target: Option<WorkTarget>) -> Result<(), VoiceError> {
        if let Some(target) = &target {
            self.validate_target(target)?;
        }
        if self.observed_target!=target {self.target_context_changed();}
        self.observed_target = target;
        Ok(())
    }
    fn validate_target(&self, target: &WorkTarget) -> Result<(), VoiceError> {
        if target.agent_session_id.as_ref() != self.agent_session_id
            || target.binding_version != self.binding_version
        {
            return Err(VoiceError::new(
                VoiceErrorKind::StaleBinding,
                "voice target belongs to another Agent binding",
            ));
        }
        Ok(())
    }
    pub fn open_context(
        &mut self,
        id: String,
        context: Option<VoiceWorkContextFact>,
    ) -> Result<(), VoiceError> {
        if id.trim().is_empty() || id.len() > 1024 {
            return Err(invalid("invalid frozen voice context identity"));
        }
        if let Some(target) = context.as_ref().and_then(|c| c.target.as_ref()) {
            self.validate_target(target)?;
        }
        if let Some(prior) = self.windows.get(&id) {
            if prior.initial != context {
                return Err(invalid(
                    "voice context identity reused with a different frozen target",
                ));
            }
            return Ok(());
        }
        if self.windows.len() >= 64 {
            return Err(VoiceError::new(
                VoiceErrorKind::Backlog,
                "voice context retention budget reached",
            ));
        }
        if context.as_ref().and_then(|context|context.target.as_ref())!=self.observed_target.as_ref() {self.target_context_changed();}
        self.windows.insert(
            id,
            FrozenWindow {
                initial: context,
                checkpoints: Vec::new(),
            },
        );
        Ok(())
    }
    pub fn context_checkpoint(
        &mut self,
        id: String,
        range: MediaRange,
        target: Option<WorkTarget>,
        receipt_id: String,
    ) -> Result<(), VoiceError> {
        if range.end_us < range.start_us || receipt_id.trim().is_empty() {
            return Err(invalid("invalid verified work-context checkpoint"));
        }
        if let Some(target) = &target {
            self.validate_target(target)?;
        }
        let changes_target=self.windows.get(&id).is_some_and(|window|window.checkpoints.last().map(|checkpoint|checkpoint.target.clone()).unwrap_or_else(||window.initial.as_ref().and_then(|context|context.target.clone()))!=target);
        let window = self
            .windows
            .get_mut(&id)
            .ok_or_else(|| invalid("checkpoint has no application-bound context opening"))?;
        if window
            .checkpoints
            .iter()
            .any(|p| p.receipt_id == receipt_id && p.range == range && p.target == target)
        {
            return Ok(());
        }
        if window
            .checkpoints
            .last()
            .is_some_and(|p| range.start_us < p.range.end_us)
        {
            return Err(invalid(
                "context timeline regressed or overlaps a previous checkpoint",
            ));
        }
        if window.checkpoints.len() >= 256 {
            return Err(VoiceError::new(
                VoiceErrorKind::Backlog,
                "work-context checkpoint retention budget reached",
            ));
        }
        window.checkpoints.push(FrozenCheckpoint {
            range,
            target,
            receipt_id,
        });
        if changes_target {self.target_context_changed();}
        Ok(())
    }
    pub fn present_approvals(
        &mut self,
        presentations: Vec<nomifun_voice_contracts::VoiceApprovalPresentation>,
    ) -> Vec<nomifun_voice_contracts::VoiceApprovalPresentation> {
        let prior=self.approvals.iter().map(|(id,(presentation,_))|(id.clone(),presentation.clone())).collect::<BTreeMap<_,_>>();
        let next=presentations.iter().map(|presentation|(presentation.target.presentation_id.clone(),presentation.clone())).collect::<BTreeMap<_,_>>();
        if prior!=next {self.target_context_changed();}
        let current = presentations
            .iter()
            .map(|p| p.target.presentation_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        self.approvals.retain(|key, _| current.contains(key));
        let mut fresh = Vec::new();
        for presentation in presentations {
            if !self
                .approvals
                .contains_key(&presentation.target.presentation_id)
            {
                fresh.push(presentation.clone());
                self.approvals.insert(
                    presentation.target.presentation_id.clone(),
                    (presentation, self.transcript_ordinal),
                );
            }
        }
        fresh
    }
    pub fn transcript(&mut self, fragment: TranscriptFragment) -> bool {
        if fragment.speaker != VoiceSpeaker::User
            || fragment.text.len() > 32 * 1024
            || fragment
                .media_range
                .as_ref()
                .is_some_and(|r| r.end_us < r.start_us)
        {
            return false;
        }
        if self
            .transcripts
            .get(&fragment.fragment_id)
            .is_some_and(|prior| prior.revision >= fragment.revision)
        {
            return false;
        }
        self.transcript_ordinal += 1;
        if fragment.commit==TranscriptCommit::Committed&&!self.source_target_proofs.contains_key(&fragment.fragment_id) {
            let proof=self.freeze_source_target(&fragment);self.source_target_proofs.insert(fragment.fragment_id.clone(),proof);
        }
        if fragment.commit==TranscriptCommit::Committed&&self.required_source.as_ref().is_some_and(|source|source.fragment_id==fragment.fragment_id&&source.revision!=fragment.revision) {
            // A popup for revision 1 cannot confirm revised words while
            // retrying revision 1's original command. Re-prepare revision 2.
            self.required_source=None;self.required_source_trigger=None;self.confirmed_source_trigger=None;
        }
        self.first_transcript_ordinal
            .entry(fragment.fragment_id.clone())
            .or_insert(self.transcript_ordinal);
        self.transcripts
            .insert(fragment.fragment_id.clone(), fragment);
        if self.transcripts.len() > 256 {
            let oldest = self
                .transcripts
                .iter()
                .min_by_key(|(id, _)| self.first_transcript_ordinal.get(*id).copied().unwrap_or(0))
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.transcripts.remove(&id);
                self.first_transcript_ordinal.remove(&id);
                self.source_target_proofs.remove(&id);self.confirmed_source_targets.remove(&id);
                if self.required_source.as_ref().is_some_and(|source|source.fragment_id==id){self.required_source=None;self.required_source_trigger=None;}
            }
        }
        true
    }
    /// Registration is synchronous; canonical correction goes through the same
    /// bounded work-worker lane as a model trigger, not the model read actor.
    pub fn transcript_revision(
        &mut self,
        fragment: TranscriptFragment,
    ) -> Result<Option<WorkTrigger>, VoiceError> {
        let prior = self.transcripts.get(&fragment.fragment_id).cloned();
        let id = fragment.fragment_id.clone();
        let committed = fragment.commit == TranscriptCommit::Committed;
        if !self.transcript(fragment) || !committed || prior.is_none() {
            return Ok(None);
        }
        let linked = self
            .trigger_sources
            .iter()
            .filter(|(_, source)| source.fragments.contains(&id))
            .map(|(id, source)| (id.clone(), source.clone()))
            .collect::<Vec<_>>();
        if linked.is_empty() {
            return Ok(None);
        }
        let mut admissions=BTreeMap::new();for (trigger,source) in linked {admissions.entry(source.admission_operation_key.clone()).or_insert((trigger,source));}
        if admissions.len() != 1 {
            return Err(invalid(
                "revised input names multiple accepted work requests; clarify the exact task",
            ));
        }
        let (trigger_id, source) = admissions.into_values().next().expect("one distinct original admission");
        let text = self.source_text(&source.fragments)?;
        if substantive(&text) == substantive(&source.last_text) {
            return Ok(None);
        }
        let request = if let Some(target) = source.bound_target {
            self.validate_target(&target)?;
            VoiceWorkRequest::Steer { target, text }
        } else if let (Some(pending_input_id), Some(expected_revision)) = (
            source.receipt.pending_input_id,
            source.receipt.pending_input_revision,
        ) {
            if source.receipt.status != VoiceWorkStatus::Queued {
                return Err(invalid(
                    "revised input is no longer proven queued; resolve its exact admission first",
                ));
            }
            VoiceWorkRequest::ReviseQueued {
                pending_input_id,
                text,
                expected_revision,
            }
        } else {
            return Err(VoiceError::new(
                VoiceErrorKind::StaleBinding,
                "revised input has no canonical exact task or queued-input revision",
            ));
        };
        Ok(Some(WorkTrigger::TypedToolCall {
            upstream_trigger_id: trigger_id,
            name: "nomi_work".into(),
            arguments: serde_json::to_value(request).map_err(|e| invalid(&e.to_string()))?,
            transcript_window_ref: None,
        }))
    }
    fn source_text(&self, ids: &[String]) -> Result<String, VoiceError> {
        let mut text = String::new();
        for id in ids {
            let fragment = self
                .transcripts
                .get(id)
                .filter(|f| f.commit == TranscriptCommit::Committed&&f.speaker==VoiceSpeaker::User)
                .ok_or_else(|| {
                    invalid("accepted transcript window is incomplete; clarify the correction")
                })?;
            text.push_str(&fragment.text);
        }
        if text.trim().is_empty() || text.len() > 32 * 1024 {
            return Err(invalid("committed work input is empty or too large"));
        }
        Ok(text)
    }
    fn target_for_range(
        &self,
        window_id: &str,
        range: &MediaRange,
    ) -> Result<Option<WorkTarget>, VoiceError> {
        let window = self.windows.get(window_id).ok_or_else(|| {
            invalid("delegation context was not explicitly opened by the application")
        })?;
        let mut target = window
            .initial
            .as_ref()
            .ok_or_else(|| {
                invalid("work target context is unknown; clarify or rebuild voice context")
            })?
            .target
            .clone();
        for checkpoint in &window.checkpoints {
            if checkpoint.range.end_us < range.start_us {
                target = checkpoint.target.clone();
                continue;
            }
            if checkpoint.range.start_us <= range.end_us && checkpoint.target != target {
                return Err(invalid(
                    "utterance crosses a verified work-target checkpoint; clarify the exact task",
                ));
            }
        }
        Ok(target)
    }
    fn delegation_window(
        &self,
        trigger: &WorkTrigger,
    ) -> Result<(String, Vec<String>, String, Option<WorkTarget>, u64), VoiceError> {
        let WorkTrigger::DelegationTrigger {
            target,
            offset_ms,
            context_window_ref,
            ..
        } = trigger
        else {
            return Err(invalid("not a delegation"));
        };
        if target != "client" {
            return Err(invalid("delegation is not addressed to this application"));
        }
        let id = context_window_ref
            .as_ref()
            .ok_or_else(|| invalid("delegation requires an exact transcript context window"))?;
        let last = *self.last_delegation_offset.get(id).unwrap_or(&0);
        if *offset_ms <= last {
            return Err(VoiceError::new(
                VoiceErrorKind::StaleEpoch,
                "delegation timeline precedes an already admitted input",
            ));
        }
        let end = offset_ms
            .checked_mul(1000)
            .ok_or_else(|| invalid("delegation timeline exceeds bounds"))?;
        let mut window = self
            .transcripts
            .values()
            .filter(|f| {
                f.commit == TranscriptCommit::Committed
                    && f.fragment_id.starts_with(&format!("{id}:"))
                    && f.media_range
                        .as_ref()
                        .is_some_and(|r| r.end_us > last.saturating_mul(1000) && r.end_us <= end)
            })
            .collect::<Vec<_>>();
        window.sort_by_key(|f| f.media_range.as_ref().map_or(0, |r| r.start_us));
        let ids = window
            .iter()
            .map(|f| f.fragment_id.clone())
            .collect::<Vec<_>>();
        let text = self.source_text(&ids)?;
        let range = MediaRange {
            start_us: window
                .first()
                .and_then(|f| f.media_range.as_ref())
                .map_or(0, |r| r.start_us),
            end_us: window
                .last()
                .and_then(|f| f.media_range.as_ref())
                .map_or(0, |r| r.end_us),
        };
        let target = self.target_for_range(id, &range)?;
        Ok((id.clone(), ids, text, target, *offset_ms))
    }
    pub fn delegation_media_intent(
        &self,
        trigger: &WorkTrigger,
    ) -> Result<Option<VoiceBridgeMediaIntent>, VoiceError> {
        if let WorkTrigger::TypedToolCall {
            name, arguments, ..
        } = trigger
        {
            if name != "nomi_work" {
                return Ok(None);
            }
            return Ok(match arguments.get("action").and_then(|v| v.as_str()) {
                Some("pause_speech") => Some(VoiceBridgeMediaIntent::InterruptOutput),
                Some("mute_input") => Some(VoiceBridgeMediaIntent::MuteInput {
                    muted: arguments
                        .get("muted")
                        .and_then(|v| v.as_bool())
                        .ok_or_else(|| invalid("mute_input requires an explicit boolean"))?,
                }),
                _ => None,
            });
        }
        let (_, _, text, _, _) = self.delegation_window(trigger)?;
        Ok(match intent_text(&text).as_str() {
            "先别念" | "别念了" | "不要念了" | "停止播报" | "停止说话" | "stopspeaking"
            | "stopreading" | "pausespeech" => Some(VoiceBridgeMediaIntent::InterruptOutput),
            "静音麦克风" | "暂停麦克风" | "mutemicrophone" => {
                Some(VoiceBridgeMediaIntent::MuteInput { muted: true })
            }
            "恢复麦克风" | "unmutemicrophone" => {
                Some(VoiceBridgeMediaIntent::MuteInput { muted: false })
            }
            _ => None,
        })
    }
    pub fn prepare_trigger(
        &mut self,
        trigger: WorkTrigger,
    ) -> Result<VoicePreparedWork, VoiceError> {
        let trigger_id = trigger.upstream_trigger_id().to_owned();
        if trigger_id.trim().is_empty() || trigger_id.len() > 1024 {
            return Err(invalid("invalid work trigger identity"));
        }
        let payload_revision = match &trigger {
            WorkTrigger::TypedToolCall {
                name, arguments, ..
            } => {
                let mut arguments = arguments.clone();
                if let Some(text) = arguments.get("text").and_then(|value| value.as_str()) {
                    arguments["text"] = serde_json::json!(substantive(text));
                }
                Some(
                    nomifun_agent_contracts::digest_payload(&(name, arguments))
                        .map_err(|e| invalid(&e.to_string()))?
                        .0,
                )
            }
            _ => None,
        };
        if let Some(operation) = payload_revision.as_ref().and_then(|revision| {
            self.trigger_payload_operations
                .get(&trigger_id)
                .and_then(|prior| prior.get(revision))
        }) {
            return Ok(self.prepared(PreparedProof {
                trigger_id,
                request: None,
                revision: String::new(),
                source_ids: Vec::new(),
                source_revisions:Vec::new(),original_admission:None,captured_target:None,
                source_text: String::new(),
                window_offset: None,
                known_operation_key: Some(operation.clone()),
                payload_revision,
            }));
        }
        if self.trigger_receipts.len() >= 4096 || self.pending_sources.len() >= 256 {
            return Err(VoiceError::new(
                VoiceErrorKind::Backlog,
                "voice trigger retention budget reached",
            ));
        }
        if self
            .trigger_payload_operations
            .get(&trigger_id)
            .is_some_and(|prior| prior.len() >= 64)
        {
            return Err(VoiceError::new(
                VoiceErrorKind::Backlog,
                "voice trigger revision retention budget reached",
            ));
        }
        if matches!(&trigger, WorkTrigger::DelegationTrigger { .. }) {
            if let Some(receipt) = self.trigger_receipts.get(&trigger_id) {
                return Ok(self.prepared(PreparedProof {
                    trigger_id,
                    request: None,
                    revision: String::new(),
                    source_ids: Vec::new(),
                    source_revisions:Vec::new(),original_admission:None,captured_target:None,
                    source_text: String::new(),
                    window_offset: None,
                    known_operation_key: Some(receipt.operation_key.clone()),
                    payload_revision,
                }));
            }
        }
        let mut window_offset = None;
        let (mut request, mut source_ids, mut source_text) = match &trigger {
            WorkTrigger::TypedToolCall {
                name,
                arguments,
                transcript_window_ref,
                ..
            } => {
                if name != "nomi_work" {
                    return Err(VoiceError::new(
                        VoiceErrorKind::Unsupported,
                        "voice tool is not an admitted work bridge action",
                    ));
                }
                let mut payload=arguments.clone();let explicit=payload.as_object_mut().and_then(|fields|fields.remove("source_ref"));
                let request = self.typed_request(payload)?;
                let ids = if !matches!(&request,VoiceWorkRequest::Observe{..}) {
                    if let Some(reference)=explicit{
                        let id=reference.get("fragment_id").and_then(serde_json::Value::as_str).ok_or_else(||invalid("work source reference needs fragment_id"))?;
                        let revision=reference.get("revision").and_then(serde_json::Value::as_u64).ok_or_else(||invalid("work source reference needs revision"))?;
                        if !self.transcripts.get(id).is_some_and(|fragment|fragment.speaker==VoiceSpeaker::User&&fragment.commit==TranscriptCommit::Committed&&fragment.revision==revision){return Err(invalid("work source reference is unknown, tentative, or revised"));}
                        vec![id.into()]
                    }else{match transcript_window_ref {
                        Some(id) => vec![id.clone()],
                        None=>self.trigger_sources.get(&trigger_id).filter(|_|matches!(request,VoiceWorkRequest::Steer{..}|VoiceWorkRequest::ReviseQueued{..})).map(|source|source.fragments.clone())
                            .ok_or_else(||invalid("work requires an explicit committed source_ref; latest transcription cannot prove its origin"))?,
                    }}
                } else {
                    Vec::new()
                };
                let text = if ids.is_empty() {
                    String::new()
                } else {
                    self.source_text(&ids)?
                };
                if !ids.is_empty()&&matches!(intent_text(&text).as_str(),"嗯"|"是"|"好"|"谢谢"|"停一下"|"算了"|"yes"|"okay"|"ok"|"stop"|"nevermind"|"uhhuh"|"thankyou"|"先别念"|"别念了"|"不要念了"|"停止播报"|"停止说话"|"stopspeaking"|"stopreading"|"pausespeech"|"静音麦克风"|"暂停麦克风"|"mutemicrophone"|"pausemicrophone"){
                    return Err(invalid("backchannel or ambiguous voice input cannot mutate work"));
                }
                (request, ids, text)
            }
            WorkTrigger::DelegationTrigger { .. } => {
                let (window, ids, text, target, offset) = self.delegation_window(&trigger)?;
                let request = self.delegated_request(text.clone(), target.as_ref(), &ids)?;
                window_offset = Some((window, offset));
                (request, ids, text)
            }
        };
        if source_ids.is_empty()
            && matches!(
                &request,
                VoiceWorkRequest::Steer { .. } | VoiceWorkRequest::ReviseQueued { .. }
            )
        {
            if let Some(prior) = self.trigger_sources.get(&trigger_id) {
                source_ids = prior.fragments.clone();
                source_text = self.source_text(&source_ids)?;
            }
        }
        let source_revisions=source_ids.iter().map(|id|self.transcripts.get(id).map(|source|source.revision).ok_or_else(||invalid("prepared source disappeared"))).collect::<Result<Vec<_>,_>>()?;
        let mut original_admission=None;
        if matches!(request,VoiceWorkRequest::Start{..}|VoiceWorkRequest::Steer{..}|VoiceWorkRequest::ReviseQueued{..}) {
            let candidates=self.trigger_sources.values().filter(|source|source.fragments==source_ids&&source.admission_revisions.len()==source_revisions.len()).collect::<Vec<_>>();
            let revised=candidates.iter().any(|source|source_revisions.iter().zip(&source.admission_revisions).any(|(current,initial)|current>initial));
            if revised {
                let mut distinct=BTreeMap::new();for source in candidates {distinct.entry(source.admission_operation_key.clone()).or_insert(source);}
                if distinct.len()!=1{return Err(invalid("a revised source names multiple original admissions; clarify the exact task"));}
                let original=distinct.into_values().next().expect("one original source admission");
                if matches!(&request,VoiceWorkRequest::Steer{target,..} if original.bound_target.as_ref()!=Some(target))
                    ||matches!(&request,VoiceWorkRequest::ReviseQueued{pending_input_id,..} if original.receipt.pending_input_id.as_ref()!=Some(pending_input_id)) {return Err(invalid("a source revision cannot redirect its original task or queued input"));}
                original_admission=Some((original.admission_operation_key.clone(),original.admission_revisions.clone(),original.bound_target.clone()));
                if substantive(&original.last_text)==substantive(&source_text) {
                    return Ok(self.prepared(PreparedProof {trigger_id,request:None,revision:String::new(),source_ids:Vec::new(),source_revisions:Vec::new(),original_admission:None,captured_target:None,source_text:String::new(),window_offset:None,known_operation_key:Some(original.receipt.operation_key.clone()),payload_revision}));
                }
                // The committed ASR correction is the complete source. Vendor
                // paraphrases are accepted, but cannot turn its revision into
                // another task or create a second correction for that revision.
                request=if let Some(target)=&original.bound_target {VoiceWorkRequest::Steer {target:target.clone(),text:source_text.clone()}}
                    else if let (Some(pending_input_id),Some(expected_revision))=(&original.receipt.pending_input_id,original.receipt.pending_input_revision) {
                        if original.receipt.status!=VoiceWorkStatus::Queued{return Err(invalid("revised source admission is not confirmed queued"));}
                        VoiceWorkRequest::ReviseQueued {pending_input_id:pending_input_id.clone(),expected_revision,text:source_text.clone()}
                    }else{return Err(invalid("revised source has no original canonical or queued receipt"));};
            }
        }
        if original_admission.is_none()&&let Some(source)=self.trigger_sources.get(&trigger_id) {original_admission=Some((source.admission_operation_key.clone(),source.admission_revisions.clone(),source.bound_target.clone()));}
        if let Some(prior) = self.trigger_receipts.get(&trigger_id) {
            if let (Some(target), VoiceWorkRequest::Start { text }) = (self.trigger_sources.get(&trigger_id).and_then(|source|source.bound_target.as_ref()), &request) {
                request = VoiceWorkRequest::Steer {
                    target: target.clone(),
                    text: text.clone(),
                };
            } else if matches!(&request, VoiceWorkRequest::Start { .. })
                && prior.pending_input_id.is_some()
            {
                return Err(invalid(
                    "a changed queued request requires its exact pending-input revision",
                ));
            }
        }
        let context_key=match &trigger {WorkTrigger::TypedToolCall {arguments,..}=>arguments.get("context_key").and_then(serde_json::Value::as_str),_=>None};
        if let Err(error)=self.guard_source_target(&request,&source_ids,&source_text,context_key) {
            if error.kind==VoiceErrorKind::SourceContextRequired {
                let requested=match &request {VoiceWorkRequest::Steer{target,..}|VoiceWorkRequest::Cancel{target}|VoiceWorkRequest::Observe{target}=>Some(target),VoiceWorkRequest::AnswerApproval{answer}=>Some(&answer.target.work_target),_=>None};
                self.required_source_trigger=(requested==self.context_presentation().target.as_ref()).then(||trigger.clone());
            }
            return Err(error);
        }
        let mut request_key=serde_json::to_value(&request).map_err(|e|invalid(&e.to_string()))?;
        if let Some(text)=request_key.get("text").and_then(serde_json::Value::as_str){request_key["text"]=serde_json::json!(substantive(text));}
        let revision=nomifun_agent_contracts::digest_payload(&(&request_key,&source_ids,substantive(&source_text))).map_err(|error|invalid(&error.to_string()))?.0;
        let captured_target=match &request {VoiceWorkRequest::Steer{target,..}|VoiceWorkRequest::Cancel{target}|VoiceWorkRequest::Observe{target}=>Some(target.clone()),VoiceWorkRequest::AnswerApproval{answer}=>Some(answer.target.work_target.clone()),_=>None};
        if !source_ids.is_empty() {
            self.pending_sources
                .insert(trigger_id.clone(), source_ids.clone());
        }
        Ok(self.prepared(PreparedProof {
            trigger_id,
            request: Some(request),
            revision,
            source_ids,
            source_revisions,original_admission,captured_target,
            source_text,
            window_offset,
            known_operation_key: None,
            payload_revision,
        }))
    }
    fn prepared(&self, proof: PreparedProof) -> VoicePreparedWork {
        VoicePreparedWork {
            port: self.port.clone(),
            journal: self.journal.clone(),
            owner: self.owner_id.clone(),
            session: self.agent_session_id.clone(),
            binding: self.binding_version,
            voice: self.voice_session_id.clone(),
            epoch: self.epoch,
            proof,
            work_steering_policy:self.work_steering_policy,
        }
    }
    pub fn finish_trigger(&mut self, executed: VoiceExecutedWork) -> VoiceBridgeCompletion {
        let VoiceExecutedWork { proof, receipt } = executed;
        self.pending_sources.remove(&proof.trigger_id);
        if let Some((window, offset)) = proof.window_offset {
            let previous = self.last_delegation_offset.entry(window).or_insert(0);
            *previous = (*previous).max(offset);
        }
        self.note_receipt(&receipt);
        if proof.request.as_ref().is_some_and(|request| {
            matches!(
                request,
                VoiceWorkRequest::Start { .. }
                    | VoiceWorkRequest::Steer { .. }
                    | VoiceWorkRequest::ReviseQueued { .. }
            )
        }) {
            if let Some(source) = self.trigger_sources.get_mut(&proof.trigger_id) {
                source.receipt = receipt.clone();
                if !proof.source_text.is_empty() {
                    source.last_text = proof.source_text.clone();
                }
            } else if !proof.source_ids.is_empty() {
                self.trigger_sources.insert(
                    proof.trigger_id.clone(),
                    TriggerSource {
                        fragments: proof.source_ids.clone(),
                        receipt: receipt.clone(),
                        admission_operation_key: proof.original_admission.as_ref().map_or_else(||receipt.operation_key.clone(),|(key,_,_)|key.clone()),
                        admission_revisions:proof.original_admission.as_ref().map_or_else(||proof.source_revisions.clone(),|(_,revisions,_)|revisions.clone()),
                        bound_target:proof.original_admission.as_ref().map(|(_,_,target)|target.clone()).unwrap_or_else(||proof.captured_target.clone().or_else(||bound_admission_target(&receipt))),
                        last_text: proof.source_text.clone(),
                    },
                );
            }
            if let Some(admission)=self.trigger_sources.get(&proof.trigger_id).map(|source|source.admission_operation_key.clone()) {
                for source in self.trigger_sources.values_mut().filter(|source|source.admission_operation_key==admission) {
                    source.receipt=receipt.clone();if !proof.source_text.is_empty(){source.last_text=proof.source_text.clone();}
                }
            }
        }
        self.trigger_receipts
            .insert(proof.trigger_id.clone(), receipt.clone());
        if let Some(payload_revision) = proof.payload_revision {
            self.trigger_payload_operations
                .entry(proof.trigger_id.clone())
                .or_default()
                .insert(payload_revision, receipt.operation_key.clone());
        }
        let correction = self.correction_for_source(&proof.trigger_id);
        match correction {
            Ok(revision_correction) => VoiceBridgeCompletion {
                receipt,
                revision_correction,
                revision_error: None,
            },
            Err(error) => VoiceBridgeCompletion {
                receipt,
                revision_correction: None,
                revision_error: Some(error),
            },
        }
    }
    /// Convenience for a single owner. Production prepares/executes/finishes
    /// separately so no bridge mutex is held across journal or canonical I/O.
    pub async fn trigger(&mut self, trigger: WorkTrigger) -> Result<VoiceWorkReceipt, VoiceError> {
        let prepared = self.prepare_trigger(trigger)?;
        let executed = prepared.execute().await?;
        Ok(self.finish_trigger(executed).receipt)
    }
    fn correction_for_source(&self, trigger_id: &str) -> Result<Option<WorkTrigger>, VoiceError> {
        let Some(source) = self.trigger_sources.get(trigger_id) else {
            return Ok(None);
        };
        let text = self.source_text(&source.fragments)?;
        if substantive(&text) == substantive(&source.last_text) {
            return Ok(None);
        }
        let request = if let Some(target) = &source.bound_target {
            self.validate_target(target)?;
            VoiceWorkRequest::Steer {
                target: target.clone(),
                text,
            }
        } else if let (Some(pending_input_id), Some(expected_revision)) = (
            &source.receipt.pending_input_id,
            source.receipt.pending_input_revision,
        ) {
            if source.receipt.status != VoiceWorkStatus::Queued {
                return Err(invalid(
                    "revised input is no longer proven queued; resolve its exact admission first",
                ));
            }
            VoiceWorkRequest::ReviseQueued {
                pending_input_id: pending_input_id.clone(),
                text,
                expected_revision,
            }
        } else {
            return Err(VoiceError::new(
                VoiceErrorKind::StaleBinding,
                "revised input has no canonical exact task or queued-input revision",
            ));
        };
        Ok(Some(WorkTrigger::TypedToolCall {
            upstream_trigger_id: trigger_id.into(),
            name: "nomi_work".into(),
            arguments: serde_json::to_value(request).map_err(|e| invalid(&e.to_string()))?,
            transcript_window_ref: None,
        }))
    }
    fn approval_matches(
        presentation: &nomifun_voice_contracts::VoiceApprovalPresentation,
        answer: &str,
    ) -> bool {
        let normalized = substantive(answer);
        let verbs = normalized.to_lowercase();
        let explicit = [
            "批准",
            "允许",
            "拒绝",
            "不允许",
            "approve",
            "reject",
            "deny",
        ]
        .iter()
        .any(|verb| verbs.contains(verb));
        let question = substantive(&presentation.question);
        let question = question
            .trim_start_matches("是否")
            .trim_start_matches("允许")
            .trim_start_matches("批准")
            .trim_start_matches("执行");
        explicit
            && (answer.contains(&presentation.target.presentation_id)
                || answer.contains(presentation.target.action_id.as_ref())
                || (question.chars().count() >= 4 && normalized.contains(question)))
    }
    fn typed_request(&self, mut arguments: serde_json::Value) -> Result<VoiceWorkRequest, VoiceError> {
        if let Some(key)=arguments.as_object_mut().and_then(|fields|fields.remove("context_key")) {
            if !key.as_str().is_some_and(|key|!key.is_empty()&&key.len()<=1024){return Err(invalid("context reference must be a bounded application key"));}
        }
        // Missing exact target is an error, never the latest presentation cache.
        let request: VoiceWorkRequest = serde_json::from_value(arguments).map_err(|_| {
            invalid("work action requires complete typed arguments and an exact target")
        })?;
        if matches!(&request,VoiceWorkRequest::Start{text}|VoiceWorkRequest::Steer{text,..}|VoiceWorkRequest::ReviseQueued{text,..} if text.trim().is_empty()||text.len()>32*1024)
        {
            return Err(invalid("work instruction is empty or too large"));
        }
        match &request {
            VoiceWorkRequest::AnswerApproval { answer } => {
                let (presentation, ordinal) = self
                    .approvals
                    .get(&answer.presented_context_id)
                    .ok_or_else(|| {
                        invalid("approval has no application-proven active presentation")
                    })?;
                if presentation.target != answer.target
                    || answer.target.interaction_agent_session_id.as_ref() != self.agent_session_id
                    || answer.target.interaction_binding_version != self.binding_version
                    || answer.target.interaction_mode
                        != nomifun_voice_contracts::ApprovalInteractionMode::VoiceAllowed
                {
                    return Err(VoiceError::new(
                        VoiceErrorKind::Unsupported,
                        "approval context is stale or requires an explicit click",
                    ));
                }
                if !Self::approval_matches(presentation, &answer.answer)
                    || !self.transcripts.values().any(|f| {
                        f.commit == TranscriptCommit::Committed
                            && f.text.trim() == answer.answer.trim()
                            && self
                                .first_transcript_ordinal
                                .get(&f.fragment_id)
                                .is_some_and(|first| first > ordinal)
                    })
                {
                    return Err(invalid(
                        "answer must identify the presented decision and match a newly committed spoken answer",
                    ));
                }
            }
            VoiceWorkRequest::Steer { target, .. }
            | VoiceWorkRequest::Observe { target }
            | VoiceWorkRequest::Cancel { target } => self.validate_target(target)?,
            VoiceWorkRequest::ReviseQueued {
                pending_input_id,
                expected_revision,
                ..
            } => {
                if pending_input_id.trim().is_empty() || *expected_revision == 0 {
                    return Err(invalid(
                        "queued correction requires exact pending-input identity and revision",
                    ));
                }
            }
            _ => {}
        }
        Ok(request)
    }
    fn delegated_request(
        &self,
        text: String,
        target: Option<&WorkTarget>,
        ids: &[String],
    ) -> Result<VoiceWorkRequest, VoiceError> {
        let normalized = intent_text(&text);
        if matches!(
            normalized.as_str(),
            "停止这个任务" | "取消这个任务" | "stopthistask" | "cancelthistask"
        ) {
            return Ok(VoiceWorkRequest::Cancel {
                target: target
                    .cloned()
                    .ok_or_else(|| invalid("no frozen exact task to cancel; clarify the target"))?,
            });
        }
        if matches!(
            normalized.as_str(),
            "现在进度如何"
                | "现在进行到哪一步"
                | "任务进度如何"
                | "whatistheprogress"
                | "howisthetaskgoing"
        ) {
            return Ok(VoiceWorkRequest::Observe {
                target: target
                    .cloned()
                    .ok_or_else(|| invalid("no frozen exact task to query; clarify the target"))?,
            });
        }
        if matches!(
            normalized.as_str(),
            "嗯" | "是"
                | "好"
                | "停一下"
                | "算了"
                | "谢谢"
                | "yes"
                | "okay"
                | "ok"
                | "stop"
                | "nevermind"
                | "uhhuh"
                | "thankyou"
        ) {
            return Err(invalid(
                "ambiguous voice intent; clarify speech, work or a specific decision",
            ));
        }
        if matches!(
            normalized.as_str(),
            "先别念"
                | "别念了"
                | "不要念了"
                | "停止播报"
                | "停止说话"
                | "stopspeaking"
                | "stopreading"
                | "pausespeech"
                | "静音麦克风"
                | "暂停麦克风"
                | "mutemicrophone"
                | "恢复麦克风"
                | "unmutemicrophone"
        ) {
            return Err(VoiceError::new(
                VoiceErrorKind::Unsupported,
                "media intent must be handled by the application media controller",
            ));
        }
        if ["批准", "允许", "拒绝", "approve", "reject", "deny", "同意"]
            .iter()
            .any(|verb| {
                normalized.starts_with(verb) || normalized.starts_with(&format!("我{verb}"))
            })
        {
            let matches = self
                .approvals
                .values()
                .filter(|(presentation, ordinal)| {
                    Self::approval_matches(presentation, &text)
                        && target.is_some_and(|target| target == &presentation.target.work_target)
                        && presentation.target.interaction_mode
                            == nomifun_voice_contracts::ApprovalInteractionMode::VoiceAllowed
                        && presentation.target.interaction_agent_session_id.as_ref()
                            == self.agent_session_id
                        && presentation.target.interaction_binding_version == self.binding_version
                        && ids.iter().all(|id| {
                            self.first_transcript_ordinal
                                .get(id)
                                .is_some_and(|first| first > ordinal)
                        })
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(invalid(
                    "spoken approval must identify one current canonical decision; clarify or use its explicit control",
                ));
            }
            let presentation = &matches[0].0;
            return Ok(VoiceWorkRequest::AnswerApproval {
                answer: nomifun_voice_contracts::VoiceApprovalAnswer {
                    target: presentation.target.clone(),
                    answer: text,
                    presented_context_id: presentation.target.presentation_id.clone(),
                },
            });
        }
        let explicit_new=["新任务","开始新任务","另一个任务","startnewtask","startanewtask"].iter().any(|prefix|normalized.starts_with(prefix));
        if explicit_new {return Ok(VoiceWorkRequest::Start {text});}
        if target_proof::correction_intent(&text) {
            return Ok(VoiceWorkRequest::Steer {
                target: target.cloned().ok_or_else(|| {
                    invalid("correction has no frozen exact task; it cannot become a new task")
                })?,
                text,
            });
        }
        if target.is_some(){return Err(invalid("work context already has an exact task; clarify a correction or explicitly request a new task"));}
        Ok(VoiceWorkRequest::Start { text })
    }
}
#[cfg(test)]
mod tests;
