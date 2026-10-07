//! Stable identities for materialized MCP tools and frozen connections.
//!
//! Connection and OAuth lifecycle never enter tool identities. Each remote
//! tool becomes one namespaced Capability Module with one exact Action, and
//! the Snapshot separately freezes its server, tool-key, schema digest, and
//! materialization revision.

use std::collections::BTreeMap;

use nomifun_api_types::McpServerId;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::types::McpServerTransport;

pub const MCP_TOOL_CAPABILITY_PREFIX: &str = "nomi.mcp.v1.";
pub const MCP_TOOL_MATERIALIZATION_REVISION: u64 = 1;
pub const RETIRED_MCP_AUTHORING_CAPABILITY_IDS: [&str; 6] = [
    concat!("mcp", ".", "connect"),
    concat!("mcp", ".", "oauth"),
    concat!("mcp", ".", "resource"),
    concat!("mcp", ".", "tool_proxy"),
    concat!("connector", ".", "data", ".", "read"),
    concat!("connector", ".", "data", ".", "write"),
];

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum McpToolIdentityError {
    #[error("MCP server/tool identity is malformed")]
    Malformed,
    #[error("MCP server/tool identity could not be canonicalized")]
    Canonicalization,
}

/// Freeze only the executable connection settings, separately from catalog
/// schemas and the database's optimistic probe revision. Availability probes
/// and presentation edits must not invalidate an unchanged Session grant.
/// Configured credential values enter the digest but never the public ref.
pub fn canonical_mcp_connection_config_ref(
    server_id: &McpServerId,
    transport: &McpServerTransport,
) -> Result<String, McpToolIdentityError> {
    let payload = match transport {
        McpServerTransport::Stdio { command, args, env } => serde_json::to_vec(&(
            "nomifun-mcp-connection-v1", server_id.as_ref(), "stdio", command, args,
            env.iter().collect::<BTreeMap<_, _>>(),
        )),
        McpServerTransport::Sse { url, headers } => serde_json::to_vec(&(
            "nomifun-mcp-connection-v1", server_id.as_ref(), "sse", url,
            headers.iter().collect::<BTreeMap<_, _>>(),
        )),
        McpServerTransport::Http { url, headers } => serde_json::to_vec(&(
            "nomifun-mcp-connection-v1", server_id.as_ref(), "http", url,
            headers.iter().collect::<BTreeMap<_, _>>(),
        )),
    }.map_err(|_| McpToolIdentityError::Canonicalization)?;
    Ok(format!("mcp-server:{}@sha256:{:x}", server_id.as_ref(), Sha256::digest(payload)))
}

/// Build the stable, namespaced Capability ID for one remote MCP tool.
///
/// The remote name is never accepted from model arguments. A catalog refresh
/// must recompute this identity from its exact server row and `tools/list`
/// descriptor before publishing the corresponding Action.
pub fn canonical_mcp_tool_capability_id(
    server_id: &McpServerId,
    remote_tool_name: &str,
) -> Result<String, McpToolIdentityError> {
    validate_remote_tool_name(remote_tool_name)?;
    let payload = serde_json::to_vec(&(server_id.as_ref(), remote_tool_name))
        .map_err(|_| McpToolIdentityError::Canonicalization)?;
    Ok(format!(
        "{MCP_TOOL_CAPABILITY_PREFIX}{:x}",
        Sha256::digest(payload)
    ))
}

pub fn is_namespaced_mcp_tool_capability(value: &str) -> bool {
    value
        .strip_prefix(MCP_TOOL_CAPABILITY_PREFIX)
        .is_some_and(|suffix| {
            suffix.len() == 64
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

pub fn is_retired_mcp_authoring_capability(value: &str) -> bool {
    RETIRED_MCP_AUTHORING_CAPABILITY_IDS.contains(&value)
}

pub fn canonical_mcp_tool_action_id(
    capability_id: &str,
) -> Result<String, McpToolIdentityError> {
    if !is_namespaced_mcp_tool_capability(capability_id) {
        return Err(McpToolIdentityError::Malformed);
    }
    Ok(format!("{capability_id}.invoke"))
}

fn validate_remote_tool_name(value: &str) -> Result<(), McpToolIdentityError> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(McpToolIdentityError::Malformed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SERVER_A: &str = "0195f7c0-7b6a-7c21-8f4a-1234567890ab";
    const SERVER_B: &str = "0195f7c0-7b6a-7c21-8f4a-1234567890ac";

    #[test]
    fn connection_ref_is_order_independent_and_server_specific() {
        let server_a = McpServerId::parse(SERVER_A).unwrap();
        let server_b = McpServerId::parse(SERVER_B).unwrap();
        let transport = McpServerTransport::Http {
            url: "https://example.test/mcp".into(),
            headers: HashMap::from([("x-one".into(), "one".into()), ("x-two".into(), "two".into())]),
        };
        let reordered = McpServerTransport::Http {
            url: "https://example.test/mcp".into(),
            headers: HashMap::from([("x-two".into(), "two".into()), ("x-one".into(), "one".into())]),
        };
        let first = canonical_mcp_connection_config_ref(&server_a, &transport).unwrap();
        assert_eq!(canonical_mcp_connection_config_ref(&server_a, &reordered).unwrap(), first);
        assert_ne!(canonical_mcp_connection_config_ref(&server_b, &transport).unwrap(), first);
    }

    #[test]
    fn connection_ref_changes_with_transport_and_credentials_without_exposing_them() {
        let server = McpServerId::parse(SERVER_A).unwrap();
        let transport = McpServerTransport::Http {
            url: "https://example.test/mcp".into(),
            headers: HashMap::from([("authorization".into(), "Bearer private-secret".into())]),
        };
        let first = canonical_mcp_connection_config_ref(&server, &transport).unwrap();
        assert!(!first.contains("private-secret"));
        assert!(!first.contains("example.test"));
        let mut changed = transport.clone();
        let McpServerTransport::Http { headers, .. } = &mut changed else { unreachable!() };
        headers.insert("authorization".into(), "Bearer changed-secret".into());
        assert_ne!(canonical_mcp_connection_config_ref(&server, &changed).unwrap(), first);
        let McpServerTransport::Http { url, headers } = transport else { unreachable!() };
        assert_ne!(canonical_mcp_connection_config_ref(&server, &McpServerTransport::Sse { url, headers }).unwrap(), first);
    }

    #[test]
    fn stdio_connection_ref_covers_command_arguments_and_environment() {
        let server = McpServerId::parse(SERVER_A).unwrap();
        let original = McpServerTransport::Stdio {
            command: "node".into(), args: vec!["server.js".into()],
            env: HashMap::from([("TOKEN".into(), "secret".into())]),
        };
        let first = canonical_mcp_connection_config_ref(&server, &original).unwrap();
        for changed in [
            McpServerTransport::Stdio { command: "bun".into(), args: vec!["server.js".into()], env: HashMap::from([("TOKEN".into(), "secret".into())]) },
            McpServerTransport::Stdio { command: "node".into(), args: vec!["other.js".into()], env: HashMap::from([("TOKEN".into(), "secret".into())]) },
            McpServerTransport::Stdio { command: "node".into(), args: vec!["server.js".into()], env: HashMap::from([("TOKEN".into(), "other".into())]) },
        ] {
            assert_ne!(canonical_mcp_connection_config_ref(&server, &changed).unwrap(), first);
        }
    }

    #[test]
    fn identity_is_stable_namespaced_and_pair_specific() {
        let server_a = McpServerId::parse(SERVER_A).unwrap();
        let server_b = McpServerId::parse(SERVER_B).unwrap();
        let first = canonical_mcp_tool_capability_id(&server_a, "lookup").unwrap();
        assert!(is_namespaced_mcp_tool_capability(&first));
        assert_eq!(
            canonical_mcp_tool_capability_id(&server_a, "lookup").unwrap(),
            first
        );
        assert_ne!(
            canonical_mcp_tool_capability_id(&server_b, "lookup").unwrap(),
            first
        );
        assert_ne!(
            canonical_mcp_tool_capability_id(&server_a, "write").unwrap(),
            first
        );
        assert_eq!(
            canonical_mcp_tool_action_id(&first).unwrap(),
            format!("{first}.invoke")
        );
    }

    #[test]
    fn legacy_broad_ids_and_malformed_parts_are_not_tool_identities() {
        for legacy in RETIRED_MCP_AUTHORING_CAPABILITY_IDS {
            assert!(!is_namespaced_mcp_tool_capability(legacy));
            assert_eq!(
                canonical_mcp_tool_action_id(legacy),
                Err(McpToolIdentityError::Malformed)
            );
        }
        let server = McpServerId::parse(SERVER_A).unwrap();
        for tool in ["", " lookup", "lookup\n"] {
            assert_eq!(
                canonical_mcp_tool_capability_id(&server, tool),
                Err(McpToolIdentityError::Malformed)
            );
        }
        for invalid in ["server-a", "550e8400-e29b-41d4-a716-446655440000"] {
            assert!(McpServerId::parse(invalid).is_err());
        }
    }
}
