use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock, Weak};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    AGENT_CORE_PACKAGE_ID, AGENT_SESSION_SERVICE_VERSION, AgentSessionId,
    ArtifactEnvelope, CancellationDescriptor,
    DeclaredServiceViewDescriptor, DeleteAgentSessionCommand, HostPortId, HostPortRef,
    IdempotencyKey, InProcessEntrypointMetadata, LocalizedMetadata,
    ManagedTaskRegistrationDescriptor, OperationId, PackageContributions,
    PackageManifest, PluginBootCriticality,
    PluginBootState, PluginContextDescriptor, PluginDesiredState,
    PluginEffectiveState, PluginIdentityDescriptor,
    PluginRegistrarDescriptor, PluginRegistrarOperation,
    PluginRegistrationMetadata, PluginSourceKind, PluginSourceMetadata,
    PluginStateHandleDescriptor, PluginStateMethod, PrincipalRef, ScopeKey,
    ServiceProvision, SessionEventAppend, SessionEventCursor, StrictJsonValue,
    ValidatedPluginConfig, VersionString, agent_core_mount_id,
    agent_core_package_ref, agent_session_command_service_ref,
    agent_session_query_service_ref, digest_payload,
};
use nomifun_agent_kernel::{
    ActivationOutcome, PluginRegistration, ServiceKey,
};
use nomifun_agent_session::{
    DeleteResult, ForkRequest, ForkResult, SessionCreateResult,
    SessionEventAppendResult, SessionEventPage, SessionHeadProjection,
    SessionObservation, SessionRehydrationInput,
};

use crate::{
    ActivateCapabilityRequest, AgentPlatform, AgentPlatformError,
    AgentSessionCommandPort, AgentSessionDeletePort, AgentSessionQueryPort,
    AgentTurnDispatch, InvokeCapabilityCommand, OpenAgentSessionRequest,
    StartAgentTurnRequest,
};

const PLUGIN_CANCEL_PORT: &str = "host.plugin.cancel";
const PLUGIN_TASKS_PORT: &str = "host.plugin.tasks";

#[async_trait]
pub trait CanonicalAgentSessionCommandPort:
    AgentSessionCommandPort + AgentSessionDeletePort
{
    async fn cancel_turn(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        target_operation_id: OperationId,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionEventAppendResult, AgentPlatformError>;
}

pub fn agent_session_command_service_key(
) -> ServiceKey<dyn CanonicalAgentSessionCommandPort> {
    ServiceKey::new(
        agent_session_command_service_ref().id,
        AGENT_SESSION_SERVICE_VERSION,
    )
}

pub fn agent_session_query_service_key(
) -> ServiceKey<dyn AgentSessionQueryPort> {
    ServiceKey::new(
        agent_session_query_service_ref().id,
        AGENT_SESSION_SERVICE_VERSION,
    )
}

#[derive(Default)]
pub(crate) struct AgentSessionServiceProxy {
    platform: OnceLock<Weak<AgentPlatform>>,
}

impl AgentSessionServiceProxy {
    pub(crate) fn bind(
        &self,
        platform: &Arc<AgentPlatform>,
    ) -> Result<(), AgentPlatformError> {
        self.platform
            .set(Arc::downgrade(platform))
            .map_err(|_| {
                AgentPlatformError::Contract(
                    "AgentSession ServiceKey proxy was bound more than once"
                        .to_owned(),
                )
            })
    }

    fn platform(&self) -> Result<Arc<AgentPlatform>, AgentPlatformError> {
        self.platform
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| {
                AgentPlatformError::Contract(
                    "AgentSession ServiceKey provider is unavailable"
                        .to_owned(),
                )
            })
    }
}

#[async_trait]
impl AgentSessionCommandPort for AgentSessionServiceProxy {
    async fn open_session(
        &self,
        request: OpenAgentSessionRequest,
    ) -> Result<SessionCreateResult, AgentPlatformError> {
        AgentSessionCommandPort::open_session(
            self.platform()?.as_ref(),
            request,
        )
        .await
    }

    async fn append_event(
        &self,
        append: &SessionEventAppend,
    ) -> Result<SessionEventAppendResult, AgentPlatformError> {
        AgentSessionCommandPort::append_event(
            self.platform()?.as_ref(),
            append,
        )
        .await
    }

    async fn start_turn(
        &self,
        request: StartAgentTurnRequest,
    ) -> Result<AgentTurnDispatch, AgentPlatformError> {
        AgentSessionCommandPort::start_turn(
            self.platform()?.as_ref(),
            request,
        )
        .await
    }

    async fn activate_capability(
        &self,
        request: ActivateCapabilityRequest,
    ) -> Result<ActivationOutcome, AgentPlatformError> {
        AgentSessionCommandPort::activate_capability(
            self.platform()?.as_ref(),
            request,
        )
        .await
    }

    async fn invoke_capability(
        &self,
        command: InvokeCapabilityCommand,
    ) -> Result<StrictJsonValue, AgentPlatformError> {
        AgentSessionCommandPort::invoke_capability(
            self.platform()?.as_ref(),
            command,
        )
        .await
    }

    async fn fork_session(
        &self,
        parent_session_id: &AgentSessionId,
        request: ForkRequest,
    ) -> Result<ForkResult, AgentPlatformError> {
        AgentSessionCommandPort::fork_session(
            self.platform()?.as_ref(),
            parent_session_id,
            request,
        )
        .await
    }
}

#[async_trait]
impl AgentSessionQueryPort for AgentSessionServiceProxy {
    async fn observe_session(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionObservation, AgentPlatformError> {
        AgentSessionQueryPort::observe_session(
            self.platform()?.as_ref(),
            principal,
            session_id,
            after,
            limit,
        )
        .await
    }

    async fn session_head(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<SessionHeadProjection, AgentPlatformError> {
        AgentSessionQueryPort::session_head(
            self.platform()?.as_ref(),
            principal,
            session_id,
        )
        .await
    }

    async fn session_events(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionEventPage, AgentPlatformError> {
        AgentSessionQueryPort::session_events(
            self.platform()?.as_ref(),
            principal,
            session_id,
            after,
            limit,
        )
        .await
    }

    async fn rehydration_input(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<SessionRehydrationInput, AgentPlatformError> {
        AgentSessionQueryPort::rehydration_input(
            self.platform()?.as_ref(),
            principal,
            session_id,
        )
        .await
    }
}

#[async_trait]
impl AgentSessionDeletePort for AgentSessionServiceProxy {
    async fn delete_session(
        &self,
        command: DeleteAgentSessionCommand,
        deleted_at: i64,
    ) -> Result<DeleteResult, AgentPlatformError> {
        AgentSessionDeletePort::delete_session(
            self.platform()?.as_ref(),
            command,
            deleted_at,
        )
        .await
    }
}

#[async_trait]
impl CanonicalAgentSessionCommandPort for AgentSessionServiceProxy {
    async fn cancel_turn(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        target_operation_id: OperationId,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionEventAppendResult, AgentPlatformError> {
        self.platform()?
            .cancel_turn(
                principal,
                session_id,
                target_operation_id,
                idempotency_key,
            )
            .await
    }
}

#[async_trait]
impl CanonicalAgentSessionCommandPort for AgentPlatform {
    async fn cancel_turn(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        target_operation_id: OperationId,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionEventAppendResult, AgentPlatformError> {
        AgentPlatform::cancel_turn(
            self,
            principal,
            session_id,
            target_operation_id,
            idempotency_key,
        )
        .await
    }
}

pub(crate) fn agent_session_service_registration(
) -> Result<
    (PluginRegistration, Arc<AgentSessionServiceProxy>),
    AgentPlatformError,
> {
    let package = agent_core_package_ref();
    let mount_id = agent_core_mount_id();
    let command_key = agent_session_command_service_key();
    let query_key = agent_session_query_service_key();
    let command_ref = command_key.reference().clone();
    let query_ref = query_key.reference().clone();
    let config_schema = StrictJsonValue(serde_json::json!({
        "type": "object",
        "additionalProperties": false
    }));
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: AGENT_CORE_PACKAGE_ID.to_owned(),
        source_digest: None,
    };
    let cancel_port = host_port(PLUGIN_CANCEL_PORT);
    let task_port = host_port(PLUGIN_TASKS_PORT);
    let manifest = PackageManifest {
        schema_version: VersionString::from(AGENT_SESSION_SERVICE_VERSION),
        host_contract_version: VersionString::from(AGENT_SESSION_SERVICE_VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display(
            "Agent Session Core",
            "Canonical command and query services for one AgentPlatform generation.",
        ),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: vec![
            ServiceProvision {
                service: command_ref.clone(),
            },
            ServiceProvision {
                service: query_ref.clone(),
            },
        ],
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: "platform.agent-core.entrypoint".to_owned(),
            contract_version: VersionString::from(AGENT_SESSION_SERVICE_VERSION),
        },
        contributions: PackageContributions::default(),
    };
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: mount_id.clone(),
    };
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest)?,
        mount_id: mount_id.clone(),
        source: source.clone(),
        boot_state: PluginBootState {
            criticality: PluginBootCriticality::Required,
            desired_state: PluginDesiredState::Enabled,
            effective_state: PluginEffectiveState::Active,
            diagnostic_code: None,
        },
        registrar: PluginRegistrarDescriptor {
            identity: identity.clone(),
            allowed_operations: BTreeSet::from([
                PluginRegistrarOperation::ProvideService,
                PluginRegistrarOperation::BindHostPort,
            ]),
            declared_capability_ids: BTreeSet::new(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_service_keys: BTreeSet::from([
                command_ref.id.clone(),
                query_ref.id.clone(),
            ]),
            declared_host_ports: BTreeSet::from([
                cancel_port.id.clone(),
                task_port.id.clone(),
            ]),
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema.0)?,
                config_revision: 1,
                value: StrictJsonValue(serde_json::json!({})),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: mount_id.clone(),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor {
                provided_services: vec![
                    command_ref,
                    query_ref,
                ],
                required_service_handles: Vec::new(),
            },
            host_ports: Vec::new(),
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port: cancel_port,
                scope_key: ScopeKey::from(format!(
                    "mount:{}",
                    mount_id.as_ref()
                )),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from(format!(
                    "mount:{}",
                    mount_id.as_ref()
                )),
            },
        },
    };
    let proxy = Arc::new(AgentSessionServiceProxy::default());
    let mut registration = PluginRegistration::new(metadata);
    registration.provide_service(
        &command_key,
        Arc::clone(&proxy) as Arc<dyn CanonicalAgentSessionCommandPort>,
    )?;
    registration.provide_service(
        &query_key,
        Arc::clone(&proxy) as Arc<dyn AgentSessionQueryPort>,
    )?;
    Ok((registration, proxy))
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(AGENT_SESSION_SERVICE_VERSION),
    }
}

#[cfg(test)]
mod tests {
    use nomifun_agent_kernel::{MaterializationPolicy, Materializer};

    use super::*;

    #[test]
    fn agent_core_registration_materializes_exact_service_keys() {
        let (registration, _proxy) =
            agent_session_service_registration().unwrap();
        assert_eq!(
            registration.service_refs(),
            BTreeSet::from([
                agent_session_command_service_key()
                    .reference()
                    .clone(),
                agent_session_query_service_key().reference().clone(),
            ])
        );
        let materialized = Materializer::materialize(
            &MaterializationPolicy::stable(AGENT_SESSION_SERVICE_VERSION),
            &[registration],
            1,
        )
        .unwrap();
        let node = materialized
            .service_dag
            .nodes
            .iter()
            .find(|node| node.package == agent_core_package_ref())
            .expect("agent-core service node");
        assert_eq!(node.mount_id, agent_core_mount_id());
        assert_eq!(node.provides.len(), 2);
        assert!(node.requires.is_empty());
    }

    #[tokio::test]
    async fn unbound_agent_session_service_proxy_fails_closed() {
        let proxy = AgentSessionServiceProxy::default();
        let error = AgentSessionQueryPort::session_head(
            &proxy,
            &PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "owner".to_owned(),
            },
            &AgentSessionId::from("session"),
        )
        .await
        .expect_err("an unbound service proxy must fail closed");
        assert!(
            error
                .to_string()
                .contains("ServiceKey provider is unavailable")
        );
    }
}
