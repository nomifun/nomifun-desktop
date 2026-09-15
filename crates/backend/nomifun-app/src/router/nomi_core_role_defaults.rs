use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    ExecutionRoleId, InstallationRoleBinding, RoleProviderSelection, UserId,
};
use nomifun_agent_control_plane::{ControlPlaneError, InstallationRoleBindingStore};
use nomifun_db::{DbError, SqlitePool};

pub(crate) struct NomiCoreRoleBindingStore {
    pool: SqlitePool,
}

impl NomiCoreRoleBindingStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
        nomifun_db::load_installation_role_bindings(&self.pool)
            .await
            .map_err(db_error)
    }

    async fn put(
        &self,
        owner: &UserId,
        selection: RoleProviderSelection,
        expected_version: u64,
    ) -> Result<InstallationRoleBinding, ControlPlaneError> {
        self.require_owner(owner).await?;
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
