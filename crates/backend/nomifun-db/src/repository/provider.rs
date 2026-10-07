use crate::error::DbError;
use crate::models::{NewProviderModel, Provider, ProviderModelRow, UpsertProviderConnectionParams};

#[async_trait::async_trait]
pub trait IProviderRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<Provider>, DbError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Provider>, DbError>;

    /// Create a provider and its first fully configured model in one
    /// transaction. A provider cannot exist in a half-configured state.
    async fn create(
        &self,
        params: CreateProviderParams<'_>,
        initial_model: &NewProviderModel<'_>,
        connections: &[UpsertProviderConnectionParams<'_>],
    ) -> Result<(Provider, ProviderModelRow), DbError>;

    /// Create the provider, selected models, and named connections as one
    /// graph. Each display name corresponds to the model at the same index.
    async fn create_graph(
        &self,
        _params: CreateProviderParams<'_>,
        _models: &[NewProviderModel<'_>],
        _display_names: &[Option<String>],
        _connections: &[UpsertProviderConnectionParams<'_>],
    ) -> Result<(Provider, Vec<ProviderModelRow>), DbError> {
        Err(DbError::Conflict("atomic provider graph creation is unsupported".into()))
    }

    async fn update(
        &self,
        id: &str,
        expected_config_revision: i64,
        params: UpdateProviderParams<'_>,
    ) -> Result<Provider, DbError>;

    /// Update the default connection and any supplied named connections in
    /// one revision-fenced transaction. Unmentioned roles are preserved.
    async fn update_with_connections(
        &self,
        _id: &str,
        _expected_config_revision: i64,
        _params: UpdateProviderParams<'_>,
        _connections: &[UpsertProviderConnectionParams<'_>],
    ) -> Result<Provider, DbError> {
        Err(DbError::Conflict("atomic provider connection update is unsupported".into()))
    }

    /// Atomically upsert a selected model set without deleting unselected
    /// models. The provider revision fences configuration and lifecycle races.
    async fn save_graph_models(
        &self,
        _provider_id: &str,
        _expected_config_revision: i64,
        _models: &[NewProviderModel<'_>],
        _display_names: &[Option<String>],
    ) -> Result<Vec<ProviderModelRow>, DbError> {
        Err(DbError::Conflict("atomic provider model synchronization is unsupported".into()))
    }

    /// Clone provider metadata, models, capabilities, and named connections in
    /// one transaction. Capability health observations are not copied.
    async fn clone_graph(
        &self,
        source_provider_id: &str,
        clone_name: &str,
    ) -> Result<Provider, DbError>;

    async fn delete(&self, id: &str) -> Result<(), DbError>;
}

#[derive(Debug)]
pub struct CreateProviderParams<'a> {
    pub provider_id: Option<&'a str>,
    pub platform: &'a str,
    pub name: &'a str,
    pub base_url: &'a str,
    pub auth_scheme: &'a str,
    pub credentials_encrypted: &'a str,
    pub enabled: bool,
    pub bedrock_config: Option<&'a str>,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Default)]
pub struct UpdateProviderParams<'a> {
    pub name: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub auth_scheme: Option<&'a str>,
    pub credentials_encrypted: Option<&'a str>,
    pub enabled: Option<bool>,
    pub bedrock_config: Option<Option<&'a str>>,
    pub sort_order: Option<i64>,
}
