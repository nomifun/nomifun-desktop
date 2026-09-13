use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    canonical_release_artifact_digest, canonical_ui_tree_digest, digest_bytes,
    digest_payload, ArtifactId, CredentialSlotDeclaration, CredentialSlotKey,
    CredentialSlotKind, DigestHex, LocalizedMetadata, MiniAppResourceContract,
    MiniAppServiceLifecycle, PackageId, PackageRef, ResourceKind, StrictJsonValue,
    VersionString,
};
use nomifun_plugin_platform::runtime::{
    build_miniapp_static_bundle, validate_no_custom_scripts,
    validate_service_source, validate_static_bundle_path,
    materialize_service_release, PluginRuntimeStaticBundleBuildError,
    PluginRuntimeStaticBundleBuilder, PluginRuntimeStaticBundleFile,
    PluginRuntimeStaticBundleInput, PluginRuntimeStaticServiceInput,
    materialize_surface_entrypoint, MINIAPP_SERVICE_ENTRYPOINT,
    MINIAPP_SURFACE_BRIDGE_BOOTSTRAP_MARKER,
};
use serde_json::json;

fn digest(value: &[u8]) -> DigestHex {
    digest_bytes(value)
}

fn base_input() -> PluginRuntimeStaticBundleInput {
    PluginRuntimeStaticBundleInput {
        artifact_id: ArtifactId::from("artifact-static-bundle"),
        display: LocalizedMetadata {
            name: "Static Bundle".to_owned(),
            description: "M1 static bundle fixture".to_owned(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        ui_index_html: br#"<!doctype html><html><body>hello</body></html>"#
            .to_vec(),
        ui_assets: vec![PluginRuntimeStaticBundleFile::new(
            "ui/app.js",
            b"export const ready = true;\n".to_vec(),
        )],
        service: None,
        package_json: Some(
            br#"{"private":true,"type":"module","scripts":{}}"#.to_vec(),
        ),
        dependency_lock_digest: digest(b"empty-lock"),
        dependency_graph_digest: digest(b"empty-graph"),
        config_schema: StrictJsonValue(json!({
            "additionalProperties": false,
            "properties": {
                "workspace": {"type": "string"}
            },
            "type": "object"
        })),
        credential_slots: Vec::new(),
        resource_contract: MiniAppResourceContract::default(),
        schemas: BTreeMap::new(),
        bridge_contract_digest: digest(b"bridge-contract"),
        contribution_package: PackageRef {
            id: PackageId::from("miniapp.example.static"),
            version: VersionString::from("1.0.0"),
        },
        contributions: Default::default(),
        migrations: Vec::new(),
    }
}

#[test]
fn ui_only_build_is_deterministic_and_declares_no_node_runtime() {
    let input = base_input();
    let expected_html = materialize_surface_entrypoint(&input.ui_index_html).unwrap();
    let artifact = PluginRuntimeStaticBundleBuilder::new()
        .build_ui_only(input)
        .unwrap();
    let manifest = &artifact.manifest.payload;
    let entrypoint = artifact
        .files
        .iter()
        .find(|file| file.normalized_relative_path == "ui/index.html")
        .unwrap();

    assert_eq!(
        PluginRuntimeStaticBundleBuilder::build_profile(),
        nomifun_agent_contracts::JavaScriptBuildProfile::MiniAppReleaseV1
    );
    assert!(!PluginRuntimeStaticBundleBuilder::ui_only_requires_node());
    assert!(manifest.service.is_none());
    assert_eq!(entrypoint.digest, digest(&expected_html));
    assert_eq!(entrypoint.size_bytes, expected_html.len() as u64);
    assert_eq!(
        manifest.ui.as_ref().unwrap().ui_tree_digest,
        canonical_ui_tree_digest(&artifact.files).unwrap()
    );
    assert_eq!(
        manifest.config_schema_digest,
        digest_payload(&manifest.config_schema.0).unwrap()
    );
    assert_eq!(
        manifest.credential_slots_digest,
        digest_payload(&manifest.credential_slots).unwrap()
    );
    assert_eq!(
        manifest.resource_contract_digest,
        digest_payload(&manifest.resource_contract).unwrap()
    );
    assert_eq!(
        artifact.artifact_digest,
        canonical_release_artifact_digest(
            &artifact.manifest,
            &artifact.files
        )
        .unwrap()
    );
}

#[test]
fn service_bytes_are_packaged_as_a_declared_module_without_starting_runtime() {
    let mut input = base_input();
    let service_bytes = b"export default async function run() {}\n".to_vec();
    let service_digest = digest(&service_bytes);
    input.service = Some(PluginRuntimeStaticServiceInput {
        main_mjs: service_bytes,
        lifecycle: MiniAppServiceLifecycle::OnDemand,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    });
    input.credential_slots = vec![CredentialSlotDeclaration {
        slot_key: CredentialSlotKey::from("api_key"),
        kind: CredentialSlotKind::SecretText,
        display_name: "API key".to_owned(),
        required: true,
    }];
    input.resource_contract.required_resource_kinds =
        [ResourceKind::from("knowledge.base")].into_iter().collect();

    let artifact = build_miniapp_static_bundle(input).unwrap();
    let manifest = &artifact.manifest.payload;
    let service = manifest.service.as_ref().unwrap();

    assert_eq!(service.entrypoint, "service/main.mjs");
    assert_eq!(service.module_digest, service_digest);
    assert_eq!(
        service.host_protocol_version.as_ref(),
        "1.0.0"
    );
    assert_eq!(service.sdk_contract_version.as_ref(), "1.0.0");
    assert_eq!(
        manifest.credential_slots_digest,
        digest_payload(&manifest.credential_slots).unwrap()
    );
    assert_eq!(
        manifest.resource_contract_digest,
        digest_payload(&manifest.resource_contract).unwrap()
    );
    assert!(artifact
        .files
        .iter()
        .any(|file| file.normalized_relative_path == MINIAPP_SERVICE_ENTRYPOINT));
}

#[test]
fn service_release_does_not_require_a_dummy_page() {
    let mut input = base_input();
    input.ui_index_html.clear();
    input.ui_assets.clear();
    input.service = Some(PluginRuntimeStaticServiceInput {
        main_mjs: b"export async function start() { return { async invoke() { return {}; } }; }".to_vec(),
        lifecycle: MiniAppServiceLifecycle::Continuous,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    });
    let artifact = build_miniapp_static_bundle(input.clone()).unwrap();
    assert!(artifact.manifest.payload.ui.is_none());
    assert!(artifact.manifest.payload.service.is_some());
    assert_eq!(artifact.files.len(), 1);
    artifact.validate().unwrap();
    input.ui_assets.push(PluginRuntimeStaticBundleFile::new("ui/orphan.js", b"orphan".to_vec()));
    assert!(build_miniapp_static_bundle(input).is_err());
}

#[test]
fn service_materialization_binds_descriptor_and_file_to_the_same_bytes() {
    let service_bytes = b"export async function start() {}\n".to_vec();
    let materialized = materialize_service_release(PluginRuntimeStaticServiceInput {
        main_mjs: service_bytes.clone(),
        lifecycle: MiniAppServiceLifecycle::Continuous,
        uses_files: true,
        uses_private_database: true,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    })
    .unwrap();

    assert_eq!(
        materialized.file.normalized_relative_path,
        MINIAPP_SERVICE_ENTRYPOINT
    );
    assert_eq!(materialized.file.bytes, service_bytes);
    assert_eq!(
        materialized.descriptor.entrypoint,
        MINIAPP_SERVICE_ENTRYPOINT
    );
    assert_eq!(
        materialized.descriptor.module_digest,
        digest(materialized.file.bytes.as_slice())
    );
    assert_eq!(
        materialized.descriptor.lifecycle,
        MiniAppServiceLifecycle::Continuous
    );
    assert!(materialized.descriptor.uses_files);
    assert!(materialized.descriptor.uses_private_database);
}

#[test]
fn service_source_validation_rejects_empty_invalid_utf8_and_nul() {
    assert!(matches!(
        validate_service_source(&[]),
        Err(PluginRuntimeStaticBundleBuildError::EmptyFile { path })
            if path == MINIAPP_SERVICE_ENTRYPOINT
    ));
    assert!(matches!(
        validate_service_source(&[0xff, 0xfe]),
        Err(PluginRuntimeStaticBundleBuildError::InvalidServiceSourceEncoding)
    ));
    assert!(matches!(
        validate_service_source(b"export const value = '\0';"),
        Err(PluginRuntimeStaticBundleBuildError::InvalidServiceSourceContent)
    ));
}

#[test]
fn empty_files_and_unsafe_paths_fail_before_artifact_creation() {
    let mut empty_ui = base_input();
    empty_ui.ui_index_html.clear();
    assert!(matches!(
        PluginRuntimeStaticBundleBuilder::new().build(empty_ui),
        Err(PluginRuntimeStaticBundleBuildError::EmptyFile { .. })
    ));

    let mut empty_service = base_input();
    empty_service.service = Some(PluginRuntimeStaticServiceInput {
        main_mjs: Vec::new(),
        lifecycle: MiniAppServiceLifecycle::OnDemand,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    });
    assert!(matches!(
        PluginRuntimeStaticBundleBuilder::new().build(empty_service),
        Err(PluginRuntimeStaticBundleBuildError::EmptyFile { .. })
    ));

    for path in [
        "../escape.js",
        "ui/../escape.js",
        "/ui/app.js",
        r"ui\app.js",
        "ui//app.js",
        "ui/NUL.js",
    ] {
        assert!(
            validate_static_bundle_path(path).is_err(),
            "path should be rejected: {path}"
        );
    }
}

#[test]
fn package_scripts_are_rejected_but_empty_scripts_are_allowed() {
    validate_no_custom_scripts(Some(
        br#"{"private":true,"scripts":{}}"#,
    ))
    .unwrap();

    assert!(matches!(
        validate_no_custom_scripts(Some(
            br#"{"private":true,"scripts":{"build":"vite"}}"#,
        )),
        Err(PluginRuntimeStaticBundleBuildError::CustomScriptsForbidden)
    ));
    assert!(matches!(
        validate_no_custom_scripts(Some(
            br#"{"private":true,"scripts":{"postinstall":"node evil.mjs"}}"#,
        )),
        Err(PluginRuntimeStaticBundleBuildError::CustomScriptsForbidden)
    ));
}

#[test]
fn ui_only_entrypoint_rejects_service_input() {
    let mut input = base_input();
    input.service = Some(PluginRuntimeStaticServiceInput {
        main_mjs: b"export default {};\n".to_vec(),
        lifecycle: MiniAppServiceLifecycle::Continuous,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    });

    assert!(matches!(
        PluginRuntimeStaticBundleBuilder::new().build_ui_only(input),
        Err(PluginRuntimeStaticBundleBuildError::UiOnlyServiceSource)
    ));
}

#[test]
fn every_release_entrypoint_gets_one_host_bridge_bootstrap() {
    let source = br#"<!doctype html><html><head></head><body>custom</body></html>"#;
    let first = materialize_surface_entrypoint(source).unwrap();
    let second = materialize_surface_entrypoint(&first).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        String::from_utf8_lossy(&first)
            .matches(MINIAPP_SURFACE_BRIDGE_BOOTSTRAP_MARKER)
            .count(),
        1
    );
}
