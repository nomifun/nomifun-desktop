//! Immutable, host-verified reference data. These values carry no filesystem,
//! executable, activation or capability authority. Each engine owns how and
//! when it admits them into model context.
#[derive(Clone, Debug)]
pub struct EngineContextResource {
    pub label: String,
    pub provenance: String,
    pub content: EngineContextContent,
}

#[derive(Clone, Debug)]
pub enum EngineContextContent {
    Text {
        text: String,
    },
    /// Must be decoded, bounded and re-encoded by the authenticated host.
    Image {
        media_type: String,
        data_base64: String,
    },
}

/// Dynamic resource reads are not immutable Skill data. The platform owns
/// connection authority, task lifetime, receipts and bounded projections.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineResourceQuery {
    ListMcpResources,
    ReadMcpResource {
        uri: String,
    },
    ListMcpResourceTemplates,
    /// Exact advertised template; bounded string/list/map values are expanded by the platform,
    /// never treated as a URL that the Engine may fetch independently.
    ReadMcpResourceTemplate {
        uri_template: String,
        /// Strings, arrays of strings, or objects with string values only.
        /// The platform rejects null/numbers/nesting before owner effects.
        variables: std::collections::BTreeMap<String, serde_json::Value>,
    },
}

/// Shared model-facing syntax, not authorization. Owner preflight additionally
/// enforces aggregate UTF-8/component and expanded URI budgets.
pub fn mcp_template_variables_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","maxProperties":64,
    "propertyNames":{"minLength":1,"maxLength":256},"additionalProperties":{"oneOf":[
        {"type":"string","maxLength":4096},
        {"type":"array","maxItems":256,"items":{"type":"string","maxLength":4096}},
        {"type":"object","maxProperties":128,"propertyNames":{"maxLength":4096},
            "additionalProperties":{"type":"string","maxLength":4096}}
    ]}})
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineResourceRead {
    /// Exact product server ID from the frozen resource bindings, never a URL
    /// or connection config. Omission is valid only for a single bound server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    pub query: EngineResourceQuery,
    #[serde(default)]
    pub offset: usize,
    pub limit: usize,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

/// Explicit image observation. The digest is of the original decoded blob,
/// obtained from a normal resource page, NOT the page or prepared pixel digest.
/// Each call reobserves the resource; content_index is zero-based and must still
/// match that digest. Only ReadMcpResource/ReadMcpResourceTemplate are accepted.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineResourceImageRead {
    pub server_id: Option<String>,
    pub query: EngineResourceQuery,
    pub content_index: usize,
    pub expected_source_sha256: String,
}

#[async_trait::async_trait]
pub trait EngineResourcePort: Send + Sync + std::fmt::Debug {
    /// Each read reobserves the server. Follow next_offset with the previous
    /// sha256, rejecting changed content instead of combining two versions.
    /// Binary contents are host-generated descriptors (index, source byte count
    /// and original decoded SHA-256), not base64 model text. The page digest
    /// covers the projected representation; omitted extension metadata is not
    /// a complete remote snapshot. Images require the separate explicit port.
    /// Production requires a selected namespaced MCP ResourceProvider
    /// contribution and an explicitly selected bound server with connect/read.
    /// This is resource-read authority, never broad tool-call authority.
    /// Dropping a read future abandons its result, not the platform-owned task.
    /// Engines must settle owned reads before the next model call or cleanup;
    /// uncertain owner outcomes must never be retried automatically.
    /// A returned page has a platform-generated `is_error` boolean and optional
    /// `failure` metadata, repeated on every page independently of fragments.
    /// is_error=true is a known rejection after protocol cleanup, not resource
    /// data, proof of no effects, rollback, or automatic retry authorization.
    /// Err covers failed admission/projection or unproven owner completion;
    /// Engines must not infer that every Err is a harmless remote rejection.
    async fn read(
        &self,
        causality: &nomifun_chat_model_broker::ChatCausality,
        generation: u64,
        call_id: &str,
        request: EngineResourceRead,
    ) -> Result<serde_json::Value, crate::EngineToolError>;

    /// Optional explicit media port; never fall back to text/base64 on refusal.
    /// Production additionally requires active llm.vision and primary ImageInput
    /// before remote IO and before returning host-decoded/re-encoded pixels.
    /// The same retained task/receipt/settlement rules as read apply. Known
    /// remote rejections return is_error=true with no image. No client URL fetch.
    async fn read_image(
        &self,
        _causality: &nomifun_chat_model_broker::ChatCausality,
        _generation: u64,
        _call_id: &str,
        _request: EngineResourceImageRead,
    ) -> Result<crate::EngineToolResult, crate::EngineToolError> {
        Err(crate::EngineToolError::ToolInvocation(
            "This host does not support explicit MCP resource images".into(),
        ))
    }
}
