use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const MANIFEST_FILE: &str = "managed-companion-skills.json";
const COPY_MARKER_FILE: &str = ".nomifun-managed-skill.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ManagedSkillRecord {
    pub(crate) source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) copy_token: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ManagedSkillManifest {
    #[serde(default)]
    pub(crate) managed: BTreeMap<String, ManagedSkillRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CopyMarker {
    token: String,
}

pub(crate) fn load_manifest(nomi_dir: &Path) -> ManagedSkillManifest {
    std::fs::read_to_string(nomi_dir.join(MANIFEST_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(crate) fn save_manifest(nomi_dir: &Path, manifest: &ManagedSkillManifest) -> io::Result<()> {
    crate::fsio::save_json_atomic(nomi_dir, MANIFEST_FILE, manifest)
}

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && !name.contains("..")
}

fn path_key(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let key = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        key.trim_start_matches("//?/").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        key
    }
}

pub(crate) fn record_source_matches(record: &ManagedSkillRecord, source: &Path) -> bool {
    path_key(&record.source) == path_key(source)
}

fn link_target_path(target: &Path) -> Option<PathBuf> {
    let linked = std::fs::read_link(target).ok()?;
    if linked.is_absolute() {
        Some(linked)
    } else {
        Some(target.parent().unwrap_or_else(|| Path::new(".")).join(linked))
    }
}

fn read_copy_marker(target: &Path) -> Option<CopyMarker> {
    std::fs::read_to_string(target.join(COPY_MARKER_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

fn managed_target_is_owned(target: &Path, record: &ManagedSkillRecord) -> bool {
    if let Some(linked) = link_target_path(target) {
        return record.copy_token.is_none() && path_key(&linked) == path_key(&record.source);
    }
    match (&record.copy_token, read_copy_marker(target)) {
        (Some(expected), Some(marker)) => marker.token == *expected,
        _ => false,
    }
}

#[cfg(windows)]
fn clear_readonly_tree(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() && std::fs::read_link(path).is_err() {
        for entry in std::fs::read_dir(path)? {
            clear_readonly_tree(&entry?.path())?;
        }
    }
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn clear_readonly_tree(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn remove_managed_target(target: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(target)?;
    if std::fs::read_link(target).is_ok() {
        return if metadata.is_dir() {
            std::fs::remove_dir(target)
        } else {
            std::fs::remove_file(target)
        };
    }
    if metadata.is_dir() {
        clear_readonly_tree(target)?;
        std::fs::remove_dir_all(target)
    } else {
        std::fs::remove_file(target)
    }
}

/// Remove obsolete entries only when their current target still proves that
/// this synchronizer owns it. Unverifiable targets are preserved and dropped
/// from the returned manifest so they can never be deleted on a later pass.
pub(crate) fn remove_stale_managed_entries(
    skills_dir: &Path,
    manifest: &ManagedSkillManifest,
    desired_names: &HashSet<&str>,
) -> ManagedSkillManifest {
    let mut retained = ManagedSkillManifest::default();
    for (name, record) in &manifest.managed {
        if !valid_skill_name(name) {
            continue;
        }
        let target = skills_dir.join(name);
        let Ok(_) = std::fs::symlink_metadata(&target) else {
            continue;
        };
        if !managed_target_is_owned(&target, record) {
            continue;
        }
        if desired_names.contains(name.as_str()) {
            retained.managed.insert(name.clone(), record.clone());
        } else if let Err(error) = remove_managed_target(&target) {
            tracing::warn!(error = %error, target = %target.display(), "remove stale managed companion skill failed");
            retained.managed.insert(name.clone(), record.clone());
        }
    }
    retained
}

pub(crate) fn write_copy_marker(target: &Path, token: &str) -> io::Result<()> {
    crate::fsio::save_json_atomic(
        target,
        COPY_MARKER_FILE,
        &CopyMarker {
            token: token.to_owned(),
        },
    )
}

/// Inspect a target created by the common Skill linker and capture enough
/// ownership evidence for a later safe cleanup.
pub(crate) fn record_managed_entry(target: &Path, source: &Path) -> io::Result<Option<ManagedSkillRecord>> {
    let metadata = std::fs::symlink_metadata(target)?;
    if link_target_path(target).is_some() {
        return Ok(Some(ManagedSkillRecord {
            source: source.to_path_buf(),
            copy_token: None,
        }));
    }
    if !metadata.is_dir() {
        return Ok(None);
    }
    let token = nomifun_common::generate_id();
    write_copy_marker(target, &token)?;
    Ok(Some(ManagedSkillRecord {
        source: source.to_path_buf(),
        copy_token: Some(token),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use super::*;

    fn manifest_with_copy(name: &str, source: &str, token: &str) -> ManagedSkillManifest {
        ManagedSkillManifest {
            managed: BTreeMap::from([(
                name.to_owned(),
                ManagedSkillRecord {
                    source: source.into(),
                    copy_token: Some(token.to_owned()),
                },
            )]),
        }
    }

    #[test]
    fn stale_manifest_does_not_delete_user_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let skills = temp.path().join(".nomi/skills");
        let target = skills.join("mermaid");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("user.txt"), "mine").unwrap();
        let manifest = manifest_with_copy("mermaid", "C:/source/mermaid", "managed-token");

        let retained = remove_stale_managed_entries(&skills, &manifest, &HashSet::new());

        assert!(target.join("user.txt").exists());
        assert!(retained.managed.is_empty());
    }

    #[test]
    fn matching_copy_marker_allows_stale_managed_copy_removal() {
        let temp = tempfile::tempdir().unwrap();
        let skills = temp.path().join(".nomi/skills");
        let target = skills.join("mermaid");
        std::fs::create_dir_all(&target).unwrap();
        write_copy_marker(&target, "managed-token").unwrap();
        let manifest = manifest_with_copy("mermaid", "C:/source/mermaid", "managed-token");

        let retained = remove_stale_managed_entries(&skills, &manifest, &HashSet::new());

        assert!(!target.exists());
        assert!(retained.managed.is_empty());
    }
}
