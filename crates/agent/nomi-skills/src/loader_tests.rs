use super::*;
use serial_test::serial;
use std::fs;
use tempfile::TempDir;

pub(super) fn write_skill(dir: &Path, rel_path: &str, content: &str) {
    let full = dir.join(rel_path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

// --- build_namespace ---

#[test]
fn test_build_namespace_simple() {
    let base = Path::new("/skills");
    let target = Path::new("/skills/my-skill");
    assert_eq!(build_namespace(base, target), "my-skill");
}

#[test]
fn test_build_namespace_nested() {
    let base = Path::new("/skills");
    let target = Path::new("/skills/db/migrate");
    assert_eq!(build_namespace(base, target), "db:migrate");
}

#[test]
fn test_build_namespace_three_levels() {
    let base = Path::new("/skills");
    let target = Path::new("/skills/a/b/c");
    assert_eq!(build_namespace(base, target), "a:b:c");
}

#[test]
fn test_build_namespace_same_dir() {
    let base = Path::new("/skills");
    // target == base → empty string
    let result = build_namespace(base, base);
    assert_eq!(result, "");
}

// --- try_canonicalize ---

#[test]
fn test_try_canonicalize_existing_path() {
    let tmp = TempDir::new().unwrap();
    let result = try_canonicalize(tmp.path());
    assert!(result.is_some());
}

#[test]
fn test_try_canonicalize_nonexistent_returns_none() {
    let result = try_canonicalize(Path::new("/nonexistent/path/xyz"));
    assert!(result.is_none());
}

// --- deduplicate ---

#[test]
fn test_deduplicate_removes_duplicates() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("skill.md");
    fs::write(&file, "").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();

    let fm = crate::types::FrontmatterData::default();
    let make_meta = || {
        crate::frontmatter::parse_skill_fields(
            &fm,
            "",
            "test",
            SkillSource::User,
            LoadedFrom::Skills,
            None,
        )
    };

    let skills = vec![
        LoadedSkill {
            metadata: make_meta(),
            resolved_path: canonical.clone(),
        },
        LoadedSkill {
            metadata: make_meta(),
            resolved_path: canonical.clone(),
        },
    ];

    let result = deduplicate(skills);
    assert_eq!(result.len(), 1);
}

#[test]
fn test_deduplicate_different_paths_preserved() {
    let tmp = TempDir::new().unwrap();
    let file1 = tmp.path().join("skill1.md");
    let file2 = tmp.path().join("skill2.md");
    fs::write(&file1, "").unwrap();
    fs::write(&file2, "").unwrap();

    let fm = crate::types::FrontmatterData::default();
    let make_meta = || {
        crate::frontmatter::parse_skill_fields(
            &fm,
            "",
            "test",
            SkillSource::User,
            LoadedFrom::Skills,
            None,
        )
    };

    let skills = vec![
        LoadedSkill {
            metadata: make_meta(),
            resolved_path: std::fs::canonicalize(&file1).unwrap(),
        },
        LoadedSkill {
            metadata: make_meta(),
            resolved_path: std::fs::canonicalize(&file2).unwrap(),
        },
    ];

    let result = deduplicate(skills);
    assert_eq!(result.len(), 2);
}

// --- load_skills_from_dir ---

#[cfg(any(unix, windows))]
#[tokio::test]
async fn test_directory_links_are_followed_without_cycles() {
    for cycle in [true, false] {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("skills");
        write_skill(&base, "valid/SKILL.md", "---\n---\n");
        let target = if cycle {
            base.clone()
        } else {
            tmp.path().join("external")
        };
        if !cycle {
            write_skill(&target, "other/SKILL.md", "---\n---\n");
        }
        let link = base.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        {
            // Junctions do not require Windows Developer Mode or symlink privileges.
            let output = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command",
                    "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:NOMI_TEST_LINK -Target $env:NOMI_TEST_TARGET | Out-Null"])
                .env("NOMI_TEST_LINK", &link)
                .env("NOMI_TEST_TARGET", &target)
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let skills = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            load_skills_from_dir(&base, SkillSource::User, LoadedFrom::Skills),
        )
        .await;
        let commands = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            load_skills_from_commands_dir(&base, SkillSource::User),
        )
        .await;
        // Remove only the link before TempDir cleans up the fixture.
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
        #[cfg(unix)]
        fs::remove_file(&link).unwrap();
        let expected = if cycle {
            vec!["valid"]
        } else {
            vec!["link:other", "valid"]
        };
        for loaded in [skills.unwrap(), commands.unwrap()] {
            let names: Vec<_> = loaded.iter().map(|s| s.metadata.name.as_str()).collect();
            assert_eq!(names, expected);
        }
    }
}

#[tokio::test]
async fn test_load_skills_from_dir_basic() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "my-skill/SKILL.md",
        "---\nname: my-skill\ndescription: A test skill\n---\n# Body\n",
    );

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "my-skill");
}

#[tokio::test]
async fn test_load_skills_from_dir_nested_namespace() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "db/migrate/SKILL.md",
        "---\ndescription: Migrate DB\n---\n",
    );

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "db:migrate");
}

#[tokio::test]
async fn test_load_skills_from_dir_case_sensitive_skill_md() {
    let tmp = TempDir::new().unwrap();
    // Only lowercase "skill.md" — should NOT be loaded
    write_skill(tmp.path(), "my-skill/skill.md", "---\n---\n# Body\n");

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert!(
        skills.is_empty(),
        "skill.md (lowercase) should not be loaded"
    );
}

#[tokio::test]
async fn test_load_skills_from_dir_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert!(skills.is_empty());
}

#[tokio::test]
async fn test_load_skills_from_dir_nonexistent_silently_skipped() {
    let skills = load_skills_from_dir(
        Path::new("/nonexistent/path"),
        SkillSource::User,
        LoadedFrom::Skills,
    )
    .await;
    assert!(skills.is_empty());
}

// --- load_skills_from_commands_dir ---

#[tokio::test]
async fn test_load_commands_directory_format() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "my-cmd/SKILL.md",
        "---\ndescription: A command\n---\n",
    );

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(
        skills[0].metadata.loaded_from,
        LoadedFrom::CommandsDeprecated
    );
}

#[tokio::test]
async fn test_load_commands_flat_format() {
    let tmp = TempDir::new().unwrap();
    write_skill(tmp.path(), "simple.md", "---\ndescription: Simple\n---\n");

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "simple");
    assert_eq!(skills[0].metadata.skill_root.as_deref(), Some(tmp.path().to_str().unwrap()));
    assert_eq!(
        skills[0].metadata.loaded_from,
        LoadedFrom::CommandsDeprecated
    );
}

#[tokio::test]
async fn test_load_commands_dir_format_takes_precedence_over_flat() {
    let tmp = TempDir::new().unwrap();
    // Both my-cmd/SKILL.md and my-cmd.md exist — directory format wins
    write_skill(
        tmp.path(),
        "my-cmd/SKILL.md",
        "---\ndescription: Directory version\n---\n",
    );
    write_skill(
        tmp.path(),
        "my-cmd.md",
        "---\ndescription: Flat version\n---\n",
    );

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    let descriptions: Vec<_> = skills
        .iter()
        .map(|s| s.metadata.description.as_str())
        .collect();
    assert!(
        descriptions.contains(&"Directory version"),
        "directory format should be loaded"
    );
    assert!(
        !descriptions.contains(&"Flat version"),
        "flat format should be skipped when directory exists"
    );
}

#[tokio::test]
async fn test_load_commands_nested_flat() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "db/migrate.md",
        "---\ndescription: DB migrate\n---\n",
    );

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "db:migrate");
}

// --- load_all_skills ---

#[tokio::test]
#[serial]
async fn test_load_all_skills_bare_mode() {
    let tmp = TempDir::new().unwrap();
    // Create .nomi/skills/ under the add_dir
    let skills_dir = tmp.path().join(".nomi").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    write_skill(&skills_dir, "my-skill/SKILL.md", "---\n---\n");

    let result = load_all_skills(
        Path::new("/nonexistent"),
        &[tmp.path().to_owned()],
        true,
        None,
    )
    .await;
    assert!(
        result.iter().any(|skill| skill.name == "my-skill"),
        "bare mode should load skills from explicit add_dirs"
    );
}

#[tokio::test]
#[serial]
async fn test_load_all_skills_deduplicates() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // Create git root
    fs::create_dir(root.join(".git")).unwrap();

    // Create same skill in project dir (will appear twice due to walk)
    let skills_dir = root.join(".nomi").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    write_skill(&skills_dir, "my-skill/SKILL.md", "---\n---\n");

    let result = load_all_skills(root, &[], false, None).await;
    let names: Vec<_> = result.iter().map(|s| s.name.as_str()).collect();
    let count = names.iter().filter(|&&n| n == "my-skill").count();
    assert_eq!(count, 1, "skill should appear exactly once after dedup");
}
