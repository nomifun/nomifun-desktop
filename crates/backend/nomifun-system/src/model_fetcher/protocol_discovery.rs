//! Configuration-only protocol discovery; never writes models or Agent data.
use std::time::{Duration, Instant};
use nomi_config::compat::ProviderCompat;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason};
use nomi_types::tool::ToolDef;
use nomifun_api_types::{BedrockAuthMethod, BedrockConfig, DiscoverModelProtocolsAnonymousRequest,
    DiscoverModelProtocolsRequest, DiscoverModelProtocolsResponse, ModelProtocolCandidate as Candidate,
    ModelProtocolCandidateEvidence as Evidence, ModelProtocolCandidateStatus as Status,
    ModelTask, ModelTaskSource, ProtocolScope, ProviderHealthCheckErrorKind as ErrorKind, ProviderModelCapabilityInput};
use nomifun_common::{AppError, ProviderId};
use nomifun_model_invoke::{AuthMaterial, AuthScheme, ResolvedConnection, protocol_task_descriptor,
    protocol_descriptor, protocol_manifest_for_model_connection, resolve_submit_url};
use nomifun_model_invoke::provider_protocol_audit::{DocumentedProtocolCandidate, DocumentationStatus, documented_protocol_candidates};
use serde_json::{Value,json};
use super::{FetchConfig, ModelFetchService};

const RESPONSES: &str = "openai.responses";
const CHAT: &str = "openai.chat_text";
const CHAT_PROTOCOLS: [&str;5] = [RESPONSES,CHAT,"anthropic.messages","gemini.generate_text","bedrock.anthropic_messages"];
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
const CHAT_DISCOVERY_BUDGET: Duration = Duration::from_secs(60);
const PROBE_OUTPUT_LIMIT: u32 = 4096;
const ECHO_TOOL: &str = "nomifun_protocol_echo";

impl ModelFetchService {
    pub async fn discover_model_protocols(&self, provider_id: &str, req: &DiscoverModelProtocolsRequest,
        connections: &crate::provider_connection::ProviderConnectionService) -> Result<DiscoverModelProtocolsResponse, AppError> {
        ProviderId::parse(provider_id).map_err(|error| AppError::BadRequest(error.to_string()))?;
        let row = self.repo.find_by_id(provider_id).await?.ok_or_else(|| AppError::NotFound("provider not found".into()))?;
        let revision = row.config_revision;
        let config = self.load_provider_config(provider_id).await?;
        let mut resolved = vec![ResolvedConnection {role:"default".into(),base_url:config.base_url.clone(),auth:config.auth.clone(),extra:Value::Null}];
        for connection in connections.list(provider_id).await? {
            resolved.push(connections.resolve_detection_connection(provider_id,&connection.role).await?);
        }
        let mut result = self.discover_with_config(&config,&req.model,&req.capability,req.preferred_protocol.as_deref(),resolved,req.apply_recommended_defaults).await?;
        if self.repo.find_by_id(provider_id).await?.is_none_or(|provider| provider.config_revision != revision) {
            return Err(AppError::Conflict("provider configuration changed during protocol discovery; retry with the current draft".into()));
        }
        result.config_revision = Some(revision);
        Ok(result)
    }

    pub async fn discover_model_protocols_anonymous(&self, req: &DiscoverModelProtocolsAnonymousRequest) -> Result<DiscoverModelProtocolsResponse, AppError> {
        if crate::provider::is_retired_provider_platform(req.platform.trim()) { return Err(AppError::Forbidden("retired provider cannot be tested".into())); }
        crate::provider::validate_provider_base_url(&req.platform,&req.base_url)?;
        let scheme = crate::provider::validate_provider_auth(&req.platform,&req.auth_scheme,&req.credentials,req.bedrock_config.as_ref())?;
        let config = FetchConfig {platform:req.platform.clone(),base_url:req.base_url.clone(),
            auth:AuthMaterial {scheme:AuthScheme::parse(&scheme).map_err(|error| AppError::BadRequest(error.message))?,credentials:req.credentials.clone()},
            bedrock_config:req.bedrock_config.clone()};
        let mut resolved = vec![ResolvedConnection {role:"default".into(),base_url:config.base_url.clone(),auth:config.auth.clone(),extra:Value::Null}];
        for connection in &req.connections {
            if connection.role.trim().is_empty() || resolved.iter().any(|existing| existing.role == connection.role) {
                return Err(AppError::BadRequest("duplicate or empty connection role".into()));
            }
            crate::provider_connection::validate_role(&connection.role)?;
            let auth = AuthMaterial {scheme:AuthScheme::parse(&connection.auth_scheme).map_err(|error| AppError::BadRequest(error.message))?,credentials:connection.credentials.clone()};
            auth.validate_credentials().map_err(|error| AppError::BadRequest(error.message))?;
            resolved.push(ResolvedConnection {role:connection.role.clone(),base_url:connection.base_url.clone(),auth,extra:connection.extra.clone().unwrap_or(Value::Null)});
        }
        self.discover_with_config(&config,&req.model,&req.capability,req.preferred_protocol.as_deref(),resolved,req.apply_recommended_defaults).await
    }

    async fn discover_with_config(&self, config: &FetchConfig, model: &str, original: &ProviderModelCapabilityInput,
        preferred: Option<&str>, mut connections: Vec<ResolvedConnection>, apply_recommended_defaults: bool) -> Result<DiscoverModelProtocolsResponse, AppError> {
        if model.trim().is_empty() { return Err(AppError::BadRequest("protocol discovery requires an exact model".into())); }
        if !connections.iter().any(|connection| connection.role == original.connection_role) {
            return Err(AppError::BadRequest("selected named connection must be supplied before protocol discovery".into()));
        }
        connections.sort_by_key(|connection| (connection.role != original.connection_role,connection.role != "default",connection.role.clone()));
        let started = Instant::now();
        let selected_connection = connections.iter().find(|connection| connection.role == original.connection_role).expect("checked connection");
        let manifest = protocol_manifest_for_model_connection(&config.platform,Some(&selected_connection.base_url),Some(model),original.task);
        let platform = manifest.platform.as_str();
        let retired_note = nomifun_model_invoke::provider_protocol_audit::provider_protocol_audit(platform)
            .and_then(|audit| audit.tasks.iter().find(|entry| entry.task == original.task && entry.status == DocumentationStatus::Retired))
            .map(|entry| entry.notes.as_str());
        // Catalog reads never submit generation jobs. Inferred names and client
        // supplied task hints are not proof of a provider's task declaration.
        // Read the selected connection, not a different account's default
        // connection; the evidence is usable only for that exact role/root.
        let catalog_declared = if original.task == ModelTask::Chat
            || (retired_note.is_some() && official_origin(platform,selected_connection))
            // The gateway catalog endpoint declares Bearer auth. Do not feed
            // a native header-key connection into its Bearer-only fetcher.
            || (platform == crate::model_gateway::PLATFORM && selected_connection.auth.scheme != AuthScheme::Bearer) { false } else {
            if selected_connection.auth.scheme != AuthScheme::Bedrock {
                crate::provider_model::validate_base_url(&selected_connection.base_url)?;
            }
            let catalog_config = FetchConfig {platform:platform.into(),base_url:selected_connection.base_url.clone(),
                auth:selected_connection.auth.clone(),bedrock_config:config.bedrock_config.clone()};
            match tokio::time::timeout(ATTEMPT_TIMEOUT,self.fetch_with_config(&catalog_config,false)).await {
                Ok(Ok(catalog)) => catalog.models.iter().any(|entry| entry.id == model.trim()
                    && entry.tasks_source == Some(ModelTaskSource::ProviderDeclared) && entry.tasks.contains(&original.task)),
                _ => false,
            }
        };
        let documents = documented_protocol_candidates(platform,model,original.task,catalog_declared);
        let recommended = manifest.recommendation.as_ref().map(|recommendation| recommendation.protocol_id.as_str());
        let mut protocols: Vec<String> = manifest.protocols.iter().filter(|descriptor| {
            if original.task != ModelTask::Chat {
                return descriptor.protocol_id == original.protocol
                    || documents.iter().any(|document| document.protocol == descriptor.protocol_id);
            }
            CHAT_PROTOCOLS.contains(&descriptor.protocol_id.as_str()) && (matches!(descriptor.protocol_id.as_str(),CHAT|RESPONSES)
                || descriptor.platforms.iter().any(|candidate| candidate == platform) || descriptor.protocol_id == original.protocol
                || (matches!(platform,"custom"|"new-api") && descriptor.scopes.contains(&ProtocolScope::Custom)))
        }).filter(|descriptor| connections.iter().any(|connection| auth_matches(&descriptor.protocol_id,connection)))
            .map(|descriptor| descriptor.protocol_id.clone()).collect();
        for document in &documents { if !protocols.contains(&document.protocol) { protocols.push(document.protocol.clone()); } }
        protocols.sort_by_key(|protocol| (Some(protocol.as_str()) != recommended,protocol != &original.protocol,
            CHAT_PROTOCOLS.iter().position(|candidate| candidate == protocol).unwrap_or(usize::MAX),protocol.clone()));
        protocols.dedup();
        let mut result = DiscoverModelProtocolsResponse {selected_protocol:None,selected_capability:None,candidates:Vec::new(),elapsed_ms:0,config_revision:None};
        let mut stopped = false;
        for protocol in protocols {
            let attempt_started = Instant::now();
            let document = documents.iter().find(|document| document.protocol == protocol);
            let connection = connections.iter().find(|connection| document.and_then(|document| document.connection_role.as_deref())
                .is_none_or(|role| role == connection.role) && auth_matches(&protocol,connection));
            let mut candidate = empty_candidate(&protocol,Some(protocol.as_str()) == recommended);
            let Some(connection) = connection else {
                candidate.status = Status::Blocked;
                candidate.message = Some("the protocol requires a configured connection with its native authentication".into());
                result.candidates.push(candidate); continue;
            };
            if let Some(note) = retired_note.filter(|_| official_origin(platform,connection)) {
                candidate.status = Status::Blocked;
                candidate.message = Some(format!("the provider's official endpoint has retired this task; no verification request was sent: {note}"));
                result.candidates.push(candidate); continue;
            }
            let prepared = prepare_candidate(platform,model,original,&protocol,connection,document,apply_recommended_defaults);
            let (capability,output_limit) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    candidate.status = Status::Blocked;
                    candidate.error_kind = Some(ErrorKind::InvalidRequest);
                    candidate.message = Some(nomifun_net::secret_redaction::SecretRedactor::new(connection.auth.secrets()).redact(&error.to_string()));
                    result.candidates.push(candidate); continue;
                }
            };
            candidate.capability = Some(capability.clone());
            if original.task != ModelTask::Chat {
                if let Some(document) = document {
                    if document.catalog_required && !catalog_scope_matches(selected_connection,connection,&capability) {
                        candidate.message = Some("catalog evidence belongs to a different connection or base URL; this candidate remains unverified and no generation request was performed".into());
                    } else {
                        candidate.status = Status::Available;
                        candidate.evidence = if document.catalog_required {Evidence::CatalogTask} else {Evidence::OfficialModel};
                        candidate.message = Some("documented configuration support; no generation request was performed".into());
                    }
                } else { candidate.message = Some("registered serializer only; model support is unverified and no generation request was performed".into()); }
            } else if stopped {
                candidate.status = Status::Blocked;
                candidate.message = Some("additional billable checks stopped after an authentication, quota or rate limit failure".into());
            } else if started.elapsed() >= CHAT_DISCOVERY_BUDGET {
                candidate.status = Status::Blocked;
                candidate.message = Some("the bounded verification budget was exhausted; this candidate was not checked".into());
            } else {
                candidate.probe_output_limit = Some(output_limit);
                let remaining = CHAT_DISCOVERY_BUDGET.saturating_sub(started.elapsed()).min(ATTEMPT_TIMEOUT);
                match tokio::time::timeout(remaining,check_candidate(model,&capability,connection,config.bedrock_config.as_ref(),output_limit)).await {
                    Ok(Ok(())) => { candidate.status = Status::Available; candidate.evidence = Evidence::VerifiedStream; }
                    Ok(Err(error)) => classify_error(&mut candidate,error,&connection.auth),
                    Err(_) => { candidate.error_kind = Some(ErrorKind::Timeout); candidate.message = Some("verification timed out; protocol support remains inconclusive".into()); }
                }
                stopped = stops_billable_checks(&candidate);
            }
            candidate.elapsed_ms = millis(attempt_started);
            result.candidates.push(candidate);
        }
        let available = |candidate: &&Candidate| candidate.status == Status::Available && candidate.capability.is_some();
        let chosen = preferred.and_then(|preferred| result.candidates.iter().filter(available).find(|candidate| candidate.protocol == preferred.trim()))
            .or_else(|| result.candidates.iter().filter(available).find(|candidate| candidate.recommended))
            .or_else(|| result.candidates.iter().find(|candidate| candidate.status == Status::Available && candidate.capability.is_some()));
        if let Some(chosen) = chosen { result.selected_protocol = Some(chosen.protocol.clone()); result.selected_capability = chosen.capability.clone(); }
        result.elapsed_ms = millis(started);
        Ok(result)
    }
}

fn stops_billable_checks(candidate: &Candidate) -> bool {
    matches!(candidate.error_kind,Some(ErrorKind::Unauthorized|ErrorKind::Forbidden|ErrorKind::InsufficientQuota|ErrorKind::RateLimited))
        || candidate.http_status.is_some_and(|status| matches!(status,401|402|403|429))
}

fn official_origin(platform: &str, connection: &ResolvedConnection) -> bool {
    nomifun_model_invoke::provider_protocol_audit::is_official_provider_origin(platform,&connection.base_url)
}

fn catalog_scope_matches(catalog: &ResolvedConnection, candidate: &ResolvedConnection, capability: &ProviderModelCapabilityInput) -> bool {
    fn root(value: &str) -> Option<String> {
        let mut url = reqwest::Url::parse(value.trim()).ok()?;
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
        Some(url.to_string())
    }
    let effective_root = capability.base_url_override.as_deref().unwrap_or(&candidate.base_url);
    catalog.role == candidate.role && root(&catalog.base_url).zip(root(effective_root)).is_some_and(|(catalog,candidate)| catalog == candidate)
}

fn auth_scheme(auth: &AuthMaterial) -> String {
    match &auth.scheme {AuthScheme::Bearer => "bearer".into(),AuthScheme::TokenHeader => "token".into(),
        AuthScheme::HeaderKey(name) => format!("header_key:{name}"),AuthScheme::QueryKey(name) => format!("query_key:{name}"),
        AuthScheme::MultiHeader(_) => "volc_voice".into(),AuthScheme::Bedrock => "bedrock".into()}
}
fn auth_matches(protocol: &str, connection: &ResolvedConnection) -> bool {
    let Some(descriptor) = protocol_descriptor(protocol) else {return false;};
    let scheme = auth_scheme(&connection.auth).to_ascii_lowercase();
    descriptor.allowed_auth_schemes.iter().any(|allowed| allowed.eq_ignore_ascii_case(&scheme)
        || (allowed == "header_key:<name>" && scheme.starts_with("header_key:"))
        || (allowed == "query_key:<param>" && scheme.starts_with("query_key:")))
}
fn empty_candidate(protocol: &str, recommended: bool) -> Candidate {
    Candidate {protocol:protocol.into(),capability:None,status:Status::Inconclusive,evidence:Evidence::RegisteredOnly,recommended,
        probe_output_limit:None,message:None,error_kind:None,http_status:None,elapsed_ms:0}
}

fn prepare_candidate(platform: &str, model: &str, original: &ProviderModelCapabilityInput, protocol: &str,
    connection: &ResolvedConnection, document: Option<&DocumentedProtocolCandidate>, apply_recommended_defaults: bool) -> Result<(ProviderModelCapabilityInput,u32),AppError> {
    let mut candidate = candidate_capability(original,protocol)?;
    candidate.connection_role = connection.role.clone();
    if original.protocol.trim() != protocol && original.base_url_override.as_deref().is_some_and(|root|
        Some(root) == nomifun_model_invoke::model_profiles::documented_chat_base_url(platform,Some(model))) {
        candidate.base_url_override = None;
    }
    if original.base_url_override.is_none() {
        let manifest = protocol_manifest_for_model_connection(platform,Some(&connection.base_url),Some(model),original.task);
        if let Some(recommendation) = manifest.recommendation.filter(|recommendation| recommendation.protocol_id == protocol && recommendation.base_url_override_required) {
            // The manifest knows official regional origins. This applies also
            // when a fresh draft already names its native protocol, and must
            // win over a regionless document's domestic default.
            candidate.base_url_override = recommendation.default_base_url;
        } else if official_origin(platform,connection) {
            if let Some(root) = document.and_then(|document| document.base_url_override.as_deref()) {
                if reqwest::Url::parse(root).ok().zip(reqwest::Url::parse(&connection.base_url).ok())
                    .is_some_and(|(document,current)| document.origin() == current.origin()) {
                    candidate.base_url_override = Some(root.into());
                }
            }
        }
    }
    if let Some(document) = document {
        if original.endpoint.is_none() {candidate.endpoint = document.endpoint.clone();}
        if original.poll_endpoint.is_none() {candidate.poll_endpoint = document.poll_endpoint.clone();}
        if original.content_endpoint.is_none() {candidate.content_endpoint = document.content_endpoint.clone();}
        if original.realtime_endpoint.is_none() {candidate.realtime_endpoint = document.realtime_endpoint.clone();}
        if apply_recommended_defaults {
            let params = candidate.provider_params.as_object_mut().ok_or_else(|| AppError::BadRequest("provider_params must be an object".into()))?;
            if let Some(defaults) = document.provider_params.as_object() { for (key,value) in defaults { params.entry(key.clone()).or_insert_with(|| value.clone()); } }
        }
    }
    connection.auth.validate_credentials().map_err(|error| AppError::BadRequest(error.message))?;
    crate::provider_model::validate_known_provider_model_task(platform,model,candidate.task)?;
    crate::provider_model::validate_protocol(platform,&candidate)?;
    crate::provider_model::validate_capability_auth_scheme(&candidate,&auth_scheme(&connection.auth))?;
    crate::provider_model::validate_provider_params(protocol,original.task,&candidate.provider_params)?;
    crate::provider_model::validate_capability_urls(&candidate,&connection.base_url)?;
    crate::provider_model::validate_positive_token_limit("context_limit",candidate.context_limit)?;
    crate::provider_model::validate_positive_token_limit("output_limit",candidate.output_limit)?;
    crate::provider_model::validate_compaction_threshold(&candidate)?;
    if nomifun_model_invoke::protocol_requires_output_ceiling(protocol) && candidate.output_limit.is_none() {
        return Err(AppError::BadRequest("this native Chat protocol requires an explicit output_limit before Agent verification".into()));
    }
    let output_limit = candidate.output_limit.map(|limit| u32::try_from(limit).ok().filter(|limit| *limit > 0)
        .ok_or_else(|| AppError::BadRequest("output_limit must be positive and fit in u32".into())))
        .transpose()?.unwrap_or(PROBE_OUTPUT_LIMIT).min(PROBE_OUTPUT_LIMIT);
    Ok((candidate,output_limit))
}

fn candidate_capability(original: &ProviderModelCapabilityInput, protocol: &str) -> Result<ProviderModelCapabilityInput,AppError> {
    let mut candidate = original.clone(); candidate.protocol = protocol.into();
    let params = original.provider_params.as_object().ok_or_else(|| AppError::BadRequest("provider_params must be an object".into()))?;
    if original.task == ModelTask::Chat && matches!(original.protocol.trim(),CHAT|RESPONSES) && matches!(protocol,CHAT|RESPONSES) {
        let transition = nomifun_model_invoke::transition_chat_provider_params(params,&original.protocol,protocol);
        if !transition.issues.is_empty() {
            let issues = transition.issues.iter().map(|issue| format!("provider_params.{} ({:?})",issue.parameter,issue.kind)).collect::<Vec<_>>().join(", ");
            return Err(AppError::BadRequest(format!("protocol parameters require a choice before verification: {issues}")));
        }
        candidate.provider_params = Value::Object(transition.params);
    } else if original.protocol.trim() != protocol && params.keys().any(|key|
        key != "_nomifun_context_limit_kind" && key != nomifun_api_types::MODEL_GATEWAY_CATALOG_BASELINE_PARAM) {
        return Err(AppError::BadRequest("native protocol parameters cannot be automatically reinterpreted; configure the target protocol explicitly".into()));
    }
    if original.protocol.trim() == protocol { return Ok(candidate); }
    if let Some(endpoint) = original.endpoint.as_deref() {
        if matches!(original.protocol.trim(),CHAT|RESPONSES) && matches!(protocol,CHAT|RESPONSES) {
            let suffix_start = endpoint.find(['?','#']).unwrap_or(endpoint.len());
            let (path,tail) = endpoint.split_at(suffix_start);
            let (old,new) = if protocol == CHAT {("/responses","/chat/completions")} else {("/chat/completions","/responses")};
            let path = path.trim_end_matches('/');
            let converted = if path == old.trim_start_matches('/') {new.trim_start_matches('/').to_owned()}
                else if let Some(root) = path.strip_suffix(old) {format!("{root}{new}")}
                else {return Err(AppError::BadRequest("custom endpoint cannot be safely converted to another protocol".into()));};
            candidate.endpoint = Some(format!("{converted}{tail}"));
        } else { return Err(AppError::BadRequest("custom endpoint cannot be safely converted to another protocol".into())); }
    }
    if original.poll_endpoint.is_some() || original.content_endpoint.is_some() || original.realtime_endpoint.is_some() {
        return Err(AppError::BadRequest("custom task endpoints must be configured explicitly for another protocol".into()));
    }
    Ok(candidate)
}

fn make_provider(model: &str, capability: &ProviderModelCapabilityInput, connection: &ResolvedConnection,
    bedrock: Option<&BedrockConfig>) -> Result<Box<dyn LlmProvider>,ProviderError> {
    let mut compat = match capability.protocol.as_str() {RESPONSES => ProviderCompat::openai_responses_defaults(),CHAT => ProviderCompat::openai_defaults(),
        "anthropic.messages" => ProviderCompat::anthropic_defaults(),"gemini.generate_text" => ProviderCompat::gemini_defaults(),
        "bedrock.anthropic_messages" => ProviderCompat::bedrock_defaults(),_ => return Err(ProviderError::Config("Chat protocol has no production verifier".into()))};
    let mut params = capability.provider_params.as_object().expect("validated object").clone();
    compat.max_tokens_field = params.remove("max_tokens_field").and_then(|value| value.as_str().map(str::trim).map(str::to_owned)).or(compat.max_tokens_field);
    compat.require_reasoning_content = params.remove("require_reasoning_content").and_then(|value| value.as_bool());
    compat.chain_rounds = params.remove("chain_rounds").and_then(|value| value.as_bool());
    compat.reasoning_effort = params.remove("reasoning_effort").and_then(|value| value.as_str().map(str::to_owned));
    params.remove("_nomifun_context_limit_kind"); params.remove(nomifun_api_types::MODEL_GATEWAY_CATALOG_BASELINE_PARAM);
    compat.api_path = Some(String::new()); compat.extra_body = Some(params);
    if capability.protocol == "bedrock.anthropic_messages" {
        let metadata = bedrock.ok_or_else(|| ProviderError::Config("Bedrock verification requires the production region and authentication configuration".into()))?;
        crate::provider::validate_provider_auth("bedrock","bedrock",&connection.auth.credentials,Some(metadata)).map_err(|error| ProviderError::Config(error.to_string()))?;
        let credentials = match metadata.auth_method {
            BedrockAuthMethod::AccessKey => nomi_providers::bedrock::AwsCredentials::Explicit {
                access_key_id:connection.auth.credentials["access_key_id"].as_str().expect("validated AWS key").trim().into(),
                secret_access_key:connection.auth.credentials["secret_access_key"].as_str().expect("validated AWS key").trim().into(),
                session_token:connection.auth.credentials.get("session_token").and_then(Value::as_str).map(str::trim).map(str::to_owned)},
            BedrockAuthMethod::Profile => nomi_providers::bedrock::AwsCredentials::Profile(metadata.profile.as_deref().unwrap_or_default().trim().into()),
            BedrockAuthMethod::DefaultChain => nomi_providers::bedrock::AwsCredentials::Environment,
        };
        return Ok(Box::new(nomi_providers::bedrock::BedrockProvider::new(metadata.region.trim(),credentials,false,compat)));
    }
    let mut connection = connection.clone();
    if let Some(root) = &capability.base_url_override {connection.base_url = root.clone();}
    let descriptor = protocol_task_descriptor(&capability.protocol,ModelTask::Chat).expect("validated Chat protocol");
    let endpoint = capability.endpoint.as_deref().unwrap_or_else(|| descriptor.endpoints.iter().find(|endpoint| endpoint.field == "endpoint").expect("HTTP endpoint").default_value.as_str());
    let url = resolve_submit_url(&connection,&capability.protocol,ModelTask::Chat,endpoint,model,capability.allow_cross_origin_credentials)
        .map_err(|error| ProviderError::Config(error.message))?;
    let keys = connection.auth.secrets().join("\n");
    Ok(match capability.protocol.as_str() {
        RESPONSES => Box::new(nomi_providers::openai_responses::OpenAIResponsesProvider::new(&keys,&url,compat)),
        CHAT => Box::new(nomi_providers::openai::OpenAIProvider::new(&keys,&url,compat)),
        "anthropic.messages" => Box::new(nomi_providers::anthropic::AnthropicProvider::new(&keys,&url,compat).with_cache(false)),
        "gemini.generate_text" => Box::new(nomi_providers::gemini::GeminiProvider::new(&keys,&url,compat)),
        _ => unreachable!(),
    })
}

async fn check_candidate(model: &str, capability: &ProviderModelCapabilityInput, connection: &ResolvedConnection,
    bedrock: Option<&BedrockConfig>, output_limit: u32) -> Result<(),ProviderError> {
    let provider = make_provider(model,capability,connection,bedrock)?;
    let mut request = LlmRequest {model:model.into(),system:"This is an isolated protocol verification. First call nomifun_protocol_echo once with value ok. After the tool result, return a short visible answer. Do not call any other tools.".into(),
        messages:vec![Message::new(Role::User,vec![ContentBlock::Text{text:"Verify the echo tool, then reply OK.".into()}])],
        tools:vec![ToolDef {name:ECHO_TOOL.into(),description:"A harmless local protocol test that returns the constant ok; no external side effects.".into(),
            input_schema:json!({"type":"object","properties":{"value":{"type":"string","enum":["ok"]}},"required":["value"],"additionalProperties":false}),deferred:false}],
        max_tokens:Some(output_limit),thinking:None,reasoning_effort:None,retain_provider_round:false};
    let first = collect_round(provider.as_ref(),&request,&capability.protocol).await?;
    let mut results = Vec::new();
    for block in &first {
        if let ContentBlock::ToolUse {id,name,input,..} = block {
            if name != ECHO_TOOL || input != &json!({"value":"ok"}) || !results.is_empty() {
                return Err(ProviderError::Config("model did not produce the bounded verification tool call; Agent support remains inconclusive".into()));
            }
            results.push(ContentBlock::ToolResult {tool_use_id:id.clone(),content:"{\"value\":\"ok\"}".into(),is_error:false,images:Vec::new()});
        }
    }
    if results.is_empty() {return Err(ProviderError::Config("text generation succeeded without the verification tool; Agent support remains inconclusive".into()));}
    request.messages.push(Message::new(Role::Assistant,first));
    request.messages.push(Message::new(Role::User,results));
    let final_blocks = collect_round(provider.as_ref(),&request,&capability.protocol).await?;
    if final_blocks.iter().any(|block| matches!(block,ContentBlock::ToolUse {..}))
        || !final_blocks.iter().any(|block| matches!(block,ContentBlock::Text {text} if !text.trim().is_empty())) {
        return Err(ProviderError::Config("tool verification completed without a final visible answer; Agent support remains inconclusive".into()));
    }
    Ok(())
}

async fn collect_round(provider: &dyn LlmProvider, request: &LlmRequest, protocol: &str) -> Result<Vec<ContentBlock>,ProviderError> {
    let mut stream = provider.stream(request).await?;
    let (mut text,mut thinking,mut signature,mut tools) = (String::new(),String::new(),None::<String>,Vec::new());
    let mut signed_blocks = Vec::new();
    let mut buffered_bytes = 0usize;
    let mut terminal = false;
    while let Some(event) = stream.recv().await {
        if terminal {return Err(ProviderError::Parse("verification stream emitted an event after its terminal result".into()));}
        match event {
            LlmEvent::TextDelta(delta) => {buffered_bytes += delta.len();text.push_str(&delta);}
            LlmEvent::ThinkingDelta(delta) => {
                if signature.is_some() {signed_blocks.push(ContentBlock::Thinking {thinking:std::mem::take(&mut thinking),signature:signature.take()});}
                buffered_bytes += delta.len();thinking.push_str(&delta);
            }
            LlmEvent::ThinkingSignature(value) => {
                buffered_bytes += value.len();
                if matches!(protocol,"anthropic.messages"|"bedrock.anthropic_messages") {signature.get_or_insert_default().push_str(&value);}
                else {signature = Some(value);}
            }
            LlmEvent::ToolUse {id,name,input,extra} => {
                buffered_bytes += input.to_string().len() + extra.as_ref().map_or(0,|value| value.to_string().len());
                tools.push(ContentBlock::ToolUse {id,name,input,extra});
            }
            LlmEvent::ToolUseTruncated {..} => return Err(ProviderError::Config("verification tool arguments were truncated; Agent support remains inconclusive".into())),
            LlmEvent::ProviderRoundId(_) => return Err(ProviderError::Config("verification received unexpected retained provider state".into())),
            LlmEvent::Done {stop_reason:StopReason::MaxTokens,..} => return Err(ProviderError::Config("verification output budget exhausted; protocol support remains inconclusive".into())),
            LlmEvent::Done {stop_reason:StopReason::EndTurn|StopReason::ToolUse,..} => {
                terminal = true;
            }
            LlmEvent::Done {..} => return Err(ProviderError::Config("verification ended without a normal terminal result".into())),
            LlmEvent::Error(message) => return Err(stream_error(message)),
            _ => {}
        }
        if buffered_bytes > 256 * 1024 || tools.len() > 1 {
            return Err(ProviderError::Config("verification exceeded its bounded response budget; Agent support remains inconclusive".into()));
        }
    }
    if terminal {
        let mut blocks = signed_blocks;
        if !thinking.is_empty() || signature.is_some() {blocks.push(ContentBlock::Thinking {thinking,signature});}
        if !text.is_empty() {blocks.push(ContentBlock::Text {text});}
        blocks.extend(tools); return Ok(blocks);
    }
    Err(ProviderError::Connection("verification stream closed without a terminal result".into()))
}

fn stream_error(message: String) -> ProviderError {
    if let Some(message) = message.strip_prefix("SSE parse error: ") {return ProviderError::Parse(message.into());}
    if let Some(rest) = message.strip_prefix("API error ") {if let Some((status,message)) = rest.split_once(": ") {
        if let Ok(status) = status.parse() {return ProviderError::Api {status,message:message.into()};}
    }}
    ProviderError::Connection(message)
}

fn classify_error(candidate: &mut Candidate, error: ProviderError, auth: &AuthMaterial) {
    candidate.error_kind = Some(ErrorKind::ApiError);
    match &error {
        ProviderError::Parse(_) => {candidate.status = Status::Incompatible;candidate.error_kind = Some(ErrorKind::InvalidRequest);}
        ProviderError::Api {status,message} => {
            candidate.http_status = Some(*status);
            candidate.error_kind = Some(match status {401 => ErrorKind::Unauthorized,402 => ErrorKind::InsufficientQuota,403 => ErrorKind::Forbidden,404 => ErrorKind::NotFound,
                429 => ErrorKind::RateLimited,status if *status >= 500 => ErrorKind::ApiError,_ => ErrorKind::InvalidRequest});
            let body = serde_json::Deserializer::from_str(message).into_iter::<Value>().next().and_then(Result::ok);
            let code = body.as_ref().and_then(|body| body.pointer("/error/code").or_else(|| body.get("code")).or_else(|| body.pointer("/error/type")))
                .and_then(Value::as_str).unwrap_or_default();
            if matches!(code,"invalid_api_key"|"invalid_access_token"|"authentication_error"|"unauthorized") {candidate.error_kind = Some(ErrorKind::Unauthorized);}
            else if matches!(code,"access_denied"|"forbidden"|"account_suspended") {candidate.error_kind = Some(ErrorKind::Forbidden);}
            else if matches!(code,"insufficient_quota"|"quota_exceeded"|"insufficient_balance") {candidate.error_kind = Some(ErrorKind::InsufficientQuota);}
            else if matches!(code,"model_not_found"|"model_not_available"|"model_not_supported")
                || body.as_ref().and_then(|body| body.pointer("/error/param")).and_then(Value::as_str) == Some("model") {candidate.error_kind = Some(ErrorKind::ModelUnavailable);}
            else if matches!(*status,405|410|501) || (*status == 404 && (matches!(code,"endpoint_not_found"|"route_not_found")
                || body.as_ref().and_then(|body| body.get("detail")).and_then(Value::as_str) == Some("Not Found")))
                || (matches!(*status,400|422) && matches!(code,"unsupported_protocol"|"unsupported_endpoint"|"unsupported_parameter"|"unknown_parameter"|"not_implemented")) {
                candidate.status = Status::Incompatible;
            }
        }
        ProviderError::RateLimited {..} => candidate.error_kind = Some(ErrorKind::RateLimited),
        ProviderError::Http(error) if error.is_timeout() => candidate.error_kind = Some(ErrorKind::Timeout),
        ProviderError::Connection(_) | ProviderError::Http(_) | ProviderError::StreamTruncated(_) => candidate.error_kind = Some(ErrorKind::ConnectionError),
        ProviderError::NonApiResponse {message,..} => {
            candidate.error_kind = Some(ErrorKind::NonApiResponse);
            // reject_non_api_response in the production provider library
            // formats this prefix from the actual HTTP status, never from the
            // response body. Preserve auth/rate stop authority even for HTML.
            candidate.http_status = message.strip_prefix("HTTP ").and_then(|message| message.split_once(" — "))
                .and_then(|(status,_)| status.parse::<u16>().ok()).filter(|status| (100..=599).contains(status));
            candidate.error_kind = Some(match candidate.http_status {
                Some(401) => ErrorKind::Unauthorized,Some(402) => ErrorKind::InsufficientQuota,
                Some(403) => ErrorKind::Forbidden,Some(429) => ErrorKind::RateLimited,_ => ErrorKind::NonApiResponse,
            });
        }
        ProviderError::Config(_) | ProviderError::PromptTooLong(_) => candidate.error_kind = Some(ErrorKind::InvalidRequest),
    }
    candidate.message = Some(if matches!(error,ProviderError::Http(_)) {"model verification transport failed".into()}
        else {nomifun_net::secret_redaction::SecretRedactor::new(auth.secrets()).redact(&error.to_string()).chars().take(350).collect()});
}
fn millis(started: Instant) -> u64 {started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)}

#[cfg(test)]
mod tests {
    use super::*;
    fn capability(task: ModelTask, protocol: &str) -> ProviderModelCapabilityInput {
        serde_json::from_value(json!({"task":task,"protocol":protocol,"connection_role":"default","provider_params":{}})).unwrap()
    }
    fn connection(root: &str, scheme: AuthScheme) -> ResolvedConnection {
        ResolvedConnection {role:"default".into(),base_url:root.into(),auth:AuthMaterial {scheme,credentials:json!({"api_keys":["fixture"]})},extra:Value::Null}
    }

    #[test]
    fn documented_endpoint_overrides_remain_exact_and_custom_roots_are_preserved() {
        let original = capability(ModelTask::Rerank,"generic.rerank");
        let connection = connection("https://dashscope.aliyuncs.com/compatible-api/v1",AuthScheme::Bearer);
        let document = DocumentedProtocolCandidate {
                protocol:"generic.rerank".into(),connection_role:None,base_url_override:None,
                endpoint:Some("/reranks".into()),poll_endpoint:None,content_endpoint:None,realtime_endpoint:None,
                provider_params:json!({}),sources:vec!["https://example.invalid/docs".into()],checked_at:"2026-10-15".into(),catalog_required:false,
            };
        let (candidate,_) = prepare_candidate("dashscope","qwen3-rerank",&original,"generic.rerank",&connection,Some(&document),true).unwrap();
        assert_eq!(candidate.endpoint.as_deref(),Some("/reranks"));
        assert!(candidate.base_url_override.is_none());
        let mut custom = original;
        custom.endpoint = Some("/tenant-rerank".into());
        let (candidate,_) = prepare_candidate("dashscope","qwen3-rerank",&custom,"generic.rerank",&connection,Some(&document),true).unwrap();
        assert_eq!(candidate.endpoint,custom.endpoint);
    }

    #[test]
    fn bedrock_verification_uses_typed_production_configuration_without_a_live_call() {
        let mut capability = capability(ModelTask::Chat,"bedrock.anthropic_messages");
        capability.output_limit = Some(4096);
        let connection = ResolvedConnection {role:"default".into(),base_url:String::new(),extra:Value::Null,
            auth:AuthMaterial {scheme:AuthScheme::Bedrock,credentials:json!({"access_key_id":"fixture-access","secret_access_key":"fixture-secret"})}};
        assert!(make_provider("anthropic.fixture",&capability,&connection,None).is_err());
        let metadata = BedrockConfig {auth_method:BedrockAuthMethod::AccessKey,region:"us-east-1".into(),profile:None};
        assert!(make_provider("anthropic.fixture",&capability,&connection,Some(&metadata)).is_ok());
        capability.output_limit = None;
        assert!(prepare_candidate("bedrock","anthropic.fixture",&capability,"bedrock.anthropic_messages",&connection,None,true).is_err());
    }

    #[test]
    fn screenshot_decoder_failure_is_incompatible_while_transport_failure_is_inconclusive() {
        let auth = AuthMaterial {scheme:AuthScheme::Bearer,credentials:json!({"api_keys":["fixture"]})};
        let mut parse = empty_candidate(RESPONSES,true);
        classify_error(&mut parse,stream_error("SSE parse error: Responses content part does not match its message item".into()),&auth);
        assert_eq!(parse.status,Status::Incompatible);
        let mut transport = empty_candidate(RESPONSES,true);
        classify_error(&mut transport,ProviderError::Connection("connection reset".into()),&auth);
        assert_eq!(transport.status,Status::Inconclusive);
        assert!(!stops_billable_checks(&transport));
    }

    #[test]
    fn global_stops_are_limited_to_authentication_quota_and_rate_failures() {
        for kind in [ErrorKind::Unauthorized,ErrorKind::Forbidden,ErrorKind::InsufficientQuota,ErrorKind::RateLimited] {
            let mut candidate=empty_candidate(RESPONSES,true);candidate.error_kind=Some(kind);
            assert!(stops_billable_checks(&candidate));
        }
        for kind in [ErrorKind::Timeout,ErrorKind::ConnectionError,ErrorKind::NonApiResponse,ErrorKind::InvalidRequest,ErrorKind::ApiError] {
            let mut candidate=empty_candidate(RESPONSES,true);candidate.error_kind=Some(kind);candidate.http_status=Some(503);
            assert!(!stops_billable_checks(&candidate));
        }
    }

    #[test]
    fn catalog_scope_does_not_transfer_between_roles_or_task_root_overrides() {
        let catalog=connection("https://relay.example/v1/",AuthScheme::Bearer);
        let mut target=catalog.clone();let mut capability=capability(ModelTask::ImageGeneration,"openai.images");
        assert!(catalog_scope_matches(&catalog,&target,&capability));
        target.role="media".into();assert!(!catalog_scope_matches(&catalog,&target,&capability));
        target.role="default".into();capability.base_url_override=Some("https://relay.example/other/v1".into());
        assert!(!catalog_scope_matches(&catalog,&target,&capability));
        capability.base_url_override=Some("https://another.example/v1".into());assert!(!catalog_scope_matches(&catalog,&target,&capability));
    }

    #[test]
    fn retired_vendor_origin_is_distinct_from_a_user_relay_without_any_network_check() {
        let profile=nomifun_model_invoke::provider_protocol_audit::provider_protocol_audit("lingyi").unwrap();
        assert!(profile.tasks.iter().any(|task| task.task==ModelTask::Chat && task.status==DocumentationStatus::Retired));
        let root=nomifun_model_invoke::platform_presets().into_iter().find(|preset| preset.platform=="lingyi")
            .and_then(|preset| preset.platform_default_base_url).unwrap();
        assert!(official_origin("lingyi",&connection(&root,AuthScheme::Bearer)));
        assert!(!official_origin("lingyi",&connection("https://relay.example/v1",AuthScheme::Bearer)));
    }

    #[test]
    fn native_dashscope_drafts_keep_their_official_region_and_never_redirect_a_relay() {
        let original=capability(ModelTask::ImageGeneration,"dashscope.images");
        let documents=documented_protocol_candidates("dashscope","qwen-image-3.0-pro",ModelTask::ImageGeneration,false);
        let document=documents.iter().find(|document| document.protocol=="dashscope.images").unwrap();
        for origin in ["https://dashscope.aliyuncs.com","https://dashscope-intl.aliyuncs.com", "https://dashscope-us.aliyuncs.com","https://cn-hongkong.dashscope.aliyuncs.com"] {
            let connection=connection(&format!("{origin}/compatible-mode/v1"),AuthScheme::Bearer);
            let (prepared,_)=prepare_candidate("dashscope","qwen-image-3.0-pro",&original,"dashscope.images",&connection,Some(document),false).unwrap();
            let root=reqwest::Url::parse(prepared.base_url_override.as_deref().unwrap()).unwrap();
            assert_eq!(root.origin().ascii_serialization(),origin);assert_eq!(root.path(),"/");
            assert_eq!(prepared.protocol,original.protocol,"the same-protocol fresh draft still needs its native origin");
        }
        let relay=connection("https://relay.example",AuthScheme::Bearer);
        let (prepared,_)=prepare_candidate("dashscope","qwen-image-3.0-pro",&original,"dashscope.images",&relay,Some(document),false).unwrap();
        assert!(prepared.base_url_override.is_none());
    }
}
