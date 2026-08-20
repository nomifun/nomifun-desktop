//! nomifun-model-invoke — the unified multimodal model invocation layer.
//!
//! Runtime resolution reads one exact persisted
//! `(provider_id, model, task)` capability ([`resolve`]). Its protocol id owns
//! the transport and endpoint contract; [`service`] never guesses from a
//! provider name or falls back to another task. Typed requests/results live in
//! [`types`], declarative auth in [`auth`], resolved calls in [`call`], and
//! protocol executors behind the [`adapter`] registry.
//!
//! [`manifest`] and [`routes_table`] expose configuration-time recommendations
//! for saving a capability. They are never runtime routing or fallback
//! authority. Concrete executors live in [`adapters`] and are selected only by
//! the persisted protocol id.

pub mod adapter;
pub mod adapters;
pub mod auth;
pub mod call;
pub mod error;
pub mod manifest;
pub mod realtime;
pub mod materialize;
pub mod resolve;
pub mod routes_table;
pub mod service;
pub mod transport;
pub mod types;
pub mod url_algebra;

pub use adapter::{AdapterRegistry, ProtocolAdapter};
pub use adapters::{
    default_adapters, default_realtime_adapters, is_reserved_local_transport_param_key,
    reserved_local_transport_param_keys,
};
pub use auth::{AuthMaterial, AuthScheme};
pub use call::{
    ResolvedCall, ResolvedConnection, ResolvedTaskConfig, ResolvedTaskTransport,
    resolve_submit_url, validate_credentialed_target_url,
};
pub use error::{InvokeError, InvokeErrorKind};
pub use manifest::{
    ALL_MODEL_TASKS, AuthSchemeDescriptor, ModelProtocolManifestResponse,
    PlatformPresetDescriptor, ProtocolDefaultConnection, ProtocolDescriptor,
    ProtocolEndpointDescriptor, ProtocolEndpointPurpose, ProtocolExecutorKind,
    ProtocolManifestRegistry, ProtocolRecommendation, ProtocolScope,
    ProtocolTaskDescriptor, ProtocolTransportKind, auth_scheme_descriptors,
    default_protocol_registry, platform_presets, protocol_descriptor,
    protocol_manifest_for, protocol_manifest_for_connection,
    protocol_manifest_for_model_connection, protocol_task_descriptor,
    try_default_protocol_registry, validate_endpoint_template,
    expand_protocol_endpoint_template, validate_provider_params_for_protocol,
};
pub use materialize::{MaterializeLimits, MaterializedAsset};
pub use realtime::{
    RealtimeAdapterRegistry, RealtimeClientCommand, RealtimeProtocolAdapter, RealtimeSendError,
    RealtimeServerEvent, RealtimeSession, RealtimeSessionConfig, RealtimeSessionLimits,
    RealtimeTurnDetection, ResolvedRealtimeCall,
};
pub use routes_table::{TaskRoute, preset_protocol_recommendation};
pub use service::{InvocationContext, ModelInvokeService, ProbeReport};
pub use transport::{
    MAX_ARTIFACT_BYTES, decode_b64, encode_b64, error_from_response, net_err, read_body_capped,
};
pub use types::{
    AsrRequest, EmbedRequest, ImageEditRequest, ImageGenRequest, InputAsset, JobHandle,
    ModelRef, ProducedAsset, ProducedData, RerankRequest, RerankResult, TaskOutcome, TaskRequest, TaskResult,
    TtsRequest, VideoGenRequest,
};
pub use url_algebra::{
    ROOT_VERSION_SUFFIXES, is_version_segment, join_endpoint, root_candidates,
    root_declares_version, root_matches_shape,
};
