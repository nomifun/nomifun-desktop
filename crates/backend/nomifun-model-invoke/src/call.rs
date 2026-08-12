//! Fully resolved task and call values.
//!
//! Catalog transport configuration is task-scoped.  The resolver reads one
//! `(provider, model, task)` capability row and produces [`ResolvedTaskConfig`];
//! one-shot and realtime executors consume that same value.  There is no
//! platform route, model-level protocol, or params compatibility layer here.

use crate::auth::AuthMaterial;
use crate::error::InvokeError;
use crate::manifest::expand_protocol_endpoint_template;
use crate::types::TaskRequest;
use nomifun_api_types::ProtocolTransportKind;

/// The only URL families allowed to receive a resolved connection's
/// credentials. Batch adapters use HTTP(S); persistent realtime adapters use
/// WebSocket(S).
#[derive(Clone, Copy)]
pub(crate) enum CredentialedUrlKind {
    Http,
    WebSocket,
}

fn scheme_allowed(kind: CredentialedUrlKind, scheme: &str) -> bool {
    match kind {
        CredentialedUrlKind::Http => matches!(scheme, "http" | "https"),
        CredentialedUrlKind::WebSocket => matches!(scheme, "ws" | "wss"),
    }
}

fn credential_scheme(scheme: &str) -> &str {
    match scheme {
        "ws" => "http",
        "wss" => "https",
        other => other,
    }
}

fn same_credential_origin(left: &reqwest::Url, right: &reqwest::Url) -> bool {
    credential_scheme(left.scheme()) == credential_scheme(right.scheme())
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn validate_absolute_url(
    parsed: &reqwest::Url,
    kind: CredentialedUrlKind,
    field: &str,
) -> Result<(), InvokeError> {
    if !scheme_allowed(kind, parsed.scheme())
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        let family = match kind {
            CredentialedUrlKind::Http => "HTTP(S)",
            CredentialedUrlKind::WebSocket => "WS(S)",
        };
        return Err(InvokeError::config(format!(
            "capability field '{field}' must be a {family} URL with a host, no userinfo and no fragment"
        )));
    }
    Ok(())
}

/// Validate a URL which will receive the selected connection's credentials.
/// The raw value is deliberately never copied into an error message.
///
/// Relative paths are accepted only when `allow_relative` is true and must
/// remain relative to the selected origin. Absolute cross-origin targets
/// require the capability's explicit credential-forwarding acknowledgement.
pub(crate) fn validate_credentialed_url(
    connection: &ResolvedConnection,
    allow_cross_origin_credentials: bool,
    raw: &str,
    field: &str,
    kind: CredentialedUrlKind,
    allow_relative: bool,
) -> Result<(), InvokeError> {
    validate_credentialed_target_with_kind(
        &connection.base_url,
        allow_cross_origin_credentials,
        raw,
        field,
        kind,
        allow_relative,
    )
}

fn validate_credentialed_target_with_kind(
    connection_base_url: &str,
    allow_cross_origin_credentials: bool,
    raw: &str,
    field: &str,
    kind: CredentialedUrlKind,
    allow_relative: bool,
) -> Result<(), InvokeError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(InvokeError::config(format!(
            "capability field '{field}' must not be empty"
        )));
    }

    let selected = reqwest::Url::parse(connection_base_url.trim()).map_err(|_| {
        InvokeError::config("selected connection base URL is not a valid absolute URL")
    })?;
    if selected.host_str().is_none()
        || !selected.username().is_empty()
        || selected.password().is_some()
        || selected.query().is_some()
        || selected.fragment().is_some()
    {
        return Err(InvokeError::config(
            "selected connection base URL must have a host and no userinfo, query or fragment",
        ));
    }

    match reqwest::Url::parse(raw) {
        Ok(target) => {
            validate_absolute_url(&target, kind, field)?;
            if !same_credential_origin(&selected, &target)
                && !allow_cross_origin_credentials
            {
                return Err(InvokeError::config(format!(
                    "capability field '{field}' would send connection credentials to a different origin; enable allow_cross_origin_credentials to confirm"
                )));
            }
        }
        Err(_) if allow_relative => {
            // Resolve against a fixed sentinel solely to reject scheme-relative
            // paths, malformed URI schemes, userinfo and fragments.
            let sentinel = reqwest::Url::parse("https://relative.invalid/")
                .expect("static validation URL is valid");
            let joined = sentinel.join(raw).map_err(|_| {
                InvokeError::config(format!(
                    "capability field '{field}' must be a relative path or valid absolute URL"
                ))
            })?;
            if joined.origin() != sentinel.origin()
                || joined.fragment().is_some()
                || !joined.username().is_empty()
                || joined.password().is_some()
            {
                return Err(InvokeError::config(format!(
                    "capability field '{field}' must be a same-origin relative path with no fragment"
                )));
            }
        }
        Err(_) => {
            return Err(InvokeError::config(format!(
                "capability field '{field}' must be a valid absolute URL"
            )));
        }
    }
    Ok(())
}

/// Save/runtime shared authority for credential-bearing endpoint origins.
///
/// HTTP protocols accept HTTP(S) roots and targets. WebSocket protocols accept
/// HTTP(S) or WS(S), treating `http`/`ws` and `https`/`wss` as the same
/// credential origin because realtime adapters canonicalize the handshake
/// scheme after validation. Relative targets remain on the selected root;
/// absolute cross-origin targets require an explicit acknowledgement.
pub fn validate_credentialed_target_url(
    connection_base_url: &str,
    allow_cross_origin_credentials: bool,
    raw_target: &str,
    field: &str,
    transport: ProtocolTransportKind,
    allow_relative: bool,
) -> Result<(), InvokeError> {
    let selected = reqwest::Url::parse(connection_base_url.trim()).map_err(|_| {
        InvokeError::config("selected connection base URL is not a valid absolute URL")
    })?;
    let selected_scheme_allowed = match transport {
        ProtocolTransportKind::Http => matches!(selected.scheme(), "http" | "https"),
        ProtocolTransportKind::Websocket => {
            matches!(selected.scheme(), "http" | "https" | "ws" | "wss")
        }
        ProtocolTransportKind::Sdk => false,
    };
    if !selected_scheme_allowed {
        return Err(InvokeError::config(
            "selected connection base URL scheme is incompatible with the protocol transport",
        ));
    }

    let kind = match transport {
        ProtocolTransportKind::Http => CredentialedUrlKind::Http,
        ProtocolTransportKind::Websocket => match reqwest::Url::parse(raw_target.trim())
            .ok()
            .map(|url| url.scheme().to_owned())
            .as_deref()
        {
            Some("http" | "https") => CredentialedUrlKind::Http,
            _ => CredentialedUrlKind::WebSocket,
        },
        ProtocolTransportKind::Sdk => {
            return Err(InvokeError::config(
                "SDK protocols do not accept credential-bearing URL targets",
            ));
        }
    };
    validate_credentialed_target_with_kind(
        connection_base_url,
        allow_cross_origin_credentials,
        raw_target,
        field,
        kind,
        allow_relative,
    )
}

/// The connection profile a call rides on (the provider's explicit
/// `"default"` profile or a named role), with decrypted auth material.
/// Deliberately not `Debug`: `auth` holds live credentials.
#[derive(Clone)]
pub struct ResolvedConnection {
    pub role: String,
    /// An absolute connection root. It is never interpreted as a complete
    /// request endpoint; complete URLs belong to capability endpoint fields.
    pub base_url: String,
    pub auth: AuthMaterial,
    /// Connection-level provider configuration, not task transport metadata.
    pub extra: serde_json::Value,
}

/// Typed transport fields owned by exactly one capability row.
#[derive(Clone, Default)]
pub struct ResolvedTaskTransport {
    pub endpoint: Option<String>,
    pub poll_endpoint: Option<String>,
    pub content_endpoint: Option<String>,
    pub realtime_endpoint: Option<String>,
    pub allow_cross_origin_credentials: bool,
}

/// Canonical task-scoped runtime configuration shared by Chat, one-shot
/// multimodal calls, health probes and realtime sessions.
#[derive(Clone)]
pub struct ResolvedTaskConfig {
    pub provider_id: String,
    /// Monotonic revision of the provider's complete invocation graph.
    pub config_revision: i64,
    /// Provider identity is retained for observability and provider-native
    /// request defaults only; it never selects protocol or URL transport.
    pub platform: String,
    pub model: String,
    pub task: nomifun_api_types::ModelTask,
    /// Exact refinements declared on this task capability. Callers must not
    /// infer input support from a model name or provider family.
    pub traits: Vec<nomifun_api_types::ModelTrait>,
    pub protocol: String,
    pub connection: ResolvedConnection,
    pub transport: ResolvedTaskTransport,
    /// Open-ended provider request parameters. Local transport/auth keys are
    /// rejected before this object is constructed.
    pub provider_params: serde_json::Value,
    pub context_limit: Option<i64>,
    /// Non-secret Bedrock SDK metadata (auth method, region and optional
    /// profile). Access-key material lives only in `connection.auth`.
    pub bedrock_config: Option<String>,
}

impl ResolvedTaskConfig {
    /// Build the adapter-facing parameter object. This is an ephemeral view,
    /// not a second persistence schema: every transport value originates from
    /// the typed capability row and provider parameters remain disjoint.
    pub fn execution_params(&self) -> serde_json::Value {
        let mut fields = self.provider_params.as_object().cloned().unwrap_or_default();
        if let Some(endpoint) = self.transport.endpoint.as_deref() {
            fields.insert("endpoint".into(), serde_json::Value::String(endpoint.to_owned()));
        }
        for (name, value) in [
            ("poll_endpoint", self.transport.poll_endpoint.as_deref()),
            ("content_endpoint", self.transport.content_endpoint.as_deref()),
            ("realtime_endpoint", self.transport.realtime_endpoint.as_deref()),
        ] {
            if let Some(value) = value {
                fields.insert(name.into(), serde_json::Value::String(value.to_owned()));
            }
        }
        if self.transport.allow_cross_origin_credentials {
            fields.insert(
                "allow_cross_origin_credentials".into(),
                serde_json::Value::Bool(true),
            );
        }
        serde_json::Value::Object(fields)
    }

    /// Resolve an HTTP submit endpoint from the connection root and the
    /// capability/descriptor endpoint. This is also the exact endpoint passed
    /// to the Chat agent serializer.
    pub fn http_endpoint(&self) -> Result<String, InvokeError> {
        let endpoint = self
            .transport
            .endpoint
            .as_deref()
            .ok_or_else(|| {
                InvokeError::config(format!(
                    "protocol {:?} has no submit endpoint for task {:?}",
                    self.protocol, self.task
                ))
            })?;
        let endpoint = expand_protocol_endpoint_template(
            &self.protocol,
            self.task,
            "endpoint",
            endpoint,
            &self.model,
        )?;
        validate_credentialed_url(
            &self.connection,
            self.transport.allow_cross_origin_credentials,
            &endpoint,
            "endpoint",
            CredentialedUrlKind::Http,
            true,
        )?;
        Ok(resolve_endpoint(&self.connection.base_url, &endpoint))
    }
}

/// Append a relative endpoint to a configured root while preserving version
/// prefixes such as `/v1`, `/v2` or `/api/v4`. Absolute endpoints win exactly.
pub(crate) fn resolve_endpoint(base_url: &str, endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    if reqwest::Url::parse(endpoint).is_ok() {
        endpoint.to_owned()
    } else {
        format!(
            "{}/{}",
            base_url.trim().trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        )
    }
}

/// One task invocation, fully resolved against one capability and connection.
#[derive(Clone)]
pub struct ResolvedCall {
    pub provider_id: String,
    pub config_revision: i64,
    pub platform: String,
    pub model: String,
    pub task: nomifun_api_types::ModelTask,
    pub protocol: String,
    pub connection: ResolvedConnection,
    /// Ephemeral adapter view built by [`ResolvedTaskConfig::execution_params`].
    pub model_params: serde_json::Value,
    pub request: TaskRequest,
}

impl ResolvedCall {
    /// Exact HTTP submit endpoint injected by the task-capability resolver.
    /// Provider parameters cannot supply this reserved field, so adapters have
    /// no URL convention or second transport source.
    pub fn endpoint_url(&self) -> Result<String, InvokeError> {
        let endpoint = self
            .model_params
            .get("endpoint")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                InvokeError::config(format!(
                    "resolved protocol {:?} has no injected submit endpoint",
                    self.protocol
                ))
            })?;
        let endpoint = expand_protocol_endpoint_template(
            &self.protocol,
            self.task,
            "endpoint",
            endpoint,
            &self.model,
        )?;
        validate_credentialed_url(
            &self.connection,
            self.model_params
                .get("allow_cross_origin_credentials")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            &endpoint,
            "endpoint",
            CredentialedUrlKind::Http,
            true,
        )?;
        Ok(resolve_endpoint(&self.connection.base_url, &endpoint))
    }

    /// Revalidate an adapter-produced or persisted polling URL immediately
    /// before attaching this call's credentials.
    pub(crate) fn credentialed_http_url(
        &self,
        raw: &str,
        field: &str,
    ) -> Result<String, InvokeError> {
        validate_credentialed_url(
            &self.connection,
            self.model_params
                .get("allow_cross_origin_credentials")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            raw,
            field,
            CredentialedUrlKind::Http,
            false,
        )?;
        Ok(raw.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use nomifun_api_types::ModelTask;
    use serde_json::json;

    use super::*;
    use crate::auth::AuthScheme;

    fn connection(base_url: &str) -> ResolvedConnection {
        ResolvedConnection {
            role: "default".into(),
            base_url: base_url.into(),
            auth: AuthMaterial {
                scheme: AuthScheme::Bearer,
                credentials: json!({"api_keys": ["sk"]}),
            },
            extra: json!({}),
        }
    }

    #[test]
    fn relative_endpoint_preserves_configured_version_root() {
        assert_eq!(
            resolve_endpoint("https://api.example/v2/", "/chat/completions"),
            "https://api.example/v2/chat/completions"
        );
    }

    #[test]
    fn task_config_uses_only_typed_transport_fields() {
        let config = ResolvedTaskConfig {
            provider_id: "p".into(),
            config_revision: 3,
            platform: "custom".into(),
            model: "m".into(),
            task: ModelTask::Chat,
            traits: vec![],
            protocol: "openai.chat_text".into(),
            connection: connection("https://api.example/v1"),
            transport: ResolvedTaskTransport {
                endpoint: Some("/custom/chat".into()),
                allow_cross_origin_credentials: true,
                ..Default::default()
            },
            provider_params: json!({"temperature": 0.2}),
            context_limit: Some(32_000),
            bedrock_config: None,
        };
        let params = config.execution_params();
        assert_eq!(params["temperature"], 0.2);
        assert_eq!(params["endpoint"], "/custom/chat");
        assert_eq!(params["allow_cross_origin_credentials"], true);
        assert_eq!(
            config.http_endpoint().unwrap(),
            "https://api.example/v1/custom/chat"
        );
    }

    #[test]
    fn cross_origin_endpoint_requires_explicit_acknowledgement() {
        let config = ResolvedTaskConfig {
            provider_id: "p".into(),
            config_revision: 3,
            platform: "custom".into(),
            model: "m".into(),
            task: ModelTask::Chat,
            traits: vec![],
            protocol: "openai.chat_text".into(),
            connection: connection("https://api.example/v1"),
            transport: ResolvedTaskTransport {
                endpoint: Some("https://other.example/chat".into()),
                ..Default::default()
            },
            provider_params: json!({}),
            context_limit: None,
            bedrock_config: None,
        };
        let error = config.http_endpoint().unwrap_err();
        assert!(error.message.contains("allow_cross_origin_credentials"));
        assert!(!error.message.contains("other.example"));
    }

    #[test]
    fn connection_root_rejects_query_even_when_endpoint_query_is_allowed() {
        let connection_with_query = connection("https://api.example/v1?token=shadow");
        let error = validate_credentialed_url(
            &connection_with_query,
            false,
            "/chat/completions?alt=sse",
            "endpoint",
            CredentialedUrlKind::Http,
            true,
        )
        .unwrap_err();
        assert!(error.message.contains("no userinfo, query or fragment"));

        let clean_connection = connection("https://api.example/v1");
        validate_credentialed_url(
            &clean_connection,
            false,
            "/chat/completions?alt=sse",
            "endpoint",
            CredentialedUrlKind::Http,
            true,
        )
        .unwrap();
    }

    #[test]
    fn shared_origin_validator_treats_http_and_websocket_schemes_as_one_origin() {
        for target in [
            "https://api.example/realtime?model=m",
            "wss://api.example/realtime?model=m",
        ] {
            validate_credentialed_target_url(
                "https://api.example/v1",
                false,
                target,
                "realtime_endpoint",
                ProtocolTransportKind::Websocket,
                true,
            )
            .unwrap();
        }

        let error = validate_credentialed_target_url(
            "https://api.example/v1",
            false,
            "wss://other.example/realtime",
            "realtime_endpoint",
            ProtocolTransportKind::Websocket,
            true,
        )
        .unwrap_err();
        assert!(error.message.contains("allow_cross_origin_credentials"));
        assert!(!error.message.contains("other.example"));
    }

    #[test]
    fn shared_origin_validator_rejects_websocket_targets_for_http_protocols() {
        let error = validate_credentialed_target_url(
            "https://api.example/v1",
            false,
            "wss://api.example/realtime",
            "endpoint",
            ProtocolTransportKind::Http,
            true,
        )
        .unwrap_err();
        assert!(error.message.contains("HTTP(S)"));
    }

}
