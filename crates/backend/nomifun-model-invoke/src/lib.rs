//! nomifun-model-invoke — the unified multimodal model invocation layer.
//!
//! Foundation crate of the P1 invoke redesign (see
//! `docs/specs/2026-07-28-multimodal-model-provider-redesign.zh.md`): typed
//! task requests/results ([`types`]), the single error currency ([`error`]),
//! declarative auth schemes ([`auth`]), shared HTTP transport helpers
//! ([`transport`]), the protocol-adapter seam + registry ([`adapter`]), the
//! built-in platform routing table ([`routes_table`]) and the fully-resolved
//! call value ([`call`]).
//!
//! This crate deliberately contains no catalog/DB access (the resolver and
//! service arrive in later tasks) and no concrete protocol adapters.

pub mod adapter;
pub mod auth;
pub mod call;
pub mod error;
pub mod routes_table;
pub mod transport;
pub mod types;

pub use adapter::{AdapterRegistry, ProtocolAdapter};
pub use auth::{AuthMaterial, AuthScheme};
pub use call::{ResolvedCall, ResolvedConnection};
pub use error::{InvokeError, InvokeErrorKind};
pub use routes_table::{TaskRoute, platform_route};
pub use transport::{
    MAX_ARTIFACT_BYTES, decode_b64, encode_b64, error_from_response, net_err, read_body_capped,
};
pub use types::{
    AsrRequest, ChatTextRequest, EmbedRequest, ImageEditRequest, ImageGenRequest, InputAsset, JobHandle,
    ModelRef, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, TtsRequest, VideoGenRequest,
};
