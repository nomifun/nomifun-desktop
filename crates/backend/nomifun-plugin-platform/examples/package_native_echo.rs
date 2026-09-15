//! Developer-side packaging example, not a host-side Cargo build service.
//! Usage: cargo run -p nomifun-plugin-platform --example package_native_echo -- <echo-executable> <new-output-directory>
use nomifun_agent_contracts::*;
use nomifun_plugin_platform::runtime::*;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected <echo-executable> <new-output-directory>".into());
    }
    let executable = std::fs::read(&args[0])?;
    let output = PathBuf::from(&args[1]);
    let target = NativePluginTarget::current().ok_or("unsupported native target")?;
    let package = PackageRef {
        id: "plugin.rust-echo".into(),
        version: "1.0.0".into(),
    };
    let display = LocalizedMetadata {
        name: "Rust Echo".into(),
        description: "Native Rust Service capability example".into(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    };
    let schema = StrictJsonValue(json!({"type":"object"}));
    let schema_ref: CanonicalSchemaRef = format!(
        "schema://plugin.rust-echo/object@1#{}",
        digest_payload(&schema.0)?.as_ref()
    )
    .into();
    let capability = CapabilityManifest {
        id: "plugin.rust-echo".into(),
        contribution_id: "contribution:plugin.rust-echo".into(),
        version: "1.0.0".into(),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: display.clone(),
        requires: vec![],
        conflicts: vec![],
        requires_runtime_features: vec![],
        supported_platforms: vec![PlatformConstraint::Any],
        supported_surfaces: capability_surface_declarations(
            ["desktop", "headless"],
            [CapabilityConsumer::Agent, CapabilityConsumer::PluginService],
        ),
        config_schema: schema.clone(),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: "plugin.rust-echo.invoke".into(),
                input_schema: schema_ref.clone(),
                output_schema: schema_ref.clone(),
                effect_class: EffectClass::ReadLocal,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };
    let input = PluginRuntimeStaticBundleInput {
        artifact_id: uuid::Uuid::now_v7().to_string().into(),
        display,
        ui_index_html: vec![],
        ui_assets: vec![],
        service: None,
        package_json: None,
        dependency_lock_digest: digest_bytes(b"native-example-lock-v1"),
        dependency_graph_digest: digest_bytes(b"native-example-graph-v1"),
        config_schema: schema.clone(),
        credential_slots: vec![],
        resource_contract: Default::default(),
        schemas: BTreeMap::from([(schema_ref, schema)]),
        bridge_contract_digest: digest_bytes(b"native-example-bridge-v1"),
        contribution_package: package,
        contributions: PackageContributions {
            capabilities: vec![capability],
            ..Default::default()
        },
        migrations: vec![],
    };
    let materialized = materialize_native_service_release(PluginRuntimeNativeServiceInput {
        executable: executable.clone(),
        target,
        lifecycle: PluginServiceLifecycle::OnDemand,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest_bytes(b"native-example-contract-v1"),
        runtime_requirements_digest: digest_bytes(target.as_str().as_bytes()),
    })?;
    let artifact = build_plugin_native_bundle(
        input,
        PluginRuntimeNativeServiceInput {
            executable,
            target,
            lifecycle: PluginServiceLifecycle::OnDemand,
            uses_files: false,
            uses_private_database: false,
            service_contract_digest: materialized.descriptor.service_contract_digest.clone(),
            runtime_requirements_digest: materialized
                .descriptor
                .runtime_requirements_digest
                .clone(),
        },
    )?;
    // Refuse overwrite; all packaging writes stay in a newly created directory.
    std::fs::create_dir(&output)?;
    let store = PluginRuntimeReleaseStore::new(output.join("store"))?;
    let stored = store
        .publish(PluginRuntimeReleasePublishRequest::service(
            PluginRuntimeSourceScope::new("developer", "rust-echo", "rust-echo")?,
            digest_payload(&artifact.manifest.payload)?,
            artifact.manifest.payload.dependency_lock_digest.clone(),
            1,
            artifact,
            vec![PluginRuntimeReleaseFileBytes::new(
                materialized.file.normalized_relative_path,
                materialized.file.bytes,
            )],
        ))?
        .stored;
    let bundle = PluginRuntimeShareBundleFilesystem::default().export(
        PluginRuntimeShareBundleExport {
            bundle_id: uuid::Uuid::now_v7().to_string().into(),
            source_plugin_product_id: None,
            release: &stored,
            source: None,
            test_provenance: None,
        },
        &output.join("bundle"),
    )?;
    println!(
        "Import prebuilt release: {}",
        output.join("bundle/release").display()
    );
    println!(
        "Artifact digest: {}",
        bundle.release.artifact_digest.as_ref()
    );
    Ok(())
}
