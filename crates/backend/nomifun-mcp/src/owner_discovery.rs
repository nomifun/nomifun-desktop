//! Settings discovery shares the execution protocol, without tool-call
//! authority. Caller-provided credentials are used only for this transaction.
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use super::{McpOwnerError, McpSession, SESSION_CLEANUP_TIMEOUT, validate_http_endpoint};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};

/// The client must have redirects disabled. A protocol deadline does not
/// cancel the subsequent bounded cleanup of an already allocated HTTP session.
pub(crate) async fn discover_network(
    client: reqwest::Client,
    endpoint: &str,
    configured_headers: &HashMap<String, String>,
    legacy: bool,
    budget: Duration,
) -> Result<serde_json::Value, McpOwnerError> {
    validate_http_endpoint(endpoint)?;
    let headers = discovery_headers(configured_headers)?;
    let mut session = McpSession::new(client, endpoint.to_owned(), headers);
    session.legacy_mode = legacy;
    let outcome = tokio::time::timeout(budget, async {
        session.initialize().await?;
        let (_, tools) = session.read_catalog(None).await?;
        Ok(serde_json::json!({ "tools": tools }))
    })
    .await
    .unwrap_or_else(|_| {
        Err(McpOwnerError::new(
            "MCP_TIMEOUT",
            "MCP discovery timed out before its complete catalog was received",
        ))
    });
    let cleanup = tokio::time::timeout(SESSION_CLEANUP_TIMEOUT, session.close())
        .await
        .unwrap_or_else(|_| {
            Err(McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                "MCP discovery cleanup exceeded its bounded deadline",
            ))
        });
    // Never publish a successful catalog if explicit HTTP session cleanup is
    // unsupported/unknown. Dropping a legacy stream does not stop the server.
    cleanup?;
    outcome
}

fn discovery_headers(configured: &HashMap<String, String>) -> Result<HeaderMap, McpOwnerError> {
    if configured.len() > 128
        || configured
            .iter()
            .map(|(key, value)| key.len().saturating_add(value.len()))
            .sum::<usize>()
            > 64 * 1024
    {
        return Err(McpOwnerError::invalid_binding(
            "MCP discovery headers exceed the configured limit",
        ));
    }
    let mut headers = HeaderMap::new();
    let mut names = BTreeSet::new();
    for (key, value) in configured {
        let name = HeaderName::from_bytes(key.as_bytes()).map_err(|_| {
            McpOwnerError::invalid_binding("MCP discovery has an invalid header name")
        })?;
        if !names.insert(name.as_str().to_owned()) {
            return Err(McpOwnerError::invalid_binding(
                "MCP discovery has duplicate header names",
            ));
        }
        if matches!(
            name.as_str(),
            "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "upgrade"
                | "trailer"
                | "te"
                | "proxy-connection"
                | "proxy-authorization"
                | "last-event-id"
                | "mcp-session-id"
                | "mcp-protocol-version"
        ) {
            return Err(McpOwnerError::invalid_binding(
                "MCP discovery cannot override transport-owned headers",
            ));
        }
        let mut value = HeaderValue::from_str(value).map_err(|_| {
            McpOwnerError::invalid_binding("MCP discovery has an invalid header value")
        })?;
        // Settings may include existing OAuth or configured API-key headers.
        // Even custom header values must not appear in HTTP diagnostics.
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    Ok(headers)
}
