//! Production Kernel/resource assembly shared by source-integrated engines.
//! Strategy, tool naming and event codecs remain engine-owned. This module
//! retains actual resource owners and their cleanup witnesses, not fake exits.
use std::sync::{Arc, Mutex};

use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomifun_agent_contracts::{AgentSessionId, ScopeKey};
use nomifun_agent_kernel::{
    CompiledSnapshot, CompilerEnvironment, KernelRegistry, MaterializedRegistry,
    SessionCapabilityState,
};
use nomifun_ai_agent::engine_effect_scope::guard_effect_settlement;
use nomifun_api_types::{ExecutionConstraints, RuntimeBuildBinding};
use nomifun_common::{AgentToolPolicy, AppError};
use nomifun_engine_core::{EngineToolExposure, EngineToolPlan, KernelEngineToolInvoker};

use super::engine_journal::EngineTurnJournal;
use super::engine_session_host::{AdmittedEngineSession, EngineTurnReceipt};
use super::engine_tool_host::{EngineToolHost, EngineToolObservationPolicy};

type Completion = Shared<BoxFuture<'static, Result<(), String>>>;

struct ConstrainedTools {
    inner: Arc<dyn nomifun_engine_core::EngineToolInvoker>,
    plan: EngineToolPlan,
    constraints: ExecutionConstraints,
    wave2: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    git_root: Option<std::path::PathBuf>,
}

#[async_trait::async_trait]
impl nomifun_engine_core::EngineToolInvoker for ConstrainedTools {
    async fn invoke(
        &self,
        invocation: nomifun_engine_core::EngineToolInvocation,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<nomifun_engine_core::EngineToolResult, nomifun_engine_core::EngineToolError> {
        if self.plan.binding(&invocation.call.name) != Some(&invocation.binding)
            || !constraints_allow_action(
                self.constraints,
                invocation.binding.capability_id.as_ref(),
                invocation.binding.action_id.as_ref(),
            )
        {
            return Err(nomifun_engine_core::EngineToolError::ToolInvocation(
                "Tool is outside the frozen Session execution ceiling".into(),
            ));
        }
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
    pub kernel: Arc<KernelRegistry>,
    pub environment: CompilerEnvironment,
    pub wave2: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    pub plugin_product: super::engine_plugin_product_tools::PluginProductOwner,
    pub robot: Option<Arc<super::nomi_core_robot::RobotModuleOwner>>,
}

fn constraints_allow_action(
    constraints: ExecutionConstraints,
    capability_id: &str,
    action_id: &str,
) -> bool {
    match capability_id {
        nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID => match constraints.tool_scope {
            AgentToolPolicy::Full => true,
            AgentToolPolicy::ReadOnly | AgentToolPolicy::ReadShell => matches!(
                action_id,
                "workspace.files/read" | "workspace.files/search"
            ),
        },
        nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID => match constraints.tool_scope {
            AgentToolPolicy::Full => true,
            AgentToolPolicy::ReadShell => action_id == "workspace.process/exec",
            AgentToolPolicy::ReadOnly => false,
        },
        nomifun_agent_domain_wave2::WORKSPACE_VCS_MODULE_ID
        | nomifun_agent_domain_wave2::WORKSPACE_ARTIFACTS_MODULE_ID => {
            constraints.tool_scope == AgentToolPolicy::Full
        }
        _ => constraints.allows_capability(capability_id),
    }
}

#[path = "engine_mcp_resources.rs"]
mod mcp_resources;
pub(crate) use mcp_resources::{page as project_resource_page, validate_page as validate_resource_page, resource_operation as mcp_resource_operation};
pub(crate) use mcp_resources::{resource_server_ids, select_resource_server};
pub(crate) use mcp_resources::validate_owner_result as validate_resource_owner_result;

struct Turn {
    operation: String,
    root: String,
    cleanup: Option<Completion>,
    resource_operations: std::collections::BTreeSet<String>,
}
#[derive(Default)]
struct State {
    tools: Option<Arc<EngineToolHost>>,
    turn: Option<Turn>,
    release: Option<Completion>,
}

/// One production resource context per live registered Session runtime.
/// Created only by EngineSessionHost from genuine owner-resolved facts.
pub struct EngineKernelSession {
    resource_tasks: nomifun_ai_agent::engine_sdk::EngineTaskGroup,
    resource_settlement_failed: std::sync::atomic::AtomicBool,
    source: Arc<()>,
    session_id: AgentSessionId,
    principal: nomifun_agent_contracts::PrincipalRef,
    binding: RuntimeBuildBinding,
    constraints: ExecutionConstraints,
    workspace: String,
    git_root: Option<std::path::PathBuf>,
    process_selected: bool,
    primary_image_input: bool,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    kernel: Arc<KernelRegistry>,
    wave2: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    plugin_product: super::engine_plugin_product_tools::PluginProductOwner,
    plugin_product_plan: tokio::sync::OnceCell<EngineToolPlan>,
    robot: Option<Arc<super::nomi_core_robot::RobotModuleOwner>>,
    robot_tools: tokio::sync::OnceCell<Option<Arc<super::engine_robot_tools::FrozenTools>>>,
    state: Mutex<State>,
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine resources: {message}"))
}

impl EngineKernelSession {
    pub(super) fn new(
        source: Arc<()>,
        session: &AdmittedEngineSession,
        assembly: &EngineKernelAssembly,
    ) -> Result<Self, AppError> {
        let mut binding = session.agent_binding().clone();
        let constraints = ExecutionConstraints::from_extra(&session.session().extra)?;
        let snapshot = session.snapshot();
        let session_id = AgentSessionId::from(session.session().conversation_id.clone());
        let principal = session.principal();
        // Use the owner-rebased response, never caller options or model JSON.
        let workspace = session
            .session()
            .extra
            .get("workspace")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let selected = || {
            snapshot
                .content
                .enabled_capabilities
                .iter()

        };
        let workspace_selected = selected().any(|item| {
            matches!(
                item.capability.id.as_ref(),
                nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID
                    | nomifun_agent_domain_wave2::WORKSPACE_VCS_MODULE_ID
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
        let primary_image_input = snapshot
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
                record
                    .primary
                    .features
                    .contains(&nomifun_agent_contracts::ChatRouteFeature::ImageInput)
            });
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
            resource_tasks: nomifun_ai_agent::engine_sdk::EngineTaskGroup::new(32)?,
            resource_settlement_failed: false.into(),
            source,
            session_id,
            principal: principal.clone(),
            binding: session.engine_binding().clone(),
            constraints,
            workspace,
            process_selected,
            primary_image_input,
            active: Arc::new(SessionCapabilityState::new(&compiled)),
            compiled,
            kernel: assembly.kernel.clone(),
            wave2: assembly.wave2.clone(),
            git_root,
            plugin_product: assembly.plugin_product.clone(),
            plugin_product_plan: tokio::sync::OnceCell::new(),
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
            && self.workspace
                == session
                    .session()
                    .extra
                    .get("workspace")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
    }
    pub fn compiled(&self) -> &Arc<CompiledSnapshot> {
        &self.compiled
    }
    pub fn active_state(&self) -> &Arc<SessionCapabilityState> {
        &self.active
    }
    pub fn workspace(&self) -> &str {
        &self.workspace
    }
    pub fn execution_constraints(&self) -> ExecutionConstraints {
        self.constraints
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
            self.constraints.allows_capability(id.as_ref())
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

    /// Plugin Product actions from the immutable enabled capability set.
    /// Reading their schemas does not activate capabilities or start Services.
    pub async fn plugin_product_tool_plan(&self) -> Result<EngineToolPlan, AppError> {
        if self.constraints.restricted() {
            return Ok(EngineToolPlan::default());
        }
        let plan = self
            .plugin_product_plan
            .get_or_try_init(|| async {
                let exposures = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    self.plugin_product
                        .exposures(&self.principal.principal_id, &self.compiled),
                )
                .await
                .map_err(|_| failure("Plugin Product schema resolution timed out"))??;
                self.compile_tool_plan(exposures)
            })
            .await?;
        Ok(plan.clone())
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
            self.plugin_product
                .receipts
                .context(&self.principal.principal_id, self.session_id.as_ref()),
        )
        .await
        .map_err(|_| failure("hosted effect context timed out"))?
    }

    pub async fn ensure_hosted_effects_settled(&self) -> Result<(), AppError> {
        if self.resource_settlement_failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(failure("resource settlement journal is unproven"));
        }
        if let Some(root) = &self.git_root {
            self.wave2.ensure_workspace_git_evidence(root).await?;
        }
        self.plugin_product
            .receipts
            .ensure_settled(&self.principal.principal_id, self.session_id.as_ref())
            .await
    }

    /// Host-frozen Robot device schemas. Catalog reads never activate link or
    /// audio; the retained tool owner acquires only already-active grants.
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
                let owner = self
                    .robot
                    .clone()
                    .ok_or_else(|| failure("Robot owner unavailable"))?;
                let frozen = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    super::engine_robot_tools::FrozenTools::resolve(
                        owner,
                        self.plugin_product.receipts.clone(),
                        self.principal.clone(),
                        self.session_id.clone(),
                        &self.compiled,
                        |exposures| self.compile_tool_plan(exposures),
                    ),
                )
                .await
                .map_err(|_| failure("Robot catalog resolution timed out"))??;
                Ok(Some(Arc::new(frozen)))
            })
            .await?;
        Ok(tools
            .as_ref()
            .map(|tools| tools.plan.clone())
            .unwrap_or_default())
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
        let exposures = exposures.into_iter().take(257).collect::<Vec<_>>();
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
        policy: Arc<dyn EngineToolObservationPolicy>,
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
        let exposures = plan.model_definitions().into_iter().map(|definition| {
            let binding = plan
                .binding(&definition.name)
                .expect("plan definitions have bindings");
            EngineToolExposure {
                definition,
                capability_id: binding.capability_id.clone(),
                action_id: binding.action_id.clone(),
            }
        });
        if plan != self.compile_tool_plan(exposures)? {
            return Err(failure(
                "tool plan differs from the canonical Session mapping",
            ));
        }
        for definition in plan.model_definitions() {
            let binding = plan
                .binding(&definition.name)
                .expect("plan definition has binding");
            if super::engine_robot_tools::supported_ids().contains(&binding.capability_id)
                && self
                    .robot_tools
                    .get()
                    .and_then(Option::as_ref)
                    .and_then(|frozen| frozen.plan.binding(&definition.name))
                    != Some(binding)
            {
                return Err(failure(
                    "Robot tools require the exact host-frozen device surface",
                ));
            }
            if self
                .compiled
                .resolved_capability(&binding.capability_id)
                .is_some_and(|selected| selected.contribution_lock.source_kind == nomifun_agent_contracts::ContributionSourceKind::PluginProductActiveRelease)
                && self
                    .plugin_product_plan
                    .get()
                    .and_then(|frozen| frozen.binding(&definition.name))
                    != Some(binding)
            {
                return Err(failure(
                    "Plugin Product tools require the exact host-resolved schema surface",
                ));
            }
        }
        let invoker = KernelEngineToolInvoker::for_session(
            self.kernel.clone(),
            self.compiled.clone(),
            self.active.clone(),
            self.principal.clone(),
            self.session_id.clone(),
            plan.clone(),
        )
        .map_err(failure)?;
        let invoker = super::engine_workspace_media::WorkspaceMediaTools {
            inner: Arc::new(invoker),
            snapshot: self.compiled.clone(),
            active: self.active.clone(),
            primary_image_input: self.primary_image_input,
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
        let invoker = super::engine_plugin_product_tools::SessionTools {
            owner: self.plugin_product.clone(),
            inner: invoker,
            snapshot: self.compiled.clone(),
            active: self.active.clone(),
            principal: self.principal.clone(),
            session: self.session_id.clone(),
            plan: plan.clone(),
        };
        // Check the exact frozen mapping BEFORE Robot/Plugin Product/media adapters;
        // none may dispatch using a caller-supplied capability/effect label.
        let invoker = ConstrainedTools {
            inner: Arc::new(invoker),
            plan,
            constraints: self.constraints,
            wave2: self.wave2.clone(),
            git_root: self.git_root.clone(),
        };
        let tools = Arc::new(EngineToolHost::new(Arc::new(invoker), policy));
        state.tools = Some(tools.clone());
        Ok(tools)
    }

    pub fn open_turn(
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
        if let Some(root) = &self.git_root {
            self.wave2.ensure_workspace_git_ready(root)?;
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| failure("resource state poisoned"))?;
        if state.release.is_some()
            || state.turn.as_ref().is_some_and(|turn| {
                turn.operation == receipt.operation_id()
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
            root: receipt.root_message_id().into(),
            cleanup: None,
            resource_operations: Default::default(),
        });
        if self.process_selected {
            self.wave2.open_runtime_turn(
                &self.principal.principal_id,
                self.session_id.as_ref(),
                receipt.operation_id(),
                &self.workspace,
                journal,
            )?;
        }
        Ok(())
    }

    /// Observation only; callers must also join tool tasks and account for
    /// unread outcomes before capability transitions or terminal publication.
    pub async fn processes_quiescent(&self) -> Result<bool, AppError> {
        self.wave2
            .runtime_processes_quiescent(&self.principal.principal_id, self.session_id.as_ref())
            .await
    }

    async fn settle_owned(&self, tools: &EngineToolHost) -> Result<(), AppError> {
        let admission = tools.close_turn();
        // Catch each owner separately: the outer retained task alone would
        // preserve failure, but an unwind would skip all subsequent owners.
        let processes = guard_effect_settlement(|| {
            self.wave2
                .cleanup_runtime_session(&self.principal.principal_id, self.session_id.as_ref())
        })
        .await;
        let tasks = guard_effect_settlement(|| tools.join()).await;
        let resource_tasks = guard_effect_settlement(|| self.resource_tasks.join()).await;
        let git = guard_effect_settlement(|| async {
            match &self.git_root {
                Some(root) => self.wave2.settle_workspace_git(root).await,
                None => Ok(()),
            }
        })
        .await;
        let mcp = guard_effect_settlement(|| {
            self.wave2
                .ensure_mcp_settled(&self.principal.principal_id, self.session_id.as_ref())
        })
        .await;
        let hosted = guard_effect_settlement(|| self.ensure_hosted_effects_settled()).await;
        admission?;
        processes?;
        tasks?;
        resource_tasks?;
        if self.resource_settlement_failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(failure("resource settlement journal is unproven"));
        }
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
                    owner
                        .settle_owned(&tools)
                        .await
                        .map_err(|error| error.to_string())
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
        done.await.map_err(failure)
    }

    /// Final release cannot be mistaken for a fresh successful release after
    /// a consumed handle or a timed-out waiter. All callers reuse the result.
    pub async fn cleanup_session(self: &Arc<Self>) -> Result<(), AppError> {
        let done = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| failure("resource state poisoned"))?;
            if let Some(done) = &state.release {
                done.clone()
            } else {
                let tools = state.tools.clone();
                self.resource_tasks.close()?;
                if let Some(tools) = &tools {
                    tools.close_session()?;
                }
                let prior = state.turn.as_ref().and_then(|turn| turn.cleanup.clone());
                let owner = self.clone();
                let executor = tokio::runtime::Handle::try_current().map_err(failure)?;
                let task = executor.spawn(async move {
                    let prior = match prior {
                        Some(done) => done.await,
                        None => Ok(()),
                    };
                    let settled = match &tools {
                        Some(tools) => owner
                            .settle_owned(tools)
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
                    prior?;
                    settled?;
                    robot_closed?;
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
        done.await.map_err(failure)
    }
}

#[cfg(test)]
mod workspace_module_tests {
    use super::*;

    fn constraints(tool_scope: AgentToolPolicy) -> ExecutionConstraints {
        ExecutionConstraints {
            version: 1,
            tool_scope,
            exclude_delegation: false,
        }
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
            "workspace.process/exec",
        ));

        let read_shell = constraints(AgentToolPolicy::ReadShell);
        assert!(constraints_allow_action(
            read_shell,
            nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
            "workspace.process/exec",
        ));
        for action in [
            "workspace.process/start",
            "workspace.process/input",
            "workspace.process/cancel",
        ] {
            assert!(!constraints_allow_action(
                read_shell,
                nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
                action,
            ));
        }
    }
}
