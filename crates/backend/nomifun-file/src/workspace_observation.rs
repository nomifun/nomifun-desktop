//! Identity of the workspace path actually resolved by the file owner.
//! This is an observation, not a lease against concurrent native renames.
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) fn canonical_root_sha256(root: &Path) -> Option<String> {
    Some(format!("{:x}", Sha256::digest(root.to_str()?.as_bytes())))
}

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
        // Global native paths can name entries that project presentation
        // cannot represent (or that belong to the platform's own subtree).
        // Keep their native receipt without inventing a portable project path.
        if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control)
            || crate::artifact_store::normalized_workspace_relative(&path, false).is_err() { return None; }
        Some(Self {
            root_sha256: canonical_root_sha256(&root)?,
            path,
            case_resolved: cfg!(windows),
        })
    }
}
