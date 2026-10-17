//! Platform-owned workspace.files/read projection. Only verified workspace bytes reach the
//! shared bounded image decoder; neither the model nor an engine opens paths.
use nomi_native_files::{NativeReadAcquisition, NativeReadRequest, NativeSource};
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::ChatToolResultPart;
use nomifun_common::AppError;
use nomifun_engine_core::EngineExecutionIdentity;
use nomifun_file::{AgentSessionWorkspaceBinding, AgentTextReadRequest, FileService};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Retained by the blocking decode task, even if its async caller is dropped.
static IMAGE_READS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageRead {
    path: String,
    #[serde(default)]
    expected_sha256: Option<String>,
}

/// Internal canonical owner result, converted to typed pixels before the
/// shared engine journal. Not a generic plugin-controlled image discriminator.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkspaceImage {
    pub kind: String,
    pub path: String,
    pub source_sha256: String,
    pub source_bytes: usize,
    pub media_type: String,
    pub data_base64: String,
}

pub(super) async fn read(
    files: &FileService,
    scope: &AgentSessionWorkspaceBinding,
    input: StrictJsonValue,
) -> Result<Option<StrictJsonValue>, AppError> {
    read_with_acquirer(files, scope, input, &system_acquire).await
}

async fn system_acquire(request: NativeReadRequest) -> Result<NativeReadAcquisition, AppError> {
    let client = nomi_system_execution::SystemExecutionClient::connect_for_read()
        .await
        .map_err(system_read_error)?;
    let receipt = client
        .acquire_read(request)
        .await
        .map_err(system_read_error)?;
    Ok(receipt.acquisition)
}

fn system_read_error(error: nomi_system_execution::SystemExecutionError) -> AppError {
    if let Some(native) = error.details.get("native_file_error") {
        return match serde_json::from_value::<nomi_native_files::NativeFileFailure>(native.clone()) {
            Ok(failure) if failure.code == error.code => nomifun_file::native_file_failure_error(failure),
            _ => AppError::Internal("Native readonly owner returned an inconsistent typed failure".into()),
        };
    }
    let message = format!("{}: {}", error.code, error.message);
    match error.code.as_str() {
        "SYSTEM_ACCESS_DENIED" => AppError::NativeSystemAccessDenied {
            operation: message,
            source: error
                .details
                .pointer("/native_file_error/raw_os_error")
                .and_then(serde_json::Value::as_i64)
                .and_then(|code| i32::try_from(code).ok())
                .map(std::io::Error::from_raw_os_error)
                .unwrap_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Native system readonly access was refused",
                    )
                }),
        },
        "INVALID_PAYLOAD" | "invalid_system_read_request" => AppError::BadRequest(message),
        "FILE_CHANGED_DURING_READ" => AppError::Conflict(message),
        "RESOURCE_NOT_FOUND" => AppError::NotFound(message),
        "SYSTEM_EXECUTION_NOT_AVAILABLE"
        | "SYSTEM_EXECUTION_APPROVAL_REQUIRED"
        | "SYSTEM_EXECUTION_AUTHENTICATION_UNAVAILABLE"
        | "NATIVE_FILE_ACQUIRE_UNAVAILABLE" => AppError::ComponentUnavailable {
            component: "system-execution".into(),
            reason: message,
        },
        "SYSTEM_EXECUTION_AUTHENTICATION_DENIED"
        | "SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED" => AppError::Forbidden(message),
        "SYSTEM_EXECUTION_AUTHENTICATION_TIMED_OUT" => AppError::Timeout(message),
        _ => AppError::Internal(format!("{message}; native read details: {}", error.details)),
    }
}

fn absolute_read_path(
    scope: &AgentSessionWorkspaceBinding,
    path: &str,
) -> Result<String, AppError> {
    scope
        .resolve_path(path)?
        .into_os_string()
        .into_string()
        .map_err(|_| AppError::BadRequest("Native readonly path is not UTF-8".into()))
}

async fn acquire_source<F, Fut>(
    scope: &AgentSessionWorkspaceBinding,
    path: &str,
    max_bytes: usize,
    acquire: &F,
) -> Result<Option<NativeSource>, AppError>
where
    F: Fn(NativeReadRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<NativeReadAcquisition, AppError>> + Send,
{
    match acquire(NativeReadRequest::Source {
        path: absolute_read_path(scope, path)?,
        max_bytes,
    })
    .await?
    {
        NativeReadAcquisition::Source { source } => {
            if source.as_ref().is_some_and(|source| {
                source.bytes.len() > max_bytes
                    || !std::path::Path::new(&source.canonical_path).is_absolute()
                    || source.canonical_path.len() > 4096
                    || source.canonical_path.contains('\0')
                    || source.sha256 != format!("{:x}", Sha256::digest(&source.bytes))
            }) {
                return Err(AppError::Internal("Native readonly acquisition differs from its bounded source/digest/path contract".into()));
            }
            Ok(source)
        }
        _ => Err(AppError::Internal(
            "Native readonly owner returned the wrong acquisition kind".into(),
        )),
    }
}

async fn read_with_acquirer<F, Fut>(
    files: &FileService,
    scope: &AgentSessionWorkspaceBinding,
    input: StrictJsonValue,
    acquire: &F,
) -> Result<Option<StrictJsonValue>, AppError>
where
    F: Fn(NativeReadRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<NativeReadAcquisition, AppError>> + Send,
{
    scope.require_operation(nomifun_file::WORKSPACE_READ_OPERATION)?;
    let mut input = input.0;
    let object = input
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("workspace.files/read requires an object".into()))?;
    let identity = object
        .remove("execution_identity")
        .map(serde_json::from_value::<EngineExecutionIdentity>)
        .transpose()
        .map_err(|error| AppError::BadRequest(error.to_string()))?
        .unwrap_or_default();
    let format = object.remove("format");
    let kind = format.as_ref().and_then(|value| value.as_str());
    // Explicit neutral defaults are equivalent to omission, even when a
    // generic tool caller includes them for another read format.
    if kind != Some("instruction_scope")
        && object.get("recursive") == Some(&serde_json::Value::Bool(false))
    {
        object.remove("recursive");
    }
    if matches!(kind, Some("image" | "instruction_scope"))
        && object.get("missing_ok") == Some(&serde_json::Value::Bool(false))
    {
        object.remove("missing_ok");
    }
    match format.as_ref().and_then(|value| value.as_str()) {
        None if format.is_none() => {}
        Some("text") => {}
        Some("image") => return read_image(files, scope, input, identity, acquire).await,
        Some("instruction_scope") => {
            let request: nomifun_file::AgentInstructionScopeRequest = serde_json::from_value(input)
                .map_err(|error| AppError::BadRequest(error.to_string()))?;
            if identity == EngineExecutionIdentity::Administrator {
                nomifun_file::validate_agent_instruction_scope_request(&request)?;
                let response = acquire(NativeReadRequest::InstructionScope {
                    path: absolute_read_path(scope, &request.path)?,
                    recursive: request.recursive,
                    path_semantics: request.path_semantics,
                })
                .await?;
                let NativeReadAcquisition::InstructionScope { scope: native } = response else {
                    return Err(AppError::Internal(
                        "Native readonly owner returned the wrong scope kind".into(),
                    ));
                };
                let projected = nomifun_file::project_agent_instruction_scope(
                    request,
                    native,
                    scope.workspace_root(),
                )?;
                return serde_json::to_value(projected)
                    .map(StrictJsonValue)
                    .map(Some)
                    .map_err(|error| AppError::Internal(error.to_string()));
            }
            return files
                .instruction_scope_for_agent_session(scope, request)
                .await
                .and_then(|scope| {
                    serde_json::to_value(scope)
                        .map(StrictJsonValue)
                        .map(Some)
                        .map_err(|error| AppError::Internal(error.to_string()))
                });
        }
        _ => {
            return Err(AppError::BadRequest(
                "workspace.files/read format must be text, image or instruction_scope".into(),
            ));
        }
    }
    let missing_ok = match input
        .as_object_mut()
        .and_then(|object| object.remove("missing_ok"))
    {
        None | Some(serde_json::Value::Bool(false)) => false,
        Some(serde_json::Value::Bool(true)) => true,
        _ => return Err(AppError::BadRequest("missing_ok must be a boolean".into())),
    };
    let request: AgentTextReadRequest =
        serde_json::from_value(input).map_err(|error| AppError::BadRequest(error.to_string()))?;
    let requested_path = request.path.clone();
    let page = if identity == EngineExecutionIdentity::Administrator {
        nomifun_file::validate_agent_text_read_request(&request)?;
        acquire_source(
            scope,
            &request.path,
            nomi_native_files::MAX_SOURCE_BYTES,
            acquire,
        )
        .await?
        .map(|source| {
            nomifun_file::project_agent_text_page(
                request,
                source.bytes,
                source.sha256,
                scope.workspace_root(),
                std::path::Path::new(&source.canonical_path),
            )
        })
        .transpose()?
    } else {
        files
            .read_text_page_for_agent_session(scope, request)
            .await?
    };
    if page.is_none() && missing_ok {
        return Ok(Some(StrictJsonValue(
            serde_json::json!({"kind":"workspace_file_absent", "path":requested_path}),
        )));
    }
    page.map(|page| {
        serde_json::to_value(page)
            .map(StrictJsonValue)
            .map_err(|error| AppError::Internal(error.to_string()))
    })
    .transpose()
}

async fn read_image<F, Fut>(
    files: &FileService,
    scope: &AgentSessionWorkspaceBinding,
    input: serde_json::Value,
    identity: EngineExecutionIdentity,
    acquire: &F,
) -> Result<Option<StrictJsonValue>, AppError>
where
    F: Fn(NativeReadRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<NativeReadAcquisition, AppError>> + Send,
{
    // Image reads have no byte pages: even offset:null/limit:null are unknown
    // fields here, so they cannot be silently interpreted as text paging.
    let request: ImageRead =
        serde_json::from_value(input).map_err(|error| AppError::BadRequest(error.to_string()))?;
    if request.expected_sha256.as_ref().is_some_and(|digest| {
        digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(AppError::BadRequest(
            "expected_sha256 must be a lowercase SHA-256 digest".into(),
        ));
    }
    let extension = std::path::Path::new(&request.path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
        return Err(AppError::BadRequest(
            "Workspace images support PNG, JPEG and WebP only".into(),
        ));
    }
    let permit = IMAGE_READS
        .acquire()
        .await
        .map_err(|_| AppError::Conflict("Image reader is closed".into()))?;
    let source = if identity == EngineExecutionIdentity::Administrator {
        acquire_source(scope, &request.path, 4 * 1024 * 1024, acquire)
            .await?
            .map(|source| (source.bytes, source.sha256))
    } else {
        files
            .read_bytes_for_agent_session(scope, &request.path, 4 * 1024 * 1024)
            .await?
    };
    let Some((bytes, source_sha256)) = source else {
        return Ok(None);
    };
    if request
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| expected != &source_sha256)
    {
        return Err(AppError::Conflict("FILE_CONTENT_CHANGED: workspace image differs from the expected source digest; no pixels returned".into()));
    }
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let ChatToolResultPart::Image {
            media_type,
            data_base64,
        } = nomifun_ai_agent::model_attachments::prepare_image_resource(&bytes, &request.path)
            .map_err(|error| AppError::BadRequest(error.to_string()))?
        else {
            return Err(AppError::Internal(
                "Image decoder returned a non-image result".into(),
            ));
        };
        if data_base64.is_empty() || data_base64.len() > 2 * 1024 * 1024 {
            return Err(AppError::BadRequest(
                "Prepared workspace image exceeds the transport budget".into(),
            ));
        }
        serde_json::to_value(WorkspaceImage {
            kind: "workspace_image".into(),
            path: request.path.clone(),
            source_sha256,
            source_bytes: bytes.len(),
            media_type,
            data_base64,
        })
        .map(|value| Some(StrictJsonValue(value)))
        .map_err(|error| AppError::Internal(error.to_string()))
    })
    .await
    .map_err(|error| AppError::Internal(format!("workspace image preparation failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::WebSocketMessage;
    use nomifun_file::{WORKSPACE_READ_OPERATION, workspace_binding};
    use nomifun_realtime::UserEventSink;
    use std::sync::Arc;

    struct Events;
    impl UserEventSink for Events {
        fn send_to_user(&self, _: &str, _: WebSocketMessage<serde_json::Value>) {}
    }

    #[tokio::test]
    async fn native_image_paths_preserve_exact_spelling_inside_and_outside_default_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let external = fixture.path().join("外部 image.png");
        let image = image::RgbImage::from_pixel(1, 1, image::Rgb([10, 20, 30]));
        image
            .save_with_format(&external, image::ImageFormat::Png)
            .unwrap();
        image
            .save_with_format(workspace.join(" leading.png"), image::ImageFormat::Png)
            .unwrap();
        let scope = workspace_binding(
            nomifun_common::generate_id(),
            "binding",
            "project",
            "owner",
            [WORKSPACE_READ_OPERATION],
            &workspace,
        )
        .unwrap();
        let files = FileService::new(Arc::new(Events), vec![workspace]);
        for path in [
            external.to_str().unwrap(),
            "../外部 image.png",
            " leading.png",
        ] {
            let result = read(
                &files,
                &scope,
                StrictJsonValue(serde_json::json!({"path":path,"format":"image"})),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result.0["path"], path);
            assert_eq!(result.0["kind"], "workspace_image");
            assert_eq!(result.0["media_type"], "image/png");
        }
    }

    #[tokio::test]
    async fn administrator_read_identity_reuses_text_scope_and_image_owner_projections() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let external = fixture.path().join("external");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&external).unwrap();
        let text = external.join("AGENTS.md");
        std::fs::write(&text, "规则中文\r\nsecond line\n").unwrap();
        let image_path = external.join("image.png");
        image::RgbImage::from_pixel(1, 1, image::Rgb([1, 2, 3]))
            .save_with_format(&image_path, image::ImageFormat::Png)
            .unwrap();
        let scope = workspace_binding(
            nomifun_common::generate_id(),
            "binding",
            "project",
            "owner",
            [WORKSPACE_READ_OPERATION],
            &workspace,
        )
        .unwrap();
        let files = FileService::new(Arc::new(Events), vec![workspace]);
        let invoked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = invoked.clone();
        let acquire = move |request| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                nomi_native_files::acquire(request)
                    .map_err(nomifun_file::native_file_acquisition_error)
            }
        };
        for mut input in [
            serde_json::json!({"path":text,"offset":0,"limit":8}),
            serde_json::json!({"path":external,"format":"instruction_scope"}),
            serde_json::json!({"path":external,"format":"instruction_scope","path_semantics":"entry"}),
            serde_json::json!({"path":image_path,"format":"image"}),
        ] {
            let ordinary = read(&files, &scope, StrictJsonValue(input.clone()))
                .await
                .unwrap()
                .unwrap();
            input["execution_identity"] = serde_json::json!("administrator");
            let native = read_with_acquirer(&files, &scope, StrictJsonValue(input), &acquire)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ordinary.0, native.0);
        }
        assert_eq!(invoked.load(std::sync::atomic::Ordering::Relaxed), 4);
        let explicit = read_with_acquirer(
            &files,
            &scope,
            StrictJsonValue(serde_json::json!({"path":text,"execution_identity":"current_user"})),
            &|_| async {
                panic!("ordinary identity must not enter the native administrator owner")
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(explicit.0["content"], "规则中文\r\nsecond line\n");
    }

    #[tokio::test]
    async fn administrator_entry_scope_rejects_a_follow_receipt_without_downgrading_identity() {
        let root = tempfile::tempdir().unwrap();
        let scope = workspace_binding(nomifun_common::generate_id(), "binding", "project", "owner",
            [WORKSPACE_READ_OPERATION], root.path()).unwrap();
        let files = FileService::new(Arc::new(Events), vec![root.path().to_owned()]);
        let result = read_with_acquirer(&files, &scope, StrictJsonValue(serde_json::json!({
            "path":".","format":"instruction_scope","execution_identity":"administrator","path_semantics":"entry"
        })), &|request| async move {
            let NativeReadRequest::InstructionScope { path, recursive, path_semantics } = request else {
                panic!("entry metadata must retain its typed acquisition kind");
            };
            assert_eq!(path_semantics, nomi_native_files::NativePathSemantics::Entry);
            nomi_native_files::acquire(NativeReadRequest::InstructionScope {
                path, recursive, path_semantics: nomi_native_files::NativePathSemantics::Follow,
            }).map_err(nomifun_file::native_file_acquisition_error)
        }).await.unwrap_err();
        assert!(matches!(result, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn administrator_missing_ok_never_converts_denied_or_invalid_receipts_to_absence() {
        let root = tempfile::tempdir().unwrap();
        let scope = workspace_binding(
            nomifun_common::generate_id(),
            "binding",
            "project",
            "owner",
            [WORKSPACE_READ_OPERATION],
            root.path(),
        )
        .unwrap();
        let files = FileService::new(Arc::new(Events), vec![root.path().to_owned()]);
        let input = serde_json::json!({"path":"missing.md","missing_ok":true,"execution_identity":"administrator"});
        let absent = read_with_acquirer(
            &files,
            &scope,
            StrictJsonValue(input.clone()),
            &|request| async move {
                nomi_native_files::acquire(request)
                    .map_err(nomifun_file::native_file_acquisition_error)
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            absent.0,
            serde_json::json!({"kind":"workspace_file_absent","path":"missing.md"})
        );
        let denied =
            read_with_acquirer(&files, &scope, StrictJsonValue(input.clone()), &|_| async {
                Err(AppError::NativeSystemAccessDenied {
                    operation: "test root target".into(),
                    source: std::io::Error::from_raw_os_error(13),
                })
            })
            .await
            .unwrap_err();
        assert_eq!(denied.error_code(), "SYSTEM_ACCESS_DENIED");
        let source_path = root.path().join("source.md").to_str().unwrap().to_owned();
        let corrupt = read_with_acquirer(&files, &scope, StrictJsonValue(input), &|_| {
            let path = source_path.clone();
            async move {
                Ok(NativeReadAcquisition::Source {
                    source: Some(NativeSource {
                        canonical_path: path,
                        bytes: b"actual".to_vec(),
                        sha256: "0".repeat(64),
                    }),
                })
            }
        })
        .await
        .unwrap_err();
        assert!(matches!(corrupt, AppError::Internal(_)));
        let invalid = read_with_acquirer(
            &files,
            &scope,
            StrictJsonValue(serde_json::json!({"path":"x","execution_identity":"root"})),
            &|_| async { panic!("invalid selector must not dispatch") },
        )
        .await
        .unwrap_err();
        assert!(matches!(invalid, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn native_not_found_failure_is_never_an_absence_receipt_or_missing_scope() {
        let root=tempfile::tempdir().unwrap();
        let scope=workspace_binding(nomifun_common::generate_id(),"binding","project","owner",[WORKSPACE_READ_OPERATION],root.path()).unwrap();
        let files=FileService::new(Arc::new(Events),vec![root.path().to_owned()]);
        let failure=nomi_native_files::NativeFileFailure{code:"RESOURCE_NOT_FOUND".into(),message:"resolving native read root: unavailable namespace".into(),kind:Some("NotFound".into()),raw_os_error:Some(67)};
        for input in [serde_json::json!({"path":"missing.md","missing_ok":true,"execution_identity":"administrator"}),serde_json::json!({"path":"missing.md","format":"instruction_scope","execution_identity":"administrator"})] {
            let expected=serde_json::to_value(&failure).unwrap();let copy=failure.clone();
            let result=read_with_acquirer(&files,&scope,StrictJsonValue(input),&move |_| {
                let native=copy.clone();async move {Err(system_read_error(nomi_system_execution::SystemExecutionError{code:native.code.clone(),message:native.message.clone(),details:serde_json::json!({"native_file_error":native,"worker":{"state":"exited","exit_code":0,"cleanup":{"reaped":true}},"cleanup_proven":true})}))}
            }).await.unwrap_err();
            assert_eq!(result.error_code(),"RESOURCE_NOT_FOUND");assert_eq!(result.error_details().unwrap()["native_file_error"],expected);
            assert!(matches!(result,AppError::NativeFileAcquisition{source:None,..}));
        }
        let ordinary=read_with_acquirer(&files,&scope,StrictJsonValue(serde_json::json!({"path":"missing.md","missing_ok":true})),&|_|async{panic!("ordinary reader must retain its own identity")}).await.unwrap().unwrap();
        assert_eq!(ordinary.0,serde_json::json!({"kind":"workspace_file_absent","path":"missing.md"}));
    }

    #[test]
    fn inconsistent_native_error_receipts_do_not_adopt_a_transport_code_or_invent_os_fields() {
        let error=system_read_error(nomi_system_execution::SystemExecutionError{code:"SYSTEM_ACCESS_DENIED".into(),message:"transport text".into(),details:serde_json::json!({"native_file_error":{"code":"RESOURCE_NOT_FOUND","message":"native text","kind":"NotFound","raw_os_error":67}})});
        assert!(matches!(error,AppError::Internal(_)));
    }
}
