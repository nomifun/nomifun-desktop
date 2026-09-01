//! Canonical machine contracts for Agent Capability Platform v2.
//!
//! C0/G0 freezes types and deterministic artifacts only. This crate must not
//! depend on legacy Nomi, Conversation, application composition, or product
//! runtime crates.

pub mod closure;
pub mod deletion;
pub mod digest;
pub mod event;
pub mod manifest;
pub mod model_route;
pub mod package;
pub mod preset;
pub mod primitives;
pub mod remote;
pub mod root;
pub mod runtime;
pub mod schema;
pub mod session;
pub mod validation;

pub use closure::*;
pub use deletion::*;
pub use digest::{
    ArtifactEnvelope, CanonicalDigestError, canonical_json_bytes, digest_bytes, digest_payload,
};
pub use event::*;
pub use manifest::*;
pub use model_route::*;
pub use package::*;
pub use preset::*;
pub use primitives::*;
pub use remote::*;
pub use root::*;
pub use runtime::*;
pub use schema::{
    CHAT_ROUTE_RECORD_JSON_SCHEMA, FRESH_V4_BASELINE_SQL, FRESH_V4_DATA_GENERATION,
    FRESH_V4_MIGRATION_HEAD, FRESH_V4_PROJECTION_SCHEMA_VERSION,
    FreshV4SchemaManifestPayload, SchemaTableContract, fresh_v4_schema_manifest_payload,
};
pub use session::*;
pub use validation::*;
