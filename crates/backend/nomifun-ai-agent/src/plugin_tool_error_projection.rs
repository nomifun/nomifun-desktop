use nomi_types::tool::ToolResult;
use serde::Serialize;

use crate::plugin_tools::NomiPluginToolError;

const CONTRACT_ERROR_CODE: &str = "NOMI_PLUGIN_TOOL_CONTRACT_ERROR";
const SCHEMA_ERROR_CODE: &str = "NOMI_PLUGIN_TOOL_SCHEMA_ERROR";

#[derive(Serialize)]
struct ModelSafeToolError<'a> {
    code: &'a str,
    message: &'static str,
}

/// Project a trusted runtime error into the only failure shape visible to the
/// model. The typed canonical code is retained, while host/AppError/DB/HTTP
/// diagnostics remain behind the provider boundary.
pub(crate) fn model_safe_tool_error(error: &NomiPluginToolError) -> ToolResult {
    let (code, message) = match error {
        NomiPluginToolError::Kernel(kernel_error) => {
            let code = kernel_error.canonical_code();
            let message = safe_message_for(code.as_ref());
            (code.as_ref().to_owned(), message)
        }
        NomiPluginToolError::Contract(_) => (
            CONTRACT_ERROR_CODE.to_owned(),
            "The capability runtime contract could not be verified.",
        ),
        NomiPluginToolError::Schema { .. } => (
            SCHEMA_ERROR_CODE.to_owned(),
            "The capability input contract could not be verified.",
        ),
    };
    let content = serde_json::to_string(&ModelSafeToolError {
        code: &code,
        message,
    })
    .unwrap_or_else(|_| {
        // Both fields are fixed strings or an owned UTF-8 canonical code, so
        // serialization cannot normally fail. Keep even that impossible path
        // free of the original diagnostic.
        format!(
            "{{\"code\":\"{CONTRACT_ERROR_CODE}\",\"message\":\"The capability runtime failed safely.\"}}"
        )
    });
    ToolResult::error(content)
}

fn safe_message_for(code: &str) -> &'static str {
    match code {
        "INVALID_PAYLOAD" | "WAVE3_INVALID_REQUEST" | "WAVE4_INVALID_REQUEST" => {
            "The capability request is invalid."
        }
        "RESOURCE_OWNER_MISMATCH"
        | "WAVE3_RESOURCE_OWNER_MISMATCH"
        | "WAVE4_RESOURCE_OWNER_MISMATCH" => {
            "The selected resource does not belong to this session owner."
        }
        "PRESET_RESOURCE_NOT_BOUND"
        | "WAVE3_RESOURCE_NOT_BOUND"
        | "WAVE4_RESOURCE_NOT_BOUND"
        | "WAVE3_RESOURCE_BINDING_INVALID"
        | "WAVE4_RESOURCE_BINDING_INVALID" => {
            "A required capability resource is missing or invalid."
        }
        "HUMAN_REVIEW_REQUIRED" => {
            "Human review is required before this capability can continue."
        }
        "ROBOT_EFFECT_OUTCOME_UNKNOWN" => {
            "The physical effect outcome is unknown; do not retry automatically."
        }
        _ if code.contains("NOT_FOUND") => {
            "The requested capability resource was not found."
        }
        _ if code.contains("TIMEOUT") => "The capability operation timed out.",
        _ if code.contains("UNAVAILABLE") || code.contains("OFFLINE") => {
            "The capability is currently unavailable."
        }
        _ if code.contains("CONFLICT") => {
            "The capability state changed; refresh it before trying again."
        }
        _ => "The capability could not complete the request.",
    }
}

#[cfg(test)]
mod tests {
    use nomifun_agent_kernel::KernelError;

    use super::*;

    #[test]
    fn typed_host_code_survives_while_internal_diagnostics_are_not_model_visible() {
        let error = NomiPluginToolError::Kernel(
            KernelError::capability_execution_failed(
                "DB_WRITE_FAILED",
                "sqlite C:/private/data.db failed; POST https://internal.invalid; api_key=sk-secret",
            ),
        );

        let result = model_safe_tool_error(&error);
        assert!(result.is_error);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["code"], "DB_WRITE_FAILED");
        assert_eq!(
            payload["message"],
            "The capability could not complete the request."
        );
        for forbidden in ["sqlite", "C:/private", "https://", "sk-secret", "api_key"] {
            assert!(
                !result.content.contains(forbidden),
                "provider-visible failure leaked {forbidden}: {}",
                result.content
            );
        }
    }

    #[test]
    fn typed_invalid_payload_has_a_stable_safe_message() {
        let error = NomiPluginToolError::Kernel(
            KernelError::capability_execution_failed(
                "INVALID_PAYLOAD",
                "secret field value was sk-secret",
            ),
        );

        let result = model_safe_tool_error(&error);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["code"], "INVALID_PAYLOAD");
        assert_eq!(payload["message"], "The capability request is invalid.");
        assert!(!result.content.contains("sk-secret"));
    }

    #[test]
    fn legacy_unstructured_kernel_diagnostic_is_never_forwarded() {
        let error = NomiPluginToolError::Kernel(KernelError::CapabilityExecution {
            reason: "raw HTTP response contains bearer-private".to_owned(),
        });

        let result = model_safe_tool_error(&error);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["code"], "CAPABILITY_NOT_MATERIALIZED");
        assert!(!result.content.contains("bearer-private"));
        assert!(!result.content.contains("raw HTTP"));
    }
}
