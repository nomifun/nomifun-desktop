//! `RemoteMcpHandler` — the rmcp `ServerHandler` that projects the gateway
//! `Registry` onto the Remote (external companion) surface.
//!
//! `list_tools` → `Registry::tool_specs(Surface::Remote)` (Deny-gated tools are
//! invisible). `call_tool` → `Registry::dispatch_opt` with a `CallerCtx` whose
//! `remote` marker forces `Surface::Remote`, so the danger matrix (Read/Write
//! Allow, Destructive Confirm, Sensitive Deny) is enforced centrally. The
//! handler is stateless apart from the shared `Arc<GatewayDeps>`; a fresh
//! instance is produced per session by the transport's service factory.

use std::sync::Arc;

#[cfg(feature = "browser-use")]
use std::collections::BTreeSet;

use nomifun_gateway::{CallerCtx, GatewayDeps, Registry, Surface};
#[cfg(feature = "browser-use")]
use nomifun_browser_platform::BrowserOperationKind;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};

use crate::idempotency::{CONVERSATION_SEND_TOOL, remote_operation_id};
use crate::session::{RemoteMcpSessionId, RemoteMcpSessionIdentity};

fn is_browser_tool(tool_name: &str) -> bool {
    tool_name.starts_with("nomi_browser_")
}

#[cfg(feature = "browser-use")]
fn browser_operation_scope(
    specs: &[nomifun_gateway::ToolSpec],
) -> BTreeSet<BrowserOperationKind> {
    if specs.iter().any(|spec| spec.domain == "browser") {
        nomifun_gateway::browser_registry::all_browser_operations()
    } else {
        BTreeSet::new()
    }
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then_some(v)
    })
}

pub(crate) fn domain_scope_from_query(query: Option<&str>) -> Option<Vec<String>> {
    let query = query?;
    if let Some(domains) = query_value(query, "domains") {
        let selected: Vec<String> = domains
            .split(',')
            .map(str::trim)
            .filter(|domain| !domain.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        return (!selected.is_empty()).then_some(selected);
    }
    match query_value(query, "profile") {
        Some("agent") => Some(
            crate::AGENT_PROFILE_DOMAINS
                .iter()
                .map(|d| d.to_string())
                .collect(),
        ),
        _ => None,
    }
}

fn remote_specs_for_scope(scope: Option<&[String]>) -> Vec<nomifun_gateway::ToolSpec> {
    match scope {
        Some(domains) => {
            let domain_refs: Vec<&str> = domains.iter().map(String::as_str).collect();
            Registry::global().tool_specs_for(Surface::Remote, &domain_refs)
        }
        None => Registry::global().tool_specs(Surface::Remote),
    }
}

/// MCP server handler for external (network) callers. One per MCP session;
/// holds a clone of the shared gateway service bundle. `domains` optionally
/// restricts `tools/list` to a curated profile (e.g. the `agent` profile);
/// `None` advertises the full Remote surface.
#[derive(Clone)]
pub struct RemoteMcpHandler {
    deps: Arc<GatewayDeps>,
    domains: Option<&'static [&'static str]>,
}

impl RemoteMcpHandler {
    pub fn new(deps: Arc<GatewayDeps>) -> Self {
        Self {
            deps,
            domains: None,
        }
    }

    /// Curated profile: only advertise capabilities in `domains`.
    pub fn with_domains(deps: Arc<GatewayDeps>, domains: &'static [&'static str]) -> Self {
        Self {
            deps,
            domains: Some(domains),
        }
    }
}

#[cfg(feature = "browser-use")]
async fn preflight_and_attach_remote_browser_identity(
    registry: nomifun_gateway::browser_registry::BrowserRegistry,
    caller: CallerCtx,
    tool_name: &str,
    args: &serde_json::Value,
    session_id: String,
    allowed_operations: BTreeSet<BrowserOperationKind>,
) -> Result<CallerCtx, serde_json::Value> {
    match Registry::global().validate_arguments(tool_name, args) {
        Some(Ok(())) => {}
        Some(Err(error)) => return Err(error),
        None => return Err(serde_json::json!({
            "error": format!("Unknown tool: {tool_name}")
        })),
    }
    registry
        .validate_managed_request(&caller, tool_name, args)
        .await
        .map_err(nomifun_gateway::browser_registry::platform_error_to_value)?;
    attach_remote_browser_identity(
        registry,
        caller,
        session_id,
        allowed_operations,
    )
    .await
}

#[cfg(feature = "browser-use")]
async fn attach_remote_browser_identity(
    registry: nomifun_gateway::browser_registry::BrowserRegistry,
    mut caller: CallerCtx,
    session_id: String,
    allowed_operations: BTreeSet<BrowserOperationKind>,
) -> Result<CallerCtx, serde_json::Value> {
    registry
        .attach_trusted_identity_scoped(
            &mut caller,
            &session_id,
            None,
            u64::MAX,
            nomifun_gateway::browser_registry::BrowserAttachmentAuthority::RemoteMcpSession,
            allowed_operations,
        )
        .await
        .map(|_| caller)
        .map_err(nomifun_gateway::browser_registry::platform_error_to_value)
}

impl ServerHandler for RemoteMcpHandler {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo is #[non_exhaustive] — build from Default then set fields.
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "NomiFun external companion. These tools drive the NomiFun platform \
             (agent / browser / computer / knowledge / files / and platform control). \
             Destructive actions require re-calling with `confirm: true`; some sensitive \
             actions are disabled on this surface."
                .to_string(),
        );
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let Some(identity) = context.extensions.get::<RemoteMcpSessionIdentity>() else {
            return Err(rmcp::ErrorData::invalid_request(
                "authenticated Remote MCP request has no server-pinned session identity",
                None,
            ));
        };
        let scope = identity.scope.as_deref();
        let specs = match (self.domains, scope) {
            (Some(domains), _) => Registry::global().tool_specs_for(Surface::Remote, domains),
            (None, scope) => remote_specs_for_scope(scope),
        };
        let tools: Vec<Tool> = specs
            .into_iter()
            .map(|spec| Tool::new(spec.name, spec.description, Arc::new(spec.input_schema)))
            .collect();
        Ok(ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let tool_name = request.name.into_owned();
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let scope = ctx
            .extensions
            .get::<RemoteMcpSessionIdentity>()
            .map(|identity| identity.scope.as_deref())
            .flatten();
        let allowed_specs = match (self.domains, scope) {
            (Some(domains), _) => Registry::global().tool_specs_for(Surface::Remote, domains),
            (None, scope) => remote_specs_for_scope(scope),
        };
        if !allowed_specs.iter().any(|spec| spec.name == tool_name) {
            return Ok(crate::result::build_tool_result(serde_json::json!({
                "error": format!("Tool '{tool_name}' is outside the configured Remote MCP capability scope")
            })));
        }
        // External caller == the Remote surface, bound to one companion (外部伙伴).
        // rmcp injects the originating HTTP `Parts` into the request extensions;
        // our companion_token_middleware stashed the resolved companion there.
        let request_parts = ctx
            .extensions
            .get::<axum::http::request::Parts>();
        let request_companion_id = request_parts
            .and_then(|parts| {
                parts
                    .extensions
                    .get::<crate::router::RemoteCompanion>()
            })
            .map(|rc| rc.0.clone());
        let Some(session_identity) = ctx.extensions.get::<RemoteMcpSessionIdentity>() else {
            return Ok(crate::result::build_tool_result(serde_json::json!({
                "error": "authenticated Remote MCP request has no server-pinned session identity"
            })));
        };
        if request_companion_id.as_ref() != Some(&session_identity.companion_id) {
            return Ok(crate::result::build_tool_result(serde_json::json!({
                "error": "authenticated Remote MCP request companion does not match its pinned session identity"
            })));
        }
        let companion_id = session_identity.companion_id.clone();
        let Some(session_id_marker) = ctx.extensions.get::<RemoteMcpSessionId>() else {
            return Ok(crate::result::build_tool_result(serde_json::json!({
                "error": "authenticated Remote MCP request has no validated logical session identity"
            })));
        };
        if session_identity.session_id != session_id_marker.0 {
            return Ok(crate::result::build_tool_result(serde_json::json!({
                "error": "authenticated Remote MCP request has inconsistent session identity"
            })));
        }
        let mut caller = match CallerCtx::try_remote(
            &self.deps.authoritative_user_id,
            companion_id.as_str(),
        ) {
            Ok(caller) => caller,
            Err(error) => {
                return Ok(crate::result::build_tool_result(serde_json::json!({
                    "error": format!("invalid authenticated identity: {error}")
                })));
            }
        };
        if is_browser_tool(&tool_name) {
            #[cfg(not(feature = "browser-use"))]
            {
                return Ok(crate::result::build_tool_result(serde_json::json!({
                    "error": "browser tools are not available on this Remote MCP host"
                })));
            }
            #[cfg(feature = "browser-use")]
            {
                let allowed_operations = browser_operation_scope(&allowed_specs);
                if allowed_operations.is_empty() {
                    return Ok(crate::result::build_tool_result(serde_json::json!({
                        "error": "browser tool is outside the server-derived browser operation scope"
                    })));
                }
                let Some(session_id) = ctx.extensions.get::<RemoteMcpSessionId>() else {
                    return Ok(crate::result::build_tool_result(
                        serde_json::json!({
                            "error": "authenticated Remote MCP browser request has no validated logical session identity"
                        }),
                    ));
                };
                let Some(registry) = self.deps.browser_registry.as_ref() else {
                    return Ok(crate::result::build_tool_result(serde_json::json!({
                        "error": "browser tools are not available on this Remote MCP host"
                    })));
                };
                caller = match preflight_and_attach_remote_browser_identity(
                    registry.clone(),
                    caller,
                    &tool_name,
                    &args,
                    session_id.0.as_ref().to_owned(),
                    allowed_operations,
                )
                .await
                {
                    Ok(caller) => caller,
                    Err(error) => return Ok(crate::result::build_tool_result(error)),
                };
            }
        }
        if tool_name == CONVERSATION_SEND_TOOL {
            let Some(parts) = request_parts else {
                return Err(rmcp::ErrorData::invalid_request(
                    "authenticated Remote MCP request has no transport headers",
                    None,
                ));
            };
            caller.operation_id = Some(
                remote_operation_id(
                    &parts.headers,
                    companion_id.as_str(),
                    &tool_name,
                )
                .map_err(|error| {
                    rmcp::ErrorData::invalid_request(
                        format!("invalid Idempotency-Key: {error}"),
                        None,
                    )
                })?,
            );
        }
        let result = match Registry::global()
            .dispatch_opt(self.deps.clone(), caller, &tool_name, &args)
            .await
        {
            Some(value) => value,
            None => serde_json::json!({ "error": format!("Unknown tool: {tool_name}") }),
        };
        Ok(crate::result::build_tool_result(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "browser-use")]
    use std::future::Future;
    #[cfg(feature = "browser-use")]
    use std::pin::Pin;
    #[cfg(feature = "browser-use")]
    use std::sync::Arc;

    #[cfg(feature = "browser-use")]
    use nomifun_browser_platform::{
        BrowserHostDriver, BrowserHostFactory, BrowserPlatformError,
        BrowserSessionHub, HostLaunchRequest, HubConfig, ManualClock,
    };

    #[cfg(feature = "browser-use")]
    struct TestBrowserFactory;

    #[cfg(feature = "browser-use")]
    impl BrowserHostFactory for TestBrowserFactory {
        fn launch<'a, 'async_trait>(
            &'a self,
            _request: HostLaunchRequest,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            Arc<dyn BrowserHostDriver>,
                            BrowserPlatformError,
                        >,
                    > + Send
                    + 'async_trait,
            >,
        >
        where
            'a: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async {
                Err(BrowserPlatformError::new(
                    nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                    "test browser factory is not used by this attachment test",
                    false,
                    "test",
                ))
            })
        }
    }

    #[test]
    fn domain_scope_from_query_reads_custom_domains() {
        assert_eq!(
            domain_scope_from_query(Some("domains=agent,conversation,files")),
            Some(vec![
                "agent".to_string(),
                "conversation".to_string(),
                "files".to_string()
            ])
        );
        assert_eq!(
            domain_scope_from_query(Some("profile=agent")),
            Some(
                crate::AGENT_PROFILE_DOMAINS
                    .iter()
                    .map(|d| d.to_string())
                    .collect()
            )
        );
        assert_eq!(domain_scope_from_query(Some("domains=")), None);
        assert_eq!(domain_scope_from_query(None), None);
    }

    #[test]
    fn domain_scope_from_query_is_not_an_authorization_expander() {
        let requested = domain_scope_from_query(Some("domains=files,agent,files"))
            .expect("explicit domains");
        let domain_refs: Vec<&str> = requested.iter().map(String::as_str).collect();
        let specs = Registry::global().tool_specs_for(Surface::Remote, &domain_refs);
        assert!(
            specs
                .iter()
                .all(|spec| domain_refs.contains(&spec.domain)),
            "tool selection remains limited to server-derived domains"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn remote_browser_preflight_rejects_invalid_args_before_attach_or_renewal() {
        let clock = Arc::new(ManualClock::new(1_000));
        let hub = BrowserSessionHub::with_clock(
            Arc::new(TestBrowserFactory),
            HubConfig {
                owner_lease_ttl_ms: 100,
                ..HubConfig::default()
            },
            clock.clone(),
        );
        let registry = nomifun_gateway::browser_registry::BrowserRegistry::from_hub(hub);
        let caller = CallerCtx::try_remote(
            "0190f5fe-7c00-7a00-8000-000000000001",
            "0190f5fe-7c00-7a00-8000-000000000002",
        )
        .expect("test Remote caller identity");
        let allowed_operations =
            nomifun_gateway::browser_registry::all_browser_operations();

        let invalid = preflight_and_attach_remote_browser_identity(
            registry.clone(),
            caller.clone(),
            "nomi_browser_open",
            &serde_json::json!({"lane_name": 7}),
            "remote-preflight-invalid".to_owned(),
            allowed_operations.clone(),
        )
        .await
        .expect_err("wrongly typed browser args must fail typed preflight");
        assert!(
            invalid
                .get("error")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|error| error.contains("invalid arguments")),
            "expected typed validation error, got {invalid}"
        );
        let revoked = registry
            .revoke_trusted_identity("remote-preflight-invalid")
            .await
            .expect("checking an unattached runtime must succeed");
        assert!(
            revoked.already_closed,
            "invalid args must not create a Remote MCP browser owner attachment"
        );

        let mut attached = preflight_and_attach_remote_browser_identity(
            registry.clone(),
            caller.clone(),
            "nomi_browser_open",
            &serde_json::json!({"lane_name": "default"}),
            "remote-preflight-renewal".to_owned(),
            allowed_operations.clone(),
        )
        .await
        .expect("valid browser args must attach the Remote MCP owner");
        let first_lease_id = attached
            .browser_identity
            .take()
            .expect("valid browser args must publish a browser identity")
            .owner_lease_id;

        clock.advance(50);
        let invalid = preflight_and_attach_remote_browser_identity(
            registry.clone(),
            caller.clone(),
            "nomi_browser_open",
            &serde_json::json!({"lane_name": 7}),
            "remote-preflight-renewal".to_owned(),
            allowed_operations.clone(),
        )
        .await
        .expect_err("invalid renewal args must fail before owner renewal");
        assert!(
            invalid
                .get("error")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|error| error.contains("invalid arguments")),
            "expected typed validation error on renewal, got {invalid}"
        );

        clock.advance(60);
        let renewed = preflight_and_attach_remote_browser_identity(
            registry.clone(),
            caller,
            "nomi_browser_open",
            &serde_json::json!({"lane_name": "default"}),
            "remote-preflight-renewal".to_owned(),
            allowed_operations,
        )
        .await
        .expect("valid browser args must still attach after the lease expires");
        assert_ne!(
            renewed
                .browser_identity
                .expect("replacement attachment must publish a browser identity")
                .owner_lease_id,
            first_lease_id,
            "invalid args must not renew the existing Remote MCP owner lease"
        );
    }

}
