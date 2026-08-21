use std::path::PathBuf;
use std::sync::Arc;

use nomifun_db::{
    SqliteClientPreferenceRepository, SqliteProviderConnectionRepository,
    SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
    SqliteProviderRepository, SqliteSettingsRepository,
};
use nomifun_system::{
    ClientPrefService, ManagedModelService, ModelFetchService, ProviderConnectionService,
    ProviderModelService, ProviderService, SettingsService, SystemRouterState,
    VersionCheckService,
};

struct TestProviderDeletionCoordinator;

#[async_trait::async_trait]
impl nomifun_system::provider_deletion::ProviderDeletionCoordinator
    for TestProviderDeletionCoordinator
{
    async fn usages(
        &self,
        _provider_id: &str,
    ) -> Result<Vec<nomifun_common::ProviderUsage>, nomifun_common::AppError> {
        Ok(Vec::new())
    }

    async fn prepare_soft_model_cleanup(
        &self,
        _provider_id: &str,
        _model: &str,
    ) -> Result<nomifun_db::ProviderModelCleanupPlan, nomifun_common::AppError> {
        Ok(nomifun_db::ProviderModelCleanupPlan::default())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_system_state(
    db: &nomifun_db::Database,
    encryption_key: [u8; 32],
    http_client: reqwest::Client,
    version_check_service: VersionCheckService,
    managed_model_service: Option<Arc<ManagedModelService>>,
    data_dir: PathBuf,
    work_dir: PathBuf,
    work_dir_is_cli_override: bool,
) -> SystemRouterState {
    let provider_repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
    let model_repo = Arc::new(SqliteProviderModelRepository::new(db.pool().clone()));
    let capability_repo = Arc::new(SqliteProviderModelCapabilityRepository::new(
        db.pool().clone(),
    ));
    let connection_repo = Arc::new(SqliteProviderConnectionRepository::new(db.pool().clone()));

    SystemRouterState {
        settings_service: SettingsService::new(Arc::new(SqliteSettingsRepository::new(
            db.pool().clone(),
        ))),
        client_pref_service: ClientPrefService::new(Arc::new(
            SqliteClientPreferenceRepository::new(db.pool().clone()),
        )),
        provider_service: ProviderService::new(
            provider_repo.clone(),
            model_repo.clone(),
            capability_repo.clone(),
            connection_repo.clone(),
            encryption_key,
        ),
        provider_connection_service: ProviderConnectionService::new(
            connection_repo.clone(),
            provider_repo.clone(),
            capability_repo.clone(),
            encryption_key,
        ),
        model_fetch_service: ModelFetchService::new(
            provider_repo.clone(),
            encryption_key,
            http_client,
        ),
        provider_model_service: ProviderModelService::new(
            model_repo,
            capability_repo,
            provider_repo,
            connection_repo,
            Arc::new(TestProviderDeletionCoordinator),
        ),
        managed_model_service,
        version_check_service,
        data_dir,
        work_dir,
        work_dir_is_cli_override,
    }
}
