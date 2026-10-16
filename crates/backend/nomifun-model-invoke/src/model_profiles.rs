//! Exact official-document profiles used only while editing model settings.
//! Native catalog limits remain authoritative, and unknown IDs remain valid.
use std::sync::LazyLock;

use nomifun_api_types::{ModelConfigurationDefaults, ModelContextLimitKind, ModelInfo, ModelTask, ModelTaskSource};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub platforms: Vec<String>,
    pub chat_protocol: Option<String>,
    pub base_url_override: Option<String>,
    pub checked_at: String,
    pub sources: Vec<String>,
    pub notes: String,
    #[serde(default)]
    pub models: Vec<DocumentedModel>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentedModel {
    pub ids: Vec<String>,
    pub chat_protocol: Option<String>,
    pub base_url_override: Option<String>,
    pub context_limit: Option<i64>,
    pub context_limit_kind: Option<ModelContextLimitKind>,
    pub output_limit: Option<i64>,
    #[serde(default)]
    pub reasoning_efforts: Option<Vec<String>>,
    #[serde(default = "empty_params")]
    pub provider_params: Value,
    #[serde(default)]
    pub sources: Vec<String>,
}

fn empty_params() -> Value { serde_json::json!({}) }

static PROFILES: LazyLock<Vec<ProviderProfile>> = LazyLock::new(|| {
    [
        include_str!("model_profiles/global.json"),
        include_str!("model_profiles/china-foundation.json"),
        include_str!("model_profiles/cloud-platforms.json"),
        include_str!("model_profiles/gateways.json"),
    ].into_iter().flat_map(|source| {
        serde_json::from_str::<Vec<ProviderProfile>>(source)
            .expect("bundled official model profiles must be valid")
    }).collect()
});

pub fn provider_profiles() -> &'static [ProviderProfile] { &PROFILES }

pub fn provider_profile(platform: &str) -> Option<&'static ProviderProfile> {
    PROFILES.iter().find(|profile| profile.platforms.iter().any(|id| id == platform))
}

pub fn recommended_chat_protocol(platform: &str, model: Option<&str>) -> Option<&'static str> {
    let provider = provider_profile(platform)?;
    let exact = model.and_then(|model| provider.models.iter()
        .find(|profile| profile.ids.iter().any(|id| id == model.trim())));
    exact.and_then(|profile| profile.chat_protocol.as_deref()).or(provider.chat_protocol.as_deref())
}

pub fn documented_chat_base_url(platform: &str, model: Option<&str>) -> Option<&'static str> {
    let provider = provider_profile(platform)?;
    let exact = model.and_then(|model| provider.models.iter()
        .find(|profile| profile.ids.iter().any(|id| id == model.trim())));
    exact.and_then(|profile| profile.base_url_override.as_deref()).or(provider.base_url_override.as_deref())
}

pub fn documented_model_defaults(platform: &str, model: &str) -> Option<ModelConfigurationDefaults> {
    let provider = provider_profile(platform)?;
    let model = model.trim();
    let profile = provider.models.iter().find(|profile| profile.ids.iter().any(|id| id == model))?;
    let protocol_id = profile.chat_protocol.as_ref().or(provider.chat_protocol.as_ref())?;
    let sources = documentation_sources(provider, profile);
    Some(ModelConfigurationDefaults {
        model:model.to_owned(), protocol_id:protocol_id.clone(),
        context_limit:profile.context_limit, context_limit_kind:profile.context_limit_kind,
        output_limit:profile.output_limit, provider_params:profile.provider_params.clone(),
        reasoning_efforts:profile.reasoning_efforts.clone(), sources, checked_at:provider.checked_at.clone(),
    })
}

fn documentation_sources(provider: &ProviderProfile, model: &DocumentedModel) -> Vec<String> {
    let preferred = if model.sources.is_empty() { &provider.sources } else { &model.sources };
    let mut sources = Vec::new();
    for source in preferred {
        if !sources.contains(source) { sources.push(source.clone()); }
    }
    sources
}

/// Fill only absent native catalog information, with field-level document
/// provenance. An aggregator does not inherit its upstream vendor's profile.
pub fn supplement_documented_catalog_model(platform: &str, model: &mut ModelInfo) {
    let Some(defaults) = documented_model_defaults(platform, &model.id) else { return };
    if model.tasks.is_empty() {
        model.tasks = vec![ModelTask::Chat];
        model.tasks_source = Some(ModelTaskSource::OfficialDocumentation);
    }
    if !model.tasks.contains(&ModelTask::Chat) { return; }
    let mut sources = model.token_limit_sources.clone().unwrap_or_default();
    let provider = provider_profile(platform).expect("documented model has a provider");
    let source = provider.models.iter().find(|profile| profile.ids.contains(&model.id))
        .and_then(|profile| profile.sources.first()).or_else(|| provider.sources.first())
        .cloned().unwrap_or_default();
    if model.context_limit.is_none() && defaults.context_limit.is_some() {
        model.context_limit = defaults.context_limit;
        sources.context_limit = Some(format!("official_documentation:{source}"));
        sources.context_limit_kind = defaults.context_limit_kind;
    }
    if model.output_limit.is_none() && defaults.output_limit.is_some() {
        model.output_limit = defaults.output_limit;
        sources.output_limit = Some(format!("official_documentation:{source}"));
    }
    if sources.context_limit.is_some() || sources.output_limit.is_some() {
        model.token_limit_sources = Some(sources);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use crate::manifest::{platform_presets, protocol_manifest_for_model_connection, validate_provider_params_for_protocol};

    #[test]
    fn model_documentation_sources_take_priority_and_are_deduplicated() {
        let provider: ProviderProfile = serde_json::from_value(serde_json::json!({
            "platforms":["fixture"],"checked_at":"2026-10-15","notes":"fixture",
            "sources":["https://vendor.example/audit","https://vendor.example/audit"],
            "models":[{"ids":["model"],"sources":["https://vendor.example/model","https://vendor.example/model","https://vendor.example/protocol"]}]
        })).unwrap();
        assert_eq!(documentation_sources(&provider,&provider.models[0]),
            vec!["https://vendor.example/model","https://vendor.example/protocol"]);
        assert_eq!(provider.sources.len(),2,"provider audit evidence remains intact");
    }

    #[test]
    fn model_documentation_sources_fall_back_to_provider_evidence() {
        let provider: ProviderProfile = serde_json::from_value(serde_json::json!({
            "platforms":["fixture"],"checked_at":"2026-10-15","notes":"fixture",
            "sources":["https://vendor.example/audit","https://vendor.example/audit","https://vendor.example/protocol"],
            "models":[{"ids":["model"]}]
        })).unwrap();
        assert_eq!(documentation_sources(&provider,&provider.models[0]),
            vec!["https://vendor.example/audit","https://vendor.example/protocol"]);
        let step = provider_profile("stepfun").unwrap();
        assert_eq!(documented_model_defaults("stepfun","step-3.7-flash").unwrap().sources,step.sources);
    }

    #[test]
    fn step5_documentation_is_model_specific_without_changing_its_configuration() {
        let defaults = documented_model_defaults("stepfun","step-5-preview").unwrap();
        assert_eq!(defaults.sources,vec![
            "https://platform.stepfun.com/docs/zh/guides/models/step-5-preview",
            "https://platform.stepfun.com/docs/zh/api-reference/responses/responses-create",
            "https://platform.stepfun.com/docs/zh/guides/developer/reasoning",
            "https://platform.stepfun.com/docs/zh/guides/developer/integrations/codex",
        ]);
        assert_eq!(provider_profile("stepfun").unwrap().sources.len(),8);
        assert_eq!(defaults.protocol_id,"openai.chat_text");
        assert_eq!(defaults.context_limit,Some(1_000_000));
        assert_eq!(defaults.context_limit_kind,Some(ModelContextLimitKind::Combined));
        assert_eq!(defaults.output_limit,None);
        assert_eq!(defaults.reasoning_efforts,Some(vec!["low".into(),"medium".into(),"high".into()]));
        assert_eq!(defaults.provider_params,serde_json::json!({"reasoning_effort":"medium"}));
    }

    #[test]
    fn every_preset_is_reviewed_and_documented_profiles_are_executable_configuration() {
        let mut platforms = BTreeSet::new();
        for provider in provider_profiles() {
            assert!(!provider.notes.trim().is_empty());
            assert!(!provider.sources.is_empty());
            assert!(provider.sources.iter().all(|source| source.starts_with("https://")));
            assert!(matches!(provider.checked_at.as_str(),"2026-10-15" | "2026-10-16"));
            for platform in &provider.platforms {
                assert!(platforms.insert(platform.clone()),"duplicate provider profile {platform}");
                let mut ids = BTreeSet::new();
                for model in &provider.models {
                    for id in &model.ids {
                        assert!(!id.trim().is_empty() && ids.insert(id),"duplicate/blank model {platform}: {id}");
                        let defaults = documented_model_defaults(platform,id).expect("exact model profile");
                        validate_provider_params_for_protocol(&defaults.protocol_id,ModelTask::Chat,&defaults.provider_params)
                            .unwrap_or_else(|error| panic!("{platform}/{id}: {error:?}"));
                        assert!(defaults.context_limit.is_none_or(|limit| limit > 0 && limit <= u32::MAX as i64));
                        assert!(defaults.output_limit.is_none_or(|limit| limit > 0 && limit <= u32::MAX as i64));
                        if defaults.context_limit_kind == Some(ModelContextLimitKind::Combined) {
                            if let (Some(context),Some(output)) = (defaults.context_limit,defaults.output_limit) {
                                assert!(output < context,"{platform}/{id} would reserve the whole context as output");
                            }
                        }
                        if let Some(efforts) = &defaults.reasoning_efforts {
                            if let Some(effort) = defaults.provider_params.get("reasoning_effort").and_then(Value::as_str) {
                                assert!(efforts.iter().any(|level| level == effort),"{platform}/{id} default effort not offered");
                            }
                        }
                        let manifest = protocol_manifest_for_model_connection(platform,None,Some(id),ModelTask::Chat);
                        assert_eq!(manifest.recommendation.as_ref().map(|route| route.protocol_id.as_str()),Some(defaults.protocol_id.as_str()),"{platform}/{id}");
                        assert_eq!(manifest.model_defaults,Some(defaults));
                    }
                }
            }
        }
        for preset in platform_presets() {
            assert!(platforms.contains(&preset.platform),"unreviewed preset {}",preset.preset);
        }
    }

    #[test]
    fn exact_profiles_do_not_invent_limits_for_unknown_models_or_other_platforms() {
        assert!(documented_model_defaults("openai","gpt-6-astra-unreleased").is_none());
        assert!(documented_model_defaults("custom","gpt-6-astra").is_none());
        assert!(documented_model_defaults("openrouter","gpt-6-astra").is_none());
        let unknown = protocol_manifest_for_model_connection("OpenAI",None,Some("new-user-entered-model"),ModelTask::Chat);
        assert!(unknown.model_defaults.is_none());
        assert_eq!(unknown.recommendation.unwrap().protocol_id,"openai.responses");
    }

    #[test]
    fn profiles_respect_native_limits_and_subscription_protocol_boundaries() {
        let mut model: ModelInfo = serde_json::from_value(serde_json::json!({
            "id":"gpt-6-astra", "context_limit":500000,"output_limit":32000,
            "tasks":["chat"],"tasks_source":"provider_declared",
            "token_limit_sources":{"context_limit":"context_window","output_limit":"max_output_tokens","context_limit_kind":"input_only"}
        })).unwrap();
        let native = model.clone();
        supplement_documented_catalog_model("openai",&mut model);
        assert_eq!(model,native);
        let mut absent: ModelInfo = serde_json::from_value(serde_json::json!({"id":"gpt-6-astra"})).unwrap();
        supplement_documented_catalog_model("openai",&mut absent);
        assert_eq!(absent.context_limit,Some(1050000));
        assert_eq!(absent.tasks_source,Some(ModelTaskSource::OfficialDocumentation));
        for (platform,model,protocol) in [
            ("dashscope","qwen3.7-plus","openai.chat_text"),
            ("dashscope-coding","qwen3.7-plus","openai.chat_text"),
            ("moonshot-cn","kimi-k3","openai.chat_text"),
            ("moonshot-cn","kimi-k2.7-code","openai.chat_text"),
        ] {
            let manifest = protocol_manifest_for_model_connection(platform,None,Some(model),ModelTask::Chat);
            assert_eq!(manifest.recommendation.unwrap().protocol_id,protocol,"{platform}/{model}");
        }
        let glm = protocol_manifest_for_model_connection("glm-coding-plan",None,Some("glm-5.3"),ModelTask::Chat).recommendation.unwrap();
        assert_eq!(glm.default_base_url.as_deref(),Some("https://open.bigmodel.cn/api/coding/paas/v4"));
        assert!(!glm.base_url_override_required);
        let relay = protocol_manifest_for_model_connection("glm-coding-plan",Some("https://relay.example/company/v1"),
            Some("glm-5.3"),ModelTask::Chat).recommendation.unwrap();
        assert_eq!(relay.protocol_id,"openai.chat_text");
        assert_eq!(relay.default_base_url.as_deref(),Some("https://relay.example/company/v1"));
        assert!(!relay.base_url_override_required);
        assert!(documented_model_defaults("glm-coding-plan","glm-5.3-flashx").is_none());
    }
}
