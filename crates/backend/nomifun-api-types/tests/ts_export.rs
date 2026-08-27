use std::path::Path;

use ts_rs::{Config, TS};

use nomifun_api_types::{
    AuthSchemeDescriptor, CapabilityHealth, CloneProviderRequest, EndpointRootShape,
    FetchModelsResponse, HealthStatus,
    KnowledgeEmbeddingConfig, KnowledgeEntry, KnowledgeEntryKind, KnowledgeEntryOrigin,
    KnowledgeRerankConfig, KnowledgeRetrievalConfig, KnowledgeTreeAccess, ModelInfo,
    ModelProtocolManifestResponse, ModelTask, ModelTrait, PlatformPresetDescriptor,
    ProtocolDefaultConnection, ProtocolDescriptor, ProtocolEndpointDescriptor,
    ProtocolEndpointPurpose, ProtocolExecutorKind, ProtocolRecommendation, ProtocolScope,
    ProbeCandidateResult, ProbeProviderConnectionAnonymousRequest, ProbeProviderConnectionRequest,
    ProbeProviderConnectionResponse, ProtocolTaskDescriptor, ProtocolTransportKind,
    ProviderConnectionInput,
    ProviderConnectionResponse, ProviderHealthCheckErrorKind, ProviderHealthCheckRequest,
    ProviderHealthCheckResponse, ProviderModelCapabilityInput, ProviderModelCapabilityResponse,
    ProviderReachability, RelocateKnowledgeEntryConflictPolicy, RelocateKnowledgeEntryRequest,
    RelocateKnowledgeEntryResponse, UndoKnowledgeEntryRelocationRequest,
    ProviderModelInput, ProviderModelKeyRequest, ProviderModelResponse,
    SaveProviderConnectionRequest, SaveProviderModelRequest,
};

// ts-rs preserves formatting whitespace from wrapped declarations. Keep
// committed bindings platform-independent and stable across test runs.
fn normalize_typescript_binding(generated: &str) -> String {
    let mut normalized = generated
        .lines()
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n");
    while normalized.ends_with('\n') {
        normalized.pop();
    }
    normalized.push('\n');
    normalized
}

fn export_binding_if_changed<T: TS + 'static>(file_name: &str) {
    let generated = normalize_typescript_binding(
        &T::export_to_string(&Config::default())
            .unwrap_or_else(|error| panic!("{file_name} must export to TypeScript: {error}")),
    );
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../ui/src/common/protocolBindings")
        .join(file_name);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(&generated) {
        std::fs::write(&path, generated)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    }
}

#[test]
fn export_provider_domain_bindings() {
    export_binding_if_changed::<KnowledgeEntryKind>("KnowledgeEntryKind.ts");
    export_binding_if_changed::<KnowledgeEntryOrigin>("KnowledgeEntryOrigin.ts");
    export_binding_if_changed::<KnowledgeEntry>("KnowledgeEntry.ts");
    export_binding_if_changed::<KnowledgeTreeAccess>("KnowledgeTreeAccess.ts");
    export_binding_if_changed::<RelocateKnowledgeEntryConflictPolicy>(
        "RelocateKnowledgeEntryConflictPolicy.ts",
    );
    export_binding_if_changed::<RelocateKnowledgeEntryRequest>(
        "RelocateKnowledgeEntryRequest.ts",
    );
    export_binding_if_changed::<RelocateKnowledgeEntryResponse>(
        "RelocateKnowledgeEntryResponse.ts",
    );
    export_binding_if_changed::<UndoKnowledgeEntryRelocationRequest>(
        "UndoKnowledgeEntryRelocationRequest.ts",
    );
    export_binding_if_changed::<KnowledgeEmbeddingConfig>("KnowledgeEmbeddingConfig.ts");
    export_binding_if_changed::<KnowledgeRerankConfig>("KnowledgeRerankConfig.ts");
    export_binding_if_changed::<KnowledgeRetrievalConfig>("KnowledgeRetrievalConfig.ts");
    export_binding_if_changed::<ModelTask>("ModelTask.ts");
    export_binding_if_changed::<ModelTrait>("ModelTrait.ts");
    export_binding_if_changed::<HealthStatus>("HealthStatus.ts");
    export_binding_if_changed::<ProviderHealthCheckErrorKind>("ProviderHealthCheckErrorKind.ts");
    export_binding_if_changed::<ProviderHealthCheckRequest>("ProviderHealthCheckRequest.ts");
    export_binding_if_changed::<ProviderHealthCheckResponse>("ProviderHealthCheckResponse.ts");
    export_binding_if_changed::<ProviderReachability>("ProviderReachability.ts");
    export_binding_if_changed::<ProbeCandidateResult>("ProbeCandidateResult.ts");
    export_binding_if_changed::<ProbeProviderConnectionRequest>("ProbeProviderConnectionRequest.ts");
    export_binding_if_changed::<ProbeProviderConnectionAnonymousRequest>(
        "ProbeProviderConnectionAnonymousRequest.ts",
    );
    export_binding_if_changed::<ProbeProviderConnectionResponse>(
        "ProbeProviderConnectionResponse.ts",
    );
    export_binding_if_changed::<CapabilityHealth>("CapabilityHealth.ts");
    export_binding_if_changed::<ProviderModelCapabilityInput>("ProviderModelCapabilityInput.ts");
    export_binding_if_changed::<ProviderModelCapabilityResponse>(
        "ProviderModelCapabilityResponse.ts",
    );
    export_binding_if_changed::<ProviderModelInput>("ProviderModelInput.ts");
    export_binding_if_changed::<ProviderModelResponse>("ProviderModelResponse.ts");
    export_binding_if_changed::<SaveProviderModelRequest>("SaveProviderModelRequest.ts");
    export_binding_if_changed::<ProviderModelKeyRequest>("ProviderModelKeyRequest.ts");
    export_binding_if_changed::<ModelInfo>("ModelInfo.ts");
    export_binding_if_changed::<FetchModelsResponse>("FetchModelsResponse.ts");
    export_binding_if_changed::<ProviderConnectionInput>("ProviderConnectionInput.ts");
    export_binding_if_changed::<SaveProviderConnectionRequest>("SaveProviderConnectionRequest.ts");
    export_binding_if_changed::<ProviderConnectionResponse>("ProviderConnectionResponse.ts");
    export_binding_if_changed::<CloneProviderRequest>("CloneProviderRequest.ts");
    export_binding_if_changed::<ProtocolExecutorKind>("ProtocolExecutorKind.ts");
    export_binding_if_changed::<ProtocolTransportKind>("ProtocolTransportKind.ts");
    export_binding_if_changed::<ProtocolScope>("ProtocolScope.ts");
    export_binding_if_changed::<ProtocolEndpointPurpose>("ProtocolEndpointPurpose.ts");
    export_binding_if_changed::<EndpointRootShape>("EndpointRootShape.ts");
    export_binding_if_changed::<ProtocolEndpointDescriptor>("ProtocolEndpointDescriptor.ts");
    export_binding_if_changed::<ProtocolDefaultConnection>("ProtocolDefaultConnection.ts");
    export_binding_if_changed::<ProtocolDescriptor>("ProtocolDescriptor.ts");
    export_binding_if_changed::<ProtocolTaskDescriptor>("ProtocolTaskDescriptor.ts");
    export_binding_if_changed::<PlatformPresetDescriptor>("PlatformPresetDescriptor.ts");
    export_binding_if_changed::<ProtocolRecommendation>("ProtocolRecommendation.ts");
    export_binding_if_changed::<AuthSchemeDescriptor>("AuthSchemeDescriptor.ts");
    export_binding_if_changed::<ModelProtocolManifestResponse>("ModelProtocolManifestResponse.ts");
}

#[test]
fn generated_bindings_have_deterministic_whitespace() {
    assert_eq!(
        normalize_typescript_binding("export type Value = { \r\nfield: string,\t\r\n}\r\n\r\n"),
        "export type Value = {\nfield: string,\n}\n"
    );
}

#[test]
fn generated_shapes_mirror_single_source_wire_contract() {
    let cfg = Config::default();
    let connection_create = ProviderConnectionInput::export_to_string(&cfg).unwrap();
    assert!(
        connection_create.contains("credentials: unknown,"),
        "got: {connection_create}"
    );
    assert!(
        !connection_create.contains("credentials?: unknown,"),
        "got: {connection_create}"
    );
    let connection_save = SaveProviderConnectionRequest::export_to_string(&cfg).unwrap();
    assert!(
        connection_save.contains("credentials?: unknown,"),
        "got: {connection_save}"
    );

    let save = SaveProviderModelRequest::export_to_string(&cfg).unwrap();
    assert!(save.contains("provider_id: string,"), "got: {save}");
    assert!(save.contains("model: ProviderModelInput,"), "got: {save}");

    let capability = ProviderModelCapabilityResponse::export_to_string(&cfg).unwrap();
    assert!(
        capability.contains("protocol: string,"),
        "got: {capability}"
    );
    assert!(
        capability.contains("connection_role: string,"),
        "got: {capability}"
    );
    assert!(
        capability.contains("provider_params: unknown,"),
        "got: {capability}"
    );
    assert!(
        capability.contains("health?: CapabilityHealth,"),
        "got: {capability}"
    );
    assert!(
        capability.contains("context_limit?: number,"),
        "got: {capability}"
    );
    assert!(
        capability.contains("output_limit?: number,"),
        "got: {capability}"
    );

    let response = ProviderModelResponse::export_to_string(&cfg).unwrap();
    assert!(response.contains("capabilities: Array<ProviderModelCapabilityResponse>"));
    assert!(!response.contains("protocol?:"));
    assert!(!response.contains("tasks:"));

    // The catalog carries the provider's own declared window under the same
    // name the capability persists it as, so the UI can prefill one from the
    // other without a translation table. Optional in both types.
    let catalog_model = ModelInfo::export_to_string(&cfg).unwrap();
    assert!(
        catalog_model.contains("context_limit?: number,"),
        "got: {catalog_model}"
    );
    // `tasks`/`traits` are omitted when empty and `name` is always sent, so the
    // optionality of each field must keep mirroring its serde attributes.
    assert!(
        catalog_model.contains("tasks?: Array<ModelTask>,"),
        "got: {catalog_model}"
    );
    assert!(
        catalog_model.contains("name: string | null,"),
        "got: {catalog_model}"
    );
    let catalog = FetchModelsResponse::export_to_string(&cfg).unwrap();
    assert!(
        catalog.contains("models: Array<ModelInfo>,"),
        "got: {catalog}"
    );
    assert!(
        catalog.contains("fixed_base_url?: string,"),
        "got: {catalog}"
    );

    let connection = ProviderConnectionResponse::export_to_string(&cfg).unwrap();
    assert!(!connection.contains("is_full_url"));
    assert!(connection.contains("extra: unknown,"));

    let manifest = ModelProtocolManifestResponse::export_to_string(&cfg).unwrap();
    assert!(manifest.contains("tasks: Array<ModelTask>"));
    assert!(manifest.contains("platform_default_base_url: string | null"));
    assert!(manifest.contains("recommendation: ProtocolRecommendation | null"));

    let recommendation = ProtocolRecommendation::export_to_string(&cfg).unwrap();
    assert!(recommendation.contains("default_auth_scheme: string | null"));
    assert!(recommendation.contains("base_url_override_required: boolean"));

    let connection = ProtocolDefaultConnection::export_to_string(&cfg).unwrap();
    assert!(connection.contains("connection_role: string | null"));
    assert!(connection.contains("auth_scheme: string"));
    assert!(connection.contains("requires_credentials: boolean"));

    // The version-boundary convention must reach the UI: it is the only thing
    // that tells a custom provider whether `/v1` belongs in its base URL, and a
    // custom-scope manifest ships no default connection to infer it from.
    let endpoint = ProtocolEndpointDescriptor::export_to_string(&cfg).unwrap();
    assert!(
        endpoint.contains("root_shape: EndpointRootShape,"),
        "got: {endpoint}"
    );
    let root_shape = EndpointRootShape::export_to_string(&cfg).unwrap();
    assert!(root_shape.contains("\"versioned_root\""), "got: {root_shape}");
    assert!(root_shape.contains("\"origin_root\""), "got: {root_shape}");
    let protocol = ProtocolDescriptor::export_to_string(&cfg).unwrap();
    assert!(
        protocol.contains("root_shape: EndpointRootShape | null"),
        "got: {protocol}"
    );
    assert!(
        protocol.contains("requires_output_ceiling: boolean"),
        "got: {protocol}"
    );
}
