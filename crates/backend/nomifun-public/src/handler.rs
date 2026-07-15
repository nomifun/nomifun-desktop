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

use nomifun_gateway::{CallerCtx, GatewayDeps, Registry, Surface};
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};

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

fn domain_scope_from_context(ctx: &RequestContext<RoleServer>) -> Option<Vec<String>> {
    let parts = ctx.extensions.get::<axum::http::request::Parts>()?;
    domain_scope_from_query(parts.uri.query())
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
        let query_scope = domain_scope_from_context(&context);
        let specs = match (self.domains, query_scope.as_deref()) {
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
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let query_scope = domain_scope_from_context(&ctx);
        let allowed_specs = match (self.domains, query_scope.as_deref()) {
            (Some(domains), _) => Registry::global().tool_specs_for(Surface::Remote, domains),
            (None, scope) => remote_specs_for_scope(scope),
        };
        if !allowed_specs.iter().any(|spec| spec.name == request.name) {
            return Ok(crate::result::build_tool_result(serde_json::json!({
                "error": format!("Tool '{}' is outside the configured Remote MCP capability scope", request.name)
            })));
        }
        // External caller == the Remote surface, bound to one companion (外部伙伴).
        // rmcp injects the originating HTTP `Parts` into the request extensions;
        // our companion_token_middleware stashed the resolved companion there.
        let companion_id = ctx
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<crate::router::RemoteCompanion>())
            .map(|rc| rc.0.clone());
        let Some(companion_id) = companion_id else {
            return Ok(crate::result::build_tool_result(serde_json::json!({
                "error": "authenticated Remote MCP request has no canonical companion identity"
            })));
        };
        let caller = match CallerCtx::try_remote(
            &self.deps.authoritative_user_id,
            &companion_id,
        ) {
            Ok(caller) => caller,
            Err(error) => {
                return Ok(crate::result::build_tool_result(serde_json::json!({
                    "error": format!("invalid authenticated identity: {error}")
                })));
            }
        };
        let result = match Registry::global()
            .dispatch_opt(self.deps.clone(), caller, &request.name, &args)
            .await
        {
            Some(value) => value,
            None => serde_json::json!({ "error": format!("Unknown tool: {}", request.name) }),
        };
        Ok(crate::result::build_tool_result(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
