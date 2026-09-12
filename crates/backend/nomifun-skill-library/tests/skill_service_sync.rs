use nomifun_skill_library::{ResolvedAgentSkill, SkillError, link_workspace_skills, sync_workspace_skills};

#[tokio::test]
async fn workspace_targets_are_all_validated_before_any_pruning_or_linking() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let skill_dir = workspace.join(".nomi/skills/kept");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "keep skill").unwrap();
    std::fs::write(workspace.join("project.txt"), "keep project").unwrap();

    for invalid in ["", ".", "../outside", "/absolute"] {
        let targets = [".nomi/skills", invalid];
        assert!(matches!(sync_workspace_skills(&workspace, &targets, &[]).await, Err(SkillError::InvalidSkillPath(_))));
        assert!(matches!(link_workspace_skills(&workspace, &targets, &[]).await, Err(SkillError::InvalidSkillPath(_))));
        assert_eq!(std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap(), "keep skill");
        assert_eq!(std::fs::read_to_string(workspace.join("project.txt")).unwrap(), "keep project");
    }
}

#[tokio::test]
async fn sync_workspace_skills_removes_deselected_entries_without_touching_sources() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let source_root = temp.path().join("sources");
    std::fs::create_dir_all(&workspace).unwrap();

    let make_skill = |name: &str| {
        let source_path = source_root.join(name);
        std::fs::create_dir_all(&source_path).unwrap();
        std::fs::write(source_path.join("SKILL.md"), format!("# {name}")).unwrap();
        ResolvedAgentSkill {
            name: name.to_owned(),
            source_path,
        }
    };
    let first = make_skill("first");
    let second = make_skill("second");

    sync_workspace_skills(&workspace, &[".nomi/skills"], &[first.clone(), second.clone()])
        .await
        .unwrap();
    assert!(workspace.join(".nomi/skills/first/SKILL.md").is_file());
    assert!(workspace.join(".nomi/skills/second/SKILL.md").is_file());

    sync_workspace_skills(&workspace, &[".nomi/skills"], std::slice::from_ref(&second))
        .await
        .unwrap();
    assert!(!workspace.join(".nomi/skills/first").exists());
    assert!(workspace.join(".nomi/skills/second/SKILL.md").is_file());
    assert!(first.source_path.join("SKILL.md").is_file());

    sync_workspace_skills(&workspace, &[".nomi/skills"], &[])
        .await
        .unwrap();
    assert!(!workspace.join(".nomi/skills/second").exists());
    assert!(second.source_path.join("SKILL.md").is_file());
}

#[tokio::test]
async fn sync_workspace_skills_rejects_redirected_skill_directory() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep").unwrap();

    let redirected = workspace.join(".nomi");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &redirected).unwrap();
    #[cfg(windows)]
    junction::create(&outside, &redirected).unwrap();

    let error = sync_workspace_skills(&workspace, &[".nomi/skills"], &[])
        .await
        .unwrap_err();
    assert!(matches!(error, SkillError::InvalidSkillPath(_)));
    assert_eq!(std::fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep");

    #[cfg(unix)]
    std::fs::remove_file(&redirected).unwrap();
    #[cfg(windows)]
    junction::delete(&redirected).unwrap();
}

#[tokio::test]
async fn sync_workspace_skills_unlinks_redirected_child_without_following_it() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let skills_dir = workspace.join(".nomi/skills");
    let outside = temp.path().join("outside-child");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep").unwrap();

    let redirected = skills_dir.join("stale");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &redirected).unwrap();
    #[cfg(windows)]
    junction::create(&outside, &redirected).unwrap();

    sync_workspace_skills(&workspace, &[".nomi/skills"], &[])
        .await
        .unwrap();
    assert!(!redirected.exists());
    assert_eq!(std::fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep");
}
