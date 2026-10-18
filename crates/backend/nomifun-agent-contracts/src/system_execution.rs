use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use ts_rs::TS;

/// The original native system owner failure. Optional facts remain unknown
/// when that owner did not report them; cleanup does not prove non-dispatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct SystemExecutionFailure {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub raw_os_error: Option<i32>,
    #[serde(default)]
    pub user_code_not_started: Option<bool>,
    #[serde(default)]
    pub cleanup_proven: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_native_facts_remain_unknown_and_prose_never_supplies_them() {
        let failure = SystemExecutionFailure::from_owner("SYSTEM_EXECUTION_NOT_AVAILABLE".into(),
            "raw_os_error=1223; user_code_not_started=true; cleanup_proven=true".into(), &serde_json::json!({}));
        assert_eq!(failure.raw_os_error, None);
        assert_eq!(failure.kind, None);
        assert_eq!(failure.user_code_not_started, None);
        assert_eq!(failure.cleanup_proven, None);
        assert!(failure.validate());
        let roundtrip: SystemExecutionFailure = serde_json::from_value(serde_json::to_value(&failure).unwrap()).unwrap();
        assert_eq!(roundtrip, failure);
    }
}

impl SystemExecutionFailure {
    /// Only typed fields supplied by the native owner are projected. Neither
    /// OS codes nor execution/cleanup facts are reconstructed from prose.
    pub fn from_owner(code: String, message: String, details: &serde_json::Value) -> Self {
        Self {
            code,
            message,
            kind: details.get("kind").and_then(serde_json::Value::as_str).map(str::to_owned),
            raw_os_error: details.get("raw_os_error").and_then(serde_json::Value::as_i64)
                .and_then(|value| i32::try_from(value).ok()),
            user_code_not_started: details.get("user_code_not_started").and_then(serde_json::Value::as_bool),
            cleanup_proven: details.get("cleanup_proven").and_then(serde_json::Value::as_bool),
        }
    }

    pub fn validate(&self) -> bool {
        !self.code.is_empty() && self.code.len() <= 128
            && self.code.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && self.message.len() <= 4096
            && self.kind.as_ref().is_none_or(|kind| !kind.is_empty() && kind.len() <= 64
                && kind.bytes().all(|byte| byte.is_ascii_alphabetic()))
    }
}
