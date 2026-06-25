use nomifun_api_types::{FetchModelsResponse, ModelInfo};
use nomifun_common::AppError;
use tokio::task::JoinSet;
use tracing::debug;

use super::FetchConfig;
use super::fetchers::fetch_openai_compatible;

/// URL path suffixes to probe when auto-fixing.
const URL_VARIANTS: &[&str] = &[
    "/v1",
    "/api/v1",
    "/openai/v1",
    "/compatible-mode/v1",
    "/v2",
    "/api/v3",
    "/api/paas/v4",
    "/compatibility/v1",
];

/// Try multiple URL variants in parallel and return the first successful
/// result along with its corrected base URL.
pub(crate) async fn try_fix_url(
    client: &reqwest::Client,
    config: &FetchConfig,
) -> Result<FetchModelsResponse, AppError> {
    let base = config.base_url.trim_end_matches('/');
    let candidates = build_candidates(base);

    debug!(
        base_url = base,
        candidate_count = candidates.len(),
        "Starting URL auto-fix probe"
    );

    let mut set = JoinSet::new();
    for candidate in candidates {
        let client = client.clone();
        let api_key = config.api_key.clone();
        set.spawn(async move {
            let models = fetch_openai_compatible(&client, &candidate, &api_key).await?;
            Ok::<(Vec<ModelInfo>, String), AppError>((models, candidate))
        });
    }

    while let Some(result) = set.join_next().await {
        if let Ok(Ok((models, fixed_url))) = result {
            set.abort_all();
            debug!(fixed_url = %fixed_url, "URL auto-fix succeeded");
            return Ok(FetchModelsResponse {
                models,
                fixed_base_url: Some(fixed_url),
            });
        }
    }

    Err(AppError::BadGateway("All URL variants failed during auto-fix".into()))
}

/// Build candidate URLs from the base URL and standard path suffixes.
fn build_candidates(base: &str) -> Vec<String> {
    URL_VARIANTS.iter().map(|suffix| format!("{base}{suffix}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_candidates_generates_expected_urls() {
        let candidates = build_candidates("https://api.example.com");
        assert_eq!(candidates.len(), URL_VARIANTS.len());
        assert!(candidates.contains(&"https://api.example.com/v1".to_string()));
        assert!(candidates.contains(&"https://api.example.com/api/v1".to_string()));
        assert!(candidates.contains(&"https://api.example.com/openai/v1".to_string()));
    }

    #[test]
    fn build_candidates_no_double_slash() {
        let candidates = build_candidates("https://api.example.com");
        for c in &candidates {
            // After scheme, no double slashes
            let after_scheme = c.strip_prefix("https://").unwrap();
            assert!(!after_scheme.contains("//"), "Double slash found in: {c}");
        }
    }
}
