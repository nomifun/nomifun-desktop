//! Resource protocol uses a read grant, not a synthesized tools/call mapping.
//! URI strings are sent only to the exact server; never opened as local paths
//! or fetched as arbitrary URLs by the client. Outputs remain untrusted data.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

const MAX_RESOURCES: usize = 256;
const MAX_RESOURCE_BYTES: usize = 1024 * 1024;
const MAX_URI_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpResourceOperation {
    List,
    Read {
        uri: String,
    },
    ListTemplates,
    ReadTemplate {
        uri_template: String,
        variables: std::collections::BTreeMap<String, serde_json::Value>,
    },
}

impl McpResourceOperation {
    /// Local input admission, without connection or credential effects.
    pub fn validate(&self) -> Result<(), McpOwnerError> {
        match self {
            Self::Read { uri } => validate_uri(uri),
            Self::ReadTemplate {
                uri_template,
                variables,
            } => resource_template::expand(uri_template, variables).map(|_| ()),
            Self::List | Self::ListTemplates => Ok(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct McpResourceRequest {
    pub principal_kind: String,
    pub principal_id: String,
    pub operation_id: String,
    pub server: McpServerBinding,
    pub operation: McpResourceOperation,
}

#[derive(Clone, Debug, Serialize)]
pub struct McpResourceResult {
    pub server_id: String,
    pub operation: McpResourceOperation,
    /// An explicitly observed rejection, only published after cleanup succeeds.
    /// This is not evidence of rollback, no effects or safe automatic replay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<McpResourceFailure>,
    /// A bounded resources list or resource contents, NOT prepared model media.
    pub result: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpResourceFailure {
    ResourcesUnavailable,
    ResourceNotListed,
    TemplateNotListed,
    /// Only a correlated, structurally valid response supplies this code.
    /// Remote error messages/data are not copied into platform diagnostics.
    RpcRejected {
        method: String,
        code: i64,
    },
}

enum Observed<T> {
    Available(T),
    Rejected(McpResourceFailure),
}

impl McpResourceRequest {
    /// Local checks only. Hosts use this BEFORE reserving durable authority.
    pub fn validate(&self) -> Result<(), McpOwnerError> {
        validate_connection_authority(
            &self.principal_kind,
            &self.principal_id,
            &self.operation_id,
            &self.server,
            MCP_READ_OPERATION,
        )?;
        self.operation.validate()
    }
}

impl McpOwner {
    /// One bounded, explicitly authorized resource transaction. Hosts must
    /// retain its task and reserve effect evidence before calling: initialize
    /// or starting a configured stdio server is not guaranteed side-effect free.
    pub async fn resource(
        &self,
        request: McpResourceRequest,
    ) -> Result<McpResourceResult, McpOwnerError> {
        request.validate()?;
        let deadline = Instant::now() + self.timeout;
        let mut session = match &request.server.transport {
            McpServerTransport::Stdio { command, args, env } => McpSession::from_stdio(
                stdio::StdioTransport::launch(command, args, env, deadline).await?,
            ),
            McpServerTransport::Http { url, headers }
            | McpServerTransport::Sse { url, headers } => {
                validate_http_endpoint(url)?;
                let client = self.http_client.as_ref().map_err(Clone::clone)?.clone();
                let credential = timeout_at(
                    deadline,
                    self.credentials.resolve(McpCredentialLookup {
                        server_id: request.server.server_id.clone(),
                        resource_id: request.server.resource_id.clone(),
                        connection_config_ref: request.server.connection_config_ref.clone(),
                        endpoint: url.clone(),
                    }),
                )
                .await
                .map_err(|_| owner_timeout_error(self.timeout))??;
                let mut session = McpSession::new(
                    client,
                    url.clone(),
                    build_headers(headers, credential.as_ref())?,
                );
                session.legacy_mode =
                    matches!(&request.server.transport, McpServerTransport::Sse { .. });
                session
            }
        };
        let local_started = session.stdio.is_some();
        let outcome = timeout_at(deadline, async {
            session.initialize().await?;
            let (result, failure) = match session.observe_resource(&request).await? {
                Observed::Available(value) => (value, None),
                Observed::Rejected(failure) => (Value::Null, Some(failure)),
            };
            Ok(McpResourceResult {
                server_id: request.server.server_id.clone(),
                operation: request.operation.clone(),
                result,
                failure,
            })
        })
        .await
        .unwrap_or_else(|_| Err(owner_timeout_error(self.timeout)));
        // Independent cleanup budget, even after timeout/protocol rejection.
        let budget = if session.stdio.is_some() {
            stdio::CLEANUP_TIMEOUT
        } else {
            SESSION_CLEANUP_TIMEOUT
        };
        let cleanup = timeout(budget, session.close()).await.unwrap_or_else(|_| {
            Err(McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                "MCP resource session cleanup timed out",
            ))
        });
        match (outcome, cleanup) {
            (Ok(result), Ok(())) => Ok(result),
            (_, Err(_)) => Err(McpOwnerError::new(
                "MCP_SESSION_CLEANUP_FAILED",
                "MCP resource session cleanup is unproven",
            )),
            (Err(error), Ok(())) if local_started => Err(McpOwnerError::new(
                "MCP_OUTCOME_UNKNOWN",
                format!(
                    "MCP local server may have applied effects; do not replay ({})",
                    error.code()
                ),
            )),
            (Err(error), Ok(())) => Err(error),
        }
    }
}

impl McpSession {
    async fn observe_resource(
        &mut self,
        request: &McpResourceRequest,
    ) -> Result<Observed<Value>, McpOwnerError> {
        if !self.resources_supported {
            return Ok(Observed::Rejected(McpResourceFailure::ResourcesUnavailable));
        }
        // Recheck membership in the same protocol session as the read.
        // A previous catalog is observation, not a frozen content grant.
        let templates = matches!(
            &request.operation,
            McpResourceOperation::ListTemplates | McpResourceOperation::ReadTemplate { .. }
        );
        let (next_id, resources) = match self.resource_catalog(templates).await? {
            Observed::Available(catalog) => catalog,
            Observed::Rejected(failure) => return Ok(Observed::Rejected(failure)),
        };
        match &request.operation {
            McpResourceOperation::List => Ok(Observed::Available(json!({"resources":resources}))),
            McpResourceOperation::ListTemplates => {
                Ok(Observed::Available(json!({"resourceTemplates":resources,
                "expansion_profile":"RFC6570 scalar strings; absent variables omitted; unknown variables rejected; prefix modifiers require decoded input without percent signs; composite values unsupported"})))
            }
            McpResourceOperation::Read { uri } => {
                if !resources
                    .iter()
                    .any(|item| item.get("uri").and_then(Value::as_str) == Some(uri))
                {
                    return Ok(Observed::Rejected(McpResourceFailure::ResourceNotListed));
                }
                self.read_resource(next_id, uri, &request.operation_id)
                    .await
            }
            McpResourceOperation::ReadTemplate {
                uri_template,
                variables,
            } => {
                if !resources.iter().any(|item| {
                    item.get("uriTemplate").and_then(Value::as_str) == Some(uri_template)
                }) {
                    return Ok(Observed::Rejected(McpResourceFailure::TemplateNotListed));
                }
                let uri = resource_template::expand(uri_template, variables)?;
                self.read_resource(next_id, &uri, &request.operation_id)
                    .await
            }
        }
    }

    async fn read_resource(
        &mut self,
        id: u64,
        uri: &str,
        operation: &str,
    ) -> Result<Observed<Value>, McpOwnerError> {
        let response = self
            .request(JsonRpcRequest {
                jsonrpc: "2.0",
                id: Some(id),
                method: "resources/read",
                params: Some(
                    json!({"uri":uri,"_meta":{(MCP_EXECUTION_OPERATION_META_KEY):operation}}),
                ),
            })
            .await?;
        if let Some(failure) = observed_rejection("resources/read", &response, id)? {
            return Ok(Observed::Rejected(failure));
        }
        let result = response
            .result
            .ok_or_else(|| McpOwnerError::protocol_failed("resources/read has no result"))?;
        validate_contents(&result, uri)?;
        Ok(Observed::Available(result))
    }

    async fn resource_catalog(
        &mut self,
        templates: bool,
    ) -> Result<Observed<(u64, Vec<Value>)>, McpOwnerError> {
        let (method, list_key, uri_key) = if templates {
            (
                "resources/templates/list",
                "resourceTemplates",
                "uriTemplate",
            )
        } else {
            ("resources/list", "resources", "uri")
        };
        let mut resources = Vec::new();
        let mut uris = BTreeSet::new();
        let mut cursors = BTreeSet::new();
        let mut cursor: Option<String> = None;
        let mut bytes = 0usize;
        for page in 0..MAX_CATALOG_PAGES {
            let id = page + 2;
            let response = self
                .request(JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: Some(id),
                    method,
                    params: cursor.as_ref().map(|cursor| json!({"cursor":cursor})),
                })
                .await?;
            if let Some(failure) = observed_rejection(method, &response, id)? {
                return Ok(Observed::Rejected(failure));
            }
            let result = response
                .result
                .ok_or_else(|| McpOwnerError::protocol_failed("resource catalog has no result"))?;
            bytes = bytes.saturating_add(encoded_size(&result)?);
            if bytes > MAX_RESOURCE_BYTES {
                return Err(McpOwnerError::protocol_failed(
                    "resource catalog exceeds its aggregate byte limit",
                ));
            }
            let entries = result
                .get(list_key)
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    McpOwnerError::protocol_failed("resource catalog has no expected array")
                })?;
            for entry in entries {
                let uri = entry
                    .get(uri_key)
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpOwnerError::protocol_failed("resource has no URI"))?;
                // Unsupported template grammar stays visible as unavailable;
                // never broaden a literal read into arbitrary URI membership.
                if templates {
                    if uri.is_empty()
                        || uri.len() > MAX_URI_BYTES
                        || uri.chars().any(char::is_control)
                    {
                        return Err(McpOwnerError::protocol_failed(
                            "resource template is malformed or oversized",
                        ));
                    }
                } else {
                    validate_uri(uri)?;
                }
                if uris.len() >= MAX_RESOURCES || !uris.insert(uri.to_owned()) {
                    return Err(McpOwnerError::protocol_failed(
                        "resource catalog is oversized or contains duplicate URIs",
                    ));
                }
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty() && name.len() <= 1024)
                    .ok_or_else(|| {
                        McpOwnerError::protocol_failed("resource has no bounded name")
                    })?;
                // Project only data fields with explicit bounds. Server metadata
                // cannot introduce protocol/client behavior or a new URI grant.
                let mut record = json!({(uri_key):uri,"name":name});
                if templates {
                    match resource_template::variables(uri) {
                        Ok(names) => {
                            record["scalar_expansion_supported"] = json!(true);
                            record["composite_expansion_supported"] = json!(true);
                            record["expansion_value_types"] =
                                json!(["string", "string_list", "string_map"]);
                            record["variables"] = json!(names);
                        }
                        Err(_) => {
                            record["scalar_expansion_supported"] = json!(false);
                            record["composite_expansion_supported"] = json!(false);
                        }
                    }
                }
                for (key, limit) in [("description", 4096), ("mimeType", 256), ("title", 1024)] {
                    if let Some(value) = entry.get(key) {
                        let value = value
                            .as_str()
                            .filter(|value| value.len() <= limit)
                            .ok_or_else(|| {
                                McpOwnerError::protocol_failed(
                                    "resource metadata is malformed or oversized",
                                )
                            })?;
                        record[key] = json!(value);
                    }
                }
                resources.push(record);
            }
            match result.get("nextCursor") {
                None | Some(Value::Null) => return Ok(Observed::Available((id + 1, resources))),
                Some(Value::String(next))
                    if !next.is_empty()
                        && next.len() <= MAX_CURSOR_BYTES
                        && cursors.insert(next.clone()) =>
                {
                    cursor = Some(next.clone())
                }
                _ => {
                    return Err(McpOwnerError::protocol_failed(
                        "resource catalog cursor is malformed or repeated",
                    ));
                }
            }
        }
        Err(McpOwnerError::protocol_failed(
            "resource catalog exceeds its page limit",
        ))
    }
}

fn observed_rejection(
    method: &str,
    response: &JsonRpcResponse,
    id: u64,
) -> Result<Option<McpResourceFailure>, McpOwnerError> {
    // Do not classify by a general error code or error message. Only this
    // validated response, received after initialize, is a returned rejection.
    ensure_response_id(response, id)?;
    if response.jsonrpc != "2.0" || response.result.is_some() == response.error.is_some() {
        return Err(McpOwnerError::protocol_failed(
            "Resource response is not an exclusive JSON-RPC result or error",
        ));
    }
    Ok(response
        .error
        .as_ref()
        .map(|error| McpResourceFailure::RpcRejected {
            method: method.to_owned(),
            code: error.code,
        }))
}

pub(super) fn validate_uri(uri: &str) -> Result<(), McpOwnerError> {
    if uri.is_empty()
        || uri.len() > MAX_URI_BYTES
        || uri.chars().any(char::is_control)
        || reqwest::Url::parse(uri).is_err()
    {
        return Err(McpOwnerError::new(
            "MCP_RESOURCE_URI_INVALID",
            "Resource URI must be a bounded absolute URI",
        ));
    }
    Ok(())
}

fn encoded_size(value: &Value) -> Result<usize, McpOwnerError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| McpOwnerError::protocol_failed("resource result cannot be serialized"))
}

fn validate_contents(result: &Value, uri: &str) -> Result<(), McpOwnerError> {
    if encoded_size(result)? > MAX_RESOURCE_BYTES {
        return Err(McpOwnerError::protocol_failed(
            "resource contents exceed the byte limit",
        ));
    }
    let contents = result
        .get("contents")
        .and_then(Value::as_array)
        .filter(|contents| !contents.is_empty() && contents.len() <= 64)
        .ok_or_else(|| {
            McpOwnerError::protocol_failed("resource result has no bounded contents array")
        })?;
    let mut binary_bytes = 0usize;
    for content in contents {
        if content.get("uri").and_then(Value::as_str) != Some(uri) {
            return Err(McpOwnerError::protocol_failed(
                "resource contents have a different URI",
            ));
        }
        if content.get("mimeType").is_some_and(|value| {
            value.as_str().is_none_or(|value| {
                value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
            })
        }) {
            return Err(McpOwnerError::protocol_failed(
                "resource content media type is malformed",
            ));
        }
        // Binary is protocol data, never implicit model text or image input.
        // The host separately projects descriptors or prepares authorized pixels.
        match (content.get("text"), content.get("blob")) {
            (Some(Value::String(_)), None) => {}
            (None, Some(Value::String(blob))) => {
                let bytes = STANDARD.decode(blob).map_err(|_| {
                    McpOwnerError::protocol_failed("resource blob is not canonical padded base64")
                })?;
                binary_bytes = binary_bytes.saturating_add(bytes.len());
                if binary_bytes > 512 * 1024 {
                    return Err(McpOwnerError::protocol_failed(
                        "resource decoded blobs exceed the aggregate byte limit",
                    ));
                }
            }
            _ => {
                return Err(McpOwnerError::protocol_failed(
                    "resource content requires exactly one text string or base64 blob",
                ));
            }
        }
    }
    Ok(())
}
