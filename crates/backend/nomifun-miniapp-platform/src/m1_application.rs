use std::sync::Arc;

use nomifun_api_types::{
    CredentialBindingStatusDto, CredentialSlotBindingDto,
    CreateMiniAppProjectRequest, MiniAppKindDto, MiniAppLibraryResponseDto,
    MiniAppLifecycleDto, MiniAppProjectSourceStateDto,
    MiniAppReleasePointersDto, MiniAppReleaseRefDto, MiniAppReleaseTestDto,
    MiniAppReadyReleaseDto, MiniAppServiceHealthDto, MiniAppSummaryDto,
    MiniAppTestStatusDto, MiniAppWorkshopDto, PluginConfigSchemaDto,
    PluginConfigStateDto,
};
use nomifun_db::{
    CreateMiniAppM1Params, IMiniAppM1Repository, MiniAppM1Kind,
    MiniAppM1Snapshot, MiniAppProductRow,
};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MiniAppM1ApplicationError {
    #[error("MiniApp input is invalid: {0}")]
    Invalid(String),
    #[error("MiniApp was not found")]
    NotFound,
    #[error("MiniApp database failed: {0}")]
    Database(#[from] nomifun_db::DbError),
}

#[derive(Clone)]
pub struct MiniAppM1ApplicationService {
    repository: Arc<dyn IMiniAppM1Repository>,
}

impl std::fmt::Debug for MiniAppM1ApplicationService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MiniAppM1ApplicationService")
            .finish_non_exhaustive()
    }
}

impl MiniAppM1ApplicationService {
    pub fn new(repository: Arc<dyn IMiniAppM1Repository>) -> Self {
        Self { repository }
    }

    pub async fn library(
        &self,
        owner_user_id: &str,
    ) -> Result<MiniAppLibraryResponseDto, MiniAppM1ApplicationError> {
        let library = self.repository.library(owner_user_id).await?;
        let mut miniapps = Vec::with_capacity(library.products.len());
        for product in &library.products {
            let snapshot = self
                .repository
                .get(owner_user_id, &product.miniapp_id)
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(format!(
                        "MiniApp {} disappeared from its owner-scoped Library",
                        product.miniapp_id
                    ))
                })?;
            miniapps.push(summary_from_snapshot(&snapshot)?);
        }
        Ok(MiniAppLibraryResponseDto {
            library_revision: nonnegative_u64(
                library.library.revision,
                "MiniApp library revision",
            )?,
            miniapps,
        })
    }

    pub async fn create(
        &self,
        owner_user_id: &str,
        request: CreateMiniAppProjectRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let miniapp_id = Uuid::now_v7().to_string();
        let project_id = Uuid::now_v7().to_string();
        let kind = match request.kind {
            MiniAppKindDto::UiOnly => MiniAppM1Kind::UiOnly,
            MiniAppKindDto::Service => MiniAppM1Kind::Service,
        };
        let catalog_digest = nomifun_agent_contracts::digest_bytes(
            b"miniapp-m1-empty-catalog",
        );
        let snapshot = self
            .repository
            .create(&CreateMiniAppM1Params {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id,
                project_id,
                expected_library_revision: i64::try_from(
                    request.expected_library_revision,
                )
                .map_err(|_| MiniAppM1ApplicationError::Invalid(
                    "library revision exceeds SQLite range".to_owned(),
                ))?,
                display_name: request.display_name,
                description: request.description,
                icon_asset_id: None,
                kind,
                materialized_catalog_digest: catalog_digest.as_ref().to_owned(),
                config_schema_json: r#"{"type":"object"}"#.to_owned(),
                config_json: "{}".to_owned(),
                created_at: nomifun_common::now_ms(),
            })
            .await?;
        workshop_from_snapshot(&snapshot)
    }

    pub async fn workshop(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let snapshot = self
            .repository
            .get(owner_user_id, miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        workshop_from_snapshot(&snapshot)
    }
}

fn summary_from_product(
    product: &MiniAppProductRow,
) -> Result<MiniAppSummaryDto, MiniAppM1ApplicationError> {
    let kind = kind_dto(&product.kind)?;
    let lifecycle = lifecycle_dto(&product.lifecycle)?;
    let product_revision =
        positive_u64(product.product_revision, "MiniApp product revision")?;
    let pointer_revision =
        positive_u64(product.pointer_revision, "MiniApp pointer revision")?;
    let active_release_epoch = nonnegative_u64(
        product.active_release_epoch,
        "MiniApp active release epoch",
    )?;
    Ok(MiniAppSummaryDto {
        miniapp_id: product.miniapp_id.clone(),
        product_revision,
        display_name: product.display_name.clone(),
        description: product.description.clone(),
        icon_asset_id: product.icon_asset_id.clone(),
        kind,
        lifecycle,
        releases: MiniAppReleasePointersDto {
            pointer_revision,
            active_release_epoch,
            ready: None,
            active: None,
            previous: None,
        },
        service_health: if kind == MiniAppKindDto::UiOnly {
            MiniAppServiceHealthDto::NotApplicable
        } else {
            MiniAppServiceHealthDto::Stopped
        },
        surface_available: lifecycle == MiniAppLifecycleDto::Enabled
            && product.active_release_id.is_some(),
        updated_at_ms: product.updated_at,
    })
}

fn summary_from_snapshot(
    snapshot: &MiniAppM1Snapshot,
) -> Result<MiniAppSummaryDto, MiniAppM1ApplicationError> {
    let mut summary = summary_from_product(&snapshot.product)?;
    summary.releases.ready = snapshot
        .ready_release
        .as_ref()
        .map(release_ref_from_row);
    summary.releases.active = snapshot
        .active_release
        .as_ref()
        .map(release_ref_from_row);
    summary.releases.previous = snapshot
        .previous_release
        .as_ref()
        .map(release_ref_from_row);
    Ok(summary)
}

fn workshop_from_snapshot(
    snapshot: &MiniAppM1Snapshot,
) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
    let product = &snapshot.product;
    let project = &snapshot.project;
    let summary = summary_from_snapshot(snapshot)?;
    let source_state = match project.source_state.as_str() {
        "empty" => MiniAppProjectSourceStateDto::Empty,
        "editable" => MiniAppProjectSourceStateDto::Editable,
        "runtime_only" => MiniAppProjectSourceStateDto::RuntimeOnly,
        value => {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "unknown MiniApp source_state {value}"
            )));
        }
    };
    let ready = snapshot
        .ready_release
        .as_ref()
        .map(|release| -> Result<_, MiniAppM1ApplicationError> {
        let release_ref = release_ref_from_row(release);
        Ok(MiniAppReadyReleaseDto {
            release: release_ref.clone(),
            project_build_generation: match release.build_generation {
                Some(value) => positive_u64(
                    value,
                    "MiniApp Ready Release build generation",
                )?,
                None => 0,
            },
            kind: summary.kind,
            service: None,
            test: MiniAppReleaseTestDto {
                status: MiniAppTestStatusDto::NotRequired,
                release_id: release.release_id.clone(),
                expected_release_digest: release.release_digest.clone(),
                receipt_id: None,
                expected_service_run_key: None,
                issued_at_ms: None,
                error_code: None,
            },
            migration_count: 0,
            can_publish: false,
            can_auto_publish: false,
            blocking_reasons: vec!["MINIAPP_PUBLISH_NOT_AVAILABLE".to_owned()],
        })
    })
    .transpose()?;
    let schema: Value = serde_json::from_str(&product.config_schema_json)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    let values: Value = serde_json::from_str(&product.config_json)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    let config_schema_digest =
        nomifun_agent_contracts::digest_payload(&schema).map_err(|error| {
            MiniAppM1ApplicationError::Invalid(error.to_string())
        })?;
    Ok(MiniAppWorkshopDto {
        miniapp: summary,
        project_id: project.project_id.clone(),
        project_revision: positive_u64(
            project.project_revision,
            "MiniApp Project revision",
        )?,
        source_state,
        build_generation: nonnegative_u64(
            project.build_generation,
            "MiniApp build generation",
        )?,
        source_snapshot_digest: project.source_head_digest.clone(),
        dependency_lock_digest: project.dependency_lock_digest.clone(),
        ready,
        config_schema: PluginConfigSchemaDto {
            schema_digest: config_schema_digest.as_ref().to_owned(),
            schema,
        },
        config: PluginConfigStateDto {
            config_revision: positive_u64(
                product.config_revision,
                "MiniApp config revision",
            )?,
            schema_digest: config_schema_digest.as_ref().to_owned(),
            values,
            valid: true,
            validation_errors: Vec::new(),
        },
        credential_bindings_revision: positive_u64(
            product.credential_bindings_revision,
            "MiniApp credential bindings revision",
        )?,
        credential_slots: snapshot
            .credential_bindings
            .iter()
            .map(|binding| CredentialSlotBindingDto {
                slot_key: binding.slot_key.clone(),
                display_name: binding.slot_key.clone(),
                required: true,
                status: CredentialBindingStatusDto::Bound,
                credential_id: Some(binding.credential_id.clone()),
            })
            .collect(),
        capabilities: Vec::new(),
        active_operation: None,
    })
}

fn nonnegative_u64(
    value: i64,
    label: &str,
) -> Result<u64, MiniAppM1ApplicationError> {
    u64::try_from(value).map_err(|_| {
        MiniAppM1ApplicationError::Invalid(format!("{label} is negative"))
    })
}

fn positive_u64(
    value: i64,
    label: &str,
) -> Result<u64, MiniAppM1ApplicationError> {
    let value = nonnegative_u64(value, label)?;
    if value == 0 {
        Err(MiniAppM1ApplicationError::Invalid(format!(
            "{label} must be positive"
        )))
    } else {
        Ok(value)
    }
}

fn kind_dto(value: &str) -> Result<MiniAppKindDto, MiniAppM1ApplicationError> {
    match value {
        "ui_only" => Ok(MiniAppKindDto::UiOnly),
        "service" => Ok(MiniAppKindDto::Service),
        value => Err(MiniAppM1ApplicationError::Invalid(format!(
            "unknown MiniApp kind {value}"
        ))),
    }
}

fn lifecycle_dto(value: &str) -> Result<MiniAppLifecycleDto, MiniAppM1ApplicationError> {
    match value {
        "enabled" => Ok(MiniAppLifecycleDto::Enabled),
        "disabled" => Ok(MiniAppLifecycleDto::Disabled),
        "trashed" => Ok(MiniAppLifecycleDto::Trashed),
        "deleting" => Ok(MiniAppLifecycleDto::Deleting),
        value => Err(MiniAppM1ApplicationError::Invalid(format!(
            "unknown MiniApp lifecycle {value}"
        ))),
    }
}

fn release_ref_from_row(
    release: &nomifun_db::MiniAppReleaseRow,
) -> MiniAppReleaseRefDto {
    MiniAppReleaseRefDto {
        release_id: release.release_id.clone(),
        artifact_id: release.artifact_id.clone(),
        release_digest: release.release_digest.clone(),
        manifest_digest: release.manifest_digest.clone(),
    }
}
