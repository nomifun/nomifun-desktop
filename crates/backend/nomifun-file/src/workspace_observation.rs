//! Identity of the workspace path actually resolved by the file owner.
//! This is an observation, not a lease against concurrent native renames.
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePathObservation {
    pub root_sha256: String,
    pub path: String,
    /// Windows canonicalization returns the on-disk spelling, including case.
    /// Other hosts retain conservative comparison until verified natively.
    pub case_resolved: bool,
}

impl WorkspacePathObservation {
    pub(crate) fn from_canonical(root: &Path, target: &Path) -> Option<Self> {
        let root = std::fs::canonicalize(root).ok()?;
        let relative = target.strip_prefix(&root).ok()?;
        let components = relative.components().map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        }).collect::<Option<Vec<_>>>()?;
        let path = components.join("/");
        if path.is_empty() || path.len() > 4096 { return None; }
        Some(Self {
            root_sha256: format!("{:x}", Sha256::digest(root.to_str()?.as_bytes())),
            path,
            case_resolved: cfg!(windows),
        })
    }
}
