//! Repair a release-generated capability misclassification before services are
//! exposed. The persisted graph remains the only invocation authority; neither
//! catalog reads nor adapters silently switch a caller's task or protocol.

use std::collections::HashMap;

use nomifun_api_types::{
    AgnesModelContract, ModelTask, ProviderModelCapabilityInput, agnes_model_contract,
};
use nomifun_common::AppError;
use nomifun_db::{
    IProviderConnectionRepository, IProviderModelCapabilityRepository, IProviderModelRepository,
    IProviderRepository, NewProviderModel, ProviderModelCapabilityRow,
};
use nomifun_model_invoke::join_endpoint;
use reqwest::Url;

use crate::provider::{ConnectionTarget, validate_capability};
use crate::provider_model::{capability_row_to_input, serialize_capabilities};

/// Called once at application startup, before jobs or catalog consumers run.
/// Only the exact Agnes v2.0 + single Agnes Images capability is eligible.
/// Custom capability graphs/endpoints are left intact for explicit editing.
pub async fn repair_known_provider_model_configurations(
    providers: &dyn IProviderRepository,
    models: &dyn IProviderModelRepository,
    capabilities: &dyn IProviderModelCapabilityRepository,
    connections: &dyn IProviderConnectionRepository,
) -> Result<usize, AppError> {
    let mut repaired = 0;
    for provider in providers
        .list()
        .await?
        .into_iter()
        .filter(|provider| provider.platform.trim().eq_ignore_ascii_case("agnes"))
    {
        for model in models.list_for_provider(&provider.provider_id).await? {
            if agnes_model_contract(&model.model) != Some(AgnesModelContract::VideoV20) {
                continue;
            }
            // Capture the revision before reading the graph; the ordinary
            // transactional save fences a concurrent configuration edit.
            let Some(current) = providers.find_by_id(&provider.provider_id).await? else {
                continue;
            };
            if !current.platform.trim().eq_ignore_ascii_case("agnes") {
                continue;
            }
            let Some(model) = models.get(&provider.provider_id, &model.model).await? else {
                continue;
            };
            let rows = capabilities
                .list_for_model(&provider.provider_id, &model.model)
                .await?;
            let [row] = rows.as_slice() else { continue };
            if row.task != "image_generation" || row.protocol.trim() != "agnes.images" {
                continue;
            }
            let Some(endpoint) = video_endpoint(row.endpoint.as_deref()) else {
                tracing::warn!(
                    provider_id = %provider.provider_id,
                    model = %model.model,
                    "Agnes v2.0 image capability has a custom endpoint; edit it explicitly as video_generation / agnes.video_jobs"
                );
                continue;
            };
            // Images have no polling/content/realtime transport. Do not
            // overwrite a customized or ambiguous graph as a default repair.
            if row.poll_endpoint.is_some()
                || row.content_endpoint.is_some()
                || row.realtime_endpoint.is_some()
            {
                continue;
            }
            let targets = connections
                .list_for_provider(&provider.provider_id)
                .await?
                .into_iter()
                .map(|connection| {
                    (connection.role, ConnectionTarget {
                        base_url: connection.base_url,
                        auth_scheme: connection.auth_scheme,
                    })
                })
                .collect::<HashMap<_, _>>();
            let connection_base = if row.connection_role == "default" {
                current.base_url.as_str()
            } else if let Some(target) = targets.get(&row.connection_role) {
                target.base_url.as_str()
            } else {
                continue;
            };
            let input = repaired_video_capability(row, endpoint, connection_base)?;
            if validate_capability(
                &current.platform,
                &current.base_url,
                &current.auth_scheme,
                &targets,
                &input,
            ).is_err()
            {
                // In particular, never grant cross-origin credential access
                // to make a repair pass. Validation failures need user review.
                tracing::warn!(
                    provider_id = %provider.provider_id,
                    model = %model.model,
                    "Agnes v2.0 capability repair needs explicit connection/configuration review"
                );
                continue;
            }
            let serialized = serialize_capabilities(&[input])?;
            let db_capabilities = serialized.iter().map(|value| value.as_db()).collect::<Vec<_>>();
            models.save(
                &provider.provider_id,
                current.config_revision,
                &NewProviderModel {
                    model: &model.model,
                    enabled: model.enabled,
                    sort_order: model.sort_order,
                    description: model.description.as_deref(),
                    capabilities: &db_capabilities,
                },
            ).await?;
            // Save replaces the task atomically, clears obsolete health and
            // advances the revision without modifying model identity/labels,
            // provider credentials, connections or historical job records.
            repaired += 1;
        }
    }
    Ok(repaired)
}

fn video_endpoint(endpoint: Option<&str>) -> Option<Option<String>> {
    let Some(endpoint) = endpoint else { return Some(None) };
    match endpoint.trim() {
        "/images/generations" => Some(Some("/videos".into())),
        "/v1/images/generations" => Some(Some("/v1/videos".into())),
        endpoint => {
            let mut url = Url::parse(endpoint).ok()?;
            if !matches!(url.scheme(), "http" | "https")
                || url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/v1/images/generations"
            {
                return None;
            }
            url.set_path("/v1/videos");
            Some(Some(url.into()))
        }
    }
}

fn repaired_video_capability(
    row: &ProviderModelCapabilityRow,
    endpoint: Option<String>,
    connection_base: &str,
) -> Result<ProviderModelCapabilityInput, AppError> {
    let mut input = capability_row_to_input(row)?;
    let base = input.base_url_override.as_deref().unwrap_or(connection_base);
    let submit = join_endpoint(base, endpoint.as_deref().unwrap_or("/videos"));
    let mut poll = Url::parse(&submit)
        .map_err(|_| AppError::BadRequest("invalid Agnes v2.0 repair target".into()))?;
    // Keep the existing effective origin (including proxy ports); do not
    // redirect a proxy's credential to the public Agnes host.
    poll.set_path("/agnesapi");
    poll.set_query(None);
    poll.set_fragment(None);
    let poll_endpoint = format!("{}?video_id={{id}}", poll.as_str());
    if let Some(params) = input.provider_params.as_object_mut() {
        // These keys belong to the accidentally selected image protocol.
        // Frame/dimension defaults and vendor extensions remain untouched.
        for key in ["n", "size", "quality", "response_format", "output_format"] {
            params.remove(key);
        }
    }
    input.task = ModelTask::VideoGeneration;
    input.protocol = "agnes.video_jobs".into();
    input.endpoint = endpoint;
    input.poll_endpoint = Some(poll_endpoint);
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_image_submit_paths_can_be_repaired() {
        assert_eq!(video_endpoint(None), Some(None));
        assert_eq!(video_endpoint(Some("/images/generations")), Some(Some("/videos".into())));
        assert_eq!(video_endpoint(Some("/v1/images/generations")), Some(Some("/v1/videos".into())));
        assert_eq!(
            video_endpoint(Some("https://proxy.test:9443/v1/images/generations")),
            Some(Some("https://proxy.test:9443/v1/videos".into()))
        );
        for custom in [
            "/custom/images", "//other.test/v1/images/generations",
            "https://proxy.test/v1/images/generations?key=secret",
            "https://proxy.test/v1/images/generations#fragment",
            "https://user:pass@proxy.test/v1/images/generations",
        ] {
            assert!(video_endpoint(Some(custom)).is_none());
        }
    }
}
