use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetRevisionPayload, AgentSessionAggregate, ArtifactEnvelope,
    CanonicalApiInventoryPayload, CanonicalErrorRegistryPayload, CanonicalV4SchemaManifestPayload,
    CodexRuntimeReleaseManifestPayload, CodingRuntimeFeatureInventoryPayload,
    ContractClosurePayload, ContractDigestLedgerPayload, D025FixtureContractReferencePayload,
    D025FixtureEnvelopeReference, D026OrderingOutcomeMatrix, D027TerminalSequenceMatrix,
    D028PlatformMatrix, DeletionManifest, DigestHex, FRESH_V4_BASELINE_SQL, OfficialPresetKey,
    OfficialPresetSeedManifestPayload, PackageManifest, PlatformValidationManifestPayload,
    PluginRegistrationMetadata, RemoteBinding, ResolvedSnapshotContent, RuntimeCommand,
    RuntimeHelloPayload, SessionEventRegistryPayload, TargetPackageInventoryPayload, VersionString,
    FreshV4ParentOperationMarker, FreshV4ReadyMarker, FreshV4SchemaMetadata, digest_bytes,
    digest_payload,
};
use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn main() {
    if let Err(error) = run() {
        eprintln!("agent-v2 contract tool failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "check".to_owned());
    if mode != "check" && mode != "write" {
        return Err(format!("usage: agent-v2-contract [check|write], got {mode:?}").into());
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let contracts = root.join("contracts");
    let generated = contracts.join("generated");

    let closure_path = contracts.join("closure/contract-closure.v1.json");
    let inventory_path =
        contracts.join("target-packages/target-first-party-contributions.v1.json");
    let feature_path =
        contracts.join("runtime/coding-runtime-feature-inventory.payload.json");
    let seed_path =
        contracts.join("presets/official-preset-seed-manifest.payload.json");
    let api_path = contracts.join("presets/canonical-api-inventory.payload.json");
    let event_path = contracts.join("events/session-event-registry.json");
    let error_path = contracts.join("events/error-registry.json");
    let runtime_release_path =
        contracts.join("runtime/runtime-release-fixture.json");
    let d026_path = contracts.join("validation/d026-ordering-outcomes.matrix.json");
    let d027_path = contracts.join("validation/d027-terminal-sequences.matrix.json");
    let d028_path = contracts.join("validation/d028-platform-matrix.json");
    let d025_payload_path =
        contracts.join("validation/d025-compatibility-fixture-reference.payload.json");
    let d025_reference_path =
        contracts.join("validation/d025-fixture-envelope-reference.json");
    let d025_envelope_path =
        contracts.join("validation/d025-compatibility-fixture-reference.envelope.json");
    let checkpoint_fixture_path = contracts.join("runtime/checkpoint-mismatch.json");
    let d026_remote_fixture_path =
        contracts.join("remote/d026-request-admission-ordering.fixture.json");
    let platform_path =
        contracts.join("validation/platform-validation-manifest.payload.json");

    let closure: ContractClosurePayload = read_json(&closure_path)?;
    validate_closure(&closure)?;
    let closure_digest = digest_payload(&closure)?;

    let inventory: TargetPackageInventoryPayload = read_json(&inventory_path)?;
    validate_target_inventory(&inventory)?;
    let inventory_digest = digest_payload(&inventory)?;

    let feature_inventory: CodingRuntimeFeatureInventoryPayload = read_json(&feature_path)?;
    feature_inventory.validate().map_err(|error| error.message)?;
    let feature_digest = digest_payload(&feature_inventory)?;

    let mut seed: OfficialPresetSeedManifestPayload = read_json(&seed_path)?;
    seed.target_first_party_contribution_digest = inventory_digest.clone();
    seed.target_runtime_feature_inventory_digest = feature_digest.clone();
    seed.validate().map_err(|error| error.message)?;
    seed.validate_against_target_inventory(&inventory)
        .map_err(|error| error.message)?;
    seed.validate_against_runtime_feature_inventory(
        &feature_digest,
        &feature_inventory.runtime_features,
    )
    .map_err(|error| error.message)?;
    let seed_digest = digest_payload(&seed)?;

    let api_inventory: CanonicalApiInventoryPayload = read_json(&api_path)?;
    let api_digest = digest_payload(&api_inventory)?;

    let event_registry: SessionEventRegistryPayload = read_json(&event_path)?;
    event_registry.validate()?;
    let event_digest = digest_payload(&event_registry)?;

    let error_registry: CanonicalErrorRegistryPayload = read_json(&error_path)?;
    error_registry.validate()?;
    let error_digest = digest_payload(&error_registry)?;

    let deletion_digests = validate_deletion_manifests(&contracts.join("deletion"))?;
    let deletion_set_digest = digest_payload(&deletion_digests)?;

    let d026: D026OrderingOutcomeMatrix = read_json(&d026_path)?;
    if !d026.validate_exact_contract() {
        return Err("D-026 ordering matrix is not the exact contract".into());
    }
    let d027: D027TerminalSequenceMatrix = read_json(&d027_path)?;
    if !d027.validate_exact_contract() {
        return Err("D-027 terminal matrix is not the exact contract".into());
    }
    let d028: D028PlatformMatrix = read_json(&d028_path)?;
    d028.validate_exact_contract()?;
    let availability_digest = digest_payload(&d028)?;

    let checkpoint_fixture: Value = read_json(&checkpoint_fixture_path)?;
    let checkpoint_fixture_digest = digest_payload(&checkpoint_fixture)?;
    let mut d025_payload: D025FixtureContractReferencePayload = read_json(&d025_payload_path)?;
    d025_payload.checkpoint_mismatch_fixture.digest = checkpoint_fixture_digest;
    let d025_envelope = ArtifactEnvelope::new(d025_payload.clone())?;
    let d025_envelope_contents = pretty_json(&d025_envelope)?;
    let d025_envelope_artifact_digest = digest_bytes(d025_envelope_contents.as_bytes());
    let mut d025_reference: D025FixtureEnvelopeReference = read_json(&d025_reference_path)?;
    d025_reference.fixture_envelope.normalized_relative_path =
        "contracts/validation/d025-compatibility-fixture-reference.envelope.json".to_owned();
    d025_reference.fixture_envelope.digest = d025_envelope_artifact_digest;
    let d026_remote_fixture: Value = read_json(&d026_remote_fixture_path)?;
    let d026_remote_fixture_digest = digest_payload(&d026_remote_fixture)?;
    let d026_digest = digest_payload(&d026)?;
    let d027_digest = digest_payload(&d027)?;

    let schemas = generated_schemas()?;
    let rust_contract_schema_digest = digest_payload(&schemas)?;
    let package_schema_digest = digest_payload(&schema_subset(
        &schemas,
        &[
            "package_manifest",
            "plugin_registration",
            "target_package_inventory",
        ],
    ))?;
    let runtime_protocol_digest = digest_payload(&schema_subset(
        &schemas,
        &[
            "runtime_command",
            "runtime_hello",
            "runtime_feature_inventory",
        ],
    ))?;
    let platform_contract_digest = digest_payload(&schema_subset(
        &schemas,
        &["platform_validation_manifest"],
    ))?;

    let coding_seed = seed
        .templates
        .get(&OfficialPresetKey::CodingCodex)
        .ok_or("coding.codex seed is missing")?;
    let coding_contract_digest = digest_payload(&json!({
        "seed": coding_seed,
        "runtime_features": feature_inventory.runtime_features,
        "native_actions": feature_inventory.native_actions,
        "responses_semantics": feature_inventory.responses_semantics,
    }))?;

    let database_schema_digest = digest_bytes(FRESH_V4_BASELINE_SQL.as_bytes());
    let cargo_lock_digest = digest_bytes(&fs::read(root.join("../../../Cargo.lock"))?);

    let canonical_manifest_payload = CanonicalV4SchemaManifestPayload {
        manifest_version: VersionString("1.0.0".to_owned()),
        database_schema_digest: database_schema_digest.clone(),
        rust_contract_schema_digest: rust_contract_schema_digest.clone(),
        package_schema_digest: package_schema_digest.clone(),
        official_preset_seed_manifest_digest: seed_digest.clone(),
        canonical_api_inventory_digest: api_digest.clone(),
        session_event_registry_digest: event_digest.clone(),
        error_registry_digest: error_digest.clone(),
        runtime_protocol_digest: runtime_protocol_digest.clone(),
        runtime_feature_inventory_digest: feature_digest.clone(),
        deletion_manifest_set_digest: deletion_set_digest.clone(),
        platform_validation_contract_digest: platform_contract_digest.clone(),
        confirmed_decision_contract_digest: closure_digest.clone(),
    };
    let canonical_manifest = ArtifactEnvelope::new(canonical_manifest_payload)?;
    let canonical_manifest_digest = canonical_manifest.payload_digest.clone();

    let mut runtime_release: CodexRuntimeReleaseManifestPayload =
        read_json(&runtime_release_path)?;
    normalize_runtime_fixture_digests(&mut runtime_release);
    runtime_release.cargo_lock_digest = cargo_lock_digest.clone();
    runtime_release.protocol_schema_digest = runtime_protocol_digest.clone();
    runtime_release.runtime_profile_contract_digest = rust_contract_schema_digest.clone();
    runtime_release.coding_capability_pack_digest = coding_contract_digest.clone();
    runtime_release.native_feature_contract_digest = feature_digest.clone();
    runtime_release.native_action_contract_digest =
        digest_payload(&feature_inventory.native_actions)?;
    runtime_release.validate().map_err(|error| error.message)?;
    let mut platform: PlatformValidationManifestPayload = read_json(&platform_path)?;
    platform.confirmed_decision_contract_digest.0 = closure_digest.clone();
    platform.canonical_schema_manifest_digest = canonical_manifest_digest.clone();
    platform.cargo_lock_digest = cargo_lock_digest.clone();
    platform.official_preset_seed_manifest_digest = seed_digest.clone();
    platform.capability_availability_manifest_digest = availability_digest.clone();
    platform.coding_codex_native_contract_digest = coding_contract_digest.clone();
    platform.decision_fixture_refs.d025_snapshot_compatibility = d025_reference.clone();
    platform
        .decision_fixture_refs
        .d026_request_admission_ordering
        .digest = d026_remote_fixture_digest;
    platform
        .decision_fixture_refs
        .d026_validation_outcomes
        .digest = d026_digest;
    platform.decision_fixture_refs.d027_terminal_drain.digest = d027_digest;
    platform.decision_fixture_refs.d028_platform_matrix.digest = availability_digest.clone();
    platform.validate_contract()?;
    let platform_fixture_digest = digest_payload(&platform)?;
    let runtime_release_envelope = ArtifactEnvelope::new(runtime_release.clone())?;
    let runtime_release_fixture_digest = runtime_release_envelope.payload_digest.clone();
    let platform_envelope = ArtifactEnvelope::new(platform.clone())?;

    let mut digest_map = BTreeMap::new();
    digest_map.insert("canonical_api_inventory".to_owned(), api_digest);
    digest_map.insert(
        "canonical_schema_manifest".to_owned(),
        canonical_manifest_digest,
    );
    digest_map.insert("cargo_lock".to_owned(), cargo_lock_digest);
    digest_map.insert("coding_codex_native_contract".to_owned(), coding_contract_digest);
    digest_map.insert(
        "confirmed_decision_contract".to_owned(),
        closure_digest.clone(),
    );
    digest_map.insert("database_schema".to_owned(), database_schema_digest);
    digest_map.insert("deletion_manifest_set".to_owned(), deletion_set_digest);
    digest_map.insert("error_registry".to_owned(), error_digest);
    digest_map.insert("official_preset_seed_manifest".to_owned(), seed_digest);
    digest_map.insert("package_schema".to_owned(), package_schema_digest);
    digest_map.insert(
        "platform_validation_contract".to_owned(),
        platform_contract_digest,
    );
    digest_map.insert(
        "platform_validation_fixture".to_owned(),
        platform_fixture_digest.clone(),
    );
    digest_map.insert("runtime_feature_inventory".to_owned(), feature_digest);
    digest_map.insert("runtime_protocol".to_owned(), runtime_protocol_digest);
    digest_map.insert(
        "runtime_release_fixture".to_owned(),
        runtime_release_fixture_digest,
    );
    digest_map.insert("rust_contract_schema".to_owned(), rust_contract_schema_digest);
    digest_map.insert("session_event_registry".to_owned(), event_digest);
    digest_map.insert("target_first_party_inventory".to_owned(), inventory_digest);

    let ledger = ArtifactEnvelope::new(ContractDigestLedgerPayload {
        ledger_version: VersionString("1.0.0".to_owned()),
        artifacts: digest_map,
    })?;

    let outputs = BTreeMap::from([
        (
            "schemas.json".to_owned(),
            pretty_json(&schemas)?,
        ),
        (
            "canonical-v4-schema-manifest.envelope.json".to_owned(),
            pretty_json(&canonical_manifest)?,
        ),
        (
            "contract-digest-ledger.envelope.json".to_owned(),
            pretty_json(&ledger)?,
        ),
        (
            "contract-closure.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(closure)?)?,
        ),
        (
            "target-first-party-contributions.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(inventory)?)?,
        ),
        (
            "runtime-feature-inventory.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(feature_inventory)?)?,
        ),
        (
            "official-preset-seed-manifest.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(seed.clone())?)?,
        ),
        (
            "canonical-api-inventory.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(api_inventory)?)?,
        ),
        (
            "session-event-registry.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(event_registry)?)?,
        ),
        (
            "error-registry.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(error_registry)?)?,
        ),
        (
            "deletion-manifest-set.envelope.json".to_owned(),
            pretty_json(&ArtifactEnvelope::new(deletion_digests)?)?,
        ),
        (
            "runtime-release-fixture.envelope.json".to_owned(),
            pretty_json(&runtime_release_envelope)?,
        ),
        (
            "platform-validation-fixture.envelope.json".to_owned(),
            pretty_json(&platform_envelope)?,
        ),
    ]);

    if mode == "write" {
        fs::create_dir_all(&generated)?;
        write_json(&seed_path, &seed)?;
        write_json(&runtime_release_path, &runtime_release)?;
        write_json(&platform_path, &platform)?;
        write_json(&d025_payload_path, &d025_payload)?;
        write_json(&d025_reference_path, &d025_reference)?;
        fs::write(&d025_envelope_path, &d025_envelope_contents)?;
        for (name, contents) in &outputs {
            fs::write(generated.join(name), contents)?;
        }
    } else {
        check_json(&seed_path, &seed)?;
        check_json(&runtime_release_path, &runtime_release)?;
        check_json(&platform_path, &platform)?;
        check_json(&d025_payload_path, &d025_payload)?;
        check_json(&d025_reference_path, &d025_reference)?;
        if fs::read_to_string(&d025_envelope_path)? != d025_envelope_contents {
            return Err(format!(
                "generated artifact drift: {}; run agent-v2-contract write",
                d025_envelope_path.display()
            )
            .into());
        }
        for (name, contents) in &outputs {
            let path = generated.join(name);
            let existing = fs::read_to_string(&path)
                .map_err(|_| format!("missing generated artifact {}", path.display()))?;
            if &existing != contents {
                return Err(format!(
                    "generated artifact drift: {}; run agent-v2-contract write",
                    path.display()
                )
                .into());
            }
        }
    }

    Ok(())
}

fn normalize_runtime_fixture_digests(
    runtime_release: &mut CodexRuntimeReleaseManifestPayload,
) {
    runtime_release.patch_series_digest = fixture_digest("codex-patch-series");
    runtime_release.license_artifact.digest = fixture_digest("license");
    runtime_release.notice_artifact.digest = fixture_digest("notice");
    runtime_release.sbom_artifact.digest = fixture_digest("sbom");

    for (cell_id, target) in &mut runtime_release.target_matrix {
        match target {
            nomifun_agent_contracts::RuntimeReleaseTargetPayload::Required {
                host_artifact,
                sidecar_artifact,
                helper_artifacts,
                package_content_digest,
                capability_availability_digest,
                ..
            } => {
                host_artifact.digest = fixture_digest(&format!("{cell_id}:host"));
                sidecar_artifact.digest = fixture_digest(&format!("{cell_id}:sidecar"));
                for (index, helper) in helper_artifacts.iter_mut().enumerate() {
                    helper.digest = fixture_digest(&format!("{cell_id}:helper:{index}"));
                }
                *package_content_digest = fixture_digest(&format!("{cell_id}:package"));
                *capability_availability_digest =
                    fixture_digest(&format!("{cell_id}:availability"));
            }
            nomifun_agent_contracts::RuntimeReleaseTargetPayload::Unsupported {
                capability_availability_digest,
            }
            | nomifun_agent_contracts::RuntimeReleaseTargetPayload::RemoteOnly {
                capability_availability_digest,
            } => {
                *capability_availability_digest =
                    fixture_digest(&format!("{cell_id}:availability"));
            }
        }
    }
}

fn fixture_digest(label: &str) -> DigestHex {
    digest_bytes(format!("agent-v2-contract-fixture:{label}").as_bytes())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn pretty_json<T: Serialize>(value: &T) -> Result<String, Box<dyn Error>> {
    Ok(format!("{}\n", serde_json::to_string_pretty(value)?))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Box<dyn Error>> {
    fs::write(path, pretty_json(value)?)?;
    Ok(())
}

fn check_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Box<dyn Error>> {
    let expected = pretty_json(value)?;
    let actual = fs::read_to_string(path)?;
    if actual != expected {
        return Err(format!(
            "canonical payload drift: {}; run agent-v2-contract write",
            path.display()
        )
        .into());
    }
    Ok(())
}

fn validate_closure(payload: &ContractClosurePayload) -> Result<(), Box<dyn Error>> {
    if payload.decisions.len() != 28
        || !payload.unresolved_decisions.is_empty()
        || payload.production_behavior_included
        || payload.canonical_sources.len() != 3
    {
        return Err("Contract Closure payload is incomplete".into());
    }
    let ids = payload
        .decisions
        .iter()
        .map(|decision| decision.decision_id.as_str())
        .collect::<BTreeSet<_>>();
    let expected = (1..=28)
        .map(|index| format!("D-{index:03}"))
        .collect::<BTreeSet<_>>();
    if ids != expected.iter().map(String::as_str).collect() {
        return Err("confirmed decision exact-set is not D-001 through D-028".into());
    }
    Ok(())
}

fn validate_target_inventory(
    payload: &TargetPackageInventoryPayload,
) -> Result<(), Box<dyn Error>> {
    let mut packages = BTreeSet::new();
    let mut capabilities = BTreeSet::new();
    for package in &payload.packages {
        if !packages.insert(package.package.id.as_ref().to_owned()) {
            return Err(format!("duplicate target package {}", package.package.id.as_ref()).into());
        }
        if format!("{:?}", package.source.source_kind).to_ascii_lowercase() != "bundled" {
            return Err("target first-party inventory must contain only bundled sources".into());
        }
        for capability in &package.capabilities {
            if !capabilities.insert(capability.capability.id.as_ref().to_owned()) {
                return Err(format!(
                    "duplicate target capability {}",
                    capability.capability.id.as_ref()
                )
                .into());
            }
        }
    }
    if packages.is_empty() || capabilities.is_empty() {
        return Err("target first-party inventory cannot be empty".into());
    }
    Ok(())
}

fn validate_deletion_manifests(
    directory: &Path,
) -> Result<BTreeMap<String, DigestHex>, Box<dyn Error>> {
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut digests = BTreeMap::new();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let manifest: DeletionManifest = read_json(&path)?;
        manifest.validate()?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("invalid deletion manifest filename")?
            .to_owned();
        digests.insert(name, digest_payload(&manifest)?);
    }
    if digests.len() != 6 {
        return Err(format!("expected six deletion manifests, found {}", digests.len()).into());
    }
    Ok(digests)
}

fn generated_schemas() -> Result<BTreeMap<String, Value>, Box<dyn Error>> {
    let mut schemas = BTreeMap::new();
    add_schema::<PackageManifest>(&mut schemas, "package_manifest")?;
    add_schema::<PluginRegistrationMetadata>(&mut schemas, "plugin_registration")?;
    add_schema::<TargetPackageInventoryPayload>(&mut schemas, "target_package_inventory")?;
    add_schema::<AgentPresetRevisionPayload>(&mut schemas, "agent_preset_revision")?;
    add_schema::<OfficialPresetSeedManifestPayload>(&mut schemas, "official_preset_seed")?;
    add_schema::<ResolvedSnapshotContent>(&mut schemas, "resolved_snapshot")?;
    add_schema::<AgentBindingValue>(&mut schemas, "agent_binding")?;
    add_schema::<RemoteBinding>(&mut schemas, "remote_binding")?;
    add_schema::<AgentSessionAggregate>(&mut schemas, "agent_session")?;
    add_schema::<SessionEventRegistryPayload>(&mut schemas, "session_event_registry")?;
    add_schema::<CanonicalErrorRegistryPayload>(&mut schemas, "canonical_error_registry")?;
    add_schema::<RuntimeCommand>(&mut schemas, "runtime_command")?;
    add_schema::<RuntimeHelloPayload>(&mut schemas, "runtime_hello")?;
    add_schema::<CodingRuntimeFeatureInventoryPayload>(
        &mut schemas,
        "runtime_feature_inventory",
    )?;
    add_schema::<FreshV4ParentOperationMarker>(&mut schemas, "fresh_v4_parent_marker")?;
    add_schema::<FreshV4SchemaMetadata>(&mut schemas, "fresh_v4_schema_metadata")?;
    add_schema::<FreshV4ReadyMarker>(&mut schemas, "fresh_v4_ready_marker")?;
    add_schema::<DeletionManifest>(&mut schemas, "deletion_manifest")?;
    add_schema::<PlatformValidationManifestPayload>(
        &mut schemas,
        "platform_validation_manifest",
    )?;
    Ok(schemas)
}

fn add_schema<T: JsonSchema>(
    schemas: &mut BTreeMap<String, Value>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    schemas.insert(name.to_owned(), serde_json::to_value(schema_for!(T))?);
    Ok(())
}

fn schema_subset(
    schemas: &BTreeMap<String, Value>,
    names: &[&str],
) -> BTreeMap<String, Value> {
    names
        .iter()
        .filter_map(|name| schemas.get(*name).cloned().map(|value| ((*name).to_owned(), value)))
        .collect()
}
