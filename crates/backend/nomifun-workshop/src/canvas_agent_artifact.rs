//! Strict parser for persisted Creative Canvas Agent proposal artifacts.
//!
//! This is deliberately narrower than [`CreativeAgentOp`]: model-authored
//! artifacts may add and edit text nodes, move/resize nodes, and manage
//! connections, but may not delete nodes, add media/config nodes, or mutate
//! server-owned fields.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::creative_agent_ops::CreativeAgentOp;
use crate::creative_studio::CreativeNodeType;

pub const CREATIVE_CANVAS_AGENT_ARTIFACT_KIND: &str =
    "nomifun.creative-studio.canvas-ops/v1";

const JSON_FENCE_OPENING: &str = "```json\n";
const JSON_FENCE_CLOSING: &str = "\n```";
const MAX_JSON_NESTING_DEPTH: usize = 64;
const MAX_ARTIFACT_JSON_BYTES: usize = 262_144;
const MAX_SUMMARY_CHARS: usize = 500;
const MAX_TEXT_CHARS: usize = 20_000;
const MAX_HANDLE_CHARS: usize = 128;
const MAX_OPS: usize = 64;

/// A validated, text-only Canvas Agent proposal recovered from persisted
/// assistant message content.
#[derive(Debug, Clone)]
pub struct CreativeCanvasAgentArtifact {
    pub kind: String,
    pub summary: String,
    pub ops: Vec<CreativeAgentOp>,
}

/// Parse the unique final lowercase-`json` artifact fence in persisted
/// assistant text.
///
/// Ordinary prose and well-formed artifacts owned by another product return
/// `Ok(None)`. Once the text identifies the Canvas artifact kind, malformed
/// fencing, JSON, duplicate decoded keys, or a contract violation fails
/// closed with a path-oriented error.
pub fn parse_creative_canvas_agent_artifact(
    text: &str,
) -> Result<Option<CreativeCanvasAgentArtifact>, String> {
    let candidate = text.trim_end_matches(is_ecmascript_whitespace);
    let opening_index = candidate.rfind(JSON_FENCE_OPENING);
    let has_canonical_opening = opening_index.is_some_and(|index| {
        index == 0 || candidate[..index].ends_with('\n')
    });

    if !has_canonical_opening || !candidate.ends_with(JSON_FENCE_CLOSING) {
        if text.contains("```") && text.contains(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND) {
            return Err(invalid(
                "$",
                "one final canonical canvas-ops JSON fence",
            ));
        }
        return Ok(None);
    }

    let opening_index = opening_index.expect("canonical opening was checked above");
    let json_start = opening_index + JSON_FENCE_OPENING.len();
    let json_end = candidate.len() - JSON_FENCE_CLOSING.len();
    if json_start > json_end {
        // The shortest empty fence makes the opener and closer overlap.
        // JavaScript's slice produces an empty candidate here; avoid a Rust
        // range panic and preserve the same non-target result.
        return Ok(None);
    }
    if json_end - json_start > MAX_ARTIFACT_JSON_BYTES {
        let oversized = &candidate[json_start..json_end];
        if oversized.contains(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND) {
            return Err(invalid(
                "$",
                "canvas-ops JSON no larger than 262144 bytes",
            ));
        }
        return Ok(None);
    }
    let json_text = &candidate[json_start..json_end];

    // Decode first to preserve the UI contract's target/non-target boundary.
    // The lexical pass below is still authoritative for duplicate keys and
    // depth because serde_json, like JSON.parse, otherwise accepts duplicates.
    let decoded: Value = match serde_json::from_str(json_text) {
        Ok(value) => value,
        Err(error) => {
            if json_text.contains(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND) {
                return Err(invalid(
                    "$",
                    &format!("well-formed canvas-ops JSON ({error})"),
                ));
            }
            return Ok(None);
        }
    };

    let decoded_kind = decoded
        .as_object()
        .and_then(|record| record.get("kind"))
        .and_then(Value::as_str);
    if let Err(error) = StrictJsonScanner::new(json_text).scan() {
        if decoded_kind == Some(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND)
            || json_text.contains(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND)
        {
            return Err(invalid(
                "$",
                &format!("strict JSON without duplicate keys ({error})"),
            ));
        }
        return Ok(None);
    }

    if decoded_kind != Some(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND) {
        return Ok(None);
    }

    if candidate.match_indices("```").count() != 2 {
        return Err(invalid(
            "$",
            "one final canonical canvas-ops JSON fence",
        ));
    }

    parse_artifact_value(&decoded).map(Some)
}

fn parse_artifact_value(value: &Value) -> Result<CreativeCanvasAgentArtifact, String> {
    let record = as_object(value, "$", "object")?;
    exact_keys(record, &["kind", "summary", "ops"], &[], "$")?;

    let kind = as_string(
        required(record, "kind", "$")?,
        "$.kind",
        false,
        CREATIVE_CANVAS_AGENT_ARTIFACT_KIND.chars().count(),
        false,
    )?;
    if kind != CREATIVE_CANVAS_AGENT_ARTIFACT_KIND {
        return Err(invalid(
            "$.kind",
            &format!("{CREATIVE_CANVAS_AGENT_ARTIFACT_KIND:?}"),
        ));
    }

    let summary = as_string(
        required(record, "summary", "$")?,
        "$.summary",
        false,
        MAX_SUMMARY_CHARS,
        true,
    )?
    .to_owned();
    let ops = parse_ops(required(record, "ops", "$")?, "$.ops")?;

    Ok(CreativeCanvasAgentArtifact {
        kind: CREATIVE_CANVAS_AGENT_ARTIFACT_KIND.to_owned(),
        summary,
        ops,
    })
}

fn parse_ops(value: &Value, path: &str) -> Result<Vec<CreativeAgentOp>, String> {
    let values = value
        .as_array()
        .ok_or_else(|| invalid(path, "array with 1 to 64 operations"))?;
    if values.is_empty() || values.len() > MAX_OPS {
        return Err(invalid(path, "array with 1 to 64 operations"));
    }

    values
        .iter()
        .enumerate()
        .map(|(index, value)| parse_op(value, &format!("{path}[{index}]")))
        .collect()
}

fn parse_op(value: &Value, path: &str) -> Result<CreativeAgentOp, String> {
    let record = as_object(value, path, "object")?;
    let op_type = as_string(
        record
            .get("type")
            .ok_or_else(|| invalid(&format!("{path}.type"), "present string"))?,
        &format!("{path}.type"),
        false,
        64,
        false,
    )?;

    match op_type {
        "add_node" => parse_add_node(record, path),
        "update_node_data" => parse_update_node_data(record, path),
        "move_node" => parse_move_node(record, path),
        "resize_node" => parse_resize_node(record, path),
        "connect" => parse_connect(record, path),
        "disconnect" => parse_disconnect(record, path),
        _ => Err(invalid(
            &format!("{path}.type"),
            "add_node | update_node_data | move_node | resize_node | connect | disconnect",
        )),
    }
}

fn parse_add_node(record: &Map<String, Value>, path: &str) -> Result<CreativeAgentOp, String> {
    exact_keys(
        record,
        &["type", "node_type", "x", "y", "data"],
        &["width", "height", "group_id"],
        path,
    )?;

    let node_type_path = format!("{path}.node_type");
    let node_type = as_string(
        required(record, "node_type", path)?,
        &node_type_path,
        false,
        16,
        false,
    )?;
    if node_type != "text" {
        return Err(invalid(&node_type_path, "\"text\""));
    }

    Ok(CreativeAgentOp::AddNode {
        node_type: CreativeNodeType::Text,
        x: as_finite_number(required(record, "x", path)?, &format!("{path}.x"), None, None)?,
        y: as_finite_number(required(record, "y", path)?, &format!("{path}.y"), None, None)?,
        width: optional_number(record, "width", path, Some(1.0), None)?,
        height: optional_number(record, "height", path, Some(1.0), None)?,
        group_id: optional_uuid(record, "group_id", path)?,
        data: parse_text_fields(required(record, "data", path)?, &format!("{path}.data"), true)?,
    })
}

fn parse_update_node_data(
    record: &Map<String, Value>,
    path: &str,
) -> Result<CreativeAgentOp, String> {
    exact_keys(record, &["type", "node_id", "patch"], &[], path)?;
    Ok(CreativeAgentOp::UpdateNodeData {
        node_id: as_uuid(required(record, "node_id", path)?, &format!("{path}.node_id"))?,
        patch: parse_text_fields(
            required(record, "patch", path)?,
            &format!("{path}.patch"),
            false,
        )?,
    })
}

fn parse_move_node(record: &Map<String, Value>, path: &str) -> Result<CreativeAgentOp, String> {
    exact_keys(record, &["type", "node_id", "x", "y"], &[], path)?;
    Ok(CreativeAgentOp::MoveNode {
        node_id: as_uuid(required(record, "node_id", path)?, &format!("{path}.node_id"))?,
        x: as_finite_number(required(record, "x", path)?, &format!("{path}.x"), None, None)?,
        y: as_finite_number(required(record, "y", path)?, &format!("{path}.y"), None, None)?,
    })
}

fn parse_resize_node(
    record: &Map<String, Value>,
    path: &str,
) -> Result<CreativeAgentOp, String> {
    exact_keys(
        record,
        &["type", "node_id", "width", "height"],
        &[],
        path,
    )?;
    Ok(CreativeAgentOp::ResizeNode {
        node_id: as_uuid(required(record, "node_id", path)?, &format!("{path}.node_id"))?,
        width: as_finite_number(
            required(record, "width", path)?,
            &format!("{path}.width"),
            Some(1.0),
            None,
        )?,
        height: as_finite_number(
            required(record, "height", path)?,
            &format!("{path}.height"),
            Some(1.0),
            None,
        )?,
    })
}

fn parse_connect(record: &Map<String, Value>, path: &str) -> Result<CreativeAgentOp, String> {
    exact_keys(
        record,
        &["type", "source_node_id", "target_node_id"],
        &["source_handle", "target_handle"],
        path,
    )?;
    Ok(CreativeAgentOp::Connect {
        source_node_id: as_uuid(
            required(record, "source_node_id", path)?,
            &format!("{path}.source_node_id"),
        )?,
        target_node_id: as_uuid(
            required(record, "target_node_id", path)?,
            &format!("{path}.target_node_id"),
        )?,
        source_handle: optional_handle(record, "source_handle", path)?,
        target_handle: optional_handle(record, "target_handle", path)?,
    })
}

fn parse_disconnect(record: &Map<String, Value>, path: &str) -> Result<CreativeAgentOp, String> {
    exact_keys(record, &["type", "connection_id"], &[], path)?;
    Ok(CreativeAgentOp::Disconnect {
        connection_id: as_uuid(
            required(record, "connection_id", path)?,
            &format!("{path}.connection_id"),
        )?,
    })
}

fn parse_text_fields(value: &Value, path: &str, complete: bool) -> Result<Value, String> {
    const FIELDS: [&str; 4] = ["text", "format", "fontSize", "textAlign"];
    let record = as_object(value, path, "text data object")?;
    if complete {
        exact_keys(record, &FIELDS, &[], path)?;
    } else {
        exact_keys(record, &[], &FIELDS, path)?;
        if record.is_empty() {
            return Err(invalid(path, "at least one text data field"));
        }
    }

    let mut output = Map::new();
    if let Some(value) = record.get("text") {
        let text = as_string(
            value,
            &format!("{path}.text"),
            true,
            MAX_TEXT_CHARS,
            false,
        )?;
        output.insert("text".to_owned(), Value::String(text.to_owned()));
    }
    if let Some(value) = record.get("format") {
        let format = as_literal(
            value,
            &["plain", "markdown"],
            &format!("{path}.format"),
        )?;
        output.insert("format".to_owned(), Value::String(format.to_owned()));
    }
    if let Some(value) = record.get("fontSize") {
        let font_size = as_finite_number(
            value,
            &format!("{path}.fontSize"),
            Some(8.0),
            Some(256.0),
        )?;
        // JSON.parse has only one Number type and JSON.stringify emits an
        // integral font size without a decimal/exponent. Normalize bounded
        // integral values likewise so provenance comparison with the UI's
        // submitted operation cannot differ only by `16.0` versus `16`.
        let font_size = if font_size.fract() == 0.0 {
            Value::Number(serde_json::Number::from(font_size as u64))
        } else {
            Value::Number(
                serde_json::Number::from_f64(font_size)
                    .expect("validated finite font size must be a JSON number"),
            )
        };
        output.insert("fontSize".to_owned(), font_size);
    }
    if let Some(value) = record.get("textAlign") {
        let text_align = as_literal(
            value,
            &["left", "center", "right"],
            &format!("{path}.textAlign"),
        )?;
        output.insert(
            "textAlign".to_owned(),
            Value::String(text_align.to_owned()),
        );
    }

    Ok(Value::Object(output))
}

fn required<'a>(
    record: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a Value, String> {
    record
        .get(key)
        .ok_or_else(|| invalid(&format!("{path}.{key}"), "present"))
}

fn exact_keys(
    record: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
    path: &str,
) -> Result<(), String> {
    for key in required {
        if !record.contains_key(*key) {
            return Err(invalid(&format!("{path}.{key}"), "present"));
        }
    }
    for key in record.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(invalid(
                &format!("{path}.{key}"),
                "no unknown fields",
            ));
        }
    }
    Ok(())
}

fn as_object<'a>(
    value: &'a Value,
    path: &str,
    expected: &str,
) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| invalid(path, expected))
}

fn as_string<'a>(
    value: &'a Value,
    path: &str,
    allow_empty: bool,
    max_chars: usize,
    trimmed: bool,
) -> Result<&'a str, String> {
    let value = value
        .as_str()
        .ok_or_else(|| invalid(path, "string"))?;
    let chars = value.chars().count();
    if (!allow_empty && chars == 0) || chars > max_chars {
        let nonempty = if allow_empty { "" } else { "non-empty " };
        return Err(invalid(
            path,
            &format!("{nonempty}string <= {max_chars} Unicode chars"),
        ));
    }
    if trimmed && value.trim_matches(is_ecmascript_whitespace) != value {
        return Err(invalid(
            path,
            &format!("trimmed string <= {max_chars} Unicode chars"),
        ));
    }
    Ok(value)
}

fn as_literal<'a>(value: &'a Value, allowed: &[&str], path: &str) -> Result<&'a str, String> {
    let value = value
        .as_str()
        .ok_or_else(|| invalid(path, &allowed.join(" | ")))?;
    if !allowed.contains(&value) {
        return Err(invalid(path, &allowed.join(" | ")));
    }
    Ok(value)
}

fn as_finite_number(
    value: &Value,
    path: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<f64, String> {
    let value = value
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid(path, "finite number"))?;
    // JSON.stringify canonicalizes JavaScript -0 as 0 before the UI submits
    // the operation. Normalize signed zero so persisted provenance and the
    // request serialize identically on the server.
    let value = if value == 0.0 { 0.0 } else { value };
    if min.is_some_and(|min| value < min) {
        return Err(invalid(path, &format!("number >= {}", min.unwrap())));
    }
    if max.is_some_and(|max| value > max) {
        return Err(invalid(path, &format!("number <= {}", max.unwrap())));
    }
    Ok(value)
}

fn optional_number(
    record: &Map<String, Value>,
    key: &str,
    path: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<Option<f64>, String> {
    record
        .get(key)
        .map(|value| as_finite_number(value, &format!("{path}.{key}"), min, max))
        .transpose()
}

fn as_uuid(value: &Value, path: &str) -> Result<String, String> {
    let value = value
        .as_str()
        .ok_or_else(|| invalid(path, "canonical lowercase UUIDv7"))?;
    nomifun_common::validate_uuidv7(value)
        .map_err(|_| invalid(path, "canonical lowercase UUIDv7"))?;
    Ok(value.to_owned())
}

fn optional_uuid(
    record: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<String>, String> {
    match record.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => as_uuid(value, &format!("{path}.{key}")).map(Some),
    }
}

fn optional_handle(
    record: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<String>, String> {
    match record.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => as_string(
            value,
            &format!("{path}.{key}"),
            false,
            MAX_HANDLE_CHARS,
            true,
        )
        .map(ToOwned::to_owned)
        .map(Some),
    }
}

fn invalid(path: &str, expected: &str) -> String {
    format!("invalid Creative Canvas Agent artifact at {path}: expected {expected}")
}

// ECMAScript TrimString is close to, but not identical to, Rust's Unicode
// `char::is_whitespace`: JavaScript includes U+FEFF and excludes U+0085. Keep
// this explicit so the persisted server parser and UI parser agree at edges.
fn is_ecmascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'
            | '\u{000a}'
            | '\u{000b}'
            | '\u{000c}'
            | '\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// Lexically validates JSON while retaining decoded object keys so escaped
/// spellings such as `summary` and `\\u0073ummary` cannot coexist.
struct StrictJsonScanner<'a> {
    source: &'a str,
    bytes: &'a [u8],
    index: usize,
}

impl<'a> StrictJsonScanner<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            index: 0,
        }
    }

    fn scan(mut self) -> Result<(), String> {
        self.skip_whitespace();
        self.scan_value(0)?;
        self.skip_whitespace();
        if self.index != self.bytes.len() {
            return Err(self.error("unexpected trailing JSON"));
        }
        Ok(())
    }

    fn scan_value(&mut self, depth: usize) -> Result<(), String> {
        match self.bytes.get(self.index).copied() {
            Some(b'{') => {
                self.ensure_container_depth(depth)?;
                self.scan_object(depth + 1)
            }
            Some(b'[') => {
                self.ensure_container_depth(depth)?;
                self.scan_array(depth + 1)
            }
            Some(b'"') => self.scan_string().map(|_| ()),
            Some(b't') => self.scan_literal(b"true"),
            Some(b'f') => self.scan_literal(b"false"),
            Some(b'n') => self.scan_literal(b"null"),
            Some(_) => self.scan_number(),
            None => Err(self.error("expected JSON value")),
        }
    }

    fn ensure_container_depth(&self, depth: usize) -> Result<(), String> {
        if depth >= MAX_JSON_NESTING_DEPTH {
            Err(self.error("JSON nesting exceeds 64 levels"))
        } else {
            Ok(())
        }
    }

    fn scan_object(&mut self, depth: usize) -> Result<(), String> {
        self.index += 1;
        self.skip_whitespace();
        if self.consume(b'}') {
            return Ok(());
        }

        let mut keys = HashSet::new();
        loop {
            if self.bytes.get(self.index) != Some(&b'"') {
                return Err(self.error("object key must be a string"));
            }
            let raw_key = self.scan_string()?;
            let key: String = serde_json::from_str(raw_key)
                .map_err(|_| self.error("invalid JSON object key string"))?;
            if !keys.insert(key) {
                return Err(self.error("duplicate decoded JSON object key"));
            }
            self.skip_whitespace();
            if !self.consume(b':') {
                return Err(self.error("object key must be followed by colon"));
            }
            self.skip_whitespace();
            self.scan_value(depth)?;
            self.skip_whitespace();
            if self.consume(b'}') {
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(self.error("object entries must be comma separated"));
            }
            self.skip_whitespace();
        }
    }

    fn scan_array(&mut self, depth: usize) -> Result<(), String> {
        self.index += 1;
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(());
        }

        loop {
            self.scan_value(depth)?;
            self.skip_whitespace();
            if self.consume(b']') {
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(self.error("array entries must be comma separated"));
            }
            self.skip_whitespace();
        }
    }

    fn scan_string(&mut self) -> Result<&'a str, String> {
        let start = self.index;
        if !self.consume(b'"') {
            return Err(self.error("expected JSON string"));
        }

        while let Some(current) = self.bytes.get(self.index).copied() {
            match current {
                b'"' => {
                    self.index += 1;
                    return Ok(&self.source[start..self.index]);
                }
                0x00..=0x1f => {
                    return Err(self.error("unescaped control character in JSON string"));
                }
                b'\\' => {
                    self.index += 1;
                    match self.bytes.get(self.index).copied() {
                        Some(b'u') => {
                            let hex_start = self.index + 1;
                            let hex_end = hex_start + 4;
                            if hex_end > self.bytes.len()
                                || !self.bytes[hex_start..hex_end]
                                    .iter()
                                    .all(u8::is_ascii_hexdigit)
                            {
                                return Err(self.error("invalid unicode escape"));
                            }
                            self.index = hex_end;
                        }
                        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                            self.index += 1;
                        }
                        _ => return Err(self.error("invalid JSON string escape")),
                    }
                }
                _ => self.index += 1,
            }
        }
        Err(self.error("unterminated JSON string"))
    }

    fn scan_literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.bytes[self.index..].starts_with(literal) {
            self.index += literal.len();
            Ok(())
        } else {
            Err(self.error("invalid JSON literal"))
        }
    }

    fn scan_number(&mut self) -> Result<(), String> {
        let start = self.index;
        self.consume(b'-');

        match self.bytes.get(self.index).copied() {
            Some(b'0') => self.index += 1,
            Some(b'1'..=b'9') => {
                self.index += 1;
                self.consume_digits();
            }
            _ => return Err(self.error("invalid JSON value")),
        }

        if self.consume(b'.') {
            let fraction_start = self.index;
            self.consume_digits();
            if self.index == fraction_start {
                return Err(self.error("JSON fraction requires digits"));
            }
        }

        if matches!(self.bytes.get(self.index), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.bytes.get(self.index), Some(b'+' | b'-')) {
                self.index += 1;
            }
            let exponent_start = self.index;
            self.consume_digits();
            if self.index == exponent_start {
                return Err(self.error("JSON exponent requires digits"));
            }
        }

        if self.index == start {
            Err(self.error("invalid JSON number"))
        } else {
            Ok(())
        }
    }

    fn consume_digits(&mut self) {
        while matches!(self.bytes.get(self.index), Some(b'0'..=b'9')) {
            self.index += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(
            self.bytes.get(self.index),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.index += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.index) == Some(&expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.index)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NODE_A: &str = "0190f5fe-7c00-7a00-8000-000000000801";
    const NODE_B: &str = "0190f5fe-7c00-7a00-8000-000000000802";
    const CONNECTION: &str = "0190f5fe-7c00-7a00-9000-000000000803";

    fn add_text_op() -> Value {
        json!({
            "type": "add_node",
            "node_type": "text",
            "x": -12.5,
            "y": 24,
            "width": 320,
            "height": 180,
            "group_id": null,
            "data": {
                "text": "# 标题",
                "format": "markdown",
                "fontSize": 32,
                "textAlign": "center"
            }
        })
    }

    fn all_allowed_ops() -> Value {
        json!([
            add_text_op(),
            {
                "type": "update_node_data",
                "node_id": NODE_A,
                "patch": { "text": "更新文案", "fontSize": 20 }
            },
            { "type": "move_node", "node_id": NODE_A, "x": 100, "y": -50 },
            { "type": "resize_node", "node_id": NODE_A, "width": 400, "height": 220 },
            {
                "type": "connect",
                "source_node_id": NODE_A,
                "target_node_id": NODE_B,
                "source_handle": "output",
                "target_handle": null
            },
            { "type": "disconnect", "connection_id": CONNECTION }
        ])
    }

    fn artifact(ops: Value) -> Value {
        json!({
            "kind": CREATIVE_CANVAS_AGENT_ARTIFACT_KIND,
            "summary": "新增并整理文案节点",
            "ops": ops
        })
    }

    fn fence(value: &Value) -> String {
        format!("```json\n{}\n```", serde_json::to_string(value).unwrap())
    }

    fn assert_rejected(text: &str) -> String {
        parse_creative_canvas_agent_artifact(text).unwrap_err()
    }

    fn assert_artifact_rejected(value: Value) -> String {
        assert_rejected(&fence(&value))
    }

    #[test]
    fn accepts_prefix_trailing_whitespace_and_every_allowed_operation() {
        let text = format!(
            "我准备了以下安全变更：\n{}\n \t\r\n\u{feff}",
            fence(&artifact(all_allowed_ops()))
        );
        let parsed = parse_creative_canvas_agent_artifact(&text)
            .unwrap()
            .expect("target artifact");

        assert_eq!(parsed.kind, CREATIVE_CANVAS_AGENT_ARTIFACT_KIND);
        assert_eq!(parsed.summary, "新增并整理文案节点");
        assert_eq!(parsed.ops.len(), 6);
        assert!(matches!(
            &parsed.ops[0],
            CreativeAgentOp::AddNode {
                node_type: CreativeNodeType::Text,
                x,
                y,
                width: Some(width),
                height: Some(height),
                group_id: None,
                data,
            } if *x == -12.5
                && *y == 24.0
                && *width == 320.0
                && *height == 180.0
                && data["text"] == "# 标题"
        ));
        assert!(matches!(
            &parsed.ops[1],
            CreativeAgentOp::UpdateNodeData { node_id, patch }
                if node_id == NODE_A && patch["fontSize"] == 20
        ));
        assert!(matches!(&parsed.ops[2], CreativeAgentOp::MoveNode { .. }));
        assert!(matches!(&parsed.ops[3], CreativeAgentOp::ResizeNode { .. }));
        assert!(matches!(&parsed.ops[4], CreativeAgentOp::Connect { .. }));
        assert!(matches!(&parsed.ops[5], CreativeAgentOp::Disconnect { .. }));
    }

    #[test]
    fn canonicalizes_integral_text_numbers_like_json_stringify() {
        let text = fence(&artifact(json!([add_text_op()])))
            .replace("\"fontSize\":32", "\"fontSize\":3.2e1");
        let parsed = parse_creative_canvas_agent_artifact(&text)
            .unwrap()
            .expect("target artifact");
        let CreativeAgentOp::AddNode { data, .. } = &parsed.ops[0] else {
            panic!("expected add_node");
        };
        assert_eq!(data["fontSize"].as_u64(), Some(32));
        assert_eq!(
            serde_json::to_value(data).unwrap(),
            json!({
                "fontSize": 32,
                "format": "markdown",
                "text": "# 标题",
                "textAlign": "center"
            })
        );

        let signed_zero = fence(&artifact(json!([{
            "type": "move_node",
            "node_id": NODE_A,
            "x": -0.0,
            "y": -0.0
        }])));
        let parsed = parse_creative_canvas_agent_artifact(&signed_zero)
            .unwrap()
            .expect("target artifact");
        let CreativeAgentOp::MoveNode { x, y, .. } = &parsed.ops[0] else {
            panic!("expected move_node");
        };
        assert_eq!(x.to_bits(), 0.0_f64.to_bits());
        assert_eq!(y.to_bits(), 0.0_f64.to_bits());
        let request_canonical = CreativeAgentOp::MoveNode {
            node_id: NODE_A.to_owned(),
            x: 0.0,
            y: 0.0,
        };
        assert_eq!(
            serde_json::to_string(&parsed.ops[0]).unwrap(),
            serde_json::to_string(&request_canonical).unwrap()
        );
    }

    #[test]
    fn ordinary_prose_and_other_product_artifacts_are_absent() {
        assert!(
            parse_creative_canvas_agent_artifact("这里是普通建议，没有结构化变更。")
                .unwrap()
                .is_none()
        );
        let other = json!({
            "kind": "nomifun.other-product/v1",
            "summary": "other",
            "ops": []
        });
        assert!(
            parse_creative_canvas_agent_artifact(&fence(&other))
                .unwrap()
                .is_none()
        );
        assert!(
            parse_creative_canvas_agent_artifact("```json\n```")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn trim_semantics_match_ecmascript_at_bom_and_next_line_edges() {
        let mut bom_summary = artifact(all_allowed_ops());
        bom_summary["summary"] = Value::String("summary\u{feff}".to_owned());
        assert_artifact_rejected(bom_summary);

        let mut next_line_summary = artifact(all_allowed_ops());
        next_line_summary["summary"] = Value::String("\u{0085}summary\u{0085}".to_owned());
        let parsed = parse_creative_canvas_agent_artifact(&fence(&next_line_summary))
            .unwrap()
            .expect("U+0085 is not ECMAScript trim whitespace");
        assert_eq!(parsed.summary, "\u{0085}summary\u{0085}");
    }

    #[test]
    fn rejects_malformed_target_fencing_and_extra_fences() {
        let malformed = format!(
            "```json\n{{\"kind\":\"{CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}\",\"summary\":\n```"
        );
        assert_rejected(&malformed);
        assert_rejected(&format!(
            "```text\nplan\n```\n{}",
            fence(&artifact(all_allowed_ops()))
        ));
        assert_rejected(&format!(
            "{}\ntrailing assistant prose",
            fence(&artifact(all_allowed_ops()))
        ));
        assert_rejected(&fence(&artifact(json!([{
            "type": "move_node",
            "node_id": NODE_A,
            "x": "```",
            "y": 0
        }]))));
    }

    #[test]
    fn rejects_duplicate_decoded_keys_at_any_object_depth() {
        let add = serde_json::to_string(&add_text_op()).unwrap();
        let duplicate_top = format!(
            "```json\n{{\"kind\":\"{CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}\",\"summary\":\"first\",\"\\u0073ummary\":\"second\",\"ops\":[{add}]}}\n```"
        );
        let error = assert_rejected(&duplicate_top);
        assert!(error.contains("duplicate"), "{error}");

        let duplicate_nested = format!(
            "```json\n{{\"kind\":\"{CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}\",\"summary\":\"nested\",\"ops\":[{{\"type\":\"move_node\",\"node_id\":\"{NODE_A}\",\"x\":1,\"x\":2,\"y\":3}}]}}\n```"
        );
        let error = assert_rejected(&duplicate_nested);
        assert!(error.contains("duplicate"), "{error}");

        let long_key = "k".repeat(10_000);
        let bounded_diagnostic = format!(
            "```json\n{{\"kind\":\"{CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}\",\"summary\":\"bounded\",\"ops\":[{}],\"{long_key}\":1,\"{long_key}\":2}}\n```",
            serde_json::to_string(&add_text_op()).unwrap()
        );
        let error = assert_rejected(&bounded_diagnostic);
        assert!(error.contains("duplicate"), "{error}");
        assert!(error.len() < 512, "diagnostic reflected an unbounded key");
        assert!(!error.contains(&"k".repeat(81)));
    }

    #[test]
    fn rejects_oversized_target_before_materializing_and_ignores_foreign_content() {
        let foreign = format!(
            "```json\n{}\n```",
            "x".repeat(MAX_ARTIFACT_JSON_BYTES + 1)
        );
        assert!(
            parse_creative_canvas_agent_artifact(&foreign)
                .unwrap()
                .is_none()
        );

        let target = format!(
            "```json\n{{\"kind\":\"{CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}\",\"padding\":\"{}\"}}\n```",
            "x".repeat(MAX_ARTIFACT_JSON_BYTES)
        );
        let error = assert_rejected(&target);
        assert!(error.contains("262144 bytes"), "{error}");
    }

    #[test]
    fn scanner_enforces_sixty_four_container_levels() {
        let allowed = format!("{}{}", "[".repeat(64), "]".repeat(64));
        StrictJsonScanner::new(&allowed).scan().unwrap();

        let rejected = format!("{}{}", "[".repeat(65), "]".repeat(65));
        let error = StrictJsonScanner::new(&rejected).scan().unwrap_err();
        assert!(error.contains("64"), "{error}");
    }

    #[test]
    fn enforces_exact_top_operation_data_and_patch_keys() {
        let mut top = artifact(all_allowed_ops());
        top.as_object_mut().unwrap().insert("version".into(), json!(1));
        assert_artifact_rejected(top);

        let mut op = add_text_op();
        op.as_object_mut().unwrap().insert("locked".into(), json!(false));
        assert_artifact_rejected(artifact(json!([op])));

        let mut op = add_text_op();
        op["data"]
            .as_object_mut()
            .unwrap()
            .insert("providerId".into(), Value::Null);
        assert_artifact_rejected(artifact(json!([op])));

        assert_artifact_rejected(artifact(json!([{
            "type": "update_node_data",
            "node_id": NODE_A,
            "patch": {}
        }])));
        assert_artifact_rejected(artifact(json!([{
            "type": "update_node_data",
            "node_id": NODE_A,
            "patch": { "status": "running" }
        }])));
    }

    #[test]
    fn rejects_delete_media_config_and_unknown_operations() {
        assert_artifact_rejected(artifact(json!([{
            "type": "delete_node",
            "node_id": NODE_A
        }])));

        for node_type in [
            "image",
            "panorama",
            "config",
            "video",
            "audio",
            "director",
            "group",
        ] {
            let error = assert_artifact_rejected(artifact(json!([{
                "type": "add_node",
                "node_type": node_type,
                "x": 0,
                "y": 0,
                "data": {
                    "text": "x",
                    "format": "plain",
                    "fontSize": 16,
                    "textAlign": "left"
                }
            }])));
            assert!(error.contains("node_type"), "{error}");
        }

        assert_artifact_rejected(artifact(json!([{
            "type": "run_generation",
            "node_id": NODE_A
        }])));
    }

    #[test]
    fn enforces_summary_batch_uuid_and_text_bounds() {
        for summary in ["".to_owned(), " padded ".to_owned(), "界".repeat(501)] {
            let mut value = artifact(all_allowed_ops());
            value["summary"] = Value::String(summary);
            assert_artifact_rejected(value);
        }

        assert_artifact_rejected(artifact(json!([])));
        assert_artifact_rejected(artifact(Value::Array(
            (0..65)
                .map(|_| json!({ "type": "move_node", "node_id": NODE_A, "x": 0, "y": 0 }))
                .collect(),
        )));

        for node_id in [
            "0190F5FE-7C00-7A00-8000-000000000801",
            "0190f5fe-7c00-4a00-8000-000000000801",
            "legacy-node",
        ] {
            assert_artifact_rejected(artifact(json!([{
                "type": "move_node",
                "node_id": node_id,
                "x": 0,
                "y": 0
            }])));
        }

        let mut op = add_text_op();
        op["data"]["text"] = Value::String("文".repeat(MAX_TEXT_CHARS + 1));
        assert_artifact_rejected(artifact(json!([op])));
    }

    #[test]
    fn enforces_finite_numbers_dimensions_font_size_and_handles() {
        for width in [0.0, 0.5] {
            let mut op = add_text_op();
            op["width"] = json!(width);
            assert_artifact_rejected(artifact(json!([op])));
        }

        for font_size in [7.99, 256.01] {
            let mut op = add_text_op();
            op["data"]["fontSize"] = json!(font_size);
            assert_artifact_rejected(artifact(json!([op])));
        }

        for handle in ["".to_owned(), " padded ".to_owned(), "x".repeat(129)] {
            assert_artifact_rejected(artifact(json!([{
                "type": "connect",
                "source_node_id": NODE_A,
                "target_node_id": NODE_B,
                "source_handle": handle
            }])));
        }

        let nonfinite = format!(
            "```json\n{{\"kind\":\"{CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}\",\"summary\":\"overflow\",\"ops\":[{{\"type\":\"move_node\",\"node_id\":\"{NODE_A}\",\"x\":1e400,\"y\":0}}]}}\n```"
        );
        assert_rejected(&nonfinite);
    }
}
