//! Row → wire projection for the authoritative `provider_models` entity.
//!
//! JSON parse failures on a row degrade to empty/None values with a
//! `tracing::warn!` instead of failing the whole listing — one bad row must
//! never take down `GET /api/providers` (same tolerance strategy as
//! `row_to_profile` in `model_profile.rs` uses for profile rows).

use nomifun_api_types::{ModelHealthStatus, ModelTask, ModelTrait, ProfileSource, ProviderModelResponse};
use nomifun_common::{AppError, ProviderId};
use nomifun_db::ProviderModelRow;

fn source_from_str(s: &str) -> ProfileSource {
    match s {
        "user" => ProfileSource::User,
        _ => ProfileSource::Inferred,
    }
}

/// Convert one `provider_models` row into the wire DTO.
///
/// Only a non-canonical stored `provider_id` is a hard error (it indicates
/// corrupted identity, not a degraded field); malformed JSON columns degrade
/// gracefully: `tasks`/`traits` → empty vec, `params` → `null`, `health` →
/// absent — each with a warning.
pub(crate) fn row_to_model_response(row: ProviderModelRow) -> Result<ProviderModelResponse, AppError> {
    ProviderId::parse(&row.provider_id).map_err(|error| {
        AppError::Internal(format!(
            "stored provider_models.provider_id '{}' is not canonical: {error}",
            row.provider_id
        ))
    })?;

    let tasks: Vec<ModelTask> = serde_json::from_str(&row.tasks).unwrap_or_else(|error| {
        tracing::warn!(
            provider_id = %row.provider_id,
            model = %row.model,
            %error,
            "invalid provider_models.tasks JSON; degrading to empty tasks"
        );
        Vec::new()
    });
    let traits: Vec<ModelTrait> = serde_json::from_str(&row.traits).unwrap_or_else(|error| {
        tracing::warn!(
            provider_id = %row.provider_id,
            model = %row.model,
            %error,
            "invalid provider_models.traits JSON; degrading to empty traits"
        );
        Vec::new()
    });
    let params: serde_json::Value = serde_json::from_str(&row.params).unwrap_or_else(|error| {
        tracing::warn!(
            provider_id = %row.provider_id,
            model = %row.model,
            %error,
            "invalid provider_models.params JSON; degrading to null params"
        );
        serde_json::Value::Null
    });
    let health: Option<ModelHealthStatus> = row.health.as_deref().and_then(|json| {
        serde_json::from_str(json)
            .map_err(|error| {
                tracing::warn!(
                    provider_id = %row.provider_id,
                    model = %row.model,
                    %error,
                    "invalid provider_models.health JSON; dropping health entry"
                );
            })
            .ok()
    });

    Ok(ProviderModelResponse {
        provider_id: row.provider_id,
        model: row.model,
        enabled: row.enabled,
        sort_order: row.sort_order,
        tasks,
        traits,
        protocol: row.protocol,
        connection_role: row.connection_role,
        params,
        context_limit: row.context_limit,
        description: row.description,
        source: source_from_str(&row.source),
        health,
        health_checked_at: row.health_checked_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROVIDER_ID: &str = "018f1234-5678-7abc-8def-012345678990";

    fn sample_row() -> ProviderModelRow {
        ProviderModelRow {
            id: 7,
            provider_id: PROVIDER_ID.into(),
            model: "gpt-4o".into(),
            enabled: true,
            sort_order: 2,
            tasks: r#"["chat"]"#.into(),
            traits: r#"["vision_input"]"#.into(),
            protocol: Some("openai".into()),
            connection_role: None,
            params: r#"{"temperature":0.5}"#.into(),
            context_limit: Some(128000),
            description: Some("desc".into()),
            source: "user".into(),
            health: Some(r#"{"status":"healthy","latency":320}"#.into()),
            health_checked_at: Some(123),
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn projects_all_fields() {
        let resp = row_to_model_response(sample_row()).unwrap();
        assert_eq!(resp.provider_id, PROVIDER_ID);
        assert_eq!(resp.model, "gpt-4o");
        assert!(resp.enabled);
        assert_eq!(resp.sort_order, 2);
        assert_eq!(resp.tasks, vec![ModelTask::Chat]);
        assert_eq!(resp.traits, vec![ModelTrait::VisionInput]);
        assert_eq!(resp.protocol.as_deref(), Some("openai"));
        assert_eq!(resp.params["temperature"], 0.5);
        assert_eq!(resp.context_limit, Some(128000));
        assert_eq!(resp.description.as_deref(), Some("desc"));
        assert_eq!(resp.source, ProfileSource::User);
        assert_eq!(
            resp.health.as_ref().map(|h| h.status),
            Some(nomifun_api_types::HealthStatus::Healthy)
        );
        assert_eq!(resp.health_checked_at, Some(123));
    }

    #[test]
    fn bad_json_degrades_instead_of_failing() {
        let row = ProviderModelRow {
            tasks: "not-json".into(),
            traits: "{broken".into(),
            params: "###".into(),
            health: Some("oops".into()),
            ..sample_row()
        };
        let resp = row_to_model_response(row).unwrap();
        assert!(resp.tasks.is_empty());
        assert!(resp.traits.is_empty());
        assert_eq!(resp.params, serde_json::Value::Null);
        assert!(resp.health.is_none());
    }

    #[test]
    fn noncanonical_provider_id_is_an_error() {
        let row = ProviderModelRow {
            provider_id: "not-a-uuid".into(),
            ..sample_row()
        };
        assert!(row_to_model_response(row).is_err());
    }
}
