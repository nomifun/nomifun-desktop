use nomifun_agent_contracts::{
    ExecutionRoleId, InstallationRoleBinding, RoleProviderSelection, UserId,
};
use std::collections::BTreeMap;

use crate::ControlPlaneError;

/// Host-owned installation binding storage. Reading never filters withdrawn
/// Providers: the user's selection remains a fact even when unavailable.
#[async_trait::async_trait]
pub trait InstallationRoleBindingStore: Send + Sync {
    async fn load(
        &self,
        owner: &UserId,
    ) -> Result<BTreeMap<ExecutionRoleId, InstallationRoleBinding>, ControlPlaneError>;
    async fn put(
        &self,
        owner: &UserId,
        selection: RoleProviderSelection,
        expected_version: u64,
    ) -> Result<InstallationRoleBinding, ControlPlaneError>;
}
