//! Exact host-backed bundled capability composition for the current Nomi
//! runtime.
//!
//! Domain-support declarative registrations remain useful as the complete C7
//! inventory, but they are not execution owners. This module replaces only
//! package registrations for which the Nomi application supplies real typed
//! wave hosts, and publishes separate explicit Tool and Context admission
//! sets. Event/Transport/Resource/Middleware capabilities need their own
//! Session lifecycle owners and are never inferred ready from registration
//! presence.

use std::collections::BTreeSet;
use std::sync::Arc;

use nomifun_agent_contracts::{CapabilityId, CanonicalSchemaRef, ResolvedCapability, StrictJsonValue};
use nomifun_agent_kernel::PluginRegistration;
use nomifun_ai_agent::{
    NomiPlatformBuiltinLifecycleInvocation,
    NomiPlatformBuiltinLifecycleInvoker,
    NomiPlatformBuiltinToolSchemaResolver,
    NomiPlatformBuiltinToolSchemaRouter,
};
use nomifun_agent_domain_wave4::Wave4TurnMiddlewareHostPort;

use crate::services::AppServices;

pub(crate) struct NomiCoreBuiltinPlan {
    pub registrations: Vec<PluginRegistration>,
    pub tool_capability_ids: BTreeSet<CapabilityId>,
    pub host_dynamic_tool_capability_ids: BTreeSet<CapabilityId>,
    pub context_capability_ids: BTreeSet<CapabilityId>,
    pub lifecycle_capability_ids: BTreeSet<CapabilityId>,
    pub schema_resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
    pub lifecycle_invoker: Arc<dyn NomiPlatformBuiltinLifecycleInvoker>,
    pub wave4_owners: Arc<super::nomi_core_wave4::NomiCoreWave4Owners>,
    pub wave5_owner: Arc<super::agent_wave5_host::NomiCoreWave5Host>,
    pub wave2_owner: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    pub robot_owner: Option<Arc<super::nomi_core_robot::NomiCoreRobotWave4Owner>>,
}

pub(crate) async fn build(
    services: &AppServices,
    effect_store: nomifun_agent_session::AgentSessionStore,
) -> anyhow::Result<NomiCoreBuiltinPlan> {
    let mut registrations = nomifun_agent_domain_support::registrations(
        nomifun_agent_domain_support::c7_package_specs(),
    )?;
    registrations.push(super::nomi_core_tool_discovery::registration()?);

    #[cfg(feature = "browser-use")]
    let wave1_search = services.local_web_search.as_ref().map(|provider| {
        Arc::clone(provider) as Arc<dyn nomifun_ai_agent::web_search::SearchProvider>
    });
    #[cfg(not(feature = "browser-use"))]
    let wave1_search = None;
    let wave1 = super::agent_wave1_host::wave1_registrations_for_nomi_core(
        Arc::clone(&services.knowledge_service),
        wave1_search,
        Arc::clone(&services.companion_service),
        services.database.pool().clone(),
    )?;
    replace_package_registrations(&mut registrations, wave1);
    let wave1_tools = [
        nomifun_agent_domain_wave1::WEB_RESEARCH_MODULE_ID,
        nomifun_agent_domain_wave1::KNOWLEDGE_MODULE_ID,
        nomifun_agent_domain_wave1::PROJECT_MEMORY_MODULE_ID,
        nomifun_agent_domain_wave1::COMPANION_MEMORY_MODULE_ID,
    ]
    .into_iter()
    .map(CapabilityId::from)
    .collect::<BTreeSet<_>>();
    let wave1_context = BTreeSet::new();

    let wave2_owner = super::nomi_core_wave2::action_host_port(services, effect_store.clone());
    let wave2_ports =
        nomifun_agent_domain_wave2::Wave2RoleHostPorts::with_actions(wave2_owner.clone());
    let wave2 = nomifun_agent_domain_wave2::registrations_with_role_host_ports(wave2_ports)
        .map_err(anyhow::Error::msg)?;
    replace_package_registrations(&mut registrations, wave2);
    let mcp_registrations = super::nomi_core_mcp_catalog::load_registrations(
        &nomifun_db::SqliteMcpServerRepository::new(services.database.pool().clone()),
        wave2_owner.clone(),
    )
    .await?;
    let mcp_tools = mcp_registrations
        .iter()
        .flat_map(|registration| {
            registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
                .iter()
                .map(|capability| capability.id.clone())
        })
        .collect::<BTreeSet<_>>();
    registrations.extend(mcp_registrations);
    let wave2_tools = super::nomi_core_wave2::tool_capability_ids();
    let wave2_lifecycle = super::nomi_core_wave2::event_capability_ids();

    let wave3 = super::agent_wave3_host::registrations(
        Arc::clone(&services.creation_service),
        Arc::clone(&services.workshop_service),
        Arc::clone(&services.plugin_runtime),
        Arc::clone(&services.model_invoke_service),
        services.database.pool().clone(),
        Arc::clone(&services.authoritative_user_id),
    )
    .map_err(anyhow::Error::msg)?;
    replace_package_registrations(&mut registrations, wave3);
    let wave3_tools = super::agent_wave3_host::approved_capability_ids();

    // The companion owner is ready at this composition boundary. The same
    // Channel owner Arc is installed later by channel assembly before router
    // publication; startup fails if that promised owner cannot be installed.
    let wave4 = Arc::new(super::nomi_core_wave4::NomiCoreWave4Owners::new(
        Arc::clone(&services.authoritative_user_id),
        Arc::clone(&services.companion_service),
        services.database.pool().clone(),
    ));
    let wave4_registrations = wave4.registrations().map_err(anyhow::Error::msg)?;
    replace_package_registrations(&mut registrations, wave4_registrations);
    // Channel owners are installed before router publication by
    // `build_module_states`; admission is locked now so Session execution and
    // post-install catalog refresh keep the same exact target set.
    let customer_service_owner = Arc::new(
        nomifun_customer_service::CustomerServiceAgentCapabilityOwner::new(
            Arc::clone(&services.authoritative_user_id),
            Arc::clone(&services.customer_service_service),
        ),
    );
    let customer_action_host: Arc<dyn nomifun_agent_domain_wave4::Wave4HostPort> =
        customer_service_owner.clone();
    replace_package_registrations(
        &mut registrations,
        vec![
            nomifun_agent_domain_wave4::customer_service_registration_with_all_host_ports(
                customer_action_host,
                nomifun_agent_domain_wave4::unconfigured_context_host_port(),
                customer_service_owner.clone()
                    as Arc<dyn nomifun_agent_domain_wave4::Wave4TurnMiddlewareHostPort>,
            )
            .map_err(anyhow::Error::msg)?,
        ],
    );
    let wave4_tools = super::nomi_core_wave4::nomi_core_wave4_tool_capability_ids()
        .into_iter()
        .chain([CapabilityId::from(
            nomifun_agent_domain_wave4::CUSTOMER_SERVICE_MODULE_ID,
        )])
        .collect::<BTreeSet<_>>();
    let mut wave4_context =
        super::nomi_core_wave4::nomi_core_wave4_context_capability_ids();
    let mut lifecycle_capability_ids =
        super::nomi_core_wave4::nomi_core_wave4_lifecycle_capability_ids()
            .into_iter()
            .collect::<BTreeSet<_>>();
    let robot_owner = services.robot.as_ref().map(|robot| {
        Arc::new(super::nomi_core_robot::NomiCoreRobotWave4Owner::new(
            Arc::clone(&services.authoritative_user_id),
            Arc::clone(robot),
        ))
    });
    if let Some(owner) = robot_owner.as_ref() {
        replace_package_registrations(
            &mut registrations,
            vec![super::nomi_core_robot::NomiCoreRobotWave4Owner::registration(
                Arc::clone(owner),
            )
            .map_err(anyhow::Error::msg)?],
        );
        wave4_context.extend(super::nomi_core_robot::context_capability_ids());
        lifecycle_capability_ids
            .extend(super::nomi_core_robot::lifecycle_capability_ids());
    }
    let lifecycle_invoker: Arc<dyn NomiPlatformBuiltinLifecycleInvoker> =
        Arc::new(NomiCoreLifecycleInvoker {
            wave4: Arc::clone(&wave4),
            customer_service: Arc::clone(&customer_service_owner),
            robot: robot_owner.clone(),
        });

    // Notification and Remote remain platform packages with zero Agent
    // capabilities. Installing their empty target manifests removes the old
    // EventConsumer/Transport authoring identities without deleting the real
    // outbox and ingress owners.
    replace_package_registrations(
        &mut registrations,
        vec![nomifun_agent_domain_wave4::notification_registration()
            .map_err(anyhow::Error::msg)?],
    );

    let wave5_owner = Arc::new(super::agent_wave5_host::NomiCoreWave5Host::new(
        services.authoritative_user_id.clone(),
        effect_store,
        Arc::clone(&services.requirement_service),
    ));
    let wave5 = nomifun_agent_domain_wave5::registrations_with_host_port(
        Arc::clone(&wave5_owner) as Arc<dyn nomifun_agent_domain_wave5::Wave5HostPort>,
    )
    .map_err(anyhow::Error::msg)?;
    replace_package_registrations(&mut registrations, wave5);
    let wave5_tools = nomifun_agent_domain_wave5::target_capability_ids();

    let schema_router = NomiPlatformBuiltinToolSchemaRouter::new([
        (
            wave1_tools.clone(),
            Arc::new(NomiWave1SchemaResolver)
                as Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
        ),
        (
            wave2_tools.clone(),
            super::nomi_core_wave2::schema_resolver(),
        ),
        (
            wave3_tools.clone(),
            super::agent_wave3_host::schema_resolver(),
        ),
        (
            wave4_tools.clone(),
            Arc::new(NomiWave4SchemaResolver)
                as Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
        ),
        (
            wave5_tools.clone(),
            Arc::new(NomiWave5SchemaResolver)
                as Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
        ),
    ])?;
    let tool_capability_ids = wave1_tools
        .into_iter()
        .chain(wave2_tools)
        .chain(wave3_tools)
        .chain(wave4_tools)
        .chain(wave5_tools)
        .collect();
    let mut host_dynamic_tool_capability_ids = robot_owner
        .as_ref()
        .map(|_| super::nomi_core_robot::tool_capability_ids())
        .unwrap_or_default();
    // Available through explicitly compatible engines, not Nomi's native
    // registry or its JavaScript Plugin tool path.
    host_dynamic_tool_capability_ids.extend(mcp_tools);
    let context_capability_ids = wave1_context
        .into_iter()
        .chain(wave4_context)
        .collect();
    lifecycle_capability_ids.extend(wave2_lifecycle);

    Ok(NomiCoreBuiltinPlan {
        registrations,
        tool_capability_ids,
        host_dynamic_tool_capability_ids,
        context_capability_ids,
        lifecycle_capability_ids,
        schema_resolver: Arc::new(schema_router),
        lifecycle_invoker,
        wave4_owners: wave4,
        wave5_owner,
        wave2_owner,
        robot_owner,
    })
}

fn replace_package_registrations(
    current: &mut Vec<PluginRegistration>,
    replacements: Vec<PluginRegistration>,
) {
    let package_ids = replacements
        .iter()
        .map(|registration| {
            registration.metadata.manifest.payload.package_id.clone()
        })
        .collect::<BTreeSet<_>>();
    current.retain(|registration| {
        !package_ids.contains(
            &registration.metadata.manifest.payload.package_id,
        )
    });
    current.extend(replacements);
}

struct NomiWave1SchemaResolver;

#[async_trait::async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for NomiWave1SchemaResolver {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        nomifun_agent_domain_wave1::resolve_canonical_schema(
            capability.capability.id.as_ref(),
            reference,
        )
    }
}

struct NomiWave4SchemaResolver;

#[async_trait::async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for NomiWave4SchemaResolver {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        nomifun_agent_domain_wave4::resolve_capability_schema(reference)?
            .ok_or_else(|| {
                format!(
                    "Wave 4 has no canonical schema matching {} for {}",
                    reference.as_ref(),
                    capability.capability.id.as_ref()
                )
            })
    }
}

struct NomiWave5SchemaResolver;

#[async_trait::async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for NomiWave5SchemaResolver {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        nomifun_agent_domain_wave5::resolve_action_schema(
            capability.capability.id.as_ref(),
            reference,
        )
    }
}

struct NomiCoreLifecycleInvoker {
    wave4: Arc<super::nomi_core_wave4::NomiCoreWave4Owners>,
    customer_service:
        Arc<nomifun_customer_service::CustomerServiceAgentCapabilityOwner>,
    robot: Option<Arc<super::nomi_core_robot::NomiCoreRobotWave4Owner>>,
}

#[async_trait::async_trait]
impl NomiPlatformBuiltinLifecycleInvoker for NomiCoreLifecycleInvoker {
    async fn activate(
        &self,
        request: NomiPlatformBuiltinLifecycleInvocation,
    ) -> Result<StrictJsonValue, String> {
        match request.capability.capability.id.as_ref() {
            nomifun_agent_domain_wave4::CHANNEL_RECEIVE
            | nomifun_agent_domain_wave4::CHANNEL_PAIRING
            | nomifun_agent_domain_wave4::CHANNEL_GROUP_POLICY => {
                self.wave4.activate(request).await
            }
            nomifun_agent_domain_wave4::CUSTOMER_SERVICE_DIALOGUE => {
                let schema_ref = request.schema_ref.ok_or_else(|| {
                    "customer_service.dialogue has no canonical middleware schema"
                        .to_owned()
                })?;
                self.customer_service
                    .apply(
                        nomifun_agent_domain_wave4::Wave4TurnMiddlewareHostRequest {
                            principal: request.principal,
                            agent_session_id: request.agent_session_id,
                            operation_id: request.operation_id,
                            correlation_id: request.correlation_id,
                            resolved_snapshot_ref: request.resolved_snapshot_ref,
                            registry_generation: request.registry_generation,
                            registry_digest: request.registry_digest,
                            capability_id: request.capability.capability.id,
                            state_scope_key: request.state_scope_key,
                            resource_bindings: request.resource_bindings,
                            schema_ref,
                            turn_input: request.turn_input,
                        },
                    )
                    .await
                    .map_err(|error| error.to_string())
            }
            nomifun_agent_domain_wave4::ROBOT_LINK
            | nomifun_agent_domain_wave4::ROBOT_AUDIO => self
                .robot
                .as_ref()
                .ok_or_else(|| "Nomi Robot lifecycle owner is unavailable".to_owned())?
                .activate_lifecycle(request)
                .await,
            "workspace.files" => Ok(StrictJsonValue(
                serde_json::json!({
                    "capability_id": "workspace.files",
                    "state": "active"
                }),
            )),
            other => Err(format!(
                "no Nomi lifecycle owner is admitted for {other}"
            )),
        }
    }

    async fn context_contributor(
        &self,
        request: NomiPlatformBuiltinLifecycleInvocation,
    ) -> Result<Option<Arc<dyn nomifun_ai_agent::ContextContributor>>, String> {
        if matches!(
            request.capability.capability.id.as_ref(),
            nomifun_agent_domain_wave4::ROBOT_LINK
                | nomifun_agent_domain_wave4::ROBOT_AUDIO
        ) {
            return self
                .robot
                .as_ref()
                .ok_or_else(|| "Nomi Robot lifecycle owner is unavailable".to_owned())?
                .lifecycle_context_contributor(&request)
                .await;
        }
        if request.capability.capability.id.as_ref() != "workspace.files" {
            return Ok(None);
        }
        let workspace = request
            .resource_bindings
            .iter()
            .find(|binding| binding.resource_kind.as_ref() == "workspace")
            .and_then(|binding| binding.typed_parameters.get("workspace_root"))
            .ok_or_else(|| {
                "workspace.files watch context has no server-resolved workspace root".to_owned()
            })?;
        super::nomi_core_wave2::NomiWorkspaceWatchContext::start(workspace)
            .map(|contributor| {
                Some(contributor as Arc<dyn nomifun_ai_agent::ContextContributor>)
            })
            .map_err(|error| error.to_string())
    }
}
