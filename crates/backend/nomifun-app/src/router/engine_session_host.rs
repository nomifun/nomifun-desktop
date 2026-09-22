//! Engine-neutral resolution of immutable facts from the existing Conversation
//! owner. This is not a new Session store or permission authority. The returned
//! snapshot can be compiled through open_kernel_session with real resource
//! owners; engines do not supply their own workspace or permission authority.
use std::sync::{Arc, Weak};

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetRevision, PrincipalRef, ResolvedSnapshotEnvelope,
    ResolvedSnapshotRef,
};
use nomifun_agent_control_plane::{AgentControlPlane, AuthenticatedOwner};
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_api_types::{ConversationResponse, RuntimeBuildBinding};
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};

use super::nomi_core_session::{NomiCoreSessionOwner, session_metadata};
use super::official_runtime::{OfficialRuntimeHost, binding_from_extra};

/// Constructed by application assembly only. The sole official Driver receives
/// this typed port owner from the composition root; there is no runtime
/// registration or direct Domain-handler access path.
pub struct EngineSessionHost {
    skill_artifacts: Arc<nomifun_plugin_platform::application::FsPluginArtifactStore>,
    owner: Weak<NomiCoreSessionOwner>,
    control_plane: Arc<AgentControlPlane>,
    engines: Weak<OfficialRuntimeHost>,
    pool: SqlitePool,
    broker: super::chat_broker_host::ChatBrokerHostComposition,
    http: reqwest::Client,
    source: Arc<()>,
    resources: super::engine_kernel_session::EngineKernelAssembly,
    kernel_sessions: std::sync::Mutex<
        std::collections::BTreeMap<String, Weak<super::engine_kernel_session::EngineKernelSession>>,
    >,
    journals: std::sync::Mutex<
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
    pub fn execution_constraints(&self) -> Result<nomifun_api_types::ExecutionConstraints, AppError> {
        nomifun_api_types::ExecutionConstraints::from_extra(&self.response.extra)
    }
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

    /// Exact revision-selected Skill bytes, with inventory/hash verification.
    /// This supplies data only; each engine owns its context/media policy.
    /// No library path, activation, frontmatter execution or latest-version lookup.
    pub async fn read_selected_skills(
        &self, session: &AdmittedEngineSession,
    ) -> Result<super::engine_skills::SelectedSkills, AppError> {
        let context = self.open_kernel_session(session)?;
        let registry = context.registry_snapshot()?;
        let skills = super::engine_skills::compile(context.compiled(), &registry, self.skill_artifacts.clone()).await?;
        skills.validate_extra(&session.session().extra)?;
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

    pub async fn read_model_facts(
        &self,
        session: &AdmittedEngineSession,
    ) -> Result<super::engine_model_facts::EngineRouteModelFacts, AppError> {
        if !Arc::ptr_eq(&self.source, &session.source) {
            return Err(AppError::Conflict(
                "Session belongs to another engine host".into(),
            ));
        }
        super::engine_model_facts::load(&self.pool, session).await
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

    /// Actual product Broker bound to this accepted turn's journal gate.
    /// The engine must record each model operation before opening its stream.
    /// Routes, credentials and retries stay behind the platform Broker.
    pub fn open_model_port(
        &self,
        receipt: &EngineTurnReceipt,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<
        (
            super::engine_journal::EngineTurnJournal,
            Arc<dyn nomifun_chat_model_broker::EngineModelPort>,
        ),
        AppError,
    > {
        if receipt
            .session()
            .snapshot()
            .content
            .chat_route_identity
            .is_none()
        {
            return Err(AppError::Conflict(
                "Engine Session has no exact chat route".into(),
            ));
        }
        let journal = self.open_journal(receipt, cancellation)?;
        let model = self.compose_model_port(Arc::new(journal.clone()))?;
        let model = self
            .open_kernel_session(receipt.session())?
            .wrap_model_middleware(model)?;
        Ok((journal, model))
    }

    /// Trusted application adapters can add their own live state fences over
    /// the same journal gate (the Runtime also fences capability activation).
    pub(super) fn compose_model_port(
        &self,
        gate: Arc<dyn nomifun_chat_model_broker::ChatCausalityGate>,
    ) -> Result<Arc<dyn nomifun_chat_model_broker::EngineModelPort>, AppError> {
        let invoke = self.broker.build_model_invoke(self.http.clone());
        let broker = self
            .broker
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

    pub async fn read_message_history(
        &self,
        receipt: &EngineTurnReceipt,
        limit: usize,
        byte_limit: usize,
    ) -> Result<super::engine_history::EngineMessageHistoryWindow, AppError> {
        if !Arc::ptr_eq(&self.source, &receipt.source) {
            return Err(AppError::Conflict(
                "Receipt belongs to another Session host".into(),
            ));
        }
        let owner = self.owner.upgrade().ok_or_else(|| {
            AppError::Conflict("Session owner has shut down".into())
        })?;
        super::engine_history::load_messages(
            owner.canonical().store(),
            receipt,
            limit,
            byte_limit,
        )
        .await
    }

    /// Data-only messages strictly before a historical turn. Engines can seed
    /// their native replay with an imported/forked prefix without duplicating
    /// messages belonging to replayed turns. Same owner, active generation,
    /// clear-context floor and resource bounds as the other history readers.
    /// An unaffordable first prefix row yields an empty window with has_older,
    /// not permission to skip it and search still older rows.
    pub async fn read_message_history_before_turn(
        &self,
        receipt: &EngineTurnReceipt,
        before_operation: &str,
        limit: usize,
        byte_limit: usize,
    ) -> Result<super::engine_history::EngineMessageHistoryWindow, AppError> {
        if !Arc::ptr_eq(&self.source, &receipt.source) {
            return Err(AppError::Conflict("Receipt belongs to another Session host".into()));
        }
        let owner = self.owner.upgrade().ok_or_else(|| {
            AppError::Conflict("Session owner has shut down".into())
        })?;
        super::engine_history::load_messages_before(
            owner.canonical().store(), receipt, limit, byte_limit, Some(before_operation),
        ).await
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

    pub(crate) fn new(
        owner: &Arc<NomiCoreSessionOwner>,
        control_plane: Arc<AgentControlPlane>,
        engines: &Arc<OfficialRuntimeHost>,
        pool: SqlitePool,
        encryption_key: [u8; 32],
        resources: super::engine_kernel_session::EngineKernelAssembly,
        skill_artifacts: Arc<nomifun_plugin_platform::application::FsPluginArtifactStore>,
    ) -> Self {
        Self {
            skill_artifacts,
            owner: Arc::downgrade(owner),
            control_plane,
            engines: Arc::downgrade(engines),
            broker: super::chat_broker_host::ChatBrokerHostComposition::for_nomi_core(
                pool.clone(),
                encryption_key,
            ),
            // No total response-body deadline: the model executor bounds
            // setup and complete-frame idle waits separately. Keep reqwest's
            // default system proxy/TLS policy for this shared engine port.
            http: reqwest::Client::new(),
            pool,
            source: Arc::new(()),
            resources,
            kernel_sessions: Default::default(),
            journals: Default::default(),
        }
    }

    /// Reuses the same live writer for one exact receipt. This is a journal
    /// handle cache, not a second Session coordinator or an interrupted-turn
    /// recovery path. A released writer never resumes a durable prefix.
    pub fn open_journal(
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
        let mut journals = self
            .journals
            .lock()
            .map_err(|_| AppError::Internal("Engine journal handles poisoned".into()))?;
        journals.retain(|_, journal| journal.strong_count() > 0);
        let key = (
            receipt.session().session().conversation_id.clone(),
            receipt.operation_id().to_owned(),
        );
        if let Some(journal) = journals.get(&key).and_then(Weak::upgrade) {
            return EngineTurnJournal::from_existing(journal, receipt);
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
        let journal = EngineTurnJournal::new(
            owner.canonical().store().clone(),
            receipt,
            cancellation,
        )?;
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
            admission_epoch: i64::try_from(started.seq)
                .map_err(|_| conflict("turn sequence exceeds runtime generation range"))?,
            request_payload: source_payload.clone(),
        })
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
        })
    }
}
