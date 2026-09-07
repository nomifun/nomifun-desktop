use std::collections::{BTreeMap, BTreeSet};

use semver::{Version, VersionReq};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use nomifun_agent_contracts::DigestHex;

use crate::canonical::{canonical_digest, strict_json_from_slice};
use crate::error::AuthoringError;

pub const DEPENDENCY_REQUEST_FORMAT_VERSION: &str = "1.0.0";
pub const DEPENDENCY_LOCK_FORMAT_VERSION: &str = "1.0.0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DependencyRequestSet {
    format_version: String,
    dependencies: BTreeMap<String, String>,
}

impl DependencyRequestSet {
    pub fn new(
        dependencies: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, AuthoringError> {
        let mut validated = BTreeMap::new();
        for (name, requirement) in dependencies {
            validate_npm_package_name(&name)?;
            validate_registry_requirement(&requirement)?;
            if validated.insert(name.clone(), requirement).is_some() {
                return Err(AuthoringError::InvalidDependency(format!(
                    "duplicate direct dependency {name}"
                )));
            }
        }
        Ok(Self {
            format_version: DEPENDENCY_REQUEST_FORMAT_VERSION.into(),
            dependencies: validated,
        })
    }

    pub fn empty() -> Self {
        Self {
            format_version: DEPENDENCY_REQUEST_FORMAT_VERSION.into(),
            dependencies: BTreeMap::new(),
        }
    }

    pub fn from_package_json(bytes: &[u8]) -> Result<Self, AuthoringError> {
        let package: FixedPackageJson = strict_json_from_slice(bytes)?;
        if !package.private {
            return Err(AuthoringError::InvalidPackageJson(
                "package.json must set private=true".into(),
            ));
        }
        if package.module_type != "module" {
            return Err(AuthoringError::InvalidPackageJson(
                "package.json must set type=\"module\"".into(),
            ));
        }
        if let Some(name) = &package.name {
            validate_npm_package_name(name)?;
        }
        if let Some(version) = &package.version {
            Version::parse(version).map_err(|error| {
                AuthoringError::InvalidPackageJson(format!(
                    "package.json version must be exact SemVer: {error}"
                ))
            })?;
        }
        if package
            .description
            .as_ref()
            .is_some_and(|value| value.chars().any(char::is_control))
        {
            return Err(AuthoringError::InvalidPackageJson(
                "package.json description contains control characters".into(),
            ));
        }
        Self::new(package.dependencies)
    }

    pub fn format_version(&self) -> &str {
        &self.format_version
    }

    pub fn dependencies(&self) -> &BTreeMap<String, String> {
        &self.dependencies
    }

    pub fn digest(&self) -> Result<DigestHex, AuthoringError> {
        canonical_digest(self)
    }
}

impl<'de> Deserialize<'de> for DependencyRequestSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DependencyRequestSetWire::deserialize(deserializer)?;
        if wire.format_version != DEPENDENCY_REQUEST_FORMAT_VERSION {
            return Err(D::Error::custom(format!(
                "dependency request format must be {DEPENDENCY_REQUEST_FORMAT_VERSION}"
            )));
        }
        Self::new(wire.dependencies).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyRequestSetWire {
    format_version: String,
    dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedPackageJson {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    private: bool,
    #[serde(rename = "type")]
    module_type: String,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockedPackagePolicy {
    PureJavaScriptNoLifecycleScripts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpmResolverIdentity {
    name: String,
    version: String,
}

impl NpmResolverIdentity {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self, AuthoringError> {
        let value = Self {
            name: name.into(),
            version: version.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    fn validate(&self) -> Result<(), AuthoringError> {
        validate_ascii_identifier(&self.name, "resolver.name")?;
        Version::parse(&self.version).map_err(|error| {
            AuthoringError::InvalidDependencyLock(format!(
                "resolver.version must be exact SemVer: {error}"
            ))
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedNpmPackage {
    name: String,
    version: String,
    registry_integrity: String,
    archive_sha256: DigestHex,
    package_json_sha256: DigestHex,
    policy: LockedPackagePolicy,
    dependencies: BTreeMap<String, String>,
}

impl LockedNpmPackage {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        registry_integrity: impl Into<String>,
        archive_sha256: DigestHex,
        package_json_sha256: DigestHex,
        dependencies: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, AuthoringError> {
        let version = Version::parse(&version.into()).map_err(|error| {
            AuthoringError::InvalidDependencyLock(format!(
                "locked package version must be exact SemVer: {error}"
            ))
        })?;
        let mut dependency_map = BTreeMap::new();
        for (name, target) in dependencies {
            if dependency_map.insert(name.clone(), target).is_some() {
                return Err(AuthoringError::InvalidDependencyLock(format!(
                    "locked package contains duplicate dependency {name}"
                )));
            }
        }
        let package = Self {
            name: name.into(),
            version: version.to_string(),
            registry_integrity: registry_integrity.into(),
            archive_sha256,
            package_json_sha256,
            policy: LockedPackagePolicy::PureJavaScriptNoLifecycleScripts,
            dependencies: dependency_map,
        };
        package.validate()?;
        Ok(package)
    }

    pub fn key(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn dependencies(&self) -> &BTreeMap<String, String> {
        &self.dependencies
    }

    pub fn registry_integrity(&self) -> &str {
        &self.registry_integrity
    }

    pub fn archive_sha256(&self) -> &DigestHex {
        &self.archive_sha256
    }

    pub fn package_json_sha256(&self) -> &DigestHex {
        &self.package_json_sha256
    }

    pub fn policy(&self) -> &LockedPackagePolicy {
        &self.policy
    }

    fn validate(&self) -> Result<(), AuthoringError> {
        validate_npm_package_name(&self.name)?;
        validate_digest(&self.archive_sha256)?;
        validate_digest(&self.package_json_sha256)?;
        let parsed = Version::parse(&self.version).map_err(|error| {
            AuthoringError::InvalidDependencyLock(format!(
                "locked package {} has invalid exact version: {error}",
                self.name
            ))
        })?;
        if parsed.to_string() != self.version {
            return Err(AuthoringError::InvalidDependencyLock(format!(
                "locked package {} version is not canonical",
                self.name
            )));
        }
        validate_registry_integrity(&self.registry_integrity)?;
        if self.policy != LockedPackagePolicy::PureJavaScriptNoLifecycleScripts {
            return Err(AuthoringError::InvalidDependencyLock(format!(
                "locked package {} is not certified pure JavaScript without lifecycle scripts",
                self.name
            )));
        }
        for (dependency, target) in &self.dependencies {
            validate_npm_package_name(dependency)?;
            if target.is_empty() || target.chars().any(char::is_control) {
                return Err(AuthoringError::InvalidDependencyLock(format!(
                    "locked dependency target for {dependency} is invalid"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExactDependencyLock {
    format_version: String,
    request_digest: DigestHex,
    resolver: NpmResolverIdentity,
    roots: BTreeMap<String, String>,
    packages: BTreeMap<String, LockedNpmPackage>,
}

impl ExactDependencyLock {
    pub fn new(
        request_digest: DigestHex,
        resolver: NpmResolverIdentity,
        roots: impl IntoIterator<Item = (String, String)>,
        packages: impl IntoIterator<Item = (String, LockedNpmPackage)>,
    ) -> Result<Self, AuthoringError> {
        let mut root_map = BTreeMap::new();
        for (name, target) in roots {
            if root_map.insert(name.clone(), target).is_some() {
                return Err(AuthoringError::InvalidDependencyLock(format!(
                    "lock contains duplicate root {name}"
                )));
            }
        }
        let mut package_map = BTreeMap::new();
        for (key, package) in packages {
            if package_map.insert(key.clone(), package).is_some() {
                return Err(AuthoringError::InvalidDependencyLock(format!(
                    "lock contains duplicate package key {key}"
                )));
            }
        }
        let lock = Self {
            format_version: DEPENDENCY_LOCK_FORMAT_VERSION.into(),
            request_digest,
            resolver,
            roots: root_map,
            packages: package_map,
        };
        lock.validate()?;
        Ok(lock)
    }

    pub fn empty(
        requests: &DependencyRequestSet,
        resolver: NpmResolverIdentity,
    ) -> Result<Self, AuthoringError> {
        if !requests.dependencies().is_empty() {
            return Err(AuthoringError::InvalidDependencyLock(
                "an empty lock requires an empty dependency request".into(),
            ));
        }
        Self::new(requests.digest()?, resolver, [], [])
    }

    pub fn format_version(&self) -> &str {
        &self.format_version
    }

    pub fn request_digest(&self) -> &DigestHex {
        &self.request_digest
    }

    pub fn resolver(&self) -> &NpmResolverIdentity {
        &self.resolver
    }

    pub fn roots(&self) -> &BTreeMap<String, String> {
        &self.roots
    }

    pub fn packages(&self) -> &BTreeMap<String, LockedNpmPackage> {
        &self.packages
    }

    pub fn digest(&self) -> Result<DigestHex, AuthoringError> {
        canonical_digest(self)
    }

    pub fn validate_against(&self, requests: &DependencyRequestSet) -> Result<(), AuthoringError> {
        let digest = requests.digest()?;
        if digest != self.request_digest {
            return Err(AuthoringError::InvalidDependencyLock(
                "lock request_digest does not bind the exact dependency request".into(),
            ));
        }
        let requested = requests.dependencies.keys().collect::<BTreeSet<_>>();
        let roots = self.roots.keys().collect::<BTreeSet<_>>();
        if requested != roots {
            return Err(AuthoringError::InvalidDependencyLock(
                "lock roots do not exactly match direct dependency requests".into(),
            ));
        }
        for (name, requirement) in requests.dependencies() {
            let target = self.roots.get(name).ok_or_else(|| {
                AuthoringError::InvalidDependencyLock(format!(
                    "lock is missing direct dependency root {name}"
                ))
            })?;
            let package = self.packages.get(target).ok_or_else(|| {
                AuthoringError::InvalidDependencyLock(format!(
                    "direct dependency {name} targets missing package {target}"
                ))
            })?;
            let version = Version::parse(&package.version).map_err(|error| {
                AuthoringError::InvalidDependencyLock(format!(
                    "direct dependency {name} has invalid locked version: {error}"
                ))
            })?;
            if !registry_requirement_matches(requirement, &version)? {
                return Err(AuthoringError::InvalidDependencyLock(format!(
                    "locked version {version} does not satisfy {name} request {requirement}"
                )));
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), AuthoringError> {
        if self.format_version != DEPENDENCY_LOCK_FORMAT_VERSION {
            return Err(AuthoringError::InvalidDependencyLock(format!(
                "format_version must be {DEPENDENCY_LOCK_FORMAT_VERSION}"
            )));
        }
        validate_digest(&self.request_digest)?;
        self.resolver.validate()?;
        for (key, package) in &self.packages {
            package.validate()?;
            if key != &package.key() {
                return Err(AuthoringError::InvalidDependencyLock(format!(
                    "package map key {key} does not match {}",
                    package.key()
                )));
            }
        }

        for (name, target) in &self.roots {
            validate_npm_package_name(name)?;
            let package = self.packages.get(target).ok_or_else(|| {
                AuthoringError::InvalidDependencyLock(format!(
                    "root dependency {name} targets missing package {target}"
                ))
            })?;
            if package.name != *name {
                return Err(AuthoringError::InvalidDependencyLock(format!(
                    "root dependency {name} targets package {}",
                    package.name
                )));
            }
        }

        for package in self.packages.values() {
            for (name, target) in &package.dependencies {
                let dependency = self.packages.get(target).ok_or_else(|| {
                    AuthoringError::InvalidDependencyLock(format!(
                        "{} dependency {name} targets missing package {target}",
                        package.key()
                    ))
                })?;
                if dependency.name != *name {
                    return Err(AuthoringError::InvalidDependencyLock(format!(
                        "{} dependency {name} targets package {}",
                        package.key(),
                        dependency.name
                    )));
                }
            }
        }

        let mut reachable = BTreeSet::new();
        let mut pending = self.roots.values().cloned().collect::<Vec<_>>();
        while let Some(key) = pending.pop() {
            if !reachable.insert(key.clone()) {
                continue;
            }
            let package = self.packages.get(&key).ok_or_else(|| {
                AuthoringError::InvalidDependencyLock(format!("reachable package {key} is missing"))
            })?;
            pending.extend(package.dependencies.values().cloned());
        }
        if reachable.len() != self.packages.len() {
            return Err(AuthoringError::InvalidDependencyLock(
                "lock contains packages unreachable from direct roots".into(),
            ));
        }
        Ok(())
    }
}

fn validate_digest(digest: &DigestHex) -> Result<(), AuthoringError> {
    let value = digest.as_ref();
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(AuthoringError::InvalidDigest {
            value: value.to_owned(),
        })
    }
}

impl<'de> Deserialize<'de> for ExactDependencyLock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ExactDependencyLockWire::deserialize(deserializer)?;
        if wire.format_version != DEPENDENCY_LOCK_FORMAT_VERSION {
            return Err(D::Error::custom(format!(
                "dependency lock format must be {DEPENDENCY_LOCK_FORMAT_VERSION}"
            )));
        }
        Self::new(
            wire.request_digest,
            wire.resolver,
            wire.roots,
            wire.packages,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExactDependencyLockWire {
    format_version: String,
    request_digest: DigestHex,
    resolver: NpmResolverIdentity,
    roots: BTreeMap<String, String>,
    packages: BTreeMap<String, LockedNpmPackage>,
}

fn validate_npm_package_name(name: &str) -> Result<(), AuthoringError> {
    if name.is_empty() || name.len() > 214 || name.chars().any(char::is_control) {
        return Err(AuthoringError::InvalidDependency(format!(
            "invalid npm package name {name:?}"
        )));
    }
    let parts = if let Some(scoped) = name.strip_prefix('@') {
        let (scope, package) = scoped.split_once('/').ok_or_else(|| {
            AuthoringError::InvalidDependency(format!(
                "scoped npm package {name:?} must contain one slash"
            ))
        })?;
        if package.contains('/') {
            return Err(AuthoringError::InvalidDependency(format!(
                "scoped npm package {name:?} contains multiple slashes"
            )));
        }
        vec![scope, package]
    } else {
        if name.contains('/') {
            return Err(AuthoringError::InvalidDependency(format!(
                "unscoped npm package {name:?} contains a slash"
            )));
        }
        vec![name]
    };

    for part in parts {
        if part.is_empty()
            || part.starts_with(['.', '_'])
            || !part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'.' | b'_' | b'~')
            })
        {
            return Err(AuthoringError::InvalidDependency(format!(
                "npm package name {name:?} is not canonical lowercase registry syntax"
            )));
        }
    }
    Ok(())
}

fn validate_registry_requirement(requirement: &str) -> Result<(), AuthoringError> {
    if requirement.is_empty()
        || requirement.len() > 128
        || requirement.trim() != requirement
        || !requirement.is_ascii()
    {
        return Err(AuthoringError::InvalidDependency(format!(
            "invalid registry SemVer requirement {requirement:?}"
        )));
    }
    VersionReq::parse(requirement).map_err(|error| {
        AuthoringError::InvalidDependency(format!(
            "only registry SemVer dependency requirements are supported: {error}"
        ))
    })?;
    Ok(())
}

fn registry_requirement_matches(
    requirement: &str,
    version: &Version,
) -> Result<bool, AuthoringError> {
    if let Ok(exact) = Version::parse(requirement) {
        return Ok(exact == *version);
    }
    let requirement = VersionReq::parse(requirement).map_err(|error| {
        AuthoringError::InvalidDependencyLock(format!("invalid direct dependency request: {error}"))
    })?;
    Ok(requirement.matches(version))
}

fn validate_ascii_identifier(value: &str, field: &'static str) -> Result<(), AuthoringError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Err(AuthoringError::InvalidDependencyLock(format!(
            "{field} is not a stable ASCII identifier"
        )))
    } else {
        Ok(())
    }
}

fn validate_registry_integrity(value: &str) -> Result<(), AuthoringError> {
    let Some((algorithm, digest)) = value.split_once('-') else {
        return Err(AuthoringError::InvalidDependencyLock(
            "registry_integrity must be one SRI value".into(),
        ));
    };
    if !matches!(algorithm, "sha256" | "sha384" | "sha512")
        || digest.is_empty()
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err(AuthoringError::InvalidDependencyLock(
            "registry_integrity must be a supported base64 SRI value".into(),
        ));
    }
    Ok(())
}
