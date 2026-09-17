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

#[cfg(all(test,feature="browser-use"))]
mod local_search_binding_tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    #[ignore="requires NOMIFUN_SEARCH_CHROME; local probe only, no public search"]
    async fn admitted_runtime_reaches_the_real_http_catalog() {
        let chrome=std::path::PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").expect("Chromium binary"));
        let product=nomi_browser_engine::headless_page::probe_runtime(chrome.clone()).await.unwrap();
        let mut host=crate::DesktopHostServices::default();
        host.set_browser_release(chrome,product).await.unwrap();
        let expected=host.local_web_search.as_ref().unwrap().binding().clone();
        let root=tempfile::tempdir().unwrap();
        let services=AppServices::try_from_config_with_host(nomifun_db::init_database_memory().await.unwrap(),&crate::AppConfig {
            data_dir:root.path().join("data"),work_dir:root.path().join("work"),
            auth_policy:nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret:Some(Arc::from("local-search-binding-test")),..Default::default()
        },host).await
            .unwrap_or_else(|_|panic!("isolated application startup failed"));
        let plan=build(&services).await.unwrap();
        let registration=plan.registrations.iter().find(|registration|registration.metadata.manifest.payload.package_id.as_ref()==nomifun_agent_domain_wave1::LOCAL_WEBSEARCH_PACKAGE_ID).unwrap();
        let manifest=&registration.metadata.manifest.payload.contributions.capabilities[0];
        assert_eq!(nomifun_ai_agent::local_web_search::binding_from_manifest(manifest).unwrap(),Some(expected));
        let router=crate::compatibility::create_router(&services).await;
        let response=router.oneshot(axum::http::Request::builder().uri("/api/capabilities")
            .header("x-nomi-local-trust","local-search-binding-test").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),axum::http::StatusCode::OK);
        let body=axum::body::to_bytes(response.into_body(),512*1024).await.unwrap();
        let catalog:serde_json::Value=serde_json::from_slice(&body).unwrap();
        let local=catalog["data"].as_array().unwrap().iter().find(|entry|entry["capability"]["id"]=="nomi_local_websearch").unwrap();
        assert_eq!(local["materialization_state"],"materialized");
        assert_eq!(local["required_resource_kinds"],serde_json::json!([]));
        services.shutdown_browser_platform().await.unwrap();
        services.database.close().await;
    }
}

pub(crate) struct NomiCoreBuiltinPlan {
    pub registrations: Vec<PluginRegistration>,
    pub tool_capability_ids: BTreeSet<CapabilityId>,
    pub host_dynamic_tool_capability_ids: BTreeSet<CapabilityId>,
    pub context_capability_ids: BTreeSet<CapabilityId>,
    pub lifecycle_capability_ids: BTreeSet<CapabilityId>,
    pub schema_resolver: Arc<dyn NomiPlatformBuiltinToolSchemaResolver>,
    pub lifecycle_invoker: Arc<dyn NomiPlatformBuiltinLifecycleInvoker>,
    pub wave4_owners: Arc<super::nomi_core_wave4::NomiCoreWave4Owners>,
    pub wave2_owner: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    pub robot_owner: Option<Arc<super::nomi_core_robot::NomiCoreRobotWave4Owner>>,
}

#[cfg(feature = "browser-use")]
fn bind_system_browser(
    registrations: &mut [PluginRegistration],
    binding: nomifun_browser_platform::system_browser::SystemBrowserBinding,
) -> anyhow::Result<()> {
    use nomifun_browser_platform::system_browser::{BINDING_ANNOTATION, TOOL_NAME};
    let registration = registrations.iter_mut().find(|entry|
        entry.metadata.manifest.payload.package_id.as_ref() == nomifun_agent_domain_wave1::SYSTEM_BROWSER_PACKAGE_ID)
        .ok_or_else(|| anyhow::anyhow!("System browser package is missing"))?;
    let mut manifest = registration.metadata.manifest.payload.clone();
    let capability = manifest.contributions.capabilities.iter_mut().find(|capability| capability.id.as_ref() == TOOL_NAME)
        .ok_or_else(|| anyhow::anyhow!("System browser capability is missing"))?;
    capability.config_schema.0.as_object_mut().ok_or_else(|| anyhow::anyhow!("System browser schema must be an object"))?
        .insert(BINDING_ANNOTATION.into(), serde_json::to_value(binding)?);
    registration.metadata.manifest = nomifun_agent_contracts::ArtifactEnvelope::new(manifest)?;
    Ok(())
}

#[cfg(all(test, feature = "browser-use"))]
mod system_browser_binding_tests {
    use super::*;
    use nomifun_browser_platform::system_browser::{BINDING_ANNOTATION, SystemBrowserBinding};

    #[test]
    fn binding_annotation_is_injected_only_into_the_system_browser_package() {
        let mut registrations = nomifun_agent_domain_wave1::registrations().unwrap();
        let binding = SystemBrowserBinding { schema_version: 1, runtime_digest: "a".repeat(64) };
        bind_system_browser(&mut registrations, binding.clone()).unwrap();
        for registration in registrations {
            assert!(registration.metadata.manifest.verify().unwrap());
            for manifest in &registration.metadata.manifest.payload.contributions.capabilities {
                let annotation = manifest.config_schema.0.get(BINDING_ANNOTATION);
                if manifest.id.as_ref() == nomifun_agent_domain_wave1::NOMI_SYSTEM_BROWSER {
                    assert_eq!(serde_json::from_value::<SystemBrowserBinding>(annotation.unwrap().clone()).unwrap(), binding);
                    assert!(super::super::nomi_core_agent_projection::native_capability_available(manifest));
                } else {
                    assert!(annotation.is_none(), "System-browser authority cannot be copied onto another capability");
                }
            }
        }
    }
}

pub(crate) async fn build(
    services: &AppServices,
    effect_store: nomifun_agent_session::AgentSessionStore,
) -> anyhow::Result<NomiCoreBuiltinPlan> {
    let mut registrations = nomifun_agent_domain_support::registrations(
        nomifun_agent_domain_support::c7_package_specs(),
    )?;
    registrations.push(super::nomi_core_tool_discovery::registration()?);

    let wave1 = super::agent_wave1_host::wave1_registrations_for_nomi_core(
        Arc::clone(&services.companion_service),
        services.database.pool().clone(),
    )?;
    replace_package_registrations(&mut registrations, wave1);
    #[cfg(feature = "browser-use")]
    if let Some(provider) = services.system_browser.as_ref() {
        bind_system_browser(
            &mut registrations,
            nomifun_browser_platform::system_browser::SystemBrowserHost::binding(
                provider.as_ref(),
            ),
        )?;
    }
    #[cfg(feature = "browser-use")]
    if let Some(provider) = services.local_web_search.as_ref() {
        for registration in &mut registrations {
            let mut manifest = registration.metadata.manifest.payload.clone();
            if let Some(capability) = manifest
                .contributions
                .capabilities
                .iter_mut()
                .find(|capability| {
                    capability.id.as_ref() == nomifun_ai_agent::local_web_search::TOOL_NAME
                })
            {
                capability
                    .config_schema
                    .0
                    .as_object_mut()
                    .ok_or_else(|| anyhow::anyhow!("Local search schema must be an object"))?
                    .insert(
                        nomifun_ai_agent::local_web_search::BINDING_ANNOTATION.into(),
                        serde_json::to_value(provider.binding())?,
                    );
                registration.metadata.manifest =
                    nomifun_agent_contracts::ArtifactEnvelope::new(manifest)?;
            }
        }
    }
    let wave1_tools = [
        nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
        nomifun_agent_domain_wave1::MEMORY_COMPANION_MERGE,
        nomifun_agent_domain_wave1::MEMORY_COMPANION_EVOLVE,
    ]
    .into_iter()
    .map(CapabilityId::from)
    .collect::<BTreeSet<_>>();
    let wave1_context = BTreeSet::from([CapabilityId::from(
        nomifun_agent_domain_wave1::MEMORY_COMPANION_RECALL,
    )]);

    let wave2_owner = super::nomi_core_wave2::action_host_port(services, effect_store);
    let wave2_ports =
        nomifun_agent_domain_wave2::Wave2RoleHostPorts::with_actions(wave2_owner.clone());
    #[cfg(feature = "browser-use")]
    let wave2_ports = {
        let mut ports = wave2_ports;
        if let Some(runtime) = services.headless_render.as_ref() {
            ports.browser_operation_tools = super::knowledge_browser::RenderRoleHost::new(
                runtime.clone(),
                services.authoritative_user_id.as_ref(),
            );
        }
        ports
    };
    let wave2 = nomifun_agent_domain_wave2::registrations_with_role_host_ports(wave2_ports)
        .map_err(anyhow::Error::msg)?;
    #[cfg(feature = "browser-use")]
    let wave2 = {
        let mut registrations = wave2;
        if let Some(runtime) = services.headless_render.as_ref()
            && let Some(browser) = registrations.iter_mut().find(|registration| {
                registration.metadata.manifest.payload.package_id.as_ref()
                    == nomifun_agent_domain_wave2::BROWSER_PACKAGE_ID
            })
        {
            let digest = nomifun_agent_contracts::digest_payload(&runtime.binding())?;
            browser.metadata.source.source_digest = Some(digest.clone());
            browser.metadata.context.source.source_digest = Some(digest);
        }
        registrations
    };
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
        .chain(
            [
                nomifun_agent_domain_wave4::CUSTOMER_SERVICE_NOTES_READ,
                nomifun_agent_domain_wave4::CUSTOMER_SERVICE_NOTES_WRITE,
                nomifun_agent_domain_wave4::CUSTOMER_SERVICE_HANDOFF,
            ]
            .into_iter()
            .map(CapabilityId::from),
        )
        .collect::<BTreeSet<_>>();
    let mut wave4_context =
        super::nomi_core_wave4::nomi_core_wave4_context_capability_ids();
    let mut lifecycle_capability_ids =
        super::nomi_core_wave4::nomi_core_wave4_lifecycle_capability_ids()
            .into_iter()
            .chain([CapabilityId::from(
                nomifun_agent_domain_wave4::CUSTOMER_SERVICE_DIALOGUE,
            )])
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
            customer_service: customer_service_owner,
            robot: robot_owner.clone(),
        });

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
    ])?;
    let tool_capability_ids = wave1_tools
        .into_iter()
        .chain(wave2_tools)
        .chain(wave3_tools)
        .chain(wave4_tools)
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
