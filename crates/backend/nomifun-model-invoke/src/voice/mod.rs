//! Production voice ports. Application registration owns discovery and routing;
//! these integrations own authenticated provider wire and no work authority.

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod input_control_contract_tests;
#[cfg(test)]
mod local_replay_contract_tests;
#[cfg(test)]
mod native_contract_tests;
mod openai_live_wire;
mod resolve;
pub use resolve::VoiceConnectionLease;
#[cfg(test)]
mod speech_source_contract_tests;
mod stepfun_wire;
mod transport;
mod wire;
mod worker;

use crate::{InvokeError, ResolvedConnection};
use async_trait::async_trait;
use nomifun_voice_contracts::voice::*;
pub use nomifun_voice_contracts::voice::{AudioFormat, VoiceError, VoiceErrorKind};
use nomifun_voice_core::{VoiceModelPort, VoiceModelSession, VoiceOpenRequest};
use openai_live_wire::{LiveConfig, LiveFormat};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use stepfun_wire::StepFunConfig;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub const STEPFUN_VOICE_ADAPTER_ID: &str = "stepfun.realtime_s2s";
pub const OPENAI_LIVE_ADAPTER_ID: &str = "openai.gpt_live";
pub const STEPFUN_SESSION_ENDPOINT: &str = "/realtime";
pub const OPENAI_LIVE_SESSION_ENDPOINT: &str = "/live/sessions";

pub fn stepfun_protocol_descriptor() -> crate::ProtocolDescriptor {
    protocol_projection(false)
}
pub fn openai_live_protocol_descriptor() -> crate::ProtocolDescriptor {
    protocol_projection(true)
}
fn protocol_projection(live: bool) -> crate::ProtocolDescriptor {
    use nomifun_api_types::*;
    let (id, endpoint, platforms, connections) = if live {
        (
            OPENAI_LIVE_ADAPTER_ID,
            OPENAI_LIVE_SESSION_ENDPOINT,
            vec!["openai"],
            vec![("OpenAI", "openai", "https://api.openai.com/v1")],
        )
    } else {
        (
            STEPFUN_VOICE_ADAPTER_ID,
            STEPFUN_SESSION_ENDPOINT,
            vec!["stepfun", "stepfun-plan"],
            vec![
                ("StepFun", "stepfun", "https://api.stepfun.com/v1"),
                (
                    "StepFun-Plan",
                    "stepfun-plan",
                    "https://api.stepfun.com/step_plan/v1",
                ),
            ],
        )
    };
    ProtocolDescriptor {
        protocol_id: id.into(),
        supported_tasks: vec![ModelTask::RealtimeConversation],
        executor: ProtocolExecutorKind::RealtimeSession,
        transport: ProtocolTransportKind::Websocket,
        requires_output_ceiling: false,
        allowed_auth_schemes: vec!["bearer".into()],
        scopes: vec![ProtocolScope::Native, ProtocolScope::Custom],
        platforms: platforms.into_iter().map(str::to_owned).collect(),
        default_connections: connections
            .into_iter()
            .map(|(preset, platform, base_url)| ProtocolDefaultConnection {
                preset: preset.into(),
                platform: platform.into(),
                base_url: base_url.into(),
                auth_scheme: "bearer".into(),
                connection_role: None,
                connection_label: None,
                requires_credentials: true,
            })
            .collect(),
        endpoints: vec![ProtocolEndpointDescriptor {
            task: ModelTask::RealtimeConversation,
            field: "realtime_endpoint".into(),
            purpose: ProtocolEndpointPurpose::Session,
            method: Some(if live { "POST" } else { "GET" }.into()),
            default_value: endpoint.into(),
            root_shape: EndpointRootShape::VersionedRoot,
            allowed_placeholders: vec![],
            required_placeholders: vec![],
            editable: true,
        }],
        root_shape: Some(EndpointRootShape::VersionedRoot),
    }
}

pub fn stepfun_config_schema() -> Value {
    StepFunConfig::schema()
}
pub fn openai_live_config_schema() -> Value {
    LiveConfig::schema()
}
pub fn validate_stepfun_config(value: &Value) -> Result<(), String> {
    StepFunConfig::parse(value).map(|_| ())
}
pub fn validate_openai_live_config(value: &Value) -> Result<(), String> {
    LiveConfig::parse(value).map(|_| ())
}

pub fn create_stepfun_model_port(
    connection: ResolvedConnection,
    config: Value,
    model: String,
) -> Result<Arc<dyn VoiceModelPort>, VoiceError> {
    let parsed = StepFunConfig::parse(&config)
        .map_err(|e| VoiceError::new(VoiceErrorKind::Configuration, e))?;
    let endpoint = parsed
        .endpoint
        .clone()
        .unwrap_or_else(|| STEPFUN_SESSION_ENDPOINT.into());
    let allow = parsed.allow_cross_origin_credentials;
    Ok(Arc::new(
        StepFunVoiceModel::new(connection, endpoint, config, allow, model).map_err(invoke_error)?,
    ))
}
pub fn create_openai_live_model_port(
    connection: ResolvedConnection,
    config: Value,
    model: String,
) -> Result<Arc<dyn VoiceModelPort>, VoiceError> {
    let parsed = LiveConfig::parse(&config)
        .map_err(|e| VoiceError::new(VoiceErrorKind::Configuration, e))?;
    let endpoint = parsed
        .endpoint
        .clone()
        .unwrap_or_else(|| OPENAI_LIVE_SESSION_ENDPOINT.into());
    let allow = parsed.allow_cross_origin_credentials;
    let http = nomifun_net::http_client_no_redirect().map_err(|_| {
        VoiceError::new(
            VoiceErrorKind::Configuration,
            "voice HTTP client could not initialize",
        )
    })?;
    Ok(Arc::new(
        OpenAiLiveVoiceModel::new(connection, endpoint, config, allow, http, model)
            .map_err(invoke_error)?,
    ))
}

/// A private resolved credential lease is injected once per route activation.
/// The model and protocol identity remain in VoiceOpenRequest, never inferred
/// from provider names, connection aliases or global defaults.
#[derive(Clone)]
pub struct StepFunVoiceModel {
    connection: ResolvedConnection,
    endpoint: String,
    config: StepFunConfig,
    allow_cross_origin: bool,
    model: String,
}
impl StepFunVoiceModel {
    pub fn new(
        connection: ResolvedConnection,
        endpoint: String,
        config: Value,
        allow_cross_origin: bool,
        model: String,
    ) -> Result<Self, InvokeError> {
        if connection.auth.scheme != crate::AuthScheme::Bearer {
            return Err(InvokeError::config(
                "StepFun voice requires bearer authentication",
            ));
        }
        connection.auth.validate()?;
        let config = StepFunConfig::parse(&config).map_err(InvokeError::config)?;
        transport::endpoint(&connection, &endpoint, true, allow_cross_origin)?;
        Ok(Self {
            connection,
            endpoint,
            config,
            allow_cross_origin,
            model,
        })
    }
}

#[derive(Clone)]
pub struct OpenAiLiveVoiceModel {
    connection: ResolvedConnection,
    endpoint: String,
    config: LiveConfig,
    allow_cross_origin: bool,
    http: reqwest::Client,
    model: String,
    native: bool,
}
impl OpenAiLiveVoiceModel {
    pub fn new(
        connection: ResolvedConnection,
        endpoint: String,
        config: Value,
        allow_cross_origin: bool,
        http: reqwest::Client,
        model: String,
    ) -> Result<Self, InvokeError> {
        if connection.auth.scheme != crate::AuthScheme::Bearer {
            return Err(InvokeError::config(
                "GPT-Live requires bearer authentication",
            ));
        }
        connection.auth.validate()?;
        let config = LiveConfig::parse(&config).map_err(InvokeError::config)?;
        transport::endpoint(&connection, &endpoint, true, allow_cross_origin)?;
        Ok(Self {
            connection,
            endpoint,
            config,
            allow_cross_origin,
            http,
            model,
            native: false,
        })
    }
}

#[async_trait]
impl VoiceModelPort for StepFunVoiceModel {
    fn describe(&self) -> VoiceAdapterDescriptor {
        stepfun_descriptor(&self.model)
    }
    async fn open(
        &self,
        request: VoiceOpenRequest,
        cancel: CancellationToken,
        deadline: Instant,
    ) -> Result<VoiceModelSession, VoiceError> {
        worker::open(
            worker::Provider::StepFun(self.clone()),
            request,
            cancel,
            deadline,
        )
        .await
    }
}
#[async_trait]
impl VoiceModelPort for OpenAiLiveVoiceModel {
    fn describe(&self) -> VoiceAdapterDescriptor {
        openai_live_descriptor(&self.model)
    }
    async fn open(
        &self,
        request: VoiceOpenRequest,
        cancel: CancellationToken,
        deadline: Instant,
    ) -> Result<VoiceModelSession, VoiceError> {
        worker::open(
            worker::Provider::Live(self.clone()),
            request,
            cancel,
            deadline,
        )
        .await
    }
}

pub fn stepfun_descriptor(model: &str) -> VoiceAdapterDescriptor {
    descriptor(STEPFUN_VOICE_ADAPTER_ID, "StepFun Realtime", false, model)
}
pub fn openai_live_descriptor(model: &str) -> VoiceAdapterDescriptor {
    descriptor(OPENAI_LIVE_ADAPTER_ID, "OpenAI GPT-Live", true, model)
}
fn descriptor(id: &str, label: &str, delegation: bool, model: &str) -> VoiceAdapterDescriptor {
    let source = if delegation {
        "https://developers.openai.com/api/docs/guides/live"
    } else {
        "https://platform.stepfun.com/docs/zh/guides/models/stepaudio-3-realtime"
    };
    let mut caps = BTreeMap::new();
    for feature in [
        VoiceFeature::ContinuousInput,
        VoiceFeature::UnderstandInputDuringOutput,
        VoiceFeature::SemanticTurnTaking,
        VoiceFeature::FactInjection,
        VoiceFeature::InputTranscript,
        VoiceFeature::OutputTranscript,
    ] {
        caps.insert(
            feature,
            VoiceCapabilityEvidence {
                support: CapabilitySupport::Supported,
                source: source.into(),
            },
        );
    }
    for (feature, supported) in [
        (VoiceFeature::TypedTools, !delegation),
        (VoiceFeature::Delegation, delegation),
        (VoiceFeature::NativeMedia, delegation),
        (VoiceFeature::InterruptGeneration, !delegation),
        (VoiceFeature::ExactOutputTruncation, false),
        (VoiceFeature::ControlledOutputBoundary, true),
    ] {
        caps.insert(
            feature,
            VoiceCapabilityEvidence {
                support: if supported {
                    CapabilitySupport::Supported
                } else {
                    CapabilitySupport::Unsupported
                },
                source: source.into(),
            },
        );
    }
    let documented = if delegation {
        model == "gpt-live-1"
    } else {
        model == "stepaudio-3-realtime-preview"
    };
    if !documented {
        for feature in [
            VoiceFeature::UnderstandInputDuringOutput,
            VoiceFeature::SemanticTurnTaking,
        ] {
            caps.get_mut(&feature)
                .expect("descriptor capability")
                .support = CapabilitySupport::Unknown;
        }
    }
    let specs = if delegation {
        vec![
            media_spec(LiveFormat::Pcm24k, false),
            media_spec(LiveFormat::Pcm16k, false),
        ]
    } else {
        vec![media_spec(LiveFormat::Pcm24k, false)]
    };
    VoiceAdapterDescriptor {
        adapter_id: id.into(),
        contract_version: VOICE_CONTRACT_VERSION,
        api_version: "2026-10-03".into(),
        config_schema_version: 1,
        label: label.into(),
        native_requirements: if delegation {
            Some(VoiceNativeRequirements {
                data_channel_label: Some("oai-events".into()),
            })
        } else {
            None
        },
        capabilities: caps,
        transports: if delegation {
            vec![
                VoiceTransportPreference::Relay,
                VoiceTransportPreference::NativeWebrtc,
            ]
        } else {
            vec![VoiceTransportPreference::Relay]
        },
        input_specs: specs,
        output_specs: if delegation {
            vec![
                media_spec(LiveFormat::Pcm24k, true),
                media_spec(LiveFormat::Pcm16k, true),
            ]
        } else {
            vec![media_spec(LiveFormat::Pcm24k, true)]
        },
    }
}

fn media_spec(format: LiveFormat, output: bool) -> MediaSpec {
    MediaSpec {
        format: AudioFormat::pcm16(format.rate(), 1),
        timebase: 1_000_000,
        min_frame_duration_us: if output { 1 } else { 10_000 },
        max_frame_duration_us: if output { 120_000 } else { 40_000 },
        max_frame_bytes: format.rate() * format.sample_bytes() as u32 * 120 / 1000,
        max_buffer_duration_us: 240_000,
        max_frame_age_us: 200_000,
    }
}

fn negotiated_capabilities(provider: &worker::Provider, model: &str) -> VoiceCapabilities {
    match provider {
        worker::Provider::StepFun(_) => stepfun_descriptor(model).capabilities,
        worker::Provider::Live(_) => openai_live_descriptor(model).capabilities,
    }
}

fn invoke_error(error: InvokeError) -> VoiceError {
    use crate::InvokeErrorKind as I;
    VoiceError::new(
        match error.kind {
            I::Auth => VoiceErrorKind::Authentication,
            I::QuotaExhausted | I::RateLimited => VoiceErrorKind::Quota,
            I::Timeout => VoiceErrorKind::Deadline,
            I::Config | I::InvalidParams | I::MissingConnection => VoiceErrorKind::Configuration,
            I::UnsupportedTask | I::NoAdapter => VoiceErrorKind::Unsupported,
            I::Network => VoiceErrorKind::Network,
            _ => VoiceErrorKind::Provider,
        },
        error.message,
    )
}
