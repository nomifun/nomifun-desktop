use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomifun_agent_contracts::{
    ActionId, CanonicalSchemaRef, CapabilityActionDescriptor, CapabilityConsumer,
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest, EffectClass,
    ExactVersionRef, LocalizedMetadata, PackageContributions, PlatformConstraint, StrictJsonValue,
    ToolPresentationKind, capability_surface_declarations,
};
use nomifun_js_authoring::{
    ContentAddressedNpmCache, DependencyRequestSet, ExactDependencyLock, FixedPluginPacker,
    NeverCancel, NodeBuildHost, NpmRegistryPort, NpmResolver, NpmResolverIdentity,
    PluginLanguage, PluginPackageBuildOptions, PluginProjectId, PluginScaffoldRequest,
    RegistryPackageRelease, SourceScope, SourceStore, SourceStoreLimits, UserId,
    NormalizedSourcePath,
};
use nomifun_js_authoring::{AuthoringError, OperationCancellation};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

fn id() -> String {
    Uuid::now_v7().to_string()
}

fn scope() -> SourceScope {
    SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap()
}

fn request(language: PluginLanguage) -> PluginScaffoldRequest {
    PluginScaffoldRequest {
        package_id: "example.build".into(),
        package_version: "1.0.0".into(),
        display_name: "Build".into(),
        description: "Build test package.".into(),
        language,
    }
}

fn node_executable() -> PathBuf {
    if let Ok(value) = std::env::var("NODE_EXECUTABLE") {
        return PathBuf::from(value);
    }
    if cfg!(windows) {
        PathBuf::from(r"C:\Program Files\nodejs\node.exe")
    } else {
        PathBuf::from("/usr/bin/node")
    }
}

fn fixture() -> (TempDir, SourceStore) {
    let temp = tempfile::tempdir().unwrap();
    let store =
        SourceStore::new(temp.path().join("managed"), SourceStoreLimits::default()).unwrap();
    (temp, store)
}

fn empty_lock(store: &SourceStore, scope: &SourceScope) -> ExactDependencyLock {
    store
        .load_dependency_lock(scope, &NeverCancel)
        .unwrap_or_else(|_| {
            ExactDependencyLock::empty(
                &DependencyRequestSet::empty(),
                NpmResolverIdentity::new("test", "1.0.0").unwrap(),
            )
            .unwrap()
        })
}

fn declare_test_capability(source_root: &std::path::Path) {
    let manifest_path = source_root.join("nomifun.plugin.json");
    let manifest =
        nomifun_js_authoring::PluginSourceManifest::from_canonical_bytes(
            &fs::read(&manifest_path).unwrap(),
        )
        .unwrap();
    let package = ExactVersionRef {
        id: manifest.package_id().clone(),
        version: manifest.package_version().clone(),
    };
    let input_schema = StrictJsonValue(json!({
        "additionalProperties": false,
        "properties": {"value": {}},
        "type": "object"
    }));
    let output_schema = StrictJsonValue(json!({}));
    let input_ref = CanonicalSchemaRef::from(format!(
        "schema://example.build/input@1#{}",
        nomifun_agent_contracts::digest_payload(&input_schema.0)
            .unwrap()
            .as_ref()
    ));
    let output_ref = CanonicalSchemaRef::from(format!(
        "schema://example.build/output@1#{}",
        nomifun_agent_contracts::digest_payload(&output_schema.0)
            .unwrap()
            .as_ref()
    ));
    let capability = CapabilityManifest {
        id: CapabilityId::from("example.build.run"),
        contribution_id: "capability:example.build.run".into(),
        version: "1.0.0".into(),
        kind: CapabilityKind::Tool,
        package,
        display: LocalizedMetadata {
            name: "Build".into(),
            description: "Run the build test.".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({"type": "object"})),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from("example.build.run.invoke"),
                input_schema: input_ref.clone(),
                output_schema: output_ref.clone(),
                effect_class: EffectClass::Pure,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };
    let manifest = manifest
        .with_contributions_and_schemas(
            PackageContributions {
                capabilities: vec![capability],
                ..Default::default()
            },
            BTreeMap::from([
                (input_ref, input_schema),
                (output_ref, output_schema),
            ]),
        )
        .unwrap();
    fs::write(manifest_path, manifest.canonical_bytes().unwrap()).unwrap();
}

#[test]
fn fixed_packer_builds_javascript_and_typescript_with_reproducible_outputs() {
    for language in [PluginLanguage::JavaScript, PluginLanguage::TypeScript] {
        let (_temp, store) = fixture();
        let project = store
            .create_plugin_project(scope(), &request(language), &NeverCancel)
            .unwrap();
        let scope = project.project().scope().clone();
        declare_test_capability(project.project().source_root());
        let source = store.snapshot(&scope, &NeverCancel).unwrap();
        let lock = empty_lock(&store, &scope);
        let host = NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap();
        let packer = FixedPluginPacker::new(host);
        let first_staged = store
            .stage_snapshot(&scope, source.snapshot(), &NeverCancel)
            .unwrap();
        let first = packer
            .pack(
                first_staged,
                &lock,
                &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
                &NeverCancel,
            )
            .unwrap();
        assert!(first.package_root().join("manifest.json").is_file());
        assert!(first.package_root().join("main.mjs").is_file());
        assert_eq!(
            first.manifest().payload.package.entrypoint.as_javascript().unwrap().module_digest,
            *first.main_digest()
        );
        assert_eq!(
            first.manifest().payload.minimum_node_major,
            24
        );
        assert_eq!(first.source_map_written(), language == PluginLanguage::TypeScript);
        let first_digest = first.manifest().payload_digest.clone();
        drop(first);

        let second_staged = store
            .stage_snapshot(&scope, source.snapshot(), &NeverCancel)
            .unwrap();
        let second = packer
            .pack(
                second_staged,
                &lock,
                &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
                &NeverCancel,
            )
            .unwrap();
        assert_eq!(second.manifest().payload_digest, first_digest);
    }
}

#[test]
fn fixed_packer_bundles_static_local_modules() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(
            scope(),
            &request(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    let source = project.project().source_root().join("src/main.js");
    fs::write(
        &source,
        "import './helper.js';\nexport async function activate() {}\n",
    )
    .unwrap();
    fs::write(
        project.project().source_root().join("src/helper.js"),
        "export const helper = true;\n",
    )
    .unwrap();
    let scope = project.project().scope().clone();
    declare_test_capability(project.project().source_root());
    let snapshot = store.snapshot(&scope, &NeverCancel).unwrap();
    let lock = empty_lock(&store, &scope);
    let packer = FixedPluginPacker::new(
        NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap(),
    );
    let packed = packer
        .pack(
            store
                .stage_snapshot(&scope, snapshot.snapshot(), &NeverCancel)
                .unwrap(),
            &lock,
            &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
            &NeverCancel,
        )
        .unwrap();
    assert!(
        fs::read_to_string(packed.package_root().join("main.mjs"))
            .unwrap()
            .contains("__require")
    );
}

#[test]
fn fixed_packer_rejects_undeclared_import_and_cleans_staging() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(
            scope(),
            &request(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    declare_test_capability(project.project().source_root());
    fs::write(
        project.project().source_root().join("src/main.js"),
        "import { missing } from 'not-locked';\nexport async function activate() { return missing; }\n",
    )
    .unwrap();
    let scope = project.project().scope().clone();
    let snapshot = store.snapshot(&scope, &NeverCancel).unwrap();
    let operation_root;
    let result = {
        let staged = store
            .stage_snapshot(&scope, snapshot.snapshot(), &NeverCancel)
            .unwrap();
        operation_root = staged.operation_root().to_path_buf();
        FixedPluginPacker::new(
            NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap(),
        )
        .pack(
            staged,
            &empty_lock(&store, &scope),
            &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
            &NeverCancel,
        )
    };
    assert!(matches!(
        result,
        Err(AuthoringError::LocalModuleUnsupported { .. })
    ));
    assert!(!operation_root.exists());
}

#[test]
fn fixed_packer_preserves_node_public_api_static_imports() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(
            scope(),
            &request(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    declare_test_capability(project.project().source_root());
    fs::write(
        project.project().source_root().join("src/main.js"),
        "import { basename } from 'node:path';\nexport async function activate() { return basename('/tmp/value.txt'); }\n",
    )
    .unwrap();
    let scope = project.project().scope().clone();
    let captured = store.snapshot(&scope, &NeverCancel).unwrap();
    let packed = FixedPluginPacker::new(
        NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap(),
    )
    .pack(
        store
            .stage_snapshot(&scope, captured.snapshot(), &NeverCancel)
            .unwrap(),
        &empty_lock(&store, &scope),
        &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
        &NeverCancel,
    )
    .unwrap();
    let script = "import { pathToFileURL } from 'node:url'; \
        import(pathToFileURL(process.argv[1]).href).then(async m => \
        console.log(await m.activate({})))";
    let output = Command::new(node_executable())
        .arg("--input-type=module")
        .arg("-e")
        .arg(script)
        .arg(packed.package_root().join("main.mjs"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "value.txt");
}

#[test]
fn diamond_importers_share_cached_exact_exports() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(
            scope(),
            &request(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    declare_test_capability(project.project().source_root());
    fs::write(
        project.project().source_root().join("src/main.js"),
        "import { left } from './left.js';\nimport { right } from './right.js';\nexport async function activate() { return left + right; }\n",
    )
    .unwrap();
    fs::write(
        project.project().source_root().join("src/shared.js"),
        "export const shared = 2;\n",
    )
    .unwrap();
    fs::write(
        project.project().source_root().join("src/left.js"),
        "import { shared } from './shared.js';\nexport const left = shared;\n",
    )
    .unwrap();
    fs::write(
        project.project().source_root().join("src/right.js"),
        "import { shared } from './shared.js';\nexport const right = shared;\n",
    )
    .unwrap();
    let scope = project.project().scope().clone();
    let captured = store.snapshot(&scope, &NeverCancel).unwrap();
    FixedPluginPacker::new(
        NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap(),
    )
    .pack(
        store
            .stage_snapshot(&scope, captured.snapshot(), &NeverCancel)
            .unwrap(),
        &empty_lock(&store, &scope),
        &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
        &NeverCancel,
    )
    .unwrap();
}

#[test]
fn multi_binding_export_fails_closed_and_cleans_staging() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(
            scope(),
            &request(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    declare_test_capability(project.project().source_root());
    fs::write(
        project.project().source_root().join("src/main.js"),
        "export const a = 1, b = 2;\nexport async function activate() { return a + b; }\n",
    )
    .unwrap();
    let scope = project.project().scope().clone();
    let captured = store.snapshot(&scope, &NeverCancel).unwrap();
    let staged = store
        .stage_snapshot(&scope, captured.snapshot(), &NeverCancel)
        .unwrap();
    let operation_root = staged.operation_root().to_path_buf();
    let result = FixedPluginPacker::new(
        NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap(),
    )
    .pack(
        staged,
        &empty_lock(&store, &scope),
        &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
        &NeverCancel,
    );
    assert!(matches!(result, Err(AuthoringError::PackRejected(_))));
    assert!(!operation_root.exists());
}

#[derive(Clone)]
struct FakeRegistry {
    releases: Arc<Mutex<BTreeMap<String, RegistryPackageRelease>>>,
}

impl NpmRegistryPort for FakeRegistry {
    fn resolve(
        &self,
        package_name: &str,
        _requirement: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<RegistryPackageRelease, AuthoringError> {
        if cancellation.is_cancelled() {
            return Err(AuthoringError::Canceled);
        }
        self.releases
            .lock()
            .unwrap()
            .get(package_name)
            .cloned()
            .ok_or_else(|| AuthoringError::Registry(package_name.into()))
    }
}

fn registry_package(
    name: &str,
    version: &str,
    dependencies: &[(&str, &str)],
) -> RegistryPackageRelease {
    registry_package_source(
        name,
        version,
        dependencies,
        "export const value = 1;\n",
    )
}

fn registry_package_source(
    name: &str,
    version: &str,
    dependencies: &[(&str, &str)],
    source: &str,
) -> RegistryPackageRelease {
    let dependency_json = dependencies
        .iter()
        .map(|(name, requirement)| (name.to_string(), (*requirement).to_string()))
        .collect::<BTreeMap<_, _>>();
    let package_json = serde_json::json!({
        "name": name,
        "version": version,
        "type": "module",
        "main": "index.js",
        "dependencies": dependency_json,
        "license": "MIT",
        "author": {"name": "Registry fixture"},
        "repository": {"type": "git", "url": "https://example.invalid/fixture.git"},
        "devDependencies": {"eslint": "9.0.0"},
        "scripts": {"test": "node test.js"}
    });
    RegistryPackageRelease::new(
        name,
        version,
        "sha512-YWJjZA==",
        [
            (
                NormalizedSourcePath::parse("package.json").unwrap(),
                serde_json::to_vec(&package_json).unwrap(),
            ),
            (
                NormalizedSourcePath::parse("index.js").unwrap(),
                source.as_bytes().to_vec(),
            ),
        ],
    )
    .unwrap()
}

#[test]
fn fixed_packer_bundles_realistic_registry_metadata_and_transitive_pure_javascript() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(
            scope(),
            &request(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    declare_test_capability(project.project().source_root());
    fs::write(
        project.project().source_root().join("package.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": "example.build",
            "version": "1.0.0",
            "description": "Build test package.",
            "private": true,
            "type": "module",
            "dependencies": {"alpha": "1.0.0"}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        project.project().source_root().join("src/main.js"),
        "import { value } from 'alpha';\nexport async function activate() { return value; }\n",
    )
    .unwrap();
    let scope = project.project().scope().clone();
    let captured = store.snapshot(&scope, &NeverCancel).unwrap();
    let cache = ContentAddressedNpmCache::new(store.managed_root().join("npm-cache")).unwrap();
    let releases = BTreeMap::from([
        (
            "alpha".into(),
            registry_package_source(
                "alpha",
                "1.0.0",
                &[("shared", "1.0.0")],
                "import { value as shared } from 'shared';\nexport const value = shared + 1;\n",
            ),
        ),
        (
            "shared".into(),
            registry_package_source("shared", "1.0.0", &[], "export const value = 1;\n"),
        ),
    ]);
    let registry = FakeRegistry {
        releases: Arc::new(Mutex::new(releases)),
    };
    let resolver = NpmResolver::new(
        NpmResolverIdentity::new("fake", "1.0.0").unwrap(),
        registry,
        cache.clone(),
    );
    let lock = resolver.resolve(captured.dependency_requests(), &NeverCancel).unwrap();
    let packed = FixedPluginPacker::new(
        NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap(),
    )
    .with_npm_cache(cache.clone())
    .pack(
        store
            .stage_snapshot(&scope, captured.snapshot(), &NeverCancel)
            .unwrap(),
        &lock,
        &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
        &NeverCancel,
    )
    .unwrap();
    let main = packed.package_root().join("main.mjs");
    let script = "import { pathToFileURL } from 'node:url'; \
        import(pathToFileURL(process.argv[1]).href).then(async m => \
        console.log(await m.activate({})))";
    let output = Command::new(node_executable())
        .arg("--input-type=module")
        .arg("-e")
        .arg(script)
        .arg(&main)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "2");
}

#[test]
fn offline_resolver_uses_content_addressed_cache_and_exact_lock() {
    let temp = tempfile::tempdir().unwrap();
    let cache = ContentAddressedNpmCache::new(temp.path().join("cache")).unwrap();
    let releases = BTreeMap::from([
        (
            "alpha".into(),
            registry_package("alpha", "1.0.0", &[("shared", "1.0.0")]),
        ),
        ("shared".into(), registry_package("shared", "1.0.0", &[])),
    ]);
    let registry = FakeRegistry {
        releases: Arc::new(Mutex::new(releases)),
    };
    let resolver = NpmResolver::new(
        NpmResolverIdentity::new("fake", "1.0.0").unwrap(),
        registry.clone(),
        cache.clone(),
    );
    let requests =
        DependencyRequestSet::new([("alpha".into(), "1.0.0".into())]).unwrap();
    let first = resolver.resolve(&requests, &NeverCancel).unwrap();
    let second = resolver.resolve(&requests, &NeverCancel).unwrap();
    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
    assert_eq!(first.packages().len(), 2);
    let alpha = first.packages().get("alpha@1.0.0").unwrap();
    let cached = cache.load(alpha.archive_sha256()).unwrap();
    assert_eq!(cached.package_json_digest(), alpha.package_json_sha256());
    fs::write(
        cached.object_root().join("files/index.js"),
        b"tampered",
    )
    .unwrap();
    assert!(cache.load(alpha.archive_sha256()).is_err());
}

#[test]
fn resolver_rejects_a_transitive_graph_beyond_the_fixed_depth_budget() {
    let temp = tempfile::tempdir().unwrap();
    let cache = ContentAddressedNpmCache::new(temp.path().join("cache")).unwrap();
    let mut releases = BTreeMap::new();
    for index in 0..65 {
        let name = format!("depth-{index}");
        let next = format!("depth-{}", index + 1);
        let dependencies = if index < 64 {
            vec![(next.as_str(), "1.0.0")]
        } else {
            Vec::new()
        };
        releases.insert(
            name.clone(),
            registry_package(&name, "1.0.0", &dependencies),
        );
    }
    let resolver = NpmResolver::new(
        NpmResolverIdentity::new("fake", "1.0.0").unwrap(),
        FakeRegistry {
            releases: Arc::new(Mutex::new(releases)),
        },
        cache,
    );
    let requests =
        DependencyRequestSet::new([("depth-0".into(), "1.0.0".into())]).unwrap();

    let error = resolver.resolve(&requests, &NeverCancel).unwrap_err();
    assert!(matches!(error, AuthoringError::Registry(message) if message.contains("depth limit")));
}

#[test]
fn resolver_rejects_more_than_the_fixed_package_budget() {
    let temp = tempfile::tempdir().unwrap();
    let cache = ContentAddressedNpmCache::new(temp.path().join("cache")).unwrap();
    let mut releases = BTreeMap::new();
    let mut direct = BTreeMap::new();
    for index in 0..257 {
        let name = format!("breadth-{index}");
        direct.insert(name.clone(), "1.0.0".to_owned());
        releases.insert(name.clone(), registry_package(&name, "1.0.0", &[]));
    }
    let resolver = NpmResolver::new(
        NpmResolverIdentity::new("fake", "1.0.0").unwrap(),
        FakeRegistry {
            releases: Arc::new(Mutex::new(releases)),
        },
        cache,
    );
    let requests = DependencyRequestSet::new(direct).unwrap();

    let error = resolver.resolve(&requests, &NeverCancel).unwrap_err();
    assert!(matches!(error, AuthoringError::Registry(message) if message.contains("package limit")));
}

#[test]
fn canceled_pack_drops_staging_without_starting_build_host() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(
            scope(),
            &request(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    declare_test_capability(project.project().source_root());
    let scope = project.project().scope().clone();
    let captured = store.snapshot(&scope, &NeverCancel).unwrap();
    let staged = store
        .stage_snapshot(&scope, captured.snapshot(), &NeverCancel)
        .unwrap();
    let operation_root = staged.operation_root().to_path_buf();
    let cancellation = nomifun_js_authoring::CancellationFlag::default();
    cancellation.cancel();
    let result = FixedPluginPacker::new(
        NodeBuildHost::new(node_executable(), Duration::from_secs(10)).unwrap(),
    )
    .pack(
        staged,
        &ExactDependencyLock::empty(
            &DependencyRequestSet::empty(),
            NpmResolverIdentity::new("test", "1.0.0").unwrap(),
        )
        .unwrap(),
        &PluginPackageBuildOptions::for_target("x86_64-pc-windows-msvc"),
        &cancellation,
    );
    assert!(matches!(result, Err(AuthoringError::Canceled)));
    assert!(!operation_root.exists());
}

#[test]
fn registry_rejects_commonjs_lifecycle_and_native_addon_inputs() {
    for (package_json, file_name) in [
        (
            serde_json::json!({
                "name": "bad",
                "version": "1.0.0"
            }),
            "index.js",
        ),
        (
            serde_json::json!({
                "name": "bad",
                "version": "1.0.0",
                "type": "module",
                "scripts": {"postinstall": "evil"}
            }),
            "index.js",
        ),
        (
            serde_json::json!({
                "name": "bad",
                "version": "1.0.0",
                "type": "module",
                "gypfile": true
            }),
            "index.js",
        ),
        (
            serde_json::json!({
                "name": "bad",
                "version": "1.0.0",
                "type": "module"
            }),
            "addon.node",
        ),
    ] {
        let result = RegistryPackageRelease::new(
            "bad",
            "1.0.0",
            "sha512-YWJjZA==",
            [
                (
                    NormalizedSourcePath::parse("package.json").unwrap(),
                    serde_json::to_vec(&package_json).unwrap(),
                ),
                (
                    NormalizedSourcePath::parse(file_name).unwrap(),
                    b"native".to_vec(),
                ),
            ],
        );
        assert!(result.is_err());
    }
}
