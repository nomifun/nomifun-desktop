//! Atomic writes and fail-closed inventory checks for companion-owned skills.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use nomifun_common::AppError;
use nomifun_extension::constants::SKILL_MANIFEST_FILE;
use nomifun_extension::skill_service::{self, SkillDraftInput, SkillPaths, SkillScope};

use crate::store::CompanionSkill;

fn resolve_dir(
    paths: &SkillPaths,
    scope: &SkillScope,
    draft: bool,
    name: &str,
) -> Result<PathBuf, AppError> {
    skill_service::skill_dir_for(paths, scope, name, draft)
        .map_err(|error| AppError::BadRequest(format!("invalid skill path: {error}")))
}

/// Validate markdown with the extension parser, then fsync and atomically
/// replace the durable `SKILL.md`.
pub(crate) async fn write_skill(
    paths: &SkillPaths,
    scope: &SkillScope,
    draft: bool,
    name: &str,
    content: &str,
) -> Result<(), AppError> {
    let dir = resolve_dir(paths, scope, draft, name)?;
    let mut probe = tempfile::Builder::new()
        .prefix(".skill.validate.")
        .tempfile()
        .map_err(|error| AppError::Internal(format!("create skill validation file: {error}")))?;
    probe
        .write_all(content.as_bytes())
        .and_then(|_| probe.as_file_mut().sync_all())
        .map_err(|error| AppError::Internal(format!("write skill validation file: {error}")))?;
    let (_, description) = skill_service::read_skill_info(probe.path())
        .await
        .map_err(|error| AppError::BadRequest(format!("invalid skill content: {error}")))?;
    if description.trim().is_empty() {
        return Err(AppError::BadRequest(
            "skill description must not be empty".into(),
        ));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|error| AppError::Internal(format!("create skill directory: {error}")))?;
    crate::fsio::save_bytes_atomic(&dir, SKILL_MANIFEST_FILE, content.as_bytes())
        .map_err(|error| AppError::Internal(format!("save skill atomically: {error}")))
}

pub(crate) async fn create_skill(
    paths: &SkillPaths,
    scope: &SkillScope,
    draft: bool,
    input: &SkillDraftInput,
) -> Result<(), AppError> {
    if input.description.trim().is_empty() {
        return Err(AppError::BadRequest(
            "skill description must not be empty".into(),
        ));
    }
    let content = skill_service::build_skill_md(input);
    write_skill(paths, scope, draft, &input.name, &content).await
}

/// Filesystem half of the 共享技能 removal: move the `SKILL.md` of a re-homed
/// skill out of the legacy shared tree (`{user_skills_dir}/shared/{name}`) into
/// its owner's tree, so that `validate_store` below still finds every row's body
/// where it expects it.
///
/// Driven by the FILESYSTEM, not by "did the migration just run": for each owned
/// row whose expected directory is missing while a legacy shared directory of
/// that name is present, the body is moved. That makes it idempotent, a no-op on
/// the overwhelmingly common install that never had a shared skill, and
/// self-healing if the process dies between the database re-home
/// ([`crate::store::backfill_skill_owner`]) and this.
///
/// Rows still unowned (a name collision the backfill skipped) keep pointing at
/// the shared tree and are deliberately left alone.
pub(crate) async fn rehome_unowned_skill_dirs(
    paths: &SkillPaths,
    skills: &[CompanionSkill],
) -> Result<(), AppError> {
    for skill in skills.iter().filter(|s| s.companion_id.is_some()) {
        let expected = expected_path(paths, skill)?;
        if tokio::fs::symlink_metadata(&expected).await.is_ok() {
            continue;
        }
        let legacy = resolve_dir(paths, &SkillScope::Shared, false, &skill.skill_name)?;
        match tokio::fs::symlink_metadata(&legacy).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            // Absent, or something that is not a real directory: leave it for
            // `validate_store` to report rather than moving a surprise.
            _ => continue,
        }
        if let Some(parent) = expected.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::Internal(format!("create re-homed skill root: {error}"))
            })?;
        }
        std::fs::rename(&legacy, &expected).map_err(|error| {
            AppError::Internal(format!(
                "re-home shared skill '{}' to {}: {error}",
                skill.skill_name,
                expected.display()
            ))
        })?;
        tracing::info!(
            skill = %skill.skill_name,
            path = %expected.display(),
            "moved a shared skill body into its owner's tree"
        );
    }
    Ok(())
}

/// Atomically move a reviewed draft body into the active tree. The caller
/// persists the registry status after this filesystem rename and can roll back
/// the move if that database write fails.
pub(crate) async fn promote_draft(
    paths: &SkillPaths,
    scope: &SkillScope,
    name: &str,
) -> Result<(PathBuf, PathBuf), AppError> {
    let draft = resolve_dir(paths, scope, true, name)?;
    let active = resolve_dir(paths, scope, false, name)?;
    let draft_metadata = std::fs::symlink_metadata(&draft)
        .map_err(|error| AppError::Internal(format!("inspect draft skill: {error}")))?;
    if !draft_metadata.is_dir() || draft_metadata.file_type().is_symlink() {
        return Err(AppError::Internal(
            "draft skill directory is not a real directory".into(),
        ));
    }
    let active_exists = match std::fs::symlink_metadata(&active) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect active skill directory: {error}"
            )));
        }
    };
    if active_exists {
        return Err(AppError::Conflict(format!(
            "active skill '{}' already exists",
            name
        )));
    }
    let manifest = draft.join(SKILL_MANIFEST_FILE);
    let content = std::fs::read_to_string(&manifest)
        .map_err(|error| AppError::Internal(format!("read draft skill: {error}")))?;
    skill_service::read_skill_info(&manifest)
        .await
        .map_err(|error| AppError::Internal(format!("validate draft skill: {error}")))?;
    if content.is_empty() {
        return Err(AppError::Internal("draft skill body is empty".into()));
    }
    if let Some(parent) = active.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::Internal(format!("create active skill root: {error}")))?;
    }
    std::fs::rename(&draft, &active)
        .map_err(|error| AppError::Internal(format!("promote draft skill atomically: {error}")))?;
    if let Some(draft_parent) = draft.parent()
        && let Err(error) = std::fs::remove_dir(draft_parent)
        && error.kind() != std::io::ErrorKind::DirectoryNotEmpty
        && error.kind() != std::io::ErrorKind::NotFound
    {
        std::fs::rename(&active, &draft).map_err(|rollback_error| {
            AppError::Internal(format!(
                "clean promoted draft owner directory: {error}; additionally failed to roll back promotion: {rollback_error}"
            ))
        })?;
        return Err(AppError::Internal(format!(
            "clean promoted draft owner directory: {error}"
        )));
    }
    Ok((draft, active))
}

pub(crate) fn rollback_promotion(draft: &Path, active: &Path) -> Result<(), AppError> {
    if let Some(parent) = draft.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::Internal(format!("recreate draft owner directory: {error}")))?;
    }
    std::fs::rename(active, draft)
        .map_err(|error| AppError::Internal(format!("rollback skill promotion: {error}")))
}

/// Read one row's durable `SKILL.md` body — the bytes a companion bundle
/// carries. The row's owner and status decide where that body lives, so this is
/// deliberately the same [`expected_path`] the boot inventory audit uses: a
/// bundle can never export a body from a place the audit would call orphaned.
pub(crate) async fn read_skill_body(
    paths: &SkillPaths,
    skill: &CompanionSkill,
) -> Result<String, AppError> {
    let manifest = expected_path(paths, skill)?.join(SKILL_MANIFEST_FILE);
    tokio::fs::read_to_string(&manifest).await.map_err(|error| {
        AppError::Internal(format!(
            "read skill body {}: {error}",
            manifest.display()
        ))
    })
}

/// Delete a companion's whole skill tree (active + drafts). Used by the delete
/// cascade and by the rollback of a half-imported companion bundle: leaving a
/// body behind whose row is gone makes `validate_store` hard-fail the next boot.
pub(crate) fn remove_companion_trees(
    paths: &SkillPaths,
    companion_id: &str,
) -> Result<(), AppError> {
    let companion_id = nomifun_common::CompanionId::try_from(companion_id)
        .map_err(|error| AppError::BadRequest(format!("invalid companion id: {error}")))?;
    for path in [
        skill_service::companion_skills_root(paths).join(companion_id.as_str()),
        skill_service::drafts_root(paths).join(companion_id.as_str()),
    ] {
        crate::fsio::remove_path_entry(&path).map_err(|error| {
            AppError::Internal(format!(
                "remove companion skill tree {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

/// Where a row's body must live: under its owner's tree, or — for a row the
/// re-homing migration has not claimed yet — the legacy shared tree.
fn expected_path(paths: &SkillPaths, skill: &CompanionSkill) -> Result<PathBuf, AppError> {
    let scope = skill
        .companion_id
        .as_deref()
        .map_or(SkillScope::Shared, |id| SkillScope::Companion(id.to_owned()));
    resolve_dir(paths, &scope, skill.status == "draft", &skill.skill_name)
}

fn inventory_error(path: &Path, detail: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!(
        "companion skill inventory {} is inconsistent: {detail}",
        path.display()
    ))
}

async fn require_skill_dir(path: &Path) -> Result<(), AppError> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| inventory_error(path, error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(inventory_error(
            path,
            "skill directory is not a real directory",
        ));
    }
    let manifest = path.join(SKILL_MANIFEST_FILE);
    let manifest_metadata = tokio::fs::symlink_metadata(&manifest)
        .await
        .map_err(|error| inventory_error(path, error))?;
    if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
        return Err(inventory_error(
            path,
            "SKILL.md is not a real regular file",
        ));
    }
    skill_service::read_skill_info(&manifest)
        .await
        .map_err(|error| inventory_error(path, error))?;
    Ok(())
}

async fn scan_skill_root(
    root: &Path,
    expected: &HashSet<PathBuf>,
    owner_partitioned: bool,
) -> Result<(), AppError> {
    let root_metadata = match tokio::fs::symlink_metadata(root).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if expected.iter().any(|path| path.starts_with(root)) {
                return Err(inventory_error(root, "managed root is missing"));
            }
            return Ok(());
        }
        Err(error) => return Err(inventory_error(root, error)),
    };
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(inventory_error(
            root,
            "managed root is not a real directory",
        ));
    }

    let mut outer = tokio::fs::read_dir(root)
        .await
        .map_err(|error| inventory_error(root, error))?;
    while let Some(entry) = outer
        .next_entry()
        .await
        .map_err(|error| inventory_error(root, error))?
    {
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|error| inventory_error(&path, error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(inventory_error(&path, "unexpected non-directory entry"));
        }
        if owner_partitioned {
            if !expected
                .iter()
                .any(|expected| expected.parent() == Some(path.as_path()))
            {
                return Err(inventory_error(&path, "orphaned owner directory"));
            }
            let mut skills = tokio::fs::read_dir(&path)
                .await
                .map_err(|error| inventory_error(&path, error))?;
            while let Some(skill_entry) = skills
                .next_entry()
                .await
                .map_err(|error| inventory_error(&path, error))?
            {
                let skill_path = skill_entry.path();
                let skill_metadata = tokio::fs::symlink_metadata(&skill_path)
                    .await
                    .map_err(|error| inventory_error(&skill_path, error))?;
                if !skill_metadata.is_dir() || skill_metadata.file_type().is_symlink() {
                    return Err(inventory_error(
                        &skill_path,
                        "unexpected non-directory skill entry",
                    ));
                }
                if !expected.contains(&skill_path) {
                    return Err(inventory_error(&skill_path, "orphaned skill directory"));
                }
                require_skill_dir(&skill_path).await?;
            }
        } else {
            if !expected.contains(&path) {
                return Err(inventory_error(&path, "orphaned skill directory"));
            }
            require_skill_dir(&path).await?;
        }
    }

    for path in expected.iter().filter(|path| path.starts_with(root)) {
        require_skill_dir(path).await?;
    }
    Ok(())
}

/// Audit registry rows against the managed active, draft, and shared trees.
pub(crate) async fn validate_store(
    paths: &SkillPaths,
    skills: &[CompanionSkill],
) -> Result<(), AppError> {
    let mut expected = HashSet::new();
    for skill in skills {
        if !matches!(skill.status.as_str(), "draft" | "active" | "archived") {
            return Err(AppError::Internal(format!(
                "companion skill '{}' has invalid status '{}'",
                skill.skill_name, skill.status
            )));
        }
        let path = expected_path(paths, skill)?;
        if !expected.insert(path) {
            return Err(AppError::Internal(format!(
                "duplicate durable companion skill path for '{}'",
                skill.skill_name
            )));
        }
    }
    scan_skill_root(
        &skill_service::companion_skills_root(paths),
        &expected,
        true,
    )
    .await?;
    scan_skill_root(&skill_service::drafts_root(paths), &expected, true).await?;
    scan_skill_root(
        &skill_service::shared_skills_root(paths),
        &expected,
        false,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &Path) -> SkillPaths {
        skill_service::resolve_skill_paths(root, root)
    }

    fn row(owner: &str, name: &str, status: &str) -> CompanionSkill {
        CompanionSkill {
            companion_skill_id: nomifun_common::generate_id(),
            skill_name: name.into(),
            companion_id: Some(owner.into()),
            status: status.into(),
            source: "mined".into(),
            confidence: 0.8,
            provenance_event_ids: vec![],
            strength: 1.0,
            version: 1,
            skill_pattern_id: None,
            usage_count: 0,
            last_used_at: None,
            created_at: 1,
            updated_at: 1,
            signature: String::new(),
        }
    }

    fn input(name: &str) -> SkillDraftInput {
        SkillDraftInput {
            name: name.into(),
            description: "description".into(),
            when_to_use: None,
            allowed_tools: None,
            paths: None,
            body: "body".into(),
        }
    }

    #[tokio::test]
    async fn validates_exact_inventory_and_rejects_missing_or_orphaned_directories() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let owner = nomifun_common::CompanionId::new().into_string();
        let scope = SkillScope::Companion(owner.clone());
        create_skill(&paths, &scope, true, &input("drafted"))
            .await
            .unwrap();
        let rows = vec![row(&owner, "drafted", "draft")];
        validate_store(&paths, &rows).await.unwrap();

        let expected = skill_service::skill_dir_for(&paths, &scope, "drafted", true).unwrap();
        crate::fsio::remove_path_entry(&expected).unwrap();
        assert!(validate_store(&paths, &rows).await.is_err());

        create_skill(&paths, &scope, true, &input("drafted"))
            .await
            .unwrap();
        create_skill(&paths, &scope, true, &input("orphan"))
            .await
            .unwrap();
        assert!(validate_store(&paths, &rows).await.is_err());
    }

    /// The filesystem half of the 共享技能 removal. A legacy install has the body
    /// under `{user_skills_dir}/shared/{name}`; once the row is re-homed onto its
    /// owner the boot inventory audit expects it under that owner's tree, so
    /// without this move EVERY such install would fail `validate_store` at launch.
    #[tokio::test]
    async fn rehoming_moves_a_shared_body_into_its_owner_tree_and_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let owner = nomifun_common::CompanionId::new().into_string();
        create_skill(&paths, &SkillScope::Shared, false, &input("legacy"))
            .await
            .unwrap();
        // The database row has already been re-homed by `backfill_skill_owner`.
        let rows = vec![row(&owner, "legacy", "active")];
        assert!(
            validate_store(&paths, &rows).await.is_err(),
            "the audit must reject the row until its body moves"
        );

        rehome_unowned_skill_dirs(&paths, &rows).await.unwrap();
        validate_store(&paths, &rows).await.unwrap();
        let moved =
            skill_service::skill_dir_for(&paths, &SkillScope::Companion(owner.clone()), "legacy", false).unwrap();
        assert!(moved.join(SKILL_MANIFEST_FILE).is_file(), "body must live under its owner");
        let legacy = skill_service::skill_dir_for(&paths, &SkillScope::Shared, "legacy", false).unwrap();
        assert!(!legacy.exists(), "the shared copy is MOVED, never left behind as a duplicate");

        // A second boot must not touch the already-correct body.
        rehome_unowned_skill_dirs(&paths, &rows).await.unwrap();
        validate_store(&paths, &rows).await.unwrap();

        // A row the backfill left unowned (name collision) keeps its shared body.
        let mut unowned = row(&owner, "kept", "active");
        unowned.companion_id = None;
        create_skill(&paths, &SkillScope::Shared, false, &input("kept"))
            .await
            .unwrap();
        let rows = vec![row(&owner, "legacy", "active"), unowned];
        rehome_unowned_skill_dirs(&paths, &rows).await.unwrap();
        validate_store(&paths, &rows).await.unwrap();
        assert!(
            skill_service::skill_dir_for(&paths, &SkillScope::Shared, "kept", false)
                .unwrap()
                .join(SKILL_MANIFEST_FILE)
                .is_file()
        );
    }
}
