use nomifun_agent_contracts::{FrozenSkillContent, LibrarySkillSource};
use nomifun_skill_library::{
    SkillPaths, SkillSource, builtin_skills_corpus, builtin_skills_materialize_version,
    frozen::{capture_selected, default_skill_names},
    list_available_skills, materialize_if_needed,
    skill_service::{load_builtin_skill_display_metadata, load_builtin_skill_tags},
};
use std::{collections::HashSet, fs, path::Path};
use tempfile::TempDir;

async fn materialize(root: &Path) -> SkillPaths {
    materialize_if_needed(
        root,
        builtin_skills_corpus(),
        &builtin_skills_materialize_version("fixture"),
    )
    .await
    .unwrap();
    SkillPaths {
        data_dir: root.into(),
        user_skills_dir: root.join("skills"),
        cron_skills_dir: root.join("cron/skills"),
        builtin_skills_dir: root.join("builtin-skills"),
        builtin_rules_dir: root.join("builtin-rules"),
    }
}

#[tokio::test]
async fn lark_cli_is_a_materialized_opt_in_builtin() {
    let root = TempDir::new().unwrap();
    let paths = materialize(root.path()).await;
    let skills = list_available_skills(&paths).await.unwrap();
    let lark = skills.iter().find(|skill| skill.name == "lark-cli").unwrap();

    assert_eq!(lark.source, SkillSource::Builtin);
    assert!(!lark.is_custom);
    assert_eq!(lark.relative_location.as_deref(), Some("lark-cli/SKILL.md"));
    assert_eq!(Path::new(&lark.location), paths.builtin_skills_dir.join("lark-cli/SKILL.md"));
    assert!(!lark.description.trim().is_empty());
    assert!(!default_skill_names(&paths).await.unwrap().iter().any(|name| name == "lark-cli"));
}

#[test]
fn lark_cli_has_chinese_presentation_and_existing_tags() {
    let display = load_builtin_skill_display_metadata();
    let lark = display.get("lark-cli").unwrap();
    assert_eq!(lark.name_i18n.get("zh-CN").map(String::as_str), Some("飞书 CLI"));
    assert!(lark.description_i18n.get("zh-CN").is_some_and(|text| !text.trim().is_empty()));

    let tags = load_builtin_skill_tags();
    let (audience, scenarios) = tags.get("lark-cli").unwrap();
    let known_audience: HashSet<_> = tags.iter().filter(|(name, _)| name.as_str() != "lark-cli")
        .flat_map(|(_, (audience, _))| audience).map(String::as_str).collect();
    let known_scenarios: HashSet<_> = tags.iter().filter(|(name, _)| name.as_str() != "lark-cli")
        .flat_map(|(_, (_, scenarios))| scenarios).map(String::as_str).collect();
    assert!(!audience.is_empty());
    assert!(!scenarios.is_empty());
    assert!(audience.iter().all(|tag| known_audience.contains(tag.as_str())));
    assert!(scenarios.iter().all(|tag| known_scenarios.contains(tag.as_str())));
}

#[tokio::test]
async fn lark_cli_selection_freezes_the_builtin_body_and_references() {
    let root = TempDir::new().unwrap();
    let paths = materialize(root.path()).await;
    let selected = capture_selected(&paths, &["lark-cli".into()]).await.unwrap();
    assert_eq!(selected.len(), 1);
    let lark = &selected[0];
    assert_eq!(lark.id.as_ref(), "library:lark-cli");
    assert_eq!(lark.source, LibrarySkillSource::Builtin);
    lark.validate().unwrap();

    let source_root = paths.builtin_skills_dir.join("lark-cli");
    assert_eq!(lark.body, fs::read_to_string(source_root.join("SKILL.md")).unwrap());
    for path in ["references/installation.md", "references/setup.md", "references/commands.md", "references/updates.md"] {
        let resource = lark.resources.get(path).unwrap();
        let FrozenSkillContent::Text { text } = &resource.content else {
            panic!("{path} must be a readable text resource");
        };
        assert!(!text.trim().is_empty());
        assert_eq!(text, &fs::read_to_string(source_root.join(path)).unwrap());
        assert_eq!(text.as_bytes(), builtin_skills_corpus().get_file(format!("lark-cli/{path}")).unwrap().contents());
    }
}
