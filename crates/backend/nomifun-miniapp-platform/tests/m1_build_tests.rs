use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    canonical_release_artifact_digest, canonical_ui_tree_digest, digest_bytes,
    digest_payload, ArtifactId, CredentialSlotDeclaration, CredentialSlotKey,
    CredentialSlotKind, DigestHex, LocalizedMetadata, MiniAppResourceContract,
    MiniAppServiceLifecycle, PackageId, PackageRef, ResourceKind, StrictJsonValue,
    VersionString,
};
use nomifun_miniapp_platform::{
    build_miniapp_static_bundle, validate_no_custom_scripts,
    validate_static_bundle_path, MiniAppStaticBundleBuildError,
    MiniAppStaticBundleBuilder, MiniAppStaticBundleFile,
    MiniAppStaticBundleInput, MiniAppStaticServiceInput,
    materialize_surface_entrypoint, MINIAPP_SURFACE_BRIDGE_BOOTSTRAP_MARKER,
};
use serde_json::json;

fn digest(value: &[u8]) -> DigestHex {
    digest_bytes(value)
}

fn base_input() -> MiniAppStaticBundleInput {
    MiniAppStaticBundleInput {
        artifact_id: ArtifactId::from("artifact-static-bundle"),
        display: LocalizedMetadata {
            name: "Static Bundle".to_owned(),
            description: "M1 static bundle fixture".to_owned(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        ui_index_html: br#"<!doctype html><html><body>hello</body></html>"#
            .to_vec(),
        ui_assets: vec![MiniAppStaticBundleFile::new(
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
    let artifact = MiniAppStaticBundleBuilder::new()
        .build_ui_only(input)
        .unwrap();
    let manifest = &artifact.manifest.payload;
    let entrypoint = artifact
        .files
        .iter()
        .find(|file| file.normalized_relative_path == "ui/index.html")
        .unwrap();

    assert_eq!(
        MiniAppStaticBundleBuilder::build_profile(),
        nomifun_agent_contracts::JavaScriptBuildProfile::MiniAppReleaseV1
    );
    assert!(!MiniAppStaticBundleBuilder::ui_only_requires_node());
    assert!(manifest.service.is_none());
    assert_eq!(entrypoint.digest, digest(&expected_html));
    assert_eq!(entrypoint.size_bytes, expected_html.len() as u64);
    assert_eq!(
        manifest.ui.ui_tree_digest,
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
    input.service = Some(MiniAppStaticServiceInput {
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
        .any(|file| file.normalized_relative_path == "service/main.mjs"));
}

#[test]
fn empty_files_and_unsafe_paths_fail_before_artifact_creation() {
    let mut empty_ui = base_input();
    empty_ui.ui_index_html.clear();
    assert!(matches!(
        MiniAppStaticBundleBuilder::new().build(empty_ui),
        Err(MiniAppStaticBundleBuildError::EmptyFile { .. })
    ));

    let mut empty_service = base_input();
    empty_service.service = Some(MiniAppStaticServiceInput {
        main_mjs: Vec::new(),
        lifecycle: MiniAppServiceLifecycle::OnDemand,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    });
    assert!(matches!(
        MiniAppStaticBundleBuilder::new().build(empty_service),
        Err(MiniAppStaticBundleBuildError::EmptyFile { .. })
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
        Err(MiniAppStaticBundleBuildError::CustomScriptsForbidden)
    ));
    assert!(matches!(
        validate_no_custom_scripts(Some(
            br#"{"private":true,"scripts":{"postinstall":"node evil.mjs"}}"#,
        )),
        Err(MiniAppStaticBundleBuildError::CustomScriptsForbidden)
    ));
}

#[test]
fn ui_only_entrypoint_rejects_service_input() {
    let mut input = base_input();
    input.service = Some(MiniAppStaticServiceInput {
        main_mjs: b"export default {};\n".to_vec(),
        lifecycle: MiniAppServiceLifecycle::Continuous,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest(b"service-contract"),
        runtime_requirements_digest: digest(b"runtime-requirements"),
    });

    assert!(matches!(
        MiniAppStaticBundleBuilder::new().build_ui_only(input),
        Err(MiniAppStaticBundleBuildError::UiOnlyServiceSource)
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
