//! Pure native-parameter parsing. No retries, schema validation, repair, tool
//! execution or raw argument material crosses this diagnostic boundary.
use nomifun_agent_contracts::{StrictJsonValue, digest_bytes};
use serde_json::Value;

use crate::{ChatModelEvent, ChatToolCall, ToolArgumentDiagnostic, ToolArgumentDiagnosticCategory, ToolCallId};

#[derive(Clone, Copy)]
pub enum ToolArgumentInput<'a> {
    JsonText(&'a str),
    JsonValue(&'a Value),
}

/// Protocol adapters can supply an exact parser without owning correction.
/// Implementations must preserve JSON values, require an object root, and
/// return only the bounded diagnostic contract on failure.
pub trait ToolArgumentParser: Send + Sync {
    fn parse(&self, input: ToolArgumentInput<'_>) -> Result<StrictJsonValue, ToolArgumentDiagnostic>;
}

#[derive(Default)]
pub struct JsonToolArgumentParser;

impl ToolArgumentParser for JsonToolArgumentParser {
    fn parse(&self, input: ToolArgumentInput<'_>) -> Result<StrictJsonValue, ToolArgumentDiagnostic> {
        let bytes;
        let (value, source) = match input {
            ToolArgumentInput::JsonText(text) => {
                let value = serde_json::from_str::<Value>(text).map_err(|error| ToolArgumentDiagnostic {
                    category: if error.is_eof() { ToolArgumentDiagnosticCategory::IncompleteJson }
                        else { ToolArgumentDiagnosticCategory::Syntax },
                    line: Some(u32::try_from(error.line()).unwrap_or(u32::MAX)),
                    column: Some(u32::try_from(error.column()).unwrap_or(u32::MAX)),
                    byte_len: u32::try_from(text.len()).unwrap_or(u32::MAX),
                    digest: digest_bytes(text.as_bytes()),
                })?;
                (value, text.as_bytes())
            }
            ToolArgumentInput::JsonValue(value) => {
                if value.is_object() {
                    return Ok(StrictJsonValue(value.clone()));
                }
                // A serde_json::Value has no fallible custom serialization.
                bytes = serde_json::to_vec(value).expect("JSON value serializes");
                (value.clone(), bytes.as_slice())
            }
        };
        if !value.is_object() {
            return Err(ToolArgumentDiagnostic {
                category: ToolArgumentDiagnosticCategory::RootType,
                line: None, column: None,
                byte_len: u32::try_from(source.len()).unwrap_or(u32::MAX),
                digest: digest_bytes(source),
            });
        }
        Ok(StrictJsonValue(value))
    }
}

pub(crate) fn proposal_event(
    parser: &dyn ToolArgumentParser,
    call_id: ToolCallId,
    name: String,
    input: ToolArgumentInput<'_>,
    provider_metadata: Option<StrictJsonValue>,
) -> ChatModelEvent {
    match parser.parse(input) {
        Ok(arguments) => ChatModelEvent::ToolCallCompleted {
            call: ChatToolCall { call_id, name, arguments, provider_metadata },
        },
        Err(diagnostic) => ChatModelEvent::ToolCallRejected { call_id, name, diagnostic },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_preserves_objects_and_reports_only_safe_evidence() {
        let parser = JsonToolArgumentParser;
        let object = serde_json::json!({"props":{"text":"quote \" and slash \\"}});
        assert_eq!(parser.parse(ToolArgumentInput::JsonValue(&object)).unwrap().0, object);
        for (text, category) in [("{\"PRIVATE_PATH\":1}}", ToolArgumentDiagnosticCategory::Syntax),
            ("{\"PRIVATE_SECRET\":", ToolArgumentDiagnosticCategory::IncompleteJson),
            ("[\"PRIVATE_CONTENT\"]", ToolArgumentDiagnosticCategory::RootType)] {
            let diagnostic = parser.parse(ToolArgumentInput::JsonText(text)).unwrap_err();
            assert_eq!(diagnostic.category, category);
            assert_eq!(diagnostic.byte_len as usize, text.len());
            assert_eq!(diagnostic.digest, digest_bytes(text.as_bytes()));
            diagnostic.validate().unwrap();
            assert!(!serde_json::to_string(&diagnostic).unwrap().contains("PRIVATE"));
        }
    }
}
