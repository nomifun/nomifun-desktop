//! Host-owned selection of native Conversation browsers. Background work never owns the native surface.

use futures_util::FutureExt;
use nomifun_browser_platform::{
    runtime::{BrowserProfile, BrowserWorkspaceKey},
    workspace::BrowserWorkspaceService,
};
use nomifun_db::IConversationRepository;
use std::{path::PathBuf, sync::Arc};

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
    validate_native_provider(&installed.provider)?;
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
}

pub(crate) fn resolver(
    workspaces: Option<Arc<BrowserWorkspaceService>>,
    pool: nomifun_db::SqlitePool,
    execution: Arc<dyn nomifun_conversation::ExecutionConversationBoundary>,
    data_dir: PathBuf,
    owner: Arc<str>,
) -> nomifun_ai_agent::factory::BrowserRuntimeResolver {
    let repository = Arc::new(nomifun_db::SqliteConversationRepository::new(pool));
    Arc::new(move |request| {
        let (workspaces, repository, execution, data_dir, owner) = (
            workspaces.clone(),
            repository.clone(),
            execution.clone(),
            data_dir.clone(),
            owner.clone(),
        );
        async move {
            let nomifun_ai_agent::factory::BrowserRuntimeRequest {
                user_id,
                conversation_id,
                temporary,
                selected,
                provider,
            } = request;
            if user_id != owner.as_ref() {
                return Err(nomifun_common::AppError::NotFound("Conversation not found.".into()));
            }
            let row = repository
                .get(&conversation_id)
                .await?
                .filter(|row| row.user_id == user_id)
                .ok_or_else(|| {
                    nomifun_common::AppError::NotFound("Conversation not found.".into())
                })?;
            if !selected {
                return Ok(None);
            }
            if row.cron_job_id.is_some()
                || row
                    .source
                    .as_deref()
                    .is_some_and(|source| source != "nomifun")
            {
                return Err(nomifun_common::AppError::UnprocessableEntity(
                    "Browser automation is unavailable for this background conversation.".into(),
                ));
            }
            let projection = execution.projection(&user_id, &conversation_id).await?;
            if projection.execution_step_id.is_some() {
                return Err(nomifun_common::AppError::UnprocessableEntity(
                    "Browser automation is unavailable for this background conversation.".into(),
                ));
            }
            let Some(workspaces) = workspaces else {
                return Err(nomifun_common::AppError::UnprocessableEntity(
                    "No native browser host is available for this interactive conversation.".into(),
                ));
            };
            let provider = provider.ok_or_else(|| {
                nomifun_common::AppError::UnprocessableEntity(
                    "The selected Browser capability has no exact v2 Provider binding.".into(),
                )
            })?;
            validate_native_provider(&provider)?;
            let key = BrowserWorkspaceKey {
                user_id,
                conversation_id,
            };
            let profile = BrowserProfile::for_conversation(&data_dir, &key, temporary);
            let lock = serde_json::to_string(&provider)
                .map_err(|error| nomifun_common::AppError::Internal(error.to_string()))?;
            let workspace = workspaces.ensure(key, lock, profile).await;
            workspace
                .map(Some)
                .map_err(|error| nomifun_common::AppError::Conflict(error.to_string()))
        }
        .boxed()
    })
}

fn validate_native_provider(
    provider: &nomifun_agent_contracts::ExactRoleProviderRef,
) -> Result<(), nomifun_common::AppError> {
    if provider.role.key.role_id.as_ref() != nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID
        || provider.role.key.contract_version.as_ref()
            != nomifun_agent_domain_wave2::BROWSER_ROLE_CONTRACT_VERSION
        || provider.mount_id.as_ref() != nomifun_agent_domain_wave2::BROWSER_MOUNT_ID
        || provider.package.id.as_ref() != nomifun_agent_domain_wave2::BROWSER_PACKAGE_ID
    {
        return Err(nomifun_common::AppError::UnprocessableEntity(
            "The selected Browser Provider cannot operate this native workspace.".into(),
        ));
    }
    Ok(())
}
