use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    ExecutionRoleId, InstallationRoleBinding, RoleProviderSelection, UserId,
};
use nomifun_agent_control_plane::{ControlPlaneError, InstallationRoleBindingStore};
use nomifun_db::{DbError, SqlitePool};

pub(crate) struct NomiCoreRoleBindingStore {
    pool: SqlitePool,
    host_bindings: BTreeMap<ExecutionRoleId, InstallationRoleBinding>,
}

impl NomiCoreRoleBindingStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool, host_bindings: BTreeMap::new() }
    }

    /// Native Browser selection belongs to this boot's desktop host. Authoring
    /// reloads installation defaults before every compilation, so preserve the
    /// same materialized binding here without persisting an internal setting.
    pub(crate) fn with_host_bindings(
        mut self,
        bindings: BTreeMap<ExecutionRoleId, InstallationRoleBinding>,
    ) -> Self {
        self.host_bindings = bindings;
        self
    }

    async fn require_owner(&self, owner: &UserId) -> Result<(), ControlPlaneError> {
        let installation_owner = nomifun_db::installation_owner_id(&self.pool)
            .await
            .map_err(db_error)?;
        if installation_owner != owner.as_ref() {
            return Err(ControlPlaneError::canonical(
                "ROLE_DEFAULT_OWNER_REQUIRED",
                axum::http::StatusCode::FORBIDDEN,
                "only the installation owner can manage Role defaults",
            ));
        }
        Ok(())
    }
}

fn db_error(error: DbError) -> ControlPlaneError {
    match error {
        DbError::Conflict(message) => ControlPlaneError::canonical(
            "ROLE_DEFAULT_VERSION_CONFLICT",
            axum::http::StatusCode::CONFLICT,
            message,
        ),
        other => ControlPlaneError::Wire(other.to_string()),
    }
}

#[async_trait::async_trait]
impl InstallationRoleBindingStore for NomiCoreRoleBindingStore {
    async fn load(
        &self,
        owner: &UserId,
    ) -> Result<BTreeMap<ExecutionRoleId, InstallationRoleBinding>, ControlPlaneError> {
        self.require_owner(owner).await?;
        let mut bindings = nomifun_db::load_installation_role_bindings(&self.pool)
            .await
            .map_err(db_error)?;
        // A stored choice cannot invent native ownership on a host without a
        // BrowserWorkspaceService, nor override the current native contract.
        bindings.remove(&nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into());
        bindings.extend(self.host_bindings.clone());
        Ok(bindings)
    }

    async fn put(
        &self,
        owner: &UserId,
        selection: RoleProviderSelection,
        expected_version: u64,
    ) -> Result<InstallationRoleBinding, ControlPlaneError> {
        self.require_owner(owner).await?;
        if selection.role.key.role_id.as_ref() == nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID
            || self.host_bindings.contains_key(&selection.role.key.role_id)
        {
            return Err(ControlPlaneError::canonical(
                "ROLE_DEFAULT_HOST_OWNED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "this Role Provider is selected by the application host",
            ));
        }
        nomifun_db::put_installation_role_binding(
            &self.pool,
            selection,
            expected_version,
            nomifun_common::now_ms(),
        )
        .await
        .map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{ExactRoleContractRef, RoleContractKey};

    fn selection(role: &str, provider: &str) -> RoleProviderSelection {
        RoleProviderSelection {
            role: ExactRoleContractRef {
                key: RoleContractKey { role_id: role.into(), contract_version: "1.0.0".into() },
                contract_digest: "1".repeat(64).into(),
            },
            provider_mount_id: provider.into(),
        }
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn authoring_reload_preserves_exact_native_binding_without_persisting_it() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap().into();
        let registry = nomifun_agent_kernel::Materializer::materialize(
            &nomifun_agent_kernel::MaterializationPolicy::stable("1.0.0"),
            &nomifun_agent_domain_wave2::registrations().unwrap(),
            1,
        ).unwrap();
        let native = crate::browser_workspace_provider::installation_binding(&registry, true).unwrap();
        let store = NomiCoreRoleBindingStore::new(database.pool().clone()).with_host_bindings(native.clone());

        // This is the value authoring_compiler replaces its environment with.
        assert_eq!(store.load(&owner).await.unwrap(), native);
        assert!(nomifun_db::load_installation_role_bindings(database.pool()).await.unwrap().is_empty());

        // Even an externally persisted stale choice cannot replace the boot's
        // exact provider contract; no migration is needed to establish ownership.
        let browser_role = nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID;
        nomifun_db::put_installation_role_binding(database.pool(), selection(browser_role, "stale-browser"), 0, 1)
            .await.unwrap();
        assert_eq!(store.load(&owner).await.unwrap(), native);
        let error = store.put(&owner, native[&browser_role.into()].selection.clone(), 1).await.unwrap_err();
        assert_eq!(error.code().as_ref(), "ROLE_DEFAULT_HOST_OWNED");
    }

    #[tokio::test]
    async fn non_native_host_omits_browser_and_keeps_user_defaults_editable() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap().into();
        let store = NomiCoreRoleBindingStore::new(database.pool().clone());
        let browser = selection(nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID, "stale-browser");
        nomifun_db::put_installation_role_binding(database.pool(), browser.clone(), 0, 1).await.unwrap();
        assert!(store.load(&owner).await.unwrap().is_empty());
        assert_eq!(store.put(&owner, browser, 1).await.unwrap_err().code().as_ref(), "ROLE_DEFAULT_HOST_OWNED");

        let first = store.put(&owner, selection("test.user_role", "provider-one"), 0).await.unwrap();
        let second = store.put(&owner, selection("test.user_role", "provider-two"), first.binding_version).await.unwrap();
        let loaded = store.load(&owner).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[&"test.user_role".into()], second);
        assert_eq!(store.load(&"another-user".into()).await.unwrap_err().code().as_ref(), "ROLE_DEFAULT_OWNER_REQUIRED");
    }
}
