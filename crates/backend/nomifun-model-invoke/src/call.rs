//! The fully-resolved call: everything an adapter needs to execute one task
//! against one provider connection. Produced by the resolver (Task 2); pure
//! data plus the [`ResolvedCall::dispatch_target`] glue onto
//! [`nomifun_api_types::resolve_dispatch_target`].

use crate::auth::AuthMaterial;
use crate::types::TaskRequest;

/// The connection profile a call rides on (default `providers` row or a
/// role-specific `provider_connections` row), with decrypted auth material.
/// Deliberately not `Debug`: `auth` holds live credentials.
#[derive(Clone)]
pub struct ResolvedConnection {
    /// Connection role (`"default"` or a named role such as `"voice"`).
    pub role: String,
    pub base_url: String,
    /// When set, `base_url` is already the complete endpoint URL.
    pub is_full_url: bool,
    pub auth: AuthMaterial,
    /// Connection-level extra config (opaque to the resolver).
    pub extra: serde_json::Value,
}

/// One task invocation, fully resolved against catalog + connection.
#[derive(Clone)]
pub struct ResolvedCall {
    pub provider_id: String,
    pub platform: String,
    pub model: String,
    pub task: nomifun_api_types::ModelTask,
    pub connection: ResolvedConnection,
    /// Per-model params (`provider_models.params`): endpoint/request-shape
    /// overrides, service defaults, timeouts, …
    pub model_params: serde_json::Value,
    pub request: TaskRequest,
}

impl ResolvedCall {
    /// The endpoint + request shape for this call, resolved by the single
    /// dispatch authority in `nomifun-api-types`.
    pub fn dispatch_target(&self) -> nomifun_api_types::DispatchTarget {
        nomifun_api_types::resolve_dispatch_target(
            &self.platform,
            &self.connection.base_url,
            self.connection.is_full_url,
            self.task,
            &self.model_params,
        )
    }
}

#[cfg(test)]
mod tests {
    use nomifun_api_types::{ModelTask, RequestShape};
    use serde_json::json;

    use super::*;
    use crate::auth::AuthScheme;
    use crate::types::ChatTextRequest;

    pub(crate) fn chat_call(base_url: &str, is_full_url: bool) -> ResolvedCall {
        ResolvedCall {
            provider_id: "018f1234-5678-7abc-8def-012345678990".into(),
            platform: "openai".into(),
            model: "gpt-4o-mini".into(),
            task: ModelTask::Chat,
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: base_url.into(),
                is_full_url,
                auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({"api_keys": ["sk"]}) },
                extra: json!({}),
            },
            model_params: json!({}),
            request: TaskRequest::ChatText(ChatTextRequest { prompt: "hi".into(), system: None, extra: json!({}) }),
        }
    }

    #[test]
    fn dispatch_target_delegates_to_api_types_resolver() {
        let call = chat_call("https://api.openai.com", false);
        let t = call.dispatch_target();
        assert_eq!(t.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(t.method, "POST");
        assert_eq!(t.shape, RequestShape::Json);

        // is_full_url and model_params flow through.
        let full = chat_call("https://proxy.example/custom", true);
        assert_eq!(full.dispatch_target().url, "https://proxy.example/custom");
        let mut with_params = chat_call("https://api.openai.com", false);
        with_params.model_params = json!({"endpoint": "/custom/chat", "request_shape": "multipart"});
        let t = with_params.dispatch_target();
        assert_eq!(t.url, "https://api.openai.com/custom/chat");
        assert_eq!(t.shape, RequestShape::Multipart);
    }
}
