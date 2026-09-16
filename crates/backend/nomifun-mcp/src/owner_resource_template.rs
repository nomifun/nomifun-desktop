//! Bounded RFC 6570 string/list/associative expansion. No fetching, filesystem
//! resolution or wildcard URI matching. Nested values and coercion are forbidden.
use super::McpOwnerError;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 4096;
const MAX_VARIABLES: usize = 64;
const MAX_COMPONENTS: usize = 256;

fn invalid() -> McpOwnerError {
    McpOwnerError::new(
        "MCP_RESOURCE_TEMPLATE_INVALID",
        "Resource template requires bounded strings, string lists or string maps and an absolute expanded URI; nested/null/numeric values and composite prefixes are invalid",
    )
}

#[derive(Clone, Copy)]
struct Variable<'a> {
    name: &'a str,
    prefix: Option<usize>,
    explode: bool,
}

fn variable(value: &str) -> Result<Variable<'_>, McpOwnerError> {
    let (name, prefix) = if let Some(value) = value.strip_suffix('*') {
        // Explode on a scalar has the same representation as an ordinary scalar.
        (value, None)
    } else if let Some((name, prefix)) = value.split_once(':') {
        if prefix.is_empty()
            || prefix.len() > 4
            || prefix.starts_with('0')
            || !prefix.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid());
        }
        (name, Some(prefix.parse::<usize>().map_err(|_| invalid())?))
    } else {
        (value, None)
    };
    if name.is_empty() || name.len() > 256 {
        return Err(invalid());
    }
    let bytes = name.as_bytes();
    let mut index = 0;
    let mut dot = true;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'.' {
            if dot {
                return Err(invalid());
            }
            dot = true;
        } else if byte == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return Err(invalid());
            }
            index += 2;
            dot = false;
        } else if byte.is_ascii_alphanumeric() || byte == b'_' {
            dot = false;
        } else {
            return Err(invalid());
        }
        index += 1;
    }
    if dot {
        return Err(invalid());
    }
    Ok(Variable {
        name,
        prefix,
        explode: value.ends_with('*'),
    })
}

fn validate_values(values: &BTreeMap<String, Value>) -> Result<(), McpOwnerError> {
    if values.len() > MAX_VARIABLES {
        return Err(invalid());
    }
    let mut bytes = values.keys().map(String::len).sum::<usize>();
    let mut components = 0;
    let mut text = |value: &str| -> Result<(), McpOwnerError> {
        bytes = bytes.saturating_add(value.len());
        components += 1;
        if bytes > MAX_BYTES || components > MAX_COMPONENTS || value.chars().any(char::is_control) {
            return Err(invalid());
        }
        Ok(())
    };
    for (key, value) in values {
        if key.is_empty() || key.len() > 256 || key.chars().any(char::is_control) {
            return Err(invalid());
        }
        match value {
            Value::String(value) => text(value)?,
            Value::Array(values) if values.len() <= MAX_COMPONENTS => {
                for value in values {
                    text(value.as_str().ok_or_else(invalid)?)?;
                }
            }
            Value::Object(values) if values.len() <= MAX_COMPONENTS => {
                for (key, value) in values {
                    text(key)?;
                    text(value.as_str().ok_or_else(invalid)?)?;
                }
            }
            _ => return Err(invalid()),
        }
    }
    // Empty collections consume no components, but their names still count.
    if bytes > MAX_BYTES {
        return Err(invalid());
    }
    Ok(())
}

fn named_value(name: &str, value: String, named: bool, empty_equals: bool) -> String {
    if !named {
        return value;
    }
    if value.is_empty() && !empty_equals {
        return name.to_owned();
    }
    format!("{name}={value}")
}

/// One item per expression separator. Empty collections are undefined; an
/// empty scalar (or a list containing an empty scalar) remains a defined value.
fn expand_value(
    var: Variable<'_>,
    value: &Value,
    named: bool,
    empty_equals: bool,
    reserved: bool,
) -> Result<Vec<String>, McpOwnerError> {
    if var.prefix.is_some() && !value.is_string() {
        return Err(invalid());
    }
    match value {
        Value::String(value) => {
            let value = if let Some(prefix) = var.prefix {
                // Do not truncate an already %-encoded UTF-8 sequence.
                if value.contains('%') {
                    return Err(invalid());
                }
                &value[..value
                    .char_indices()
                    .nth(prefix)
                    .map_or(value.len(), |(byte, _)| byte)]
            } else {
                value.as_str()
            };
            Ok(vec![named_value(
                var.name,
                encode(value, reserved),
                named,
                empty_equals,
            )])
        }
        Value::Array(values) => {
            if values.is_empty() {
                return Ok(Vec::new());
            }
            let values = values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(|value| encode(value, reserved))
                        .ok_or_else(invalid)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if var.explode {
                Ok(values
                    .into_iter()
                    .map(|value| named_value(var.name, value, named, empty_equals))
                    .collect())
            } else {
                Ok(vec![named_value(
                    var.name,
                    values.join(","),
                    named,
                    empty_equals,
                )])
            }
        }
        Value::Object(values) => {
            if values.is_empty() {
                return Ok(Vec::new());
            }
            // Stable associative ordering, independent of serde_json's map feature.
            let ordered = values.iter().collect::<BTreeMap<_, _>>();
            let mut parts = Vec::new();
            for (key, value) in ordered {
                let key = encode(key, reserved);
                let value = encode(value.as_str().ok_or_else(invalid)?, reserved);
                if var.explode {
                    // Exploded keys replace the variable name. For unnamed
                    // operators key=value is still required, including empty values.
                    parts.push(named_value(&key, value, true, !named || empty_equals));
                } else {
                    parts.push(key);
                    parts.push(value);
                }
            }
            if var.explode {
                Ok(parts)
            } else {
                Ok(vec![named_value(
                    var.name,
                    parts.join(","),
                    named,
                    empty_equals,
                )])
            }
        }
        _ => Err(invalid()),
    }
}

fn encode(value: &str, reserved: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut result = String::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphanumeric()
            || b"-._~".contains(&byte)
            || (reserved && b":/?#[]@!$&'()*+,;=".contains(&byte))
        {
            result.push(byte as char);
        } else if reserved
            && byte == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            result.push_str(&value[index..index + 3]);
            index += 2;
        } else {
            result.push('%');
            result.push(HEX[(byte >> 4) as usize] as char);
            result.push(HEX[(byte & 15) as usize] as char);
        }
        index += 1;
    }
    result
}

fn expand_inner(
    template: &str,
    values: &BTreeMap<String, Value>,
) -> Result<(String, BTreeSet<String>), McpOwnerError> {
    if template.is_empty()
        || template.len() > MAX_BYTES
        || template
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid());
    }
    validate_values(values)?;
    let mut output = String::new();
    let mut names = BTreeSet::new();
    let mut rest = template;
    let mut expressions = 0;
    while let Some(open) = rest.find('{') {
        let literal = &rest[..open];
        validate_literal(literal)?;
        output.push_str(literal);
        let tail = &rest[open + 1..];
        let close = tail.find('}').ok_or_else(invalid)?;
        let expression = &tail[..close];
        expressions += 1;
        if expression.is_empty() || expressions > MAX_VARIABLES {
            return Err(invalid());
        }
        let operator = expression.as_bytes()[0];
        let (first, separator, named, empty_equals, reserved, specs) = match operator {
            b'+' => ("", ",", false, false, true, &expression[1..]),
            b'#' => ("#", ",", false, false, true, &expression[1..]),
            b'.' => (".", ".", false, false, false, &expression[1..]),
            b'/' => ("/", "/", false, false, false, &expression[1..]),
            b';' => (";", ";", true, false, false, &expression[1..]),
            b'?' => ("?", "&", true, true, false, &expression[1..]),
            b'&' => ("&", "&", true, true, false, &expression[1..]),
            _ => ("", ",", false, false, false, expression),
        };
        let mut emitted = false;
        for (index, spec) in specs.split(',').enumerate() {
            if index >= MAX_VARIABLES {
                return Err(invalid());
            }
            let var = variable(spec)?;
            names.insert(var.name.to_owned());
            if names.len() > MAX_VARIABLES {
                return Err(invalid());
            }
            let Some(value) = values.get(var.name) else {
                continue;
            };
            for part in expand_value(var, value, named, empty_equals, reserved)? {
                output.push_str(if emitted { separator } else { first });
                emitted = true;
                output.push_str(&part);
                if output.len() > MAX_BYTES {
                    return Err(invalid());
                }
            }
        }
        rest = &tail[close + 1..];
    }
    validate_literal(rest)?;
    output.push_str(rest);
    if output.len() > MAX_BYTES || values.keys().any(|name| !names.contains(name)) {
        return Err(invalid());
    }
    Ok((output, names))
}

fn validate_literal(value: &str) -> Result<(), McpOwnerError> {
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().copied().enumerate() {
        if b"{}\"<>\\^`|".contains(&byte)
            || (byte == b'%'
                && (index + 2 >= bytes.len()
                    || !bytes[index + 1].is_ascii_hexdigit()
                    || !bytes[index + 2].is_ascii_hexdigit()))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Pure local preflight, also used before reserving a durable owner receipt.
pub(super) fn expand(
    template: &str,
    values: &BTreeMap<String, Value>,
) -> Result<String, McpOwnerError> {
    let (uri, _) = expand_inner(template, values)?;
    super::resources::validate_uri(&uri)?;
    Ok(uri)
}

pub(super) fn variables(template: &str) -> Result<BTreeSet<String>, McpOwnerError> {
    // Discovery checks grammar only; optional variables can make an empty
    // expansion non-absolute even when a fully supplied expansion is valid.
    expand_inner(template, &BTreeMap::new()).map(|(_, names)| names)
}
