//! Engine-neutral resolution of immutable facts from the existing Conversation
//! owner. This is not a new Session store or permission authority. The returned
//! snapshot can be compiled through open_kernel_session with real resource
//! owners; engines do not supply their own workspace or permission authority.
use std::sync::{Arc, Weak};

use nomifun_agent_contracts::{
    AgentBindingChangedPayloadV1, AgentBindingValue, AgentHandoffBindingRefV1,
    AgentHandoffEnvelopeV1, AgentHandoffMode, AgentPresetRevision, PrincipalRef,
    ResolvedSnapshotEnvelope, ResolvedSnapshotRef, SessionPayloadBody, digest_bytes,
};
use nomifun_agent_control_plane::{AgentControlPlane, AuthenticatedOwner};
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_api_types::{ConversationResponse, RuntimeBuildBinding};
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};

use super::nomi_core_session::{NomiCoreSessionOwner, session_metadata};
use super::official_runtime::{OfficialRuntimeHost, binding_from_extra};

#[path = "engine_recovery.rs"]
mod recovery;

/// Constructed by application assembly only. The sole official Driver receives
/// this typed port owner from the composition root; there is no runtime
/// registration or direct Domain-handler access path.
pub struct EngineSessionHost {
    owner: Weak<NomiCoreSessionOwner>,
    control_plane: Arc<AgentControlPlane>,
    engines: Weak<OfficialRuntimeHost>,
    pool: SqlitePool,
    broker: super::chat_broker_host::ChatBrokerHostComposition,
    http: reqwest::Client,
    source: Arc<()>,
    execution_instance_id: String,
    resources: super::engine_kernel_session::EngineKernelAssembly,
    kernel_sessions: std::sync::Mutex<
        std::collections::BTreeMap<String, Weak<super::engine_kernel_session::EngineKernelSession>>,
    >,
    journals: tokio::sync::Mutex<
        std::collections::BTreeMap<(String, String), Weak<super::engine_journal::Journal>>,
    >,
}

/// Read-only facts from one durable Session binding, never model-supplied JSON.
/// This is an admission snapshot, not a turn/effect lease: revalidate the
/// accepted message receipt and live generation before every turn/invocation.
pub struct AdmittedEngineSession {
    source: Arc<()>,
    response: ConversationResponse,
    engine_binding: RuntimeBuildBinding,
    agent_binding: AgentBindingValue,
    revision: AgentPresetRevision,
    snapshot: ResolvedSnapshotEnvelope,
    principal: PrincipalRef,
    workspace: String,
    active_set_generation: u64,
    active_capability_ids: Vec<String>,
}

/// Canonical accepted root from the existing delivery owner. Read-only facts,
/// not permission to invoke tools or a durable cleanup/terminal receipt.
pub struct EngineTurnReceipt {
    source: Arc<()>,
    session: AdmittedEngineSession,
    root_message_id: String,
    turn_started_event_id: String,
    operation_id: String,
    admission_epoch: i64,
    request_payload: serde_json::Value,
}

impl EngineTurnReceipt {
    pub(super) fn belongs_to(&self, source: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.source, source)
    }
    pub fn session(&self) -> &AdmittedEngineSession {
        &self.session
    }
    pub fn root_message_id(&self) -> &str {
        &self.root_message_id
    }
    pub fn turn_started_event_id(&self) -> &str {
        &self.turn_started_event_id
    }
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn admission_epoch(&self) -> i64 {
        self.admission_epoch
    }
    pub fn request_payload(&self) -> &serde_json::Value {
        &self.request_payload
    }
}

impl AdmittedEngineSession {
    pub fn session(&self) -> &ConversationResponse {
        &self.response
    }
    pub fn engine_binding(&self) -> &RuntimeBuildBinding {
        &self.engine_binding
    }
    pub fn agent_binding(&self) -> &AgentBindingValue {
        &self.agent_binding
    }
    pub fn revision(&self) -> &AgentPresetRevision {
        &self.revision
    }
    pub fn snapshot(&self) -> &ResolvedSnapshotEnvelope {
        &self.snapshot
    }
    pub fn principal(&self) -> &PrincipalRef {
        &self.principal
    }
    pub fn workspace(&self) -> &str {
        &self.workspace
    }
    pub fn active_set_generation(&self) -> u64 {
        self.active_set_generation
    }
    pub fn active_capability_ids(&self) -> &[String] {
        &self.active_capability_ids
    }
    pub fn execution_constraints(&self) -> Result<nomifun_api_types::ExecutionConstraints, AppError> {
        nomifun_api_types::ExecutionConstraints::from_extra(&self.response.extra)
    }
}

/// A durable Plugin delivery gap: the gate reason plus actionable detail a
/// same-turn settlement check can relay to the model.
pub(super) struct PluginDeliveryGap {
    pub reason: &'static str,
    pub detail: String,
    /// The planned same-conversation consumption case a resumed turn must run.
    pub current_conversation: Option<PlannedConversationCase>,
}

/// The plan's `current_conversation_case` resolved against the delivered Plugin.
pub(super) struct PlannedConversationCase {
    pub plugin_id: String,
    pub action: String,
    pub input: serde_json::Value,
    pub expected_output: serde_json::Value,
}

/// A managed draft this conversation owns: context data, not an obligation.
pub(super) struct ConversationPluginDraft {
    pub draft_id: String,
    pub plugin_id: Option<String>,
    pub delivered: bool,
}

/// One draft row joined with its installed Plugin for `plugin_delivery_status`.
type PluginDeliveryRow = (
    Option<String>,
    String,
    Option<String>,
    Option<bool>,
    Option<i64>,
    Option<i64>,
    String,
);

/// The durable delivery verdict for one accepted request.
pub(super) enum PluginDeliveryState {
    /// No draft is planned for this request; the gate has nothing to check.
    Dormant,
    /// At least one draft is planned for this request and every planned
    /// output is installed, observed and, when planned, consumed in this
    /// conversation.
    Delivered,
    /// A host-owned gap blocks delivery.
    Gap(Box<PluginDeliveryGap>),
}

impl EngineSessionHost {
    pub(super) fn canonical_store(
        &self,
    ) -> Result<nomifun_agent_session::AgentSessionStore, AppError> {
        self.owner
            .upgrade()
            .map(|owner| owner.canonical().store().clone())
            .ok_or_else(|| AppError::Conflict("Session owner has shut down".into()))
    }

    /// A final model proposal is not proof of a Plugin delivery. Resolve the
    /// accepted user obligation and the current installed Artifact from owners.
    pub(super) async fn plugin_delivery_pending(
        &self, receipt: &EngineTurnReceipt,
    ) -> Result<Option<&'static str>, AppError> {
        Ok(match self.plugin_delivery_status(receipt).await? {
            PluginDeliveryState::Gap(gap) => Some(gap.reason),
            _ => None,
        })
    }

    /// The same durable resolution as `plugin_delivery_pending`, plus the
    /// actionable detail a same-turn settlement check relays to the model.
    pub(super) async fn plugin_delivery_status(
        &self, receipt: &EngineTurnReceipt,
    ) -> Result<PluginDeliveryState, AppError> {
        if !receipt.belongs_to(&self.source) {
            return Err(AppError::Conflict("delivery receipt belongs to another Host".into()));
        }
        if receipt.session().session().session_purpose != nomifun_agent_contracts::SessionPurpose::PluginAuthoring {
            return Ok(PluginDeliveryState::Dormant);
        }
        let requirement = receipt.request_payload().get("plugin_delivery").filter(|value| !value.is_null())
            .map(|value| serde_json::from_value::<nomifun_api_types::PluginDeliveryRequirement>(value.clone()))
            .transpose().map_err(|error| AppError::Conflict(error.to_string()))?;
        let draft_id = requirement.as_ref().and_then(|value| value.draft_id.as_deref());
        let owner = &receipt.session().principal().principal_id;
        let conversation = &receipt.session().session().conversation_id;
        let rows: Vec<PluginDeliveryRow> =
            sqlx::query_as(
                "SELECT d.source_message_id, d.verification_json, p.active_artifact_digest, p.enabled, p.trashed_at_ms, p.revision, d.draft_id FROM plugin_drafts d LEFT JOIN plugins p ON p.plugin_id = d.plugin_id AND p.owner_user_id = d.owner_user_id WHERE d.owner_user_id = ? AND d.source_conversation_id = ? AND ((? IS NOT NULL AND d.draft_id = ?) OR (? IS NULL AND (json_extract(d.verification_json, '$.task_message_id') = ? OR (? = 1 AND d.source_message_id = ?))))"
            ).bind(owner).bind(conversation).bind(draft_id).bind(draft_id).bind(draft_id)
                .bind(receipt.root_message_id()).bind(requirement.is_some()).bind(receipt.root_message_id()).fetch_all(&self.pool).await
                .map_err(|error| AppError::Internal(error.to_string()))?;
        if rows.is_empty() && requirement.is_none() { return Ok(PluginDeliveryState::Dormant); }
        let expected_count = requirement.as_ref().map_or(1, |value| usize::from(value.expected_count));
        if rows.len() < expected_count {
            let detail = if rows.is_empty() {
                if let Some(id) = requirement.as_ref().and_then(|value| value.draft_id.as_deref()) {
                    format!("the draft {id} named by this request does not exist in this conversation; tell the user instead of creating a substitute draft.")
                } else {
                    "no plugin draft exists for this request yet. Open the working draft (open with exactly {} for a new plugin, {\"plugin_id\":...} to change an installed plugin, {\"draft_id\":...} to continue an existing draft), then plan, apply, check, preview, run every planned case and install.".to_owned()
                }
            } else {
                format!("this request requires {expected_count} plugin outputs but only {} drafts exist. Create the missing drafts with open (empty arguments {{}}), then plan and verify each.", rows.len())
            };
            return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_DELIVERY_REQUIRED", detail, current_conversation: None })));
        }
        let mut declared_outputs:Option<serde_json::Value>=None;
        let mut delivered_outputs=std::collections::BTreeSet::new();
        for (source_message, raw, artifact, enabled, trash, revision, draft_id) in rows {
            let report: serde_json::Value = serde_json::from_str(&raw).map_err(|error| AppError::Internal(error.to_string()))?;
            if report["task_message_id"].as_str() != Some(receipt.root_message_id())
                && source_message.as_deref() != Some(receipt.root_message_id()) {
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_DELIVERY_REQUIRED",
                    detail: format!("draft {draft_id} is not attached to this request; call plan on it for this request before editing."), current_conversation: None })));
            }
            // An untouched baseline is not an output. Every edited draft must
            // first acquire an immutable plan, enforced by the module Host.
            if report["plan"].is_null() { continue; }
            if let Err(validation) = nomifun_plugin_development::validate_plan(&report["plan"]) {
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_VERIFICATION_REQUIRED",
                    detail: format!("draft {draft_id} has an invalid plan: {validation}"), current_conversation: None })));
            }
            if declared_outputs.as_ref().is_some_and(|outputs|outputs!=&report["plan"]["outputs"])
                || !delivered_outputs.insert(report["plan"]["output_key"].as_str().expect("validated").to_owned()) {
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_DELIVERY_REQUIRED",
                    detail: format!("draft {draft_id}: drafts for one request must declare the same outputs and each draft must use a distinct output_key."), current_conversation: None })));
            }
            declared_outputs=Some(report["plan"]["outputs"].clone());
            let Some(cases) = report["cases"].as_object() else {
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_VERIFICATION_REQUIRED",
                    detail: format!("draft {draft_id} has no recorded case results; run every planned case."), current_conversation: None })));
            };
            let mut missing = Vec::new();
            if report["structure_passed"] != serde_json::json!(true) {
                missing.push("structure diagnostics (run check)".to_owned());
            }
            if report["runtime_ready"] != serde_json::json!(true) {
                missing.push("runtime readiness (run preview)".to_owned());
            }
            if cases.is_empty() {
                missing.push("no planned case has run".to_owned());
            }
            let unpassed: std::collections::BTreeSet<String> = report["plan"]["cases"]
                .as_object().expect("validated plan").keys()
                .filter(|name| cases.get(*name).is_none_or(|case| case["passed"] != serde_json::json!(true)))
                .cloned().collect();
            if !unpassed.is_empty() {
                missing.push(format!("accepted cases not passed: {}", unpassed.into_iter().collect::<Vec<_>>().join(", ")));
            }
            if !nomifun_plugin_development::plan_evidence_complete(&report) {
                missing.push("plan evidence is incomplete".to_owned());
            }
            if !missing.is_empty() {
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_VERIFICATION_REQUIRED",
                    detail: format!("draft {draft_id} verification is incomplete: {}. Fix failures with apply, then check, preview and rerun the planned cases with their exact steps.", missing.join("; ")), current_conversation: None })));
            }
            if super::plugin_authoring::verification_context(&self.pool,&report["execution"]["credential_bindings"])
                .await.ok().as_ref() != Some(&report["context"]) {
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_VERIFICATION_REQUIRED",
                    detail: format!("draft {draft_id}: the execution context changed after verification; run preview and the planned cases again."), current_conversation: None })));
            }
            let context_digest=nomifun_agent_contracts::digest_payload(&report["context"])
                .map_err(|error|AppError::Internal(error.to_string()))?;
            let installed = enabled == Some(true) && trash.is_none() && artifact.is_some()
                && report["artifact_digest"].as_str() == artifact.as_deref()
                && report["delivery"]["artifact_digest"].as_str() == artifact.as_deref()
                && report["delivery"]["plugin_revision"].as_i64() == revision
                && report["delivery"]["installed_at_ms"].as_i64().is_some_and(|time| time > 0);
            let observed = report["installed_observation"]["artifact_digest"].as_str() == artifact.as_deref()
                && report["installed_observation"]["plugin_revision"].as_i64() == revision
                && report["installed_observation"]["context_digest"].as_str() == Some(context_digest.as_ref())
                && report["delivery"]["context_digest"].as_str() == Some(context_digest.as_ref())
                && report["installed_observation"]["observed_at_ms"].as_i64().is_some_and(|time| time > 0);
            let ui_observed = report["has_ui"] != serde_json::json!(true)
                || report["installed_observation"]["ui_ready"] == serde_json::json!(true);
            if !(installed && observed && ui_observed) {
                // install records delivery only after a matching installed
                // observation (including UI readiness), so this branch is
                // defensive; plugin-authored diagnostics such as ui_error
                // stay out of relayed host feedback.
                let detail = if !installed {
                    format!("draft {draft_id} passed verification but is not installed; call install with the latest verification_digest (from read or the last successful test), then inspect.")
                } else {
                    format!("draft {draft_id}: the recorded installed check does not match the current installation; call install again with the latest verification_digest (from read), then inspect.")
                };
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_DELIVERY_REQUIRED", detail, current_conversation: None })));
            }
            if !super::plugin_authoring::current_conversation_consumed(&self.pool,conversation,receipt.operation_id(),&report).await? {
                let case = &report["plan"]["current_conversation_case"];
                return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_CURRENT_CONVERSATION_PENDING",
                    detail: format!("draft {draft_id} is delivered but still needs to be consumed in this conversation."),
                    current_conversation: (!case.is_null()).then(|| PlannedConversationCase {
                        plugin_id: report["delivery"]["plugin_id"].as_str().unwrap_or_default().to_owned(),
                        action: case["action"].as_str().unwrap_or_default().to_owned(),
                        input: case["input"].clone(),
                        expected_output: case["expected_output"].clone(),
                    }) })));
            }
        }
        if delivered_outputs.len() < expected_count
            || declared_outputs.as_ref().is_none_or(|outputs|outputs.as_array().expect("validated").iter()
                .any(|output|!delivered_outputs.contains(output["key"].as_str().expect("validated")))) {
            let detail = match &declared_outputs {
                Some(outputs) => {
                    let keys: Vec<String> = outputs.as_array().expect("validated").iter()
                        .filter(|output| !delivered_outputs.contains(output["key"].as_str().expect("validated")))
                        .map(|output| output["key"].as_str().expect("validated").to_owned()).collect();
                    format!("the planned outputs are not delivered yet: {}. Complete apply, check, preview, the planned cases and install for each.", keys.join(", "))
                }
                None => "no plugin draft has an accepted plan for this request yet; call plan on a draft for this request before editing.".to_owned(),
            };
            return Ok(PluginDeliveryState::Gap(Box::new(PluginDeliveryGap { reason: "PLUGIN_DELIVERY_REQUIRED", detail, current_conversation: None })));
        }
        Ok(PluginDeliveryState::Delivered)
    }

    /// Managed drafts this conversation owns, newest first, as host-generated
    /// identifiers only (no model-written text). This is context only: the
    /// completion gate covers only drafts planned for the current request, so
    /// owning a draft never creates a delivery obligation.
    pub(super) async fn conversation_plugin_drafts(
        &self,
        owner: &str,
        conversation: &str,
    ) -> Result<Vec<ConversationPluginDraft>, AppError> {
        sqlx::query_as::<_, (String, Option<String>, bool)>(
            "SELECT draft_id, plugin_id, json_extract(verification_json,'$.delivery.artifact_digest') IS NOT NULL FROM plugin_drafts WHERE owner_user_id = ? AND source_conversation_id = ? ORDER BY updated_at_ms DESC, draft_id LIMIT 32",
        )
        .bind(owner)
        .bind(conversation)
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(|(draft_id, plugin_id, delivered)| {
            ConversationPluginDraft { draft_id, plugin_id, delivered }
        }).collect())
        .map_err(|error| AppError::Internal(error.to_string()))
    }

    /// Exact revision-selected Skill bytes, with inventory/hash verification.
    /// This supplies data only; each engine owns its context/media policy.
    /// No library path, activation, frontmatter execution or latest-version lookup.
    pub async fn read_selected_skills(
        &self, session: &AdmittedEngineSession,
    ) -> Result<super::engine_skills::SelectedSkills, AppError> {
        let context = self.open_kernel_session(session)?;
        let registry = context.registry_snapshot()?;
        let skills = super::engine_skills::compile(context.compiled(), &registry).await?;
        Ok(skills)
    }

    /// Compile against the production Kernel using the Conversation owner's
    /// workspace and the Agent's already-resolved typed resource grants.
    /// Reuses one active-set/resource context, never creates a parallel owner.
    pub fn open_kernel_session(
        &self,
        session: &AdmittedEngineSession,
    ) -> Result<Arc<super::engine_kernel_session::EngineKernelSession>, AppError> {
        if !Arc::ptr_eq(&self.source, &session.source) {
            return Err(AppError::Conflict(
                "Session belongs to another engine host".into(),
            ));
        }
        let mut sessions = self
            .kernel_sessions
            .lock()
            .map_err(|_| AppError::Conflict("Engine resource handles poisoned".into()))?;
        sessions.retain(|_, context| context.strong_count() > 0);
        let id = &session.session().conversation_id;
        if let Some(context) = sessions.get(id).and_then(Weak::upgrade) {
            if !context.matches(session) {
                return Err(AppError::Conflict(
                    "Live resource context differs from Session binding".into(),
                ));
            }
            return Ok(context);
        }
        if sessions.len() >= 4096 {
            return Err(AppError::Conflict(
                "Live resource context bound reached".into(),
            ));
        }
        let context = Arc::new(super::engine_kernel_session::EngineKernelSession::new(
            self.source.clone(),
            session,
            &self.resources,
        )?);
        sessions.insert(id.clone(), Arc::downgrade(&context));
        Ok(context)
    }

    /// A closed context can remain strongly referenced by old runtime handles.
    /// Retire only that exact cache instance after proven teardown, so a
    /// replacement never revives it and a late old close cannot evict a successor.
    pub(super) fn retire_kernel_session_after_teardown(
        &self,
        session_id: &str,
        context: &Arc<super::engine_kernel_session::EngineKernelSession>,
    ) -> Result<(), AppError> {
        if !context.teardown_proven()? {
            return Err(AppError::Conflict("Engine resource teardown is not proven".into()));
        }
        let mut sessions = self.kernel_sessions.lock()
            .map_err(|_| AppError::Conflict("Engine resource handles poisoned".into()))?;
        if sessions.get(session_id).and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, context)) {
            sessions.remove(session_id);
        }
        Ok(())
    }

    /// Rebuild only the Plugin consumer view after its proven safe pause.
    pub(super) async fn retire_plugin_view_after_pause(&self, receipt:&EngineTurnReceipt) -> Result<(),AppError> {
        if !receipt.belongs_to(&self.source) { return Err(AppError::Conflict("plugin view belongs to another Host".into())); }
        let id=&receipt.session().session().conversation_id;
        let context=self.kernel_sessions.lock().map_err(|_|AppError::Conflict("Engine resource handles poisoned".into()))?
            .get(id).and_then(Weak::upgrade);
        if let Some(context)=context {
            if !context.matches(receipt.session()) { return Err(AppError::Conflict("plugin view binding changed".into())); }
            context.cleanup_session().await?;
            let mut sessions=self.kernel_sessions.lock().map_err(|_|AppError::Conflict("Engine resource handles poisoned".into()))?;
            if sessions.get(id).and_then(Weak::upgrade).is_some_and(|current|Arc::ptr_eq(&current,&context)) { sessions.remove(id); }
        }
        Ok(())
    }

    /// Read the latest canonical cross-Agent handoff for the exact current
    /// binding. The payload is bounded data only; it is never converted into an
    /// AgentPriorTask, current requirement ledger, capability grant, or replay.
    pub async fn read_agent_handoff(
        &self,
        session: &AdmittedEngineSession,
    ) -> Result<Option<AgentHandoffEnvelopeV1>, AppError> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT inline_json, seq FROM agent_events \
             WHERE session_id = ? AND kind = 'session/agent-binding-changed' \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(&session.response.conversation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("read Agent handoff transition: {error}")))?;
        let Some((raw, transition_seq)) = row else {
            return Ok(None);
        };
        let transition: AgentBindingChangedPayloadV1 = serde_json::from_str(&raw)
            .map_err(|error| AppError::Conflict(format!("Agent handoff transition is invalid: {error}")))?;
        let current_ref = AgentHandoffBindingRefV1::from(&session.agent_binding);
        let target_reached_current = transition.next_binding_ref.binding_version
            < current_ref.binding_version
            || transition.next_binding_ref == current_ref;
        if !target_reached_current || transition.completion_gate_inherited
        {
            return Err(AppError::Conflict(
                "Agent handoff transition differs from the exact current binding".to_owned(),
            ));
        }
        if transition.handoff_mode == AgentHandoffMode::ContextOnly {
            if transition.handoff_payload_id.is_some() || transition.handoff_payload_digest.is_some()
            {
                return Err(AppError::Conflict(
                    "context-only Agent transition unexpectedly carries a handoff payload"
                        .to_owned(),
                ));
            }
            return Ok(None);
        }
        let (Some(payload_id), Some(expected_digest)) = (
            transition.handoff_payload_id.as_ref(),
            transition.handoff_payload_digest.as_ref(),
        ) else {
            return Err(AppError::Conflict(
                "continue-task Agent transition lost its handoff payload reference".to_owned(),
            ));
        };
        let payload: Option<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT body, digest FROM agent_payloads WHERE payload_id = ? AND session_id = ?",
        )
        .bind(payload_id.as_ref())
        .bind(&session.response.conversation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("read Agent handoff payload: {error}")))?;
        let Some((body, stored_digest)) = payload else {
            return Err(AppError::Conflict(
                "Agent handoff payload is missing".to_owned(),
            ));
        };
        if stored_digest != expected_digest.as_ref() {
            return Err(AppError::Conflict(
                "Agent handoff payload digest differs from the transition event".to_owned(),
            ));
        }
        let body: SessionPayloadBody = serde_json::from_slice(&body).map_err(|error| {
            AppError::Conflict(format!("Agent handoff payload body is invalid: {error}"))
        })?;
        let SessionPayloadBody::Json(value) = body else {
            return Err(AppError::Conflict(
                "Agent handoff payload is not canonical JSON".to_owned(),
            ));
        };
        let logical = nomifun_agent_contracts::canonical_json_bytes(&value.0)
            .map_err(|error| AppError::Conflict(error.to_string()))?;
        if digest_bytes(&logical) != *expected_digest {
            return Err(AppError::Conflict(
                "Agent handoff payload body failed digest verification".to_owned(),
            ));
        }
        let envelope: AgentHandoffEnvelopeV1 = serde_json::from_value(value.0)
            .map_err(|error| AppError::Conflict(format!("Agent handoff is invalid: {error}")))?;
        envelope
            .validate()
            .map_err(|error| AppError::Conflict(format!("Agent handoff is invalid: {error}")))?;
        if envelope.target_binding_ref != transition.next_binding_ref
            || envelope.source_binding_ref != transition.previous_binding_ref
            || envelope.source_agent_session_id.as_ref() != session.response.conversation_id
            || i64::try_from(envelope.source_through_seq)
                .ok()
                .is_none_or(|source| source >= transition_seq)
        {
            return Err(AppError::Conflict(
                "Agent handoff identity or source boundary is invalid".to_owned(),
            ));
        }
        Ok(Some(envelope))
    }

    /// Accept a prior turn's immutable Snapshot only when the control plane
    /// proves that it differs from the current Session binding solely by the
    /// selected Chat route. This keeps model switches history-preserving while
    /// preventing another Agent's tools, instructions or resource scope from
    /// entering replay.
    pub async fn historical_model_binding_compatible(
        &self,
        session: &AdmittedEngineSession,
        historical_snapshot_ref: &ResolvedSnapshotRef,
    ) -> Result<bool, AppError> {
        if historical_snapshot_ref == &session.snapshot.snapshot_ref {
            return Ok(true);
        }
        let raw: Option<String> = sqlx::query_scalar(
            "SELECT envelope_json FROM agent_runtime_snapshots \
             WHERE snapshot_id = ? AND snapshot_digest = ?",
        )
        .bind(historical_snapshot_ref.snapshot_id.as_ref())
        .bind(historical_snapshot_ref.snapshot_digest.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| {
            AppError::Internal(format!("read historical Agent Snapshot: {error}"))
        })?;
        let Some(raw) = raw else {
            return Ok(false);
        };
        let historical_snapshot: ResolvedSnapshotEnvelope = serde_json::from_str(&raw)
            .map_err(|error| {
                AppError::Conflict(format!("historical Agent Snapshot is invalid: {error}"))
            })?;
        historical_snapshot.validate().map_err(|error| {
            AppError::Conflict(format!(
                "historical Agent Snapshot is invalid: {}: {}",
                error.code.as_ref(),
                error.message,
            ))
        })?;
        if historical_snapshot.snapshot_ref != *historical_snapshot_ref {
            return Ok(false);
        }
        let historical_binding = AgentBindingValue {
            preset_revision_ref: historical_snapshot.content.preset_revision_ref.clone(),
            resolved_snapshot_ref: historical_snapshot.snapshot_ref,
            typed_resource_bindings: session.agent_binding.typed_resource_bindings.clone(),
            binding_version: session.agent_binding.binding_version,
        };
        self.control_plane
            .agent_session_model_history_compatible(
                &nomifun_agent_contracts::UserId::from(
                    session.principal.principal_id.clone(),
                ),
                &session.agent_binding,
                &historical_binding,
            )
            .await
            .map_err(|error| {
                AppError::Conflict(format!(
                    "historical Agent model binding validation failed: {error}"
                ))
            })
    }

    pub(super) fn compose_model_port_with_configuration(
        &self,
        gate: Arc<dyn nomifun_chat_model_broker::ChatCausalityGate>,
        configuration: Option<super::chat_broker_host::TurnModelConfiguration>,
    ) -> Result<Arc<dyn nomifun_chat_model_broker::EngineModelPort>, AppError> {
        let composition = configuration.map(|view| self.broker.for_turn_configuration(view))
            .unwrap_or_else(|| self.broker.clone());
        let invoke = composition.build_model_invoke(self.http.clone());
        let broker = composition
            .build_broker(
                gate,
                invoke,
                nomifun_chat_model_broker::BrokerRetryPolicy::default(),
            )
            .map_err(|error| AppError::Conflict(format!("Engine Broker assembly: {error}")))?;
        Ok(Arc::new(
            nomifun_chat_model_broker::BrokerEngineModelPort::new(broker),
        ))
    }

    pub(super) async fn capture_turn_model_configuration(
        &self, receipt: &EngineTurnReceipt, configuration: &super::chat_broker_host::TurnModelConfiguration,
    ) -> Result<super::engine_model_facts::EngineRouteModelFacts, AppError> {
        if !receipt.belongs_to(&self.source) {
            return Err(AppError::Conflict("model configuration receipt belongs to another Host".into()));
        }
        let identity = receipt.session().snapshot().content.chat_route_identity.as_ref()
            .ok_or_else(|| AppError::Conflict("Turn has no exact model identity".into()))?;
        let record = receipt.session().revision().payload.chat_route_records.get(&identity.model_task)
            .ok_or_else(|| AppError::Conflict("Turn model configuration is missing".into()))?;
        record.validate_for(identity).map_err(|error| AppError::Conflict(error.to_string()))?;
        configuration.capture(&self.pool, receipt.operation_id(), record).await
            .map_err(|error| match error {
                nomifun_chat_model_broker::ProductionRepositoryError::InvalidData => AppError::SessionConfigurationChanged(
                    "Model configuration changed before Turn preparation; resend after the current Turn settles".into()),
                error => AppError::Conflict(format!("Turn model configuration unavailable: {error}")),
            })?;
        configuration.model_facts(identity, record)
            .map_err(|_| AppError::Conflict("Turn model limits are invalid".into()))
    }

    pub async fn read_history(
        &self,
        receipt: &EngineTurnReceipt,
        limit: usize,
    ) -> Result<super::engine_history::EngineHistoryWindow, AppError> {
        if !Arc::ptr_eq(&self.source, &receipt.source) {
            return Err(AppError::Conflict(
                "Receipt belongs to another Session host".into(),
            ));
        }
        let owner = self.owner.upgrade().ok_or_else(|| {
            AppError::Conflict("Session owner has shut down".into())
        })?;
        super::engine_history::load(owner.canonical().store(), receipt, limit).await
    }

    /// Page toward older receipts within the admitted Session. The cursor is
    /// a previous turn's operation_id from read_history, not a new authority.
    /// No codec, tool replay or completeness/cleanup inference occurs here.
    pub async fn read_history_before(
        &self,
        receipt: &EngineTurnReceipt,
        limit: usize,
        before_operation: Option<&str>,
    ) -> Result<super::engine_history::EngineHistoryWindow, AppError> {
        if !Arc::ptr_eq(&self.source, &receipt.source) {
            return Err(AppError::Conflict(
                "Receipt belongs to another Session host".into(),
            ));
        }
        let owner = self.owner.upgrade().ok_or_else(|| {
            AppError::Conflict("Session owner has shut down".into())
        })?;
        super::engine_history::load_before(
            owner.canonical().store(),
            receipt,
            limit,
            before_operation,
        )
        .await
    }

    pub(super) async fn read_history_exact(
        &self,
        receipt: &EngineTurnReceipt,
        operation: &str,
    ) -> Result<super::engine_history::EngineHistoryWindow, AppError> {
        if !receipt.belongs_to(&self.source) {
            return Err(AppError::Conflict("Receipt belongs to another Session host".into()));
        }
        let owner = self.owner.upgrade()
            .ok_or_else(|| AppError::Conflict("Session owner has shut down".into()))?;
        super::engine_history::load_exact(owner.canonical().store(), receipt, operation).await
    }

    pub(crate) fn new(
        owner: &Arc<NomiCoreSessionOwner>,
        control_plane: Arc<AgentControlPlane>,
        engines: &Arc<OfficialRuntimeHost>,
        pool: SqlitePool,
        encryption_key: [u8; 32],
        resources: super::engine_kernel_session::EngineKernelAssembly,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            owner: Arc::downgrade(owner),
            control_plane,
            engines: Arc::downgrade(engines),
            broker: super::chat_broker_host::ChatBrokerHostComposition::for_nomi_core(
                pool.clone(),
                encryption_key,
            ),
            // No total response-body deadline: the model executor bounds
            // setup and complete-frame idle waits separately. Use the shared
            // platform proxy policy (including local exclusions on macOS) and
            // disable redirects so one broker attempt stays one HTTP send.
            http: nomifun_net::http_client_no_redirect()?,
            pool,
            source: Arc::new(()),
            execution_instance_id: uuid::Uuid::now_v7().to_string(),
            resources,
            kernel_sessions: Default::default(),
            journals: Default::default(),
        })
    }

    /// Reuses the same live writer for one exact receipt. This is a journal
    /// handle cache, not a second Session coordinator or an interrupted-turn
    /// recovery path. A released writer never resumes a durable prefix.
    pub async fn open_journal(
        &self,
        receipt: &EngineTurnReceipt,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<super::engine_journal::EngineTurnJournal, AppError> {
        let journal = self.claim_journal(receipt, cancellation).await?;
        journal.refresh_budget().await?;
        Ok(journal)
    }

    pub(super) async fn claim_journal(
        &self,
        receipt: &EngineTurnReceipt,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<super::engine_journal::EngineTurnJournal, AppError> {
        use super::engine_journal::EngineTurnJournal;
        if !Arc::ptr_eq(&self.source, &receipt.source) {
            return Err(AppError::Conflict(
                "Receipt belongs to another Session host".into(),
            ));
        }
        let mut journals = self.journals.lock().await;
        journals.retain(|_, journal| journal.strong_count() > 0);
        let key = (
            receipt.session().session().conversation_id.clone(),
            receipt.operation_id().to_owned(),
        );
        if let Some(journal) = journals.get(&key).and_then(Weak::upgrade)
            .filter(|journal| EngineTurnJournal::cached_generation(journal) == receipt.admission_epoch() as u64) {
            let journal = EngineTurnJournal::from_existing(journal, receipt)?;
            journal.verify_execution_lease().await?;
            return Ok(journal);
        }
        if journals.len() >= 4096 {
            return Err(AppError::Conflict(
                "Live engine journal bound reached".into(),
            ));
        }
        let owner = self
            .owner
            .upgrade()
            .ok_or_else(|| AppError::Conflict("Session owner has shut down".into()))?;
        let store = owner.canonical().store().clone();
        let lease = store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
            owner: receipt.session().principal().clone(),
            agent_session_id: receipt.session().session().conversation_id.clone().into(),
            operation_id: receipt.operation_id().into(), snapshot: receipt.session().snapshot().snapshot_ref.clone(),
            active_set_generation: receipt.session().active_set_generation(),
            holder: self.execution_instance_id.clone(), expected_fence: 0, checkpoint: None,
        }).await;
        let journal = match lease {
            Ok(lease) => match EngineTurnJournal::new(store.clone(), receipt, cancellation, lease.clone()) {
                Ok(journal) => journal,
                Err(error) => { let _ = store.release_unattached_native_claim(&lease).await; return Err(error); }
            },
            Err(nomifun_agent_session::SessionStoreError::ExecutionLeaseActive
                | nomifun_agent_session::SessionStoreError::ExecutionFenced
                | nomifun_agent_session::SessionStoreError::RecoveryRequiresReconciliation) => {
                self.open_recovered_journal(store, receipt, cancellation).await?
            }
            Err(error) => return Err(AppError::Conflict(format!("Engine execution lease: {error}"))),
        };
        journals.insert(key, journal.downgrade());
        Ok(journal)
    }

    /// Resolve an existing accepted turn, never create a competing receipt.
    /// Attachment/Skill interpretation remains with the platform adapters;
    /// callers must not treat the supplied payload as extra authorization.
    pub async fn read_turn_receipt(
        &self,
        options: &AgentRuntimeBuildOptions,
        binding: &RuntimeBuildBinding,
        expected_snapshot: &nomifun_agent_contracts::ResolvedSnapshotRef,
        message: &SendMessageData,
    ) -> Result<EngineTurnReceipt, AppError> {
        let conflict =
            |message: &str| AppError::Conflict(format!("Engine turn receipt: {message}"));
        let session = self.resolve(options, binding).await?;
        if &session.snapshot.snapshot_ref != expected_snapshot {
            return Err(conflict("Snapshot differs from the open engine Session"));
        }
        let root = message
            .source_message_id
            .as_deref()
            .unwrap_or(&message.msg_id);
        let session_id = nomifun_agent_contracts::AgentSessionId::from(
            options.conversation_id.clone(),
        );
        let owner = self
            .owner
            .upgrade()
            .ok_or_else(|| conflict("Session owner has shut down"))?;
        let head = owner
            .canonical()
            .store()
            .head(&session_id)
            .await
            .map_err(|error| conflict(&error.to_string()))?;
        let operation_id = head
            .active_turn_id
            .clone()
            .ok_or_else(|| conflict("no committed active turn authority"))?;
        let operation = nomifun_agent_contracts::OperationId::from(operation_id.clone());
        let receipt = owner
            .canonical()
            .store()
            .read_turn_receipt(&session_id, &operation)
            .await
            .map_err(|error| conflict(&error.to_string()))?;
        if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running {
            return Err(conflict("active turn is already terminal"));
        }
        let started = receipt
            .started_event
            .ok_or_else(|| conflict("active turn has no started event"))?;
        let facts = owner
            .canonical()
            .store()
            .chat_causality_facts(&session_id, &operation)
            .await
            .map_err(|error| conflict(&error.to_string()))?;
        let started_payload = facts
            .event_payloads
            .get(started.event_id.as_ref())
            .ok_or_else(|| conflict("active turn payload is unavailable"))?;
        let source_message_id = started_payload
            .get("source_message_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| conflict("active turn has no source message identity"))?;
        if source_message_id != root {
            return Err(conflict("runtime root differs from the accepted source message"));
        }
        let source_payload = facts
            .event_payloads
            .get(source_message_id)
            .ok_or_else(|| conflict("accepted source message payload is unavailable"))?;
        if source_payload
            .get("content")
            .and_then(serde_json::Value::as_str)
            != Some(message.content.as_str())
        {
            return Err(conflict("message text differs from its durable root"));
        }
        if binding_from_extra(&session.response.extra)?.as_ref() != Some(binding) {
            return Err(conflict("Session binding changed while reading the receipt"));
        }
        self.engines
            .upgrade()
            .ok_or_else(|| conflict("engine host has shut down"))?
            .provider()?
            .validate_session_extra(&session.response.extra)?;
        Ok(EngineTurnReceipt {
            source: self.source.clone(),
            session,
            root_message_id: source_message_id.to_owned(),
            turn_started_event_id: started.event_id.as_ref().to_owned(),
            operation_id,
            admission_epoch: i64::try_from(if facts.execution_generation > 0 { facts.execution_generation } else { started.seq })
                .map_err(|_| conflict("turn sequence exceeds runtime generation range"))?,
            request_payload: source_payload.clone(),
        })
    }

    /// Confirm the immutable cancellation of an accepted root that never
    /// acquired a native execution claim. This supplies no running authority
    /// and cannot substitute for cleanup after a Turn resource was opened.
    pub(super) async fn confirm_cancelled_before_execution(
        &self,
        options: &AgentRuntimeBuildOptions,
        binding: &RuntimeBuildBinding,
        expected_snapshot: &ResolvedSnapshotRef,
        message: &SendMessageData,
    ) -> Result<bool, AppError> {
        let conflict = |reason: &str| AppError::Conflict(format!("Engine cancellation receipt: {reason}"));
        let session = self.resolve(options, binding).await?;
        if &session.snapshot.snapshot_ref != expected_snapshot {
            return Err(conflict("Snapshot differs from the open engine Session"));
        }
        let root = message.source_message_id.as_deref().unwrap_or(&message.msg_id);
        let operations: Vec<String> = sqlx::query_scalar(
            "SELECT operation_id FROM agent_turns WHERE session_id=? AND source_message_id=? LIMIT 2",
        ).bind(&options.conversation_id).bind(root).fetch_all(&self.pool).await
            .map_err(|error| conflict(&error.to_string()))?;
        let [operation] = operations.as_slice() else {
            return Err(conflict("accepted root has no unique canonical Turn"));
        };
        let session_id = options.conversation_id.clone().into();
        let operation_id = operation.clone().into();
        let store = self.canonical_store()?;
        let receipt = store.read_turn_receipt(&session_id, &operation_id).await
            .map_err(|error| conflict(&error.to_string()))?;
        if receipt.status != nomifun_agent_session::TurnReceiptStatus::Cancelled { return Ok(false); }
        let started = receipt.started_event.ok_or_else(|| conflict("cancelled Turn has no started event"))?;
        let terminal = receipt.terminal_event.ok_or_else(|| conflict("cancelled Turn has no terminal event"))?;
        if started.agent_session_id != session_id || started.kind.0 != "turn/started"
            || started.correlation_id.as_ref() != operation || started.causation_event_id.as_ref().map(|id|id.as_ref()) != Some(root)
            || terminal.agent_session_id != session_id || terminal.kind.0 != "turn/cancelled"
            || terminal.correlation_id.as_ref() != operation || terminal.causation_event_id.as_ref() != Some(&started.event_id) {
            return Err(conflict("cancellation does not belong to the exact accepted Turn"));
        }
        let facts = store.chat_causality_facts(&session_id, &operation_id).await
            .map_err(|error| conflict(&error.to_string()))?;
        if facts.execution_generation != 0 || facts.head.status != "ready" || facts.head.active_turn_id.is_some()
            || facts.session.owner_ref != session.principal || facts.session.agent_binding.resolved_snapshot_ref != *expected_snapshot {
            return Err(conflict("cancelled Turn was claimed or its Session scope changed"));
        }
        let source = facts.events.iter().find(|event| event.event_id.as_ref() == root)
            .ok_or_else(|| conflict("accepted source message is missing"))?;
        let source_payload = facts.event_payloads.get(root).ok_or_else(|| conflict("accepted source payload is missing"))?;
        let started_payload = facts.event_payloads.get(started.event_id.as_ref())
            .ok_or_else(|| conflict("accepted Turn payload is missing"))?;
        if source.agent_session_id != session_id || source.kind.0 != "message/user-accepted"
            || source.correlation_id.as_ref() != root
            || source_payload.get("content").and_then(serde_json::Value::as_str) != Some(message.content.as_str())
            || started_payload.get("source_message_id").and_then(serde_json::Value::as_str) != Some(root) {
            return Err(conflict("message differs from its durable accepted root"));
        }
        let delivery = super::runtime_attachments::delivery(source_payload);
        let origin = match delivery.get("origin") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(origin)) => Some(origin.as_str()),
            Some(_) => return Err(conflict("accepted origin is not a string")),
        };
        if super::runtime_attachments::references(source_payload)? != message.files
            || super::runtime_attachments::selected_skills(source_payload)? != message.inject_skills
            || origin != message.origin.as_deref() {
            return Err(conflict("delivery metadata differs from its durable accepted root"));
        }
        let route = session.snapshot.content.chat_route_identity.as_ref()
            .ok_or_else(|| conflict("accepted Session has no exact route"))?;
        let snapshot_value = serde_json::to_value(expected_snapshot).map_err(|error| conflict(&error.to_string()))?;
        let route_value = serde_json::to_value(route).map_err(|error| conflict(&error.to_string()))?;
        for payload in [source_payload, started_payload] {
            let admission = payload.get("admission").ok_or_else(|| conflict("accepted root has no frozen admission scope"))?;
            if admission.get("resolved_snapshot_ref") != Some(&snapshot_value) || admission.get("route_identity") != Some(&route_value) {
                return Err(conflict("accepted root Snapshot or route differs from the frozen engine scope"));
            }
        }
        Ok(true)
    }

    /// Read-only acknowledgement of the exact terminal this host already
    /// published. It never changes the recorded outcome or creates authority.
    /// The caller must also retain its own successful-publication witness.
    pub(super) async fn confirm_published_terminal(
        &self,
        options: &AgentRuntimeBuildOptions,
        binding: &RuntimeBuildBinding,
        expected_snapshot: &ResolvedSnapshotRef,
        message: &SendMessageData,
        expected_event: &nomifun_agent_runtime::AgentEngineEvent,
    ) -> Result<bool, AppError> {
        let conflict = |reason: &str| AppError::Conflict(format!("Engine terminal receipt: {reason}"));
        let session = self.resolve(options, binding).await?;
        if &session.snapshot.snapshot_ref != expected_snapshot {
            return Err(conflict("Snapshot differs from the open engine Session"));
        }
        let root = message.source_message_id.as_deref().unwrap_or(&message.msg_id);
        let operations: Vec<String> = sqlx::query_scalar(
            "SELECT operation_id FROM agent_turns WHERE session_id=? AND source_message_id=? LIMIT 2",
        ).bind(&options.conversation_id).bind(root).fetch_all(&self.pool).await
            .map_err(|error| conflict(&error.to_string()))?;
        let [operation] = operations.as_slice() else {
            return Err(conflict("accepted root has no unique canonical Turn"));
        };
        let session_id = options.conversation_id.clone().into();
        let operation_id = operation.clone().into();
        let store = self.canonical_store()?;
        let receipt = store.read_turn_receipt(&session_id, &operation_id).await
            .map_err(|error| conflict(&error.to_string()))?;
        let expected_kind = match expected_event {
            nomifun_agent_runtime::AgentEngineEvent::TurnCompleted { .. } => "turn/completed",
            nomifun_agent_runtime::AgentEngineEvent::TurnCancelled { .. } => "turn/cancelled",
            nomifun_agent_runtime::AgentEngineEvent::TurnPaused { .. } => "turn/paused",
            nomifun_agent_runtime::AgentEngineEvent::TurnFailed { .. } => "turn/failed",
            _ => return Err(conflict("terminal acknowledgement requires a terminal Runtime event")),
        };
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::Running { return Ok(false); }
        let started = receipt.started_event.ok_or_else(|| conflict("published Turn has no started event"))?;
        let terminal = receipt.terminal_event.ok_or_else(|| conflict("published Turn has no terminal event"))?;
        if started.agent_session_id != session_id || started.kind.0 != "turn/started"
            || started.correlation_id.as_ref() != operation || started.causation_event_id.as_ref().map(|id|id.as_ref()) != Some(root)
            || terminal.agent_session_id != session_id || terminal.kind.0 != expected_kind
            || terminal.correlation_id.as_ref() != operation || terminal.seq <= started.seq {
            return Err(conflict("terminal does not belong to the exact accepted Turn"));
        }
        let facts = store.chat_causality_facts(&session_id, &operation_id).await
            .map_err(|error| conflict(&error.to_string()))?;
        if facts.execution_generation == 0 || facts.session.owner_ref != session.principal || facts.session.agent_binding.resolved_snapshot_ref != *expected_snapshot {
            return Err(conflict("published Runtime Turn lacks a claim or its Session scope changed"));
        }
        let source = facts.events.iter().find(|event| event.event_id.as_ref() == root)
            .ok_or_else(|| conflict("accepted source message is missing"))?;
        let source_payload = facts.event_payloads.get(root).ok_or_else(|| conflict("accepted source payload is missing"))?;
        let started_payload = facts.event_payloads.get(started.event_id.as_ref())
            .ok_or_else(|| conflict("accepted Turn payload is missing"))?;
        if source.agent_session_id != session_id || source.kind.0 != "message/user-accepted"
            || source.correlation_id.as_ref() != root
            || source_payload.get("content").and_then(serde_json::Value::as_str) != Some(message.content.as_str())
            || started_payload.get("source_message_id").and_then(serde_json::Value::as_str) != Some(root) {
            return Err(conflict("message differs from its durable accepted root"));
        }
        let delivery = super::runtime_attachments::delivery(source_payload);
        let origin = match delivery.get("origin") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(origin)) => Some(origin.as_str()),
            Some(_) => return Err(conflict("accepted origin is not a string")),
        };
        if super::runtime_attachments::references(source_payload)? != message.files
            || super::runtime_attachments::selected_skills(source_payload)? != message.inject_skills
            || origin != message.origin.as_deref() {
            return Err(conflict("delivery metadata differs from its durable accepted root"));
        }
        let route = session.snapshot.content.chat_route_identity.as_ref()
            .ok_or_else(|| conflict("accepted Session has no exact route"))?;
        let snapshot_value = serde_json::to_value(expected_snapshot).map_err(|error| conflict(&error.to_string()))?;
        let route_value = serde_json::to_value(route).map_err(|error| conflict(&error.to_string()))?;
        for payload in [source_payload, started_payload] {
            let admission = payload.get("admission").ok_or_else(|| conflict("accepted root has no frozen admission scope"))?;
            if admission.get("resolved_snapshot_ref") != Some(&snapshot_value) || admission.get("route_identity") != Some(&route_value) {
                return Err(conflict("accepted root Snapshot or route differs from the frozen engine scope"));
            }
        }
        let expected_event = serde_json::to_value(expected_event).map_err(|error| conflict(&error.to_string()))?;
        let matching = facts.events.iter().filter(|event| event.agent_session_id == session_id
            && event.kind.0 == "runtime/progress-recorded"
            && event.correlation_id.as_ref() == operation
            && event.seq > started.seq && event.seq < terminal.seq
            && facts.event_payloads.get(event.event_id.as_ref())
                .and_then(|payload| payload.get("event")) == Some(&expected_event)).count();
        if matching != 1 { return Err(conflict("terminal retry differs from its published Runtime event")); }
        let causation = terminal.causation_event_id.as_ref().ok_or_else(|| conflict("terminal has no causation event"))?;
        if !facts.events.iter().any(|event| &event.event_id == causation
            && event.agent_session_id == session_id && event.seq >= started.seq && event.seq < terminal.seq) {
            return Err(conflict("terminal causation differs from its exact Turn facts"));
        }
        Ok(true)
    }

    pub async fn resolve(
        &self,
        options: &AgentRuntimeBuildOptions,
        binding: &RuntimeBuildBinding,
    ) -> Result<AdmittedEngineSession, AppError> {
        let conflict =
            |message: &str| AppError::Conflict(format!("Engine Session admission: {message}"));
        nomifun_common::UserId::parse(&options.user_id)
            .map_err(|_| conflict("noncanonical owner"))?;
        nomifun_common::ConversationId::parse(&options.conversation_id)
            .map_err(|_| conflict("noncanonical Session"))?;
        binding.validate()?;
        super::hosted_effect_receipts::HostedEffectReceipts::new(self.pool.clone())
            .ensure_settled(&options.user_id, &options.conversation_id).await?;
        let owner = self
            .owner
            .upgrade()
            .ok_or_else(|| conflict("Session owner has shut down"))?;
        let engines = self
            .engines
            .upgrade()
            .ok_or_else(|| conflict("engine host has shut down"))?;
        let session_id = nomifun_agent_contracts::AgentSessionId::from(
            options.conversation_id.clone(),
        );
        let response = owner
            .canonical_conversation_projection(&options.user_id, &session_id)
            .await?
            .ok_or_else(|| conflict("canonical AgentSession was not found"))?;
        let persisted_binding = binding_from_extra(&response.extra)?;
        if response.conversation_id != options.conversation_id
            || persisted_binding.as_ref() != Some(binding)
        {
            tracing::warn!(
                requested_binding = ?binding,
                persisted_binding = ?persisted_binding,
                "durable Session engine binding mismatch"
            );
            return Err(conflict(
                "durable Session engine binding differs from runtime request",
            ));
        }
        let provider = engines.provider()?;
        provider.validate_binding(binding)?;
        provider.validate_session_extra(&response.extra)?;
        let authenticated = AuthenticatedOwner(nomifun_agent_contracts::UserId::from(
            options.user_id.clone(),
        ));
        let metadata = session_metadata(&response, &authenticated)
            .map_err(|error| conflict(&error.message))?;
        let dto = serde_json::from_value(
            serde_json::to_value(&metadata.binding)
                .map_err(|error| conflict(&error.to_string()))?,
        )
        .map_err(|error| conflict(&error.to_string()))?;
        let (agent_binding, revision, snapshot) = self
            .control_plane
            .saved_binding_artifacts(&authenticated.0, &dto)
            .await
            .map_err(|error| conflict(&error.to_string()))?;
        let principal = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: options.user_id.clone(),
        };
        if agent_binding.preset_revision_ref != revision.reference
            || agent_binding.resolved_snapshot_ref != snapshot.snapshot_ref
            || snapshot.content.preset_revision_ref != revision.reference
            || snapshot.actor != principal
        {
            return Err(conflict(
                "Agent revision/Snapshot/owner identity chain differs",
            ));
        }
        provider.validate_snapshot(&snapshot)?;
        let head = owner
            .canonical()
            .store()
            .head(&session_id)
            .await
            .map_err(|error| conflict(&error.to_string()))?;
        let active_capability_ids = owner
            .canonical()
            .store()
            .active_capability_ids(&session_id)
            .await
            .map_err(|error| conflict(&error.to_string()))?;
        let workspace = std::path::PathBuf::from(&options.workspace);
        if !workspace.is_absolute() {
            return Err(conflict("runtime workspace is not an absolute host path"));
        }
        let workspace = dunce::canonicalize(&workspace)
            .map_err(|_| conflict("runtime workspace is unavailable"))?;
        if let Some(persisted_workspace) = response
            .extra
            .get("workspace")
            .and_then(serde_json::Value::as_str)
            .filter(|workspace| !workspace.trim().is_empty())
        {
            let persisted_workspace = dunce::canonicalize(persisted_workspace)
                .map_err(|_| conflict("persisted workspace is unavailable"))?;
            if persisted_workspace != workspace {
                return Err(conflict(
                    "runtime workspace differs from the canonical Session projection",
                ));
            }
        }
        let workspace = workspace
            .to_str()
            .ok_or_else(|| conflict("runtime workspace is not UTF-8"))?
            .to_owned();
        Ok(AdmittedEngineSession {
            source: self.source.clone(),
            response,
            engine_binding: binding.clone(),
            agent_binding,
            revision,
            snapshot,
            principal,
            workspace,
            active_set_generation: head.active_set_generation,
            active_capability_ids,
        })
    }
}
