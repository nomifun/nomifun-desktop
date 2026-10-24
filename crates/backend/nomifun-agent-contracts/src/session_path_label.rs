//! A file owner's observed Session path, distinct from a native request spelling.
//! Relative labels use `/`; absolute labels keep the host's native spelling.
//! This codec neither resolves files nor grants workspace/native authority.
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct SessionPathLabel(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid owner Session path label")]
pub struct SessionPathLabelError;

impl SessionPathLabel {
    pub fn parse(value: impl Into<String>) -> Result<Self, SessionPathLabelError> {
        let value = value.into();
        if value.len() > 4096 || value.contains('\0') { return Err(SessionPathLabelError); }
        if value.is_empty() { return Ok(Self(value)); }
        let path = Path::new(&value);
        if path.is_absolute() {
            if path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir))
                || path.components().collect::<PathBuf>().to_str() != Some(value.as_str()) {
                return Err(SessionPathLabelError);
            }
        } else {
            if value.contains('\\') || value.chars().any(char::is_control) { return Err(SessionPathLabelError); }
            let components = path.components().map(|part| match part {
                Component::Normal(name) => name.to_str(),
                _ => None,
            }).collect::<Option<Vec<_>>>().ok_or(SessionPathLabelError)?;
            if components.join("/") != value { return Err(SessionPathLabelError); }
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str { &self.0 }

    pub fn into_string(self) -> String { self.0 }

    /// Convert presentation separators only. The original file owner still
    /// decides canonical identity, containment, links and execution identity.
    pub fn native_path(&self) -> PathBuf { Path::new(&self.0).components().collect() }

    pub fn native_spelling(&self) -> String {
        self.native_path().to_str().expect("a UTF-8 label has UTF-8 native components").to_owned()
    }
}

impl<'de> Deserialize<'de> for SessionPathLabel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[path = "session_path_label_tests.rs"]
mod tests;
