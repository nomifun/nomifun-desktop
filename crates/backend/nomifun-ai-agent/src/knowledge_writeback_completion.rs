//! One independently owned Knowledge extraction over the shared Chat transport.
//! No Session admission, model step, retry loop or detached stream reader lives here.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use nomifun_agent_contracts::{KnowledgeWritebackDiagnostic, KnowledgeWritebackStage as Stage, ModelFailureDiagnostic};
use nomifun_api_types::{AgentErrorCode, CapabilityHealth, ModelTask, ModelTechnicalCapability};
use nomifun_chat_model_broker::{ChatContentPart, ChatFinishReason, ChatMessage, ChatModelError, ChatModelEvent,
    ChatModelInput, ChatProtocol, ChatResponseFormat, ChatRole, ChatToolChoice, PromptCachePolicy,
    ProviderWireFrame, chat_protocol_for_id, configure_standalone_non_streaming, encode_standalone_chat_input,
    new_standalone_frame_decoder,chat_request_uses_reasoning};
use nomifun_knowledge::{KnowledgeWritebackCompletion, KnowledgeWritebackCompletionError,
    KnowledgeWritebackCompletionRequest, KnowledgeWritebackLimits};
use nomifun_model_invoke::{AuthMaterial, InvokeError, InvokeErrorKind, ModelRef,
    OpaqueCredentialLease, OpaqueCredentialResolver, ResolvedTaskConfig, SingleAttemptFraming,
    SingleAttemptHttpExecutor, SingleAttemptRequest};
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::knowledge_completer::LiveKnowledgeCompleter;

// These are the shared Chat attempt phase budgets. Continued semantic output
// renews only the idle phase; healthy work has no 45-second lifetime cutoff.
const PHASE_TIMEOUT: Duration = Duration::from_secs(120);

struct Observation {
    started: Instant,
    diagnostic: KnowledgeWritebackDiagnostic,
    semantic_bytes: usize,
    target: Option<(String,String)>,
}

impl Observation {
    fn new() -> Self {
        let mut diagnostic=KnowledgeWritebackDiagnostic::new(Stage::Resolve);
        diagnostic.received_frames=Some(0);diagnostic.received_bytes=Some(0);diagnostic.output_bytes=Some(0);
        Self { started:Instant::now(), diagnostic, semantic_bytes:0,target:None }
    }

    fn finish(mut self) -> KnowledgeWritebackDiagnostic {
        self.diagnostic.elapsed_ms = Some(self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        self.diagnostic
    }

    fn failure(mut self, stage: Stage, code: &'static str, provider: Option<ModelFailureDiagnostic>) -> KnowledgeWritebackCompletionError {
        self.diagnostic.stage = stage;
        let (global, detail) = match code {
            "CANCELLED" => (None,"Knowledge extraction was cancelled"),
            "REQUEST_SETUP_TIMEOUT" => (Some(AgentErrorCode::UserLlmProviderTimeout),"Provider request setup timed out"),
            "FIRST_FRAME_TIMEOUT" => (Some(AgentErrorCode::UserLlmProviderTimeout),"Provider did not produce the first semantic response in time"),
            "STREAM_IDLE_TIMEOUT" => (Some(AgentErrorCode::UserLlmProviderTimeout),"Provider stopped producing semantic response progress"),
            "INPUT_BYTE_BUDGET_EXCEEDED" => (Some(AgentErrorCode::UserLlmProviderContextTooLarge),"Knowledge extraction input exceeded its byte budget"),
            "INPUT_TOKEN_BUDGET_EXCEEDED" => (Some(AgentErrorCode::UserLlmProviderContextTooLarge),"Knowledge extraction input exceeded the available context budget"),
            "OUTPUT_BYTE_BUDGET_EXCEEDED" => (Some(AgentErrorCode::UserLlmProviderInvalidResponse),"Knowledge extraction exceeded its output byte budget"),
            "OUTPUT_TOKEN_BUDGET_EXCEEDED" => (Some(AgentErrorCode::UserLlmProviderInvalidResponse),"Knowledge extraction exceeded its output token budget"),
            "OUTPUT_ESTIMATE_PRESSURE_EXCEEDED" => (Some(AgentErrorCode::UserLlmProviderInvalidResponse),"Knowledge extraction exceeded its estimated output pressure guard; this estimate is not model tokenization"),
            "RESPONSE_BYTE_BUDGET_EXCEEDED" => (Some(AgentErrorCode::UserLlmProviderInvalidResponse),"Knowledge extraction exceeded its wire response byte budget"),
            "OUTPUT_TRUNCATED" => (Some(AgentErrorCode::UserLlmProviderInvalidResponse),"Provider ended extraction before completing its output"),
            "EMPTY_RESPONSE" => (Some(AgentErrorCode::UserLlmProviderEmptyResponse),"Provider completed extraction without visible output"),
            "TERMINAL_MISSING" => (Some(AgentErrorCode::UserLlmProviderStreamInterrupted),"Provider response ended without a verified terminal"),
            "PROVIDER_REFUSED" => (Some(AgentErrorCode::UserLlmProviderInvalidRequest),"Provider refused the extraction request"),
            "CONFIGURATION_CHANGED" => (Some(AgentErrorCode::NomifunSessionConfigurationChanged),"Knowledge model configuration differs from the original Turn"),
            "CONFIGURATION_INVALID" => (Some(AgentErrorCode::UserLlmProviderConfigError),"Knowledge model configuration is invalid"),
            "PROTOCOL_UNSUPPORTED" => (Some(AgentErrorCode::UserLlmProviderUnsupportedFeature),"Selected Chat protocol is unsupported"),
            "REASONING_UNSUPPORTED" => (Some(AgentErrorCode::UserLlmProviderUnsupportedFeature),"Configured reasoning is unsupported by the selected model"),
            "HTTP_CLIENT_UNAVAILABLE" | "REQUEST_BUDGET_INVALID" => (Some(AgentErrorCode::NomifunInternalError),"Knowledge extraction transport could not be prepared"),
            _ => (Some(AgentErrorCode::UserLlmProviderInvalidResponse),"Knowledge extraction returned an invalid response"),
        };
        self.diagnostic.code=global.map(code_text);
        self.diagnostic.detail=Some(detail.into());
        let provider=provider.map(|mut value| {
            if let Some((provider,model))=&self.target {
                value.provider_id.get_or_insert_with(||provider.clone());
                value.model_name.get_or_insert_with(||model.clone());
            }
            value
        });
        self.diagnostic.provider_diagnostic = provider.filter(|value|
            serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 8192));
        KnowledgeWritebackCompletionError { diagnostic:self.finish() }
    }

    fn invoke_failure(self, stage: Stage, error: InvokeError) -> KnowledgeWritebackCompletionError {
        if error.kind==InvokeErrorKind::Timeout {
            let code=match stage {Stage::Connect=>"REQUEST_SETUP_TIMEOUT",Stage::FirstFrame=>"FIRST_FRAME_TIMEOUT",_=>"STREAM_IDLE_TIMEOUT"};
            return self.failure(stage,code,error.diagnostic);
        }
        let code=match error.kind {
            InvokeErrorKind::Auth=>nomifun_chat_model_broker::ChatModelErrorCode::AuthenticationFailed,
            InvokeErrorKind::RateLimited | InvokeErrorKind::QuotaExhausted=>nomifun_chat_model_broker::ChatModelErrorCode::RateLimited,
            InvokeErrorKind::UnsupportedTask | InvokeErrorKind::NoAdapter=>nomifun_chat_model_broker::ChatModelErrorCode::UnsupportedFeature,
            InvokeErrorKind::Config | InvokeErrorKind::MissingConnection=>nomifun_chat_model_broker::ChatModelErrorCode::CredentialReferenceMissing,
            InvokeErrorKind::InvalidParams | InvokeErrorKind::ContentPolicy=>nomifun_chat_model_broker::ChatModelErrorCode::InvalidRequest,
            InvokeErrorKind::ParseError | InvokeErrorKind::NonApiResponse=>nomifun_chat_model_broker::ChatModelErrorCode::ProtocolViolation,
            _=>nomifun_chat_model_broker::ChatModelErrorCode::ProviderUnavailable,
        };
        let classified=super::protocol::send_error::AgentSendError::from_model_failure(code,error.diagnostic.as_ref());
        let mut failure=self.failure(stage,"PROVIDER_REQUEST_FAILED",error.diagnostic);
        failure.diagnostic.code=classified.code().map(code_text);
        failure.diagnostic.detail=Some(classified.stream_error().message.clone());
        failure
    }

    fn model_failure(self, stage:Stage, error: ChatModelError) -> KnowledgeWritebackCompletionError {
        let classified=super::protocol::send_error::AgentSendError::from_model_failure(error.code,error.diagnostic.as_ref());
        let mut failure=self.failure(stage,"RESPONSE_DECODE_FAILED",error.diagnostic);
        failure.diagnostic.code=classified.code().map(code_text);
        failure.diagnostic.detail=Some(classified.stream_error().message.clone());
        failure
    }
}

fn code_text(code: AgentErrorCode) -> String {
    serde_json::to_value(code).expect("typed Agent error code").as_str().expect("Agent error code is a string").to_owned()
}

fn http_client() -> Result<reqwest::Client,()> {
    static CLIENT: OnceLock<Result<reqwest::Client,()>>=OnceLock::new();
    CLIENT.get_or_init(||nomifun_net::http_client_no_redirect().map_err(|_|())).clone()
}

// The opaque lease grants one already-resolved request. Authentication stays
// inside the existing executor and is consumed before its response is returned.
struct OneRequestCredential {
    handle: String,
    material: Mutex<Option<AuthMaterial>>,
}

#[async_trait::async_trait]
impl OpaqueCredentialResolver for OneRequestCredential {
    async fn resolve(&self, lease: &OpaqueCredentialLease) -> Result<AuthMaterial, InvokeError> {
        if lease.handle() != self.handle { return Err(InvokeError::config("Knowledge credential target differs")); }
        self.material.lock().map_err(|_| InvokeError::config("Knowledge credential owner is unavailable"))?
            .take().ok_or_else(|| InvokeError::config("Knowledge credential lease was consumed"))
    }
}

fn estimate_tokens(bytes: usize) -> usize { bytes.saturating_add(2) / 3 }

async fn resolve_configuration(completer: &LiveKnowledgeCompleter, model: Option<&nomifun_common::ProviderWithModel>)
    -> Result<ResolvedTaskConfig, InvokeError> {
    let (provider_id, model) = match model {
        Some(model) => (model.provider_id.clone(), model.use_model.as_deref().unwrap_or(&model.model).to_owned()),
        None => completer.resolve_default_model().await.map_err(|_| InvokeError::config("Knowledge Chat model is unavailable"))?,
    };
    completer.model_invoke.resolve_task_config(&ModelRef {provider_id,model}, ModelTask::Chat).await
}

pub(super) async fn capture(completer: &LiveKnowledgeCompleter, model: Option<&nomifun_common::ProviderWithModel>)
    -> Result<Option<Arc<ResolvedTaskConfig>>,nomifun_common::AppError> {
    resolve_configuration(completer,model).await.map(|config|Some(Arc::new(config)))
        .map_err(|_| nomifun_common::AppError::Conflict("Knowledge Chat configuration could not be captured".into()))
}

async fn selected_configuration(completer: &LiveKnowledgeCompleter, request: &KnowledgeWritebackCompletionRequest<'_>)
    -> Result<(ResolvedTaskConfig, BTreeSet<ModelTechnicalCapability>), InvokeError> {
    let selection=request.model.cloned().or_else(||request.expected_configuration.map(|configuration|nomifun_common::ProviderWithModel {
        provider_id:configuration.provider_id.clone(),model:configuration.model.clone(),use_model:None,
    }));
    let config=resolve_configuration(completer,selection.as_ref()).await?;
    let capability = completer.model_invoke.provider_model_capability_repo()
        .get(&config.provider_id, &config.model, "chat").await
        .map_err(|_| InvokeError::config("Knowledge Chat capability is unavailable"))?
        .ok_or_else(|| InvokeError::config("Knowledge Chat capability is missing"))?;
    let unsupported = capability.health.as_deref().map(serde_json::from_str::<CapabilityHealth>).transpose()
        .map_err(|_| InvokeError::config("Knowledge Chat capability observation is invalid"))?
        .map(|health| health.unsupported_technical_capabilities.into_iter().collect()).unwrap_or_default();
    let final_config=resolve_configuration(completer,selection.as_ref()).await?;
    if !config.matches_invocation_configuration(&final_config) {
        return Err(InvokeError::config("Knowledge model configuration changed during resolution"));
    }
    Ok((final_config, unsupported))
}

pub(super) async fn complete(completer: &LiveKnowledgeCompleter, request: KnowledgeWritebackCompletionRequest<'_>)
    -> Result<KnowledgeWritebackCompletion, KnowledgeWritebackCompletionError> {
    let mut observation = Observation::new();
    let cancellation = request.cancellation.cloned().unwrap_or_else(CancellationToken::new);
    let input_bytes = request.system.len().saturating_add(request.user.len());
    let input_tokens = estimate_tokens(input_bytes);
    observation.diagnostic.input_tokens_estimate = Some(input_tokens as u64);
    observation.diagnostic.output_token_limit=Some(request.limits.output_max_tokens);
    if input_bytes > request.limits.input_max_utf8_bytes { return Err(observation.failure(Stage::Validate,"INPUT_BYTE_BUDGET_EXCEEDED",None)); }
    if input_tokens > request.limits.input_max_tokens_estimate { return Err(observation.failure(Stage::Validate,"INPUT_TOKEN_BUDGET_EXCEEDED",None)); }
    let configuration = tokio::select! { biased;
        _ = cancellation.cancelled() => return Err(observation.failure(Stage::Cancelled,"CANCELLED",None)),
        result = selected_configuration(completer,&request) => result,
    };
    let (config, unsupported) = match configuration {
        Ok(configuration) => configuration,
        Err(error) => return Err(observation.invoke_failure(Stage::Resolve,error)),
    };
    if request.expected_configuration.is_some_and(|expected| !config.matches_invocation_configuration(expected)) {
        return Err(observation.failure(Stage::Resolve,"CONFIGURATION_CHANGED",None));
    }
    complete_resolved(config, unsupported, request, cancellation, observation).await
}

async fn complete_resolved(config: ResolvedTaskConfig, unsupported: BTreeSet<ModelTechnicalCapability>,
    request: KnowledgeWritebackCompletionRequest<'_>, cancellation: CancellationToken, mut observation: Observation)
    -> Result<KnowledgeWritebackCompletion, KnowledgeWritebackCompletionError> {
    observation.target=Some((config.provider_id.clone(),config.model.clone()));
    let input_tokens = estimate_tokens(request.system.len().saturating_add(request.user.len()));
    observation.diagnostic.input_tokens_estimate = Some(input_tokens as u64);
    let limits=match request.limits.for_configuration(&config) {
        Ok(limits)=>limits,
        Err(error)=>{
            let mut failure=observation.failure(Stage::Validate,"CONFIGURATION_INVALID",None);
            failure.diagnostic.detail=Some(error.to_string());
            return Err(failure);
        },
    };
    if limits.output_max_tokens==0 || limits.output_max_utf8_bytes==0 || limits.response_max_bytes==0 {
        return Err(observation.failure(Stage::Validate,"REQUEST_BUDGET_INVALID",None));
    }
    if request.system.len().saturating_add(request.user.len())>limits.input_max_utf8_bytes {
        return Err(observation.failure(Stage::Validate,"INPUT_BYTE_BUDGET_EXCEEDED",None));
    }
    if input_tokens>limits.input_max_tokens_estimate {
        return Err(observation.failure(Stage::Validate,"INPUT_TOKEN_BUDGET_EXCEEDED",None));
    }
    let local_output_limit=limits.output_max_tokens;
    let requested_output_limit=config.output_limit.map(|_|local_output_limit);
    observation.diagnostic.output_token_limit = Some(local_output_limit);
    let Some(protocol) = chat_protocol_for_id(&config.protocol) else { return Err(observation.failure(Stage::Resolve,"PROTOCOL_UNSUPPORTED",None)); };
    if config.output_limit.is_none() && nomifun_model_invoke::protocol_requires_output_ceiling(&config.protocol) {
        return Err(observation.failure(Stage::Validate,"CONFIGURATION_INVALID",None));
    }
    let input = ChatModelInput {
        instructions:vec![request.system.to_owned()],messages:vec![ChatMessage {role:ChatRole::User,
            content:vec![ChatContentPart::Text {text:request.user.to_owned()}],provider_round_id:None}],
        tools:Vec::new(),tool_choice:ChatToolChoice::None,parallel_tool_calls:None,max_output_tokens:requested_output_limit,
        reasoning:None,prompt_cache:PromptCachePolicy::Disabled,response_format:ChatResponseFormat::Text,
        requested_output_modalities:BTreeSet::new(),provider_round_parent:None,preserve_native_responses_items:false,metadata:BTreeMap::new(),
    };
    let mut body = match encode_standalone_chat_input(protocol,&config.model,&input,&config.provider_params,config.output_limit) {
        Ok(body) => body,
        Err(error) => return Err(observation.model_failure(Stage::Validate,error)),
    };
    if unsupported.contains(&ModelTechnicalCapability::Reasoning) && chat_request_uses_reasoning(protocol,&body) {
        return Err(observation.failure(Stage::Validate,"REASONING_UNSUPPORTED",None));
    }
    let region=bedrock_region(config.bedrock_config.as_deref());
    let endpoint=if protocol==ChatProtocol::Bedrock {
        region.as_deref().filter(|value|!value.trim().is_empty())
            .map(|region|format!("https://bedrock-runtime.{region}.amazonaws.com/model/{}/invoke-with-response-stream",config.model))
            .ok_or_else(||InvokeError::config("Bedrock signing region is missing"))
    } else {config.http_endpoint()};
    let mut url = match endpoint {
        Ok(url) => url,
        Err(error) => return Err(observation.invoke_failure(Stage::Resolve,error)),
    };
    let mut framing = if protocol == ChatProtocol::Bedrock {SingleAttemptFraming::AwsEventStream} else {SingleAttemptFraming::Sse};
    if unsupported.contains(&ModelTechnicalCapability::Streaming) {
        if let Err(error) = configure_standalone_non_streaming(protocol,&mut url,&mut body) { return Err(observation.model_failure(Stage::Validate,error)); }
        framing = SingleAttemptFraming::Sse;
    }
    let handle = format!("knowledge-writeback:{}",uuid::Uuid::now_v7());
    let lease = match OpaqueCredentialLease::new(handle.clone()) {
        Ok(lease) => lease,
        Err(error) => return Err(observation.invoke_failure(Stage::Resolve,error)),
    };
    let credential = Arc::new(OneRequestCredential {handle,material:Mutex::new(Some(config.connection.auth))});
    let http = match http_client() {
        Ok(http) => http,
        Err(_) => return Err(observation.failure(Stage::Connect,"HTTP_CLIENT_UNAVAILABLE",None)),
    };
    let response_budget=match nomifun_model_invoke::SingleAttemptResponseBudget::new(limits.response_max_bytes) {
        Ok(budget)=>budget,
        Err(error)=>return Err(observation.invoke_failure(Stage::Validate,error)),
    };
    let executor = SingleAttemptHttpExecutor::new(http,credential).with_response_budget(response_budget.clone());
    let opened = tokio::select! { biased;
        _ = cancellation.cancelled() => {
            observation.diagnostic.received_bytes=None;
            return Err(observation.failure(Stage::Cancelled,"CANCELLED",None));
        },
        result = executor.open_stream(SingleAttemptRequest {protocol:config.protocol,url,model:config.model,body,credential:lease,
            timeout:PHASE_TIMEOUT,idle_timeout:PHASE_TIMEOUT,framing,region}) => result,
    };
    let stream = match opened {
        Ok(stream) => stream,
        Err(error) => {
            observation.diagnostic.received_bytes=None;
            return Err(observation.invoke_failure(Stage::Connect,error));
        },
    };
    observation.diagnostic.stage = Stage::FirstFrame;
    drain(protocol,stream,response_budget,limits,local_output_limit,requested_output_limit.is_none(),cancellation,observation).await
}

async fn drain(protocol: ChatProtocol, mut stream: nomifun_model_invoke::SingleAttemptStream,
    response_budget:nomifun_model_invoke::SingleAttemptResponseBudget,limits:KnowledgeWritebackLimits,
    local_output_limit:u32,estimate_pressure_guard:bool,cancellation:CancellationToken,mut observation:Observation)
    -> Result<KnowledgeWritebackCompletion,KnowledgeWritebackCompletionError> {
    let mut decoder = new_standalone_frame_decoder(protocol);
    let mut output = String::new();
    let idle = tokio::time::sleep(PHASE_TIMEOUT);
    tokio::pin!(idle);
    loop {
        let next = tokio::select! { biased;
            _ = cancellation.cancelled() => {
                observation.diagnostic.received_bytes=Some(response_budget.received_bytes());
                return Err(observation.failure(Stage::Cancelled,"CANCELLED",None));
            },
            _ = &mut idle => {
                observation.diagnostic.received_bytes=Some(response_budget.received_bytes());
                let stage=observation.diagnostic.stage;
                let code=if stage==Stage::FirstFrame {"FIRST_FRAME_TIMEOUT"} else {"STREAM_IDLE_TIMEOUT"};
                return Err(observation.failure(stage,code,None));
            },
            next = stream.next() => next,
        };
        observation.diagnostic.received_bytes=Some(response_budget.received_bytes());
        if response_budget.received_bytes()>limits.response_max_bytes as u64 {
            return Err(observation.failure(Stage::Stream,"RESPONSE_BYTE_BUDGET_EXCEEDED",None));
        }
        let frame = match next {
            Some(Ok(frame)) => frame,
            Some(Err(error)) => { let stage=observation.diagnostic.stage; return Err(observation.invoke_failure(stage,error)); },
            None => return Err(observation.failure(Stage::Decode,"TERMINAL_MISSING",None)),
        };
        observation.diagnostic.received_frames = Some(observation.diagnostic.received_frames.unwrap_or(0).saturating_add(1));
        if observation.diagnostic.first_frame_ms.is_none() {
            observation.diagnostic.first_frame_ms=Some(observation.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        }
        let events = match decoder.decode_frame(ProviderWireFrame {event:frame.event,data:frame.data,diagnostic:frame.diagnostic}) {
            Ok(events) => events,
            Err(error) => return Err(observation.model_failure(Stage::Decode,error)),
        };
        let received_text=events.iter().filter_map(|event|match event {ChatModelEvent::OutputTextDelta{text}=>Some(text.len()),_=>None})
            .fold(0usize,usize::saturating_add);
        observation.diagnostic.output_bytes=Some(output.len().saturating_add(received_text) as u64);
        if let Some(tokens)=decoder.reported_output_tokens() {
            observation.diagnostic.output_tokens=Some(tokens);
            if tokens>u64::from(local_output_limit) {return Err(observation.failure(Stage::Stream,"OUTPUT_TOKEN_BUDGET_EXCEEDED",None));}
        }
        if decoder.has_deferred_semantic_progress() {
            observation.diagnostic.stage=Stage::Stream;
            idle.as_mut().reset(Instant::now()+PHASE_TIMEOUT);
        }
        for event in events {
            let mut semantic_progress=false;
            match event {
                ChatModelEvent::OutputTextDelta {text} => {
                    observation.semantic_bytes=observation.semantic_bytes.saturating_add(text.len());
                    if output.len().saturating_add(text.len()) > limits.output_max_utf8_bytes {
                        return Err(observation.failure(Stage::Stream,"OUTPUT_BYTE_BUDGET_EXCEEDED",None));
                    }
                    semantic_progress=!text.is_empty();output.push_str(&text);
                },
                ChatModelEvent::ReasoningDelta {text} | ChatModelEvent::ReasoningBlock {text,..} => {
                    observation.semantic_bytes=observation.semantic_bytes.saturating_add(text.len());semantic_progress=!text.is_empty();
                },
                ChatModelEvent::ProviderReasoningBlock {block} => {
                    if let Some(text)=block.visible_text() {
                        observation.semantic_bytes=observation.semantic_bytes.saturating_add(text.len());semantic_progress=true;
                    } else if matches!(block,nomifun_chat_model_broker::ChatProviderReasoning::AnthropicRedactedThinking{ref data,..} if !data.is_empty()) {
                        semantic_progress=true;
                    }
                },
                ChatModelEvent::Usage {..} => {},
                ChatModelEvent::Completed {finish_reason} => {
                    if finish_reason!=ChatFinishReason::Completed {
                        let code=match finish_reason {ChatFinishReason::MaxOutputTokens=>"OUTPUT_TRUNCATED",ChatFinishReason::Refusal=>"PROVIDER_REFUSED",_=>"UNEXPECTED_TERMINAL"};
                        return Err(observation.failure(Stage::Decode,code,None));
                    }
                    if output.trim().is_empty() { return Err(observation.failure(Stage::Decode,"EMPTY_RESPONSE",None)); }
                    observation.diagnostic.stage=Stage::Validate;
                    return Ok(KnowledgeWritebackCompletion {text:output,diagnostic:observation.finish()});
                },
                ChatModelEvent::ResponseStarted {..} | ChatModelEvent::ReasoningSignature {..} => {},
                _ => return Err(observation.failure(Stage::Decode,"UNEXPECTED_MODEL_OUTPUT",None)),
            }
            if estimate_pressure_guard && estimate_tokens(observation.semantic_bytes)> (local_output_limit as usize).saturating_mul(4) {
                return Err(observation.failure(Stage::Stream,"OUTPUT_ESTIMATE_PRESSURE_EXCEEDED",None));
            }
            if semantic_progress {
                observation.diagnostic.stage=Stage::Stream;
                idle.as_mut().reset(Instant::now()+PHASE_TIMEOUT);
            }
        }
    }
}

fn bedrock_region(config: Option<&str>) -> Option<String> {
    config.and_then(|value| serde_json::from_str::<Value>(value).ok())
        .and_then(|value| value.get("region").and_then(Value::as_str).map(str::to_owned))
}

#[cfg(test)]
#[path="knowledge_writeback_completion_tests.rs"]
mod tests;
