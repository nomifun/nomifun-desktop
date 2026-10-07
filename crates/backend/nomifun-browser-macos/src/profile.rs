//! Resolve aliases before creating a profile; never create directories outside
//! the app-owned root merely because a lexical prefix appeared safe.
use std::path::{Component, Path, PathBuf};

pub(crate) fn resolve(root: &Path, path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err("CEF profile is outside the owned data root".into());
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    while !ancestor.try_exists().map_err(|_| "CEF profile cannot be inspected")? {
        missing.push(ancestor.file_name().ok_or("CEF profile has no existing ancestor")?);
        ancestor = ancestor.parent().ok_or("CEF profile has no existing ancestor")?;
    }
    let mut resolved = ancestor.canonicalize().map_err(|_| "CEF profile cannot be resolved")?;
    for part in missing.into_iter().rev() { resolved.push(part); }
    if resolved == root || !resolved.starts_with(root) {
        return Err("CEF profile is outside the owned data root".into());
    }
    std::fs::create_dir_all(&resolved).map_err(|_| "CEF profile cannot be created")?;
    let resolved = resolved.canonicalize().map_err(|_| "CEF profile cannot be resolved")?;
    if resolved == root || !resolved.starts_with(root) {
        return Err("CEF profile is outside the owned data root".into());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_system_alias_but_rejects_symlink_escape_before_creating() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let alias = temp.path().join("会话/new");
        assert_eq!(resolve(&root, &alias).unwrap(), root.join("会话/new"));
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();
        assert!(resolve(&root, &root.join("escape/must-not-exist")).is_err());
        assert!(!outside.path().join("must-not-exist").exists());
        assert!(resolve(&root, &root).is_err());
        assert!(resolve(&root, &root.join("../sibling")).is_err());
    }
}
