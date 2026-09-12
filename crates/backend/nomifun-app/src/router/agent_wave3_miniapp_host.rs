//! Nomi-owned Wave 3 MiniApp capability adapter.
//!
//! These four capabilities operate on one exact owner-scoped MiniApp selected
//! by the frozen `miniapp` resource binding. They deliberately do not proxy a
//! caller-supplied MiniApp id and they do not reuse an Active Release's dynamic
//! Agent actions as a substitute for the first-party lifecycle operations.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave3::{
    MINIAPP_RESOURCE_KIND, Wave3CapabilityOperation, Wave3HostPort, Wave3HostPortError,
    Wave3HostRequest,
};
use nomifun_api_types::{
    PublishMiniAppRequest, ReplaceMiniAppSourceFileRequest,
};
use nomifun_miniapp_platform::{MiniAppM1ApplicationError, MiniAppM1ApplicationService};
use serde::Deserialize;

pub(crate) const WAVE3_MINIAPP_NOT_FOUND: &str = "WAVE3_MINIAPP_NOT_FOUND";
pub(crate) const WAVE3_MINIAPP_CONFLICT: &str = "WAVE3_MINIAPP_CONFLICT";
pub(crate) const WAVE3_MINIAPP_RUNTIME_FAILED: &str = "WAVE3_MINIAPP_RUNTIME_FAILED";

#[derive(Clone)]
pub(crate) struct NomiWave3MiniAppHost {
    application: Arc<MiniAppM1ApplicationService>,
}

impl NomiWave3MiniAppHost {
    pub(crate) fn new(application: Arc<MiniAppM1ApplicationService>) -> Self {
        Self { application }
    }

    pub(crate) fn into_port(self) -> Arc<dyn Wave3HostPort> {
        Arc::new(self)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MiniAppReadInput {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MiniAppEditInput {
    expected_product_revision: u64,
    project_id: String,
    expected_project_revision: u64,
    expected_build_generation: u64,
    expected_source_snapshot_digest: String,
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MiniAppPublishInput {
    expected_product_revision: u64,
    expected_pointer_revision: u64,
    expected_active_release_epoch: u64,
    ready_release_id: String,
    expected_ready_release_digest: String,
    #[serde(default)]
    expected_active_release_digest: Option<String>,
    #[serde(default)]
    expected_service_test_receipt_id: Option<String>,
    acknowledge_test_warning: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

impl Wave3HostPort for NomiWave3MiniAppHost {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            request.validate()?;
            let owner_id = request.context.principal.principal_id.clone();
            let miniapp_id = bound_resource_id(&request, MINIAPP_RESOURCE_KIND)?;
            let result = match request.operation {
                Wave3CapabilityOperation::MiniAppRead { input } => {
                    let input: MiniAppReadInput = parse_input(input)?;
                    match input.path {
                        Some(path) => serde_json::to_value(
                            self.application
                                .source_file(&owner_id, &miniapp_id, &path)
                                .await
                                .map_err(map_miniapp_error)?,
                        ),
                        None => serde_json::to_value(
                            self.application
                                .workshop(&owner_id, &miniapp_id)
                                .await
                                .map_err(map_miniapp_error)?,
                        ),
                    }
                }
                Wave3CapabilityOperation::MiniAppEdit { input } => {
                    let input: MiniAppEditInput = parse_input(input)?;
                    serde_json::to_value(
                        self.application
                            .replace_source_file(
                                &owner_id,
                                ReplaceMiniAppSourceFileRequest {
                                    miniapp_id,
                                    expected_product_revision: input.expected_product_revision,
                                    project_id: input.project_id,
                                    expected_project_revision: input.expected_project_revision,
                                    expected_build_generation: input.expected_build_generation,
                                    expected_source_snapshot_digest: input
                                        .expected_source_snapshot_digest,
                                    path: input.path,
                                    content: input.content,
                                },
                            )
                            .await
                            .map_err(map_miniapp_error)?,
                    )
                }
                Wave3CapabilityOperation::MiniAppPublish { input } => {
                    let input: MiniAppPublishInput = parse_input(input)?;
                    serde_json::to_value(
                        self.application
                            .publish(
                                &owner_id,
                                PublishMiniAppRequest {
                                    miniapp_id,
                                    expected_product_revision: input.expected_product_revision,
                                    expected_pointer_revision: input.expected_pointer_revision,
                                    expected_active_release_epoch: input
                                        .expected_active_release_epoch,
                                    ready_release_id: input.ready_release_id,
                                    expected_ready_release_digest: input
                                        .expected_ready_release_digest,
                                    expected_active_release_digest: input
                                        .expected_active_release_digest,
                                    expected_service_test_receipt_id: input
                                        .expected_service_test_receipt_id,
                                    acknowledge_test_warning: input.acknowledge_test_warning,
                                },
                            )
                            .await
                            .map_err(map_miniapp_error)?,
                    )
                }
                Wave3CapabilityOperation::MiniAppServe { input } => {
                    let _: EmptyInput = parse_input(input)?;
                    // Agent callers may inspect the exact published serving
                    // projection but must not receive a bearer-like Surface
                    // capability or create an orphan UI session.
                    let workshop = self
                        .application
                        .workshop(&owner_id, &miniapp_id)
                        .await
                        .map_err(map_miniapp_error)?;
                    if !workshop.miniapp.surface_available {
                        return Err(Wave3HostPortError::invalid_request(
                            "bound MiniApp has no enabled Active Release to serve",
                        ));
                    }
                    serde_json::to_value(workshop)
                }
                operation => {
                    return Err(Wave3HostPortError::invalid_request(format!(
                        "MiniApp host cannot execute {}",
                        operation.capability_id().as_ref()
                    )));
                }
            }
            .map_err(|error| {
                Wave3HostPortError::new(
                    "WAVE3_MINIAPP_SERIALIZATION_FAILED",
                    format!("MiniApp result serialization failed: {error}"),
                )
            })?;
            Ok(StrictJsonValue(result))
        })
    }
}

fn bound_resource_id(
    request: &Wave3HostRequest,
    expected_kind: &str,
) -> Result<String, Wave3HostPortError> {
    request
        .context
        .resource_bindings
        .iter()
        .find(|binding| binding.resource_kind.as_ref() == expected_kind)
        .map(|binding| binding.resource_id.as_ref().to_owned())
        .ok_or_else(|| {
            Wave3HostPortError::resource_binding_invalid(format!(
                "{} requires one {expected_kind} resource binding",
                request.context.capability_id.as_ref()
            ))
        })
}

fn parse_input<T: for<'de> Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, Wave3HostPortError> {
    serde_json::from_value(input.0).map_err(|error| {
        Wave3HostPortError::invalid_request(format!("MiniApp action input is invalid: {error}"))
    })
}

fn map_miniapp_error(error: MiniAppM1ApplicationError) -> Wave3HostPortError {
    match error {
        MiniAppM1ApplicationError::NotFound => {
            Wave3HostPortError::new(WAVE3_MINIAPP_NOT_FOUND, "bound MiniApp was not found")
        }
        MiniAppM1ApplicationError::Invalid(message) => {
            Wave3HostPortError::invalid_request(message)
        }
        MiniAppM1ApplicationError::Runtime(message) => {
            Wave3HostPortError::new(WAVE3_MINIAPP_RUNTIME_FAILED, message)
        }
        MiniAppM1ApplicationError::Database(error) => {
            let message = error.to_string();
            let code = if message.to_ascii_lowercase().contains("conflict") {
                WAVE3_MINIAPP_CONFLICT
            } else {
                WAVE3_MINIAPP_RUNTIME_FAILED
            };
            Wave3HostPortError::new(code, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, CorrelationId, DigestHex, IdempotencyKey,
        MiniAppId, OperationId, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef,
        ResourceBindingId, ResourceId, ResourceKind, ScopeKey, TypedResourceBinding,
    };
    use nomifun_api_types::{
        BuildMiniAppRequest, CreateMiniAppProjectRequest, MiniAppKindDto, MiniAppWorkshopDto,
        SetMiniAppEnabledRequest,
    };
    use nomifun_db::{
        IMiniAppM1Repository, SqliteMiniAppM1Repository, init_database_memory,
        installation_owner_id,
    };
    use serde_json::json;

    use super::*;

    fn request(
        owner_id: &str,
        miniapp_id: &str,
        capability_id: &str,
        operation: Wave3CapabilityOperation,
    ) -> Wave3HostRequest {
        Wave3HostRequest {
            context: nomifun_agent_domain_wave3::Wave3HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.to_owned(),
                },
                agent_session_id: AgentSessionId::from("miniapp-wave3-session"),
                operation_id: OperationId::from("miniapp-wave3-operation"),
                idempotency_key: IdempotencyKey::from("miniapp-wave3-idempotency"),
                correlation_id: CorrelationId::from("miniapp-wave3-correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: ResolvedSnapshotId::from("miniapp-wave3-snapshot"),
                    snapshot_digest: DigestHex::from("a".repeat(64)),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from(capability_id),
                action_id: ActionId::from(format!("{capability_id}.invoke")),
                state_scope_key: ScopeKey::from("session:miniapp-wave3"),
                resource_bindings: vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("miniapp-binding"),
                    resource_kind: ResourceKind::from(MINIAPP_RESOURCE_KIND),
                    resource_id: ResourceId::from(miniapp_id.to_owned()),
                    owner_id: owner_id.to_owned(),
                    operations: BTreeSet::from([
                        "read".into(),
                        "edit".into(),
                        "publish".into(),
                        "serve".into(),
                    ]),
                    connection_config_ref: None,
                    typed_parameters: Default::default(),
                }],
            },
            operation,
        }
    }

    #[tokio::test]
    async fn miniapp_actions_use_bound_identity_and_persist_edit_publish_and_serve_state() {
        let database = init_database_memory().await.expect("database");
        let owner_id = installation_owner_id(database.pool()).await.expect("owner");
        let repository: Arc<dyn IMiniAppM1Repository> =
            Arc::new(SqliteMiniAppM1Repository::new(database.pool().clone()));
        let root = tempfile::tempdir().expect("root");
        let application = Arc::new(
            MiniAppM1ApplicationService::new_with_root(repository, root.path())
                .expect("application"),
        );
        let created = application
            .create(
                &owner_id,
                CreateMiniAppProjectRequest {
                    expected_library_revision: 0,
                    display_name: "Wave3 MiniApp".into(),
                    description: None,
                    kind: MiniAppKindDto::UiOnly,
                },
            )
            .await
            .expect("create MiniApp");
        let miniapp_id = created.miniapp.miniapp_id.clone();
        let source_digest = created
            .source_snapshot_digest
            .clone()
            .expect("source digest");
        let host = NomiWave3MiniAppHost::new(Arc::clone(&application));

        let output = host
            .invoke(request(
                &owner_id,
                &miniapp_id,
                "miniapp.edit",
                Wave3CapabilityOperation::MiniAppEdit {
                    input: StrictJsonValue(json!({
                        "expected_product_revision": created.miniapp.product_revision,
                        "project_id": created.project_id,
                        "expected_project_revision": created.project_revision,
                        "expected_build_generation": created.build_generation,
                        "expected_source_snapshot_digest": source_digest,
                        "path": "ui/index.html",
                        "content": "<!doctype html><title>Wave3 persisted</title>"
                    })),
                },
            ))
            .await
            .expect("edit through Wave3");
        assert_eq!(output.0["miniapp"]["miniapp_id"], miniapp_id);
        let edited: MiniAppWorkshopDto =
            serde_json::from_value(output.0.clone()).expect("edited workshop");

        let saved = application
            .source_file(&owner_id, &miniapp_id, "ui/index.html")
            .await
            .expect("saved source");
        assert_eq!(
            saved.content,
            "<!doctype html><title>Wave3 persisted</title>"
        );

        let read = host
            .invoke(request(
                &owner_id,
                &miniapp_id,
                "miniapp.read",
                Wave3CapabilityOperation::MiniAppRead {
                    input: StrictJsonValue(json!({ "path": "ui/index.html" })),
                },
            ))
            .await
            .expect("read through Wave3");
        assert_eq!(read.0["content"], saved.content);
        assert_eq!(MiniAppId::from(miniapp_id).as_ref(), read.0["miniapp_id"]);

        let built = application
            .build(
                &owner_id,
                BuildMiniAppRequest {
                    miniapp_id: edited.miniapp.miniapp_id.clone(),
                    expected_product_revision: edited.miniapp.product_revision,
                    project_id: edited.project_id.clone(),
                    expected_project_revision: edited.project_revision,
                    expected_build_generation: edited.build_generation,
                    expected_source_snapshot_digest: edited
                        .source_snapshot_digest
                        .clone()
                        .expect("source digest"),
                    expected_dependency_lock_digest: edited
                        .dependency_lock_digest
                        .clone()
                        .expect("dependency digest"),
                    service_lifecycle: None,
                },
            )
            .await
            .expect("build Ready Release");
        let ready = built.ready.as_ref().expect("ready release");
        let published = host
            .invoke(request(
                &owner_id,
                &built.miniapp.miniapp_id,
                "miniapp.publish",
                Wave3CapabilityOperation::MiniAppPublish {
                    input: StrictJsonValue(json!({
                        "expected_product_revision": built.miniapp.product_revision,
                        "expected_pointer_revision": built.miniapp.releases.pointer_revision,
                        "expected_active_release_epoch": built.miniapp.releases.active_release_epoch,
                        "ready_release_id": ready.release.release_id,
                        "expected_ready_release_digest": ready.release.release_digest,
                        "acknowledge_test_warning": false
                    })),
                },
            ))
            .await
            .expect("publish through Wave3");
        let published: MiniAppWorkshopDto =
            serde_json::from_value(published.0).expect("published workshop");
        let active_digest = published
            .miniapp
            .releases
            .active
            .as_ref()
            .map(|release| release.release_digest.clone());
        let enabled = application
            .set_enabled(
                &owner_id,
                SetMiniAppEnabledRequest {
                    miniapp_id: published.miniapp.miniapp_id.clone(),
                    expected_product_revision: published.miniapp.product_revision,
                    expected_pointer_revision: published.miniapp.releases.pointer_revision,
                    expected_active_release_digest: active_digest,
                    enabled: true,
                },
            )
            .await
            .expect("enable published MiniApp");
        assert!(enabled.miniapp.surface_available);

        let serving = host
            .invoke(request(
                &owner_id,
                &enabled.miniapp.miniapp_id,
                "miniapp.serve",
                Wave3CapabilityOperation::MiniAppServe {
                    input: StrictJsonValue(json!({})),
                },
            ))
            .await
            .expect("read serving projection through Wave3");
        assert_eq!(serving.0["miniapp"]["surface_available"], true);
        assert!(serving.0.get("surface_capability").is_none());
    }
}
