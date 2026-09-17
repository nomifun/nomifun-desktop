//! Shared application adapter for canonical workspace image results. Images
//! become typed parts before EngineToolHost persistence; arbitrary plugin JSON
//! is never interpreted as a request to load pixels or open another path.
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::ContributionSourceKind;
use nomifun_agent_kernel::{CompiledSnapshot, SessionCapabilityState};
use nomifun_chat_model_broker::ChatToolResultPart;
use nomifun_engine_core::{
    EngineToolError, EngineToolInvocation, EngineToolInvoker, EngineToolResult,
};
use tokio_util::sync::CancellationToken;

pub(super) struct WorkspaceMediaTools {
    pub inner: Arc<dyn EngineToolInvoker>,
    pub snapshot: Arc<CompiledSnapshot>,
    pub active: Arc<SessionCapabilityState>,
    pub primary_image_input: bool,
}

fn error(message: &str) -> EngineToolError {
    EngineToolError::ToolInvocation(message.into())
}

impl WorkspaceMediaTools {
    fn admit_image(&self, generation: u64) -> Result<(), EngineToolError> {
        let active = self
            .active
            .snapshot()
            .map_err(|_| error("Image capability state is unavailable"))?;
        if !self.primary_image_input
            || active.generation != generation
            || active.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
            || !active.active.iter().any(|id| id.as_ref() == "llm.vision")
        {
            return Err(error(
                "Workspace image read requires active llm.vision and the exact primary model route's ImageInput feature; no pixels returned",
            ));
        }
        Ok(())
    }
}

fn is_workspace_image_read(
    capability_id: &str,
    action_id: &str,
    format: Option<&str>,
) -> bool {
    capability_id == "workspace.files"
        && action_id == "workspace.files/read"
        && format == Some("image")
}

#[async_trait]
impl EngineToolInvoker for WorkspaceMediaTools {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        let image = is_workspace_image_read(
            invocation.binding.capability_id.as_ref(),
            invocation.binding.action_id.as_ref(),
            invocation
                .call
                .arguments
                .0
                .get("format")
                .and_then(|value| value.as_str()),
        );
        if !image {
            return self.inner.invoke(invocation, cancellation).await;
        }
        if cancellation.is_cancelled() {
            return Err(EngineToolError::Cancelled);
        }
        let selected = self
            .snapshot
            .content()
            .enabled_capabilities
            .iter()

            .find(|item| item.capability.id == invocation.binding.capability_id);
        if !selected.is_some_and(|item| {
            item.contribution_lock.source_kind == ContributionSourceKind::PlatformBuiltin
        })
        {
            return Err(error(
                "Workspace media projection requires the canonical platform workspace.files/read contribution",
            ));
        }
        let generation = invocation.active_set_generation;
        self.admit_image(generation)?;
        let path = invocation
            .call
            .arguments
            .0
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| error("Image read requires a workspace-relative path"))?
            .trim()
            .to_owned();
        let expected = invocation
            .call
            .arguments
            .0
            .get("expected_sha256")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        // The inner Kernel independently verifies Session, principal, selected
        // action/schema/resources and live generation before any file IO.
        let result = self.inner.invoke(invocation, cancellation).await?;
        if result.is_error {
            return Ok(result);
        }
        self.admit_image(generation)?;
        let [ChatToolResultPart::Text { text }] = result.output.as_slice() else {
            return Err(error(
                "Canonical workspace image result has an invalid shape",
            ));
        };
        let image: super::workspace_file_read::WorkspaceImage = serde_json::from_str(text)
            .map_err(|_| error("Canonical workspace image result could not be decoded"))?;
        if image.kind != "workspace_image"
            || image.path != path
            || image.path.len() > 4096
            || image.source_sha256.len() != 64
            || !image
                .source_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || expected
                .as_ref()
                .is_some_and(|expected| expected != &image.source_sha256)
            || image.source_bytes > 4 * 1024 * 1024
            || image.source_bytes == 0
            || !matches!(image.media_type.as_str(), "image/png" | "image/jpeg")
            || image.data_base64.is_empty()
            || image.data_base64.len() > 2 * 1024 * 1024
        {
            return Err(error(
                "Canonical workspace image result differs from its bounded read contract",
            ));
        }
        let notice = serde_json::json!({
            "notice":"Workspace image observed through the authorized file owner. Pixels were resized/re-encoded and metadata removed; this is not an original-resolution image or a filesystem snapshot. Re-read after workspace changes or history/compaction omitted pixels. Source SHA-256 is not the re-encoded pixel hash.",
            "path":image.path, "source_sha256":image.source_sha256, "source_bytes":image.source_bytes,
            "media_type":image.media_type, "encoded_bytes":image.data_base64.len()
        }).to_string();
        let projected = EngineToolResult {
            call_id: result.call_id,
            is_error: false,
            output: vec![
                ChatToolResultPart::Text { text: notice },
                ChatToolResultPart::Image {
                    media_type: image.media_type,
                    data_base64: image.data_base64,
                },
            ],
        };
        projected.validate_for(&projected.call_id)?;
        Ok(projected)
    }
}

#[cfg(test)]
mod tests {
    use super::is_workspace_image_read;

    #[test]
    fn image_projection_requires_the_exact_workspace_files_read_action() {
        assert!(is_workspace_image_read(
            "workspace.files",
            "workspace.files/read",
            Some("image")
        ));
        assert!(!is_workspace_image_read(
            "workspace.files",
            "workspace.files/search",
            Some("image")
        ));
        assert!(!is_workspace_image_read(
            "fs.read",
            "fs.read.invoke",
            Some("image")
        ));
        assert!(!is_workspace_image_read(
            "workspace.files",
            "workspace.files/read",
            None
        ));
    }
}
