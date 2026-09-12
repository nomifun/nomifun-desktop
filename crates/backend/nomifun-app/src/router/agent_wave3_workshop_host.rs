//! Nomi-owned Wave 3 Creative Studio capability adapter.
//!
//! Canvas identity comes exclusively from the frozen `canvas` binding. Asset
//! operations are scoped by an owned `asset_library` binding and accept only a
//! concrete asset id as action input. Director is intentionally absent.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave3::{
    ASSET_LIBRARY_RESOURCE_KIND, CANVAS_RESOURCE_KIND, CREATIVE_ASSET_LIBRARY_RESOURCE_ID,
    Wave3CapabilityOperation, Wave3HostContext, Wave3HostPort, Wave3HostPortError,
    Wave3HostRequest,
};
use nomifun_common::AppError;
use nomifun_creation::CreationService;
use nomifun_workshop::service::AssetPatch;
use nomifun_workshop::template_run::CreativeTemplateInputValue;
use nomifun_workshop::{CreativeCanvasDocument, CreativeTemplateRunCreateRequest, WorkshopService};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::agent_wave3_template_runner::NomiWave3TemplateRunner;

pub(crate) const WAVE3_WORKSHOP_NOT_FOUND: &str = "WAVE3_WORKSHOP_NOT_FOUND";
pub(crate) const WAVE3_WORKSHOP_CONFLICT: &str = "WAVE3_WORKSHOP_CONFLICT";
pub(crate) const WAVE3_WORKSHOP_FAILED: &str = "WAVE3_WORKSHOP_FAILED";

#[derive(Clone)]
pub(crate) struct NomiWave3WorkshopHost {
    service: Arc<WorkshopService>,
    template_runner: NomiWave3TemplateRunner,
}

impl NomiWave3WorkshopHost {
    pub(crate) fn new(
        service: Arc<WorkshopService>,
        creation: Arc<CreationService>,
    ) -> Self {
        Self {
            template_runner: NomiWave3TemplateRunner::new(Arc::clone(&service), creation),
            service,
        }
    }

    pub(crate) fn into_port(self) -> Arc<dyn Wave3HostPort> {
        Arc::new(self)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanvasEditInput {
    expected_revision: String,
    document: CreativeCanvasDocument,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetReadInput {
    asset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetWriteInput {
    asset_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    in_library: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TemplateRunInput {
    template_id: String,
    template_revision: i64,
    inputs: Vec<CreativeTemplateInputValue>,
    reference_asset_ids: Vec<String>,
}

impl Wave3HostPort for NomiWave3WorkshopHost {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            request.validate()?;
            let context = request.context.clone();
            self.service
                .require_creative_studio_owner(&context.principal.principal_id)
                .await
                .map_err(map_workshop_error)?;
            let result = match request.operation {
                Wave3CapabilityOperation::WorkshopCanvasRead { input } => {
                    let _: EmptyInput = parse_input(input)?;
                    let canvas_id = bound_resource_id(&context, CANVAS_RESOURCE_KIND)?;
                    serde_json::to_value(
                        self.service
                            .get_creative_canvas(&canvas_id)
                            .await
                            .map_err(map_workshop_error)?,
                    )
                }
                Wave3CapabilityOperation::WorkshopCanvasEdit { input } => {
                    let input: CanvasEditInput = parse_input(input)?;
                    let canvas_id = bound_resource_id(&context, CANVAS_RESOURCE_KIND)?;
                    if input.document.canvas_id != canvas_id {
                        return Err(Wave3HostPortError::resource_binding_invalid(
                            "Canvas document identity differs from the frozen canvas binding",
                        ));
                    }
                    serde_json::to_value(
                        self.service
                            .save_creative_canvas(
                                &canvas_id,
                                &input.expected_revision,
                                &input.document,
                            )
                            .await
                            .map_err(map_workshop_error)?,
                    )
                }
                Wave3CapabilityOperation::WorkshopAssetRead { input } => {
                    let input: AssetReadInput = parse_input(input)?;
                    require_library_binding(&context)?;
                    serde_json::to_value(
                        self.service
                            .get_asset(&input.asset_id)
                            .await
                            .map_err(map_workshop_error)?,
                    )
                }
                Wave3CapabilityOperation::WorkshopAssetWrite { input } => {
                    let input: AssetWriteInput = parse_input(input)?;
                    require_library_binding(&context)?;
                    serde_json::to_value(
                        self.service
                            .patch_asset(
                                &input.asset_id,
                                AssetPatch {
                                    title: input.title,
                                    collection: input.collection,
                                    tags: input.tags,
                                    in_library: input.in_library,
                                },
                            )
                            .await
                            .map_err(map_workshop_error)?,
                    )
                }
                Wave3CapabilityOperation::WorkshopTemplateRun { input } => {
                    let input: TemplateRunInput = parse_input(input)?;
                    let canvas_id = bound_resource_id(&context, CANVAS_RESOURCE_KIND)?;
                    // The Canvas is a real selected target, not a placeholder
                    // slot. Require it to exist before accepting a durable run.
                    self.service
                        .get_creative_canvas(&canvas_id)
                        .await
                        .map_err(map_workshop_error)?;
                    let request = CreativeTemplateRunCreateRequest {
                        template_run_id: stable_template_run_id(
                            &context,
                            &canvas_id,
                            &input.template_id,
                        ),
                        template_id: input.template_id,
                        template_revision: input.template_revision,
                        inputs: input.inputs,
                        reference_asset_ids: input.reference_asset_ids,
                    };
                    serde_json::to_value(self.template_runner.run(request).await?)
                }
                operation => {
                    return Err(Wave3HostPortError::invalid_request(format!(
                        "Workshop host cannot execute {}",
                        operation.capability_id().as_ref()
                    )));
                }
            }
            .map_err(|error| {
                Wave3HostPortError::new(
                    "WAVE3_WORKSHOP_SERIALIZATION_FAILED",
                    format!("Workshop result serialization failed: {error}"),
                )
            })?;
            Ok(StrictJsonValue(result))
        })
    }
}

fn require_library_binding(context: &Wave3HostContext) -> Result<(), Wave3HostPortError> {
    let resource_id = bound_resource_id(context, ASSET_LIBRARY_RESOURCE_KIND)?;
    if resource_id != CREATIVE_ASSET_LIBRARY_RESOURCE_ID {
        return Err(Wave3HostPortError::resource_binding_invalid(format!(
            "asset_library binding must resolve to {CREATIVE_ASSET_LIBRARY_RESOURCE_ID}"
        )));
    }
    Ok(())
}

fn stable_template_run_id(
    context: &Wave3HostContext,
    canvas_id: &str,
    template_id: &str,
) -> String {
    let mut digest = Sha256::new();
    for part in [
        context.principal.principal_id.as_str(),
        context.agent_session_id.as_ref(),
        context.idempotency_key.as_ref(),
        canvas_id,
        template_id,
    ] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn bound_resource_id(
    context: &Wave3HostContext,
    expected_kind: &str,
) -> Result<String, Wave3HostPortError> {
    context
        .resource_bindings
        .iter()
        .find(|binding| binding.resource_kind.as_ref() == expected_kind)
        .map(|binding| binding.resource_id.as_ref().to_owned())
        .ok_or_else(|| {
            Wave3HostPortError::resource_binding_invalid(format!(
                "{} requires one {expected_kind} resource binding",
                context.capability_id.as_ref()
            ))
        })
}

fn parse_input<T: for<'de> Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, Wave3HostPortError> {
    serde_json::from_value(input.0).map_err(|error| {
        Wave3HostPortError::invalid_request(format!("Workshop action input is invalid: {error}"))
    })
}

fn map_workshop_error(error: AppError) -> Wave3HostPortError {
    let status = error.status_code();
    let message = error.to_string();
    let code = if status == axum::http::StatusCode::NOT_FOUND {
        WAVE3_WORKSHOP_NOT_FOUND
    } else if status == axum::http::StatusCode::CONFLICT {
        WAVE3_WORKSHOP_CONFLICT
    } else if status.is_client_error() {
        return Wave3HostPortError::invalid_request(message);
    } else {
        WAVE3_WORKSHOP_FAILED
    };
    Wave3HostPortError::new(code, message)
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, CorrelationId, DigestHex, IdempotencyKey,
        OperationId, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef, ResourceBindingId,
        ResourceId, ResourceKind, ScopeKey, TypedResourceBinding,
    };
    use nomifun_db::{
        IWorkshopRepository, SqliteWorkshopRepository, init_database_memory,
        installation_owner_id,
    };
    use nomifun_workshop::service::NewTextAsset;
    use serde_json::json;

    use super::*;

    fn request(
        owner_id: &str,
        resource_kind: &str,
        resource_id: &str,
        operations: &[&str],
        capability_id: &str,
        operation: Wave3CapabilityOperation,
    ) -> Wave3HostRequest {
        Wave3HostRequest {
            context: nomifun_agent_domain_wave3::Wave3HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.to_owned(),
                },
                agent_session_id: AgentSessionId::from("workshop-wave3-session"),
                operation_id: OperationId::from("workshop-wave3-operation"),
                idempotency_key: IdempotencyKey::from("workshop-wave3-idempotency"),
                correlation_id: CorrelationId::from("workshop-wave3-correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: ResolvedSnapshotId::from("workshop-wave3-snapshot"),
                    snapshot_digest: DigestHex::from("b".repeat(64)),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from(capability_id),
                action_id: ActionId::from(format!("{capability_id}.invoke")),
                state_scope_key: ScopeKey::from("session:workshop-wave3"),
                resource_bindings: vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("workshop-binding"),
                    resource_kind: ResourceKind::from(resource_kind),
                    resource_id: ResourceId::from(resource_id.to_owned()),
                    owner_id: owner_id.to_owned(),
                    operations: operations.iter().map(|value| (*value).to_owned()).collect(),
                    connection_config_ref: None,
                    typed_parameters: Default::default(),
                }],
            },
            operation,
        }
    }

    #[tokio::test]
    async fn canvas_and_asset_actions_persist_through_the_real_workshop_service() {
        let database = init_database_memory().await.expect("database");
        let owner_id = installation_owner_id(database.pool()).await.expect("owner");
        let repository: Arc<dyn IWorkshopRepository> =
            Arc::new(SqliteWorkshopRepository::new(database.pool().clone()));
        let root = tempfile::tempdir().expect("root");
        let service = WorkshopService::start(root.path(), repository);
        let canvas = service
            .create_creative_canvas_for_owner(&owner_id, Some("Wave3 Canvas".into()), None)
            .await
            .expect("create canvas");
        let creation = CreationService::new(Arc::new(
            nomifun_db::SqliteCreationTaskRepository::new(database.pool().clone()),
        ));
        let host = NomiWave3WorkshopHost::new(Arc::clone(&service), creation);

        let detail = host
            .invoke(request(
                &owner_id,
                CANVAS_RESOURCE_KIND,
                &canvas.canvas_id,
                &["read", "write"],
                "workshop.canvas.read",
                Wave3CapabilityOperation::WorkshopCanvasRead {
                    input: StrictJsonValue(json!({})),
                },
            ))
            .await
            .expect("read canvas");
        let document: CreativeCanvasDocument =
            serde_json::from_value(detail.0["document"].clone()).expect("document");
        let edited = host
            .invoke(request(
                &owner_id,
                CANVAS_RESOURCE_KIND,
                &canvas.canvas_id,
                &["read", "write"],
                "workshop.canvas.edit",
                Wave3CapabilityOperation::WorkshopCanvasEdit {
                    input: StrictJsonValue(json!({
                        "expected_revision": canvas.revision,
                        "document": document,
                    })),
                },
            ))
            .await
            .expect("save canvas");
        assert_eq!(edited.0["canvasId"], canvas.canvas_id);
        assert_ne!(edited.0["revision"], canvas.revision);

        let asset = service
            .create_text_asset(NewTextAsset {
                title: "Before".into(),
                text_content: "durable".into(),
                collection: None,
                tags: None,
                in_library: Some(true),
                origin: None,
            })
            .await
            .expect("create asset");
        let patched = host
            .invoke(request(
                &owner_id,
                ASSET_LIBRARY_RESOURCE_KIND,
                CREATIVE_ASSET_LIBRARY_RESOURCE_ID,
                &["read", "write"],
                "workshop.asset.write",
                Wave3CapabilityOperation::WorkshopAssetWrite {
                    input: StrictJsonValue(json!({
                        "asset_id": asset.asset_id,
                        "title": "After",
                        "tags": ["wave3"]
                    })),
                },
            ))
            .await
            .expect("patch asset");
        assert_eq!(patched.0["title"], "After");
        assert_eq!(
            service
                .get_asset(patched.0["asset_id"].as_str().expect("asset id"))
                .await
                .expect("saved asset")
                .title,
            "After"
        );

        let forged = host
            .invoke(request(
                &owner_id,
                ASSET_LIBRARY_RESOURCE_KIND,
                "another-library",
                &["read"],
                "workshop.asset.read",
                Wave3CapabilityOperation::WorkshopAssetRead {
                    input: StrictJsonValue(json!({"asset_id": asset.asset_id})),
                },
            ))
            .await
            .expect_err("cross-library binding must be rejected");
        assert_eq!(forged.code, "WAVE3_RESOURCE_BINDING_INVALID");
    }

    #[test]
    fn template_run_identity_is_bound_to_canvas_template_session_and_idempotency() {
        let owner = "0190f5fe-7c00-7000-8000-000000000001";
        let first = request(
            owner,
            CANVAS_RESOURCE_KIND,
            "0190f5fe-7c00-7000-8000-000000000010",
            &["write"],
            "workshop.template.run",
            Wave3CapabilityOperation::WorkshopTemplateRun {
                input: StrictJsonValue(json!({})),
            },
        );
        let template = "0190f5fe-7c00-7000-8000-000000000020";
        let first_id = stable_template_run_id(
            &first.context,
            "0190f5fe-7c00-7000-8000-000000000010",
            template,
        );
        let other_canvas_id = stable_template_run_id(
            &first.context,
            "0190f5fe-7c00-7000-8000-000000000011",
            template,
        );
        let other_template_id = stable_template_run_id(
            &first.context,
            "0190f5fe-7c00-7000-8000-000000000010",
            "0190f5fe-7c00-7000-8000-000000000021",
        );
        assert_ne!(first_id, other_canvas_id);
        assert_ne!(first_id, other_template_id);
    }
}
