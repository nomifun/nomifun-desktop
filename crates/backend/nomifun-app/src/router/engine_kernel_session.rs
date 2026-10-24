//! Production Kernel/resource assembly shared by source-integrated engines.
//! Strategy, tool naming and event codecs remain engine-owned. This module
//! retains actual resource owners and their cleanup witnesses, not fake exits.
use std::sync::{Arc, Mutex, Weak};

use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomifun_agent_contracts::{AgentSessionId, ScopeKey};
#[cfg(feature = "browser-use")]
use nomifun_agent_contracts::OperationId;
use nomifun_agent_kernel::{
    CompiledSnapshot, CompilerEnvironment, KernelRegistry, MaterializedRegistry,
    SessionCapabilityState,
};
use nomifun_ai_agent::engine_effect_scope::guard_effect_settlement;
use nomifun_ai_agent::context_contributor::ContextContributor;
use nomifun_api_types::{ExecutionConstraints, RuntimeBuildBinding};
use nomifun_common::AppError;
use nomifun_engine_core::{EngineToolExposure, EngineToolPlan, KernelEngineToolInvoker};

use super::engine_journal::EngineTurnJournal;
use super::engine_session_host::{AdmittedEngineSession, EngineTurnReceipt};
use super::engine_tool_host::EngineToolHost;

type Completion = Shared<BoxFuture<'static, Result<(), String>>>;

fn completed_failure(done:&Option<Completion>)->bool {
    done.as_ref().and_then(|done|done.peek()).is_some_and(Result::is_err)
}

fn retry_settlement(done:&Option<Completion>,release_started:bool)->bool {
    !release_started&&completed_failure(done)
}

#[derive(Default)]
struct CapabilityContext {
    initial: Option<String>,
    before_turn: Option<Arc<dyn ContextContributor>>,
}

struct ConstrainedTools {
    inner: Arc<dyn nomifun_engine_core::EngineToolInvoker>,
    plan: EngineToolPlan,
    constraints: ExecutionConstraints,
    wave2: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    git_root: Option<std::path::PathBuf>,
}

/// Schema visibility is a model-presentation choice, not execution authority.
/// Media hints and ToolSearch may reveal an already frozen deferred Tool. All
/// names, schemas, digests, actions, resources and effect labels must still be
/// byte-for-byte equal to the admitted binding.
pub(super) fn frozen_tool_binding<'a>(
    plan: &'a EngineToolPlan,
    provider_name: &str,
    presented: &nomifun_engine_core::EngineToolBinding,
) -> Option<&'a nomifun_engine_core::EngineToolBinding> {
    let frozen = plan.binding(provider_name)?;
    let mut normalized = frozen.clone();
    normalized.definition.deferred = presented.definition.deferred;
    (normalized == *presented).then_some(frozen)
}

fn frozen_invocation_binding<'a>(
    plan: &'a EngineToolPlan,
    invocation: &nomifun_engine_core::EngineToolInvocation,
) -> Option<&'a nomifun_engine_core::EngineToolBinding> {
    frozen_tool_binding(plan, &invocation.call.name, &invocation.binding)
}

#[async_trait::async_trait]
impl nomifun_engine_core::EngineToolInvoker for ConstrainedTools {
    async fn invoke(
        &self,
        mut invocation: nomifun_engine_core::EngineToolInvocation,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<nomifun_engine_core::EngineToolResult, nomifun_engine_core::EngineToolError> {
        let frozen = frozen_invocation_binding(&self.plan, &invocation);
        if frozen.is_none() || !constraints_allow_action(
                self.constraints,
                invocation.binding.capability_id.as_ref(),
                invocation.binding.action_id.as_ref(),
            )
        {
            return Err(nomifun_engine_core::EngineToolError::ToolInvocation(
                "Tool is outside the frozen Session execution ceiling".into(),
            ));
        }
        // Downstream Kernel, Plugin and Robot owners retain their exact frozen
        // binding checks. Restore that binding after validating the sole
        // permitted presentation difference, before any host adapter runs.
        invocation.binding = frozen.expect("binding checked above").clone();
        if let Some(root) = &self.git_root {
            self.wave2
                .ensure_workspace_git_evidence(root)
                .await
                .map_err(|error| {
                    nomifun_engine_core::EngineToolError::ToolInvocation(error.to_string())
                })?;
        }
        self.inner.invoke(invocation, cancellation).await
    }
}

pub(crate) struct EngineKernelAssembly {
    #[cfg(feature = "computer-use")]
    pub computer: Arc<nomifun_computer::Computer>,
    pub kernel: Arc<KernelRegistry>,
    pub environment: CompilerEnvironment,
    pub wave2: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    pub context_admission: Arc<nomifun_ai_agent::NomiPlatformBuiltinContextAdmission>,
    pub knowledge: Arc<nomifun_knowledge::KnowledgeService>,
    pub companion: Weak<nomifun_companion::CompanionService>,
    #[cfg(feature = "browser-use")]
    pub browser: Arc<super::engine_browser_tools::BrowserRoleOwner>,
    pub hosted_effects: super::hosted_effect_receipts::HostedEffectReceipts,
    pub robot: Option<Arc<super::nomi_core_robot::RobotModuleOwner>>,
}

/// AppServices is the strong lifecycle owner. Runtime factories/resources may
/// point back through Companion Session ports, so retaining an Arc here would
/// form a service -> Session owner -> Runtime factory -> service cycle.
pub(super) fn companion_owner(handle:&Weak<nomifun_companion::CompanionService>)
    ->Result<Arc<nomifun_companion::CompanionService>,AppError>{
    handle.upgrade().ok_or_else(||failure("Companion memory owner is unavailable"))
}

fn paired_companion_fact_owner(
    principal:&nomifun_agent_contracts::PrincipalRef,compiled:&CompiledSnapshot,
)->Result<String,AppError>{
    if principal.principal_kind!="user" {return Err(failure("Companion Context requires its user owner"));}
    let context=compiled.policy(&"companion".into()).ok_or_else(||failure("Companion Context policy missing"))?;
    let memory=compiled.policy(&"companion.memory".into()).ok_or_else(||failure("Companion memory policy missing"))?;
    let contexts=compiled.target_resource_bindings.iter().filter(|binding|
        context.resource_binding_ids.contains(&binding.binding_id)&&binding.resource_kind.as_ref()=="companion")
        .collect::<Vec<_>>();
    let [context]=contexts.as_slice() else {return Err(failure("Companion Context requires exactly one bound companion resource"));};
    if context.owner_id!=principal.principal_id || !context.operations.contains("read")
        || context.binding_id.as_ref().trim().is_empty() || context.resource_id.as_ref().trim().is_empty() {
        return Err(failure("Companion Context resource is not readable by this owner"));
    }
    let resources=compiled.target_resource_bindings.iter().filter(|binding|memory.resource_binding_ids.contains(&binding.binding_id))
        .cloned().collect::<Vec<_>>();
    let resource=super::agent_memory_authority::authorize_memory_resource(
        &principal.principal_id,super::agent_memory_authority::ProductMemoryAction::CompanionRecall,&resources)?;
    if resource.resource_id!=context.resource_id {return Err(failure("Companion Context and memory resources identify different companions"));}
    Ok(resource.resource_id.as_ref().to_owned())
}

fn constraints_allow_action(
    constraints: ExecutionConstraints,
    capability_id: &str,
    action_id: &str,
) -> bool {
    if constraints.exclude_delegation && capability_id == "agent.collaboration"
        && matches!(action_id, "agent/delegate" | "agent/fork") {
        return false;
    }
    constraints.allows_workspace_action(capability_id, action_id)
}

#[path = "engine_mcp_resources.rs"]
mod mcp_resources;
#[path = "engine_knowledge_writeback.rs"]
mod knowledge_writeback;

struct Turn {
    operation: String,
    generation: u64,
    root: String,
    cleanup: Option<Completion>,
    resource_operations: std::collections::BTreeSet<String>,
    file_capture: Option<super::engine_turn_files::TurnFileCapture>,
}
#[derive(Default)]
struct State {
    tools: Option<Arc<EngineToolHost>>,
    turn: Option<Turn>,
    release: Option<Completion>,
    kernel_release_started: bool,
}

/// One production resource context per live registered Session runtime.
/// Created only by EngineSessionHost from genuine owner-resolved facts.
pub struct EngineKernelSession {
    #[cfg(feature = "computer-use")]
    computer: Arc<nomifun_computer::Computer>,
    #[cfg(feature = "computer-use")]
    computer_attempt: Arc<super::computer_host::ComputerAttemptSurface>,
    resource_tasks: nomifun_ai_agent::engine_sdk::EngineTaskGroup,
    knowledge_writebacks: knowledge_writeback::KnowledgeWritebacks,
    resource_settlements: mcp_resources::ResourceSettlements,
    source: Arc<()>,
    session_id: AgentSessionId,
    principal: nomifun_agent_contracts::PrincipalRef,
    binding: RuntimeBuildBinding,
    constraints: ExecutionConstraints,
    workspace: String,
    git_root: Option<std::path::PathBuf>,
    process_selected: bool,
    route_image_input: bool,
    creation_turn_root: super::engine_creation_tools::CreationTurnRoot,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    kernel: Arc<KernelRegistry>,
    wave2: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    context_admission: Arc<nomifun_ai_agent::NomiPlatformBuiltinContextAdmission>,
    knowledge: Arc<nomifun_knowledge::KnowledgeService>,
    companion: Weak<nomifun_companion::CompanionService>,
    #[cfg(feature = "browser-use")]
    browser: Arc<super::engine_browser_tools::BrowserRoleOwner>,
    hosted_effects: super::hosted_effect_receipts::HostedEffectReceipts,
    initial_capability_context: tokio::sync::OnceCell<CapabilityContext>,
    workspace_watch_context: Option<Arc<dyn ContextContributor>>,
    robot: Option<Arc<super::nomi_core_robot::RobotModuleOwner>>,
    robot_tools: tokio::sync::OnceCell<Option<Arc<super::engine_robot_tools::FrozenTools>>>,
    state: Mutex<State>,
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine resources: {message}"))
}

fn explicitly_requests_knowledge_writeback(text: &str) -> bool {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty()
        || [
            "不要写入知识库",
            "不要回写知识库",
            "不要保存到知识库",
            "别写入知识库",
            "别保存到知识库",
            "无需写入知识库",
            "do not write to the knowledge base",
            "do not save to the knowledge base",
            "don't write to the knowledge base",
            "don't save to the knowledge base",
        ]
        .iter()
        .any(|negative| normalized.contains(negative))
    {
        return false;
    }
    let names_knowledge = normalized.contains("知识库")
        || normalized.contains("knowledge base")
        || normalized.contains("knowledge-base");
    names_knowledge
        && [
            "写入", "回写", "回血", "记录", "保存", "沉淀", "write", "save", "record",
            "remember", "persist",
        ]
        .iter()
        .any(|verb| normalized.contains(verb))
}

impl EngineKernelSession {
    pub(super) fn new(
        source: Arc<()>,
        session: &AdmittedEngineSession,
        assembly: &EngineKernelAssembly,
    ) -> Result<Self, AppError> {
        let mut binding = session.agent_binding().clone();
        let mut knowledge_policy_error = None;
        binding.typed_resource_bindings.retain_mut(|resource| {
            if resource.resource_kind.as_ref()
                != nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
            {
                return true;
            }
            match nomifun_agent_domain_wave1::effective_agent_knowledge_operations(resource) {
                Ok(operations) if !operations.is_empty() => {
                    resource.operations = operations;
                    true
                }
                Ok(_) => false,
                Err(error) => {
                    knowledge_policy_error = Some(error);
                    false
                }
            }
        });
        if let Some(error) = knowledge_policy_error {
            return Err(failure(error));
        }
        let constraints = ExecutionConstraints::from_extra(&session.session().extra)?;
        let snapshot = session.snapshot();
        // Restored/inherited contracts are checked as frozen facts. Runtime
        // construction must never repair missing grants or resolve a newer
        // authoring configuration on their behalf.
        super::nomi_core_resource_bindings::validate_executable_binding_resources(
            snapshot, &binding.typed_resource_bindings,
        )?;
        let session_id = AgentSessionId::from(session.session().conversation_id.clone());
        let principal = session.principal();
        // EngineSessionHost already validated this host-resolved path against
        // the canonical Session projection (or its canonical fallback). It is
        // never read from model JSON.
        let workspace = session.workspace().to_owned();
        let selected = || {
            snapshot
                .content
                .enabled_capabilities
                .iter()

        };
        let workspace_files_selected = selected().any(|item| {
            item.capability.id.as_ref() == nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID
                && item.action_allowlist.iter().any(|action_id| {
                    constraints_allow_action(
                        constraints,
                        item.capability.id.as_ref(),
                        action_id.as_ref(),
                    )
                })
        });
        let workspace_selected = workspace_files_selected || selected().any(|item| {
            matches!(
                item.capability.id.as_ref(),
                nomifun_agent_domain_wave2::WORKSPACE_VCS_MODULE_ID
                    | nomifun_agent_domain_wave2::WORKSPACE_ARTIFACTS_MODULE_ID
            ) && item.action_allowlist.iter().any(|action_id| {
                constraints_allow_action(
                    constraints,
                    item.capability.id.as_ref(),
                    action_id.as_ref(),
                )
            })
        });
        if workspace_selected {
            let resources = binding
                .typed_resource_bindings
                .iter()
                .filter(|binding| {
                    binding.resource_kind.as_ref() == nomifun_file::WORKSPACE_RESOURCE_KIND
                })
                .collect::<Vec<_>>();
            let [authority] = resources.as_slice() else {
                return Err(failure(
                    "workspace tools require one server-resolved workspace grant",
                ));
            };
            let resource = super::nomi_core_wave2::session_workspace_binding(
                &workspace,
                principal,
                &session_id,
                authority,
            )?;
            binding.typed_resource_bindings =
                super::nomi_core_wave2::with_session_workspace_binding(
                    binding.typed_resource_bindings,
                    resource,
                );
        }
        let process_selected = selected().any(|item| {
            item.capability.id.as_ref()
                == nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID
                && item.action_allowlist.iter().any(|action_id| {
                    constraints_allow_action(
                        constraints,
                        item.capability.id.as_ref(),
                        action_id.as_ref(),
                    )
                })
        });
        let git_root = (workspace_selected || process_selected)
            .then(|| {
                super::nomi_core_wave2::canonical_workspace_root(std::path::Path::new(&workspace))
            })
            .transpose()?;
        let workspace_watch_context = if workspace_files_selected && !constraints.restricted() {
            super::nomi_core_wave2::NomiWorkspaceWatchContext::start_if_present(&workspace)?
                .map(|watch| watch as Arc<dyn ContextContributor>)
        } else { None };
        let supports_route_feature = |feature| snapshot
            .content
            .chat_route_identity
            .as_ref()
            .and_then(|route| {
                session
                    .revision()
                    .payload
                    .chat_route_records
                    .get(&route.model_task)
            })
            .is_some_and(|record| {
                std::iter::once(&record.primary)
                    .chain(record.failovers.iter())
                    .any(|candidate| candidate.features.contains(
                        &feature,
                    ))
            });
        let route_image_input = supports_route_feature(nomifun_agent_contracts::ChatRouteFeature::ImageInput);
        if process_selected {
            let indices = binding
                .typed_resource_bindings
                .iter()
                .enumerate()
                .filter(|(_, binding)| binding.resource_kind.as_ref() == "process_session")
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let [index] = indices.as_slice() else {
                return Err(failure(
                    "process tools require one server-resolved process grant",
                ));
            };
            binding.typed_resource_bindings[*index] =
                super::nomi_core_wave2::session_process_binding(
                    &workspace,
                    principal,
                    &session_id,
                    &binding.typed_resource_bindings[*index],
                )?;
        }
        let compiled = Arc::new(super::nomi_core_session::compile_nomi_plugin_snapshot(
            &assembly.kernel,
            &assembly.environment,
            binding,
            session.revision().clone(),
            snapshot.clone(),
            principal,
        )?);
        let registry = assembly.kernel.snapshot().map_err(failure)?;
        super::nomi_core_mcp_catalog::validate_resources(&compiled, &registry, principal)?;
        Ok(Self {
            #[cfg(feature = "computer-use")]
            computer: assembly.computer.clone(),
            #[cfg(feature = "computer-use")]
            computer_attempt: Arc::new(super::computer_host::ComputerAttemptSurface::new(session_id.clone(), compiled.snapshot_ref().clone())),
            resource_tasks: nomifun_ai_agent::engine_sdk::EngineTaskGroup::new(32)?,
            knowledge_writebacks: knowledge_writeback::KnowledgeWritebacks::new(session.agent_binding().binding_version)?,
            resource_settlements: Default::default(),
            source,
            session_id,
            principal: principal.clone(),
            binding: session.engine_binding().clone(),
            constraints,
            workspace,
            process_selected,
            route_image_input,
            creation_turn_root: Arc::new(Mutex::new(None)),
            active: Arc::new(
                SessionCapabilityState::from_committed(
                    &compiled,
                    session.active_set_generation(),
                    session
                        .active_capability_ids()
                        .iter()
                        .cloned()
                        .map(nomifun_agent_contracts::CapabilityId::from),
                )
                .map_err(failure)?,
            ),
            compiled,
            kernel: assembly.kernel.clone(),
            wave2: assembly.wave2.clone(),
            context_admission: assembly.context_admission.clone(),
            knowledge: assembly.knowledge.clone(),
            companion: assembly.companion.clone(),
            #[cfg(feature = "browser-use")]
            browser: assembly.browser.clone(),
            git_root,
            hosted_effects: assembly.hosted_effects.clone(),
            initial_capability_context: tokio::sync::OnceCell::new(),
            workspace_watch_context,
            robot: assembly.robot.clone(),
            robot_tools: tokio::sync::OnceCell::new(),
            state: Mutex::new(State::default()),
        })
    }

    pub(super) fn matches(&self, session: &AdmittedEngineSession) -> bool {
        self.session_id.as_ref() == session.session().conversation_id
            && self.principal == *session.principal()
            && self.binding == *session.engine_binding()
            && ExecutionConstraints::from_extra(&session.session().extra).ok()
                == Some(self.constraints)
            && self.compiled.snapshot_ref() == &session.snapshot().snapshot_ref
            && self.workspace == session.workspace()
    }
    pub(super) fn teardown_proven(&self) -> Result<bool, AppError> {
        let state = self.state.lock().map_err(|_| failure("resource state poisoned"))?;
        Ok(state.kernel_release_started
            && matches!(state.release.as_ref().and_then(|done| done.peek()), Some(Ok(()))))
    }
    pub fn compiled(&self) -> &Arc<CompiledSnapshot> {
        &self.compiled
    }
    pub fn active_state(&self) -> &Arc<SessionCapabilityState> {
        &self.active
    }
    pub(super) fn workspace_root_identity(&self) -> Option<&std::path::Path> {
        self.git_root.as_deref()
    }
    pub(super) fn capture_workspace_host_retirement(&self)
        ->Result<Option<super::nomi_core_wave2::WorkspaceHostRetirementTicket>,AppError> {
        self.git_root.as_deref().map(|root|self.wave2.capture_workspace_host_retirement(root)).transpose().map(Option::flatten)
    }
    pub fn execution_constraints(&self) -> ExecutionConstraints {
        self.constraints
    }

    /// Frozen Session-start Context contributions assembled through the same
    /// Kernel authority as tools. Engines consume this canonical projection;
    /// they never hard-code Knowledge, Companion, or Plugin context sources.
    pub async fn initial_capability_context(&self) -> Result<Option<String>, AppError> {
        Ok(self.capability_context().await?.initial.clone())
    }

    pub(super) fn model_attempt_projection(&self) -> Option<Arc<dyn nomifun_chat_model_broker::ChatAttemptProjection>> {
        #[cfg(feature = "computer-use")]
        { self.computer_attempt.has_tools().then(|| self.computer_attempt.clone() as Arc<dyn nomifun_chat_model_broker::ChatAttemptProjection>) }
        #[cfg(not(feature = "computer-use"))]
        { None }
    }

    async fn capability_context(&self) -> Result<&CapabilityContext, AppError> {
        self
            .initial_capability_context
            .get_or_try_init(|| async {
                if self.constraints.restricted() {
                    return Ok(CapabilityContext::default());
                }
                let active = self.active.snapshot().map_err(failure)?;
                let registry = self.registry_snapshot()?;
                let (contributions, turn_context_ids) =
                    nomifun_ai_agent::assemble_initial_capability_context(
                        &self.kernel,
                        &self.compiled,
                        &active,
                        registry.as_ref(),
                        &self.principal,
                        &self.session_id,
                        &ScopeKey::from(format!("session:{}", self.session_id.as_ref())),
                        Some(self.context_admission.as_ref()),
                    )
                    .await
                    .map_err(failure)?;
                let initial = nomifun_ai_agent::render_initial_capability_context_section(&contributions)
                    .map_err(failure)?;
                let before_turn = (!turn_context_ids.is_empty()).then(|| {
                    Arc::new(nomifun_ai_agent::NomiTurnContextContributor::new(
                        self.kernel.clone(), self.compiled.clone(), self.active.clone(),
                        self.principal.clone(), self.session_id.clone(),
                        ScopeKey::from(format!("session:{}", self.session_id.as_ref())),
                        turn_context_ids,
                    )) as Arc<dyn ContextContributor>
                });
                Ok(CapabilityContext { initial, before_turn })
            })
            .await
    }

    fn knowledge_binding(
        &self,
    ) -> Result<Option<nomifun_knowledge::KnowledgeBinding>, AppError> {
        let resources = self
            .compiled
            .target_resource_bindings
            .iter()
            .filter(|binding| {
                binding.resource_kind.as_ref()
                    == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
            })
            .collect::<Vec<_>>();
        let Some(first) = resources.first() else {
            return Ok(None);
        };
        let enabled = nomifun_agent_domain_wave1::agent_knowledge_enabled(first)
            .map_err(failure)?;
        let (writeback, eagerness) =
            nomifun_agent_domain_wave1::agent_knowledge_writeback_policy(first)
                .map_err(failure)?;
        if !enabled {
            return Ok(None);
        }
        let mut kb_ids = Vec::with_capacity(resources.len());
        for resource in resources {
            if !nomifun_agent_domain_wave1::agent_knowledge_enabled(resource)
                .map_err(failure)?
                || nomifun_agent_domain_wave1::agent_knowledge_writeback_policy(resource)
                    .map_err(failure)?
                    != (writeback, eagerness)
            {
                return Err(failure(
                    "Knowledge resources carry inconsistent live policies",
                ));
            }
            if writeback && !resource.operations.contains("write") {
                return Err(failure(
                    "Knowledge write-back exceeds the current resource operation grant",
                ));
            }
            kb_ids.push(
                nomifun_common::KnowledgeBaseId::parse(
                    resource.resource_id.as_ref().to_owned(),
                )
                .map_err(failure)?,
            );
        }
        Ok(Some(nomifun_knowledge::KnowledgeBinding {
            enabled: true,
            writeback,
            writeback_eagerness: eagerness.to_owned(),
            channel_write_enabled: false,
            kb_ids,
        }))
    }

    pub(super) async fn capture_knowledge_writeback_configuration(
        &self, user_text: &str, model: Option<&nomifun_common::ProviderWithModel>,
    ) -> Result<Option<Arc<nomifun_model_invoke::ResolvedTaskConfig>>, nomifun_agent_contracts::KnowledgeWritebackDiagnostic> {
        let diagnostic = || {
            let mut value = nomifun_agent_contracts::KnowledgeWritebackDiagnostic::new(
                nomifun_agent_contracts::KnowledgeWritebackStage::Resolve);
            value.code = Some("USER_LLM_PROVIDER_CONFIG_ERROR".into());
            value.detail = Some("Knowledge extraction could not prepare the original selected model configuration".into());
            value
        };
        let Some(binding) = self.knowledge_binding().map_err(|_| diagnostic())? else { return Ok(None); };
        if !binding.writeback || (binding.writeback_eagerness == "manual"
            && !explicitly_requests_knowledge_writeback(user_text)) { return Ok(None); }
        self.knowledge.capture_writeback_configuration(model).await.map_err(|_| diagnostic())
    }

    /// Subtractive ceiling of this Session, not another Agent capability grant.
    pub fn allows_capability(&self, id: &nomifun_agent_contracts::CapabilityId) -> bool {
        let selected = self.compiled
            .content()
            .enabled_capabilities
            .iter()
            .find(|item| &item.capability.id == id);
        let Some(selected) = selected else {
            return false;
        };
        let policy_allows = if selected.action_allowlist.is_empty() {
            !self.constraints.restricted()
        } else {
            selected.action_allowlist.iter().any(|action_id| {
                constraints_allow_action(self.constraints, id.as_ref(), action_id.as_ref())
            })
        };
        policy_allows
            && (!self.constraints.restricted()
                || (selected.contribution_lock.source_kind
                    == nomifun_agent_contracts::ContributionSourceKind::PlatformBuiltin
                    && selected.resolved_source.source_kind
                        == nomifun_agent_contracts::PluginSourceKind::Bundled))
    }

    pub fn registry_snapshot(&self) -> Result<Arc<MaterializedRegistry>, AppError> {
        self.kernel.snapshot().map_err(failure)
    }

    pub async fn capability_context_for_turn(
        &self,
        turn: &nomifun_ai_agent::context_contributor::TurnContext,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<Option<String>, AppError> {
        let mut contributions = Vec::new();
        if let Some(contributor) = &self.workspace_watch_context {
            let context = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(failure("workspace notification context was canceled")),
                result = contributor.pre_turn_context_for_turn_result(turn) => result.map_err(failure)?,
            };
            contributions.extend(context);
        }
        if let Some(contributor) = &self.capability_context().await?.before_turn {
            // BeforeTurn capability context uses the same frozen Kernel policy
            // as tools, but must be refreshed for this accepted user input.
            let context = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(failure("capability context was canceled")),
                result = contributor.pre_turn_context_for_turn_result(turn) => result.map_err(failure)?,
            };
            contributions.extend(context);
        }
        Ok((!contributions.is_empty()).then(|| contributions.join("\n\n")))
    }

    pub fn retain_active_tools(
        &self,
        full: &EngineToolPlan,
        active: &std::collections::BTreeSet<nomifun_agent_contracts::CapabilityId>,
    ) -> Result<EngineToolPlan, AppError> {
        EngineToolPlan::new(full.model_definitions().into_iter().filter_map(|definition| {
            let binding = full.binding(&definition.name)?;
            active.contains(&binding.capability_id).then(|| binding.clone())
        })).map_err(failure)
    }

    pub fn tool_discovery_port(
        &self,
    ) -> Result<Option<Arc<dyn nomifun_agent_runtime::AgentToolDiscoveryPort>>, AppError> {
        if self.constraints.restricted() {
            return Ok(None);
        }
        super::engine_tool_discovery::port(
            self.kernel.clone(),
            self.compiled.clone(),
            self.active.clone(),
            self.principal.clone(),
            self.session_id.clone(),
        )
    }

    /// Required external-state context, independent of transcript rollback.
    /// Engines decide presentation/compaction but must not reinterpret a reply
    /// as physical quiescence or repeat missing transcript effects.
    pub async fn hosted_effect_context(&self) -> Result<Option<String>, AppError> {
        if let Some(root) = &self.git_root {
            self.wave2.ensure_workspace_git_evidence(root).await?;
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.hosted_effects
                .context(&self.principal.principal_id, self.session_id.as_ref()),
        )
        .await
        .map_err(|_| failure("hosted effect context timed out"))?
    }

    pub async fn ensure_hosted_effects_settled(&self) -> Result<(), AppError> {
        if self.resource_settlements_unconfirmed()? {
            return Err(failure("resource settlement journal is unproven"));
        }
        if let Some(root) = &self.git_root {
            self.wave2.ensure_workspace_git_evidence(root).await?;
        }
        self.hosted_effects
            .ensure_settled(&self.principal.principal_id, self.session_id.as_ref())
            .await
    }

    /// Teardown observes physical resource responsibility, not permission to
    /// dispatch or resume. A joined MCP owner may have closed its resources
    /// while its canonical external business result remains Unknown.
    pub(super) async fn ensure_effect_resources_closed(&self) -> Result<(), AppError> {
        if self.resource_settlements_unconfirmed()? {
            return Err(failure("resource settlement journal is unproven"));
        }
        if let Some(root) = &self.git_root {
            self.wave2.ensure_workspace_git_evidence(root).await?;
        }
        self.wave2.ensure_mcp_shutdown_resources_closed(
            &self.principal.principal_id, self.session_id.as_ref(),
        ).await?;
        self.hosted_effects.ensure_shutdown_resources_closed(
            &self.principal.principal_id, self.session_id.as_ref(),
        ).await
    }

    /// Host-frozen Robot device schemas. Catalog reads never activate link or
    /// audio; the retained tool owner acquires only already-active grants.
    pub fn has_frozen_robot_surface(&self) -> bool {
        // A bound offline device freezes an empty surface too. Recycle it at
        // the next idle Turn boundary so reconnect can obtain a fresh exact
        // connection; an unbound ordinary chat keeps its reusable Runtime.
        self.robot_tools.get().is_some()
            && self.allows_capability(&super::nomi_core_robot::module_capability_id())
            && self.compiled.target_resource_bindings.iter()
                .any(|binding| binding.resource_kind.as_ref() == "robot")
    }

    pub async fn robot_tool_plan(&self) -> Result<EngineToolPlan, AppError> {
        if self.constraints.restricted() {
            return Ok(EngineToolPlan::default());
        }
        let tools = self
            .robot_tools
            .get_or_try_init(|| async {
                let supported = super::engine_robot_tools::supported_ids();
                let selected = self
                    .compiled
                    .content()
                    .enabled_capabilities
                    .iter()
                    ;
                if !selected
                    .clone()
                    .any(|item| supported.contains(&item.capability.id))
                {
                    return Ok::<_, AppError>(None);
                }
                if !self
                    .compiled
                    .target_resource_bindings
                    .iter()
                    .any(|binding| binding.resource_kind.as_ref() == "robot")
                {
                    // Device authority is attached per accepted turn. A base
                    // Companion Session without a selected device remains
                    // usable and exposes no Robot tools until that owner lease
                    // is present.
                    return Ok::<_, AppError>(None);
                }
                let Some(owner) = self.robot.clone() else {
                    return Ok::<_, AppError>(None);
                };
                let frozen = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    super::engine_robot_tools::FrozenTools::resolve(
                        owner,
                        self.hosted_effects.clone(),
                        self.principal.clone(),
                        self.session_id.clone(),
                        &self.compiled,
                        |exposures| self.compile_tool_plan(exposures),
                    ),
                )
                .await
                .map_err(|_| failure("Robot catalog resolution timed out"))??;
                Ok(frozen.map(Arc::new))
            })
            .await?;
        Ok(tools
            .as_ref()
            .map(|tools| tools.plan.clone())
            .unwrap_or_default())
    }

    /// Whether a facts slot may be needed. The model-boundary reader checks the
    /// active admitted Context and its paired frozen resource grants before any
    /// fact or sibling data is read. ToolSearch activation is separate.
    pub(super) fn has_companion_memory_context(&self) -> bool {
        let id = nomifun_agent_contracts::CapabilityId::from("companion.memory");
        !self.constraints.restricted() && self.allows_capability(&id)
            && self.compiled.policy(&id).is_some_and(|policy| policy.allowed_actions.contains(
                &nomifun_agent_contracts::ActionId::from("companion.memory/recall")))
            && self.compiled.content().contributions().any(|resolved|resolved.capability.id.as_ref()=="companion")
    }

    /// Facts use the existing actual Persona Context contract plus the frozen
    /// memory read ceiling. Ordinary explicitly configured Agents retain this
    /// contract without becoming product Sessions or acquiring sibling history.
    /// A deferred memory Tool need not be activated to read this Context.
    pub(super) async fn frozen_companion_fact_owner(&self,generation:u64)->Result<Option<String>,AppError>{
        if !self.has_companion_memory_context() { return Ok(None); }
        let context_id=nomifun_agent_contracts::CapabilityId::from("companion");
        let memory_id=nomifun_agent_contracts::CapabilityId::from("companion.memory");
        let active = self.active.snapshot().map_err(failure)?;
        if active.generation != generation || active.resolved_snapshot_ref != *self.compiled.snapshot_ref() {
            return Err(failure("Companion memory context generation changed"));
        }
        if !active.active.contains(&context_id) {return Ok(None);}
        let resolved=self.compiled.content().contributions().find(|item|item.capability.id==context_id)
            .ok_or_else(||failure("Companion Context contribution missing"))?;
        if resolved.contribution_lock.source_kind!=nomifun_agent_contracts::ContributionSourceKind::PlatformBuiltin
            || resolved.resolved_source.source_kind!=nomifun_agent_contracts::PluginSourceKind::Bundled
            || !self.context_admission.approved_capability_ids().contains(&context_id) {
            return Ok(None);
        }
        if !self.compiled.capability_resources_bound(&context_id).map_err(failure)?
            || !self.compiled.action_resources_bound(&memory_id,&"companion.memory/recall".into()).map_err(failure)? {
            return Ok(None);
        }
        // Reuse the official composition admission/exact-target checks. This
        // initializer collects BeforeTurn IDs without running an extra model.
        self.capability_context().await?;
        let registry=self.registry_snapshot()?;
        let current=registry.capability(&context_id).ok_or_else(||failure("Companion Context is not materialized"))?;
        resolved.validate().map_err(|error|failure(error.message))?;
        let manifest_digest=nomifun_agent_contracts::digest_payload(&current.manifest).map_err(failure)?;
        if current.manifest.id!=resolved.capability.id || current.manifest.package!=resolved.source_package
            || current.contribution_id!=resolved.contribution_id || current.contribution_lock!=resolved.contribution_lock
            || resolved.resolved_mount_id.as_ref()!=Some(&current.mount_id) || current.source!=resolved.resolved_source
            || current.target_artifact_digest!=resolved.target_artifact_digest || current.schema_digest!=resolved.schema_digest
            || manifest_digest!=resolved.schema_digest {
            return Err(failure("Companion Context materialization differs from frozen Snapshot"));
        }
        if current.manifest.contributions.context_schema_refs.len()!=1
            || current.manifest.contributions.context_phase!=nomifun_agent_contracts::ContextContributionPhase::BeforeTurn
            || !current.manifest.supports_consumer(nomifun_agent_contracts::CapabilityConsumer::Agent) {
            return Err(failure("Companion Context no longer publishes its admitted BeforeTurn contract"));
        }
        paired_companion_fact_owner(&self.principal,&self.compiled).map(Some)
    }

    pub(super) async fn current_companion_memory_context(
        &self,companion_id:&str,generation:u64,max_bytes:usize,
    )->Result<Option<nomifun_companion::CompanionMemoryContext>,AppError>{
        let Some(authorized)=self.frozen_companion_fact_owner(generation).await? else {return Ok(None);};
        if authorized!=companion_id {return Err(failure("Companion memory resource differs from canonical product membership"));}
        let service=companion_owner(&self.companion)?;
        if self.principal.principal_id!=service.authoritative_user_id(){return Err(failure("Companion memory context belongs to another owner"));}
        let context=service.current_memory_context(companion_id,max_bytes).await?;
        if self.active.snapshot().map_err(failure)?.generation!=generation {return Err(failure("Companion memory context generation changed while reading"));}
        Ok(Some(context))
    }

    /// Latest existing Robot vision observation, not a camera capture or a
    /// second model call. Read at model boundaries; expired data is not reused.
    pub async fn robot_vision_context(&self, generation: u64) -> Result<Option<String>, AppError> {
        let id = super::nomi_core_robot::module_capability_id();
        if !self.allows_capability(&id) {
            return Ok(None);
        }
        let active = self.active.snapshot().map_err(failure)?;
        if active.generation != generation
            || active.resolved_snapshot_ref != *self.compiled.snapshot_ref()
        {
            return Err(failure("Robot context generation changed"));
        }
        if !active.active.contains(&id) {
            return Ok(None);
        }
        let turn_identity = || -> Result<String, AppError> {
            let state = self
                .state
                .lock()
                .map_err(|_| failure("resource state poisoned"))?;
            let turn = state
                .turn
                .as_ref()
                .ok_or_else(|| failure("Robot context requires an open turn"))?;
            if state.release.is_some() || turn.cleanup.is_some() {
                return Err(failure("Robot context resources closed"));
            }
            Ok(turn.operation.clone())
        };
        let operation = turn_identity()?;
        let selected = self
            .compiled
            .content()
            .enabled_capabilities
            .iter()

            .find(|item| item.capability.id == id)
            .ok_or_else(|| failure("Robot Module not selected"))?;
        let policy = self
            .compiled
            .policy(&id)
            .ok_or_else(|| failure("Robot Module policy missing"))?;
        if !policy
            .allowed_actions
            .contains(&nomifun_agent_contracts::ActionId::from(
                nomifun_robot::capability::ROBOT_VISION_ACTION_ID,
            ))
        {
            return Ok(None);
        }
        if !self
            .compiled
            .target_resource_bindings
            .iter()
            .any(|binding| binding.resource_kind.as_ref() == "robot")
        {
            // The Companion preset grants Robot capability so a device can be
            // attached later, but chat itself does not require one. Absence is
            // therefore no live context; a present-but-invalid binding still
            // fails closed in the exact validation below.
            return Ok(None);
        }
        let resources = self
            .compiled
            .target_resource_bindings
            .iter()
            .filter(|binding| {
                binding.resource_kind.as_ref() == "robot"
                    && policy.resource_binding_ids.contains(&binding.binding_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        if selected.contribution_lock.source_kind
            != nomifun_agent_contracts::ContributionSourceKind::PlatformBuiltin
            || resources.len() != 1
            || !resources[0].operations.contains("vision")
        {
            return Err(failure(
                "Robot vision requires the exact bundled Module and resource grant",
            ));
        }
        let owner = self
            .robot
            .as_ref()
            .ok_or_else(|| failure("Robot owner unavailable"))?;
        let context = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            owner.vision_context(
                &self.principal.principal_id,
                &resources[0],
                &policy.allowed_actions,
            ),
        )
        .await
        .map_err(|_| failure("Robot context read timed out"))?
        .map_err(failure)?;
        if turn_identity()? != operation
            || self.active.snapshot().map_err(failure)?.generation != generation
        {
            return Err(failure("Robot context authority changed while reading"));
        }
        context
            .map(|context| {
                let encoded = serde_json::to_string(&context.0).map_err(failure)?;
                if encoded.len() > 16 * 1024 {
                    return Err(failure("Robot observation exceeds context budget"));
                }
                Ok(encoded)
            })
            .transpose()
    }

    /// Compile an explicit surface within the immutable enabled capability set.
    pub fn compile_tool_plan(
        &self,
        exposures: impl IntoIterator<Item = EngineToolExposure>,
    ) -> Result<EngineToolPlan, AppError> {
        let preview = self.active.snapshot().map_err(failure)?;
        let registry = self.registry_snapshot()?;
        let exposures = exposures.into_iter().take(257);
        #[cfg(feature = "computer-use")]
        let exposures = exposures.filter_map(|exposure| super::computer_host::project_exposure(
            exposure, nomifun_computer::surface::RouteTier::Full,
        ));
        #[cfg(not(feature = "computer-use"))]
        let exposures = exposures.filter(|exposure| exposure.capability_id.as_ref() != "computer");
        let exposures = exposures.collect::<Vec<_>>();
        if exposures.len() > 256 {
            return Err(failure("tool surface exceeds the platform bound"));
        }
        let plan = nomifun_engine_core::compile_engine_tool_plan(
            &self.compiled,
            &preview,
            &registry,
            exposures,
        )
        .map_err(failure)?;
        // Compile first: a forged alias/schema cannot be laundered by filtering.
        EngineToolPlan::new(plan.model_definitions().iter().filter_map(|definition| {
            let binding = plan.binding(&definition.name)?;
            (self.allows_capability(&binding.capability_id)
                && constraints_allow_action(
                    self.constraints,
                    binding.capability_id.as_ref(),
                    binding.action_id.as_ref(),
                ))
            .then(|| binding.clone())
        }))
        .map_err(failure)
    }

    /// Install once before opening a turn; resource cleanup retains this exact
    /// tool host even if the driver drops its own reference. No raw invoker API.
    pub fn install_tools(
        &self,
        plan: EngineToolPlan,
    ) -> Result<Arc<EngineToolHost>, AppError> {
        if plan.len() > 256 {
            return Err(failure("tool surface exceeds the platform bound"));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| failure("resource state poisoned"))?;
        if state.tools.is_some() || state.turn.is_some() || state.release.is_some() {
            return Err(failure(
                "tool surface already installed or resources closed",
            ));
        }
        let kernel_plan = plan.clone();
        let exposures = kernel_plan.model_definitions().into_iter().map(|definition| {
            let binding = kernel_plan
                .binding(&definition.name)
                .expect("kernel plan definitions have bindings");
            EngineToolExposure {
                definition,
                capability_id: binding.capability_id.clone(),
                action_id: binding.action_id.clone(),
            }
        });
        let canonical = self
            .compile_tool_plan(exposures)?;
        if plan != canonical {
            return Err(failure(
                "tool plan differs from the canonical Session mapping",
            ));
        }
        #[cfg(feature = "computer-use")]
        self.computer_attempt.bind_plan(&plan).map_err(failure)?;
        for definition in plan.model_definitions() {
            let binding = plan
                .binding(&definition.name)
                .expect("plan definition has binding");
            if super::engine_robot_tools::supported_ids().contains(&binding.capability_id)
                && self
                    .robot_tools
                    .get()
                    .and_then(Option::as_ref)
                    .and_then(|frozen| frozen_tool_binding(&frozen.plan, &definition.name, binding))
                    .is_none()
            {
                return Err(failure(
                    "Robot tools require the exact host-frozen device surface",
                ));
            }
        }
        let invoker = KernelEngineToolInvoker::for_session(
            self.kernel.clone(),
            self.compiled.clone(),
            self.active.clone(),
            self.principal.clone(),
            self.session_id.clone(),
            kernel_plan,
        )
        .map_err(failure)?;
        let invoker: Arc<dyn nomifun_engine_core::EngineToolInvoker> = Arc::new(
            super::engine_creation_tools::ConversationCreationTools {
                inner: Arc::new(invoker),
                session_id: self.session_id.clone(),
                turn_root: self.creation_turn_root.clone(),
            },
        );
        let invoker = super::engine_workspace_media::WorkspaceMediaTools {
            inner: invoker,
            snapshot: self.compiled.clone(),
            active: self.active.clone(),
            route_image_input: self.route_image_input,
        };
        let invoker: Arc<dyn nomifun_engine_core::EngineToolInvoker> = Arc::new(invoker);
        #[cfg(feature = "computer-use")]
        let invoker: Arc<dyn nomifun_engine_core::EngineToolInvoker> = Arc::new(super::computer_host::ComputerModelTools {
            inner: invoker, snapshot: self.compiled.clone(), active: self.active.clone(),
            attempt: self.computer_attempt.clone(),
        });
        let invoker = super::engine_computer_media::BrowserMediaTools {
            inner: invoker,
            snapshot: self.compiled.clone(),
            active: self.active.clone(),
            route_image_input: self.route_image_input,
        };
        let invoker: Arc<dyn nomifun_engine_core::EngineToolInvoker> = Arc::new(invoker);
        let invoker = match self.robot_tools.get().and_then(Option::as_ref) {
            Some(frozen) => Arc::new(super::engine_robot_tools::SessionTools {
                frozen: frozen.clone(),
                inner: invoker,
                snapshot: self.compiled.clone(),
                active: self.active.clone(),
            }) as Arc<dyn nomifun_engine_core::EngineToolInvoker>,
            None => invoker,
        };
        // Check the exact frozen mapping before host adapters; none may
        // dispatch using caller-supplied capability or effect labels.
        let invoker = ConstrainedTools {
            inner: invoker,
            plan,
            constraints: self.constraints,
            wave2: self.wave2.clone(),
            git_root: self.git_root.clone(),
        };
        let tools = Arc::new(EngineToolHost::new(Arc::new(invoker)));
        state.tools = Some(tools.clone());
        Ok(tools)
    }

    pub async fn open_turn(
        &self,
        receipt: &EngineTurnReceipt,
        journal: EngineTurnJournal,
    ) -> Result<(), AppError> {
        if !receipt.belongs_to(&self.source)
            || receipt.session().execution_constraints()? != self.constraints
            || self.session_id.as_ref() != receipt.session().session().conversation_id
            || self.principal != *receipt.session().principal()
            || self.binding != *receipt.session().engine_binding()
            || self.compiled.snapshot_ref() != &receipt.session().snapshot().snapshot_ref
        {
            return Err(failure("turn differs from the compiled Session authority"));
        }
        journal.validate_receipt(receipt)?;
        #[cfg(feature = "browser-use")]
        self.browser.ensure_previous_turn_finished(&self.principal.principal_id, self.session_id.as_ref())?;

        if let Some(root) = &self.git_root {
            self.wave2.ensure_workspace_git_ready(root)?;
        }
        {
        let mut state = self
            .state
            .lock()
            .map_err(|_| failure("resource state poisoned"))?;
        if state.release.is_some()
            || state.turn.as_ref().is_some_and(|turn| {
                (turn.operation == receipt.operation_id() && turn.generation == journal.generation())
                    || !matches!(
                        turn.cleanup.as_ref().and_then(|done| done.peek()),
                        Some(Ok(()))
                    )
            })
        {
            return Err(failure("previous resource turn is not proven closed"));
        }
        let tools = state
            .tools
            .as_ref()
            .ok_or_else(|| failure("install the exact tool surface before opening a turn"))?
            .clone();
        tools.bind_turn(receipt.operation_id().into(), journal.clone())?;
        // Retain the turn before opening resources so partial opening is always
        // covered by cleanup, including errors before engine TurnStarted.
        state.turn = Some(Turn {
            operation: receipt.operation_id().into(),
            generation: journal.generation(),
            root: receipt.root_message_id().into(),
            cleanup: None,
            resource_operations: Default::default(),
            file_capture: None,
        });
        }
        *self
            .creation_turn_root
            .lock()
            .map_err(|_| failure("creation turn identity lock poisoned"))? =
            Some(receipt.root_message_id().to_owned());
        #[cfg(feature = "browser-use")]
        self.browser.open_turn(
            &self.principal,
            &self.session_id,
            &OperationId::from(receipt.operation_id()),
            &self.workspace,
            &self.compiled,
        ).await?;
        if self.process_selected {
            self.wave2.open_runtime_turn(
                &self.principal.principal_id,
                self.session_id.as_ref(),
                receipt.operation_id(),
                &self.workspace,
                journal.clone(),
                super::engine_process_host::ProcessAuthority {
                    snapshot: self.compiled.snapshot_ref().clone(),
                    actions: self.compiled.policy(&"workspace.process".into()).ok_or_else(|| failure("process policy missing"))?.allowed_actions.clone(),
                    resources: self.compiled.target_resource_bindings.iter().filter(|binding|
                        self.compiled.policy(&"workspace.process".into()).is_some_and(|policy| policy.resource_binding_ids.contains(&binding.binding_id)))
                        .cloned().collect(),
                },
            )?;
        }
        let capture_generation = journal.generation();
        let file_capture = super::engine_turn_files::TurnFileCapture::begin(&self.workspace, journal).await;
        let mut state = self.state.lock().map_err(|_| failure("resource state poisoned"))?;
        if let Some(turn) = state.turn.as_mut().filter(|turn| turn.root == receipt.root_message_id()
            && turn.operation == receipt.operation_id() && turn.generation == capture_generation && turn.cleanup.is_none()) {
            turn.file_capture = Some(file_capture);
        }
        Ok(())
    }

    /// Session teardown retains the join witness even if an input caller drops.
    pub(super) fn own_input_operation<T:Send+'static>(&self,operation:impl std::future::Future<Output=T>+Send+'static)
        ->Result<nomifun_ai_agent::engine_sdk::EngineOwnedTask<T>,AppError> {
        self.resource_tasks.spawn(operation)
    }

    /// Model parking retains the original process owner and deadlines.
    pub(super) async fn wait_processes(&self, operation: &str,
        processes: &[nomifun_agent_runtime::AgentWaitProcess], wake_policy: nomifun_agent_runtime::AgentWaitWakePolicy,
        cancellation: tokio_util::sync::CancellationToken)
        -> Result<Vec<nomifun_agent_runtime::AgentOwnedProcessObservation>, AppError> {
        {
            let state = self.state.lock().map_err(|_| failure("resource state poisoned"))?;
            if !state.turn.as_ref().is_some_and(|turn| turn.operation == operation && turn.cleanup.is_none()) {
                return Err(failure("wait differs from the original live resource Turn"));
            }
        }
        self.wave2.wait_runtime_processes(&self.principal.principal_id, self.session_id.as_ref(), operation, processes, wake_policy, cancellation).await
    }

    pub(super) async fn process_retention(&self) ->Result<Option<nomifun_agent_contracts::NativeSessionProcessRetention>,AppError> {
        self.wave2.runtime_process_retention(&self.principal.principal_id,self.session_id.as_ref()).await
    }

    /// A canonical parked machine/intervention boundary releases Browser
    /// ownership while preserving Session process handles. A later Browser
    /// invocation uses a fresh run and must obtain new observations.
    pub(super) async fn yield_browser_for_parked_wait(&self, operation: &str,
        cancellation: &tokio_util::sync::CancellationToken) -> Result<(), AppError> {
        #[cfg(feature = "browser-use")]
        {
            {
                let state = self.state.lock().map_err(|_| failure("resource state poisoned"))?;
                if !state.turn.as_ref().is_some_and(|turn| turn.operation == operation && turn.cleanup.is_none()) {
                    return Err(failure("Browser yield differs from the live resource Turn"));
                }
            }
            self.browser.yield_to_user(&self.principal.principal_id, self.session_id.as_ref(), operation).await?;
            if !cancellation.is_cancelled() {
                self.browser.open_turn(&self.principal, &self.session_id, &operation.into(), &self.workspace, &self.compiled).await?;
            }
        }
        #[cfg(not(feature = "browser-use"))]
        let _ = (operation, cancellation);
        Ok(())
    }

    /// The journal owner calls this after its exact root's durable terminal.
    pub async fn finish_browser_turn(&self, root_message_id: &str) -> Result<(), AppError> {
        #[cfg(feature = "browser-use")]
        {
            let operation = {
                let state = self.state.lock().map_err(|_| failure("resource state poisoned"))?;
                let Some(turn) = &state.turn else { return Ok(()); };
                if turn.root != root_message_id {
                    // A successor cancelled before preparation owns no Browser
                    // run. An old cached resource root may remain, but only an
                    // already-finished Browser owner permits this no-op.
                    self.browser.ensure_previous_turn_finished(&self.principal.principal_id, self.session_id.as_ref())?;
                    return Ok(());
                }
                turn.operation.clone()
            };
            self.browser.finish_turn(&self.principal.principal_id, self.session_id.as_ref(), &operation).await?;
        }
        #[cfg(not(feature = "browser-use"))]
        let _ = root_message_id;
        Ok(())
    }

    pub(super) fn execution_window_near_limit(&self) -> Result<bool, AppError> {
        let state = self.state.lock().map_err(|_| failure("resource state poisoned"))?;
        Ok(state.turn.as_ref().is_some_and(|turn| turn.resource_operations.len() >= 56)
            || state.tools.as_ref().map(|tools| tools.execution_window_near_limit()).transpose()?.unwrap_or(false))
    }

    async fn settle_owned(&self, tools: &EngineToolHost, release_session: bool) -> Result<(), AppError> {
        let admission = tools.close_turn();
        #[cfg(feature = "browser-use")]
        let browser_turn = self
            .state
            .lock()
            .map(|state| state.turn.as_ref().map(|turn| turn.operation.clone()))
            .map_err(|_| failure("resource state poisoned"));
        #[cfg(feature = "browser-use")]
        let browser_cancel = match &browser_turn {
            Ok(Some(turn_id)) => self.browser.cancel_turn(
                &self.principal.principal_id,
                self.session_id.as_ref(),
                turn_id,
            ),
            Ok(None) => Ok(()),
            Err(error) => Err(failure(error)),
        };
        // Catch each owner separately: the outer retained task alone would
        // preserve failure, but an unwind would skip all subsequent owners.
        let processes = guard_effect_settlement(|| async {
            if release_session {
                self.wave2.cleanup_runtime_session(&self.principal.principal_id, self.session_id.as_ref()).await
            } else {
                let operation = self.state.lock().map_err(|_| failure("resource state poisoned"))?
                    .turn.as_ref().map(|turn| turn.operation.clone());
                match operation { Some(operation) => self.wave2.finish_runtime_process_turn(
                    &self.principal.principal_id, self.session_id.as_ref(), &operation), None => Ok(()) }
            }
        })
        .await;
        let tasks = guard_effect_settlement(|| tools.join()).await;
        #[cfg(feature = "computer-use")]
        if tasks.is_ok() { self.computer_attempt.clear(); }
        let retained_processes = guard_effect_settlement(|| async {
            if release_session { Ok(()) } else {
                self.wave2.ensure_runtime_processes_retained(&self.principal.principal_id, self.session_id.as_ref()).await
            }
        }).await;
        #[cfg(feature = "browser-use")]
        let browser = guard_effect_settlement(|| async {
            match browser_turn {
                Ok(Some(turn_id)) => {
                    self.browser
                        .settle_turn(
                            &self.principal.principal_id,
                            self.session_id.as_ref(),
                            &turn_id,
                        )
                        .await
                }
                Ok(None) => Ok(()),
                Err(error) => Err(error),
            }
        })
        .await;
        let resource_tasks = guard_effect_settlement(|| self.resource_tasks.join()).await;
        // Even an abnormal sibling task must not discard another operation's
        // already-returned result. Retry only these original journal records.
        let resource_settlements = guard_effect_settlement(|| self.retry_resource_settlements()).await;
        let git = guard_effect_settlement(|| async {
            match &self.git_root {
                Some(root) => self.wave2.settle_workspace_git(root).await,
                None => Ok(()),
            }
        })
        .await;
        let mcp = guard_effect_settlement(|| {
            self.wave2
                .ensure_mcp_shutdown_resources_closed(&self.principal.principal_id, self.session_id.as_ref())
        })
        .await;
        // SDK shutdown first retries this exact Turn cleanup. Both soft and
        // hard teardown need resource proof; strict admission/context/resume
        // gates continue to reject the unchanged Unknown business Effect.
        let hosted = guard_effect_settlement(|| async {
            if let Some(root) = &self.git_root {
                self.wave2.ensure_workspace_git_evidence(root).await?;
            }
            self.hosted_effects.ensure_shutdown_resources_closed(
                &self.principal.principal_id, self.session_id.as_ref(),
            ).await
        }).await;
        for (stage,failed) in [("admission",admission.is_err()),("process_owner",processes.is_err()),
            ("tool_tasks",tasks.is_err()),("resource_tasks",resource_tasks.is_err()),
            ("resource_settlements",resource_settlements.is_err()),("git_owner",git.is_err()),
            ("retained_processes",retained_processes.is_err()),
            ("mcp_owner",mcp.is_err()),("hosted_receipts",hosted.is_err())] {
            if failed {tracing::warn!(stage,session_id=self.session_id.as_ref(),"owned resource cleanup remains unproven");}
        }
        #[cfg(feature = "browser-use")]
        for (stage,failed) in [("browser_cancel",browser_cancel.is_err()),("browser_owner",browser.is_err())] {
            if failed {tracing::warn!(stage,session_id=self.session_id.as_ref(),"owned resource cleanup remains unproven");}
        }
        admission?;
        #[cfg(feature = "browser-use")]
        browser_cancel?;
        processes?;
        tasks?;
        retained_processes?;
        #[cfg(feature = "browser-use")]
        browser?;
        resource_tasks?;
        resource_settlements?;
        git?;
        mcp?;
        hosted?;
        Ok(())
    }

    /// The whole resource cleanup is owned, not only Kernel release. Dropping
    /// a waiter leaves process cleanup and settlement running with one witness.
    pub async fn cleanup_turn(self: &Arc<Self>, root_message_id: &str) -> Result<(), AppError> {
        let done = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| failure("resource state poisoned"))?;
            let Some(turn) = state.turn.as_ref() else {
                return Ok(());
            };
            if turn.root != root_message_id {
                return Err(failure("cleanup targets a different accepted root"));
            }
            // A finished failed waiter is not a successful cleanup witness.
            // Keep the same root/tools fenced and retry only owned settlement;
            // pending/successful flights stay single-flight and cached.
            if retry_settlement(&turn.cleanup,state.release.is_some()) {
                state.turn.as_mut().expect("same turn retained under lock").cleanup=None;
            }
            let turn=state.turn.as_ref().expect("same turn retained under lock");
            if let Some(done) = &turn.cleanup {
                done.clone()
            } else {
                let tools = state
                    .tools
                    .as_ref()
                    .ok_or_else(|| failure("missing owned tool host"))?
                    .clone();
                tools.close_turn()?;
                let owner = self.clone();
                let executor = tokio::runtime::Handle::try_current().map_err(failure)?;
                let task = executor.spawn(async move {
                    // A completed or parked computation releases this Turn's
                    // admission and joins its calls. The independently owned
                    // Session processes remain under their original handles;
                    // only explicit Session/resource teardown may stop them.
                    owner.settle_owned(&tools, false).await.map_err(|error| error.to_string())?;
                    let file_capture = {
                        let mut state = owner.state.lock().map_err(|_| "resource state poisoned".to_owned())?;
                        state.turn.as_mut().and_then(|turn| turn.file_capture.take())
                    };
                    if let Some(capture) = file_capture { capture.finish().await; }
                    Ok(())
                });
                let done = async move { task.await.map_err(|error| error.to_string())? }
                    .boxed()
                    .shared();
                state
                    .turn
                    .as_mut()
                    .expect("turn retained under lock")
                    .cleanup = Some(done.clone());
                done
            }
        };
        done.await.map_err(failure)?;
        let mut creation_root = self
            .creation_turn_root
            .lock()
            .map_err(|_| failure("creation turn identity lock poisoned"))?;
        if creation_root.as_deref() == Some(root_message_id) {
            *creation_root = None;
        }
        Ok(())
    }

    /// Pending/success reuse the exact flight. A completed pre-release
    /// failure may retry owned settlement; consumed Kernel release never does.
    pub async fn cleanup_session(self: &Arc<Self>, quiesce_processes: bool) -> Result<(), AppError> {
        let done = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| failure("resource state poisoned"))?;
            if retry_settlement(&state.release,state.kernel_release_started) {state.release=None;}
            if let Some(done) = &state.release {
                done.clone()
            } else {
                let tools = state.tools.clone();
                self.resource_tasks.close()?;
                // Soft Runtime retirement retains Session responsibilities.
                // Close their admission and join the original writebacks
                // naturally; only explicit Session teardown cancels them.
                self.knowledge_writebacks.close(quiesce_processes)?;
                if let Some(tools) = &tools {
                    tools.close_session()?;
                }
                let prior = state.turn.as_ref().and_then(|turn| turn.cleanup.clone());
                let owner = self.clone();
                let executor = tokio::runtime::Handle::try_current().map_err(failure)?;
                let task = executor.spawn(async move {
                    let knowledge = guard_effect_settlement(|| owner.knowledge_writebacks.join()).await
                        .map_err(|error| error.to_string());
                    let prior = match prior {
                        Some(done) => done.await,
                        None => Ok(()),
                    };
                    let settled = match &tools {
                        Some(tools) => owner
                            .settle_owned(tools, quiesce_processes)
                            .await
                            .map_err(|error| error.to_string()),
                        None => Ok(()),
                    };
                    // Revoke local Robot authority even when settlement is
                    // unknown. This is not a physical-stop receipt.
                    let robot_closed = match owner.robot_tools.get().and_then(Option::as_ref) {
                        Some(frozen) => frozen.close().map_err(|error| error.to_string()),
                        None => Ok(()),
                    };
                    // The new exact-owner settlement supersedes a completed
                    // old failure, never a pending flight or failed new proof.
                    // Preserve the old error in its original caller/journal.
                    if prior.is_err() {tracing::warn!(stage="prior_turn_settlement","prior resource cleanup failed; current owned settlement must independently prove shutdown");}
                    settled?;
                    robot_closed?;
                    knowledge?;
                    owner.state.lock().map_err(|_|"resource state poisoned".to_owned())?.kernel_release_started=true;
                    owner
                        .kernel
                        .release_resources(&ScopeKey::from(format!(
                            "session:{}",
                            owner.session_id.as_ref()
                        )))
                        .await
                        .map_err(|error| error.to_string())
                });
                let done = async move { task.await.map_err(|error| error.to_string())? }
                    .boxed()
                    .shared();
                state.release = Some(done.clone());
                done
            }
        };
        done.await.map_err(failure)?;
        if quiesce_processes {
            // Also covers a previously completed soft Runtime recycle and a
            // fresh Runtime with no ToolHost but retained Session processes.
            self.wave2.cleanup_runtime_session(&self.principal.principal_id,self.session_id.as_ref()).await?;
        }
        #[cfg(feature = "computer-use")]
        self.computer.close_session(self.session_id.as_ref()).await.map_err(failure)?;
        *self
            .creation_turn_root
            .lock()
            .map_err(|_| failure("creation turn identity lock poisoned"))? = None;
        Ok(())
    }
}

#[cfg(test)]
mod workspace_module_tests {
    use super::*;
    use nomifun_common::AgentToolPolicy;

    fn media_invocation(action: &str) -> nomifun_engine_core::EngineToolInvocation {
        use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
        use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition};
        use nomifun_engine_core::{EngineEffectClass, EngineToolBinding, EngineToolInvocation};
        let schema = StrictJsonValue(serde_json::json!({"type":"object","additionalProperties":false,"properties":{"prompt":{"type":"string"}},"required":["prompt"]}));
        EngineToolInvocation {
            agent_session_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
            principal: nomifun_agent_contracts::PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
            resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "a".repeat(64).into() },
            active_set_generation: 0, turn_operation_id: "turn".into(), operation_id: "tool".into(),
            idempotency_key: "key".into(), correlation_id: "call".into(),
            call: ChatToolCall { call_id: "call".into(), name: "media".into(), arguments: StrictJsonValue(serde_json::json!({"prompt":"cat"})), provider_metadata: Default::default() },
            binding: EngineToolBinding {
                model_name: "media".into(),
                definition: ChatToolDefinition { name: "media".into(), description: "Create media".into(), input_schema: schema.clone(), deferred: true },
                schema_digest: nomifun_engine_core::input_schema_digest(&schema).unwrap(),
                canonical_input_schema_ref: format!("schema://{action}/input").into(),
                capability_contract_digest: "b".repeat(64).into(), capability_id: CapabilityId::from("creation.media"), action_id: ActionId::from(action),
                resource_binding_ids: Default::default(), effect_class: EngineEffectClass::ManagedEffect, parallel_safe: false,
            },
        }
    }

    #[test]
    fn media_schema_visibility_keeps_the_exact_frozen_execution_mapping() {
        for action in nomifun_agent_domain_wave3::CREATION_MEDIA_ACTION_IDS {
            let frozen = media_invocation(action);
            let plan = EngineToolPlan::new([frozen.binding.clone()]).unwrap();
            let mut visible = frozen.clone();
            visible.binding.definition.deferred = false;
            assert_ne!(visible.binding, frozen.binding, "presentation really changed");
            assert_eq!(frozen_invocation_binding(&plan, &visible), Some(&frozen.binding));
            assert!(plan.binding("media").unwrap().definition.deferred, "the admitted plan stays frozen");
            for scope in [AgentToolPolicy::ReadOnly, AgentToolPolicy::ReadShell] {
                assert!(!constraints_allow_action(constraints(scope), "creation.media", action));
            }
        }
    }

    #[test]
    fn robot_schema_deferral_keeps_the_host_frozen_install_and_dispatch_mapping() {
        let mut robot = media_invocation("robot/device");
        robot.call.name = "robot_extension_read".into();
        robot.binding.model_name = robot.call.name.clone();
        robot.binding.definition.name = robot.call.name.clone();
        // A rich device definition is genuinely deferred by the existing
        // eager-device budget; small Robot controls remain visible by policy.
        robot.binding.definition.description = "Read the bound device status and its typed extension fields. ".repeat(128);
        robot.binding.definition.deferred = false;
        robot.binding.capability_id = "robot".into();
        robot.binding.effect_class = nomifun_engine_core::EngineEffectClass::ExternalUncertainEffect;
        robot.binding.resource_binding_ids.insert("robot-binding".into());
        let frozen = EngineToolPlan::new([robot.binding.clone()]).unwrap();
        let mut definitions = vec![robot.binding.clone()];
        // Exercise the real presentation budget. A large selected catalog
        // defers Robot schemas after their device owner has already frozen
        // the exact tool mapping, just as ordinary ToolSearch later reveals it.
        for index in 0..4 {
            let mut filler = media_invocation("creation.media/image").binding;
            filler.model_name = format!("catalog_tool_{index}");
            filler.definition.name = filler.model_name.clone();
            filler.definition.description = "x".repeat(8192);
            filler.definition.deferred = false;
            definitions.push(filler);
        }
        let projected = super::super::agent_tool_presentation::project(
            EngineToolPlan::new(definitions).unwrap(), true,
        ).unwrap();
        let presented = projected.binding(&robot.call.name).unwrap();
        assert!(presented.definition.deferred);
        assert_ne!(presented, &robot.binding, "presentation changed after device freezing");
        assert_eq!(frozen_tool_binding(&frozen, &robot.call.name, presented), Some(&robot.binding));
        robot.binding = presented.clone();
        assert_eq!(frozen_invocation_binding(&frozen, &robot), frozen.binding(&robot.call.name));
        robot.binding.definition.deferred = false;
        assert_eq!(frozen_invocation_binding(&frozen, &robot), frozen.binding(&robot.call.name));
    }

    #[test]
    fn visible_tool_cannot_change_any_execution_authority_or_schema_field() {
        use nomifun_engine_core::{EngineEffectClass, EngineToolBinding};
        let frozen = media_invocation("creation.media/image");
        let plan = EngineToolPlan::new([frozen.binding.clone()]).unwrap();
        let forgeries: &[fn(&mut EngineToolBinding)] = &[
            |binding| binding.model_name = "different-name".into(),
            |binding| binding.definition.name = "different-name".into(),
            |binding| binding.definition.description = "different-contract".into(),
            |binding| binding.definition.input_schema.0["properties"]["extra"] = serde_json::json!({"type":"string"}),
            |binding| binding.schema_digest = "c".repeat(64).into(),
            |binding| binding.canonical_input_schema_ref = "schema://different/input".into(),
            |binding| binding.capability_contract_digest = "c".repeat(64).into(),
            |binding| binding.capability_id = "creative.workshop".into(),
            |binding| binding.action_id = "creation.media/video".into(),
            |binding| { binding.resource_binding_ids.insert("ungranted-resource".into()); },
            |binding| binding.effect_class = EngineEffectClass::ReadOnly,
            |binding| binding.parallel_safe = true,
        ];
        for change in forgeries {
            let mut forged = frozen.clone();
            forged.binding.definition.deferred = false;
            change(&mut forged.binding);
            assert!(frozen_tool_binding(&plan, &forged.call.name, &forged.binding).is_none());
            assert!(frozen_invocation_binding(&plan, &forged).is_none());
        }
        let mut renamed = frozen;
        renamed.call.name = "not-admitted".into();
        assert!(frozen_invocation_binding(&plan, &renamed).is_none());
        assert!(frozen_invocation_binding(&EngineToolPlan::default(), &renamed).is_none());
    }

    #[tokio::test]
    async fn completed_cleanup_failure_can_retry_but_pending_and_success_stay_shared() {
        let pending=futures_util::future::pending::<Result<(),String>>().boxed().shared();
        assert!(!completed_failure(&Some(pending)));
        let success=futures_util::future::ready(Ok::<(),String>(())).boxed().shared();
        success.clone().await.unwrap();assert!(!completed_failure(&Some(success)));
        let failed=futures_util::future::ready(Err::<(),String>("first owner cleanup unproven".into())).boxed().shared();
        assert!(failed.clone().await.is_err());assert!(completed_failure(&Some(failed.clone())));
        assert!(retry_settlement(&Some(failed.clone()),false));
        assert!(!retry_settlement(&Some(failed),true),"consumed release cannot retry");
        let retry=futures_util::future::ready(Ok::<(),String>(())).boxed().shared();
        retry.clone().await.unwrap();assert!(!completed_failure(&Some(retry)));
    }

    fn constraints(tool_scope: AgentToolPolicy) -> ExecutionConstraints {
        ExecutionConstraints {
            version: 1,
            tool_scope,
            exclude_delegation: false,
            managed_process_only: false,
        }
    }

    #[test]
    fn delegation_ceiling_preserves_user_decision_handoffs() {
        let ceiling = ExecutionConstraints {
            exclude_delegation: true,
            ..constraints(AgentToolPolicy::Full)
        };
        assert!(!constraints_allow_action(ceiling, "agent.collaboration", "agent/delegate"));
        assert!(!constraints_allow_action(ceiling, "agent.collaboration", "agent/fork"));
        assert!(constraints_allow_action(ceiling, "agent.collaboration", "agent/request_user_decision"));
    }

    #[test]
    fn frozen_companion_fact_resource_requires_exact_paired_read_grants() {
        use nomifun_agent_contracts::{PrincipalRef,TypedResourceBinding};
        use nomifun_agent_kernel::CompiledCapabilityPolicy;
        let principal=PrincipalRef{principal_kind:"user".into(),principal_id:"fact-owner".into()};
        let binding=|id:&str,kind:&str,resource:&str|TypedResourceBinding{
            binding_id:id.into(),resource_kind:kind.into(),resource_id:resource.into(),owner_id:principal.principal_id.clone(),
            operations:["read".into()].into(),connection_config_ref:None,typed_parameters:Default::default(),
        };
        let digest="a".repeat(64);
        let envelope=serde_json::from_value(serde_json::json!({
            "snapshot_ref":{"snapshot_id":"fact-snapshot","snapshot_digest":digest},
            "content":{"schema_version":"1","resolver_version":"1", "preset_revision_ref":{
                "preset_id":"fact-preset","revision":1,"revision_digest":digest},
                "required_runtime_protocol_version":"1","required_runtime_profile":"managed_minimal",
                "runtime_feature_inventory_digest":digest,"required_runtime_features":[],"compiled_runtime_profile_digest":digest,
                "model_route_refs":{},"enabled_capabilities":[],"capability_allowlist":[],"skill_locks":[],"mcp_tool_locks":[],
                "canonical_schema_manifest_digest":digest,"target_contribution_manifest_digest":digest},
            "actor":principal,"scene":"agent_chat","surface":"test","audience":"owner","created_at_ms":1,
            "resolver_run_id":"fact-run","availability_evidence_revision":"1",
        })).unwrap();
        let mut compiled=CompiledSnapshot{
            envelope,authority_policies:[
                ("companion".into(),CompiledCapabilityPolicy{allowed_actions:Default::default(),resource_binding_ids:["persona".into()].into(),required_resource_kinds:["companion".into()].into()}),
                ("companion.memory".into(),CompiledCapabilityPolicy{allowed_actions:["companion.memory/recall".into()].into(),resource_binding_ids:["memory".into()].into(),required_resource_kinds:["companion_memory".into()].into()}),
            ].into(),
            target_resource_bindings:vec![binding("persona","companion","chosen"),binding("memory","companion_memory","chosen"),
                binding("unbound","companion_memory","unrelated")],
            registry_generation:1,registry_digest:digest.into(),non_executable_capability_ids:Default::default(),
        };
        assert_eq!(paired_companion_fact_owner(&principal,&compiled).unwrap(),"chosen");
        compiled.target_resource_bindings[1].resource_id="another-companion".into();
        assert!(paired_companion_fact_owner(&principal,&compiled).is_err());
        compiled.target_resource_bindings[1].resource_id="chosen".into();
        compiled.target_resource_bindings[1].owner_id="different-owner".into();
        assert!(paired_companion_fact_owner(&principal,&compiled).is_err());
        compiled.target_resource_bindings[1].owner_id=principal.principal_id.clone();
        compiled.target_resource_bindings[1].operations=["write".into()].into();
        assert!(paired_companion_fact_owner(&principal,&compiled).is_err());
        compiled.target_resource_bindings[1].operations=["read".into()].into();
        compiled.authority_policies.get_mut(&"companion.memory".into()).unwrap().resource_binding_ids.clear();
        assert!(paired_companion_fact_owner(&principal,&compiled).is_err(),"an unrelated same-kind grant is not the policy-bound memory resource");
    }

    #[test]
    fn restricted_attempts_filter_exact_workspace_actions_not_whole_modules() {
        let read_only = constraints(AgentToolPolicy::ReadOnly);
        assert!(constraints_allow_action(
            read_only,
            nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID,
            "workspace.files/read",
        ));
        assert!(constraints_allow_action(
            read_only,
            nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID,
            "workspace.files/search",
        ));
        assert!(!constraints_allow_action(
            read_only,
            nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID,
            "workspace.files/write",
        ));
        assert!(!constraints_allow_action(
            read_only,
            nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
            "workspace.process/start",
        ));

        let read_shell = constraints(AgentToolPolicy::ReadShell);
        assert!(constraints_allow_action(
            read_shell,
            nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
            "workspace.process/start",
        ));
        for action in [
            "workspace.process/poll",
            "workspace.process/input",
            "workspace.process/close_stdin",
            "workspace.process/resize",
            "workspace.process/cancel",
        ] {
            assert!(!constraints_allow_action(
                read_shell,
                nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
                action,
            ));
        }

        let managed = ExecutionConstraints {
            managed_process_only: true,
            ..constraints(AgentToolPolicy::Full)
        };
        for action in [
            "workspace.process/start",
            "workspace.process/poll",
            "workspace.process/input",
            "workspace.process/close_stdin",
            "workspace.process/resize",
            "workspace.process/cancel",
        ] {
            assert!(constraints_allow_action(
                managed,
                nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
                action,
            ));
        }
        assert!(!constraints_allow_action(
            managed,
            nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
            "workspace.process/exec",
        ));
        assert!(!constraints_allow_action(
            managed,
            nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID,
            "workspace.files/read",
        ));
    }

    #[test]
    fn manual_knowledge_writeback_requires_explicit_non_negated_intent() {
        assert!(explicitly_requests_knowledge_writeback(
            "请把这段结论写入知识库"
        ));
        assert!(explicitly_requests_knowledge_writeback(
            "Save this durable rule to the knowledge base"
        ));
        assert!(!explicitly_requests_knowledge_writeback(
            "这是一段可能有用的结论"
        ));
        assert!(!explicitly_requests_knowledge_writeback(
            "不要写入知识库，只在当前回答中使用"
        ));
    }
}
