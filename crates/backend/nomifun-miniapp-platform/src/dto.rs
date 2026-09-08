use nomifun_agent_contracts::{MiniAppProductLifecycleState, MiniAppServiceLifecycle};
use nomifun_api_types::{
    CredentialBindingStatusDto, CredentialSlotBindingDto, MiniAppKindDto,
    MiniAppLibraryResponseDto, MiniAppLifecycleDto, MiniAppProjectSourceStateDto,
    MiniAppReadyReleaseDto, MiniAppReleasePointersDto, MiniAppReleaseRefDto,
    MiniAppReleaseTestDto, MiniAppServiceDescriptorDto, MiniAppServiceHealthDto,
    MiniAppServiceLifecycleDto, MiniAppSummaryDto, MiniAppTestStatusDto,
    MiniAppPublishModeDto, MiniAppWorkshopDto,
    PluginConfigSchemaDto, PluginConfigStateDto,
};

use crate::{
    MiniAppKind, MiniAppPlatformResult, MiniAppProjectSourceState, MiniAppRepositorySnapshot,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniAppServiceObservation {
    Stopped,
    Starting {
        release: nomifun_agent_contracts::MiniAppReleaseRef,
    },
    Ready {
        release: nomifun_agent_contracts::MiniAppReleaseRef,
        started_at_ms: i64,
    },
    Failed {
        release: nomifun_agent_contracts::MiniAppReleaseRef,
        error_code: String,
    },
}

pub fn library_dto(snapshots: &[MiniAppRepositorySnapshot]) -> MiniAppLibraryResponseDto {
    MiniAppLibraryResponseDto {
        library_revision: snapshots
            .first()
            .map_or(0, |snapshot| snapshot.library_revision),
        miniapps: snapshots
            .iter()
            .map(|snapshot| summary_dto(snapshot, None))
            .collect(),
    }
}

pub fn summary_dto(
    snapshot: &MiniAppRepositorySnapshot,
    service: Option<&MiniAppServiceObservation>,
) -> MiniAppSummaryDto {
    let product = &snapshot.root.product;
    MiniAppSummaryDto {
        miniapp_id: product.miniapp_id.0.clone(),
        product_revision: product.product_revision,
        display_name: product.display_name.clone(),
        description: product.description.clone(),
        icon_asset_id: product.icon_asset_id.clone(),
        kind: kind_dto(product.kind),
        lifecycle: lifecycle_dto(product.lifecycle),
        releases: pointers_dto(snapshot),
        service_health: service_health_dto(product.kind, service),
        surface_available: product.lifecycle == MiniAppProductLifecycleState::Enabled
            && product.pointers.active_release.is_some(),
        updated_at_ms: product.updated_at_ms,
    }
}

pub fn workshop_dto(
    snapshot: &MiniAppRepositorySnapshot,
    service: Option<&MiniAppServiceObservation>,
) -> MiniAppPlatformResult<MiniAppWorkshopDto> {
    snapshot.validate()?;
    let root = &snapshot.root;
    let ready = root
        .product
        .pointers
        .ready_release
        .as_ref()
        .and_then(|ready_ref| root.releases.get(&ready_ref.release_digest))
        .map(|stored| {
            let ready = &stored.ready;
            let service = stored.artifact.manifest.payload.service.as_ref();
            MiniAppReadyReleaseDto {
                release: release_dto(&ready.release),
                project_build_generation: match &ready.source_lineage {
                    nomifun_agent_contracts::MiniAppSourceLineage::Managed {
                        build_generation,
                        ..
                    } => *build_generation,
                    nomifun_agent_contracts::MiniAppSourceLineage::RuntimeOnly => 0,
                },
                created_at_ms: ready.created_at_ms,
                kind: kind_dto(root.product.kind),
                service: service.map(|service| MiniAppServiceDescriptorDto {
                    lifecycle: match service.lifecycle {
                        MiniAppServiceLifecycle::OnDemand => {
                            MiniAppServiceLifecycleDto::OnDemand
                        }
                        MiniAppServiceLifecycle::Continuous => {
                            MiniAppServiceLifecycleDto::Continuous
                        }
                    },
                    uses_files: service.uses_files,
                    uses_private_database: service.uses_private_database,
                    service_contract_digest: service.service_contract_digest.0.clone(),
                }),
                test: MiniAppReleaseTestDto {
                    status: if service.is_none() {
                        MiniAppTestStatusDto::NotRequired
                    } else if ready.matching_service_test_receipt.is_some() {
                        MiniAppTestStatusDto::Passed
                    } else {
                        MiniAppTestStatusDto::NotRun
                    },
                    release_id: ready.release.release_id.0.clone(),
                    expected_release_digest: ready.release.release_digest.0.clone(),
                    receipt_id: ready
                        .matching_service_test_receipt
                        .as_ref()
                        .map(|receipt| receipt.receipt_id.0.clone()),
                    expected_service_run_key: ready
                        .matching_service_test_receipt
                        .as_ref()
                        .map(|receipt| receipt.service_run_key.0.clone()),
                    issued_at_ms: None,
                    error_code: None,
                },
                migration_count: stored.artifact.manifest.payload.migrations.len() as u32,
                can_publish: true,
                can_auto_publish: root.product.pointers.active_release.is_some()
                    && root
                        .product
                        .auto_publish
                        .as_ref()
                        .is_some_and(|authorization| authorization.enabled),
                blocking_reasons: Vec::new(),
            }
        });
    let active_operation = snapshot
        .deletion
        .as_ref()
        .map(|deletion| deletion.operation.to_dto());

    Ok(MiniAppWorkshopDto {
        miniapp: summary_dto(snapshot, service),
        publish_mode: if root.product.auto_publish.as_ref().is_some_and(|value| value.enabled) {
            MiniAppPublishModeDto::AutoUiOnly
        } else {
            MiniAppPublishModeDto::Manual
        },
        project_id: root.project.project_id.0.clone(),
        project_revision: root.project.project_revision,
        source_state: match root.project.source_state {
            MiniAppProjectSourceState::Empty => MiniAppProjectSourceStateDto::Empty,
            MiniAppProjectSourceState::Editable => MiniAppProjectSourceStateDto::Editable,
            MiniAppProjectSourceState::RuntimeOnly => MiniAppProjectSourceStateDto::RuntimeOnly,
        },
        build_generation: root.project.build_generation,
        source_snapshot_digest: root
            .project
            .source_snapshot_digest
            .as_ref()
            .map(|value| value.0.clone()),
        dependency_lock_digest: root
            .project
            .dependency_lock_digest
            .as_ref()
            .map(|value| value.0.clone()),
        ready,
        config_schema: PluginConfigSchemaDto {
            schema_digest: root.config.schema_digest.0.clone(),
            schema: root.config.schema.0.clone(),
        },
        config: PluginConfigStateDto {
            config_revision: root.config.revision,
            schema_digest: root.config.schema_digest.0.clone(),
            values: root.config.values.0.clone(),
            valid: root.config.valid,
            validation_errors: root.config.validation_errors.clone(),
        },
        credential_bindings_revision: root.credential_bindings.revision,
        credential_slots: root
            .credential_bindings
            .slots
            .iter()
            .map(|(slot, credential)| CredentialSlotBindingDto {
                slot_key: slot.clone(),
                display_name: slot.clone(),
                required: true,
                status: if credential.is_some() {
                    CredentialBindingStatusDto::Bound
                } else {
                    CredentialBindingStatusDto::Unbound
                },
                credential_id: credential.as_ref().map(|value| value.0.clone()),
            })
            .collect(),
        capabilities: Vec::new(),
        active_operation,
    })
}

fn kind_dto(kind: MiniAppKind) -> MiniAppKindDto {
    match kind {
        MiniAppKind::UiOnly => MiniAppKindDto::UiOnly,
        MiniAppKind::Service => MiniAppKindDto::Service,
    }
}

fn lifecycle_dto(lifecycle: MiniAppProductLifecycleState) -> MiniAppLifecycleDto {
    match lifecycle {
        MiniAppProductLifecycleState::Enabled => MiniAppLifecycleDto::Enabled,
        MiniAppProductLifecycleState::Disabled => MiniAppLifecycleDto::Disabled,
        MiniAppProductLifecycleState::Trashed => MiniAppLifecycleDto::Trashed,
        MiniAppProductLifecycleState::Deleting => MiniAppLifecycleDto::Deleting,
    }
}

fn pointers_dto(snapshot: &MiniAppRepositorySnapshot) -> MiniAppReleasePointersDto {
    let root = &snapshot.root;
    MiniAppReleasePointersDto {
        pointer_revision: root.product.pointers.pointer_revision,
        active_release_epoch: root.product.pointers.active_release_epoch,
        ready: root
            .product
            .pointers
            .ready_release
            .as_ref()
            .and_then(|ready| root.releases.get(&ready.release_digest))
            .map(|stored| release_dto(stored.release_ref())),
        active: root
            .product
            .pointers
            .active_release
            .as_ref()
            .map(release_dto),
        previous: root
            .product
            .pointers
            .previous_release
            .as_ref()
            .map(release_dto),
    }
}

fn release_dto(value: &nomifun_agent_contracts::MiniAppReleaseRef) -> MiniAppReleaseRefDto {
    MiniAppReleaseRefDto {
        release_id: value.release_id.0.clone(),
        artifact_id: value.artifact_id.0.clone(),
        release_digest: value.release_digest.0.clone(),
        manifest_digest: value.manifest_digest.0.clone(),
    }
}

fn service_health_dto(
    kind: MiniAppKind,
    value: Option<&MiniAppServiceObservation>,
) -> MiniAppServiceHealthDto {
    if kind == MiniAppKind::UiOnly {
        return MiniAppServiceHealthDto::NotApplicable;
    }
    match value {
        None | Some(MiniAppServiceObservation::Stopped) => MiniAppServiceHealthDto::Stopped,
        Some(MiniAppServiceObservation::Starting { release }) => {
            MiniAppServiceHealthDto::Starting {
                release_id: release.release_id.0.clone(),
                expected_release_digest: release.release_digest.0.clone(),
            }
        }
        Some(MiniAppServiceObservation::Ready {
            release,
            started_at_ms,
        }) => MiniAppServiceHealthDto::Ready {
            release_id: release.release_id.0.clone(),
            expected_release_digest: release.release_digest.0.clone(),
            started_at_ms: *started_at_ms,
        },
        Some(MiniAppServiceObservation::Failed {
            release,
            error_code,
        }) => MiniAppServiceHealthDto::Failed {
            release_id: release.release_id.0.clone(),
            expected_release_digest: release.release_digest.0.clone(),
            error_code: error_code.clone(),
        },
    }
}
