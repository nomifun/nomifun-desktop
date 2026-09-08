use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use nomifun_agent_contracts::{
    CanonicalErrorCode, DigestHex, NodeRuntimeFingerprint, PackageId,
    PackageRef, PluginHostCommitFence, PluginHostTargetLock, PluginMountId,
    PluginMountRuntimeContext, PluginStateHandleDescriptor, PluginStateMethod,
    RuntimeInstallationId, RuntimeSelectionRecord, StrictJsonValue,
    RuntimeSwitchValidationResult, ValidatedPluginConfig, VersionString,
};
use nomifun_api_types::{
    ApiResponse, BeginJavascriptRuntimeSwitchRequest,
    ConfirmJavascriptRuntimeDownloadRequest,
    DecideJavascriptRuntimeSwitchRequest, ErrorResponse,
    JavascriptRuntimeStatusDto, ProbeJavascriptRuntimeRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_db::{
    DbError, IJavaScriptRuntimeSelectionRepository,
    JavaScriptRuntimeSelectionRecord, SaveJavaScriptRuntimeSelectionParams,
    SqliteJavaScriptRuntimeSelectionRepository, SqlitePool,
};
use nomifun_js_host::{
    ExtensionHostSupervisor, ImmutablePluginModule, JavaScriptHostConfig,
    MountLoadDemand, materialize_bundled_extension_host,
};
use nomifun_js_authoring::NodeBuildHost;
use nomifun_js_runtime::{
    CoordinatedRuntimeSwitch, JavaScriptRuntimeError,
    JavaScriptRuntimeService, ManagedNodeProvisioner, NodeRuntimeManager,
    ResolvedNodeRuntime, RuntimeAuthority, RuntimeSelectionStore,
    RuntimeSelectionStoreError, RuntimeSwitchCoordinator,
    RuntimeParticipantValidation, RuntimeQuiesceResult,
    RuntimeSwitchParticipant, SystemNodeRuntimeProbePort,
    VersionedRuntimeSelection,
};
use nomifun_plugin_service::PluginRouterState;
use serde_json::json;
use sha2::{Digest, Sha256};

const RUNTIME_DIRECTORY: &str = "javascript-runtime";
const MANAGED_DIRECTORY: &str = "managed";
const FOUNDATION_DIRECTORY: &str = "foundation";
const HOST_DIRECTORY: &str = "host";

#[derive(Clone)]
pub(crate) struct JavaScriptRuntimeRouterState {
    service: Arc<JavaScriptRuntimeService>,
}

impl JavaScriptRuntimeRouterState {
    fn new(service: Arc<JavaScriptRuntimeService>) -> Self {
        Self { service }
    }
}

pub(crate) struct JavaScriptRuntimeFoundation {
    authority: Arc<RuntimeAuthority>,
    probe: Arc<SystemNodeRuntimeProbePort>,
    managed: Arc<ManagedNodeProvisioner>,
    foundation_root: PathBuf,
    host_root: PathBuf,
}

impl JavaScriptRuntimeFoundation {
    pub(crate) fn authority(&self) -> Arc<RuntimeAuthority> {
        Arc::clone(&self.authority)
    }
}

pub(crate) async fn build_javascript_runtime_foundation(
    pool: SqlitePool,
    data_root: PathBuf,
) -> anyhow::Result<JavaScriptRuntimeFoundation> {
    let runtime_root = data_root.join(RUNTIME_DIRECTORY);
    tokio::fs::create_dir_all(&runtime_root).await?;
    let runtime_root = std::fs::canonicalize(runtime_root)?;
    let store = Arc::new(SqliteRuntimeSelectionStore::new(pool));
    let manager = Arc::new(NodeRuntimeManager::new(store));
    let probe = Arc::new(SystemNodeRuntimeProbePort::default());
    let authority = RuntimeAuthority::new(manager, probe.clone());
    authority.initialize_if_empty().await?;
    let managed = Arc::new(ManagedNodeProvisioner::new(
        runtime_root.join(MANAGED_DIRECTORY),
    )?);
    Ok(JavaScriptRuntimeFoundation {
        authority,
        probe,
        managed,
        foundation_root: runtime_root.join(FOUNDATION_DIRECTORY),
        host_root: runtime_root.join(HOST_DIRECTORY),
    })
}

pub(crate) async fn build_javascript_runtime_state(
    foundation: JavaScriptRuntimeFoundation,
    plugin: PluginRouterState,
    plugin_runtime:
        Arc<super::plugin_platform::NomiCorePluginRuntimeParticipant>,
    miniapp_application:
        Arc<nomifun_miniapp_platform::MiniAppM1ApplicationService>,
) -> anyhow::Result<JavaScriptRuntimeRouterState> {
    let participant = Arc::new(NomiCoreRuntimeSwitchParticipant {
        plugin,
        plugin_runtime,
        miniapp_application,
        foundation_root: foundation.foundation_root,
        host_root: foundation.host_root,
    });
    let switch = CoordinatedRuntimeSwitch::new(
        Arc::clone(&foundation.authority),
        vec![participant],
    )?;
    switch.recover_interrupted_switch().await?;
    Ok(JavaScriptRuntimeRouterState::new(Arc::new(
        JavaScriptRuntimeService::new(
            foundation.authority,
            foundation.probe,
            foundation.managed,
            switch,
        ),
    )))
}

struct SqliteRuntimeSelectionStore {
    repository: SqliteJavaScriptRuntimeSelectionRepository,
}

impl SqliteRuntimeSelectionStore {
    fn new(pool: SqlitePool) -> Self {
        Self {
            repository: SqliteJavaScriptRuntimeSelectionRepository::new(pool),
        }
    }
}

#[async_trait]
impl RuntimeSelectionStore for SqliteRuntimeSelectionStore {
    async fn load(
        &self,
    ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
        let record = self.repository.load().await.map_err(store_error)?;
        decode_selection(record)
    }

    async fn save_cas(
        &self,
        expected_revision: u64,
        selection: &RuntimeSelectionRecord,
        selected_executable_path: Option<&Path>,
        pending_candidate_executable_path: Option<&Path>,
        updated_at_ms: i64,
    ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
        selection.validate().map_err(|error| {
            RuntimeSelectionStoreError::Corrupt(error.to_string())
        })?;
        let expected_revision = i64::try_from(expected_revision).map_err(|_| {
            RuntimeSelectionStoreError::Conflict(
                "Runtime selection revision exceeds SQLite i64".into(),
            )
        })?;
        let record = self
            .repository
            .save_cas(&SaveJavaScriptRuntimeSelectionParams {
                expected_revision,
                selected_runtime: encode_selection_value(
                    selection.selected_runtime.as_ref(),
                    "selected_runtime",
                )?,
                selected_executable_path: selected_executable_path
                    .map(|path| path.display().to_string()),
                pending_candidate: encode_selection_value(
                    selection.pending_candidate.as_ref(),
                    "pending_candidate",
                )?,
                pending_candidate_executable_path:
                    pending_candidate_executable_path
                        .map(|path| path.display().to_string()),
                validation_result: encode_selection_value(
                    selection.validation_result.as_ref(),
                    "validation_result",
                )?,
                last_error_code: selection
                    .last_error
                    .as_ref()
                    .map(|code| code.as_ref().to_owned()),
                non_recommended_warning_acknowledged: selection
                    .non_recommended_warning_acknowledged
                    .iter()
                    .map(|id| id.as_ref().to_owned())
                    .collect(),
                updated_at: updated_at_ms,
            })
            .await
            .map_err(store_error)?;
        decode_selection(record)
    }
}

fn decode_selection_value<T>(
    value: Option<serde_json::Value>,
    field: &str,
) -> Result<Option<T>, RuntimeSelectionStoreError>
where
    T: serde::de::DeserializeOwned,
{
    value
        .map(|value| {
            serde_json::from_value(value).map_err(|error| {
                RuntimeSelectionStoreError::Corrupt(format!(
                    "{field} does not match the frozen Runtime contract: {error}"
                ))
            })
        })
        .transpose()
}

fn decode_selection(
    record: JavaScriptRuntimeSelectionRecord,
) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
    let revision = u64::try_from(record.revision).map_err(|_| {
        RuntimeSelectionStoreError::Corrupt(
            "Runtime selection revision is negative".into(),
        )
    })?;
    let selection = RuntimeSelectionRecord {
        selected_runtime: decode_selection_value::<NodeRuntimeFingerprint>(
            record.selected_runtime,
            "selected_runtime",
        )?,
        pending_candidate: decode_selection_value::<NodeRuntimeFingerprint>(
            record.pending_candidate,
            "pending_candidate",
        )?,
        validation_result:
            decode_selection_value::<RuntimeSwitchValidationResult>(
                record.validation_result,
                "validation_result",
            )?,
        last_error: record.last_error_code.map(CanonicalErrorCode::from),
        non_recommended_warning_acknowledged: record
            .non_recommended_warning_acknowledged
            .into_iter()
            .map(RuntimeInstallationId::from)
            .collect(),
    };
    let versioned = VersionedRuntimeSelection {
        selection,
        selected_executable_path: record
            .selected_executable_path
            .map(Into::into),
        pending_candidate_executable_path: record
            .pending_candidate_executable_path
            .map(Into::into),
        revision,
        updated_at_ms: record.updated_at,
    };
    versioned.validate().map_err(|error| {
        RuntimeSelectionStoreError::Corrupt(error.to_string())
    })?;
    Ok(versioned)
}

fn encode_selection_value<T>(
    value: Option<&T>,
    field: &str,
) -> Result<Option<serde_json::Value>, RuntimeSelectionStoreError>
where
    T: serde::Serialize,
{
    value
        .map(|value| {
            serde_json::to_value(value).map_err(|error| {
                RuntimeSelectionStoreError::Corrupt(format!(
                    "{field} cannot be serialized: {error}"
                ))
            })
        })
        .transpose()
}

fn store_error(error: DbError) -> RuntimeSelectionStoreError {
    match error {
        DbError::Conflict(message) => {
            RuntimeSelectionStoreError::Conflict(message)
        }
        DbError::Init(message) => RuntimeSelectionStoreError::Corrupt(message),
        other => RuntimeSelectionStoreError::Unavailable(other.to_string()),
    }
}

pub(crate) fn javascript_runtime_routes(
    state: JavaScriptRuntimeRouterState,
) -> Router {
    Router::new()
        .route("/api/javascript-runtime/status", get(status))
        .route("/api/javascript-runtime/probe", post(probe))
        .route(
            "/api/javascript-runtime/download",
            post(confirm_download),
        )
        .route(
            "/api/javascript-runtime/switch/begin",
            post(begin_switch),
        )
        .route(
            "/api/javascript-runtime/switch/decision",
            post(decide_switch),
        )
        .with_state(state)
}

#[derive(Debug)]
struct JavaScriptRuntimeHttpError(JavaScriptRuntimeError);

impl From<JavaScriptRuntimeError> for JavaScriptRuntimeHttpError {
    fn from(value: JavaScriptRuntimeError) -> Self {
        Self(value)
    }
}

impl IntoResponse for JavaScriptRuntimeHttpError {
    fn into_response(self) -> Response {
        let code = self.0.code();
        let status = match code {
            nomifun_js_runtime::ERR_RUNTIME_INVALID_INPUT => {
                StatusCode::BAD_REQUEST
            }
            nomifun_js_runtime::ERR_RUNTIME_NOT_FOUND => StatusCode::NOT_FOUND,
            nomifun_js_runtime::ERR_RUNTIME_STALE
            | nomifun_js_runtime::ERR_RUNTIME_BUSY
            | nomifun_js_runtime::ERR_RUNTIME_DOWNLOAD_OFFER_STALE
            | nomifun_js_runtime::ERR_RUNTIME_DOWNLOAD_RUNNING
            | nomifun_js_runtime::ERR_RUNTIME_NON_RECOMMENDED_CONFIRMATION => {
                StatusCode::CONFLICT
            }
            nomifun_js_runtime::ERR_RUNTIME_FOUNDATION_FAILED => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            nomifun_js_runtime::ERR_RUNTIME_NOT_COVERED
            | nomifun_js_runtime::ERR_RUNTIME_UNAVAILABLE => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(ErrorResponse::new(self.0.to_string(), code)),
        )
            .into_response()
    }
}

async fn status(
    State(state): State<JavaScriptRuntimeRouterState>,
) -> Result<
    Json<ApiResponse<JavascriptRuntimeStatusDto>>,
    JavaScriptRuntimeHttpError,
> {
    Ok(Json(ApiResponse::ok(state.service.status().await?)))
}

async fn probe(
    State(state): State<JavaScriptRuntimeRouterState>,
    Json(request): Json<ProbeJavascriptRuntimeRequest>,
) -> Result<
    Json<ApiResponse<JavascriptRuntimeStatusDto>>,
    JavaScriptRuntimeHttpError,
> {
    Ok(Json(ApiResponse::ok(
        state.service.probe(request).await?,
    )))
}

async fn confirm_download(
    State(state): State<JavaScriptRuntimeRouterState>,
    Json(request): Json<ConfirmJavascriptRuntimeDownloadRequest>,
) -> Result<
    Json<ApiResponse<JavascriptRuntimeStatusDto>>,
    JavaScriptRuntimeHttpError,
> {
    Ok(Json(ApiResponse::ok(
        state.service.confirm_download(request).await?,
    )))
}

async fn begin_switch(
    State(state): State<JavaScriptRuntimeRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<BeginJavascriptRuntimeSwitchRequest>,
) -> Result<
    Json<ApiResponse<JavascriptRuntimeStatusDto>>,
    JavaScriptRuntimeHttpError,
> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .begin_switch(user.id.as_str(), request)
            .await?,
    )))
}

async fn decide_switch(
    State(state): State<JavaScriptRuntimeRouterState>,
    Json(request): Json<DecideJavascriptRuntimeSwitchRequest>,
) -> Result<
    Json<ApiResponse<JavascriptRuntimeStatusDto>>,
    JavaScriptRuntimeHttpError,
> {
    Ok(Json(ApiResponse::ok(
        state.service.decide_switch(request).await?,
    )))
}

struct NomiCoreRuntimeSwitchParticipant {
    plugin: PluginRouterState,
    plugin_runtime:
        Arc<super::plugin_platform::NomiCorePluginRuntimeParticipant>,
    miniapp_application:
        Arc<nomifun_miniapp_platform::MiniAppM1ApplicationService>,
    foundation_root: PathBuf,
    host_root: PathBuf,
}

fn miniapp_service_participant_result(
    passed: bool,
) -> nomifun_agent_contracts::RuntimeSwitchParticipantResult {
    nomifun_agent_contracts::RuntimeSwitchParticipantResult {
        kind: nomifun_agent_contracts::RuntimeSwitchParticipantKind::MiniappService,
        owner_id: "miniapp-production-host".to_owned(),
        outcome: if passed {
            nomifun_agent_contracts::RuntimeSwitchParticipantOutcome::Passed
        } else {
            nomifun_agent_contracts::RuntimeSwitchParticipantOutcome::Failed
        },
        error_code: (!passed).then(|| {
            nomifun_agent_contracts::CanonicalErrorCode::from(
                "MINIAPP_SERVICE_RUNTIME_VALIDATION_FAILED",
            )
        }),
    }
}

#[async_trait]
impl RuntimeSwitchParticipant for NomiCoreRuntimeSwitchParticipant {
    async fn quiesce_and_stop(
        &self,
        owner_user_id: &str,
        _selected: Option<&ResolvedNodeRuntime>,
    ) -> Result<RuntimeQuiesceResult, JavaScriptRuntimeError> {
        let operations = self
            .plugin
            .service
            .list_operations(owner_user_id)
            .await
            .map_err(|error| {
                JavaScriptRuntimeError::SwitchNotCovered(format!(
                    "Plugin operation inventory is unavailable: {error}"
                ))
            })?;
        let running = operations
            .into_iter()
            .filter(|operation| {
                operation.state
                    == nomifun_api_types::DurableOperationStateDto::Running
            })
            .map(|operation| operation.operation_id)
            .collect::<Vec<_>>();
        if !running.is_empty() {
            return Err(JavaScriptRuntimeError::SwitchBusy(format!(
                "Plugin operations are running: {}",
                running.join(", ")
            )));
        }
        self.miniapp_application
            .shutdown_service_runtime(owner_user_id)
            .await
            .map_err(|error| {
                JavaScriptRuntimeError::SwitchBusy(format!(
                    "MiniApp Service Hosts could not be stopped: {error}"
                ))
            })?;
        self.plugin_runtime
            .stop_for_runtime_switch()
            .await
            .map(|()| RuntimeQuiesceResult {
                old_runtime_process_tree_zero: true,
            })
    }

    async fn validate_candidate(
        &self,
        owner_user_id: &str,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<RuntimeParticipantValidation, JavaScriptRuntimeError> {
        validate_foundation(
            &self.foundation_root,
            &self.host_root,
            &nomifun_js_runtime::RuntimeCandidate {
                fingerprint: candidate.fingerprint.clone(),
                executable_path: candidate.executable_path.clone(),
                disposition: if candidate.fingerprint.node_major
                    == nomifun_agent_contracts::RECOMMENDED_NODE_LTS_MAJOR
                {
                    nomifun_agent_contracts::NodeProbeDisposition::CompatibleRecommended
                } else {
                    nomifun_agent_contracts::NodeProbeDisposition::CompatibleNonRecommended
                },
            },
        )
        .await?;

        let build_validation = NodeBuildHost::new(
            &candidate.executable_path,
            Duration::from_secs(30),
        )
        .and_then(|host| host.validate_foundation());
        let build_passed = build_validation.is_ok();
        let mut results =
            vec![nomifun_agent_contracts::RuntimeSwitchParticipantResult {
                kind: nomifun_agent_contracts::RuntimeSwitchParticipantKind::BuildFoundation,
                owner_id: "javascript-build-foundation".to_owned(),
                outcome: if build_passed {
                    nomifun_agent_contracts::RuntimeSwitchParticipantOutcome::Passed
                } else {
                    nomifun_agent_contracts::RuntimeSwitchParticipantOutcome::Failed
                },
                error_code: (!build_passed).then(|| {
                    nomifun_agent_contracts::CanonicalErrorCode::from(
                        "JAVASCRIPT_BUILD_FOUNDATION_FAILED",
                    )
                }),
            }];
        results.extend(
            self.plugin_runtime
                .validate_candidate(owner_user_id, candidate)
                .await?,
        );
        let miniapp_service_validation = self
            .miniapp_application
            .validate_service_runtime_candidate(owner_user_id, candidate)
            .await;
        results.push(miniapp_service_participant_result(
            miniapp_service_validation.is_ok(),
        ));
        Ok(RuntimeParticipantValidation {
            foundation_hello_passed: true,
            participants: results,
        })
    }

    async fn prepare_candidate(
        &self,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<(), JavaScriptRuntimeError> {
        self.plugin_runtime.prepare_runtime(Some(candidate)).await
    }

    async fn finalize_candidate(
        &self,
        _candidate: &ResolvedNodeRuntime,
    ) -> Result<(), JavaScriptRuntimeError> {
        self.plugin_runtime.finalize_runtime().await
    }

    async fn restore_selected(
        &self,
        selected: Option<&ResolvedNodeRuntime>,
    ) -> Result<(), JavaScriptRuntimeError> {
        self.plugin_runtime.restore_runtime(selected).await
    }
}

#[allow(dead_code)]
async fn validate_foundation(
    foundation_root: &Path,
    host_root: &Path,
    candidate: &nomifun_js_runtime::RuntimeCandidate,
) -> Result<(), JavaScriptRuntimeError> {
    let run_root = foundation_root.join(uuid::Uuid::now_v7().to_string());
    tokio::fs::create_dir_all(&run_root).await.map_err(|error| {
        JavaScriptRuntimeError::FoundationValidationFailed(format!(
            "cannot create foundation directory: {error}"
        ))
    })?;
    let cleanup = FoundationCleanup(run_root.clone());
    let module_path = run_root.join("main.mjs");
    let module_bytes =
        b"export async function activate() { return {}; }\n";
    tokio::fs::write(&module_path, module_bytes)
        .await
        .map_err(|error| {
            JavaScriptRuntimeError::FoundationValidationFailed(format!(
                "cannot write foundation module: {error}"
            ))
        })?;
    let module_path = std::fs::canonicalize(&module_path).map_err(|error| {
        JavaScriptRuntimeError::FoundationValidationFailed(format!(
            "cannot canonicalize foundation module: {error}"
        ))
    })?;
    let data_dir = run_root.join("data");
    tokio::fs::create_dir_all(&data_dir).await.map_err(|error| {
        JavaScriptRuntimeError::FoundationValidationFailed(format!(
            "cannot create foundation data directory: {error}"
        ))
    })?;
    let data_dir = std::fs::canonicalize(&data_dir).map_err(|error| {
        JavaScriptRuntimeError::FoundationValidationFailed(format!(
            "cannot canonicalize foundation data directory: {error}"
        ))
    })?;
    let host_module =
        materialize_bundled_extension_host(host_root).map_err(|error| {
            JavaScriptRuntimeError::FoundationValidationFailed(
                error.to_string(),
            )
        })?;
    let supervisor = ExtensionHostSupervisor::candidate_test(
        JavaScriptHostConfig::for_host_module(
            candidate.executable_path.clone(),
            candidate.fingerprint.clone(),
            host_module,
        ),
    )
    .map_err(|error| {
        JavaScriptRuntimeError::FoundationValidationFailed(error.to_string())
    })?;
    let target = foundation_target();
    let generation = supervisor
        .load_mount(MountLoadDemand {
            context: foundation_context(target.clone(), &data_dir),
            module: ImmutablePluginModule::new(
                module_path,
                DigestHex::from(hex_digest(module_bytes)),
                target,
            ),
        })
        .await
        .map_err(|error| {
            JavaScriptRuntimeError::FoundationValidationFailed(
                error.to_string(),
            )
        })?;
    let fence = supervisor
        .stop_generation(generation)
        .await
        .map_err(|error| {
            JavaScriptRuntimeError::FoundationValidationFailed(
                error.to_string(),
            )
        })?;
    if !matches!(fence, PluginHostCommitFence::ResidentFenced { .. })
        || supervisor.process_count() != 0
    {
        return Err(JavaScriptRuntimeError::FoundationValidationFailed(
            "foundation Host did not prove process-tree cleanup".into(),
        ));
    }
    drop(cleanup);
    Ok(())
}

fn foundation_target() -> PluginHostTargetLock {
    PluginHostTargetLock {
        mount_id: PluginMountId::from("runtime-foundation"),
        package: PackageRef {
            id: PackageId::from("dev.nomifun.runtime-foundation"),
            version: VersionString::from("1.0.0"),
        },
        artifact_digest: DigestHex::from("a".repeat(64)),
        manifest_digest: DigestHex::from("b".repeat(64)),
    }
}

fn foundation_context(
    target: PluginHostTargetLock,
    data_dir: &Path,
) -> PluginMountRuntimeContext {
    PluginMountRuntimeContext {
        target: target.clone(),
        mount_handle_id: "runtime-foundation-handle".into(),
        config: ValidatedPluginConfig {
            schema_digest: DigestHex::from("c".repeat(64)),
            config_revision: 1,
            value: StrictJsonValue(json!({})),
        },
        credential_bindings: Vec::new(),
        state: PluginStateHandleDescriptor {
            package_id: target.package.id.clone(),
            mount_id: target.mount_id.clone(),
            methods: PluginStateMethod::REQUIRED
                .into_iter()
                .collect::<BTreeSet<_>>(),
        },
        data_dir: data_dir.display().to_string(),
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct FoundationCleanup(PathBuf);

impl Drop for FoundationCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        JAVASCRIPT_HOST_PROTOCOL_VERSION, JAVASCRIPT_SDK_CONTRACT_VERSION,
        NodeRuntimeSourceKind, RuntimeTarget,
    };

    #[test]
    fn runtime_http_errors_preserve_typed_switch_boundaries() {
        let busy = JavaScriptRuntimeHttpError(
            JavaScriptRuntimeError::SwitchBusy("build-1".into()),
        )
        .into_response();
        assert_eq!(busy.status(), StatusCode::CONFLICT);

        let uncovered = JavaScriptRuntimeHttpError(
            JavaScriptRuntimeError::SwitchNotCovered(
                "global Runtime switch coordination is not wired".into(),
            ),
        )
        .into_response();
        assert_eq!(
            uncovered.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn miniapp_service_switch_participant_is_covered_or_failed() {
        let passed = miniapp_service_participant_result(true);
        assert_eq!(
            passed.outcome,
            nomifun_agent_contracts::RuntimeSwitchParticipantOutcome::Passed
        );
        assert_eq!(passed.error_code, None);

        let failed = miniapp_service_participant_result(false);
        assert_eq!(
            failed.outcome,
            nomifun_agent_contracts::RuntimeSwitchParticipantOutcome::Failed
        );
        assert_eq!(
            failed.error_code.as_ref().map(AsRef::as_ref),
            Some("MINIAPP_SERVICE_RUNTIME_VALIDATION_FAILED")
        );
    }

    #[test]
    fn route_tree_exposes_only_the_frozen_runtime_actions() {
        let source = include_str!("javascript_runtime.rs");
        for path in [
            "/api/javascript-runtime/status",
            "/api/javascript-runtime/probe",
            "/api/javascript-runtime/download",
            "/api/javascript-runtime/switch/begin",
            "/api/javascript-runtime/switch/decision",
        ] {
            assert!(source.contains(path), "missing route {path}");
        }
        let routes = include_str!("routes.rs");
        let runtime_group = routes
            .split_once("let javascript_runtime_authenticated")
            .expect("Runtime routes must have a protected route group")
            .1
            .split_once("// Unified agent listing")
            .expect("Runtime route group must end before Agent routes")
            .0;
        assert!(runtime_group.contains("protect_instance_owner("));
        assert!(runtime_group.contains("require_local_trust_middleware"));
    }

    #[tokio::test]
    async fn sqlite_adapter_preserves_typed_selection_path_and_exact_cas() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let store =
            SqliteRuntimeSelectionStore::new(database.pool().clone());
        let runtime = NodeRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from(
                "node-managed",
            ),
            source_kind: NodeRuntimeSourceKind::Managed,
            node_version: VersionString::from("24.8.0"),
            node_major: 24,
            runtime_target: RuntimeTarget::from(
                "x86_64-pc-windows-msvc",
            ),
            executable_digest: DigestHex::from("a".repeat(64)),
            javascript_host_protocol_version:
                JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version:
                JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
        };
        let selection = RuntimeSelectionRecord {
            selected_runtime: Some(runtime),
            pending_candidate: None,
            validation_result: None,
            last_error: None,
            non_recommended_warning_acknowledged: BTreeSet::new(),
        };
        let executable = std::env::current_dir().unwrap().join("node.exe");
        let saved = store
            .save_cas(0, &selection, Some(&executable), None, 42)
            .await
            .unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.selection, selection);
        assert_eq!(
            saved.selected_executable_path.as_deref(),
            Some(executable.as_path())
        );

        let stale = store
            .save_cas(0, &selection, Some(&executable), None, 43)
            .await
            .unwrap_err();
        assert!(matches!(
            stale,
            RuntimeSelectionStoreError::Conflict(message)
                if message.contains("revision CAS")
        ));
    }
}
