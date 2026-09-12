//! Session-bound lazy MCP runtime for canonical Agent capabilities.
//!
//! Construction retains only exact resource/config references. The host
//! connector resolves transports and credentials after explicit activation;
//! model input can never provide owner, Session, endpoint, or secret fields.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_mcp::manager::McpManager;
use nomi_mcp::protocol::McpToolDef;
use nomi_mcp::tool_proxy::McpToolProxy;
use nomi_protocol::events::ToolCategory;
use nomi_tools::{Tool, ToolExecutionContext};
use nomi_types::tool::{JsonSchema, ToolResult};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp_capability_tools::{
    MCP_RESOURCE_LIST_TOOL_NAME, MCP_RESOURCE_READ_TOOL_NAME,
    McpResourceListTool, McpResourceReadTool,
};

pub const MCP_CONNECT_TOOL_NAME: &str = "mcp_connect";
pub const MCP_GENERIC_PROXY_TOOL_NAME: &str = "mcp_tool_proxy";
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const TOOL_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_BINDINGS: usize = 16;
const MAX_SELECTOR_BYTES: usize = 256;

/// Exact server-owned binding identity. Secret bytes cannot enter this type.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionMcpBindingRef {
    resource_id: String,
    connection_config_ref: String,
}

impl SessionMcpBindingRef {
    pub fn new(
        resource_id: impl Into<String>,
        connection_config_ref: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            resource_id: exact_identity("MCP resource ID", resource_id.into())?,
            connection_config_ref: exact_identity(
                "MCP connection-config reference",
                connection_config_ref.into(),
            )?,
        })
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub fn connection_config_ref(&self) -> &str {
        &self.connection_config_ref
    }
}

fn exact_identity(label: &str, value: String) -> Result<String, String> {
    if value.is_empty()
        || value != value.trim()
        || value.len() > 512
        || value.chars().any(char::is_control)
    {
        Err(format!(
            "{label} must be a trimmed non-empty value of at most 512 bytes"
        ))
    } else {
        Ok(value)
    }
}

/// Stable, non-reflective failure surface. Connector implementations log only
/// their already-redacted internal diagnostics, then return one category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMcpConnectFailure {
    ResourceUnavailable,
    CredentialUnavailable,
    TransportUnavailable,
    CleanupUnavailable,
}

impl SessionMcpConnectFailure {
    fn code(self) -> &'static str {
        match self {
            Self::ResourceUnavailable => "MCP_RESOURCE_UNAVAILABLE",
            Self::CredentialUnavailable => "MCP_CREDENTIAL_UNAVAILABLE",
            Self::TransportUnavailable => "MCP_TRANSPORT_UNAVAILABLE",
            Self::CleanupUnavailable => "MCP_CLEANUP_UNAVAILABLE",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::ResourceUnavailable => "the bound MCP server is unavailable",
            Self::CredentialUnavailable => "the bound MCP credential could not be resolved",
            Self::TransportUnavailable => "the bound MCP transport could not be initialized",
            Self::CleanupUnavailable => "the bound MCP transport could not be closed exactly",
        }
    }
}

#[async_trait]
pub trait SessionMcpConnector: Send + Sync {
    /// Resolve exact config refs and secret refs, then establish the physical
    /// connections. This is never called by runtime construction.
    async fn connect(
        &self,
        bindings: &[SessionMcpBindingRef],
    ) -> Result<Vec<Arc<McpManager>>, SessionMcpConnectFailure>;
}

#[derive(Clone)]
pub struct LazyMcpRuntime(Arc<LazyMcpRuntimeInner>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LazyMcpActivationSnapshot {
    pub generation: u64,
    pub active: bool,
    pub closed: bool,
}

struct LazyMcpRuntimeInner {
    bindings: Arc<[SessionMcpBindingRef]>,
    connector: Arc<dyn SessionMcpConnector>,
    managers: tokio::sync::OnceCell<Arc<[Arc<McpManager>]>>,
    lifecycle: tokio::sync::Mutex<()>,
    closed: AtomicBool,
    cleanup_started: AtomicBool,
    activation: tokio::sync::watch::Sender<LazyMcpActivationSnapshot>,
}

impl LazyMcpRuntime {
    pub fn new(
        bindings: Vec<SessionMcpBindingRef>,
        connector: Arc<dyn SessionMcpConnector>,
    ) -> Result<Self, String> {
        if bindings.is_empty() || bindings.len() > MAX_BINDINGS {
            return Err(format!(
                "lazy MCP runtime requires between 1 and {MAX_BINDINGS} exact bindings"
            ));
        }
        let identities = bindings
            .iter()
            .map(|binding| {
                (
                    binding.resource_id.clone(),
                    binding.connection_config_ref.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        if identities.len() != bindings.len() {
            return Err("lazy MCP runtime contains a duplicate binding".to_owned());
        }
        let (activation, _) = tokio::sync::watch::channel(
            LazyMcpActivationSnapshot {
                generation: 0,
                active: false,
                closed: false,
            },
        );
        Ok(Self(Arc::new(LazyMcpRuntimeInner {
            bindings: Arc::from(bindings),
            connector,
            managers: tokio::sync::OnceCell::new(),
            lifecycle: tokio::sync::Mutex::new(()),
            closed: AtomicBool::new(false),
            cleanup_started: AtomicBool::new(false),
            activation,
        })))
    }

    pub fn binding_refs(&self) -> &[SessionMcpBindingRef] {
        &self.0.bindings
    }

    pub fn is_connected(&self) -> bool {
        self.0.managers.get().is_some() && !self.0.closed.load(Ordering::Acquire)
    }

    /// Real-time physical-runtime activation state. This is deliberately a
    /// separate, narrow handle from the canonical Kernel capability state:
    /// the host may call [`Self::activate`] only after the latter admits the
    /// deferred capability, then observe this generation for runtime cleanup
    /// and diagnostics without reaching into Session internals.
    pub fn activation_snapshot(&self) -> LazyMcpActivationSnapshot {
        *self.0.activation.borrow()
    }

    pub fn subscribe_activation(
        &self,
    ) -> tokio::sync::watch::Receiver<LazyMcpActivationSnapshot> {
        self.0.activation.subscribe()
    }

    /// Success is cached; failure is retryable. Concurrent activations remain
    /// single-flight under the Session lifecycle gate.
    pub async fn activate(&self) -> Result<Vec<String>, SessionMcpConnectFailure> {
        let _lifecycle = self.0.lifecycle.lock().await;
        if self.0.closed.load(Ordering::Acquire) {
            return Err(SessionMcpConnectFailure::TransportUnavailable);
        }
        let was_connected = self.0.managers.get().is_some();
        let managers = self
            .0
            .managers
            .get_or_try_init(|| async {
                let managers = tokio::time::timeout(
                    ACTIVATION_TIMEOUT,
                    self.0.connector.connect(&self.0.bindings),
                )
                .await
                .map_err(|_| SessionMcpConnectFailure::TransportUnavailable)??;
                validate_managers(managers)
            })
            .await?;
        if !was_connected {
            self.publish_activation(true, false);
        }
        Ok(server_names(managers))
    }

    pub fn connected_managers(
        &self,
    ) -> Result<Vec<Arc<McpManager>>, SessionMcpConnectFailure> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(SessionMcpConnectFailure::TransportUnavailable);
        }
        self.0
            .managers
            .get()
            .map(|managers| managers.iter().cloned().collect())
            .ok_or(SessionMcpConnectFailure::ResourceUnavailable)
    }

    pub fn exact_tool(
        &self,
        server: &str,
        tool: &str,
    ) -> Result<(Arc<McpManager>, McpToolDef), SessionMcpConnectFailure> {
        if !valid_selector(server) || !valid_selector(tool) {
            return Err(SessionMcpConnectFailure::ResourceUnavailable);
        }
        let mut matches = self
            .connected_managers()?
            .into_iter()
            .filter_map(|manager| {
                let tool = manager
                    .all_tools()
                    .into_iter()
                    .find(|(name, definition)| *name == server && definition.name == tool)
                    .map(|(_, definition)| definition.clone());
                tool.map(|tool| (manager, tool))
            });
        let found = matches
            .next()
            .ok_or(SessionMcpConnectFailure::ResourceUnavailable)?;
        if matches.next().is_some() {
            return Err(SessionMcpConnectFailure::ResourceUnavailable);
        }
        Ok(found)
    }

    pub async fn shutdown(&self) -> Result<(), SessionMcpConnectFailure> {
        let _lifecycle = self.0.lifecycle.lock().await;
        self.0.closed.store(true, Ordering::Release);
        self.publish_activation(false, true);
        if self.0.cleanup_started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        match self.0.managers.get() {
            Some(managers) => shutdown_managers(managers.iter().cloned().collect()).await,
            None => Ok(()),
        }
    }

    fn publish_activation(&self, active: bool, closed: bool) {
        let current = *self.0.activation.borrow();
        if current.active == active && current.closed == closed {
            return;
        }
        let _ = self.0.activation.send(LazyMcpActivationSnapshot {
            generation: current.generation.saturating_add(1),
            active,
            closed,
        });
    }
}

fn validate_managers(
    managers: Vec<Arc<McpManager>>,
) -> Result<Arc<[Arc<McpManager>]>, SessionMcpConnectFailure> {
    let names = server_names(&managers);
    if managers.is_empty() || names.is_empty() || names.iter().collect::<BTreeSet<_>>().len() != names.len() {
        return Err(SessionMcpConnectFailure::TransportUnavailable);
    }
    Ok(Arc::from(managers))
}

fn server_names(managers: &[Arc<McpManager>]) -> Vec<String> {
    let mut names = managers
        .iter()
        .flat_map(|manager| manager.server_names())
        .collect::<Vec<_>>();
    names.sort();
    names
}

async fn shutdown_managers(
    managers: Vec<Arc<McpManager>>,
) -> Result<(), SessionMcpConnectFailure> {
    tokio::time::timeout(CLEANUP_TIMEOUT, async move {
        for manager in managers {
            manager
                .shutdown()
                .await
                .map_err(|_| SessionMcpConnectFailure::CleanupUnavailable)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| SessionMcpConnectFailure::CleanupUnavailable)?
}

impl Drop for LazyMcpRuntimeInner {
    fn drop(&mut self) {
        if self.cleanup_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(managers) = self.managers.get() else {
            return;
        };
        let managers = managers.iter().cloned().collect();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    if let Err(error) = shutdown_managers(managers).await {
                        tracing::error!(code = error.code(), "lazy MCP cleanup was not proven");
                    }
                });
            }
            Err(_) => tracing::error!(
                "lazy MCP runtime dropped outside Tokio; transport Drop is the cleanup fallback"
            ),
        }
    }
}

fn valid_selector(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.len() <= MAX_SELECTOR_BYTES
        && !value.chars().any(char::is_control)
}

fn safe_error(error: SessionMcpConnectFailure) -> ToolResult {
    ToolResult::error(json!({"code": error.code(), "message": error.message()}).to_string())
}

#[derive(Clone)]
pub struct McpConnectTool(LazyMcpRuntime);

impl McpConnectTool {
    pub fn new(runtime: LazyMcpRuntime) -> Self {
        Self(runtime)
    }
}

#[async_trait]
impl Tool for McpConnectTool {
    fn name(&self) -> &str { MCP_CONNECT_TOOL_NAME }
    fn artifact_identity(&self) -> &str { "mcp.connect" }
    fn deferred_search_aliases(&self) -> Vec<String> {
        vec!["mcp.connect".to_owned(), "mcp".to_owned(), "connect mcp".to_owned()]
    }
    fn description(&self) -> &str {
        "Activate the exact MCP servers bound to this AgentSession. Call before MCP proxy or resource tools."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({"type": "object", "additionalProperties": false})
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }
    fn is_deferred(&self) -> bool { true }
    async fn execute(&self, input: Value) -> ToolResult {
        if input.as_object().is_none_or(|object| !object.is_empty()) {
            return ToolResult::error(json!({
                "code": "INVALID_PAYLOAD", "message": "mcp_connect accepts no arguments"
            }).to_string());
        }
        match self.0.activate().await {
            Ok(servers) => ToolResult::text(json!({
                "status": "connected", "server_count": servers.len(), "servers": servers
            }).to_string()),
            Err(error) => safe_error(error),
        }
    }
    fn category(&self) -> ToolCategory { ToolCategory::Exec }
    fn execution_timeout(&self, _input: &Value) -> Duration {
        ACTIVATION_TIMEOUT + Duration::from_secs(1)
    }
}

#[derive(Clone)]
pub struct GenericMcpToolProxy(LazyMcpRuntime);

impl GenericMcpToolProxy {
    pub fn new(runtime: LazyMcpRuntime) -> Self { Self(runtime) }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProxyInput {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: serde_json::Map<String, Value>,
}

#[async_trait]
impl Tool for GenericMcpToolProxy {
    fn name(&self) -> &str { MCP_GENERIC_PROXY_TOOL_NAME }
    fn artifact_identity(&self) -> &str { "mcp.tool_proxy" }
    fn deferred_search_aliases(&self) -> Vec<String> {
        vec![
            "mcp.tool_proxy".to_owned(),
            "mcp tool".to_owned(),
            "mcp proxy".to_owned(),
        ]
    }
    fn description(&self) -> &str {
        "Call one exact tool on an activated MCP server bound to this AgentSession."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "server": {"type": "string", "minLength": 1, "maxLength": MAX_SELECTOR_BYTES},
                "tool": {"type": "string", "minLength": 1, "maxLength": MAX_SELECTOR_BYTES},
                "arguments": {"type": "object"}
            },
            "required": ["server", "tool"],
            "additionalProperties": false
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }
    fn is_deferred(&self) -> bool { true }
    async fn execute(&self, input: Value) -> ToolResult {
        self.execute_with_context(input, &ToolExecutionContext::from_scoped_tool_call(
            "lazy-mcp-direct", "generic-proxy"
        )).await
    }
    async fn execute_with_context(
        &self,
        input: Value,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let input: ProxyInput = match serde_json::from_value(input) {
            Ok(input) => input,
            Err(_) => return ToolResult::error(json!({
                "code": "INVALID_PAYLOAD", "message": "server, tool and object arguments are required"
            }).to_string()),
        };
        if !valid_selector(&input.server) || !valid_selector(&input.tool) {
            return ToolResult::error(json!({
                "code": "INVALID_PAYLOAD", "message": "server and tool must be exact bounded names"
            }).to_string());
        }
        let (manager, definition) = match self.0.exact_tool(&input.server, &input.tool) {
            Ok(found) => found,
            Err(error) => return safe_error(error),
        };
        McpToolProxy::new(
            definition.name,
            input.server,
            definition.description.unwrap_or_default(),
            definition.input_schema,
            manager,
            false,
            definition.annotations,
        )
        .execute_with_context(Value::Object(input.arguments), context)
        .await
    }
    fn category(&self) -> ToolCategory { ToolCategory::Exec }
    fn execution_timeout(&self, _input: &Value) -> Duration { TOOL_TIMEOUT }
}

macro_rules! lazy_resource_tool {
    ($name:ident, $inner:ident, $tool_name:expr) => {
        #[derive(Clone)]
        pub struct $name(LazyMcpRuntime);
        impl $name {
            pub fn new(runtime: LazyMcpRuntime) -> Self { Self(runtime) }
        }
        #[async_trait]
        impl Tool for $name {
            fn name(&self) -> &str { $tool_name }
            fn artifact_identity(&self) -> &str { "mcp.resource" }
            fn deferred_search_aliases(&self) -> Vec<String> {
                vec![
                    "mcp.resource".to_owned(),
                    "mcp resources".to_owned(),
                    "mcp resource".to_owned(),
                ]
            }
            fn description(&self) -> &str {
                "Access resources from an activated MCP server bound to this AgentSession."
            }
            fn input_schema(&self) -> JsonSchema { $inner::new(Vec::new()).input_schema() }
            fn is_concurrency_safe(&self, _input: &Value) -> bool { true }
            fn is_deferred(&self) -> bool { true }
            async fn execute(&self, input: Value) -> ToolResult {
                match self.0.connected_managers() {
                    Ok(managers) => $inner::new(managers).execute(input).await,
                    Err(error) => safe_error(error),
                }
            }
            fn category(&self) -> ToolCategory { ToolCategory::Info }
        }
    };
}

lazy_resource_tool!(LazyMcpResourceListTool, McpResourceListTool, MCP_RESOURCE_LIST_TOOL_NAME);
lazy_resource_tool!(LazyMcpResourceReadTool, McpResourceReadTool, MCP_RESOURCE_READ_TOOL_NAME);

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    use nomi_mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
    use nomi_mcp::transport::{McpError, McpTransport};
    use super::*;

    struct Transport {
        responses: Mutex<VecDeque<Value>>,
        requests: Arc<Mutex<Vec<Value>>>,
        closes: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl McpTransport for Transport {
        async fn request(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
            self.requests.lock().unwrap().push(serde_json::to_value(request).unwrap());
            Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_owned(), id: request.id,
                result: Some(self.responses.lock().unwrap().pop_front().unwrap()), error: None,
            })
        }
        async fn notify(&self, _request: &JsonRpcRequest) -> Result<(), McpError> { Ok(()) }
        async fn close(&self) -> Result<(), McpError> {
            self.closes.fetch_add(1, Ordering::AcqRel); Ok(())
        }
    }

    struct Connector {
        connects: Arc<AtomicUsize>,
        manager: Mutex<Option<Arc<McpManager>>>,
        secret: String,
    }
    #[async_trait]
    impl SessionMcpConnector for Connector {
        async fn connect(&self, bindings: &[SessionMcpBindingRef])
            -> Result<Vec<Arc<McpManager>>, SessionMcpConnectFailure>
        {
            self.connects.fetch_add(1, Ordering::AcqRel);
            assert_eq!(bindings[0].resource_id(), "server-id");
            assert_eq!(bindings[0].connection_config_ref(), "mcp:server-id@7");
            assert!(!self.secret.is_empty());
            Ok(vec![self.manager.lock().unwrap().take().unwrap()])
        }
    }

    fn fixture() -> (LazyMcpRuntime, Arc<AtomicUsize>, Arc<Mutex<Vec<Value>>>, Arc<AtomicUsize>, String) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let closes = Arc::new(AtomicUsize::new(0));
        let manager = Arc::new(McpManager::new_for_test_with_tools(vec![(
            "bound", true, vec![McpToolDef {
                name: "lookup".to_owned(), description: Some("Lookup".to_owned()),
                input_schema: json!({"type":"object"}), annotations: None,
            }], Box::new(Transport {
                responses: Mutex::new(VecDeque::from([json!({
                    "content": [{"type":"text","text":"fixture result"}], "isError": false
                })])), requests: Arc::clone(&requests), closes: Arc::clone(&closes),
            })
        )]));
        let connects = Arc::new(AtomicUsize::new(0));
        let secret = "fixture-secret-never-serialized".to_owned();
        let runtime = LazyMcpRuntime::new(
            vec![SessionMcpBindingRef::new("server-id", "mcp:server-id@7").unwrap()],
            Arc::new(Connector {
                connects: Arc::clone(&connects), manager: Mutex::new(Some(manager)),
                secret: secret.clone(),
            }),
        ).unwrap();
        (runtime, connects, requests, closes, secret)
    }

    #[tokio::test]
    async fn zero_connection_before_explicit_activation_then_real_proxy_call() {
        let (runtime, connects, requests, _closes, secret) = fixture();
        let proxy = GenericMcpToolProxy::new(runtime.clone());
        let before = proxy.execute(json!({
            "server":"bound", "tool":"lookup", "arguments":{}
        })).await;
        assert!(before.is_error);
        assert_eq!(connects.load(Ordering::Acquire), 0);
        assert!(requests.lock().unwrap().is_empty());

        let receipt = McpConnectTool::new(runtime.clone()).execute(json!({})).await;
        assert!(!receipt.is_error, "{}", receipt.content);
        assert_eq!(connects.load(Ordering::Acquire), 1);
        assert!(!receipt.content.contains(&secret));
        assert!(!receipt.content.contains("connection_config_ref"));

        let after = proxy.execute(json!({
            "server":"bound", "tool":"lookup", "arguments":{}
        })).await;
        assert!(!after.is_error, "{}", after.content);
        assert!(after.content.contains("fixture result"));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn shutdown_is_idempotent_and_closes_connected_manager() {
        let (runtime, _, _, closes, _) = fixture();
        assert_eq!(runtime.activation_snapshot().generation, 0);
        let mut activation = runtime.subscribe_activation();
        runtime.activate().await.unwrap();
        activation.changed().await.unwrap();
        assert_eq!(
            *activation.borrow_and_update(),
            LazyMcpActivationSnapshot {
                generation: 1,
                active: true,
                closed: false,
            }
        );
        runtime.shutdown().await.unwrap();
        activation.changed().await.unwrap();
        runtime.shutdown().await.unwrap();
        assert_eq!(closes.load(Ordering::Acquire), 1);
        assert!(!runtime.is_connected());
        assert_eq!(
            runtime.activation_snapshot(),
            LazyMcpActivationSnapshot {
                generation: 2,
                active: false,
                closed: true,
            }
        );
    }

    #[test]
    fn schemas_never_expose_binding_session_or_secret_fields() {
        let (runtime, _, _, _, secret) = fixture();
        for schema in [
            McpConnectTool::new(runtime.clone()).input_schema(),
            GenericMcpToolProxy::new(runtime.clone()).input_schema(),
            LazyMcpResourceListTool::new(runtime.clone()).input_schema(),
            LazyMcpResourceReadTool::new(runtime).input_schema(),
        ] {
            let schema = schema.to_string();
            for forbidden in [&secret, "owner", "session", "credential", "connection_config_ref"] {
                assert!(!schema.contains(forbidden), "schema leaked {forbidden}");
            }
        }
    }

    #[test]
    fn all_lazy_mcp_tools_are_deferred_until_tool_search() {
        let (runtime, connects, _, _, _) = fixture();
        let mut registry = nomi_tools::registry::ToolRegistry::new();
        for tool in [
            Box::new(McpConnectTool::new(runtime.clone())) as Box<dyn Tool>,
            Box::new(GenericMcpToolProxy::new(runtime.clone())),
            Box::new(LazyMcpResourceListTool::new(runtime.clone())),
            Box::new(LazyMcpResourceReadTool::new(runtime)),
        ] {
            assert!(registry.register(tool));
        }
        let definitions = registry.to_tool_defs();
        assert_eq!(definitions.len(), 4);
        assert!(definitions.iter().all(|definition| definition.deferred));
        assert_eq!(connects.load(Ordering::Acquire), 0);
        assert_eq!(
            registry.get(MCP_CONNECT_TOOL_NAME).unwrap().artifact_identity(),
            "mcp.connect"
        );
        assert_eq!(
            registry.get(MCP_GENERIC_PROXY_TOOL_NAME).unwrap().artifact_identity(),
            "mcp.tool_proxy"
        );
        assert_eq!(
            registry.get(MCP_RESOURCE_LIST_TOOL_NAME).unwrap().artifact_identity(),
            "mcp.resource"
        );
    }
}
