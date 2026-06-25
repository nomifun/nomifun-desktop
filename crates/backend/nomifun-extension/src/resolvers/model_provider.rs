use crate::types::{ExtModelProvider, ResolvedModelProvider};

/// Resolve a single model provider contribution.
pub fn resolve_model_provider(provider: &ExtModelProvider, extension_name: &str) -> ResolvedModelProvider {
    ResolvedModelProvider {
        extension_name: extension_name.to_owned(),
        id: provider.id.clone(),
        name: provider.name.clone(),
        description: provider.description.clone(),
        protocol: provider.protocol.clone(),
        base_url: provider.base_url.clone(),
        models: provider.models.clone(),
    }
}

/// Resolve all model provider contributions from an extension.
pub fn resolve_model_providers(providers: &[ExtModelProvider], extension_name: &str) -> Vec<ResolvedModelProvider> {
    providers
        .iter()
        .map(|p| resolve_model_provider(p, extension_name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_model_provider() {
        let provider = ExtModelProvider {
            id: "openai-compat".into(),
            name: "OpenAI Compatible".into(),
            description: Some("An OpenAI-compatible provider".into()),
            protocol: Some("openai".into()),
            base_url: Some("https://api.example.com/v1".into()),
            models: vec!["gpt-4".into(), "gpt-3.5-turbo".into()],
        };

        let result = resolve_model_provider(&provider, "my-ext");

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "openai-compat");
        assert_eq!(result.protocol.as_deref(), Some("openai"));
        assert_eq!(result.models.len(), 2);
    }

    #[test]
    fn test_resolve_model_provider_minimal() {
        let provider = ExtModelProvider {
            id: "minimal".into(),
            name: "Minimal".into(),
            description: None,
            protocol: None,
            base_url: None,
            models: vec![],
        };

        let result = resolve_model_provider(&provider, "my-ext");
        assert!(result.description.is_none());
        assert!(result.protocol.is_none());
        assert!(result.models.is_empty());
    }

    #[test]
    fn test_resolve_model_providers_empty() {
        let result = resolve_model_providers(&[], "my-ext");
        assert!(result.is_empty());
    }
}
