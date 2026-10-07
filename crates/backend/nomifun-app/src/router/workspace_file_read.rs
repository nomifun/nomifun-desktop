//! Platform-owned workspace.files/read projection. Only verified workspace bytes reach the
//! shared bounded image decoder; neither the model nor an engine opens paths.
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::ChatToolResultPart;
use nomifun_common::AppError;
use nomifun_file::{AgentSessionWorkspaceBinding, AgentTextReadRequest, FileService};
use serde::{Deserialize, Serialize};

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
    let mut input = input.0;
    let object = input
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("workspace.files/read requires an object".into()))?;
    let format = object.remove("format");
    let kind=format.as_ref().and_then(|value| value.as_str());
    // Explicit neutral defaults are equivalent to omission, even when a
    // generic tool caller includes them for another read format.
    if kind != Some("instruction_scope") && object.get("recursive") == Some(&serde_json::Value::Bool(false)) {
        object.remove("recursive");
    }
    if matches!(kind,Some("image"|"instruction_scope")) && object.get("missing_ok") == Some(&serde_json::Value::Bool(false)) {
        object.remove("missing_ok");
    }
    match format.as_ref().and_then(|value| value.as_str()) {
        None if format.is_none() => {}
        Some("text") => {}
        Some("image") => return read_image(files, scope, input).await,
        Some("instruction_scope") => {
            let request = serde_json::from_value(input)
                .map_err(|error| AppError::BadRequest(error.to_string()))?;
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
    let page = files
        .read_text_page_for_agent_session(scope, request)
        .await?;
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

async fn read_image(
    files: &FileService,
    scope: &AgentSessionWorkspaceBinding,
    input: serde_json::Value,
) -> Result<Option<StrictJsonValue>, AppError> {
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
    let extension = std::path::Path::new(request.path.trim())
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
    let Some((bytes, source_sha256)) = files
        .read_bytes_for_agent_session(scope, &request.path, 4 * 1024 * 1024)
        .await?
    else {
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
        } = nomifun_ai_agent::model_attachments::prepare_image_resource(
            &bytes,
            request.path.trim(),
        )
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
            path: request.path.trim().to_owned(),
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
