//! Documented configuration capabilities, independent of runtime dispatch.
//! A native catalog declaration or an explicitly documented model selector is
//! required before a non-Chat route can be advertised as configurable.
use std::{collections::BTreeMap, sync::LazyLock};

use nomifun_api_types::ModelTask;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentationStatus { Supported, CatalogRequired, NotImplemented, Unverified, Retired }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProtocolAudit {
    pub platforms: Vec<String>,
    /// Exact additional official origins verified by the cited documentation.
    /// Workspace or relay suffixes are never inferred from a shared hostname.
    #[serde(default)]
    pub official_origins: Vec<String>,
    pub checked_at: String,
    pub sources: Vec<String>,
    pub notes: String,
    pub tasks: Vec<DocumentedTaskProtocol>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentedTaskProtocol {
    pub task: ModelTask,
    pub status: DocumentationStatus,
    pub protocols: Vec<String>,
    #[serde(default)]
    pub model_ids: Vec<String>,
    /// Prefixes are allowed only for model families expressly described by the
    /// cited vendor documentation. These are not general name heuristics.
    #[serde(default)]
    pub model_prefixes: Vec<String>,
    /// Optional per-dialect limits within a model family. A catalog's task
    /// alone must not bypass a documented restriction to particular models.
    #[serde(default)]
    pub protocol_model_ids: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub protocol_model_prefixes: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub protocol_base_url_overrides: BTreeMap<String, String>,
    pub connection_role: Option<String>,
    pub base_url_override: Option<String>,
    pub endpoint: Option<String>,
    pub poll_endpoint: Option<String>,
    pub content_endpoint: Option<String>,
    pub realtime_endpoint: Option<String>,
    #[serde(default = "empty_params")]
    pub provider_params: Value,
    #[serde(default)]
    pub sources: Vec<String>,
    pub notes: String,
}

fn empty_params() -> Value { serde_json::json!({}) }

#[derive(Debug, Clone)]
pub struct DocumentedProtocolCandidate {
    pub protocol: String,
    pub connection_role: Option<String>,
    pub base_url_override: Option<String>,
    pub endpoint: Option<String>,
    pub poll_endpoint: Option<String>,
    pub content_endpoint: Option<String>,
    pub realtime_endpoint: Option<String>,
    pub provider_params: Value,
    pub sources: Vec<String>,
    pub checked_at: String,
    pub catalog_required: bool,
}

static AUDITS: LazyLock<Vec<ProviderProtocolAudit>> = LazyLock::new(|| {
    [include_str!("provider_protocol_audit/global-and-gateways.json"),
        include_str!("provider_protocol_audit/china-and-voice.json"),
        include_str!("provider_protocol_audit/gateways.json")]
        .into_iter().flat_map(|source| serde_json::from_str::<Vec<ProviderProtocolAudit>>(source)
            .expect("bundled provider protocol audits must be valid")).collect()
});

pub fn provider_protocol_audits() -> &'static [ProviderProtocolAudit] { &AUDITS }

pub fn provider_protocol_audit(platform: &str) -> Option<&'static ProviderProtocolAudit> {
    AUDITS.iter().find(|profile| profile.platforms.iter().any(|value| value == platform.trim()))
}

/// Whether a configured URL belongs to a preset or an explicitly documented
/// regional origin. Paths are deliberately ignored; ports and schemes are not.
pub fn is_official_provider_origin(platform: &str, base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else { return false };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() { return false; }
    let origin = url.origin();
    crate::platform_presets().into_iter().filter(|preset| preset.platform == platform.trim())
        .filter_map(|preset| preset.platform_default_base_url)
        .chain(provider_protocol_audit(platform).into_iter().flat_map(|audit| audit.official_origins.iter().cloned()))
        .filter_map(|root| reqwest::Url::parse(&root).ok())
        .any(|root| root.origin() == origin)
}

fn model_matches(platform: &str, model: &str, task: &DocumentedTaskProtocol) -> bool {
    task.model_ids.iter().any(|id| id == model)
        || task.model_prefixes.iter().any(|prefix| !prefix.is_empty() && model.starts_with(prefix))
        || (task.task == ModelTask::Chat
            && crate::model_profiles::documented_model_defaults(platform, model).is_some())
}

pub fn documented_protocol_candidates(platform: &str, model: &str, task: ModelTask,
    catalog_task_declared: bool) -> Vec<DocumentedProtocolCandidate> {
    let Some(profile) = provider_protocol_audit(platform) else { return Vec::new() };
    let model = model.trim();
    if model.is_empty() { return Vec::new(); }
    let mut candidates = Vec::new();
    for entry in &profile.tasks {
        if entry.task != task || !matches!(entry.status, DocumentationStatus::Supported | DocumentationStatus::CatalogRequired) {
            continue;
        }
        let exact = model_matches(platform, model, entry);
        // Chat candidates are subsequently verified through an actual Agent
        // tool round trip. Media has no such billable automatic probe, so its
        // model/task declaration must come from exact docs or a native catalog.
        if task != ModelTask::Chat {
            match entry.status {
                // A task label cannot grant another model a different native
                // codec: e.g. Flux streaming ASR versus Nova file ASR, or a
                // multimodal embedding versus text arrays.
                DocumentationStatus::Supported if !exact => continue,
                DocumentationStatus::CatalogRequired if !catalog_task_declared => continue,
                _ => {}
            }
        }
        for protocol in &entry.protocols {
            let limited_ids = entry.protocol_model_ids.get(protocol);
            let limited_prefixes = entry.protocol_model_prefixes.get(protocol);
            if (limited_ids.is_some() || limited_prefixes.is_some())
                && !limited_ids.is_some_and(|ids| ids.iter().any(|id| id == model))
                && !limited_prefixes.is_some_and(|prefixes| prefixes.iter().any(|prefix| !prefix.is_empty() && model.starts_with(prefix))) {
                continue;
            }
            // Documentation alone cannot create an executor that this product
            // does not implement for the task.
            if crate::protocol_task_descriptor(protocol, task).is_none()
                || candidates.iter().any(|candidate: &DocumentedProtocolCandidate| candidate.protocol == *protocol) { continue; }
            let provider_params = if task == ModelTask::Chat {
                let original = entry.provider_params.as_object().expect("documented parameters must be an object");
                let conversion = crate::transition_chat_provider_params(original,
                    entry.protocols.first().map(String::as_str).unwrap_or(protocol), protocol);
                // Recommendations are optional defaults, never user-authored
                // constraints. An unmappable recommendation belongs only to
                // its original dialect; do not inject it into another one.
                if conversion.issues.is_empty() { Value::Object(conversion.params) } else { empty_params() }
            } else { entry.provider_params.clone() };
            candidates.push(DocumentedProtocolCandidate {
                protocol: protocol.clone(), connection_role: entry.connection_role.clone(),
                base_url_override: entry.protocol_base_url_overrides.get(protocol).cloned().or_else(|| entry.base_url_override.clone()), provider_params,
                endpoint: entry.endpoint.clone(), poll_endpoint: entry.poll_endpoint.clone(),
                content_endpoint: entry.content_endpoint.clone(), realtime_endpoint: entry.realtime_endpoint.clone(),
                sources: if entry.sources.is_empty() { profile.sources.clone() } else { entry.sources.clone() },
                checked_at: profile.checked_at.clone(), catalog_required: !exact || entry.status == DocumentationStatus::CatalogRequired,
            });
        }
    }
    candidates
}

/// Only exact documented selectors can supplement a catalog's missing tasks.
/// A platform-level API or an upstream model's name is insufficient evidence.
pub fn documented_model_tasks(platform: &str, model: &str) -> Vec<ModelTask> {
    let Some(profile) = provider_protocol_audit(platform) else { return Vec::new() };
    profile.tasks.iter().filter(|task| task.status == DocumentationStatus::Supported
        && model_matches(platform, model.trim(), task)
        && task.protocols.iter().any(|protocol| crate::protocol_task_descriptor(protocol, task.task).is_some()))
        .map(|task| task.task).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn official_origins_require_exact_documented_scheme_host_and_port() {
        for root in ["https://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/",
            "https://dashscope-us.aliyuncs.com/api/v1",
            "https://cn-hongkong.dashscope.aliyuncs.com"] {
            assert!(is_official_provider_origin("dashscope", root), "{root}");
        }
        assert!(is_official_provider_origin("zhipu", "https://open.bigmodel.cn/api/v1"));
        for root in ["https://relay.example/compatible-mode/v1", "https://dashscope-intl.aliyuncs.com.relay.example/v1",
            "https://dashscope-intl.aliyuncs.com:8443/v1", "http://dashscope-intl.aliyuncs.com/v1",
            "https://unknown-workspace.ap-southeast-1.maas.aliyuncs.com/v1", "not a URL"] {
            assert!(!is_official_provider_origin("dashscope", root), "{root}");
        }
        assert!(!is_official_provider_origin("custom", "https://dashscope-intl.aliyuncs.com/v1"));
    }

    #[test]
    fn native_catalog_tags_cannot_bypass_per_protocol_model_restrictions() {
        for catalog in [false,true] {
            let step = documented_protocol_candidates("stepfun","step-3.5-flash",ModelTask::Chat,catalog);
            assert_eq!(step.iter().map(|candidate| candidate.protocol.as_str()).collect::<Vec<_>>(),["openai.chat_text"]);
            let scope = documented_protocol_candidates("modelscope","Other/new-chat",ModelTask::Chat,catalog);
            assert!(!scope.iter().any(|candidate| candidate.protocol == "openai.responses"));
            let gemini = documented_protocol_candidates("gemini","gemini-nano-banana-2.1",ModelTask::ImageGeneration,catalog);
            assert_eq!(gemini.iter().map(|candidate| candidate.protocol.as_str()).collect::<Vec<_>>(),["gemini.interactions_images"]);
        }
        assert!(documented_protocol_candidates("deepgram","flux-general-en",ModelTask::SpeechRecognition,true).is_empty());
        assert!(documented_protocol_candidates("hunyuan","kinfra-vl-embedding-2b",ModelTask::Embedding,true).is_empty());
    }

    #[test]
    fn exact_documents_supplement_media_tasks_without_guessing_unknown_names() {
        assert!(documented_model_tasks("gemini","gemini-embedding-001").contains(&ModelTask::Embedding));
        assert!(documented_model_tasks("minimax","asr-1.0").contains(&ModelTask::SpeechRecognition));
        assert!(documented_model_tasks("openai","future-image-name").is_empty());
        assert!(documented_protocol_candidates("stepfun","step-2x-large",ModelTask::ImageGeneration,true).is_empty());
        assert!(documented_protocol_candidates("lingyi","yi-lightning",ModelTask::Chat,true).is_empty());
    }

    #[test]
    fn chat_defaults_follow_the_candidate_dialect_and_gateway_contract() {
        let router = documented_protocol_candidates("openrouter","vendor/current-model",ModelTask::Chat,false);
        assert_eq!(router[0].protocol,"openai.responses");
        assert_eq!(router[0].provider_params["chain_rounds"],false);
        assert!(router.iter().find(|candidate| candidate.protocol == "openai.chat_text").unwrap().provider_params.get("chain_rounds").is_none());
        let scope = documented_protocol_candidates("modelscope","Qwen/Qwen3.5-397B-A17B",ModelTask::Chat,false);
        assert_eq!(scope[0].protocol,"openai.responses");
        let ppio = documented_protocol_candidates("ppio","current-catalog-model",ModelTask::Chat,false);
        assert_eq!(ppio[0].protocol,"openai.responses");
    }

    #[test]
    fn every_preset_has_an_explicit_complete_protocol_audit() {
        let mut platforms = BTreeSet::new();
        for audit in provider_protocol_audits() {
            assert!(!audit.sources.is_empty());
            assert!(audit.sources.iter().all(|url| url.starts_with("https://")));
            for origin in &audit.official_origins {
                let url = reqwest::Url::parse(origin).expect("documented official origin URL");
                assert_eq!(url.scheme(), "https");
                assert_eq!(url.origin().ascii_serialization(), *origin, "official origin cannot contain a path or credentials");
            }
            assert!(!audit.notes.trim().is_empty());
            assert_eq!(audit.checked_at,"2026-10-16");
            assert_eq!(audit.tasks.len(),crate::ALL_MODEL_TASKS.len(),"{:?}",audit.platforms);
            let mut tasks = BTreeSet::new();
            for task in &audit.tasks {
                assert!(tasks.insert(serde_json::to_string(&task.task).unwrap()),"duplicate task");
                assert!(!task.notes.trim().is_empty());
                assert!(task.provider_params.is_object());
                if matches!(task.status,DocumentationStatus::Supported | DocumentationStatus::CatalogRequired) {
                    assert!(!task.protocols.is_empty());
                } else { assert!(task.protocols.is_empty(),"unimplemented protocols cannot be advertised"); }
                for protocol in &task.protocols {
                    assert!(crate::protocol_task_descriptor(protocol,task.task).is_some(),"{protocol}");
                }
            }
            for platform in &audit.platforms { assert!(platforms.insert(platform.clone()),"duplicate platform {platform}"); }
        }
        for preset in crate::platform_presets() { assert!(platforms.contains(&preset.platform),"missing audit {}",preset.platform); }
    }
}
