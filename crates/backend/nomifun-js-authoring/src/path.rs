use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use unicode_normalization::UnicodeNormalization;

use crate::error::AuthoringError;

const MAX_NORMALIZED_PATH_BYTES: usize = 1024;
const MAX_PATH_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalizedSourcePath(String);

impl NormalizedSourcePath {
    pub fn parse(value: impl Into<String>) -> Result<Self, AuthoringError> {
        let value = value.into();
        if value.is_empty()
            || value.starts_with('/')
            || value.ends_with('/')
            || value.contains('\\')
            || value.contains('\0')
        {
            return Err(unsafe_path(
                &value,
                "path must be a non-empty forward-slash relative path",
            ));
        }
        if value.len() > MAX_NORMALIZED_PATH_BYTES {
            return Err(unsafe_path(
                &value,
                &format!("normalized path exceeds {MAX_NORMALIZED_PATH_BYTES} UTF-8 bytes"),
            ));
        }

        for component in value.split('/') {
            validate_component(&value, component)?;
        }
        windows_collision_key(&value)?;
        Ok(Self(value))
    }

    pub(crate) fn from_filesystem_path(path: &Path) -> Result<Self, AuthoringError> {
        if path.as_os_str().is_empty() || path.is_absolute() {
            return Err(unsafe_path(
                &path.display().to_string(),
                "path must be non-empty and relative",
            ));
        }
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => {
                    let value = value.to_str().ok_or_else(|| {
                        unsafe_path(&path.display().to_string(), "path must be valid UTF-8")
                    })?;
                    validate_component(&path.display().to_string(), value)?;
                    components.push(value);
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(unsafe_path(
                        &path.display().to_string(),
                        "absolute and traversal components are forbidden",
                    ));
                }
            }
        }
        Self::parse(components.join("/"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn join(&self, root: &Path) -> PathBuf {
        self.0
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    pub(crate) fn collision_key(&self) -> Result<String, AuthoringError> {
        windows_collision_key(self.as_str())
    }

    pub(crate) fn fixed_profile_rejection(&self) -> Option<&'static str> {
        let lower_components = self
            .0
            .split('/')
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        if lower_components
            .iter()
            .any(|component| component == "node_modules")
        {
            return Some("runtime node_modules is not a supported source input");
        }

        let lower = self.0.to_ascii_lowercase();
        if lower.ends_with(".node") {
            return Some("native Node addons are not supported");
        }
        if matches!(
            lower.as_str(),
            "package-lock.json" | "npm-shrinkwrap.json" | "yarn.lock" | "pnpm-lock.yaml" | ".npmrc"
        ) {
            return Some("dependency resolution and lock files are host-owned");
        }
        if lower == "package.json" && self.0 != "package.json" {
            return Some("the fixed package manifest path is lowercase package.json");
        }
        None
    }
}

impl AsRef<str> for NormalizedSourcePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for NormalizedSourcePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for NormalizedSourcePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NormalizedSourcePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(D::Error::custom)
    }
}

fn validate_component(path: &str, component: &str) -> Result<(), AuthoringError> {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.contains(':')
        || component.len() > MAX_PATH_COMPONENT_BYTES
    {
        return Err(unsafe_path(
            path,
            "path contains an empty, traversal, drive, or oversized component",
        ));
    }
    let nfc = component.nfc().collect::<String>();
    if nfc != component {
        return Err(unsafe_path(
            path,
            "path components must use Unicode NFC normalization",
        ));
    }
    Ok(())
}

fn windows_collision_key(path: &str) -> Result<String, AuthoringError> {
    let mut normalized = Vec::new();
    for component in path.split('/') {
        let trimmed = component.trim_end_matches([' ', '.']);
        if trimmed.is_empty() || trimmed != component || is_windows_reserved_name(trimmed) {
            return Err(unsafe_path(
                path,
                "path is not stable under Windows filename semantics",
            ));
        }
        normalized.push(trimmed.to_lowercase());
    }
    Ok(normalized.join("/"))
}

fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn unsafe_path(path: &str, reason: &str) -> AuthoringError {
    AuthoringError::UnsafeSourcePath {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_reject_traversal_and_windows_unstable_components() {
        for path in [
            "../escape.js",
            "src/../escape.js",
            "/absolute.js",
            "C:/absolute.js",
            "src\\main.js",
            "src//main.js",
            "src/NUL.txt",
            "src/trailing.",
            "src/trailing ",
        ] {
            assert!(NormalizedSourcePath::parse(path).is_err(), "{path}");
        }
    }

    #[test]
    fn collision_keys_follow_windows_case_rules() {
        let upper = NormalizedSourcePath::parse("Src/Main.ts").unwrap();
        let lower = NormalizedSourcePath::parse("src/main.ts").unwrap();
        assert_eq!(
            upper.collision_key().unwrap(),
            lower.collision_key().unwrap()
        );
    }
}
