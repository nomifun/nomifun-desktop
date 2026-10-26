//! Composition for the instance-owned CLI office workbench.
use std::sync::Arc;
use nomifun_office_hub::{OfficeHubService, runtime::CliRuntime, service::HubSpawner, store::OfficeHubStore};
use crate::services::AppServices;

pub(crate) async fn build(services:&AppServices)->anyhow::Result<Arc<OfficeHubService>>{
    let service=services.office_hub.get_or_try_init(||async {
        let tasks=services.background_tasks.clone();
        let spawn:HubSpawner=Arc::new(move|task|tasks.spawn(task));
        let store=Arc::new(OfficeHubStore::new(services.database.pool().clone()));
        let runtime=Arc::new(CliRuntime::new(services.data_dir.clone()));
        OfficeHubService::new(services.authoritative_user_id.to_string(),store,runtime,spawn,
            services.background_shutdown.clone(),services._boot_reconciliation_authority.is_some()).await
    }).await?;
    Ok(service.clone())
}
