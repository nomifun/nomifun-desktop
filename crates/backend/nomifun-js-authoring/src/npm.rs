use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use flate2::read::GzDecoder;
use nomifun_agent_contracts::{DigestHex, digest_bytes};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha512};
use url::Url;
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
const MAX_REGISTRY_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REGISTRY_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RESOLUTION_PACKAGE_COUNT: usize = 256;
const MAX_RESOLUTION_FILE_COUNT: usize = 65_536;
const MAX_RESOLUTION_DEPTH: usize = 64;
const MAX_RESOLUTION_TRANSFER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RESOLUTION_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RESOLUTION_DURATION: Duration = Duration::from_secs(120);
const NPMJS_REGISTRY_BASE: &str = "https://registry.npmjs.org/";
const NPM_REGISTRY_TIMEOUT: Duration = Duration::from_secs(30);

pub trait NpmRegistryPort: Send + Sync {
    fn resolve(
        &self,
        package_name: &str,
        requirement: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<RegistryPackageRelease, AuthoringError>;
}

impl<T> NpmRegistryPort for Arc<T>
where
    T: NpmRegistryPort + ?Sized,
{
    fn resolve(
        &self,
        package_name: &str,
        requirement: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<RegistryPackageRelease, AuthoringError> {
        (**self).resolve(package_name, requirement, cancellation)
    }
}

/// Bounded, same-origin npm registry adapter for the production Plugin
/// authoring composition. It downloads metadata and the selected tarball,
/// verifies the registry's sha512 SRI before extraction, and never executes an
/// npm lifecycle or package-manager command.
#[derive(Clone)]
pub struct NpmRegistryHttpClient {
    client: ureq::Agent,
    registry_base: Url,
}

impl NpmRegistryHttpClient {
    pub fn npmjs() -> Result<Self, AuthoringError> {
        let registry_base = Url::parse(NPMJS_REGISTRY_BASE)
            .map_err(|error| AuthoringError::Registry(error.to_string()))?;
        Self::new(registry_base)
    }

    pub fn new(registry_base: Url) -> Result<Self, AuthoringError> {
        validate_registry_base(&registry_base)?;
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(NPM_REGISTRY_TIMEOUT))
            .user_agent("NomiFun-Plugin-Authoring/1.0")
            .build();
        let client = ureq::Agent::new_with_config(config);
        Ok(Self {
            client,
            registry_base,
        })
    }

    fn fetch(&self, url: Url, limit: u64) -> Result<Vec<u8>, AuthoringError> {
        if !same_registry_origin(&self.registry_base, &url) {
            return Err(AuthoringError::Registry(
                "npm registry response URL escaped the configured HTTPS origin".into(),
            ));
        }
        let mut response = self
            .client
            .get(url.as_str())
            .call()
            .map_err(|error| AuthoringError::Registry(format!("npm request failed: {error}")))?;
        if !response.status().is_success() {
            return Err(AuthoringError::Registry(format!(
                "npm registry returned HTTP {}",
                response.status()
            )));
        }
        if response
            .body()
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(AuthoringError::Registry(format!(
                "npm response exceeds the fixed {limit}-byte limit"
            )));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                AuthoringError::Registry(format!("cannot read npm response: {error}"))
            })?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
            return Err(AuthoringError::Registry(format!(
                "npm response exceeds the fixed {limit}-byte limit"
            )));
        }
        Ok(bytes)
    }
}

impl NpmRegistryPort for NpmRegistryHttpClient {
    fn resolve(
        &self,
        package_name: &str,
        requirement: &str,
        cancellation: &dyn OperationCancellation,
    ) -> Result<RegistryPackageRelease, AuthoringError> {
        check_canceled(cancellation)?;
        // Reuse the canonical dependency validators before interpolating any
        // user-controlled package name into a URL.
        DependencyRequestSet::new([(
            package_name.to_owned(),
            requirement.to_owned(),
        )])?;
        let encoded_name = package_name.replace('/', "%2F");
        let metadata_url = Url::parse(&format!("{}{}", self.registry_base, encoded_name))
            .map_err(|error| AuthoringError::Registry(error.to_string()))?;
        let metadata_bytes = self.fetch(metadata_url, MAX_REGISTRY_METADATA_BYTES)?;
        check_canceled(cancellation)?;
        let metadata: RegistryMetadata = strict_json_from_slice(&metadata_bytes)
            .map_err(|error| AuthoringError::Registry(format!("invalid npm metadata: {error}")))?;
        if metadata.name != package_name {
            return Err(AuthoringError::Registry(format!(
                "registry returned metadata for {} instead of {package_name}",
                metadata.name
            )));
        }
        let selected = select_registry_version(&metadata.versions, requirement)?;
        if selected.name != package_name {
            return Err(AuthoringError::Registry(
                "selected npm version identity differs from the request".into(),
            ));
        }
        let tarball_url = Url::parse(&selected.dist.tarball).map_err(|error| {
            AuthoringError::Registry(format!("invalid npm tarball URL: {error}"))
        })?;
        if !same_registry_origin(&self.registry_base, &tarball_url) {
            return Err(AuthoringError::Registry(
                "npm tarball must remain on the configured HTTPS registry origin".into(),
            ));
        }
        let archive = self.fetch(tarball_url, MAX_REGISTRY_ARCHIVE_BYTES)?;
        check_canceled(cancellation)?;
        let integrity_value = selected.dist.integrity.as_deref().ok_or_else(|| {
            AuthoringError::Registry("selected npm release is missing registry integrity".into())
        })?;
        let integrity = verify_sha512_integrity(integrity_value, &archive)?;
        let files = extract_registry_archive(&archive, cancellation)?;
        RegistryPackageRelease::new_with_registry_transfer_bytes(
            selected.name.clone(),
            selected.version.clone(),
            integrity,
            files,
            u64::try_from(metadata_bytes.len())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::try_from(archive.len()).unwrap_or(u64::MAX)),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryPackageRelease {
    name: String,
    version: String,
    registry_integrity: String,
    files: BTreeMap<NormalizedSourcePath, Vec<u8>>,
    registry_transfer_bytes: u64,
}

impl RegistryPackageRelease {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        registry_integrity: impl Into<String>,
        files: impl IntoIterator<Item = (NormalizedSourcePath, Vec<u8>)>,
    ) -> Result<Self, AuthoringError> {
        Self::new_internal(name, version, registry_integrity, files, None)
    }

    fn new_with_registry_transfer_bytes(
        name: impl Into<String>,
        version: impl Into<String>,
        registry_integrity: impl Into<String>,
        files: impl IntoIterator<Item = (NormalizedSourcePath, Vec<u8>)>,
        registry_transfer_bytes: u64,
    ) -> Result<Self, AuthoringError> {
        Self::new_internal(
            name,
            version,
            registry_integrity,
            files,
            Some(registry_transfer_bytes),
        )
    }

    fn new_internal(
        name: impl Into<String>,
        version: impl Into<String>,
        registry_integrity: impl Into<String>,
        files: impl IntoIterator<Item = (NormalizedSourcePath, Vec<u8>)>,
        registry_transfer_bytes: Option<u64>,
    ) -> Result<Self, AuthoringError> {
        let mut file_map = BTreeMap::new();
        let mut collision_keys = BTreeSet::new();
        for (path, bytes) in files {
            let collision_key = path.collision_key().map_err(|error| {
                AuthoringError::Registry(error.to_string())
            })?;
            if !collision_keys.insert(collision_key) {
                return Err(AuthoringError::Registry(format!(
                    "package contains a Windows-colliding path {path}"
                )));
            }
            if file_map.insert(path.clone(), bytes).is_some() {
                return Err(AuthoringError::Registry(format!(
                    "package contains duplicate path {path}"
                )));
            }
        }
        let extracted_bytes = file_map.values().fold(0u64, |total, bytes| {
            total.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
        });
        let release = Self {
            name: name.into(),
            version: version.into(),
            registry_integrity: registry_integrity.into(),
            files: file_map,
            registry_transfer_bytes: registry_transfer_bytes.unwrap_or(extracted_bytes),
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

    fn extracted_bytes(&self) -> u64 {
        self.files.values().fold(0u64, |total, bytes| {
            total.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
        })
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
        let validated =
            validate_registry_package_json(package_json, &self.name, &self.version)?;
        if !self.files.contains_key(validated.entrypoint()) {
            return Err(AuthoringError::Registry(format!(
                "npm package entrypoint {} is missing",
                validated.entrypoint()
            )));
        }
        Ok(validated)
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
        let mut graph = ResolutionGraph::new();
        let mut roots = BTreeMap::new();
        for (name, requirement) in requests.dependencies() {
            let target = self.resolve_one(
                name,
                requirement,
                &mut graph,
                cancellation,
                0,
            )?;
            roots.insert(name.clone(), target);
        }
        let packages = graph
            .nodes
            .into_iter()
            .map(|(key, node)| {
                LockedNpmPackage::new(
                    node.name,
                    node.version,
                    node.registry_integrity,
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
        depth: usize,
    ) -> Result<String, AuthoringError> {
        check_canceled(cancellation)?;
        graph.ensure_budget(depth)?;
        let request_key = (name.to_owned(), requirement.to_owned());
        if let Some(key) = graph.requests.get(&request_key) {
            return Ok(key.clone());
        }
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
        graph.ensure_budget(depth)?;
        let key = format!("{}@{}", release.name, release.version);
        graph.record_release(&release)?;
        if !graph.nodes.contains_key(&key)
            && graph.nodes.len() >= MAX_RESOLUTION_PACKAGE_COUNT
        {
            return Err(AuthoringError::Registry(format!(
                "npm resolution exceeds the fixed {MAX_RESOLUTION_PACKAGE_COUNT}-package limit"
            )));
        }
        let cached = self.cache.store(&release, cancellation)?;
        graph.ensure_budget(depth)?;
        if let Some(existing) = graph.nodes.get(&key) {
            if existing.cached.archive_digest() != cached.archive_digest() {
                return Err(AuthoringError::Registry(format!(
                    "registry returned non-reproducible bytes for {key}"
                )));
            }
            graph.requests.insert(request_key, key.clone());
            return Ok(key);
        }
        let dependency_requests = validated.dependencies().dependencies().clone();
        graph.nodes.insert(
            key.clone(),
            ResolvedNode {
                name: release.name.clone(),
                version: release.version.clone(),
                registry_integrity: release.registry_integrity.clone(),
                cached,
                dependencies: BTreeMap::new(),
            },
        );
        graph.requests.insert(request_key, key.clone());
        // Only the immutable cache record and small lock facts are needed
        // during recursive resolution; do not retain every package's bytes on
        // the recursion stack.
        drop(validated);
        drop(release);
        if !graph.resolving.insert(key.clone()) {
            return Ok(key);
        }
        let mut dependencies = BTreeMap::new();
        for (dependency, requirement) in &dependency_requests {
            let target = self.resolve_one(
                dependency,
                requirement,
                graph,
                cancellation,
                depth + 1,
            )?;
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

struct ResolutionGraph {
    nodes: BTreeMap<String, ResolvedNode>,
    resolving: BTreeSet<String>,
    requests: BTreeMap<(String, String), String>,
    started_at: std::time::Instant,
    transfer_bytes: u64,
    extracted_bytes: u64,
    file_count: usize,
}

impl ResolutionGraph {
    fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            resolving: BTreeSet::new(),
            requests: BTreeMap::new(),
            started_at: std::time::Instant::now(),
            transfer_bytes: 0,
            extracted_bytes: 0,
            file_count: 0,
        }
    }

    fn ensure_budget(&self, depth: usize) -> Result<(), AuthoringError> {
        if depth >= MAX_RESOLUTION_DEPTH {
            return Err(AuthoringError::Registry(format!(
                "npm resolution exceeds the fixed {MAX_RESOLUTION_DEPTH}-level depth limit"
            )));
        }
        if self.started_at.elapsed() > MAX_RESOLUTION_DURATION {
            return Err(AuthoringError::Registry(format!(
                "npm resolution exceeds the fixed {MAX_RESOLUTION_DURATION:?} time budget"
            )));
        }
        Ok(())
    }

    fn record_release(&mut self, release: &RegistryPackageRelease) -> Result<(), AuthoringError> {
        self.transfer_bytes = self
            .transfer_bytes
            .checked_add(release.registry_transfer_bytes)
            .ok_or_else(|| AuthoringError::Registry("npm transfer budget overflow".into()))?;
        self.extracted_bytes = self
            .extracted_bytes
            .checked_add(release.extracted_bytes())
            .ok_or_else(|| AuthoringError::Registry("npm extraction budget overflow".into()))?;
        self.file_count = self
            .file_count
            .checked_add(release.files.len())
            .ok_or_else(|| AuthoringError::Registry("npm file-count budget overflow".into()))?;
        if self.transfer_bytes > MAX_RESOLUTION_TRANSFER_BYTES
            || self.extracted_bytes > MAX_RESOLUTION_EXTRACTED_BYTES
            || self.file_count > MAX_RESOLUTION_FILE_COUNT
        {
            return Err(AuthoringError::Registry(
                "npm resolution exceeds its fixed cumulative resource budget".into(),
            ));
        }
        Ok(())
    }
}

struct ResolvedNode {
    name: String,
    version: String,
    registry_integrity: String,
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

pub(crate) struct ValidatedRegistryPackage {
    dependencies: DependencyRequestSet,
    package_json_digest: DigestHex,
    entrypoint: NormalizedSourcePath,
}

impl ValidatedRegistryPackage {
    pub(crate) fn dependencies(&self) -> &DependencyRequestSet {
        &self.dependencies
    }

    pub(crate) fn entrypoint(&self) -> &NormalizedSourcePath {
        &self.entrypoint
    }
}

#[derive(Deserialize)]
struct RegistryMetadata {
    name: String,
    versions: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
struct RegistryVersionMetadata {
    name: String,
    version: String,
    dist: RegistryDistribution,
}

#[derive(Deserialize, Serialize)]
struct RegistryDistribution {
    tarball: String,
    #[serde(default)]
    integrity: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct RegistryPackageJson {
    name: String,
    version: String,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    optional_dependencies: Option<BTreeMap<String, String>>,
    #[serde(default)]
    peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(
        rename = "bundledDependencies",
        alias = "bundleDependencies",
        default
    )]
    bundled_dependencies: Option<serde_json::Value>,
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
    #[serde(default)]
    os: Option<Vec<String>>,
    #[serde(default)]
    cpu: Option<Vec<String>>,
    #[serde(default)]
    libc: Option<Vec<String>>,
}

fn is_npm_lifecycle_script(name: &str) -> bool {
    matches!(
        name,
        "preinstall"
            | "install"
            | "postinstall"
            | "preuninstall"
            | "uninstall"
            | "postuninstall"
            | "prepublish"
            | "prepublishOnly"
            | "prepare"
            | "prepack"
            | "postpack"
    )
}

fn nonempty_json_collection(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(values) => !values.is_empty(),
        serde_json::Value::Object(values) => !values.is_empty(),
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Null => false,
        _ => true,
    }
}

pub(crate) fn validate_registry_package_json(
    package_json: &[u8],
    expected_name: &str,
    expected_version: &str,
) -> Result<ValidatedRegistryPackage, AuthoringError> {
    let package: RegistryPackageJson = strict_json_from_slice(package_json)
        .map_err(|error| AuthoringError::Registry(error.to_string()))?;
    if package.name != expected_name || package.version != expected_version {
        return Err(AuthoringError::Registry(
            "package.json identity does not match registry release".into(),
        ));
    }
    if package.module_type.as_deref() != Some("module") {
        return Err(AuthoringError::Registry(
            "npm package must declare type=\"module\" for the fixed ESM bundler".into(),
        ));
    }
    if package.scripts.as_ref().is_some_and(|scripts| {
        scripts
            .keys()
            .any(|script| is_npm_lifecycle_script(script))
    }) {
        return Err(AuthoringError::Registry(
            "npm lifecycle scripts are forbidden".into(),
        ));
    }
    if package.gypfile == Some(true)
        || package
            .optional_dependencies
            .as_ref()
            .is_some_and(|dependencies| !dependencies.is_empty())
        || package
            .peer_dependencies
            .as_ref()
            .is_some_and(|dependencies| !dependencies.is_empty())
        || package
            .bundled_dependencies
            .as_ref()
            .is_some_and(nonempty_json_collection)
        || package.os.as_ref().is_some_and(|values| !values.is_empty())
        || package.cpu.as_ref().is_some_and(|values| !values.is_empty())
        || package.libc.as_ref().is_some_and(|values| !values.is_empty())
    {
        return Err(AuthoringError::Registry(
            "native, optional, peer, bundled, or platform-specific install surfaces are forbidden"
                .into(),
        ));
    }
    let entrypoint = registry_package_entrypoint(&package)?;
    Ok(ValidatedRegistryPackage {
        dependencies: DependencyRequestSet::new(package.dependencies)?,
        package_json_digest: digest_bytes(package_json),
        entrypoint,
    })
}

fn registry_package_entrypoint(
    package: &RegistryPackageJson,
) -> Result<NormalizedSourcePath, AuthoringError> {
    let declared = if let Some(main) = package.main.as_deref() {
        main
    } else if let Some(module) = package.module.as_deref() {
        module
    } else if let Some(exports) = package.exports.as_ref() {
        simple_export_entrypoint(exports).ok_or_else(|| {
            AuthoringError::Registry(
                "npm package exports are not a supported single ESM entrypoint".into(),
            )
        })?
    } else {
        "index.js"
    };
    let declared = declared.strip_prefix("./").unwrap_or(declared);
    let entrypoint = NormalizedSourcePath::parse(declared.to_owned())
        .map_err(|error| AuthoringError::Registry(error.to_string()))?;
    let lower = entrypoint.as_str().to_ascii_lowercase();
    if !lower.ends_with(".js") && !lower.ends_with(".mjs") {
        return Err(AuthoringError::Registry(
            "npm package entrypoint must be a JavaScript ESM file".into(),
        ));
    }
    Ok(entrypoint)
}

fn simple_export_entrypoint(exports: &serde_json::Value) -> Option<&str> {
    match exports {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Object(values) => match values.get(".") {
            Some(serde_json::Value::String(value)) => Some(value),
            Some(serde_json::Value::Object(root)) => root
                .get("import")
                .or_else(|| root.get("default"))
                .and_then(serde_json::Value::as_str),
            _ => None,
        },
        _ => None,
    }
}

fn validate_registry_base(registry_base: &Url) -> Result<(), AuthoringError> {
    if registry_base.scheme() != "https"
        || registry_base.host_str().is_none()
        || registry_base.username() != ""
        || registry_base.password().is_some()
        || registry_base.query().is_some()
        || registry_base.fragment().is_some()
        || !registry_base.path().ends_with('/')
    {
        return Err(AuthoringError::Registry(
            "npm registry base must be an HTTPS origin path ending in '/' without credentials, query, or fragment"
                .into(),
        ));
    }
    Ok(())
}

fn same_registry_origin(registry_base: &Url, candidate: &Url) -> bool {
    candidate.scheme() == "https"
        && candidate.username().is_empty()
        && candidate.password().is_none()
        && candidate.host_str() == registry_base.host_str()
        && candidate.port_or_known_default() == registry_base.port_or_known_default()
}

fn select_registry_version(
    versions: &BTreeMap<String, serde_json::Value>,
    requirement: &str,
) -> Result<RegistryVersionMetadata, AuthoringError> {
    let exact = Version::parse(requirement).ok();
    let range = if exact.is_none() {
        Some(VersionReq::parse(requirement).map_err(|error| {
            AuthoringError::Registry(format!("invalid npm SemVer requirement: {error}"))
        })?)
    } else {
        None
    };
    let mut matches = versions
        .keys()
        .filter_map(|key| {
            let version = Version::parse(key).ok()?;
            if version.to_string() != *key {
                return None;
            }
            let matched = exact
                .as_ref()
                .is_some_and(|exact| exact == &version)
                || range
                    .as_ref()
                    .is_some_and(|requirement| requirement.matches(&version));
            matched.then_some((version, key))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.0.cmp(&right.0));
    let selected = matches
        .pop()
        .map(|(_, key)| key)
        .ok_or_else(|| {
            AuthoringError::Registry(format!(
                "registry contains no version matching {requirement}"
            ))
        })?;
    let release: RegistryVersionMetadata = serde_json::from_value(
        versions
            .get(selected)
            .expect("selected registry version key came from the map")
            .clone(),
    )
    .map_err(|error| {
        AuthoringError::Registry(format!("selected npm version metadata is invalid: {error}"))
    })?;
    if release.version.as_str() != selected {
        return Err(AuthoringError::Registry(
            "selected npm version payload does not match its metadata key".into(),
        ));
    }
    Ok(release)
}

fn verify_sha512_integrity(integrity: &str, archive: &[u8]) -> Result<String, AuthoringError> {
    let mut candidates = integrity
        .split_ascii_whitespace()
        .filter(|value| value.starts_with("sha512-"));
    let selected = candidates.next().ok_or_else(|| {
        AuthoringError::Registry("npm release is missing sha512 integrity".into())
    })?;
    if candidates.next().is_some() {
        return Err(AuthoringError::Registry(
            "npm release contains multiple sha512 integrity values".into(),
        ));
    }
    let expected = BASE64_STANDARD
        .decode(selected.trim_start_matches("sha512-"))
        .map_err(|error| AuthoringError::Registry(format!("invalid npm sha512 SRI: {error}")))?;
    let observed = Sha512::digest(archive);
    if expected.as_slice() != observed.as_slice() {
        return Err(AuthoringError::Registry(
            "npm tarball failed sha512 integrity verification".into(),
        ));
    }
    Ok(selected.to_owned())
}

fn extract_registry_archive(
    archive: &[u8],
    cancellation: &dyn OperationCancellation,
) -> Result<BTreeMap<NormalizedSourcePath, Vec<u8>>, AuthoringError> {
    let decoder = GzDecoder::new(Cursor::new(archive));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| AuthoringError::Registry(format!("invalid npm tarball: {error}")))?;
    let mut files = BTreeMap::new();
    let mut collision_keys = BTreeSet::new();
    let mut total = 0u64;
    let mut entry_count = 0usize;
    for entry in entries {
        check_canceled(cancellation)?;
        entry_count = entry_count.saturating_add(1);
        if entry_count > MAX_REGISTRY_FILE_COUNT {
            return Err(AuthoringError::Registry(
                "npm tarball exceeds the fixed entry-count limit".into(),
            ));
        }
        let mut entry = entry
            .map_err(|error| AuthoringError::Registry(format!("invalid npm entry: {error}")))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        if !entry_type.is_file() {
            return Err(AuthoringError::Registry(
                "npm tarball contains a link or special file".into(),
            ));
        }
        let path_bytes = entry.path_bytes();
        let archive_path = std::str::from_utf8(path_bytes.as_ref()).map_err(|_| {
            AuthoringError::Registry("npm tarball path is not valid UTF-8".into())
        })?;
        let relative = archive_path.strip_prefix("package/").ok_or_else(|| {
            AuthoringError::Registry(
                "npm tarball entry is outside the canonical package/ root".into(),
            )
        })?;
        let path = NormalizedSourcePath::parse(relative.to_owned())
            .map_err(|error| AuthoringError::Registry(error.to_string()))?;
        reject_registry_path(&path)?;
        let collision_key = path
            .collision_key()
            .map_err(|error| AuthoringError::Registry(error.to_string()))?;
        if !collision_keys.insert(collision_key) {
            return Err(AuthoringError::Registry(format!(
                "npm tarball contains a Windows-colliding path {path}"
            )));
        }
        if entry.size() > MAX_REGISTRY_FILE_BYTES {
            return Err(AuthoringError::Registry(format!(
                "npm package file {path} exceeds fixed limits"
            )));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_REGISTRY_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                AuthoringError::Registry(format!("cannot read npm package file {path}: {error}"))
            })?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > MAX_REGISTRY_FILE_BYTES {
            return Err(AuthoringError::Registry(format!(
                "npm package file {path} exceeds fixed limits"
            )));
        }
        total = total.saturating_add(size);
        if files.len() >= MAX_REGISTRY_FILE_COUNT || total > MAX_REGISTRY_TOTAL_BYTES {
            return Err(AuthoringError::Registry(
                "npm tarball exceeds fixed inventory limits".into(),
            ));
        }
        if files.insert(path.clone(), bytes).is_some() {
            return Err(AuthoringError::Registry(format!(
                "npm tarball contains duplicate path {path}"
            )));
        }
    }
    Ok(files)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NeverCancel;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::{Builder, EntryType, Header};

    fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        for (path, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, *path, Cursor::new(*bytes))
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn symlink_tarball() -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_mode(0o777);
        header.set_size(0);
        header.set_cksum();
        builder
            .append_link(&mut header, "package/link.js", "../escape.js")
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn release(version: &str) -> RegistryVersionMetadata {
        RegistryVersionMetadata {
            name: "alpha".into(),
            version: version.into(),
            dist: RegistryDistribution {
                tarball: format!("https://registry.npmjs.org/alpha/-/alpha-{version}.tgz"),
                integrity: Some("sha512-unused".into()),
            },
        }
    }

    #[test]
    fn registry_selects_highest_matching_canonical_version() {
        let mut versions: BTreeMap<String, serde_json::Value> =
            ["1.0.0", "1.4.0", "2.0.0"]
            .into_iter()
            .map(|version| {
                (
                    version.to_owned(),
                    serde_json::to_value(release(version)).unwrap(),
                )
            })
            .collect();
        versions.insert("0.1.0".into(), serde_json::json!({"legacy": true}));
        assert_eq!(
            select_registry_version(&versions, "^1.0.0")
                .unwrap()
                .version,
            "1.4.0"
        );
        assert_eq!(
            select_registry_version(&versions, "1.0.0")
                .unwrap()
                .version,
            "1.0.0"
        );
    }

    #[test]
    fn registry_verifies_sri_then_extracts_only_package_files() {
        let package = br#"{"name":"alpha","version":"1.0.0"}"#;
        let archive = tarball(&[
            ("package/package.json", package),
            ("package/index.js", b"export const value = 1;\n"),
        ]);
        let integrity = format!(
            "sha512-{}",
            BASE64_STANDARD.encode(Sha512::digest(&archive))
        );
        assert_eq!(
            verify_sha512_integrity(&integrity, &archive).unwrap(),
            integrity
        );
        let files = extract_registry_archive(&archive, &NeverCancel).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(
            files
                .get(&NormalizedSourcePath::parse("index.js").unwrap())
                .unwrap(),
            b"export const value = 1;\n"
        );
        assert!(verify_sha512_integrity(&integrity, b"tampered").is_err());
    }

    #[test]
    fn registry_rejects_entries_outside_package_root() {
        let archive = tarball(&[("escape.js", b"bad")]);
        assert!(extract_registry_archive(&archive, &NeverCancel).is_err());
        let collision = tarball(&[
            ("package/Index.js", b"first"),
            ("package/index.js", b"second"),
        ]);
        assert!(extract_registry_archive(&collision, &NeverCancel).is_err());
        assert!(extract_registry_archive(&symlink_tarball(), &NeverCancel).is_err());
        assert!(NpmRegistryHttpClient::new(
            Url::parse("http://registry.example.test/").unwrap()
        )
        .is_err());
    }

    #[test]
    #[ignore = "requires access to the public npm registry"]
    fn npmjs_adapter_resolves_and_verifies_a_real_pure_javascript_release() {
        let temp = tempfile::tempdir().unwrap();
        let cache = ContentAddressedNpmCache::new(temp.path().join("cache")).unwrap();
        let resolver = NpmResolver::new(
            NpmResolverIdentity::new("nomifun-npm", "1.0.0").unwrap(),
            NpmRegistryHttpClient::npmjs().unwrap(),
            cache.clone(),
        );
        let requests = DependencyRequestSet::new([(
            "yocto-queue".to_owned(),
            "1.2.1".to_owned(),
        )])
        .unwrap();
        let lock = resolver.resolve(&requests, &NeverCancel).unwrap();
        let package = lock.packages().get("yocto-queue@1.2.1").unwrap();
        assert_eq!(package.name(), "yocto-queue");
        assert_eq!(package.version(), "1.2.1");
        let cached = cache.load(package.archive_sha256()).unwrap();
        assert!(cached.contains_file(
            &NormalizedSourcePath::parse("package.json").unwrap()
        ));
    }
}
