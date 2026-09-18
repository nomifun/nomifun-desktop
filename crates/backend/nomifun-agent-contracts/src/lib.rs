//! Canonical machine contracts for Agent Capability Platform v2.
//!
//! C0/G0 freezes types and deterministic artifacts only. This crate must not
//! depend on legacy Nomi, Conversation, application composition, or product
//! runtime crates.

pub mod closure;
pub mod catalog;
pub mod chat_model;
pub mod chat_provider_reasoning;
pub mod deletion;
pub mod digest;
pub mod event;
pub mod engine_features;
pub mod impact;
pub mod manifest;
pub mod plugin_runtime;
pub mod model_route;
pub mod model_middleware;
pub mod tool_middleware;
pub mod package;
pub mod plugin_n1;
pub mod preset;
pub mod primitives;
pub mod remote;
pub mod runtime;
pub mod schema;
pub mod session;
pub mod validation;

pub use closure::*;
pub use catalog::*;
pub use deletion::*;
pub use digest::{
    ArtifactEnvelope, CanonicalDigestError, canonical_json_bytes, digest_bytes, digest_payload,
};
pub use event::*;
pub use engine_features::*;
pub use impact::*;
pub use manifest::*;
pub use plugin_runtime::*;
pub use model_route::*;
pub use package::*;
pub use plugin_n1::*;
pub use preset::*;
pub use primitives::*;
pub use remote::*;
pub use runtime::*;
pub use schema::{
    AGENT_STORE_BASELINE_SQL, AGENT_STORE_DATA_GENERATION, AGENT_STORE_MIGRATION_HEAD,
    AGENT_STORE_PROJECTION_SCHEMA_VERSION, AgentStoreSchemaManifestPayload,
    CHAT_ROUTE_RECORD_JSON_SCHEMA, SchemaResetScope, SchemaTableContract,
    agent_store_schema_manifest_payload,
};
pub use session::*;
pub use validation::*;
