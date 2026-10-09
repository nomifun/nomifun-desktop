//! Project canonical Computer and Browser screenshots into typed model image parts.
//!
//! The native role owner returns a bounded JSON envelope because the Kernel
//! capability port is JSON-only. This exact adapter removes pixel bodies from
//! that JSON before model/history use and restores them as typed image parts.
//! Arbitrary plugin JSON is never interpreted as native media.

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

fn is_browser_screenshot(capability_id: &str, action_id: &str, arguments: &Value) -> bool {
    capability_id == nomifun_agent_domain_wave2::BROWSER_MODULE_ID
        && action_id == "browser/observe"
        && arguments.get("screenshot").and_then(Value::as_bool) == Some(true)
}

fn project_browser_screenshot(result: EngineToolResult) -> Result<EngineToolResult, EngineToolError> {
    if result.is_error { return Ok(result); }
    let [ChatToolResultPart::Text { text }] = result.output.as_slice() else {
        return Err(error("Canonical Browser screenshot result has an invalid shape"));
    };
    let mut envelope: Value = serde_json::from_str(text)
        .map_err(|_| error("Canonical Browser screenshot result could not be decoded"))?;
    // BrowserRoleOwner returns the observation object itself. KernelToolInvoker
    // serializes output.0 unchanged; Computer's result envelope is unrelated.
    let observation = envelope.as_object_mut()
        .ok_or_else(|| error("Canonical Browser observation payload is missing"))?;
    if observation.get("screenshot_status").and_then(Value::as_str) == Some("awaiting_dialog")
        && observation.get("script_dialog").is_some_and(Value::is_object)
        && !observation.contains_key("screenshot") {
        return Ok(result);
    }
    let target = observation.get("target").cloned()
        .ok_or_else(|| error("Canonical Browser observation target is missing"))?;
    let screenshot = observation.get_mut("screenshot").and_then(Value::as_object_mut)
        .ok_or_else(|| error("Canonical Browser screenshot payload is missing"))?;
    if screenshot.get("target") != Some(&target)
        || !["width", "height"].iter().all(|key| screenshot.get(*key).and_then(Value::as_u64).is_some_and(|value| value > 0 && value <= 32_768)) {
        return Err(error("Canonical Browser screenshot target or dimensions are invalid"));
    }
    let data_base64 = screenshot.remove("png_base64").and_then(|body| body.as_str().map(str::to_owned))
        .filter(|body| !body.is_empty() && body.len() <= MAX_COMPUTER_IMAGE_ENCODED_BYTES)
        .ok_or_else(|| error("Canonical Browser screenshot encoded body is invalid"))?;
    let decoded = STANDARD.decode(data_base64.as_bytes())
        .map_err(|_| error("Canonical Browser screenshot is not valid base64"))?;
    if decoded.len() > MAX_COMPUTER_IMAGE_DECODED_BYTES || !decoded.starts_with(PNG_SIGNATURE) {
        return Err(error("Canonical Browser screenshot body is not bounded PNG"));
    }
    screenshot.insert("media_type".into(), json!("image/png"));
    screenshot.insert("notice".into(), json!("Visible viewport from the same authorized native Browser tab. DOM content and pixels are separate observations; observe again after actions or navigation. History may omit pixels."));
    let projected = EngineToolResult {
        call_id: result.call_id,
        output: vec![ChatToolResultPart::Text { text: envelope.to_string() }, ChatToolResultPart::Image { media_type: "image/png".into(), data_base64 }],
        is_error: false,
    };
    projected.validate_for(&projected.call_id)?;
    Ok(projected)
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
                "Native screenshot requires an eligible exact model route with ImageInput; no pixels delivered",
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
        let browser_screenshot = is_browser_screenshot(
            invocation.binding.capability_id.as_ref(), invocation.binding.action_id.as_ref(), &invocation.call.arguments.0,
        );
        if !screenshot && !browser_screenshot {
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
                "Native media projection requires the canonical platform Computer or Browser observation contribution",
            ));
        }
        let generation = invocation.active_set_generation;
        self.admit_image(generation)?;
        let result = self.inner.invoke(invocation, cancellation).await?;
        self.admit_image(generation)?;
        if browser_screenshot { project_browser_screenshot(result) } else { project_computer_screenshot(result) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_chat_model_broker::ToolCallId;
    use nomifun_browser_platform::runtime::{BrowserDialog, BrowserDialogKind, BrowserInteractionCapabilities, BrowserObservation, BrowserScreenshot, BrowserTabTarget};

    const BROWSER_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a1ZkAAAAASUVORK5CYII=";

    fn browser_observation() -> BrowserObservation {
        BrowserObservation {
            target: BrowserTabTarget { tab_id: "tab".into(), runtime_generation: 1, document_generation: 2 },
            observation_generation: 3,
            content: "Page text".into(), elements: vec![], script_dialog: None, unobserved_frames: 1,
        }
    }

    fn browser_screenshot(target: BrowserTabTarget, png_base64: String) -> BrowserScreenshot {
        BrowserScreenshot { target, width: 1, height: 1, viewport_width: 880., viewport_height: 600., png_base64 }
    }

    fn encoded_browser_result(observation: BrowserObservation, screenshot: Option<BrowserScreenshot>) -> EngineToolResult {
        // Exercise the BrowserRoleOwner encoder and KernelToolInvoker's actual
        // output.0 serialization, rather than a Computer-shaped mock envelope.
        let output = super::super::engine_browser_tools::encode_managed_observation(
            observation, Some(BrowserInteractionCapabilities::wk_webview()), screenshot, true,
        ).unwrap();
        EngineToolResult::text(ToolCallId::from("page"), serde_json::to_string(&output.0).unwrap(), false)
    }

    #[test]
    fn browser_screenshot_is_opt_in_and_exact_to_canonical_observe() {
        assert!(is_browser_screenshot("browser", "browser/observe", &json!({"screenshot":true})));
        assert!(!is_browser_screenshot("browser", "browser/observe", &json!({})));
        assert!(!is_browser_screenshot("foreign.browser", "browser/observe", &json!({"screenshot":true})));
        assert!(!is_browser_screenshot("browser", "browser/render_content", &json!({"screenshot":true})));
    }

    #[test]
    fn browser_projection_preserves_dom_metadata_and_removes_pixels_from_text() {
        let encoded = BROWSER_PNG.to_owned();
        let observation = browser_observation();
        let screenshot = browser_screenshot(observation.target.clone(), encoded.clone());
        let result = encoded_browser_result(observation, Some(screenshot));
        assert!(serde_json::from_str::<Value>(&result.output_text()).unwrap().get("result").is_none());
        let projected = project_browser_screenshot(result).unwrap();
        let ChatToolResultPart::Text { text } = &projected.output[0] else { panic!("text metadata missing") };
        assert!(!text.contains(&encoded));
        assert!(!text.contains("png_base64"));
        assert!(text.contains("semantic_dom"));
        assert!(text.contains("Page text"));
        assert!(matches!(&projected.output[1], ChatToolResultPart::Image { media_type, data_base64 } if media_type == "image/png" && data_base64 == &encoded));
    }

    #[test]
    fn browser_projection_rejects_mismatched_page_and_malformed_image() {
        let observation = browser_observation();
        let mut old_target = observation.target.clone();
        old_target.document_generation -= 1;
        let stale = super::super::engine_browser_tools::encode_managed_observation(
            observation.clone(), None, Some(browser_screenshot(old_target, BROWSER_PNG.into())), true,
        );
        assert!(matches!(stale, Err(super::super::engine_browser_tools::BrowserHostFailure::Workspace(
            nomifun_browser_platform::runtime::WorkspaceError::StaleTarget
        ))));
        let malformed = browser_screenshot(observation.target.clone(), STANDARD.encode(b"not png"));
        assert!(project_browser_screenshot(encoded_browser_result(observation, Some(malformed))).is_err());
    }

    #[test]
    fn pending_dialog_returns_metadata_without_fabricated_pixels() {
        let mut observation = browser_observation();
        observation.script_dialog = Some(BrowserDialog {
            target: observation.target.clone(), kind: BrowserDialogKind::Alert, request_id: "dialog".into(),
            message: "Page alert".into(), default_text: String::new(), origin: "http://localhost".into(), text_truncated: false,
        });
        // A dialog opening after capture discards those no-longer-operable pixels.
        let screenshot = browser_screenshot(observation.target.clone(), BROWSER_PNG.into());
        let result = encoded_browser_result(observation, Some(screenshot));
        let projected = project_browser_screenshot(result).unwrap();
        assert_eq!(projected.output.len(), 1);
        assert!(!projected.output_text().contains(BROWSER_PNG));
    }

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
