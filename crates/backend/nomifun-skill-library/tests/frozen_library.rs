use nomifun_skill_library::{SkillPaths, frozen::{capture_inventory, capture_selected, default_skill_names}};
use nomifun_agent_contracts::{FrozenSkillContent, LibrarySkillSource};
use std::fs;
use tempfile::TempDir;
fn paths(root: &std::path::Path) -> SkillPaths {
    SkillPaths { data_dir:root.into(), user_skills_dir:root.join("skills"), cron_skills_dir:root.join("cron/skills"),
        builtin_skills_dir:root.join("builtin-skills"), builtin_rules_dir:root.join("builtin-rules") }
}
fn write_skill(root: &std::path::Path, name: &str) {
    fs::create_dir_all(root.join("references")).unwrap();
    fs::write(root.join("SKILL.md"), format!("---\nname: {name}\ndescription: Use exact instructions\nallowed-tools: ignored\n---\nRead references/guide.md.")).unwrap();
    fs::write(root.join("references/guide.md"), "frozen original").unwrap();
}
#[tokio::test]
async fn capture_preserves_user_skill_bytes_and_does_not_follow_later_source_edits() {
    let root=TempDir::new().unwrap(); let paths=paths(root.path());
    write_skill(&paths.user_skills_dir.join("guide"), "guide");
    let frozen=capture_selected(&paths,&["guide".into()]).await.unwrap().remove(0);
    frozen.validate().unwrap(); assert_eq!(frozen.id.as_ref(), "library:guide"); assert_eq!(frozen.source,LibrarySkillSource::Custom);
    fs::write(paths.user_skills_dir.join("guide/references/guide.md"),"new source").unwrap();
    assert!(matches!(&frozen.resources["references/guide.md"].content,FrozenSkillContent::Text{text} if text=="frozen original"));
    let current=capture_selected(&paths,&["guide".into()]).await.unwrap().remove(0);
    assert_ne!(frozen.source_digest,current.source_digest);
    assert!(capture_selected(&paths,&["uninstalled".into()]).await.is_err());
}
#[tokio::test]
async fn default_names_are_real_auto_entries_and_custom_override_keeps_its_source() {
    let root=TempDir::new().unwrap();let paths=paths(root.path());
    write_skill(&paths.builtin_skills_dir.join("auto-inject/guide"),"guide");
    write_skill(&paths.user_skills_dir.join("guide"),"guide");
    assert_eq!(default_skill_names(&paths).await.unwrap(),vec!["guide"]);
    let frozen=capture_selected(&paths,&["guide".into()]).await.unwrap().remove(0);
    assert_eq!(frozen.source,LibrarySkillSource::Custom);
}
#[cfg(unix)]
#[tokio::test]
async fn explicitly_imported_root_link_is_captured_but_resource_links_are_rejected() {
    use std::os::unix::fs::symlink;
    let root=TempDir::new().unwrap();let paths=paths(root.path());let external=root.path().join("external");
    write_skill(&external,"linked");fs::create_dir_all(&paths.user_skills_dir).unwrap();
    symlink(&external,paths.user_skills_dir.join("linked")).unwrap();
    let captured=capture_selected(&paths,&["linked".into()]).await.unwrap();assert_eq!(captured.len(),1);
    symlink(root.path(),external.join("references/escape")).unwrap();
    assert!(capture_selected(&paths,&["linked".into()]).await.is_err());
}

#[tokio::test]
async fn bundled_skill_inventory_can_be_frozen_through_the_same_current_library_path() {
    let root=TempDir::new().unwrap();let paths=paths(root.path());
    nomifun_skill_library::materialize_if_needed(root.path(),nomifun_skill_library::builtin_skills_corpus(),"fixture").await.unwrap();
    let inventory=capture_inventory(&paths).await.unwrap();
    assert!(inventory.unavailable.is_empty(), "{:?}", inventory.unavailable);
    let inventory=inventory.skills;
    assert!(inventory.iter().any(|skill| skill.name=="creative-studio-canvas"));
    assert!(inventory.iter().any(|skill| skill.name=="skill-creator"));
    for skill in inventory { skill.validate().unwrap(); }
}

#[tokio::test]
async fn unselected_binary_attachment_cannot_poison_the_inventory_or_an_empty_selection() {
    let root=TempDir::new().unwrap();let paths=paths(root.path());
    write_skill(&paths.user_skills_dir.join("good"),"good");
    write_skill(&paths.user_skills_dir.join("broken"),"broken");
    fs::write(paths.user_skills_dir.join("broken/references/binary.pdf"),[0xff,0xfe,0,1]).unwrap();
    let inventory=capture_inventory(&paths).await.unwrap();
    assert_eq!(inventory.skills.iter().map(|skill|skill.name.as_str()).collect::<Vec<_>>(),vec!["good"]);
    assert_eq!(inventory.unavailable.len(),1);
    assert_eq!(inventory.unavailable[0].name,"broken");
    assert!(inventory.unavailable[0].reason.contains("UTF-8"));
    assert!(capture_selected(&paths,&[]).await.unwrap().is_empty());
    assert_eq!(capture_selected(&paths,&["good".into()]).await.unwrap().len(),1);
    assert!(capture_selected(&paths,&["broken".into()]).await.is_err());
    // Management still sees the bad package; only native availability is denied.
    assert!(nomifun_skill_library::list_available_skills(&paths).await.unwrap().iter().any(|item|item.name=="broken"));
}

#[tokio::test]
async fn malformed_pixels_are_diagnosed_without_a_mutable_resource_fallback() {
    let root=TempDir::new().unwrap();let paths=paths(root.path());
    write_skill(&paths.user_skills_dir.join("image-guide"),"image-guide");
    fs::write(paths.user_skills_dir.join("image-guide/references/bad.png"),b"not an image").unwrap();
    let inventory=capture_inventory(&paths).await.unwrap();
    assert!(inventory.skills.is_empty());
    assert_eq!(inventory.unavailable[0].name,"image-guide");
    assert!(capture_selected(&paths,&["image-guide".into()]).await.is_err());
}

#[tokio::test]
async fn duplicate_logical_names_are_reported_instead_of_freezing_an_arbitrary_source() {
    let root=TempDir::new().unwrap();let paths=paths(root.path());
    write_skill(&paths.user_skills_dir.join("first-source"),"duplicate");
    write_skill(&paths.user_skills_dir.join("second-source"),"duplicate");
    let inventory=capture_inventory(&paths).await.unwrap();
    assert!(inventory.skills.is_empty());assert_eq!(inventory.unavailable.len(),1);
    assert_eq!(inventory.unavailable[0].name,"duplicate");
    assert!(capture_selected(&paths,&["duplicate".into()]).await.is_err());
}
