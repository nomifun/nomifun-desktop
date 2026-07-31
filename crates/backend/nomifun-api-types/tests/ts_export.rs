//! ts-rs contract generation for the P0 provider domain: a cargo test IS the
//! generator (same pattern as `nomifun-ai-agent`'s protocol events / the
//! `AgentExecutionEventKind` binding in `nomifun-common`). Each binding is
//! rendered via `export_to_string` and written into
//! `ui/src/common/protocolBindings/` only when its content changed, so a plain
//! `cargo test -p nomifun-api-types` keeps the committed TypeScript in sync
//! and CI fails loudly if a type cannot render.

use std::path::Path;

use ts_rs::{Config, TS};

use nomifun_api_types::{
    CatalogModelRef, CloneProviderRequest, CreateProviderModelRequest, HealthStatus,
    ModelHealthStatus, ModelTask, ModelTrait, ProfileSource, ProviderConnectionResponse,
    ProviderModelKeyRequest, ProviderModelResponse, ResolveModelsRequest, ResolveModelsResponse,
    UpdateProviderModelRequest, UpsertProviderConnectionRequest,
};

fn export_binding_if_changed<T: TS + 'static>(file_name: &str) {
    let generated = T::export_to_string(&Config::default())
        .unwrap_or_else(|error| panic!("{file_name} must export to TypeScript: {error}"));
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../ui/src/common/protocolBindings")
        .join(file_name);
    let unchanged = std::fs::read_to_string(&path)
        .map(|current| current == generated)
        .unwrap_or(false);
    if !unchanged {
        std::fs::write(&path, generated)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    }
}

#[test]
fn export_provider_domain_bindings() {
    // Vocabulary enums.
    export_binding_if_changed::<ModelTask>("ModelTask.ts");
    export_binding_if_changed::<ModelTrait>("ModelTrait.ts");
    export_binding_if_changed::<ProfileSource>("ProfileSource.ts");
    export_binding_if_changed::<HealthStatus>("HealthStatus.ts");
    // Row-level provider model catalog DTOs.
    export_binding_if_changed::<ModelHealthStatus>("ModelHealthStatus.ts");
    export_binding_if_changed::<ProviderModelResponse>("ProviderModelResponse.ts");
    export_binding_if_changed::<CreateProviderModelRequest>("CreateProviderModelRequest.ts");
    export_binding_if_changed::<UpdateProviderModelRequest>("UpdateProviderModelRequest.ts");
    export_binding_if_changed::<ProviderModelKeyRequest>("ProviderModelKeyRequest.ts");
    // Connection profiles.
    export_binding_if_changed::<ProviderConnectionResponse>("ProviderConnectionResponse.ts");
    export_binding_if_changed::<UpsertProviderConnectionRequest>("UpsertProviderConnectionRequest.ts");
    // Catalog resolution + provider clone.
    export_binding_if_changed::<CatalogModelRef>("CatalogModelRef.ts");
    export_binding_if_changed::<ResolveModelsRequest>("ResolveModelsRequest.ts");
    export_binding_if_changed::<ResolveModelsResponse>("ResolveModelsResponse.ts");
    export_binding_if_changed::<CloneProviderRequest>("CloneProviderRequest.ts");
}

/// The emitted shapes must mirror the serde wire truth — especially the
/// double-Option tri-state fields (`x?: T | null`) and the `unknown` mapping
/// for opaque `serde_json::Value` payloads.
#[test]
fn generated_shapes_mirror_serde_truth() {
    let cfg = Config::default();

    let update = UpdateProviderModelRequest::export_to_string(&cfg).unwrap();
    // Double-Option: absent = keep, null = clear, value = set → `x?: T | null`.
    assert!(update.contains("protocol?: string | null,"), "got: {update}");
    assert!(update.contains("connection_role?: string | null,"), "got: {update}");
    assert!(update.contains("context_limit?: number | null,"), "got: {update}");
    assert!(update.contains("description?: string | null,"), "got: {update}");
    // Opaque params → unknown; identity keys stay required.
    assert!(update.contains("params?: unknown,"), "got: {update}");
    assert!(update.contains("provider_id: string,"), "got: {update}");
    assert!(update.contains("model: string,"), "got: {update}");

    let response = ProviderModelResponse::export_to_string(&cfg).unwrap();
    // skip_serializing_if optionals → `x?: T` (never null on the wire).
    assert!(response.contains("protocol?: string,"), "got: {response}");
    assert!(response.contains("health?: ModelHealthStatus,"), "got: {response}");
    assert!(response.contains("health_checked_at?: number,"), "got: {response}");
    // i64 renders as number (plain JSON numbers on this API), not bigint.
    assert!(response.contains("sort_order: number,"), "got: {response}");
    assert!(!response.contains("bigint"), "got: {response}");
    assert!(response.contains("params: unknown,"), "got: {response}");

    let connection = ProviderConnectionResponse::export_to_string(&cfg).unwrap();
    assert!(connection.contains("extra: unknown,"), "got: {connection}");
    assert!(connection.contains("label?: string,"), "got: {connection}");
    assert!(connection.contains("has_credentials: boolean,"), "got: {connection}");

    let clone = CloneProviderRequest::export_to_string(&cfg).unwrap();
    assert!(clone.contains("name?: string | null"), "got: {clone}");

    // Enum vocabularies stay snake_case wire values.
    let task = ModelTask::export_to_string(&cfg).unwrap();
    for value in [
        "\"chat\"",
        "\"image_generation\"",
        "\"image_edit\"",
        "\"video_generation\"",
        "\"speech_synthesis\"",
        "\"speech_recognition\"",
        "\"embedding\"",
        "\"rerank\"",
    ] {
        assert!(task.contains(value), "ModelTask missing {value}: {task}");
    }
    let health = HealthStatus::export_to_string(&cfg).unwrap();
    assert!(health.contains("\"unknown\" | \"healthy\" | \"unhealthy\""), "got: {health}");
    let source = ProfileSource::export_to_string(&cfg).unwrap();
    assert!(source.contains("\"inferred\" | \"user\""), "got: {source}");
}
