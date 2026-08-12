use crate::error::DbError;
use crate::models::ClientPreference;

const MODEL_FAILOVER_KEY: &str = "agent.model_failover";
const COLLABORATION_MODELS_KEY: &str = "nomi.collaborationModels";
const NOMI_DEFAULT_MODEL_KEY: &str = "nomi.defaultModel";
const KNOWLEDGE_AUTOGEN_MODEL_KEY: &str = "knowledge.autogenModel";
pub const KNOWLEDGE_RETRIEVAL_KEY: &str = "knowledge.retrieval";
const IMAGE_GENERATION_MODEL_KEY: &str = "tools.imageGenerationModel";
const SPEECH_TO_TEXT_KEY: &str = "tools.speechToText";
const TEXT_TO_SPEECH_KEY: &str = "tools.textToSpeech";

/// Client preference data access abstraction.
///
/// Provides CRUD operations on the generic key-value `client_preferences` table.
#[async_trait::async_trait]
pub trait IClientPreferenceRepository: Send + Sync {
    /// Returns all client preferences.
    async fn get_all(&self) -> Result<Vec<ClientPreference>, DbError>;

    /// Returns preferences for the given keys only.
    /// Keys that don't exist are simply omitted from the result.
    async fn get_by_keys(&self, keys: &[&str]) -> Result<Vec<ClientPreference>, DbError>;

    /// Inserts or updates a batch of key-value pairs.
    async fn upsert_batch(&self, entries: &[(&str, &str)]) -> Result<(), DbError>;

    /// Deletes the given keys.
    async fn delete_keys(&self, keys: &[&str]) -> Result<(), DbError>;

    /// Applies upserts and deletes as one logical update.
    ///
    /// SQLite overrides this method so Provider parent validation and all
    /// preference changes share one writer transaction. Test doubles may rely
    /// on this default implementation when transaction semantics are
    /// irrelevant to the test.
    async fn update_batch(
        &self,
        upserts: &[(&str, &str)],
        delete_keys: &[&str],
    ) -> Result<(), DbError> {
        self.delete_keys(delete_keys).await?;
        self.upsert_batch(upserts).await
    }
}

#[derive(Debug)]
pub(crate) struct NormalizedProviderPreference {
    pub value: String,
    pub provider_ids: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProviderPreferenceDeleteAction {
    Keep,
    Delete,
    Update(String),
}

#[derive(Debug, Clone, Copy)]
enum ProviderPreferenceKind {
    ModelFailover,
    CollaborationModels,
    RequiredModelObject,
    OptionalObjectProviderId,
    KnowledgeRetrieval,
}

fn provider_preference_kind(key: &str) -> Option<ProviderPreferenceKind> {
    match key {
        MODEL_FAILOVER_KEY => Some(ProviderPreferenceKind::ModelFailover),
        COLLABORATION_MODELS_KEY => Some(ProviderPreferenceKind::CollaborationModels),
        NOMI_DEFAULT_MODEL_KEY
        | KNOWLEDGE_AUTOGEN_MODEL_KEY
        | IMAGE_GENERATION_MODEL_KEY
        // TTS has no enabled switch: `provider_id` and `model` are both required,
        // so a Provider deletion drops the whole key ("no global default") rather
        // than leaving a half-broken reference behind.
        | TEXT_TO_SPEECH_KEY => Some(ProviderPreferenceKind::RequiredModelObject),
        SPEECH_TO_TEXT_KEY => Some(ProviderPreferenceKind::OptionalObjectProviderId),
        KNOWLEDGE_RETRIEVAL_KEY => Some(ProviderPreferenceKind::KnowledgeRetrieval),
        _ if is_channel_default_model_key(key) => {
            Some(ProviderPreferenceKind::RequiredModelObject)
        }
        _ => None,
    }
}

fn reject_unknown_fields(
    key: &str,
    path: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
) -> Result<(), DbError> {
    if let Some(field) = object.keys().find(|field| !allowed.contains(&field.as_str())) {
        return Err(invalid_preference(
            key,
            format!("{path}.{field} is not a supported field"),
        ));
    }
    Ok(())
}

fn normalize_retrieval_stage(
    key: &str,
    path: &str,
    value: &serde_json::Value,
    provider_ids: &mut Vec<String>,
) -> Result<serde_json::Value, DbError> {
    let object = require_object(key, path, value)?;
    let mode = object
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid_preference(key, format!("{path}.mode must be 'local' or 'remote'")))?;
    match mode {
        "local" => {
            reject_unknown_fields(key, path, object, &["mode"])?;
            Ok(serde_json::json!({"mode": "local"}))
        }
        "remote" => {
            reject_unknown_fields(key, path, object, &["mode", "provider_id", "model"])?;
            reject_legacy_provider_id_field(key, path, object)?;
            require_model_field(key, path, object)?;
            let provider_id = required_provider_field(key, path, object, "provider_id")?;
            let model = object
                .get("model")
                .and_then(serde_json::Value::as_str)
                .expect("validated retrieval model");
            provider_ids.push(provider_id.clone());
            Ok(serde_json::json!({
                "mode": "remote",
                "provider_id": provider_id,
                "model": model,
            }))
        }
        _ => Err(invalid_preference(
            key,
            format!("{path}.mode must be 'local' or 'remote'"),
        )),
    }
}

fn parse_knowledge_retrieval_preference(
    key: &str,
    value: &str,
) -> Result<NormalizedProviderPreference, DbError> {
    let parsed = parse_json(key, value)?;
    let object = require_object(key, "$", &parsed)?;
    reject_unknown_fields(key, "$", object, &["embedding", "rerank"])?;
    let mut provider_ids = Vec::new();
    let embedding_value = object.get("embedding").ok_or_else(|| {
        invalid_preference(
            key,
            "$.embedding is required; use {\"mode\":\"local\"} for local retrieval",
        )
    })?;
    let rerank_value = object.get("rerank").ok_or_else(|| {
        invalid_preference(
            key,
            "$.rerank is required; use {\"mode\":\"local\"} for local retrieval",
        )
    })?;
    let embedding = normalize_retrieval_stage(
        key,
        "$.embedding",
        embedding_value,
        &mut provider_ids,
    )?;
    let rerank = normalize_retrieval_stage(
        key,
        "$.rerank",
        rerank_value,
        &mut provider_ids,
    )?;
    provider_ids.sort();
    provider_ids.dedup();
    Ok(NormalizedProviderPreference {
        value: serde_json::json!({
            "embedding": embedding,
            "rerank": rerank,
        })
        .to_string(),
        provider_ids,
    })
}

fn is_channel_default_model_key(key: &str) -> bool {
    key.strip_prefix("channels.")
        .and_then(|rest| rest.strip_suffix(".defaultModel"))
        .is_some_and(|platform| !platform.is_empty())
}

fn invalid_preference(key: &str, message: impl std::fmt::Display) -> DbError {
    DbError::Conflict(format!("invalid client preference '{key}': {message}"))
}

fn canonical_provider_id(
    key: &str,
    path: &str,
    value: &str,
) -> Result<String, DbError> {
    nomifun_common::ProviderId::parse(value)
        .map(nomifun_common::ProviderId::into_string)
        .map_err(|error| {
            invalid_preference(
                key,
                format!("Provider ID at {path} is not a canonical UUIDv7: {error}"),
            )
        })
}

fn parse_json(key: &str, value: &str) -> Result<serde_json::Value, DbError> {
    serde_json::from_str(value)
        .map_err(|error| invalid_preference(key, format!("value must be valid JSON: {error}")))
}

fn require_object<'a>(
    key: &str,
    path: &str,
    value: &'a serde_json::Value,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, DbError> {
    value
        .as_object()
        .ok_or_else(|| invalid_preference(key, format!("{path} must be an object")))
}

fn require_array<'a>(
    key: &str,
    path: &str,
    value: &'a serde_json::Value,
) -> Result<&'a Vec<serde_json::Value>, DbError> {
    value
        .as_array()
        .ok_or_else(|| invalid_preference(key, format!("{path} must be an array")))
}

fn required_provider_field(
    key: &str,
    path: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<String, DbError> {
    let value = object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            invalid_preference(
                key,
                format!("{path}.{field} must be a canonical Provider UUIDv7 string"),
            )
        })?;
    canonical_provider_id(key, &format!("{path}.{field}"), value)
}

fn reject_legacy_provider_id_field(
    key: &str,
    path: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), DbError> {
    if object.contains_key("id") {
        return Err(invalid_preference(
            key,
            format!("{path}.id is a legacy Provider field; use {path}.provider_id"),
        ));
    }
    Ok(())
}

fn require_model_field(
    key: &str,
    path: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), DbError> {
    let model = object
        .get("model")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            invalid_preference(
                key,
                format!("{path}.model must be a non-empty trimmed string"),
            )
        })?;
    if model.is_empty() || model.trim() != model {
        return Err(invalid_preference(
            key,
            format!("{path}.model must be a non-empty trimmed string"),
        ));
    }
    Ok(())
}

fn optional_provider_field(
    key: &str,
    path: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<String>, DbError> {
    match object.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => {
            canonical_provider_id(key, &format!("{path}.{field}"), value).map(Some)
        }
        Some(_) => Err(invalid_preference(
            key,
            format!("{path}.{field} must be null or a canonical Provider UUIDv7 string"),
        )),
    }
}

fn parse_json_provider_preference(
    key: &str,
    value: &str,
    kind: ProviderPreferenceKind,
) -> Result<NormalizedProviderPreference, DbError> {
    let parsed = parse_json(key, value)?;
    let mut provider_ids = Vec::new();

    match kind {
        ProviderPreferenceKind::ModelFailover => {
            let object = require_object(key, "$", &parsed)?;
            if let Some(queue) = object.get("queue") {
                for (index, item) in require_array(key, "$.queue", queue)?.iter().enumerate() {
                    let path = format!("$.queue[{index}]");
                    let item = require_object(key, &path, item)?;
                    provider_ids.push(required_provider_field(
                        key,
                        &path,
                        item,
                        "provider_id",
                    )?);
                }
            }
        }
        ProviderPreferenceKind::CollaborationModels => {
            for (index, item) in require_array(key, "$", &parsed)?.iter().enumerate() {
                let path = format!("$[{index}]");
                let item = require_object(key, &path, item)?;
                provider_ids.push(required_provider_field(
                    key,
                    &path,
                    item,
                    "provider_id",
                )?);
            }
        }
        ProviderPreferenceKind::RequiredModelObject => {
            let object = require_object(key, "$", &parsed)?;
            reject_legacy_provider_id_field(key, "$", object)?;
            require_model_field(key, "$", object)?;
            provider_ids.push(required_provider_field(
                key,
                "$",
                object,
                "provider_id",
            )?);
        }
        ProviderPreferenceKind::OptionalObjectProviderId => {
            let object = require_object(key, "$", &parsed)?;
            if let Some(provider_id) =
                optional_provider_field(key, "$", object, "provider_id")?
            {
                provider_ids.push(provider_id);
            }
        }
        ProviderPreferenceKind::KnowledgeRetrieval => {
            return parse_knowledge_retrieval_preference(key, value);
        }
    }

    provider_ids.sort();
    provider_ids.dedup();
    Ok(NormalizedProviderPreference {
        value: parsed.to_string(),
        provider_ids,
    })
}

pub(crate) fn normalize_provider_preference(
    key: &str,
    value: &str,
) -> Result<NormalizedProviderPreference, DbError> {
    let Some(kind) = provider_preference_kind(key) else {
        return Ok(NormalizedProviderPreference {
            value: value.to_owned(),
            provider_ids: Vec::new(),
        });
    };

    parse_json_provider_preference(key, value, kind)
}

pub(crate) fn provider_preference_delete_action(
    key: &str,
    value: &str,
    provider_id: &str,
) -> Result<ProviderPreferenceDeleteAction, DbError> {
    let Some(kind) = provider_preference_kind(key) else {
        return Ok(ProviderPreferenceDeleteAction::Keep);
    };

    let normalized = parse_json_provider_preference(key, value, kind)?;
    if !normalized.provider_ids.iter().any(|id| id == provider_id) {
        return Ok(ProviderPreferenceDeleteAction::Keep);
    }

    let mut parsed = parse_json(key, &normalized.value)?;
    match kind {
        ProviderPreferenceKind::ModelFailover => {
            let queue = parsed
                .as_object_mut()
                .and_then(|object| object.get_mut("queue"))
                .and_then(serde_json::Value::as_array_mut)
                .expect("validated model failover queue");
            queue.retain(|item| {
                item.get("provider_id").and_then(serde_json::Value::as_str)
                    != Some(provider_id)
            });
            Ok(ProviderPreferenceDeleteAction::Update(parsed.to_string()))
        }
        ProviderPreferenceKind::CollaborationModels => {
            let models = parsed
                .as_array_mut()
                .expect("validated collaboration model array");
            models.retain(|item| {
                item.get("provider_id").and_then(serde_json::Value::as_str)
                    != Some(provider_id)
            });
            Ok(ProviderPreferenceDeleteAction::Update(parsed.to_string()))
        }
        ProviderPreferenceKind::RequiredModelObject => {
            Ok(ProviderPreferenceDeleteAction::Delete)
        }
        ProviderPreferenceKind::OptionalObjectProviderId => {
            parsed
                .as_object_mut()
                .expect("validated optional Provider preference")
                .insert("provider_id".to_owned(), serde_json::Value::Null);
            Ok(ProviderPreferenceDeleteAction::Update(parsed.to_string()))
        }
        ProviderPreferenceKind::KnowledgeRetrieval => {
            let object = parsed
                .as_object_mut()
                .expect("validated knowledge retrieval preference");
            for stage in ["embedding", "rerank"] {
                let stage_value = object
                    .get_mut(stage)
                    .expect("normalized retrieval stage");
                if stage_value
                    .get("provider_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(provider_id)
                {
                    *stage_value = serde_json::json!({"mode": "local"});
                }
            }
            Ok(ProviderPreferenceDeleteAction::Update(parsed.to_string()))
        }
    }
}

#[cfg(test)]
mod provider_reference_tests {
    use super::*;

    const PROVIDER_A: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const PROVIDER_B: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    #[test]
    fn registry_extracts_every_supported_provider_reference_shape() {
        let cases = [
            (
                MODEL_FAILOVER_KEY,
                serde_json::json!({
                    "queue": [
                        {"provider_id": PROVIDER_A, "model": "a"},
                        {"provider_id": PROVIDER_B, "model": "b"}
                    ]
                })
                .to_string(),
                2,
            ),
            (
                COLLABORATION_MODELS_KEY,
                serde_json::json!([{"provider_id": PROVIDER_A, "model": "a"}])
                    .to_string(),
                1,
            ),
            (
                NOMI_DEFAULT_MODEL_KEY,
                serde_json::json!({"provider_id": PROVIDER_A, "model": "a"}).to_string(),
                1,
            ),
            (
                KNOWLEDGE_AUTOGEN_MODEL_KEY,
                serde_json::json!({"provider_id": PROVIDER_A, "model": "a"})
                    .to_string(),
                1,
            ),
            (
                KNOWLEDGE_RETRIEVAL_KEY,
                serde_json::json!({
                    "embedding": {"mode": "remote", "provider_id": PROVIDER_A, "model": "embed"},
                    "rerank": {"mode": "remote", "provider_id": PROVIDER_B, "model": "rerank"}
                })
                .to_string(),
                2,
            ),
            (
                IMAGE_GENERATION_MODEL_KEY,
                serde_json::json!({"provider_id": PROVIDER_A, "model": "a"}).to_string(),
                1,
            ),
            (
                SPEECH_TO_TEXT_KEY,
                serde_json::json!({"enabled": true, "provider_id": PROVIDER_A})
                    .to_string(),
                1,
            ),
            (
                TEXT_TO_SPEECH_KEY,
                serde_json::json!({"provider_id": PROVIDER_A, "model": "tts-1", "voice": null})
                    .to_string(),
                1,
            ),
            (
                "channels.telegram.defaultModel",
                serde_json::json!({"provider_id": PROVIDER_A, "model": "a"}).to_string(),
                1,
            ),
        ];

        for (key, value, expected_count) in cases {
            let normalized = normalize_provider_preference(key, &value).unwrap();
            assert_eq!(
                normalized.provider_ids.len(),
                expected_count,
                "unexpected Provider reference count for {key}"
            );
        }
    }

    #[test]
    fn registry_rejects_noncanonical_or_malformed_registered_values() {
        for (key, value) in [
            (
                MODEL_FAILOVER_KEY,
                r#"{"queue":[{"provider_id":"prov_legacy","model":"a"}]}"#,
            ),
            (COLLABORATION_MODELS_KEY, r#"[{"model":"a"}]"#),
            (NOMI_DEFAULT_MODEL_KEY, r#"{"id":42}"#),
            (KNOWLEDGE_AUTOGEN_MODEL_KEY, "not-json"),
            (
                KNOWLEDGE_RETRIEVAL_KEY,
                r#"{"embedding":{"mode":"remote","provider_id":"prov_legacy","model":"e"},"rerank":{"mode":"local"}}"#,
            ),
            (IMAGE_GENERATION_MODEL_KEY, r#"[]"#),
            (SPEECH_TO_TEXT_KEY, r#"{"provider_id":42}"#),
            (
                "channels.telegram.defaultModel",
                r#"{"id":"0190f5fe-7c00-4a00-8000-000000000001"}"#,
            ),
        ] {
            assert!(
                normalize_provider_preference(key, value).is_err(),
                "{key} unexpectedly accepted malformed Provider reference data"
            );
        }
    }

    #[test]
    fn registry_rejects_legacy_id_for_default_model_objects() {
        for key in [
            NOMI_DEFAULT_MODEL_KEY,
            IMAGE_GENERATION_MODEL_KEY,
            "channels.telegram.defaultModel",
        ] {
            let value = serde_json::json!({
                "id": PROVIDER_A,
                "use_model": "a",
            })
            .to_string();
            let error = normalize_provider_preference(key, &value).unwrap_err();
            assert!(
                error.to_string().contains("legacy Provider field"),
                "{key} returned an unexpected error: {error}"
            );
        }
    }

    #[test]
    fn delete_actions_filter_arrays_delete_defaults_and_null_optional_reference() {
        let collaboration = serde_json::json!([
            {"provider_id": PROVIDER_A, "model": "first"},
            {"provider_id": PROVIDER_B, "model": "keep"},
            {"provider_id": PROVIDER_A, "model": "last"}
        ])
        .to_string();
        let ProviderPreferenceDeleteAction::Update(collaboration) =
            provider_preference_delete_action(
                COLLABORATION_MODELS_KEY,
                &collaboration,
                PROVIDER_A,
            )
            .unwrap()
        else {
            panic!("collaboration models must be filtered");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&collaboration).unwrap(),
            serde_json::json!([{"provider_id": PROVIDER_B, "model": "keep"}])
        );

        assert_eq!(
            provider_preference_delete_action(
                NOMI_DEFAULT_MODEL_KEY,
                &serde_json::json!({"provider_id": PROVIDER_A, "model": "a"})
                    .to_string(),
                PROVIDER_A,
            )
            .unwrap(),
            ProviderPreferenceDeleteAction::Delete
        );

        let ProviderPreferenceDeleteAction::Update(speech) =
            provider_preference_delete_action(
                SPEECH_TO_TEXT_KEY,
                &serde_json::json!({
                    "enabled": true,
                    "provider_id": PROVIDER_A,
                    "model": "whisper"
                })
                .to_string(),
                PROVIDER_A,
            )
            .unwrap()
        else {
            panic!("speech preference must be updated");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&speech).unwrap(),
            serde_json::json!({
                "enabled": true,
                "provider_id": null,
                "model": "whisper"
            })
        );
    }

    #[test]
    fn text_to_speech_preference_is_a_required_model_reference() {
        // A malformed global TTS default must be refused at the write boundary,
        // not stored and then discovered by the robot gateway at speak time.
        for value in [
            r#"{"model":"tts-1"}"#,
            r#"{"provider_id":"prov_legacy","model":"tts-1"}"#,
            r#"{"provider_id":"0190f5fe-7c00-7a00-8000-000000000001","model":" "}"#,
        ] {
            assert!(normalize_provider_preference(TEXT_TO_SPEECH_KEY, value).is_err());
        }
        // Deleting the Provider deletes the default outright — a half-broken
        // default would silently pick the wrong voice on the next turn.
        assert_eq!(
            provider_preference_delete_action(
                TEXT_TO_SPEECH_KEY,
                &serde_json::json!({"provider_id": PROVIDER_A, "model": "tts-1", "voice": "alloy"})
                    .to_string(),
                PROVIDER_A,
            )
            .unwrap(),
            ProviderPreferenceDeleteAction::Delete
        );
    }

    #[test]
    fn knowledge_retrieval_requires_both_explicit_stages() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"embedding": {"mode": "local"}}),
            serde_json::json!({"rerank": {"mode": "local"}}),
        ] {
            assert!(
                normalize_provider_preference(KNOWLEDGE_RETRIEVAL_KEY, &value.to_string())
                    .is_err(),
                "incomplete knowledge retrieval preference unexpectedly accepted: {value}"
            );
        }

        let normalized = normalize_provider_preference(
            KNOWLEDGE_RETRIEVAL_KEY,
            &serde_json::json!({
                "embedding": {"mode": "local"},
                "rerank": {"mode": "local"}
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&normalized.value).unwrap(),
            serde_json::json!({
                "embedding": {"mode": "local"},
                "rerank": {"mode": "local"}
            })
        );
    }

    #[test]
    fn knowledge_retrieval_clears_only_deleted_provider_stages() {
        let value = serde_json::json!({
            "embedding": {"mode": "remote", "provider_id": PROVIDER_A, "model": "embed"},
            "rerank": {"mode": "remote", "provider_id": PROVIDER_B, "model": "rerank"}
        })
        .to_string();
        let ProviderPreferenceDeleteAction::Update(updated) =
            provider_preference_delete_action(KNOWLEDGE_RETRIEVAL_KEY, &value, PROVIDER_A)
                .unwrap()
        else {
            panic!("knowledge retrieval must be atomically rewritten");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&updated).unwrap(),
            serde_json::json!({
                "embedding": {"mode": "local"},
                "rerank": {"mode": "remote", "provider_id": PROVIDER_B, "model": "rerank"}
            })
        );
    }
}
