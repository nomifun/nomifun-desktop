//! Settings discovery shares the execution protocol, without tool-call
//! authority. Caller-provided credentials are used only for this transaction.
use std::collections::HashMap;

use super::{McpOwnerError, McpSession, McpSetupPolicy, McpInvocationControl, McpCleanupStatus, validate_http_endpoint};
use super::control::controlled;
use reqwest::header::HeaderMap;

/// The client must have redirects disabled. A protocol deadline does not
/// cancel the subsequent bounded cleanup of an already allocated HTTP session.
pub(crate) async fn discover_network(
    client: reqwest::Client,
    endpoint: &str,
    configured_headers: &HashMap<String, String>,
    legacy: bool,
    policy: &McpSetupPolicy,
    credential: Option<super::McpCredential>,
) -> Result<serde_json::Value, McpOwnerError> {
    validate_http_endpoint(endpoint)?;
    let headers = match credential.as_ref() {
        Some(credential) => super::build_headers(configured_headers, Some(credential))?,
        None => discovery_headers(configured_headers)?,
    };
    let mut session = McpSession::new(client, endpoint.to_owned(), headers);
    session.credential_lease = credential.as_ref().and_then(|credential| credential.lease.clone());
    session.legacy_mode = legacy;
    let setup = policy.start();
    let control = McpInvocationControl::default();
    let phase = setup.phase("protocol_setup", policy.protocol_timeout);
    let result = controlled(async {
        session.initialize().await?;
        let (_, tools) = session.read_catalog(None).await?;
        Ok(serde_json::json!({ "tools": tools }))
    }, &control, Some(phase.deadline), || phase.error()).await;
    let outcome = super::finish_session(session, result, &control).await;
    if outcome.cleanup.local == McpCleanupStatus::Unproven || outcome.cleanup.remote == McpCleanupStatus::Unproven {
        return Err(outcome.cleanup.error.expect("unproven cleanup has diagnostic"));
    }
    outcome.result
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
    // Draft probes use only explicitly supplied credentials. They cannot look
    // up a saved grant by URL. Saved connections use the shared authority.
    let secrets = configured.iter().filter(|(key, _)| crate::auth_service::credential_header(key))
        .map(|(key, value)| (key.clone(), value.clone())).collect();
    let credential = super::McpCredential::configured(&secrets)?;
    super::build_headers(configured, Some(&credential))
}
