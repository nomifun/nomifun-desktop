use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use nomifun_agent_contracts::{DigestHex, digest_bytes};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::canonical::{canonical_json_bytes, strict_json_from_slice};
use crate::dependency::{
    DependencyRequestSet, ExactDependencyLock, LockedNpmPackage,
    NpmResolverIdentity,
};
use crate::error::{AuthoringError, io_error};
use crate::model::{OperationCancellation, check_canceled};
use crate::path::NormalizedSourcePath;

const CACHE_RECORD_VERSION: &str = "1.0.0";
const CACHE_OBJECTS_DIRECTORY: &str = "objects";
const CACHE_STAGING_DIRECTORY: &str = ".staging";
const CACHE_RECORD_FILE: &str = "object.json";
const CACHE_FILES_DIRECTORY: &str = "files";
const MAX_REGISTRY_FILE_COUNT: usize = 16_384;
const MAX_REGISTRY_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REGISTRY_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

pub trait NpmRegistryPort: Send + Sync {
    fn resolve(
        &self,
        package_name: &str,
        requirement: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<RegistryPackageRelease, AuthoringError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryPackageRelease {
    name: String,
    version: String,
    registry_integrity: String,
    files: BTreeMap<NormalizedSourcePath, Vec<u8>>,
}

impl RegistryPackageRelease {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        registry_integrity: impl Into<String>,
        files: impl IntoIterator<Item = (NormalizedSourcePath, Vec<u8>)>,
    ) -> Result<Self, AuthoringError> {
        let mut file_map = BTreeMap::new();
        for (path, bytes) in files {
            if file_map.insert(path.clone(), bytes).is_some() {
                return Err(AuthoringError::Registry(format!(
                    "package contains duplicate path {path}"
                )));
            }
        }
        let release = Self {
            name: name.into(),
            version: version.into(),
            registry_integrity: registry_integrity.into(),
            files: file_map,
        };
        release.validate()?;
        Ok(release)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn registry_integrity(&self) -> &str {
        &self.registry_integrity
    }

    pub fn files(&self) -> &BTreeMap<NormalizedSourcePath, Vec<u8>> {
        &self.files
    }

    fn validate(&self) -> Result<ValidatedRegistryPackage, AuthoringError> {
        let version = Version::parse(&self.version)
            .map_err(|error| AuthoringError::Registry(format!("invalid exact version: {error}")))?;
        if version.to_string() != self.version {
            return Err(AuthoringError::Registry(
                "registry package version must be canonical exact SemVer".into(),
            ));
        }
        if self.registry_integrity.is_empty()
            || self.registry_integrity.len() > 512
            || !self.registry_integrity.is_ascii()
        {
            return Err(AuthoringError::Registry(
                "registry integrity must be one bounded ASCII SRI value".into(),
            ));
        }
        if self.files.is_empty() || self.files.len() > MAX_REGISTRY_FILE_COUNT {
            return Err(AuthoringError::Registry(
                "registry package file count is outside fixed limits".into(),
            ));
        }
        let mut total = 0u64;
        let mut package_json = None;
        for (path, bytes) in &self.files {
            reject_registry_path(path)?;
            let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if size > MAX_REGISTRY_FILE_BYTES {
                return Err(AuthoringError::Registry(format!(
                    "registry package file {path} exceeds fixed limits"
                )));
            }
            total = total.saturating_add(size);
            if total > MAX_REGISTRY_TOTAL_BYTES {
                return Err(AuthoringError::Registry(
                    "registry package exceeds fixed total byte limit".into(),
                ));
            }
            if path.as_str() == "package.json" {
                package_json = Some(bytes.as_slice());
            }
        }
        let package_json = package_json.ok_or_else(|| {
            AuthoringError::Registry("registry package is missing package.json".into())
        })?;
        let package: RegistryPackageJson = strict_json_from_slice(package_json)
            .map_err(|error| AuthoringError::Registry(error.to_string()))?;
        if package.name != self.name || package.version != self.version {
            return Err(AuthoringError::Registry(
                "package.json identity does not match registry release".into(),
            ));
        }
        if package
            .scripts
            .as_ref()
            .is_some_and(|scripts| !scripts.is_empty())
        {
            return Err(AuthoringError::Registry(
                "npm lifecycle and package scripts are forbidden".into(),
            ));
        }
        if package.gypfile == Some(true)
            || package
                .optional_dependencies
                .as_ref()
                .is_some_and(|dependencies| !dependencies.is_empty())
        {
            return Err(AuthoringError::Registry(
                "native/optional install surfaces are forbidden".into(),
            ));
        }
        let dependencies = DependencyRequestSet::new(package.dependencies)?;
        Ok(ValidatedRegistryPackage {
            dependencies,
            package_json_digest: digest_bytes(package_json),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ContentAddressedNpmCache {
    root: PathBuf,
    objects_root: PathBuf,
    staging_root: PathBuf,
}

impl ContentAddressedNpmCache {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, AuthoringError> {
        fs::create_dir_all(root.as_ref()).map_err(|error| io_error(root.as_ref(), error))?;
        let root =
            fs::canonicalize(root.as_ref()).map_err(|error| io_error(root.as_ref(), error))?;
        let objects_root = root.join(CACHE_OBJECTS_DIRECTORY);
        let staging_root = root.join(CACHE_STAGING_DIRECTORY);
        ensure_direct_directory(&root, &objects_root)?;
        ensure_direct_directory(&root, &staging_root)?;
        Ok(Self {
            root,
            objects_root,
            staging_root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn store(
        &self,
        release: &RegistryPackageRelease,
        cancellation: &dyn OperationCancellation,
    ) -> Result<CachedNpmPackage, AuthoringError> {
        check_canceled(cancellation)?;
        let validated = release.validate()?;
        let record = cache_record(release, &validated)?;
        let archive_digest = record.archive_digest.clone();
        let final_root = self.objects_root.join(archive_digest.as_ref());
        if final_root.exists() {
            return self.load(&archive_digest);
        }

        let staging = self
            .staging_root
            .join(format!("cache-{}", Uuid::now_v7()));
        fs::create_dir(&staging).map_err(|error| io_error(&staging, error))?;
        let guard = CacheStagingGuard {
            staging_root: self.staging_root.clone(),
            path: staging.clone(),
        };
        let files_root = staging.join(CACHE_FILES_DIRECTORY);
        fs::create_dir(&files_root).map_err(|error| io_error(&files_root, error))?;
        for (path, bytes) in &release.files {
            check_canceled(cancellation)?;
            let target = path.join(&files_root);
            ensure_contained_parent(&files_root, &target)?;
            write_new(&target, bytes)?;
        }
        write_new(
            &staging.join(CACHE_RECORD_FILE),
            &canonical_json_bytes(&record)?,
        )?;
        check_canceled(cancellation)?;
        match fs::rename(&staging, &final_root) {
            Ok(()) => {
                guard.disarm();
                self.load(&archive_digest)
            }
            Err(_error) if final_root.exists() => self.load(&archive_digest),
            Err(error) => Err(io_error(&final_root, error)),
        }
    }

    pub fn load(&self, digest: &DigestHex) -> Result<CachedNpmPackage, AuthoringError> {
        validate_digest(digest)?;
        let object_root = self.objects_root.join(digest.as_ref());
        let canonical =
            fs::canonicalize(&object_root).map_err(|error| io_error(&object_root, error))?;
        if canonical.parent() != Some(self.objects_root.as_path()) {
            return Err(AuthoringError::Cache(
                "cache object escaped the objects root".into(),
            ));
        }
        let record_path = canonical.join(CACHE_RECORD_FILE);
        let bytes = fs::read(&record_path).map_err(|error| io_error(&record_path, error))?;
        let record: NpmCacheRecord = strict_json_from_slice(&bytes)
            .map_err(|error| AuthoringError::Cache(error.to_string()))?;
        if canonical_json_bytes(&record)? != bytes
            || record.format_version != CACHE_RECORD_VERSION
            || record.archive_digest != *digest
        {
            return Err(AuthoringError::Cache(
                "cache object record is non-canonical or mismatched".into(),
            ));
        }
        let files_root = canonical.join(CACHE_FILES_DIRECTORY);
        for file in &record.files {
            let path = file.path.join(&files_root);
            let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(AuthoringError::Cache(format!(
                    "cache file {} is not regular",
                    file.path
                )));
            }
            let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
            if digest_bytes(&bytes) != file.digest
                || u64::try_from(bytes.len()).unwrap_or(u64::MAX) != file.size_bytes
            {
                return Err(AuthoringError::Cache(format!(
                    "cache file {} failed digest verification",
                    file.path
                )));
            }
        }
        Ok(CachedNpmPackage {
            object_root: canonical,
            record,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CachedNpmPackage {
    object_root: PathBuf,
    record: NpmCacheRecord,
}

impl CachedNpmPackage {
    pub fn object_root(&self) -> &Path {
        &self.object_root
    }

    pub fn archive_digest(&self) -> &DigestHex {
        &self.record.archive_digest
    }

    pub fn package_json_digest(&self) -> &DigestHex {
        &self.record.package_json_digest
    }

    pub fn name(&self) -> &str {
        &self.record.name
    }

    pub fn version(&self) -> &str {
        &self.record.version
    }

    pub fn file_bytes(&self, path: &NormalizedSourcePath) -> Result<Vec<u8>, AuthoringError> {
        let expected = self
            .record
            .files
            .iter()
            .find(|file| &file.path == path)
            .ok_or_else(|| {
                AuthoringError::Cache(format!("cache object does not contain {path}"))
            })?;
        let file = path.join(&self.object_root.join(CACHE_FILES_DIRECTORY));
        let bytes = fs::read(&file).map_err(|error| io_error(&file, error))?;
        if digest_bytes(&bytes) != expected.digest
            || u64::try_from(bytes.len()).unwrap_or(u64::MAX) != expected.size_bytes
        {
            return Err(AuthoringError::Cache(format!(
                "cache file {path} failed digest verification"
            )));
        }
        Ok(bytes)
    }

    pub fn contains_file(&self, path: &NormalizedSourcePath) -> bool {
        self.record.files.iter().any(|file| &file.path == path)
    }

    pub fn materialize_into(&self, target_root: &Path) -> Result<(), AuthoringError> {
        if target_root.exists() {
            return Err(AuthoringError::Cache(format!(
                "dependency materialization target already exists: {}",
                target_root.display()
            )));
        }
        fs::create_dir_all(target_root).map_err(|error| io_error(target_root, error))?;
        for file in &self.record.files {
            let source = file.path.join(&self.object_root.join(CACHE_FILES_DIRECTORY));
            let target = file.path.join(target_root);
            ensure_contained_parent(target_root, &target)?;
            fs::copy(&source, &target).map_err(|error| io_error(&target, error))?;
        }
        Ok(())
    }
}

pub struct NpmResolver<R> {
    identity: NpmResolverIdentity,
    registry: R,
    cache: ContentAddressedNpmCache,
}

impl<R> NpmResolver<R>
where
    R: NpmRegistryPort,
{
    pub fn new(
        identity: NpmResolverIdentity,
        registry: R,
        cache: ContentAddressedNpmCache,
    ) -> Self {
        Self {
            identity,
            registry,
            cache,
        }
    }

    pub fn resolve(
        &self,
        requests: &DependencyRequestSet,
        cancellation: &dyn OperationCancellation,
    ) -> Result<ExactDependencyLock, AuthoringError> {
        check_canceled(cancellation)?;
        let mut graph = ResolutionGraph::default();
        let mut roots = BTreeMap::new();
        for (name, requirement) in requests.dependencies() {
            let target = self.resolve_one(
                name,
                requirement,
                &mut graph,
                cancellation,
            )?;
            roots.insert(name.clone(), target);
        }
        let packages = graph
            .nodes
            .into_iter()
            .map(|(key, node)| {
                LockedNpmPackage::new(
                    node.release.name,
                    node.release.version,
                    node.release.registry_integrity,
                    node.cached.archive_digest().clone(),
                    node.cached.package_json_digest().clone(),
                    node.dependencies,
                )
                .map(|package| (key, package))
            })
            .collect::<Result<Vec<_>, _>>()?;
        ExactDependencyLock::new(
            requests.digest()?,
            self.identity.clone(),
            roots,
            packages,
        )
    }

    fn resolve_one(
        &self,
        name: &str,
        requirement: &str,
        graph: &mut ResolutionGraph,
        cancellation: &dyn OperationCancellation,
    ) -> Result<String, AuthoringError> {
        check_canceled(cancellation)?;
        let release = self.registry.resolve(name, requirement, cancellation)?;
        if release.name != name {
            return Err(AuthoringError::Registry(format!(
                "registry returned {} for request {name}",
                release.name
            )));
        }
        let version = Version::parse(&release.version)
            .map_err(|error| AuthoringError::Registry(error.to_string()))?;
        let matches = if let Ok(exact) = Version::parse(requirement) {
            exact == version
        } else {
            VersionReq::parse(requirement)
                .map_err(|error| AuthoringError::Registry(error.to_string()))?
                .matches(&version)
        };
        if !matches {
            return Err(AuthoringError::Registry(format!(
                "{}@{} does not satisfy {requirement}",
                release.name, release.version
            )));
        }
        let validated = release.validate()?;
        let cached = self.cache.store(&release, cancellation)?;
        let key = format!("{}@{}", release.name, release.version);
        if let Some(existing) = graph.nodes.get(&key) {
            if existing.cached.archive_digest() != cached.archive_digest() {
                return Err(AuthoringError::Registry(format!(
                    "registry returned non-reproducible bytes for {key}"
                )));
            }
            return Ok(key);
        }
        graph.nodes.insert(
            key.clone(),
            ResolvedNode {
                release: release.clone(),
                cached,
                dependencies: BTreeMap::new(),
            },
        );
        if !graph.resolving.insert(key.clone()) {
            return Ok(key);
        }
        let mut dependencies = BTreeMap::new();
        for (dependency, requirement) in validated.dependencies.dependencies() {
            let target =
                self.resolve_one(dependency, requirement, graph, cancellation)?;
            dependencies.insert(dependency.clone(), target);
        }
        graph.resolving.remove(&key);
        graph
            .nodes
            .get_mut(&key)
            .expect("resolution node is inserted before recursion")
            .dependencies = dependencies;
        Ok(key)
    }
}

#[derive(Default)]
struct ResolutionGraph {
    nodes: BTreeMap<String, ResolvedNode>,
    resolving: BTreeSet<String>,
}

struct ResolvedNode {
    release: RegistryPackageRelease,
    cached: CachedNpmPackage,
    dependencies: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NpmCacheRecord {
    format_version: String,
    name: String,
    version: String,
    registry_integrity: String,
    archive_digest: DigestHex,
    package_json_digest: DigestHex,
    files: Vec<NpmCacheFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NpmCacheFile {
    path: NormalizedSourcePath,
    digest: DigestHex,
    size_bytes: u64,
}

#[derive(Serialize)]
struct NpmCacheDigestPayload<'a> {
    format_version: &'static str,
    name: &'a str,
    version: &'a str,
    registry_integrity: &'a str,
    files: &'a [NpmCacheFile],
}

struct ValidatedRegistryPackage {
    dependencies: DependencyRequestSet,
    package_json_digest: DigestHex,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct RegistryPackageJson {
    name: String,
    version: String,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    optional_dependencies: Option<BTreeMap<String, String>>,
    #[serde(default)]
    scripts: Option<BTreeMap<String, String>>,
    #[serde(default)]
    gypfile: Option<bool>,
    #[serde(rename = "type", default)]
    module_type: Option<String>,
    #[serde(default)]
    main: Option<String>,
    #[serde(default)]
    module: Option<String>,
    #[serde(default)]
    exports: Option<serde_json::Value>,
}

fn cache_record(
    release: &RegistryPackageRelease,
    validated: &ValidatedRegistryPackage,
) -> Result<NpmCacheRecord, AuthoringError> {
    let files = release
        .files
        .iter()
        .map(|(path, bytes)| NpmCacheFile {
            path: path.clone(),
            digest: digest_bytes(bytes),
            size_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        })
        .collect::<Vec<_>>();
    let payload = NpmCacheDigestPayload {
        format_version: CACHE_RECORD_VERSION,
        name: &release.name,
        version: &release.version,
        registry_integrity: &release.registry_integrity,
        files: &files,
    };
    let archive_digest = digest_bytes(&canonical_json_bytes(&payload)?);
    Ok(NpmCacheRecord {
        format_version: CACHE_RECORD_VERSION.into(),
        name: release.name.clone(),
        version: release.version.clone(),
        registry_integrity: release.registry_integrity.clone(),
        archive_digest,
        package_json_digest: validated.package_json_digest.clone(),
        files,
    })
}

fn reject_registry_path(path: &NormalizedSourcePath) -> Result<(), AuthoringError> {
    let lower = path.as_str().to_ascii_lowercase();
    if lower
        .split('/')
        .any(|component| component == "node_modules")
        || lower.ends_with(".node")
        || lower.ends_with("/binding.gyp")
        || lower == "binding.gyp"
    {
        return Err(AuthoringError::Registry(format!(
            "forbidden package entry {path}"
        )));
    }
    Ok(())
}

fn ensure_direct_directory(parent: &Path, child: &Path) -> Result<(), AuthoringError> {
    if child.parent() != Some(parent) {
        return Err(AuthoringError::Cache(
            "cache directory is not a direct child".into(),
        ));
    }
    fs::create_dir_all(child).map_err(|error| io_error(child, error))?;
    let canonical = fs::canonicalize(child).map_err(|error| io_error(child, error))?;
    if canonical.parent() != Some(parent) {
        return Err(AuthoringError::Cache(
            "cache directory escaped its parent".into(),
        ));
    }
    Ok(())
}

fn ensure_contained_parent(root: &Path, target: &Path) -> Result<(), AuthoringError> {
    let parent = target
        .parent()
        .ok_or_else(|| AuthoringError::Cache("cache target has no parent".into()))?;
    if !parent.starts_with(root) {
        return Err(AuthoringError::Cache(
            "cache target escaped files root".into(),
        ));
    }
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))
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

fn validate_digest(digest: &DigestHex) -> Result<(), AuthoringError> {
    if digest.as_ref().len() == 64
        && digest
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(AuthoringError::InvalidDigest {
            value: digest.as_ref().to_owned(),
        })
    }
}

struct CacheStagingGuard {
    staging_root: PathBuf,
    path: PathBuf,
}

impl CacheStagingGuard {
    fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for CacheStagingGuard {
    fn drop(&mut self) {
        if self.path.parent() == Some(self.staging_root.as_path())
            && self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("cache-"))
                .is_some_and(|uuid| Uuid::parse_str(uuid).is_ok())
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
