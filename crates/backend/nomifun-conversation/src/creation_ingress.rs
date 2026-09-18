//! Bounded local-file ingress for canonical Agent creation tasks.

use std::sync::Arc;

use nomifun_common::AppError;
use nomifun_creation::{CreationInput, CreativeCreationTask};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitConversationCreation {
    pub preset_id: String,
    pub provider_id: String,
    pub model: String,
    pub capability: String,
    pub params: Value,
    #[serde(default)]
    pub inputs: Vec<CreationInput>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Serialize)]
pub struct ConversationCreationResponse {
    pub message_id: String,
    pub tasks: Vec<CreativeCreationTask>,
}

#[derive(Serialize)]
pub struct ConversationCreationPage {
    pub items: Vec<CreativeCreationTask>,
}

pub async fn import_creation_files(
    engine: &Arc<nomifun_creation::CreationService>,
    agent_session_id: &str,
    files: &[String],
    capability: &str,
    existing_inputs: &[CreationInput],
    in_library: bool,
) -> Result<Vec<CreationInput>, AppError> {
    use tokio::io::AsyncReadExt;
    if files.len() > 8 {
        return Err(AppError::BadRequest("Attach up to 8 generation references".into()));
    }
    let mut prepared = Vec::new();
    let mut total = 0;
    for path in files {
        let path = nomifun_file::path_safety::validate_path_authority(
            path,
            &nomifun_file::PathAuthority::Unrestricted,
        )?;
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("Reference").to_owned();
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase();
        let mime = match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "mp4" | "m4v" => "video/mp4",
            "webm" => "video/webm",
            "mov" => "video/quicktime",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "ogg" => "audio/ogg",
            "flac" => "audio/flac",
            "m4a" => "audio/mp4",
            _ => return Err(AppError::BadRequest(format!("{name}: choose an image, video or audio file"))),
        };
        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|error| AppError::BadRequest(format!("Cannot read {name}: {error}")))?;
        let mut bytes = Vec::new();
        file.take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        total += bytes.len();
        if bytes.len() > 64 * 1024 * 1024 || total > 256 * 1024 * 1024 {
            return Err(AppError::BadRequest("References exceed the supported upload size".into()));
        }
        let mime = nomifun_creation::validate_artifact_payload(&bytes, mime)
            .map_err(|error| AppError::BadRequest(format!("{name}: {}", error.message)))?;
        prepared.push((bytes, mime, name));
    }
    let mut inputs = Vec::new();
    let mut assign_first_frame = capability == "i2v"
        && prepared.iter().filter(|(_, mime, _)| mime.starts_with("image/")).count() == 1
        && existing_inputs.iter().all(|input| input.role == "last_frame");
    for (bytes, mime, name) in prepared {
        let mut input = engine
            .import_reference(
                bytes,
                mime,
                json!({
                    "source": "agent_session_attachment",
                    "attachment_context": {"agent_session_id": agent_session_id},
                    "title": name,
                }),
                in_library,
            )
            .await?;
        input.role = match input.kind {
            nomifun_creation::CreationInputKind::Image if assign_first_frame => {
                assign_first_frame = false;
                "first_frame"
            }
            nomifun_creation::CreationInputKind::Video => "video",
            nomifun_creation::CreationInputKind::Audio => "audio",
            _ => "reference",
        }
        .into();
        inputs.push(input);
    }
    Ok(inputs)
}
