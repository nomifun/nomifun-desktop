//! Project canonical Computer screenshots into typed model image parts.
//!
//! The native role owner returns a bounded JSON envelope because the Kernel
//! capability port is JSON-only. This exact adapter removes pixel bodies from
//! that JSON before model/history use and restores them as typed image parts.
//! Arbitrary plugin JSON is never interpreted as Computer media.

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use nomifun_agent_contracts::ContributionSourceKind;
use nomifun_agent_kernel::{CompiledSnapshot, SessionCapabilityState};
use nomifun_chat_model_broker::ChatToolResultPart;
use nomifun_engine_core::{
    EngineToolError, EngineToolInvocation, EngineToolInvoker, EngineToolResult,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const MAX_COMPUTER_IMAGE_ENCODED_BYTES: usize = 2 * 1024 * 1024;
const MAX_COMPUTER_IMAGE_DECODED_BYTES: usize = 3 * 1024 * 1024 / 2;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub(super) struct ComputerMediaTools {
    pub inner: Arc<dyn EngineToolInvoker>,
    pub snapshot: Arc<CompiledSnapshot>,
    pub active: Arc<SessionCapabilityState>,
    pub route_image_input: bool,
}

fn error(message: &str) -> EngineToolError {
    EngineToolError::ToolInvocation(message.into())
}

fn is_computer_screenshot(capability_id: &str, action_id: &str, native_action: Option<&str>) -> bool {
    capability_id == nomifun_agent_domain_wave2::COMPUTER_MODULE_ID
        && action_id == "computer/observe"
        && native_action == Some("screenshot")
}

impl ComputerMediaTools {
    fn admit_image(&self, generation: u64) -> Result<(), EngineToolError> {
        let active = self
            .active
            .snapshot()
            .map_err(|_| error("Computer image capability state is unavailable"))?;
        if !self.route_image_input
            || active.generation != generation
            || active.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
        {
            return Err(error(
                "Computer screenshot requires an eligible exact model route with ImageInput; no pixels captured",
            ));
        }
        Ok(())
    }
}

fn project_computer_screenshot(result: EngineToolResult) -> Result<EngineToolResult, EngineToolError> {
    if result.is_error {
        return Ok(result);
    }
    let [ChatToolResultPart::Text { text }] = result.output.as_slice() else {
        return Err(error("Canonical Computer screenshot result has an invalid shape"));
    };
    let mut envelope: Value = serde_json::from_str(text)
        .map_err(|_| error("Canonical Computer screenshot result could not be decoded"))?;
    let generation = envelope
        .get("generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| error("Canonical Computer screenshot generation is missing"))?;
    let result_object = envelope
        .get_mut("result")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| error("Canonical Computer screenshot payload is missing"))?;
    let text = result_object
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| error("Canonical Computer screenshot text is missing"))?
        .to_owned();
    let images = result_object
        .remove("images")
        .and_then(|images| images.as_array().cloned())
        .filter(|images| images.len() == 1)
        .ok_or_else(|| error("Canonical Computer screenshot requires exactly one image"))?;

    let mut output = Vec::with_capacity(2);
    let mut encoded_bytes = 0usize;
    for image in images {
        let media_type = image
            .get("media_type")
            .and_then(Value::as_str)
            .filter(|media_type| *media_type == "image/png")
            .ok_or_else(|| error("Canonical Computer screenshot media type is invalid"))?
            .to_owned();
        let data_base64 = image
            .get("data")
            .and_then(Value::as_str)
            .filter(|data| !data.is_empty() && data.len() <= MAX_COMPUTER_IMAGE_ENCODED_BYTES)
            .ok_or_else(|| error("Canonical Computer screenshot encoded body is invalid"))?
            .to_owned();
        let decoded = STANDARD
            .decode(data_base64.as_bytes())
            .map_err(|_| error("Canonical Computer screenshot is not valid base64"))?;
        if decoded.len() > MAX_COMPUTER_IMAGE_DECODED_BYTES {
            return Err(error("Canonical Computer screenshot decoded body exceeds its limit"));
        }
        if !decoded.starts_with(PNG_SIGNATURE) {
            return Err(error("Canonical Computer screenshot body is not PNG"));
        }
        encoded_bytes = encoded_bytes.saturating_add(data_base64.len());
        output.push(ChatToolResultPart::Image {
            media_type,
            data_base64,
        });
    }
    output.insert(
        0,
        ChatToolResultPart::Text {
            text: json!({
                "kind":"computer_screenshot",
                "generation":generation,
                "text":text,
                "image_count":1,
                "encoded_bytes":encoded_bytes,
                "notice":"Pixels came from the exact authorized local Computer owner. History or compaction may omit them; re-observe before any later pixel action."
            })
            .to_string(),
        },
    );
    let projected = EngineToolResult {
        call_id: result.call_id,
        output,
        is_error: false,
    };
    projected.validate_for(&projected.call_id)?;
    Ok(projected)
}

#[async_trait]
impl EngineToolInvoker for ComputerMediaTools {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        let screenshot = is_computer_screenshot(
            invocation.binding.capability_id.as_ref(),
            invocation.binding.action_id.as_ref(),
            invocation
                .call
                .arguments
                .0
                .get("action")
                .and_then(Value::as_str),
        );
        if !screenshot {
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
        }) {
            return Err(error(
                "Computer media projection requires the canonical platform computer/observe contribution",
            ));
        }
        let generation = invocation.active_set_generation;
        self.admit_image(generation)?;
        let result = self.inner.invoke(invocation, cancellation).await?;
        self.admit_image(generation)?;
        project_computer_screenshot(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_chat_model_broker::ToolCallId;

    #[test]
    fn projection_is_exact_to_the_canonical_screenshot_action() {
        assert!(is_computer_screenshot(
            "computer",
            "computer/observe",
            Some("screenshot")
        ));
        assert!(!is_computer_screenshot(
            "computer",
            "computer/a11y.observe",
            Some("observe")
        ));
        assert!(!is_computer_screenshot(
            "foreign.computer",
            "computer/observe",
            Some("screenshot")
        ));
    }

    #[test]
    fn projection_removes_base64_from_text_and_returns_one_typed_image() {
        let encoded = STANDARD.encode([PNG_SIGNATURE.as_slice(), b"bounded fixture"].concat());
        let result = EngineToolResult::text(
            ToolCallId::from("screen"),
            json!({
                "generation": 3,
                "result": {
                    "text":"Screenshot captured: 10x10.",
                    "images":[{"media_type":"image/png","data":encoded}]
                }
            })
            .to_string(),
            false,
        );
        let projected = project_computer_screenshot(result).unwrap();
        assert_eq!(projected.output.len(), 2);
        let ChatToolResultPart::Text { text } = &projected.output[0] else {
            panic!("first part must be text")
        };
        assert!(!text.contains(&encoded));
        assert!(text.contains("computer_screenshot"));
        assert!(matches!(
            &projected.output[1],
            ChatToolResultPart::Image { media_type, data_base64 }
                if media_type == "image/png" && data_base64 == &encoded
        ));
    }

    #[test]
    fn projection_rejects_a_non_png_body() {
        let result = EngineToolResult::text(
            ToolCallId::from("screen"),
            json!({
                "generation": 3,
                "result": {
                    "text":"Screenshot captured: 10x10.",
                    "images":[{"media_type":"image/png","data":STANDARD.encode(b"not png")}]
                }
            })
            .to_string(),
            false,
        );
        assert!(project_computer_screenshot(result)
            .unwrap_err()
            .to_string()
            .contains("not PNG"));
    }
}
