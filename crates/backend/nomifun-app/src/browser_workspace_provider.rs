//! Host-owned Browser Provider selection and AgentSession Resource binding.
//!
//! Provider availability is implementation metadata only. Exact Browser
//! Action authority and the Browser Resource binding arrive independently
//! from the frozen AgentSession Snapshot.

use futures_util::FutureExt;
use nomifun_browser_platform::{
    bound_resource::BoundBrowserProviderResource,
    product::{
        BrowserCapabilityAction, BrowserProviderDescriptor, BrowserProviderKind,
        BrowserResourceBinding, BrowserSessionAuthority, BROWSER_RESOURCE_KIND,
    },
    runtime::{BrowserProfilePersistence, BrowserProfileStore},
    attached_browser::{
        AttachedBrowserProviderHost, AttachedBrowserRuntimeError,
        AuthorizedAttachedBrowserResource,
    },
    workspace::BrowserResourceService,
};
use std::{path::PathBuf, sync::Arc};

#[path = "browser_workspace_provider/attached_provider.rs"]
pub mod attached_provider;

struct CanonicalAttachedBrowserSessionVerifier {
    sessions: nomifun_conversation::CanonicalAgentSessionOwner,
}

#[async_trait::async_trait]
impl attached_provider::AttachedBrowserSessionVerifier
    for CanonicalAttachedBrowserSessionVerifier
{
    async fn verify(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), AttachedBrowserRuntimeError> {
        let principal = nomifun_agent_contracts::PrincipalRef {
            principal_kind: "user".into(),
            principal_id: principal_id.to_owned(),
        };
        let session_id = nomifun_agent_contracts::AgentSessionId::from(
            agent_session_id.to_owned(),
        );
        self.sessions
            .get(&principal, &session_id)
            .await
            .map_err(|_| AttachedBrowserRuntimeError::ActionDenied)?;
        let active = self
            .sessions
            .active_capability_ids(&principal, &session_id)
            .await
            .map_err(|_| AttachedBrowserRuntimeError::ActionDenied)?;
        if !active
            .iter()
            .any(|capability| capability == nomifun_browser_platform::product::BROWSER_MODULE_ID)
        {
            return Err(AttachedBrowserRuntimeError::ActionDenied);
        }
        Ok(())
    }
}

pub(crate) fn install_attached_session_verifier(
    service: &attached_provider::AttachedChromeProviderService,
    sessions: nomifun_conversation::CanonicalAgentSessionOwner,
) -> anyhow::Result<()> {
    service
        .install_session_verifier(Arc::new(CanonicalAttachedBrowserSessionVerifier {
            sessions,
        }))
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

/// The desktop host selects its installed native provider before compilation.
/// Derive the exact contract digest from the materialized registry; do not make
/// users configure internal Role bindings or persist a product setting.
pub(crate) fn installation_binding(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
    native_available: bool,
) -> anyhow::Result<
    std::collections::BTreeMap<
        nomifun_agent_contracts::ExecutionRoleId,
        nomifun_agent_contracts::InstallationRoleBinding,
    >,
> {
    use nomifun_agent_contracts::{
        InstallationRoleBinding, PluginSourceKind, RoleProviderSelection,
    };
    if !native_available {
        return Ok(Default::default());
    }
    let role_id = nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into();
    let mount_id = nomifun_agent_domain_wave2::BROWSER_MOUNT_ID.into();
    let installed = registry.role_provider(&role_id, &mount_id).ok_or_else(|| {
        anyhow::anyhow!("Native Browser Role Provider is missing from the host registry")
    })?;
    validate_browser_provider(&installed.provider)?;
    anyhow::ensure!(
        installed.source.source_kind == PluginSourceKind::Bundled,
        "Native Browser Provider must be bundled"
    );
    Ok(std::collections::BTreeMap::from([(
        role_id,
        InstallationRoleBinding {
            selection: RoleProviderSelection {
                role: installed.provider.role.clone(),
                provider_mount_id: installed.provider.mount_id.clone(),
            },
            binding_version: 1,
            // This is immutable boot composition, not a database migration/update.
            updated_at_ms: 0,
        },
    )]))
}

#[cfg(test)]
mod binding_tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding,
    };

    use super::*;
    #[test]
    fn native_browser_binding_uses_the_materialized_v2_contract_only_when_owned() {
        let registrations = nomifun_agent_domain_wave2::registrations().unwrap();
        let registry = nomifun_agent_kernel::Materializer::materialize(
            &nomifun_agent_kernel::MaterializationPolicy::stable("1.0.0"),
            &registrations,
            1,
        )
        .unwrap();
        assert!(installation_binding(&registry, false).unwrap().is_empty());
        let binding = installation_binding(&registry, true).unwrap();
        assert_eq!(binding.len(), 1);
        let binding = &binding[&nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into()];
        let contract = registry
            .role_contract(&nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into())
            .unwrap();
        assert_eq!(
            binding.selection.role.contract_digest,
            contract.contract_digest
        );
        assert_eq!(
            binding.selection.role.key.contract_version.as_ref(),
            "2.0.0"
        );
        let empty = nomifun_agent_kernel::Materializer::materialize(
            &nomifun_agent_kernel::MaterializationPolicy::stable("1.0.0"),
            &[],
            1,
        )
        .unwrap();
        assert!(installation_binding(&empty, true).is_err());
    }

    #[test]
    fn provider_and_resource_availability_do_not_create_action_authority() {
        let registrations = nomifun_agent_domain_wave2::registrations().unwrap();
        let registry = nomifun_agent_kernel::Materializer::materialize(
            &nomifun_agent_kernel::MaterializationPolicy::stable("1.0.0"),
            &registrations,
            1,
        )
        .unwrap();
        let installed = registry
            .role_provider(
                &nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into(),
                &nomifun_agent_domain_wave2::BROWSER_MOUNT_ID.into(),
            )
            .unwrap();
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("browser-binding"),
            resource_kind: ResourceKind::from(BROWSER_RESOURCE_KIND),
            resource_id: ResourceId::from("browser-resource"),
            owner_id: "alice".into(),
            operations: BTreeSet::from(["observe".into(), "navigate".into()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                "provider_kind".into(),
                "managed".into(),
            )]),
        };
        let descriptor = provider_descriptor(&installed.provider, &binding).unwrap();
        let resource = browser_resource_binding(binding, descriptor).unwrap();
        let authority = BrowserSessionAuthority::from_action_ids(
            "alice",
            "delegated-agent-session",
            std::iter::empty::<&str>(),
            resource,
        )
        .unwrap();
        assert_eq!(
            authority.authorize(BrowserCapabilityAction::Observe),
            Err(nomifun_browser_platform::runtime::WorkspaceError::ActionDenied)
        );
    }

    #[test]
    fn browser_resource_rejects_web_research_operations() {
        let registrations = nomifun_agent_domain_wave2::registrations().unwrap();
        let registry = nomifun_agent_kernel::Materializer::materialize(
            &nomifun_agent_kernel::MaterializationPolicy::stable("1.0.0"),
            &registrations,
            1,
        )
        .unwrap();
        let installed = registry
            .role_provider(
                &nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into(),
                &nomifun_agent_domain_wave2::BROWSER_MOUNT_ID.into(),
            )
            .unwrap();
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("browser-binding"),
            resource_kind: ResourceKind::from(BROWSER_RESOURCE_KIND),
            resource_id: ResourceId::from("browser-resource"),
            owner_id: "alice".into(),
            operations: BTreeSet::from(["search".into()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let descriptor = provider_descriptor(&installed.provider, &binding).unwrap();
        assert!(browser_resource_binding(binding, descriptor).is_err());
    }

    #[test]
    fn browser_profile_lifetime_is_frozen_by_the_resource_binding() {
        let mut binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("browser-binding"),
            resource_kind: ResourceKind::from(BROWSER_RESOURCE_KIND),
            resource_id: ResourceId::from("browser-resource"),
            owner_id: "alice".into(),
            operations: BTreeSet::from(["observe".into()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        assert!(!browser_resource_ephemeral(&binding).unwrap());
        binding
            .typed_parameters
            .insert("persistence".into(), "ephemeral".into());
        assert!(browser_resource_ephemeral(&binding).unwrap());
        binding
            .typed_parameters
            .insert("persistence".into(), "temporary-ish".into());
        assert!(browser_resource_ephemeral(&binding).is_err());
    }
}

pub(crate) fn resolver(
    resources: Option<Arc<BrowserResourceService>>,
    attached_chrome: Option<Arc<attached_provider::AttachedChromeProviderService>>,
    data_dir: PathBuf,
    owner: Arc<str>,
) -> nomifun_ai_agent::factory::BrowserRuntimeResolver {
    Arc::new(move |request| {
        let (resources, attached_chrome, data_dir, owner) = (
            resources.clone(),
            attached_chrome.clone(),
            data_dir.clone(),
            owner.clone(),
        );
        async move {
            let nomifun_ai_agent::factory::BrowserRuntimeRequest {
                principal_id,
                agent_session_id,
                action_allowlist,
                provider,
                resource_binding,
            } = request;
            if principal_id != owner.as_ref() {
                return Err(nomifun_common::AppError::NotFound(
                    "AgentSession not found.".into(),
                ));
            }
            if action_allowlist.is_empty() {
                return Ok(None);
            }
            let provider = provider.ok_or_else(|| {
                nomifun_common::AppError::UnprocessableEntity(
                    "The Browser Module has no exact Provider binding.".into(),
                )
            })?;
            validate_browser_provider(&provider)?;
            let binding = resource_binding.ok_or_else(|| {
                nomifun_common::AppError::UnprocessableEntity(
                    "The Browser Module has no exact Browser Resource binding.".into(),
                )
            })?;
            let ephemeral = browser_resource_ephemeral(&binding)?;
            let provider = provider_descriptor(&provider, &binding)?;
            let resource = browser_resource_binding(binding, provider)?;
            let authority = BrowserSessionAuthority::from_action_ids(
                principal_id,
                agent_session_id,
                action_allowlist.iter().map(|action| action.as_ref()),
                resource,
            )
            .map_err(|error| {
                nomifun_common::AppError::UnprocessableEntity(error.to_string())
            })?;
            bind_authorized_resource(
                resources,
                attached_chrome,
                &data_dir,
                authority,
                ephemeral,
            )
            .await
            .map(Some)
        }
        .boxed()
    })
}

/// Materialize the exact provider selected by an already-authorized canonical
/// AgentSession. Both the Agent runtime resolver and Browser REST route use this
/// function, so neither surface can silently fall back to the managed provider.
pub(crate) async fn bind_authorized_resource(
    resources: Option<Arc<BrowserResourceService>>,
    attached_chrome: Option<Arc<attached_provider::AttachedChromeProviderService>>,
    data_dir: &std::path::Path,
    authority: BrowserSessionAuthority,
    ephemeral: bool,
) -> Result<BoundBrowserProviderResource, nomifun_common::AppError> {
    match authority.resource().provider().kind() {
        BrowserProviderKind::Managed => {
            let resources = resources.ok_or_else(|| {
                nomifun_common::AppError::UnprocessableEntity(
                    "The managed Browser Provider is unavailable on this host.".into(),
                )
            })?;
            let profile = BrowserProfileStore::new(data_dir.to_path_buf())
                .and_then(|store| {
                    store.profile_for(
                        &authority.key(),
                        if ephemeral {
                            BrowserProfilePersistence::Ephemeral
                        } else {
                            BrowserProfilePersistence::Persistent
                        },
                    )
                })
                .map_err(|error| {
                    nomifun_common::AppError::Conflict(error.to_string())
                })?;
            resources
                .ensure(authority, profile)
                .await
                .map(BoundBrowserProviderResource::Managed)
                .map_err(|error| nomifun_common::AppError::Conflict(error.to_string()))
        }
        BrowserProviderKind::AttachedChrome => {
            let attached_chrome = attached_chrome.ok_or_else(|| {
                nomifun_common::AppError::UnprocessableEntity(
                    "The attached Chrome Provider is unavailable on this host.".into(),
                )
            })?;
            let inner = attached_chrome
                .resource(authority.principal_id(), authority.agent_session_id())
                .await
                .map_err(|error| nomifun_common::AppError::Conflict(error.to_string()))?;
            AuthorizedAttachedBrowserResource::new(authority, inner)
                .map(Arc::new)
                .map(BoundBrowserProviderResource::AttachedChrome)
                .map_err(|error| nomifun_common::AppError::Conflict(error.to_string()))
        }
    }
}

fn validate_browser_provider(
    provider: &nomifun_agent_contracts::ExactRoleProviderRef,
) -> Result<(), nomifun_common::AppError> {
    if provider.role.key.role_id.as_ref() != nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID
        || provider.role.key.contract_version.as_ref()
            != nomifun_agent_domain_wave2::BROWSER_ROLE_CONTRACT_VERSION
    {
        return Err(nomifun_common::AppError::UnprocessableEntity(
            "The selected Browser Provider does not implement the canonical Browser Role."
                .into(),
        ));
    }
    Ok(())
}

pub(crate) fn provider_descriptor(
    provider: &nomifun_agent_contracts::ExactRoleProviderRef,
    binding: &nomifun_agent_contracts::TypedResourceBinding,
) -> Result<BrowserProviderDescriptor, nomifun_common::AppError> {
    let kind = match binding
        .typed_parameters
        .get("provider_kind")
        .map(String::as_str)
    {
        Some("attached_chrome") => BrowserProviderKind::AttachedChrome,
        Some("managed") | None => BrowserProviderKind::Managed,
        Some(_) => {
            return Err(nomifun_common::AppError::UnprocessableEntity(
                "Browser Resource selects an unknown Provider kind.".into(),
            ));
        }
    };
    let implemented_actions = match kind {
        BrowserProviderKind::Managed => BrowserCapabilityAction::all().to_vec(),
        BrowserProviderKind::AttachedChrome => vec![
            BrowserCapabilityAction::Observe,
            BrowserCapabilityAction::Navigate,
            BrowserCapabilityAction::Act,
        ],
    };
    let provider_kind_id = match kind {
        BrowserProviderKind::Managed => "managed",
        BrowserProviderKind::AttachedChrome => "attached_chrome",
    };
    let provider_id = format!(
        "{provider_kind_id}:{}:{}",
        provider.package.id.as_ref(),
        provider.mount_id.as_ref()
    );
    let immutable_identity = serde_json::to_string(provider)
        .map_err(|error| nomifun_common::AppError::Internal(error.to_string()))?;
    BrowserProviderDescriptor::new(provider_id, kind, immutable_identity, implemented_actions)
        .map_err(|error| nomifun_common::AppError::UnprocessableEntity(error.to_string()))
}

pub(crate) fn browser_resource_binding(
    binding: nomifun_agent_contracts::TypedResourceBinding,
    provider: BrowserProviderDescriptor,
) -> Result<BrowserResourceBinding, nomifun_common::AppError> {
    if binding.resource_kind.as_ref() != BROWSER_RESOURCE_KIND {
        return Err(nomifun_common::AppError::UnprocessableEntity(
            "Browser Module received a non-Browser Resource binding.".into(),
        ));
    }
    BrowserResourceBinding::from_operation_names(
        binding.binding_id.as_ref(),
        binding.resource_id.as_ref(),
        binding.owner_id,
        provider,
        binding.operations,
    )
    .map_err(|error| nomifun_common::AppError::UnprocessableEntity(error.to_string()))
}

pub(crate) fn browser_resource_ephemeral(
    binding: &nomifun_agent_contracts::TypedResourceBinding,
) -> Result<bool, nomifun_common::AppError> {
    match binding
        .typed_parameters
        .get("persistence")
        .map(String::as_str)
    {
        None | Some("persistent") => Ok(false),
        Some("ephemeral") => Ok(true),
        Some(_) => Err(nomifun_common::AppError::UnprocessableEntity(
            "Browser Resource has an invalid persistence policy.".into(),
        )),
    }
}
