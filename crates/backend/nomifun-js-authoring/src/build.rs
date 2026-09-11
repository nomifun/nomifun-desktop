use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use nomifun_agent_contracts::{
    ArtifactEnvelope, DigestHex, JAVASCRIPT_HOST_PROTOCOL_VERSION,
    JAVASCRIPT_SDK_CONTRACT_VERSION, JavaScriptBuildProfile,
    JavaScriptEntrypointMetadata, MINIMUM_NODE_MAJOR, PLUGIN_N1_SCHEMA_VERSION,
    PLUGIN_PACKAGE_PROFILE_VERSION, PackageEntrypointMetadata, PackageManifest,
    PluginPackageV1Manifest, RuntimeTarget, canonical_json_bytes,
    digest_bytes,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::canonical::strict_json_from_slice;
use crate::dependency::ExactDependencyLock;
use crate::error::{AuthoringError, io_error};
use crate::manifest::{
    PLUGIN_SOURCE_MANIFEST_FILE, PluginLanguage, PluginSourceManifest,
};
use crate::model::{OperationCancellation, check_canceled};
use crate::path::NormalizedSourcePath;
use crate::npm::{
    CachedNpmPackage, ContentAddressedNpmCache,
    validate_registry_package_json,
};
use crate::snapshot::CapturedSource;
use crate::store::StagedSource;

const BUILD_HOST_SOURCE: &str = include_str!("../assets/build-host.mjs");
const BUILD_REQUEST_VERSION: &str = "1.0.0";
const MAX_BUILD_DIAGNOSTIC_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NodeBuildHost {
    node_executable: PathBuf,
    timeout: Duration,
    poll_interval: Duration,
    host_source: &'static str,
}

impl NodeBuildHost {
    pub fn new(
        node_executable: impl AsRef<Path>,
        timeout: Duration,
    ) -> Result<Self, AuthoringError> {
        if timeout.is_zero() {
            return Err(AuthoringError::InvalidField {
                field: "build_timeout",
                reason: "must be non-zero".into(),
            });
        }
        let node_executable = require_absolute_regular_file(node_executable.as_ref())?;
        Ok(Self {
            node_executable,
            timeout,
            poll_interval: Duration::from_millis(10),
            host_source: BUILD_HOST_SOURCE,
        })
    }

    pub fn node_executable(&self) -> &Path {
        &self.node_executable
    }

    pub fn validate_foundation(&self) -> Result<(), AuthoringError> {
        let root = std::env::temp_dir()
            .join("nomifun-build-foundation")
            .join(Uuid::now_v7().to_string());
        fs::create_dir_all(&root).map_err(|error| io_error(&root, error))?;
        let cleanup = BuildFoundationCleanup(root.clone());
        let source_path = root.join("foundation.js");
        let output_path = root.join("main.mjs");
        let map_path = root.join("main.mjs.map");
        write_new(
            &source_path,
            b"export const nomifunBuildFoundation = true;\n",
        )?;
        let output = self.run_build_host(
            &root,
            &source_path,
            &output_path,
            &map_path,
            PluginLanguage::JavaScript,
            "nomifun-source:///runtime-foundation",
            &crate::NeverCancel,
        )?;
        if output.source_map_path.is_some()
            || fs::read(&output.main_path)
                .map_err(|error| io_error(&output.main_path, error))?
                != b"export const nomifunBuildFoundation = true;\n"
        {
            return Err(AuthoringError::BuildHostFailed {
                code: None,
                stderr: "Build Foundation output differs from the exact input"
                    .to_owned(),
            });
        }
        drop(cleanup);
        Ok(())
    }

    fn build_bundle(
        &self,
        staged: &StagedSource,
        bundle_path: &Path,
        language: PluginLanguage,
        cancellation: &dyn OperationCancellation,
    ) -> Result<BuildHostOutput, AuthoringError> {
        let output_path = staged.output_root().join("main.mjs");
        let map_path = staged.output_root().join("main.mjs.map");
        self.run_build_host(
            staged.operation_root(),
            bundle_path,
            &output_path,
            &map_path,
            language,
            "nomifun-source:///bundle-entry",
            cancellation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_build_host(
        &self,
        operation_root: &Path,
        source_path: &Path,
        output_path: &Path,
        map_path: &Path,
        language: PluginLanguage,
        source_url: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<BuildHostOutput, AuthoringError> {
        check_canceled(cancellation)?;
        let host_path = operation_root.join("build-host.mjs");
        let request_path = operation_root.join("build-request.json");
        let response_path = operation_root.join("build-response.json");
        let stderr_path = operation_root.join("build-stderr.log");
        write_new(&host_path, self.host_source.as_bytes())?;
        let request = BuildHostRequest {
            format_version: BUILD_REQUEST_VERSION,
            language,
            source_path: node_visible_path(source_path),
            output_path: node_visible_path(&output_path),
            map_path: node_visible_path(&map_path),
            source_url: source_url.to_owned(),
        };
        write_new(
            &request_path,
            &serde_json::to_vec(&request)
                .map_err(|error| AuthoringError::CanonicalSerialization(error.to_string()))?,
        )?;
        let stderr = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stderr_path)
            .map_err(|error| io_error(&stderr_path, error))?;

        // `run_build_host` is a synchronous API and is also called from an
        // async Runtime validation path. Give the shared process-tree owner a
        // dedicated current-thread Tokio runtime instead of nesting block_on
        // in the caller's runtime or reimplementing taskkill/killpg here.
        let status = thread::scope(|scope| {
            scope
                .spawn(|| {
                    run_managed_build_process(
                        &self.node_executable,
                        operation_root,
                        &host_path,
                        &request_path,
                        &response_path,
                        stderr,
                        self.timeout,
                        self.poll_interval,
                        cancellation,
                    )
                })
                .join()
        })
        .map_err(|_| {
            AuthoringError::BuildHostUnavailable(
                "Plugin Build Host process owner panicked".into(),
            )
        })??;

        let stderr = read_bounded_diagnostic(&stderr_path);
        let response = fs::read(&response_path)
            .ok()
            .and_then(|bytes| strict_json_from_slice::<BuildHostResponse>(&bytes).ok());
        match response.filter(BuildHostResponse::has_supported_version) {
            Some(BuildHostResponse::Ok {
                source_map_written,
                ..
            }) if status.success() => Ok(BuildHostOutput {
                main_path: output_path.to_path_buf(),
                source_map_path: source_map_written
                    .then(|| map_path.to_path_buf()),
            }),
            Some(BuildHostResponse::UnsupportedModule { specifier, .. }) => {
                Err(AuthoringError::LocalModuleUnsupported {
                    path: "bundle".into(),
                    reason: format!(
                        "module specifier {specifier:?} requires a trusted fixed bundler"
                    ),
                })
            }
            Some(BuildHostResponse::Failed { message, .. }) => {
                Err(AuthoringError::BuildHostFailed {
                    code: status.code(),
                    stderr: bounded_message(&format!("{message}\n{stderr}")),
                })
            }
            _ => Err(AuthoringError::BuildHostFailed {
                code: status.code(),
                stderr: bounded_message(&stderr),
            }),
        }
    }
}

struct BuildFoundationCleanup(PathBuf);

impl Drop for BuildFoundationCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Debug)]
pub struct PluginPackageBuildOptions {
    pub supported_targets: BTreeSet<RuntimeTarget>,
}

impl PluginPackageBuildOptions {
    pub fn for_target(target: impl Into<RuntimeTarget>) -> Self {
        Self {
            supported_targets: BTreeSet::from([target.into()]),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FixedPluginPacker {
    build_host: NodeBuildHost,
    npm_cache: Option<ContentAddressedNpmCache>,
}

impl FixedPluginPacker {
    pub fn new(build_host: NodeBuildHost) -> Self {
        Self {
            build_host,
            npm_cache: None,
        }
    }

    pub fn with_npm_cache(mut self, npm_cache: ContentAddressedNpmCache) -> Self {
        self.npm_cache = Some(npm_cache);
        self
    }

    pub fn pack(
        &self,
        staged: StagedSource,
        dependency_lock: &ExactDependencyLock,
        options: &PluginPackageBuildOptions,
        cancellation: &dyn OperationCancellation,
    ) -> Result<PackedPluginPackage, AuthoringError> {
        check_canceled(cancellation)?;
        dependency_lock.validate_against(staged.capture().dependency_requests())?;
        if options.supported_targets.is_empty() {
            return Err(AuthoringError::PackRejected(
                "at least one supported runtime target is required".into(),
            ));
        }

        let source_manifest_path = staged.source_root().join(PLUGIN_SOURCE_MANIFEST_FILE);
        let source_manifest = PluginSourceManifest::from_canonical_bytes(
            &fs::read(&source_manifest_path)
                .map_err(|error| io_error(&source_manifest_path, error))?,
        )?;
        validate_source_inventory(&staged, &source_manifest)?;
        let entrypoint_path = source_manifest.entrypoint().join(staged.source_root());
        require_regular_contained(staged.source_root(), &entrypoint_path)?;
        validate_package_json_identity(
            staged.source_root(),
            &source_manifest,
            dependency_lock,
        )?;

        let bundle = FixedEsmBundler::new(
            staged.source_root(),
            staged.capture(),
            dependency_lock,
            self.npm_cache.as_ref(),
        )
        .bundle(&source_manifest, cancellation)?;
        let bundle_language = bundle.language;
        let bundle_path = staged.operation_root().join(match bundle_language {
            PluginLanguage::JavaScript => "bundle-input.js",
            PluginLanguage::TypeScript => "bundle-input.ts",
        });
        write_new(&bundle_path, bundle.source.as_bytes())?;
        let host_output =
            self.build_host
                .build_bundle(&staged, &bundle_path, bundle_language, cancellation)?;
        check_canceled(cancellation)?;
        copy_resources(&staged, cancellation)?;

        let main_bytes =
            fs::read(&host_output.main_path).map_err(|error| io_error(&host_output.main_path, error))?;
        let main_digest = digest_bytes(&main_bytes);
        let manifest = ArtifactEnvelope::new(PluginPackageV1Manifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            build_profile: JavaScriptBuildProfile::PluginPackageV1,
            build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            package: PackageManifest {
                schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
                host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                package_id: source_manifest.package_id().clone(),
                package_version: source_manifest.package_version().clone(),
                display: source_manifest.display().clone(),
                package_dependencies: Vec::new(),
                requires_runtime_features: source_manifest.requires_runtime_features().to_vec(),
                config_schema: source_manifest.config_schema().clone(),
                provides_services: Vec::new(),
                requires_services: Vec::new(),
                entrypoint: PackageEntrypointMetadata::javascript(
                    JavaScriptEntrypointMetadata {
                        normalized_relative_path: "main.mjs".into(),
                        module_digest: main_digest.clone(),
                        host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                        sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                    },
                ),
                contributions: source_manifest.contributions().clone(),
            },
            schemas: source_manifest.schemas().clone(),
            supported_targets: options.supported_targets.clone(),
            minimum_node_major: MINIMUM_NODE_MAJOR.max(24),
            dependency_lock_digest: dependency_lock.digest()?,
            credential_slots: source_manifest.credential_slots().to_vec(),
        })
        .map_err(|error| AuthoringError::CanonicalSerialization(error.to_string()))?;
        manifest
            .payload
            .validate()
            .map_err(|error| AuthoringError::PackRejected(error.to_string()))?;
        let manifest_path = staged.output_root().join("manifest.json");
        write_new(
            &manifest_path,
            &canonical_json_bytes(&manifest)
                .map_err(|error| AuthoringError::CanonicalSerialization(error.to_string()))?,
        )?;
        check_canceled(cancellation)?;

        Ok(PackedPluginPackage {
            staged,
            manifest,
            main_digest,
            source_map_written: host_output.source_map_path.is_some(),
        })
    }
}

#[derive(Debug)]
pub struct PackedPluginPackage {
    staged: StagedSource,
    manifest: ArtifactEnvelope<PluginPackageV1Manifest>,
    main_digest: DigestHex,
    source_map_written: bool,
}

impl PackedPluginPackage {
    pub fn package_root(&self) -> &Path {
        self.staged.output_root()
    }

    pub fn manifest(&self) -> &ArtifactEnvelope<PluginPackageV1Manifest> {
        &self.manifest
    }

    pub fn main_digest(&self) -> &DigestHex {
        &self.main_digest
    }

    pub fn source_map_written(&self) -> bool {
        self.source_map_written
    }
}

#[derive(Serialize)]
struct BuildHostRequest {
    format_version: &'static str,
    language: PluginLanguage,
    source_path: String,
    output_path: String,
    map_path: String,
    source_url: String,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum BuildHostResponse {
    Ok {
        format_version: String,
        source_map_written: bool,
    },
    UnsupportedModule {
        format_version: String,
        specifier: String,
    },
    Failed {
        format_version: String,
        message: String,
    },
}

impl BuildHostResponse {
    fn has_supported_version(&self) -> bool {
        match self {
            Self::Ok { format_version, .. }
            | Self::UnsupportedModule { format_version, .. }
            | Self::Failed { format_version, .. } => {
                format_version == BUILD_REQUEST_VERSION
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageJsonIdentity {
    name: String,
    version: String,
    description: String,
    private: bool,
    #[serde(rename = "type")]
    module_type: String,
    #[serde(default)]
    dependencies: std::collections::BTreeMap<String, String>,
}

struct BuildHostOutput {
    main_path: PathBuf,
    source_map_path: Option<PathBuf>,
}

struct BundleOutput {
    source: String,
    language: PluginLanguage,
}

struct FixedEsmBundler<'a> {
    source_root: &'a Path,
    lock: &'a ExactDependencyLock,
    cache: Option<&'a ContentAddressedNpmCache>,
    source_files: BTreeSet<String>,
    npm_cache: BTreeMap<String, CachedNpmPackage>,
    npm_entrypoints: BTreeMap<String, NormalizedSourcePath>,
    visiting: BTreeSet<String>,
    emitted: BTreeSet<String>,
    modules: Vec<(String, String)>,
    languages: BTreeSet<PluginLanguage>,
    module_exports: BTreeMap<String, Vec<String>>,
    external_imports: Vec<String>,
    external_counter: usize,
}

impl<'a> FixedEsmBundler<'a> {
    fn new(
        source_root: &'a Path,
        captured: &'a CapturedSource,
        lock: &'a ExactDependencyLock,
        cache: Option<&'a ContentAddressedNpmCache>,
    ) -> Self {
        let source_files = captured
            .snapshot()
            .files()
            .iter()
            .map(|file| file.normalized_relative_path().as_str().to_owned())
            .collect();
        Self {
            source_root,
            lock,
            cache,
            source_files,
            npm_cache: BTreeMap::new(),
            npm_entrypoints: BTreeMap::new(),
            visiting: BTreeSet::new(),
            emitted: BTreeSet::new(),
            modules: Vec::new(),
            languages: BTreeSet::new(),
            module_exports: BTreeMap::new(),
            external_imports: Vec::new(),
            external_counter: 0,
        }
    }

    fn bundle(
        mut self,
        manifest: &PluginSourceManifest,
        cancellation: &dyn OperationCancellation,
    ) -> Result<BundleOutput, AuthoringError> {
        let entry = manifest.entrypoint().as_str().to_owned();
        self.ensure_source_module(&entry)?;
        let entry_id = format!("source:{entry}");
        let entry_exports = self.visit(&ModuleRef::Source(entry.clone()), cancellation)?;
        if !entry_exports.iter().any(|value| value == "activate") {
            return Err(AuthoringError::PackRejected(
                "source entrypoint must export activate".into(),
            ));
        }
        let executable_sources = self
            .source_files
            .iter()
            .filter(|path| is_executable_source_path(path))
            .cloned()
            .collect::<BTreeSet<_>>();
        let visited_sources = self
            .emitted
            .iter()
            .filter_map(|id| id.strip_prefix("source:"))
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        if executable_sources != visited_sources {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: "source".into(),
                reason: "every JavaScript/TypeScript source file must be reachable from the entrypoint"
                    .into(),
            });
        }
        let mut output = String::new();
        for declaration in &self.external_imports {
            output.push_str(declaration);
            output.push('\n');
        }
        output.push_str(
            "const __modules = Object.create(null);\n\
             const __cache = Object.create(null);\n\
             const __require = (id) => {\n\
               if (__cache[id]) return __cache[id];\n\
               const __exports = {};\n\
               __cache[id] = __exports;\n\
               __modules[id](__exports, __require);\n\
               return __exports;\n\
             };\n",
        );
        for (id, body) in self.modules {
            output.push_str("__modules[");
            output.push_str(&serde_json::to_string(&id).unwrap());
            output.push_str("] = (__exports, __require) => {\n");
            output.push_str(&body);
            output.push_str("\n};\n");
        }
        output.push_str("const __entry = __require(");
        output.push_str(&serde_json::to_string(&entry_id).unwrap());
        output.push_str(");\nexport const activate = __entry.activate;\n");
        Ok(BundleOutput {
            source: output,
            language: if self.languages.contains(&PluginLanguage::TypeScript) {
                PluginLanguage::TypeScript
            } else {
                PluginLanguage::JavaScript
            },
        })
    }

    fn visit(
        &mut self,
        module: &ModuleRef,
        cancellation: &dyn OperationCancellation,
    ) -> Result<Vec<String>, AuthoringError> {
        check_canceled(cancellation)?;
        let id = module.id();
        if let Some(exports) = self.module_exports.get(&id) {
            return Ok(exports.clone());
        }
        if !self.visiting.insert(id.clone()) {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: id,
                reason: "cyclic ESM graphs are outside the fixed bundler subset".into(),
            });
        }
        let (source, language) = self.read_module(module)?;
        let parsed = parse_module_source(&source)?;
        self.languages.insert(language);
        let mut resolved_imports = Vec::new();
        let mut external_bindings = String::new();
        for import in &parsed.imports {
            let resolved = self.resolve_specifier(module, &import.specifier)?;
            if let ModuleRef::External(specifier) = &resolved {
                external_bindings.push_str(&self.bind_external(import, specifier)?);
                continue;
            }
            let exports = self.visit(&resolved, cancellation)?;
            validate_import_bindings(import, &exports)?;
            resolved_imports.push((import.clone(), resolved, exports));
        }
        let mut body = external_bindings;
        for (import, resolved, _) in resolved_imports {
            let dependency_id = resolved.id();
            match import.kind {
                ImportKind::SideEffect => {
                    body.push_str(&format!(
                        "__require({});\n",
                        serde_json::to_string(&dependency_id).unwrap()
                    ));
                }
                ImportKind::Named(bindings) => {
                    body.push_str("const { ");
                    for (index, (source_name, local_name)) in bindings.iter().enumerate() {
                        if index > 0 {
                            body.push_str(", ");
                        }
                        body.push_str(source_name);
                        if source_name != local_name {
                            body.push_str(": ");
                            body.push_str(local_name);
                        }
                    }
                    body.push_str(" } = __require(");
                    body.push_str(&serde_json::to_string(&dependency_id).unwrap());
                    body.push_str(");\n");
                }
                ImportKind::Namespace(local_name) => {
                    body.push_str("const ");
                    body.push_str(&local_name);
                    body.push_str(" = __require(");
                    body.push_str(&serde_json::to_string(&dependency_id).unwrap());
                    body.push_str(");\n");
                }
                ImportKind::Default(local_name) => {
                    body.push_str("const ");
                    body.push_str(&local_name);
                    body.push_str(" = __require(");
                    body.push_str(&serde_json::to_string(&dependency_id).unwrap());
                    body.push_str(").default;\n");
                }
                ImportKind::DefaultAndNamed {
                    default_name,
                    bindings,
                } => {
                    body.push_str("const ");
                    body.push_str(&default_name);
                    body.push_str(" = __require(");
                    body.push_str(&serde_json::to_string(&dependency_id).unwrap());
                    body.push_str(").default;\nconst { ");
                    for (index, (source_name, local_name)) in bindings.iter().enumerate() {
                        if index > 0 {
                            body.push_str(", ");
                        }
                        body.push_str(source_name);
                        if source_name != local_name {
                            body.push_str(": ");
                            body.push_str(local_name);
                        }
                    }
                    body.push_str(" } = __require(");
                    body.push_str(&serde_json::to_string(&dependency_id).unwrap());
                    body.push_str(");\n");
                }
            }
        }
        let transformed = transform_module_body(&parsed.body, &parsed.exports)?;
        body.push_str(&transformed);
        for export in &parsed.exports {
            body.push_str("\n__exports.");
            body.push_str(&export.export_name);
            body.push_str(" = ");
            body.push_str(&export.local_name);
            body.push(';');
        }
        self.modules.push((id.clone(), body));
        self.visiting.remove(&id);
        self.emitted.insert(id);
        let exports = parsed
            .exports
            .iter()
            .map(|export| export.export_name.clone())
            .collect::<Vec<_>>();
        self.module_exports.insert(module.id(), exports.clone());
        Ok(exports)
    }

    fn read_module(&mut self, module: &ModuleRef) -> Result<(String, PluginLanguage), AuthoringError> {
        match module {
            ModuleRef::Source(path) => {
                let normalized = NormalizedSourcePath::parse(path.clone())?;
                let file = normalized.join(self.source_root);
                require_regular_contained(self.source_root, &file)?;
                let source = fs::read_to_string(&file).map_err(|error| io_error(&file, error))?;
                Ok((source, language_for_path(path)?))
            }
            ModuleRef::Npm { package_key, path } => {
                let package = self.package_cache(package_key)?;
                let source = String::from_utf8(package.file_bytes(path)?).map_err(|_| {
                    AuthoringError::PackRejected(format!(
                        "npm module {} is not valid UTF-8",
                        module.id()
                    ))
                })?;
                Ok((source, language_for_path(path.as_str())?))
            }
            ModuleRef::External(specifier) => Err(AuthoringError::PackRejected(format!(
                "external module {specifier} cannot be read as source"
            ))),
        }
    }

    fn package_cache(&mut self, key: &str) -> Result<&CachedNpmPackage, AuthoringError> {
        if !self.npm_cache.contains_key(key) {
            let cache = self.cache.ok_or_else(|| {
                AuthoringError::PackRejected(
                    "a ContentAddressedNpmCache is required for npm bundling".into(),
                )
            })?;
            let locked = self.lock.packages().get(key).ok_or_else(|| {
                AuthoringError::PackRejected(format!("lock graph is missing {key}"))
            })?;
            let package = cache.load(locked.archive_sha256())?;
            if package.name() != locked.name() || package.version() != locked.version() {
                return Err(AuthoringError::Cache(format!(
                    "cache object does not match lock package {key}"
                )));
            }
            let package_json = package.file_bytes(&NormalizedSourcePath::parse("package.json")?)?;
            let metadata = validate_registry_package_json(
                &package_json,
                locked.name(),
                locked.version(),
            )
            .map_err(|error| AuthoringError::Cache(error.to_string()))?;
            let locked_dependencies = self
                .lock
                .packages()
                .get(key)
                .expect("lock package checked before cache load")
                .dependencies();
            if metadata.dependencies().dependencies().keys().collect::<BTreeSet<_>>()
                != locked_dependencies.keys().collect::<BTreeSet<_>>()
            {
                return Err(AuthoringError::Cache(format!(
                    "npm package {key} dependencies drift from exact lock"
                )));
            }
            for (dependency, requirement) in metadata.dependencies().dependencies() {
                let target = locked_dependencies.get(dependency).ok_or_else(|| {
                    AuthoringError::Cache(format!(
                        "npm package {key} has an undeclared lock edge {dependency}"
                    ))
                })?;
                let target_package = self.lock.packages().get(target).ok_or_else(|| {
                    AuthoringError::PackRejected(format!(
                        "lock dependency target {target} is missing"
                    ))
                })?;
                let version = semver::Version::parse(target_package.version()).map_err(|error| {
                    AuthoringError::PackRejected(format!(
                        "lock target {target} has invalid version: {error}"
                    ))
                })?;
                let matches = if let Ok(exact) = semver::Version::parse(requirement) {
                    exact == version
                } else {
                    semver::VersionReq::parse(requirement)
                        .map_err(|error| AuthoringError::Cache(error.to_string()))?
                        .matches(&version)
                };
                if !matches {
                    return Err(AuthoringError::Cache(format!(
                        "npm package {key} dependency {dependency} does not match lock target {target}"
                    )));
                }
            }
            let entry = metadata.entrypoint().clone();
            if !package.contains_file(&entry) {
                return Err(AuthoringError::Cache(format!(
                    "npm package {key} entrypoint {entry} is missing"
                )));
            }
            self.npm_entrypoints.insert(key.to_owned(), entry);
            self.npm_cache.insert(key.to_owned(), package);
        }
        Ok(self.npm_cache.get(key).expect("cache inserted"))
    }

    fn ensure_source_module(&self, path: &str) -> Result<(), AuthoringError> {
        if !self.source_files.contains(path) {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: path.into(),
                reason: "declared source entrypoint is missing".into(),
            });
        }
        Ok(())
    }

    fn resolve_specifier(
        &mut self,
        importer: &ModuleRef,
        specifier: &str,
    ) -> Result<ModuleRef, AuthoringError> {
        if specifier == "node:module" {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: importer.id(),
                reason: "node:module can create runtime dependency loaders and is forbidden".into(),
            });
        }
        if specifier.starts_with("node:") {
            if !specifier
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b':' | b'/' | b'_' | b'-'))
            {
                return Err(AuthoringError::LocalModuleUnsupported {
                    path: importer.id(),
                    reason: format!("invalid Node public API specifier {specifier:?}"),
                });
            }
            return Ok(ModuleRef::External(specifier.into()));
        }
        if specifier.starts_with('/') {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: importer.id(),
                reason: "absolute module specifiers are forbidden".into(),
            });
        }
        if specifier.starts_with('.') {
            return self.resolve_local(importer, specifier);
        }
        if specifier.contains('\\') || specifier.contains(':') {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: importer.id(),
                reason: format!("unsupported module specifier {specifier:?}"),
            });
        }
        let (package_name, subpath) = split_package_specifier(specifier)?;
        let dependency_target = match importer {
            ModuleRef::Source(_) => self.lock.roots().get(package_name),
            ModuleRef::Npm { package_key, .. } => self
                .lock
                .packages()
                .get(package_key)
                .and_then(|package| package.dependencies().get(package_name)),
            ModuleRef::External(_) => None,
        }
        .ok_or_else(|| AuthoringError::LocalModuleUnsupported {
            path: importer.id(),
            reason: format!("undeclared npm import {specifier:?}"),
        })?;
        self.package_cache(dependency_target)?;
        let path = if subpath.is_empty() {
            self.npm_entrypoints
                .get(dependency_target)
                .cloned()
                .ok_or_else(|| AuthoringError::PackRejected("missing package entry".into()))?
        } else {
            NormalizedSourcePath::parse(subpath.to_owned())?
        };
        if !self
            .npm_cache
            .get(dependency_target)
            .expect("package cache inserted")
            .contains_file(&path)
        {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: specifier.into(),
                reason: "npm subpath is not present in the exact cache object".into(),
            });
        }
        Ok(ModuleRef::Npm {
            package_key: dependency_target.clone(),
            path,
        })
    }

    fn bind_external(
        &mut self,
        import: &ParsedImport,
        specifier: &str,
    ) -> Result<String, AuthoringError> {
        let index = self.external_counter;
        self.external_counter += 1;
        let quoted = serde_json::to_string(specifier).unwrap();
        match &import.kind {
            ImportKind::SideEffect => {
                self.external_imports.push(format!("import {quoted};"));
                Ok(String::new())
            }
            ImportKind::Namespace(local_name) => {
                let alias = format!("__external_{index}");
                self.external_imports
                    .push(format!("import * as {alias} from {quoted};"));
                Ok(format!("const {local_name} = {alias};\n"))
            }
            ImportKind::Default(local_name) => {
                let alias = format!("__external_{index}");
                self.external_imports
                    .push(format!("import {alias} from {quoted};"));
                Ok(format!("const {local_name} = {alias};\n"))
            }
            ImportKind::Named(bindings) => {
                let mut declarations = Vec::new();
                let mut assignments = String::new();
                for (binding_index, (source_name, local_name)) in bindings.iter().enumerate() {
                    let alias = format!("__external_{index}_{binding_index}");
                    declarations.push(format!("{source_name} as {alias}"));
                    assignments.push_str(&format!("const {local_name} = {alias};\n"));
                }
                self.external_imports.push(format!(
                    "import {{ {} }} from {quoted};",
                    declarations.join(", ")
                ));
                Ok(assignments)
            }
            ImportKind::DefaultAndNamed {
                default_name,
                bindings,
            } => {
                let default_alias = format!("__external_{index}_default");
                let mut declarations = Vec::new();
                let mut assignments = format!("const {default_name} = {default_alias};\n");
                for (binding_index, (source_name, local_name)) in bindings.iter().enumerate() {
                    let alias = format!("__external_{index}_{binding_index}");
                    declarations.push(format!("{source_name} as {alias}"));
                    assignments.push_str(&format!("const {local_name} = {alias};\n"));
                }
                self.external_imports.push(format!(
                    "import {default_alias}, {{ {} }} from {quoted};",
                    declarations.join(", ")
                ));
                Ok(assignments)
            }
        }
    }

    fn resolve_local(
        &mut self,
        importer: &ModuleRef,
        specifier: &str,
    ) -> Result<ModuleRef, AuthoringError> {
        let base = match importer {
            ModuleRef::Source(path) => path.rsplit_once('/').map_or("", |(parent, _)| parent),
            ModuleRef::Npm { path, .. } => {
                path.as_str().rsplit_once('/').map_or("", |(parent, _)| parent)
            }
            ModuleRef::External(specifier) => {
                return Err(AuthoringError::LocalModuleUnsupported {
                    path: specifier.clone(),
                    reason: "Node external modules cannot resolve relative imports".into(),
                });
            }
        };
        let joined = normalize_relative(base, specifier)?;
        let candidates = extension_candidates(&joined);
        match importer {
            ModuleRef::Source(_) => candidates
                .into_iter()
                .find(|path| self.source_files.contains(path))
                .map(|path| ModuleRef::Source(path.to_owned())),
            ModuleRef::Npm { package_key, .. } => {
                let package = self.npm_cache.get(package_key).ok_or_else(|| {
                    AuthoringError::PackRejected(format!("package cache missing {package_key}"))
                })?;
                candidates.into_iter().find_map(|path| {
                    let normalized = NormalizedSourcePath::parse(path).ok()?;
                    package.contains_file(&normalized).then_some(ModuleRef::Npm {
                            package_key: package_key.clone(),
                            path: normalized,
                        })
                })
            }
            ModuleRef::External(_) => None,
        }
        .ok_or_else(|| AuthoringError::LocalModuleUnsupported {
            path: specifier.into(),
            reason: "local module target is missing or outside the allowed graph".into(),
        })
    }
}

fn is_executable_source_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".d.ts") || lower.ends_with(".d.mts") {
        return false;
    }
    lower.ends_with(".js")
        || lower.ends_with(".mjs")
        || lower.ends_with(".ts")
        || lower.ends_with(".mts")
}

fn language_for_path(path: &str) -> Result<PluginLanguage, AuthoringError> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".js") || lower.ends_with(".mjs") {
        Ok(PluginLanguage::JavaScript)
    } else if lower.ends_with(".ts") || lower.ends_with(".mts") {
        Ok(PluginLanguage::TypeScript)
    } else {
        Err(AuthoringError::LocalModuleUnsupported {
            path: path.into(),
            reason: "only .js/.mjs/.ts/.mts modules are supported".into(),
        })
    }
}

fn extension_candidates(path: &str) -> Vec<String> {
    let mut candidates = vec![path.to_owned()];
    if Path::new(path).extension().is_none() {
        for extension in [".js", ".mjs", ".ts", ".mts"] {
            candidates.push(format!("{path}{extension}"));
        }
        for extension in [".js", ".mjs", ".ts", ".mts"] {
            candidates.push(format!("{path}/index{extension}"));
        }
    }
    candidates
}

fn split_package_specifier(specifier: &str) -> Result<(&str, &str), AuthoringError> {
    if specifier.starts_with('@') {
        let mut parts = specifier.splitn(3, '/');
        let scope = parts.next().unwrap_or_default();
        let package = parts.next().unwrap_or_default();
        if scope.is_empty() || package.is_empty() {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: specifier.into(),
                reason: "scoped npm specifier must contain @scope/name".into(),
            });
        }
        let subpath = parts.next().unwrap_or_default();
        // The returned package name must live for the call. Avoid allocating
        // by accepting scoped names through the caller's original slice.
        let package_end = scope.len() + 1 + package.len();
        let package_name = &specifier[..package_end];
        Ok((package_name, subpath))
    } else {
        let (package, subpath) = specifier
            .split_once('/')
            .map_or((specifier, ""), |(package, rest)| (package, rest));
        if package.is_empty() {
            return Err(AuthoringError::LocalModuleUnsupported {
                path: specifier.into(),
                reason: "npm package name is empty".into(),
            });
        }
        Ok((package, subpath))
    }
}

fn normalize_relative(base: &str, specifier: &str) -> Result<String, AuthoringError> {
    let mut parts = base
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for part in specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(AuthoringError::LocalModuleUnsupported {
                        path: specifier.into(),
                        reason: "relative import escapes its package root".into(),
                    });
                }
            }
            value if value.contains('\\') || value.contains(':') => {
                return Err(AuthoringError::LocalModuleUnsupported {
                    path: specifier.into(),
                    reason: "relative import contains an unsafe path component".into(),
                });
            }
            value => parts.push(value.to_owned()),
        }
    }
    if parts.is_empty() {
        return Err(AuthoringError::LocalModuleUnsupported {
            path: specifier.into(),
            reason: "relative import resolves to an empty path".into(),
        });
    }
    Ok(parts.join("/"))
}

fn parse_module_source(source: &str) -> Result<ParsedModule, AuthoringError> {
    if source.contains("import(")
        || source.contains("import (")
        || source.contains("require(")
    {
        return Err(AuthoringError::LocalModuleUnsupported {
            path: "module".into(),
            reason: "dynamic import and require are forbidden".into(),
        });
    }
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut body = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") {
            imports.push(parse_import(trimmed)?);
        } else if trimmed.starts_with("export ") {
            let (rewritten, declarations) = parse_export(trimmed)?;
            if !rewritten.is_empty() {
                body.push_str(&rewritten);
                body.push('\n');
            }
            exports.extend(declarations);
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    Ok(ParsedModule {
        imports,
        exports,
        body,
    })
}

fn parse_import(line: &str) -> Result<ParsedImport, AuthoringError> {
    let line = line
        .strip_prefix("import ")
        .and_then(|value| value.strip_suffix(';').or(Some(value)))
        .map(str::trim)
        .ok_or_else(|| unsupported_syntax("import statement"))?;
    if let Some(specifier) = quoted_specifier(line) {
        return Ok(ParsedImport {
            specifier,
            kind: ImportKind::SideEffect,
        });
    }
    let (clause, specifier) = line
        .split_once(" from ")
        .and_then(|(clause, specifier)| quoted_specifier(specifier).map(|value| (clause, value)))
        .ok_or_else(|| unsupported_syntax("static import declaration"))?;
    let clause = clause.trim();
    if let Some(value) = clause.strip_prefix("* as ") {
        return valid_binding(value).map(|local_name| ParsedImport {
            specifier,
            kind: ImportKind::Namespace(local_name),
        });
    }
    if let Some(value) = clause.strip_prefix('{') {
        return Ok(ParsedImport {
            specifier,
            kind: ImportKind::Named(parse_bindings(value.strip_suffix('}').unwrap_or_default())?),
        });
    }
    let Some((default_name, rest)) = clause.split_once(',') else {
        return valid_binding(clause).map(|local_name| ParsedImport {
            specifier,
            kind: ImportKind::Default(local_name),
        });
    };
    let default_name = valid_binding(default_name.trim())?;
    let rest = rest.trim();
    if let Some(value) = rest.strip_prefix('{') {
        return Ok(ParsedImport {
            specifier,
            kind: ImportKind::DefaultAndNamed {
                default_name,
                bindings: parse_bindings(value.strip_suffix('}').unwrap_or_default())?,
            },
        });
    }
    Err(unsupported_syntax("import clause"))
}

fn parse_export(
    line: &str,
) -> Result<(String, Vec<ParsedExport>), AuthoringError> {
    let line = line
        .strip_prefix("export ")
        .ok_or_else(|| unsupported_syntax("export statement"))?;
    if let Some(value) = line.strip_prefix('{') {
        let value = value
            .strip_suffix("};")
            .or_else(|| value.strip_suffix('}'))
            .unwrap_or_default();
        let bindings = parse_bindings(value)?;
        return Ok((String::new(), bindings
            .into_iter()
            .map(|(local_name, export_name)| ParsedExport {
                local_name,
                export_name,
            })
            .collect()));
    }
    for keyword in ["const ", "let ", "var "] {
        if let Some(value) = line.strip_prefix(keyword) {
            if value.contains(',') || value.contains(" export ") {
                return Err(unsupported_syntax(
                    "multi-binding or multi-statement variable export",
                ));
            }
            let name = value
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            valid_binding(name)?;
            return Ok((format!("{keyword}{value}"), vec![ParsedExport {
                local_name: name.into(),
                export_name: name.into(),
            }]));
        }
    }
    if let Some(value) = line.strip_prefix("async function ") {
        let name = value.split('(').next().unwrap_or_default().trim();
        valid_binding(name)?;
        return Ok((format!("async function {value}"), vec![ParsedExport {
            local_name: name.into(),
            export_name: name.into(),
        }]));
    }
    if let Some(value) = line.strip_prefix("function ") {
        let name = value.split('(').next().unwrap_or_default().trim();
        valid_binding(name)?;
        return Ok((format!("function {value}"), vec![ParsedExport {
            local_name: name.into(),
            export_name: name.into(),
        }]));
    }
    Err(unsupported_syntax("export declaration"))
}

fn parse_bindings(value: &str) -> Result<Vec<(String, String)>, AuthoringError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let (source_name, local_name) = value
                .split_once(" as ")
                .map_or((value, value), |(source, local)| (source.trim(), local.trim()));
            valid_binding(source_name)?;
            valid_binding(local_name)?;
            Ok((source_name.into(), local_name.into()))
        })
        .collect()
}

fn quoted_specifier(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches(';').trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        Some(value[1..value.len() - 1].to_owned())
    } else {
        None
    }
}

fn valid_binding(value: &str) -> Result<String, AuthoringError> {
    if value.is_empty()
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| {
                byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
            })
    {
        Err(unsupported_syntax("JavaScript binding"))
    } else {
        Ok(value.into())
    }
}

fn transform_module_body(
    body: &str,
    _exports: &[ParsedExport],
) -> Result<String, AuthoringError> {
    if body.contains("export ")
        || body.contains("import ")
        || body.contains("require(")
        || body.contains("import(")
    {
        return Err(unsupported_syntax(
            "module must use one-line static declarations",
        ));
    }
    Ok(body.to_owned())
}

fn validate_import_bindings(
    import: &ParsedImport,
    exports: &[String],
) -> Result<(), AuthoringError> {
    let has_export = |name: &str| exports.iter().any(|export| export == name);
    let missing = match &import.kind {
        ImportKind::SideEffect | ImportKind::Namespace(_) => None,
        ImportKind::Default(_) => (!has_export("default")).then_some("default"),
        ImportKind::Named(bindings) => bindings
            .iter()
            .find_map(|(source_name, _)| (!has_export(source_name)).then_some(source_name.as_str())),
        ImportKind::DefaultAndNamed {
            default_name: _,
            bindings,
        } => if !has_export("default") {
            Some("default")
        } else {
            bindings
                .iter()
                .find_map(|(source_name, _)| (!has_export(source_name)).then_some(source_name.as_str()))
        },
    };
    if let Some(name) = missing {
        return Err(AuthoringError::PackRejected(format!(
            "import requests missing export {name}"
        )));
    }
    Ok(())
}

fn unsupported_syntax(what: &str) -> AuthoringError {
    AuthoringError::PackRejected(format!(
        "fixed ESM bundler does not support {what}"
    ))
}

#[derive(Clone, Debug)]
enum ModuleRef {
    Source(String),
    Npm {
        package_key: String,
        path: NormalizedSourcePath,
    },
    External(String),
}

impl ModuleRef {
    fn id(&self) -> String {
        match self {
            Self::Source(path) => format!("source:{path}"),
            Self::Npm { package_key, path } => format!("npm:{package_key}:{path}"),
            Self::External(specifier) => format!("external:{specifier}"),
        }
    }
}

#[derive(Clone, Debug)]
struct ParsedModule {
    imports: Vec<ParsedImport>,
    exports: Vec<ParsedExport>,
    body: String,
}

#[derive(Clone, Debug)]
struct ParsedImport {
    specifier: String,
    kind: ImportKind,
}

#[derive(Clone, Debug)]
enum ImportKind {
    SideEffect,
    Named(Vec<(String, String)>),
    Namespace(String),
    Default(String),
    DefaultAndNamed {
        default_name: String,
        bindings: Vec<(String, String)>,
    },
}

#[derive(Clone, Debug)]
struct ParsedExport {
    export_name: String,
    local_name: String,
}

fn validate_package_json_identity(
    source_root: &Path,
    manifest: &PluginSourceManifest,
    dependency_lock: &ExactDependencyLock,
) -> Result<(), AuthoringError> {
    let path = source_root.join("package.json");
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    let package: PackageJsonIdentity = strict_json_from_slice(&bytes)?;
    if !package.private
        || package.module_type != "module"
        || package.name != manifest.package_id().as_ref()
        || package.version != manifest.package_version().as_ref()
        || package.description != manifest.display().description
    {
        return Err(AuthoringError::PackRejected(
            "package.json identity must exactly match the source manifest".into(),
        ));
    }
    if package.dependencies.keys().collect::<BTreeSet<_>>()
        != dependency_lock.roots().keys().collect::<BTreeSet<_>>()
    {
        return Err(AuthoringError::PackRejected(
            "package.json dependencies must exactly match dependency lock roots".into(),
        ));
    }
    Ok(())
}

fn validate_source_inventory(
    staged: &StagedSource,
    manifest: &PluginSourceManifest,
) -> Result<(), AuthoringError> {
    for file in staged.capture().snapshot().files() {
        let path = file.normalized_relative_path().as_str();
        if path == PLUGIN_SOURCE_MANIFEST_FILE
            || path == "package.json"
            || path == "nomifun-plugin-sdk.d.ts"
            || path == manifest.entrypoint().as_str()
        {
            continue;
        }
        if let Some(relative) = path.strip_prefix("resources/") {
            let lower = relative.to_ascii_lowercase();
            if lower.ends_with(".js")
                || lower.ends_with(".mjs")
                || lower.ends_with(".cjs")
                || lower.ends_with(".ts")
                || lower.ends_with(".mts")
                || lower.ends_with(".cts")
                || lower.ends_with(".wasm")
                || lower.ends_with(".node")
            {
                return Err(AuthoringError::LocalModuleUnsupported {
                    path: path.into(),
                    reason: "executable resources are forbidden until the fixed bundler can include their module graph"
                        .into(),
                });
            }
            continue;
        }
        if is_executable_source_path(path) {
            continue;
        }
        return Err(AuthoringError::LocalModuleUnsupported {
            path: path.into(),
            reason: "only the declared entrypoint is executable in the single-file build profile"
                .into(),
        });
    }
    Ok(())
}

fn copy_resources(
    staged: &StagedSource,
    cancellation: &dyn OperationCancellation,
) -> Result<(), AuthoringError> {
    for file in staged.capture().snapshot().files() {
        let path = file.normalized_relative_path().as_str();
        let Some(relative) = path.strip_prefix("resources/") else {
            continue;
        };
        check_canceled(cancellation)?;
        let relative = NormalizedSourcePath::parse(relative)?;
        let source = file.normalized_relative_path().join(staged.source_root());
        let resources_root = staged.output_root().join("resources");
        let target = relative.join(&resources_root);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        }
        fs::copy(&source, &target).map_err(|error| io_error(&target, error))?;
        let copied = fs::read(&target).map_err(|error| io_error(&target, error))?;
        if digest_bytes(&copied) != *file.digest() {
            return Err(AuthoringError::SourceChanged {
                expected: file.digest().as_ref().to_owned(),
                observed: digest_bytes(&copied).as_ref().to_owned(),
            });
        }
    }
    Ok(())
}

fn require_absolute_regular_file(path: &Path) -> Result<PathBuf, AuthoringError> {
    if !path.is_absolute() {
        return Err(AuthoringError::BuildHostUnavailable(
            "Node executable path must be absolute".into(),
        ));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| AuthoringError::BuildHostUnavailable(error.to_string()))?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| AuthoringError::BuildHostUnavailable(error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AuthoringError::BuildHostUnavailable(
            "Node executable must be a regular file".into(),
        ));
    }
    Ok(canonical)
}

fn require_regular_contained(root: &Path, path: &Path) -> Result<(), AuthoringError> {
    let canonical_root = fs::canonicalize(root).map_err(|error| io_error(root, error))?;
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || !canonical.starts_with(&canonical_root)
    {
        return Err(AuthoringError::UnsafeManagedPath {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), AuthoringError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn read_bounded_diagnostic(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() <= MAX_BUILD_DIAGNOSTIC_BYTES => {
            fs::read_to_string(path).unwrap_or_else(|_| "Build Host emitted non-UTF-8 stderr".into())
        }
        Ok(_) => "Build Host stderr exceeded the diagnostic limit".into(),
        Err(_) => String::new(),
    }
}

fn bounded_message(value: &str) -> String {
    const MAX_CHARS: usize = 16 * 1024;
    value.chars().take(MAX_CHARS).collect()
}

fn node_visible_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(drive_path) = value.strip_prefix(r"\\?\") {
            return drive_path.to_owned();
        }
    }
    value.into_owned()
}

#[allow(clippy::too_many_arguments)]
fn run_managed_build_process(
    node_executable: &Path,
    operation_root: &Path,
    host_path: &Path,
    request_path: &Path,
    response_path: &Path,
    stderr: fs::File,
    timeout: Duration,
    poll_interval: Duration,
    cancellation: &dyn OperationCancellation,
) -> Result<ExitStatus, AuthoringError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            AuthoringError::BuildHostUnavailable(format!(
                "cannot create Plugin Build Host process runtime: {error}"
            ))
        })?;
    let mut builder = ChildProcessBuilder::new(node_executable);
    builder
        .arg("--experimental-vm-modules")
        .arg("--disable-warning=ExperimentalWarning")
        .arg(node_visible_path(host_path))
        .arg(node_visible_path(request_path))
        .arg(node_visible_path(response_path))
        .current_dir(operation_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH");
    let mut process = {
        let _runtime = runtime.enter();
        builder.spawn_managed().map_err(|error| {
            AuthoringError::BuildHostUnavailable(format!(
                "{}: {error}",
                node_executable.display()
            ))
        })?
    };

    let started = Instant::now();
    let status = loop {
        if cancellation.is_cancelled() {
            shutdown_build_process(&runtime, &mut process, node_executable)?;
            return Err(AuthoringError::Canceled);
        }
        if started.elapsed() >= timeout {
            shutdown_build_process(&runtime, &mut process, node_executable)?;
            return Err(AuthoringError::BuildHostTimeout(timeout));
        }
        match process
            .child_mut()
            .try_wait()
            .map_err(|error| io_error(node_executable, error))?
        {
            Some(status) => break status,
            None => thread::sleep(poll_interval),
        }
    };
    shutdown_build_process(&runtime, &mut process, node_executable)?;
    Ok(status)
}

fn shutdown_build_process(
    runtime: &tokio::runtime::Runtime,
    process: &mut ManagedChildProcess,
    node_executable: &Path,
) -> Result<(), AuthoringError> {
    runtime.block_on(process.shutdown()).map_err(|error| {
        AuthoringError::BuildHostUnavailable(format!(
            "Plugin Build Host process tree cleanup failed for {}: {error}",
            node_executable.display()
        ))
    })
}
